//! Run std's own integration tests (library/std/tests, adapted by the
//! `stdtests` crate) against rust_std: on the host, and on aarch64 under qemu
//! when available. Each test runs in its own process, since a panic aborts.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use harness::*;

struct Case {
    name: String,
    should_panic: Option<String>,
    ignore: bool,
}

fn list(bin: &Path, runner: Option<&Path>) -> Vec<Case> {
    let r = run_timeout(bin, runner, &["--list"], &std::env::temp_dir(), Duration::from_secs(120)).expect("--list timed out");
    assert_eq!(r.code(), Some(0), "--list failed: {}", r.stderr());
    r.stdout()
        .lines()
        .map(|l| {
            let f: Vec<&str> = l.splitn(4, '\t').collect();
            Case {
                name: f[0].to_string(),
                should_panic: (f[1] == "1").then(|| f[3].to_string()),
                ignore: f[2] == "1",
            }
        })
        .collect()
}

/// Run every test; returns (passed, failures).
fn run_all(bin: &Path, runner: Option<&Path>, timeout: Duration) -> (usize, Vec<String>) {
    let cases = list(bin, runner);
    assert!(cases.len() > 200, "suspiciously few tests: {}", cases.len());
    let next = AtomicUsize::new(0);
    let passed = AtomicUsize::new(0);
    let failures = Mutex::new(Vec::new());
    // tests use temporary dirs and relative paths: run them somewhere scratch
    let cwd: PathBuf = std::env::temp_dir();
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(case) = cases.get(i) else { break };
                    if case.ignore {
                        continue;
                    }
                    let result = run_timeout(bin, runner, &[&case.name], &cwd, timeout);
                    let verdict = match (result, &case.should_panic) {
                        (None, _) => Err("timed out".to_string()),
                        (Some(r), None) if r.code() == Some(0) => Ok(()),
                        (Some(r), None) => Err(format!("status {:?}\n{}", r.output.status, r.stderr())),
                        (Some(r), Some(msg)) => {
                            if r.code() != Some(0) && r.stderr().contains(msg.as_str()) {
                                Ok(())
                            } else {
                                Err(format!("expected a panic containing {msg:?}; status {:?}\n{}", r.output.status, r.stderr()))
                            }
                        }
                    };
                    match verdict {
                        Ok(()) => {
                            passed.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) => failures.lock().unwrap().push(format!("{}: {e}", case.name)),
                    }
                }
            });
        }
    });
    (passed.into_inner(), failures.into_inner().unwrap())
}

fn check(arch: &str, bin: &Path, runner: Option<&Path>, timeout: Duration) {
    let (passed, failures) = run_all(bin, runner, timeout);
    eprintln!("stdtests ({arch}): {passed} passed, {} failed", failures.len());
    assert!(failures.is_empty(), "failing std tests ({arch}):\n\n{}", failures.join("\n\n"));
}

#[test]
fn host_stdtests() {
    check(std::env::consts::ARCH, &stdtests_bin(None), None, Duration::from_secs(60));
}

#[test]
fn aarch64_stdtests() {
    let Some(qemu) = qemu_aarch64() else {
        eprintln!("skipping aarch64: set QEMU_AARCH64 or put qemu-aarch64 on PATH");
        return;
    };
    if !aarch64_target_installed() {
        eprintln!("skipping aarch64: run `rustup target add {AARCH64}`");
        return;
    }
    check("aarch64 (qemu)", &stdtests_bin(Some(AARCH64)), Some(&qemu), Duration::from_secs(300));
}
