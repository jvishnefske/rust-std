//! `cargo xtask sysroot`: assemble a std source tree with the linux-none
//! backend and register it as the `linux-none` rustup toolchain.
//!
//! Layout under `target/linux-none/`:
//! - `std-src/library`: the pinned nightly's `rust-src`, with `std-patches/`
//!   applied as git commits (tags `upstream` and `base` mark the unpatched
//!   tree, before and after the libc fixup).
//! - `std-src/libc`: std's libc version, patched to treat linux-none as musl.
//! - `std-src/library/linux_none_rt`: a symlink to `rt/`.
//! - `toolchain`: a hard-linked copy of the pinned toolchain whose `rust-src`
//!   is `std-src`. `-Zbuild-std` reads the std sources from the
//!   toolchain's sysroot, so this is how cargo finds the patched tree.
//!
//! `cargo xtask sysroot --out DIR --no-toolchain` builds a standalone copy of
//! `std-src` in DIR (rt copied in, no rustup), for builds without rustup
//! such as the Nix flake; `$LINUX_NONE_STD_SRC` points the harness at it.
//!
//! `cargo xtask libc-fixup DIR` applies the libc fixup to the libc crate in
//! DIR, for crates outside std that depend on another libc version;
//! `cargo xtask musl-cfg DIR` applies its cfg part to any crate (`nix`,
//! `socket2`, ...) that picks libc's musl definitions by `target_env`.
//!
//! `cargo xtask export-patches` writes the commits on top of `base` back to
//! `std-patches/`, so the patched tree can be edited in place.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs, io};

const TOOLCHAIN: &str = "linux-none";

/// The repository root.
pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

/// `target/linux-none`.
pub fn out_dir() -> PathBuf {
    repo().join("target/linux-none")
}

fn err(msg: impl Into<String>) -> io::Error {
    io::Error::other(msg.into())
}

