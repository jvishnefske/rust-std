//! Differential fuzzing of rust_std against std (see `harness/src/fuzz.rs`).
//!
//! Environment:
//! * `FUZZ_SEED`  — seed to reproduce a run (printed on failure)
//! * `FUZZ_ITERS` — cases per target (default 3000; a quarter under qemu)
//! * `FUZZ_SECS`  — keep fuzzing each target for this long instead

use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use harness::fuzz::{Fuzzee, TARGETS, fuzz};
use harness::*;

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok().map(|v| v.parse().unwrap_or_else(|_| panic!("{name} must be a number")))
}

fn run(arch: &str, bin: &Path, runner: Option<&Path>, default_iters: usize) {
    let seed = env_u64("FUZZ_SEED").unwrap_or_else(|| SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos() as u64);
    let secs = env_u64("FUZZ_SECS");
    let iters = match secs {
        Some(_) => usize::MAX,
        None => env_u64("FUZZ_ITERS").map_or(default_iters, |n| n as usize),
    };
    let deadline = secs.map(|s| Instant::now() + Duration::from_secs(s));

    // one fuzzee process per target, all targets in parallel
    let reports: Vec<_> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..TARGETS.len())
            .map(|t| s.spawn(move || fuzz(&mut Fuzzee::new(bin, runner), t, seed, iters, deadline)))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    let mut failures = Vec::new();
    for (t, r) in reports.iter().enumerate() {
        eprintln!("fuzz {arch} {:>10}: {} cases{}", TARGETS[t], r.cases, if r.finding.is_some() { ", FAILED" } else { "" });
        if let Some(f) = &r.finding {
            failures.push(f.to_string());
        }
    }
    assert!(failures.is_empty(), "fuzzing found differences ({arch}, FUZZ_SEED={seed}):\n\n{}", failures.join("\n\n"));
}

#[test]
fn host_fuzz() {
    run(std::env::consts::ARCH, &fuzzee_bin(None), None, 3000);
}

#[test]
fn aarch64_fuzz() {
    let Some(qemu) = qemu_aarch64() else {
        eprintln!("skipping aarch64: set QEMU_AARCH64 or put qemu-aarch64 on PATH");
        return;
    };
    if !aarch64_target_installed() {
        eprintln!("skipping aarch64: run `rustup target add {AARCH64}`");
        return;
    }
    run("aarch64", &fuzzee_bin(Some(AARCH64)), Some(&qemu), 750);
}
