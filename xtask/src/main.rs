//! `cargo xtask sysroot`: assemble a std source tree with the linux-none
//! backend and register it as the `linux-none` rustup toolchain.
//!
//! Layout under `target/linux-none/`:
//! - `std-src/library`: the pinned nightly's `rust-src`, with `std-patches/`
//!   applied as git commits (tags `upstream` and `base` mark the unpatched
//!   tree, before and after the libc fixup).
//! - `std-src/libc`: std's libc version, patched to treat linux-none as musl.
//! - `std-src/linux_none_rt`: a symlink to `rt/`.
//! - `toolchain`: a hard-linked copy of the pinned toolchain whose `rust-src`
//!   is `std-src`. `-Zbuild-std` reads the std sources from the
//!   toolchain's sysroot, so this is how cargo finds the patched tree.
//!
//! `cargo xtask export-patches` writes the commits on top of `base` back to
//! `std-patches/`, so the patched tree can be edited in place.

use std::path::{Path, PathBuf};
use std::process::{Command, exit};
use std::{env, fs, io};

const TOOLCHAIN: &str = "linux-none";

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("sysroot") => sysroot(args.iter().any(|a| a == "--force")),
        Some("export-patches") => export_patches(),
        _ => {
            eprintln!("usage: cargo xtask sysroot [--force] | export-patches");
            exit(2)
        }
    };
    if let Err(e) = r {
        eprintln!("xtask: {e}");
        exit(1);
    }
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn out_dir() -> PathBuf {
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
fn pinned_sysroot() -> io::Result<PathBuf> {
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

fn sysroot(force: bool) -> io::Result<()> {
    let sysroot = pinned_sysroot()?;
    let library = sysroot.join("lib/rustlib/src/rust/library");
    if !library.join("std/Cargo.toml").exists() {
        return Err(err(format!("{} has no rust-src; run `rustup component add rust-src`", sysroot.display())));
    }
    let out = out_dir();
    let src = out.join("std-src");

    if src.exists() {
        fs::remove_dir_all(&src)?;
    }
    fs::create_dir_all(&src)?;
    eprintln!("xtask: copying {}", library.display());
    copy_tree(&library, &src.join("library"), false)?;
    let libc = libc_dir(&library)?;
    copy_tree(&libc, &src.join("libc"), false)?;
    let _ = fs::remove_file(src.join("libc/.cargo-checksum.json"));
    std::os::unix::fs::symlink(repo().join("rt"), src.join("linux_none_rt"))?;
    fs::write(src.join(".gitignore"), "/linux_none_rt\n")?;

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

    toolchain(&sysroot, &out, force)?;
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

/// libc has no items for linux with an empty `target_env`. std only needs its
/// types and constants (for `os::unix` and friends), and musl's match the
/// kernel's, so select musl's definitions for linux-none too.
fn libc_as_musl(dir: &Path) -> io::Result<()> {
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
    // ...except for linking: there is no libc to link
    let f = dir.join("src/unix/mod.rs");
    let s = fs::read_to_string(&f)?;
    let edits = [
        (format!("all({TO}, not(libc_pauthtest))"), format!("all({FROM}, not(libc_pauthtest))")),
        (
            "    } else if #[cfg(target_os = \"emscripten\")] {".to_string(),
            "    } else if #[cfg(all(target_os = \"linux\", target_env = \"\"))] {\n        \
             // linux-none: no libc to link\n    \
             } else if #[cfg(target_os = \"emscripten\")] {"
                .to_string(),
        ),
    ];
    let mut s = s;
    for (from, to) in edits {
        if s.matches(&from).count() != 1 {
            return Err(err(format!("libc fixup: `{from}` not found exactly once in {}", f.display())));
        }
        s = s.replace(&from, &to);
    }
    fs::write(&f, s)?;
    Ok(())
}

/// Hard-link the pinned toolchain into `out/toolchain`, with `rust-src`
/// pointing at the patched library, and register it with rustup.
fn toolchain(sysroot: &Path, out: &Path, force: bool) -> io::Result<()> {
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
    // (`../libc`, `../linux_none_rt`) resolve inside this directory: link it
    // as a whole
    let src = tc.join("lib/rustlib/src");
    if src.exists() {
        fs::remove_dir_all(&src)?;
    }
    fs::create_dir_all(&src)?;
    std::os::unix::fs::symlink(out.join("std-src"), src.join("rust"))?;
    run(Command::new("rustup").args(["toolchain", "link", TOOLCHAIN]).arg(&tc))?;
    Ok(())
}

/// Copy `from` to `to`, hard-linking regular files when `link` is set.
/// The toolchain's own `rust-src` is skipped when linking.
fn copy_tree(from: &Path, to: &Path, link: bool) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let (src, dst) = (e.path(), to.join(e.file_name()));
        let ty = e.file_type()?;
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

fn export_patches() -> io::Result<()> {
    let src = out_dir().join("std-src");
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
