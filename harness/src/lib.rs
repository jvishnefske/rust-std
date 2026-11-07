//! Helpers for building the programs in `apps/` with std for
//! `*-unknown-linux-none` and running them, from ordinary host tests (see
//! `tests/`).
//!
//! Builds use the pinned nightly with `__CARGO_TESTS_ONLY_SRC_ROOT` pointing
//! at the patched `library/` (rather than the `linux-none` toolchain), and
//! carry a hash of the std and rt sources in their rustflags: cargo does not
//! notice edits to `-Zbuild-std` crates on its own.
//!
//! Everything builds in two configurations ([`Panic`]): `panic=abort`, as
//! `apps/` is set up, and `panic=unwind` (in `apps/target/unwind`).

pub mod fuzz;
pub mod unittests;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    Aarch64,
}

impl Arch {
    /// The `--target` argument: the built-in triple, or our JSON spec.
    fn target_arg(self) -> String {
        match self {
            Arch::X86_64 => "x86_64-unknown-linux-none".into(),
            Arch::Aarch64 => repo().join("targets/aarch64-unknown-linux-none.json").display().to_string(),
        }
    }

    /// The target's name (output directory, `[target.<name>]` config key).
    pub fn name(self) -> &'static str {
        match self {
            Arch::X86_64 => "x86_64-unknown-linux-none",
            Arch::Aarch64 => "aarch64-unknown-linux-none",
        }
    }

    pub fn machine(self) -> u16 {
        match self {
            Arch::X86_64 => EM_X86_64,
            Arch::Aarch64 => EM_AARCH64,
        }
    }

    /// The runner for this arch's binaries: none when it matches the host,
    /// else qemu if available (`None` means they cannot run here).
    pub fn runner(self) -> Option<Option<PathBuf>> {
        let native = matches!((self, std::env::consts::ARCH), (Arch::X86_64, "x86_64") | (Arch::Aarch64, "aarch64"));
        if native {
            return Some(None);
        }
        match self {
            Arch::Aarch64 => qemu_aarch64().map(Some),
            Arch::X86_64 => None,
        }
    }
}

/// The panic strategy programs and std are built with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panic {
    /// `apps/`'s own configuration (`panic = "abort"` in its profiles).
    Abort,
    /// `-Cpanic=unwind`, with std's `panic-unwind` feature.
    Unwind,
}

impl Panic {
    /// The cargo target directory, relative to `apps/`.
    fn target_dir(self) -> &'static str {
        match self {
            Panic::Abort => "target",
            Panic::Unwind => "target/unwind",
        }
    }
}

impl Arch {
    /// Whether this arch's programs can start copies of themselves (libtest's
    /// per-test processes, `current_exe` children): natively, or under qemu
    /// registered with binfmt_misc.
    pub fn can_spawn_self(self) -> bool {
        match self.runner() {
            Some(None) => true,
            Some(Some(_)) => Path::new("/proc/sys/fs/binfmt_misc/qemu-aarch64").exists(),
            None => false,
        }
    }
}

pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn apps() -> PathBuf {
    repo().join("apps")
}

/// `cargo` running the toolchain pinned in `rust-toolchain.toml`: through
/// rustup (`cargo +<pinned>`, since `apps/` selects another toolchain), or
/// the `cargo` on PATH when that is not a rustup proxy (e.g. under Nix).
pub(crate) fn pinned_cargo() -> Command {
    let mut cmd = Command::new("cargo");
    if cargo_is_rustup_proxy() {
        cmd.arg(format!("+{}", pinned_toolchain()));
    }
    cmd
}

fn cargo_is_rustup_proxy() -> bool {
    let Some(path) = std::env::var_os("PATH") else { return false };
    let Some(cargo) = std::env::split_paths(&path).map(|d| d.join("cargo")).find(|p| p.is_file()) else {
        return false;
    };
    // rustup's proxies are links (hard or symbolic) to rustup itself
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(&cargo), std::fs::metadata(cargo.with_file_name("rustup"))) {
        (Ok(a), Ok(b)) => (a.dev(), a.ino()) == (b.dev(), b.ino()),
        _ => false,
    }
}

/// The toolchain pinned in `rust-toolchain.toml`.
fn pinned_toolchain() -> String {
    let s = std::fs::read_to_string(repo().join("rust-toolchain.toml")).unwrap();
    s.lines()
        .find_map(|l| l.trim().strip_prefix("channel = \""))
        .and_then(|l| l.strip_suffix('"'))
        .expect("no channel in rust-toolchain.toml")
        .to_string()
}

