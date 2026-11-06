//! Helpers for building and running rust_std binaries (the examples and the
//! `stdtests` suite) from ordinary std-based tests. See `tests/`.

pub mod fuzz;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;

pub const AARCH64: &str = "aarch64-unknown-linux-gnu";

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

/// `cargo build --release <args> [--target T]` in the workspace; returns the
/// release output directory.
fn cargo_build(args: &[&str], target: Option<&str>) -> PathBuf {
    let root = workspace_root();
    let mut cmd = Command::new(env!("CARGO"));
    cmd.current_dir(&root).args(["build", "--release"]).args(args);
    if let Some(t) = target {
        cmd.args(["--target", t]);
    }
    let status = cmd.status().expect("failed to run cargo");
    assert!(status.success(), "cargo build {args:?} failed for {target:?}");
    let mut dir = root.join("target");
    if let Some(t) = target {
        dir.push(t);
    }
    dir.join("release")
}

/// Build all examples for `target` (None = host) once per test process and
/// return the directory holding them.
pub fn examples_dir(target: Option<&str>) -> PathBuf {
    static HOST: OnceLock<PathBuf> = OnceLock::new();
    static CROSS: OnceLock<PathBuf> = OnceLock::new();
    let cell = if target.is_some() { &CROSS } else { &HOST };
    cell.get_or_init(|| cargo_build(&["--examples", "-p", "rust_std"], target).join("examples")).clone()
}

/// Build the `fuzzee` binary (the rust_std side of the fuzzer) for `target`.
pub fn fuzzee_bin(target: Option<&str>) -> PathBuf {
    cargo_build(&["-p", "fuzzee"], target).join("fuzzee")
}

/// Build the `stdtests` binary (std's own tests on rust_std) for `target`.
pub fn stdtests_bin(target: Option<&str>) -> PathBuf {
    cargo_build(&["-p", "stdtests"], target).join("stdtests")
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

/// Whether the aarch64 target's standard library is installed.
pub fn aarch64_target_installed() -> bool {
    Command::new("rustc")
        .args(["--print", "target-libdir", "--target", AARCH64])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .is_some_and(|dir| Path::new(dir.trim()).is_dir())
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

/// Run an example binary, natively or through `runner` (e.g. qemu).
pub fn run(dir: &Path, runner: Option<&Path>, name: &str, args: &[&str], stdin: &[u8], env: &[(&str, &str)]) -> Run {
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
    cmd.args(args).envs(env.iter().copied()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap_or_else(|e| panic!("failed to start {}: {e}", exe.display()));
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    Run { output: child.wait_with_output().unwrap() }
}

/// Run `exe args...` (through `runner` if given) with core dumps disabled,
/// killing it after `timeout`. Returns None on timeout.
pub fn run_timeout(exe: &Path, runner: Option<&Path>, args: &[&str], cwd: &Path, timeout: std::time::Duration) -> Option<Run> {
    use std::io::Read;
    // core dumps are off unless STDTESTS_CORE_DIR asks for them (debugging)
    let core_dir = std::env::var_os("STDTESTS_CORE_DIR").map(PathBuf::from);
    let ulimit = if core_dir.is_some() { "ulimit -c unlimited" } else { "ulimit -c 0" };
    let mut cmd = Command::new("sh");
    cmd.args(["-c", &format!("{ulimit}; exec \"$@\""), "sh"]);
    if let Some(r) = runner {
        cmd.arg(r);
    }
    cmd.arg(exe).args(args).current_dir(core_dir.as_deref().unwrap_or(cwd));
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap_or_else(|e| panic!("failed to start {}: {e}", exe.display()));
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let out_t = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = out.read_to_end(&mut v);
        v
    });
    let err_t = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = err.read_to_end(&mut v);
        v
    });
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break Some(s);
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    let (stdout, stderr) = (out_t.join().unwrap(), err_t.join().unwrap());
    status.map(|status| Run { output: Output { status, stdout, stderr } })
}

/// ELF checks: statically linked (no PT_INTERP / PT_DYNAMIC), and the
/// expected machine type.
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
}

pub const EM_X86_64: u16 = 62;
pub const EM_AARCH64: u16 = 183;
