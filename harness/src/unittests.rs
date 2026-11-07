//! std's in-crate unit tests (`library/std/src/**/tests.rs`), built the way
//! `x.py test library/std` builds them: in the patched library workspace,
//! with std itself as the test crate.
//!
//! The test crate links against a std from the sysroot (`extern crate std as
//! realstd`: libtest, the panic handler, the `io::Error` methods and other
//! lang items and globals come from there), so it is a two-step build, as in
//! bootstrap:
//!
//! 1. `cargo build -p sysroot` in `library/`, whose rlibs make up a sysroot
//!    for the target.
//! 2. `cargo test -p std --lib` with `--sysroot` pointing at it.
//!
//! Both steps share one target directory and identical flags, so the crates
//! below std (core, alloc, linux_none_rt, ...) are the same build in both:
//! rustc then treats the test crate's copy and the sysroot's as one crate
//! (same `StableCrateId` and hash). The flags are bootstrap's for std
//! (`-Zforce-unstable-if-unmarked`, so that std's unstable dependencies can
//! be used) plus the static, non-PIE link of `apps/`. `RUSTC_BOOTSTRAP` is
//! not needed: the pinned toolchain is a nightly. Unlike bootstrap's,
//! everything is built with `panic=abort` and the tests run under
//! `-Zpanic-abort-tests`, one child process per test.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

use crate::{Arch, pinned_toolchain, prepare, repo};

/// Where both steps build, and where the sysroots go.
fn target_dir() -> PathBuf {
    repo().join("target/linux-none/unittests")
}

fn sysroot(arch: Arch) -> PathBuf {
    target_dir().join(format!("sysroot-{}", arch.name()))
}

/// `cargo <args>` in the patched `library/` workspace for `arch`.
fn library_cargo(arch: Arch, args: &[&str]) -> Command {
    prepare();
    let sysroot = sysroot(arch);
    let flags = [
        "-Zforce-unstable-if-unmarked",
        "-Ctarget-feature=+crt-static",
        "-Crelocation-model=static",
        "-Clink-arg=-no-pie",
        "--sysroot",
        sysroot.to_str().unwrap(),
    ];
    let mut cmd = Command::new("cargo");
    cmd.arg(format!("+{}", pinned_toolchain()))
        .args(args)
        .current_dir(xtask::library_dir())
        .env_remove("RUSTFLAGS")
        .env("CARGO_ENCODED_RUSTFLAGS", flags.join("\x1f"))
        .args(["--target", &arch.target_arg(), "--target-dir"])
        .arg(target_dir())
        .args(["-Zjson-target-spec", "--config", "profile.dev.panic=\"abort\""])
        .args(["--no-default-features", "--features", "compiler-builtins-mem"]);
    cmd
}

/// The string values of `"key":[...]` in a line of cargo's JSON output (paths
/// do not contain quotes here).
fn json_strings<'a>(line: &'a str, key: &str) -> Vec<&'a str> {
    let Some(start) = line.find(&format!("\"{key}\":[")) else { return Vec::new() };
    let rest = &line[start + key.len() + 4..];
    let list = &rest[..rest.find(']').unwrap()];
    list.split(',').filter_map(|s| s.strip_prefix('"')?.strip_suffix('"')).collect()
}

/// The string value of `"key":"..."` in a line of cargo's JSON output.
fn json_string<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let start = line.find(&format!("\"{key}\":\""))? + key.len() + 4;
    Some(&line[start..start + line[start..].find('"')?])
}

/// Run cargo with `--message-format=json-render-diagnostics`, returning the
/// JSON lines (diagnostics go to stderr, rendered).
fn cargo_json(mut cmd: Command, what: &str) -> Vec<String> {
    let out = cmd.arg("--message-format=json-render-diagnostics").stderr(Stdio::inherit()).output().unwrap();
    assert!(out.status.success(), "{what} failed");
    String::from_utf8(out.stdout).unwrap().lines().map(String::from).collect()
}

/// Step 1: build std and its siblings in the library workspace and gather
/// their rlibs (with the `.rmeta` files that carry their full metadata) into
/// a fresh sysroot.
fn build_sysroot(arch: Arch) {
    // (built without one, so that no stale crate can be picked up from it)
    let _ = std::fs::remove_dir_all(sysroot(arch));
    let cmd = library_cargo(arch, &["build", "-p", "sysroot"]);
    let lines = cargo_json(cmd, "building the sysroot");
    let lib = sysroot(arch).join("lib/rustlib").join(arch.name()).join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    for l in lines.iter().filter(|l| l.contains("\"reason\":\"compiler-artifact\"")) {
        for f in json_strings(l, "filenames").into_iter().map(Path::new) {
            let name = f.file_name().unwrap();
            let keep = f.extension().is_some_and(|e| e == "rlib" || e == "rmeta");
            if keep && !name.to_string_lossy().starts_with("libsysroot") {
                std::fs::hard_link(f, lib.join(name)).or_else(|_| std::fs::copy(f, lib.join(name)).map(drop)).unwrap();
            }
        }
    }
    assert!(lib.join("libstd.rlib").exists(), "no libstd.rlib among the sysroot build's artifacts");
}

/// One build of the unit tests at a time (cargo would only wait for the
/// target directory's lock).
static BUILD: Mutex<()> = Mutex::new(());

/// Steps 1 and 2: build the unit test binary of std for `arch` and return
/// its path.
pub fn std_unittests_bin(arch: Arch) -> PathBuf {
    let _g = BUILD.lock().unwrap_or_else(|e| e.into_inner());
    build_sysroot(arch);
    let cmd = library_cargo(arch, &["test", "-p", "std", "--lib", "-Zpanic-abort-tests", "--no-run"]);
    let lines = cargo_json(cmd, "building std's unit tests");
    let exe = lines.iter().filter(|l| l.contains("\"reason\":\"compiler-artifact\"")).find_map(|l| json_string(l, "executable"));
    PathBuf::from(exe.expect("no test executable in cargo's output"))
}

/// The directory std's tests run in, as under `cargo test` (some read files
/// relative to it).
pub fn std_dir() -> PathBuf {
    xtask::library_dir().join("std")
}