/// A hash of the path, size and mtime of every source file std is built from.
fn sources_stamp() -> String {
    fn walk(p: &Path, h: &mut u64) {
        let Ok(md) = std::fs::metadata(p) else { return };
        if md.is_dir() {
            let mut entries: Vec<_> = std::fs::read_dir(p).unwrap().map(|e| e.unwrap().path()).collect();
            entries.sort();
            for e in entries {
                walk(&e, h);
            }
            return;
        }
        let mtime = md.modified().unwrap().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        for b in p.as_os_str().as_encoded_bytes().iter().chain(&md.len().to_le_bytes()).chain(&mtime.to_le_bytes()) {
            *h = (*h ^ *b as u64).wrapping_mul(0x100000001b3);
        }
    }
    let mut h: u64 = 0xcbf29ce484222325;
    let lib = xtask::library_dir();
    for p in [
        lib.join("Cargo.toml"),
        lib.join("std"),
        lib.join("unwind"),
        lib.join("backtrace/src"),
        lib.join("../libc/src"),
        repo().join("rt"),
    ] {
        if p.ends_with("rt") {
            for f in ["Cargo.toml", "build.rs", "src"] {
                walk(&p.join(f), &mut h);
            }
        } else {
            walk(&p, &mut h);
        }
    }
    format!("{h:016x}")
}

/// Build the patched std tree if needed, once per test process.
fn prepare() -> &'static str {
    static STAMP: OnceLock<String> = OnceLock::new();
    STAMP.get_or_init(|| {
        xtask::ensure_sysroot().expect("cargo xtask sysroot failed");
        sources_stamp()
    })
}

/// `cargo <args>` in `apps/` for `arch`, building std from the patched tree.
pub fn apps_cargo(arch: Arch, args: &[&str]) -> Command {
    apps_cargo_with(arch, Panic::Abort, args)
}

/// [`apps_cargo`] for the panic strategy `panic`.
///
/// The built-in `x86_64-unknown-linux-none` target (and our aarch64 spec,
/// copied from it) defaults to `panic=abort`. rustc accepts `-Cpanic=unwind`
/// for it, but cargo never passes that flag (it takes unwind to be every
/// target's default), so for unwinding it goes in the rustflags.
pub fn apps_cargo_with(arch: Arch, panic: Panic, args: &[&str]) -> Command {
    let stamp = prepare();
    let mut cmd = pinned_cargo();
    cmd.args(args)
        .current_dir(apps())
        .env("__CARGO_TESTS_ONLY_SRC_ROOT", xtask::library_dir())
        .args(["--target", &arch.target_arg()])
        .args(["--target-dir", panic.target_dir()]);
    let panic_flag = match panic {
        Panic::Abort => {
            cmd.arg("-Zbuild-std=std,panic_abort,test");
            ""
        }
        Panic::Unwind => {
            cmd.arg("-Zbuild-std=std,panic_abort,panic_unwind,test")
                .arg("-Zbuild-std-features=compiler-builtins-mem,backtrace,panic-unwind")
                .args(["--config", "profile.dev.panic=\"unwind\""]);
            "\"-Cpanic=unwind\", "
        }
    };
    cmd.arg("--config").arg(format!(
        "target.{}.rustflags=[{panic_flag}\"--cfg=linux_none_src=\\\"{stamp}\\\"\", \"--check-cfg=cfg(linux_none_src, values(any()))\"]",
        arch.name()
    ));
    if let Some(Some(r)) = arch.runner() {
        cmd.arg("--config").arg(format!("target.{}.runner=[\"{}\"]", arch.name(), r.display()));
    }
    cmd
}

/// One cargo build in `apps/` at a time: concurrent ones only wait for the
/// build directory lock (and print about it).
static BUILD: Mutex<()> = Mutex::new(());

/// `cargo build <args>` for `arch`; returns the debug output directory.
fn build(arch: Arch, panic: Panic, args: &[&str]) -> PathBuf {
    let _g = BUILD.lock().unwrap_or_else(|e| e.into_inner());
    let status = apps_cargo_with(arch, panic, &[&["build"], args].concat()).status().expect("failed to run cargo");
    assert!(status.success(), "cargo build {args:?} failed for {arch:?} ({panic:?})");
    apps().join(panic.target_dir()).join(arch.name()).join("debug")
}

/// Build the example programs for `arch` and `panic` once per test process
/// and return the directory holding them.
pub fn examples_dir(arch: Arch, panic: Panic) -> PathBuf {
    static DIRS: [[OnceLock<PathBuf>; 2]; 2] = [const { [const { OnceLock::new() }; 2] }; 2];
    DIRS[arch as usize][panic as usize].get_or_init(|| build(arch, panic, &["-p", "examples"])).clone()
}

/// Build the `fuzzee` binary (the linux-none side of the fuzzer) for `arch`.
pub fn fuzzee_bin(arch: Arch) -> PathBuf {
    build(arch, Panic::Abort, &["-p", "fuzzee"]).join("fuzzee")
}

