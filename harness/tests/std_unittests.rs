//! std's in-crate unit tests (`library/std/src/**/tests.rs` of the patched
//! tree), built as `x.py test library/std` builds them (see
//! `harness::unittests`) and run under libtest with `-Zpanic-abort-tests`.
//!
//! Set `SKIP_STD_UNITTESTS=1` to leave them out: the two extra std builds per
//! architecture take a few minutes.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use harness::unittests::{std_dir, std_unittests_bin};
use harness::*;

/// Tests that fail on linux-none for a reason that is not a bug in it: each
/// fails the same way with std on x86_64-unknown-linux-gnu, built and run
/// like this under `panic=abort`. (Tests that need unwinding are mostly
/// `#[ignore]`d upstream under `panic=abort` already.)
const SKIP_PANIC_ABORT: &[(&str, &str)] = &[(
    "io::stdio::tests::panic_doesnt_poison",
    "panics on another thread while holding stdout's lock and expects to continue",
)];

/// Tests that fail because of something linux-none lacks. None do now: the
/// gaps they found are fixed in std-patches (errno shared by both copies of
/// std, signal names in `ExitStatus`'s `Display`) or the tests were ported
/// off libc calls. Tests that do not exist on linux-none are glibc's or
/// libc's own (`conf::glibc_version`, `weak`, `thread_local::key`, the
/// unsupported-wait-status emulation).
const SKIP_LINUX_NONE: &[(&str, &str)] = &[];

/// Tests that cannot pass under qemu without binfmt_misc, where each test is
/// started by the harness (see [`run_each`]) rather than by libtest.
const SKIP_QEMU_NO_BINFMT: &[(&str, &str)] = &[];

fn skipped() -> impl Iterator<Item = &'static str> {
    SKIP_PANIC_ABORT.iter().chain(SKIP_LINUX_NONE).map(|(name, _)| *name)
}

/// `sh -c 'ulimit -c 0; exec ...'`: no core files from aborting tests (qemu
/// would write them to the working directory).
fn no_core(runner: Option<&Path>, bin: &Path) -> Command {
    let mut cmd = Command::new("sh");
    cmd.args(["-c", "ulimit -c 0; exec \"$@\"", "sh"]);
    cmd.args(runner).arg(bin).current_dir(std_dir());
    cmd
}

/// The names of the tests `bin` holds (`--list`), ignored ones with
/// `--ignored`.
fn list(runner: Option<&Path>, bin: &Path, ignored: bool) -> Vec<String> {
    let mut cmd = no_core(runner, bin);
    cmd.arg("--list");
    if ignored {
        cmd.arg("--ignored");
    }
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "--list failed: {}", String::from_utf8_lossy(&out.stderr));
    let out = String::from_utf8(out.stdout).unwrap();
    out.lines().filter_map(|l| l.strip_suffix(": test")).map(String::from).collect()
}

/// Run every test of `bin` the way libtest's parent process starts each in a
/// child under `-Zpanic-abort-tests`: with the test's name in
/// `__RUST_TEST_INVOKE`, expecting exit status 50 for a pass. Used under qemu
/// without binfmt_misc, where libtest could not start the children itself.
fn run_each(arch: Arch, qemu: &Path, bin: &Path) {
    let ignored = list(Some(qemu), bin, true);
    let skip: Vec<&str> = skipped().chain(SKIP_QEMU_NO_BINFMT.iter().map(|(n, _)| *n)).collect();
    let todo: Vec<String> =
        list(Some(qemu), bin, false).into_iter().filter(|t| !ignored.contains(t) && !skip.contains(&t.as_str())).collect();
    let next = Mutex::new(todo.iter());
    let failed = Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                while let Some(test) = { next.lock().unwrap().next() } {
                    let out = no_core(Some(qemu), bin)
                        .env("__RUST_TEST_INVOKE", test)
                        .stdin(Stdio::null())
                        .output()
                        .unwrap();
                    if out.status.code() != Some(50) {
                        let msg = format!(
                            "---- {test} ({})\n{}{}",
                            out.status,
                            String::from_utf8_lossy(&out.stdout),
                            String::from_utf8_lossy(&out.stderr)
                        );
                        failed.lock().unwrap().push(msg);
                    }
                }
            });
        }
    });
    let failed = failed.into_inner().unwrap();
    let passed = todo.len() - failed.len();
    eprintln!(
        "std unit tests on {} (each started by the harness under qemu): {passed} passed, {} failed, {} ignored, {} skipped",
        arch.name(),
        failed.len(),
        ignored.len(),
        skip.len()
    );
    assert!(failed.is_empty(), "std's unit tests failed on {}:\n{}", arch.name(), failed.join("\n"));
    assert!(passed > 400, "suspiciously few tests passed: {passed}");
}

/// Run `bin` under libtest, which starts a child process per test.
fn run_libtest(arch: Arch, runner: Option<&Path>, bin: &Path) {
    let mut cmd = no_core(runner, bin);
    cmd.arg("--exact");
    for s in skipped() {
        cmd.args(["--skip", s]);
    }
    let out = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let passed: usize = stdout
        .lines()
        .filter_map(|l| l.strip_prefix("test result: ")?.split("; ").next()?.split(' ').nth(1)?.parse::<usize>().ok())
        .sum();
    eprintln!("std unit tests on {}: {passed} passed", arch.name());
    assert!(out.status.success(), "std's unit tests failed on {}:\n{stdout}\n{stderr}", arch.name());
    assert!(passed > 400, "suspiciously few tests passed: {passed}\n{stdout}");
}

fn run(arch: Arch) {
    if std::env::var_os("SKIP_STD_UNITTESTS").is_some_and(|v| v != "0") {
        eprintln!("SKIP_STD_UNITTESTS is set: not building std's unit tests");
        return;
    }
    let bin = std_unittests_bin(arch);
    // linked without libc, like everything else
    assert_static_elf(&bin, arch.machine());
    match arch.runner() {
        None => eprintln!("std's unit tests on {}: built only (set QEMU_AARCH64 to run them)", arch.name()),
        Some(runner) if arch.can_spawn_self() => run_libtest(arch, runner.as_deref(), &bin),
        Some(Some(qemu)) => run_each(arch, &qemu, &bin),
        Some(None) => unreachable!("native programs can start themselves"),
    }
}

#[test]
fn x86_64_std_unittests() {
    run(Arch::X86_64);
}

#[test]
fn aarch64_std_unittests() {
    run(Arch::Aarch64);
}
