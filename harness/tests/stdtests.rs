//! std's own integration tests (`library/std/tests` of the patched tree, the
//! `apps/stdtests` crate) under libtest, built for linux-none with
//! `-Zpanic-abort-tests`: each test runs in a subprocess of the test binary,
//! which also exercises process spawning. They run again built with
//! `panic=unwind`, where libtest runs each test on a thread and catches its
//! panic: nothing is skipped there.

use harness::*;

/// Tests skipped under `panic=abort`. Each fails the same way with std on
/// x86_64-unknown-linux-gnu under `-Zpanic-abort-tests`: they expect a panic
/// on another thread (or inside a `LazyLock` initializer) to be caught, which
/// `panic=abort` cannot do. They pass under `panic=unwind`.
const SKIP: &[&str] = &[
    "lazy_lock::lazy_lock_deref_mut_panic",
    "mpmc::oneshot_multi_task_recv_then_close",
    "mpmc::oneshot_multi_thread_send_close_stress",
    "mpmc::oneshot_single_thread_recv_chan_close",
    "mpsc::oneshot_multi_task_recv_then_close",
    "mpsc::oneshot_multi_thread_send_close_stress",
    "mpsc::oneshot_single_thread_recv_chan_close",
    "mpsc_sync::oneshot_multi_task_recv_then_close",
    "mpsc_sync::oneshot_multi_thread_send_close_stress",
    "mpsc_sync::oneshot_single_thread_recv_chan_close",
];

fn run(arch: Arch) {
    // libtest runs each test in a child process of the test binary
    if !arch.can_spawn_self() {
        eprintln!(
            "not running std's tests on {}: needs qemu-aarch64, registered with binfmt_misc",
            arch.name()
        );
        // but they must still build and link without libc
        let out = apps_cargo(arch, &["test", "-p", "stdtests", "-Zpanic-abort-tests", "--no-run"])
            .output()
            .expect("failed to run cargo test");
        assert!(out.status.success(), "std's tests failed to build:\n{}", String::from_utf8_lossy(&out.stderr));
        return;
    }
    let mut cmd = apps_cargo(arch, &["test", "-p", "stdtests", "-Zpanic-abort-tests", "--no-fail-fast"]);
    cmd.arg("--");
    for s in SKIP {
        cmd.args(["--skip", s]);
    }
    let out = cmd.output().expect("failed to run cargo test");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let passed: usize = stdout
        .lines()
        .filter_map(|l| l.strip_prefix("test result: ")?.split("; ").next()?.split(' ').nth(1)?.parse::<usize>().ok())
        .sum();
    eprintln!("std tests on {}: {passed} passed", arch.name());
    assert!(out.status.success(), "std's tests failed on {}:\n{stdout}\n{stderr}", arch.name());
    assert!(passed > 400, "suspiciously few tests passed: {passed}\n{stdout}");
}

/// Test targets that start copies of the test binary: they need
/// [`Arch::can_spawn_self`] even under `panic=unwind`.
const SPAWN_SELF: &[&str] = &["pipe-subprocess", "process_spawning"];

/// Too slow under qemu: takes 2^30 read locks.
const SLOW_UNDER_QEMU: &[&str] = &["rwlock::test_rwlock_max_readers"];

/// The `[[test]]` target names of `apps/stdtests`.
fn test_targets() -> Vec<String> {
    let toml = std::fs::read_to_string(repo().join("apps/stdtests/Cargo.toml")).unwrap();
    toml.split("[[test]]")
        .skip(1)
        .filter_map(|t| t.lines().find_map(|l| l.strip_prefix("name = \"")?.strip_suffix('"')).map(String::from))
        .collect()
}

fn run_unwind(arch: Arch) {
    if arch.runner().is_none() {
        eprintln!("not running std's tests on {}: set QEMU_AARCH64 or put qemu-aarch64 on PATH", arch.name());
        let out = apps_cargo_with(arch, Panic::Unwind, &["test", "-p", "stdtests", "--no-run"])
            .output()
            .expect("failed to run cargo test");
        assert!(out.status.success(), "std's tests failed to build:\n{}", String::from_utf8_lossy(&out.stderr));
        return;
    }
    let mut args = vec!["test".to_string(), "-p".into(), "stdtests".into(), "--no-fail-fast".into()];
    let emulated = !matches!(arch.runner(), Some(None));
    if !arch.can_spawn_self() {
        eprintln!("not running {SPAWN_SELF:?} on {}: they start copies of themselves", arch.name());
        for t in test_targets().into_iter().filter(|t| !SPAWN_SELF.contains(&t.as_str())) {
            args.extend(["--test".into(), t]);
        }
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut cmd = apps_cargo_with(arch, Panic::Unwind, &args);
    cmd.arg("--");
    if emulated {
        for s in SLOW_UNDER_QEMU {
            cmd.args(["--skip", s]);
        }
    }
    let out = cmd.output().expect("failed to run cargo test");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let passed: usize = stdout
        .lines()
        .filter_map(|l| l.strip_prefix("test result: ")?.split("; ").next()?.split(' ').nth(1)?.parse::<usize>().ok())
        .sum();
    eprintln!("std tests on {} (panic=unwind): {passed} passed", arch.name());
    assert!(out.status.success(), "std's tests failed on {} (panic=unwind):\n{stdout}\n{stderr}", arch.name());
    // the tests panic=abort has to skip catch their panics here
    for t in SKIP {
        assert!(
            stdout.lines().any(|l| l.starts_with(&format!("test {t} ")) && l.ends_with(" ok")),
            "{t} did not pass under panic=unwind:\n{stdout}"
        );
    }
    assert!(passed > 400, "suspiciously few tests passed: {passed}\n{stdout}");
}

#[test]
fn x86_64_std_tests() {
    run(Arch::X86_64);
}

#[test]
fn aarch64_std_tests() {
    run(Arch::Aarch64);
}

#[test]
fn x86_64_std_tests_unwind() {
    run_unwind(Arch::X86_64);
}

#[test]
fn aarch64_std_tests_unwind() {
    run_unwind(Arch::Aarch64);
}