/// The major version of the qemu at `qemu` ("qemu-aarch64 version 11.1.2").
pub fn qemu_major(qemu: &Path) -> Option<u32> {
    let out = Command::new(qemu).arg("--version").output().ok()?;
    let out = String::from_utf8_lossy(&out.stdout);
    out.split_once(" version ")?.1.split('.').next()?.parse().ok()
}

/// A qemu user-mode emulator for aarch64: `$QEMU_AARCH64`, or `qemu-aarch64`
/// on PATH.
pub fn qemu_aarch64() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("QEMU_AARCH64") {
        return Some(PathBuf::from(p));
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join("qemu-aarch64")).find(|p| p.is_file())
}

pub struct Run {
    pub output: Output,
}

impl Run {
    pub fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.output.stdout).into_owned()
    }
    pub fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.output.stderr).into_owned()
    }
    pub fn code(&self) -> Option<i32> {
        self.output.status.code()
    }
}

/// Run a binary from `dir`, natively or through `runner` (e.g. qemu).
pub fn run(dir: &Path, runner: Option<&Path>, name: &str, args: &[&str], stdin: &[u8], env: &[(&str, &str)]) -> Run {
    run_unset(dir, runner, name, args, stdin, env, &[])
}

/// [`run`], with the variables in `unset` removed from the environment.
pub fn run_unset(
    dir: &Path,
    runner: Option<&Path>,
    name: &str,
    args: &[&str],
    stdin: &[u8],
    env: &[(&str, &str)],
    unset: &[&str],
) -> Run {
    use std::io::Write;
    let exe = dir.join(name);
    // run via sh to disable core dumps: the panic example aborts on purpose,
    // and qemu would otherwise drop a core file in the working directory
    let mut cmd = Command::new("sh");
    cmd.args(["-c", "ulimit -c 0; exec \"$@\"", "sh"]);
    if let Some(r) = runner {
        cmd.arg(r);
    }
    cmd.arg(&exe);
    for var in unset {
        cmd.env_remove(var);
    }
    cmd.args(args).envs(env.iter().copied()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap_or_else(|e| panic!("failed to start {}: {e}", exe.display()));
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    Run { output: child.wait_with_output().unwrap() }
}

/// ELF checks: a static executable (ET_EXEC, no PT_INTERP / PT_DYNAMIC) for
/// `machine`, with no undefined symbols: nothing is left for a libc.
pub fn assert_static_elf(path: &Path, machine: u16) {
    let data = std::fs::read(path).unwrap();
    assert_eq!(&data[..4], b"\x7fELF", "{} is not ELF", path.display());
    assert_eq!(data[4], 2, "not ELF64");
    let u16_at = |o: usize| u16::from_le_bytes(data[o..o + 2].try_into().unwrap());
    let u32_at = |o: usize| u32::from_le_bytes(data[o..o + 4].try_into().unwrap());
    let u64_at = |o: usize| u64::from_le_bytes(data[o..o + 8].try_into().unwrap());
    assert_eq!(u16_at(16), 2, "{} is not ET_EXEC (PIE or shared object?)", path.display());
    assert_eq!(u16_at(18), machine, "wrong e_machine");
    let (phoff, phentsize, phnum) = (u64_at(32) as usize, u16_at(54) as usize, u16_at(56) as usize);
    for i in 0..phnum {
        let p_type = u32_at(phoff + i * phentsize);
        assert_ne!(p_type, 3, "{} has PT_INTERP (needs a dynamic loader)", path.display());
        assert_ne!(p_type, 2, "{} has PT_DYNAMIC (dynamically linked)", path.display());
    }

    // `nm -u`: symbols in .symtab with st_shndx == SHN_UNDEF
    const SHT_SYMTAB: u32 = 2;
    let (shoff, shentsize, shnum) = (u64_at(40) as usize, u16_at(58) as usize, u16_at(60) as usize);
    let section = |i: usize| shoff + i * shentsize;
    let mut undefined = Vec::new();
    for i in 0..shnum {
        let sh = section(i);
        if u32_at(sh + 4) != SHT_SYMTAB {
            continue;
        }
        let (off, size, entsize) = (u64_at(sh + 24) as usize, u64_at(sh + 32) as usize, u64_at(sh + 56) as usize);
        let strtab = u64_at(section(u32_at(sh + 40) as usize) + 24) as usize;
        for sym in (off..off + size).step_by(entsize).skip(1) {
            let (name, shndx) = (u32_at(sym) as usize, u16_at(sym + 6));
            if shndx == 0 && name != 0 {
                let end = data[strtab + name..].iter().position(|&c| c == 0).unwrap();
                undefined.push(String::from_utf8_lossy(&data[strtab + name..strtab + name + end]).into_owned());
            }
        }
    }
    assert!(undefined.is_empty(), "{} has undefined symbols: {undefined:?}", path.display());
}

pub const EM_X86_64: u16 = 62;
pub const EM_AARCH64: u16 = 183;
