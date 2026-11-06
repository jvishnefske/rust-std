//! std's own integration tests (`library/std/tests` of the patched tree, the
//! `apps/stdtests` crate) under libtest, built for linux-none with
//! `-Zpanic-abort-tests`: each test runs in a subprocess of the test binary,
//! which also exercises process spawning.

use harness::*;

/// Tests that fail on linux-none for a reason that is not a bug in it. Each
/// fails the same way with std on x86_64-unknown-linux-gnu under
/// `-Zpanic-abort-tests`: they expect a panic on another thread (or inside a
/// `LazyLock` initializer) to be caught, which `panic=abort` cannot do.
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

#[test]
fn x86_64_std_tests() {
    run(Arch::X86_64);
}

#[test]
fn aarch64_std_tests() {
    run(Arch::Aarch64);
}