fn run(cmd: &mut Command) -> io::Result<String> {
    let o = cmd.output()?;
    if !o.status.success() {
        return Err(err(format!("{cmd:?} failed:\n{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))));
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn git(dir: &Path) -> Command {
    let mut c = Command::new("git");
    c.current_dir(dir)
        .args(["-c", "user.name=xtask", "-c", "user.email=xtask@localhost", "-c", "commit.gpgsign=false"])
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z");
    c
}

/// The sysroot of the toolchain pinned in `rust-toolchain.toml`.
pub fn pinned_sysroot() -> io::Result<PathBuf> {
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let s = run(Command::new(rustc).current_dir(repo()).args(["--print", "sysroot"]))?;
    Ok(PathBuf::from(s))
}

fn patches() -> io::Result<Vec<PathBuf>> {
    let mut v: Vec<PathBuf> = fs::read_dir(repo().join("std-patches"))?
        .map(|e| e.map(|e| e.path()))
        .collect::<io::Result<_>>()?;
    v.retain(|p| p.extension().is_some_and(|e| e == "patch"));
    v.sort();
    Ok(v)
}

/// The patched `library/` directory (`__CARGO_TESTS_ONLY_SRC_ROOT` for
/// builds that must notice changes to it, see README.md).
pub fn library_dir() -> PathBuf {
    std_src().join("library")
}

/// A hash of what the patched tree is made from: the pinned sysroot, the
/// patch series and this file (which patches libc).
pub fn inputs_hash() -> io::Result<String> {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut eat = |b: &[u8]| {
        for &x in b {
            h = (h ^ x as u64).wrapping_mul(0x100000001b3);
        }
    };
    eat(pinned_sysroot()?.to_string_lossy().as_bytes());
    eat(include_bytes!("lib.rs"));
    for p in patches()? {
        eat(&fs::read(p)?);
    }
    Ok(format!("{h:016x}"))
}

fn stamp() -> PathBuf {
    std_src().join(".inputs-hash")
}

/// The patched tree: `$LINUX_NONE_STD_SRC`, else `target/linux-none/std-src`.
pub fn std_src() -> PathBuf {
    env::var_os("LINUX_NONE_STD_SRC").map(PathBuf::from).unwrap_or_else(|| out_dir().join("std-src"))
}

/// Build the patched tree unless it is already built from the current
/// inputs. (Rebuilding discards edits made in it: export them first.)
pub fn ensure_sysroot() -> io::Result<()> {
    let hash = inputs_hash()?;
    if fs::read_to_string(stamp()).is_ok_and(|s| s == hash) {
        return Ok(());
    }
    sysroot(&Options::default())
}

#[derive(Default)]
pub struct Options {
    /// Rebuild the toolchain even if it is up to date.
    pub force: bool,
    /// Write a standalone tree here (rt copied, not linked) instead of
    /// `std_src()`.
    pub out: Option<PathBuf>,
    /// Skip the `linux-none` rustup toolchain.
    pub no_toolchain: bool,
}

/// Build the patched tree and the `linux-none` toolchain.
pub fn sysroot(opts: &Options) -> io::Result<()> {
    let sysroot = pinned_sysroot()?;
    let library = sysroot.join("lib/rustlib/src/rust/library");
    if !library.join("std/Cargo.toml").exists() {
        return Err(err(format!("{} has no rust-src; run `rustup component add rust-src`", sysroot.display())));
    }
    let out = out_dir();
    let src = opts.out.clone().unwrap_or_else(std_src);

    if src.exists() {
        fs::remove_dir_all(&src)?;
    }
    fs::create_dir_all(&src)?;
    eprintln!("xtask: copying {}", library.display());
    copy_tree(&library, &src.join("library"), false)?;
    let libc = libc_dir(&library)?;
    copy_tree(&libc, &src.join("libc"), false)?;
    let _ = fs::remove_file(src.join("libc/.cargo-checksum.json"));
    // rt is a member of the library workspace (std-patches adds it)
    let rt = src.join("library/linux_none_rt");
    if opts.out.is_some() {
        fs::create_dir_all(rt.join("src"))?;
        copy_tree(&repo().join("rt/src"), &rt.join("src"), false)?;
        for f in ["Cargo.toml", "build.rs"] {
            fs::copy(repo().join("rt").join(f), rt.join(f))?;
        }
    } else {
        std::os::unix::fs::symlink(repo().join("rt"), &rt)?;
    }
    fs::write(src.join(".gitignore"), "/library/linux_none_rt\n/.inputs-hash\n")?;

    run(git(&src).args(["init", "-q"]))?;
    run(git(&src).args(["add", "-A"]))?;
    run(git(&src).args(["commit", "-q", "-m", "upstream"]))?;
    run(git(&src).args(["tag", "upstream"]))?;
    libc_as_musl(&src.join("libc"))?;
    run(git(&src).args(["commit", "-q", "-a", "-m", "libc: treat linux with an empty target_env as musl"]))?;
    run(git(&src).args(["tag", "base"]))?;
    for p in patches()? {
        eprintln!("xtask: applying {}", p.file_name().unwrap().to_string_lossy());
        run(git(&src).args(["am", "-q", "--keep-cr"]).arg(&p))?;
    }

    fs::write(src.join(".inputs-hash"), inputs_hash()?)?;
    if opts.no_toolchain {
        eprintln!("xtask: patched tree is ready in {}", src.display());
        return Ok(());
    }
    toolchain(&sysroot, &out, &src, opts.force)?;
    eprintln!("xtask: toolchain `{TOOLCHAIN}` is ready; build with `cargo +{TOOLCHAIN} build --target x86_64-unknown-linux-none`");
    Ok(())
}

/// The vendored libc crate std depends on.
fn libc_dir(library: &Path) -> io::Result<PathBuf> {
    let lock = fs::read_to_string(library.join("Cargo.lock"))?;
    let version = lock
        .split("[[package]]")
        .find(|p| p.contains("name = \"libc\""))
        .and_then(|p| p.lines().find_map(|l| l.strip_prefix("version = \"")))
        .and_then(|v| v.strip_suffix('"'))
        .ok_or_else(|| err("libc not found in library/Cargo.lock"))?;
    let dir = library.join(format!("vendor/libc-{version}"));
    if !dir.exists() {
        return Err(err(format!("{} is missing", dir.display())));
    }
    Ok(dir)
}

/// Make the `target_env = "musl"` cfgs of the crate in `dir` hold on
/// linux-none too. For libc (see [`libc_as_musl`]) and for crates that
/// choose between libc's glibc and musl definitions, such as `nix` and
/// `socket2`: on linux-none libc has musl's.
pub fn musl_cfg(dir: &Path) -> io::Result<()> {
    const FROM: &str = "target_env = \"musl\"";
    const TO: &str = "any(target_env = \"musl\", all(target_os = \"linux\", target_env = \"\"))";
    for f in walk(&dir.join("src"))? {
        if f.extension().is_some_and(|e| e == "rs") {
            let s = fs::read_to_string(&f)?;
            if s.contains(FROM) {
                fs::write(&f, s.replace(FROM, TO))?;
            }
        }
    }
    Ok(())
}

/// libc has no items for linux with an empty `target_env`. std only needs its
/// types and constants (for `os::unix` and friends), and musl's match the
/// kernel's, so select musl's definitions for linux-none too.
pub fn libc_as_musl(dir: &Path) -> io::Result<()> {
    musl_cfg(dir)?;
    // ...except for linking: there is no libc to link. The arm goes first in
    // the `cfg_if!` that links libc, whose arm order varies between versions.
    let f = dir.join("src/unix/mod.rs");
    let mut s = fs::read_to_string(&f)?;
    const ARMS: &str = "cfg_if! {\n    if #[cfg(";
    let at = s
        .find("name = \"c\"")
        .and_then(|link| s[..link].rfind(ARMS))
        .ok_or_else(|| err(format!("libc fixup: the libc link `cfg_if!` not found in {}", f.display())))?;
    s.insert_str(
        at + "cfg_if! {\n    ".len(),
        "if #[cfg(all(target_os = \"linux\", target_env = \"\"))] {\n        \
         // linux-none: no libc to link\n    \
         } else ",
    );
    fs::write(&f, s)?;
    Ok(())
}

/// Hard-link the pinned toolchain into `out/toolchain`, with `rust-src`
/// pointing at the patched library, and register it with rustup.
fn toolchain(sysroot: &Path, out: &Path, std_src: &Path, force: bool) -> io::Result<()> {
    let tc = out.join("toolchain");
    let stamp = tc.join(".pinned-sysroot");
    let fresh = fs::read_to_string(&stamp).is_ok_and(|s| s == sysroot.to_string_lossy());
    if force || !fresh {
        if tc.exists() {
            fs::remove_dir_all(&tc)?;
        }
        eprintln!("xtask: linking toolchain from {}", sysroot.display());
        copy_tree(sysroot, &tc, true)?;
        fs::write(&stamp, sysroot.to_string_lossy().as_bytes())?;
    }
    // cargo does not canonicalize the source path, so std's relative paths
    // (`../libc`) resolve inside this directory: link it as a whole
    let src = tc.join("lib/rustlib/src");
    if src.exists() {
        fs::remove_dir_all(&src)?;
    }
    fs::create_dir_all(&src)?;
    std::os::unix::fs::symlink(std_src, src.join("rust"))?;
    if Command::new("rustup").arg("--version").output().is_err() {
        eprintln!("xtask: no rustup; not registering `{TOOLCHAIN}` (use {} as a sysroot)", tc.display());
        return Ok(());
    }
    run(Command::new("rustup").args(["toolchain", "link", TOOLCHAIN]).arg(&tc))?;
    Ok(())
}

/// Copy `from` to `to`, hard-linking regular files (and keeping symlinks)
/// when `link` is set.
/// The toolchain's own `rust-src` is skipped when linking.
fn copy_tree(from: &Path, to: &Path, link: bool) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let (src, dst) = (e.path(), to.join(e.file_name()));
        // a copied source tree gets real files, also where the source is a
        // symlink farm (rust-overlay's toolchains in the Nix store)
        let ty = if link { e.file_type()? } else { fs::metadata(&src)?.file_type() };
        if link && src.ends_with("lib/rustlib/src") {
            continue;
        }
        if ty.is_dir() {
            copy_tree(&src, &dst, link)?;
        } else if ty.is_symlink() {
            std::os::unix::fs::symlink(fs::read_link(&src)?, &dst)?;
        } else if link {
            fs::hard_link(&src, &dst).or_else(|_| fs::copy(&src, &dst).map(drop))?;
        } else {
            fs::copy(&src, &dst)?;
            // a read-only source (the Nix store) stays read-only otherwise
            let mode = fs::metadata(&dst)?.permissions().mode();
            fs::set_permissions(&dst, fs::Permissions::from_mode(mode | 0o200))?;
        }
    }
    Ok(())
}

fn walk(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut v = Vec::new();
    for e in fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            v.extend(walk(&p)?);
        } else {
            v.push(p);
        }
    }
    Ok(v)
}

/// Write the commits on top of `base` in the patched tree to `std-patches/`.
pub fn export_patches() -> io::Result<()> {
    let src = std_src();
    let dir = repo().join("std-patches");
    for p in patches()? {
        fs::remove_file(p)?;
    }
    run(git(&src)
        .args(["format-patch", "-q", "--no-signature", "--zero-commit", "--no-numbered", "-o"])
        .arg(&dir)
        .arg("base..HEAD"))?;
    for p in patches()? {
        eprintln!("xtask: wrote {}", p.strip_prefix(repo()).unwrap().display());
    }
    Ok(())
}
