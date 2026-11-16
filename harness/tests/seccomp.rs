//! rt's opt-in seccomp filter (rt/src/seccomp.rs): the `sandbox` example,
//! built with an allowlist that leaves out `openat`, cannot open a file.
//! With `action: errno` the open fails with EPERM; with `action: kill` the
//! process dies of SIGSYS.
//!
//! Runs natively only: qemu-user does not emulate seccomp for its guest.

use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Command;

use harness::*;

const SIGSYS: i32 = 31;

fn allowlist(action: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/data/sandbox-{action}.seccomp"))
}

fn run(arch: Arch, action: &str) -> Option<std::process::Output> {
    let bin = seccomp_example(arch, "sandbox", &allowlist(action));
    assert_static_elf(&bin, arch.machine());
    if arch.runner() != Some(None) {
        eprintln!("skipping {} run: seccomp needs a native {} host", arch.name(), arch.name());
        return None;
    }
    Some(Command::new(&bin).output().unwrap())
}

fn check(arch: Arch) {
    if let Some(o) = run(arch, "errno") {
        let out = String::from_utf8_lossy(&o.stdout);
        assert!(o.status.success(), "{:?}\n{out}{}", o.status, String::from_utf8_lossy(&o.stderr));
        assert_eq!(out, "before open\nopen: PermissionDenied errno=1\n");
    }
    if let Some(o) = run(arch, "kill") {
        let out = String::from_utf8_lossy(&o.stdout);
        assert_eq!(o.status.signal(), Some(SIGSYS), "{:?}\n{out}", o.status);
        assert_eq!(out, "before open\n");
    }
}

#[test]
fn x86_64_seccomp() {
    check(Arch::X86_64);
}

#[test]
fn aarch64_seccomp() {
    check(Arch::Aarch64);
}
