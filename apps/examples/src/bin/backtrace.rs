//! Backtraces: walked by the unwinder from the executable's .eh_frame_hdr and
//! symbolized from its own debug info (`current_exe`). Works under both panic
//! strategies; with an argument it then panics, for RUST_BACKTRACE.

use std::backtrace::{Backtrace, BacktraceStatus};

#[inline(never)]
fn inner_frame() -> Backtrace {
    Backtrace::force_capture()
}

#[inline(never)]
fn outer_frame() -> Backtrace {
    std::hint::black_box(inner_frame())
}

#[inline(never)]
fn panicking_frame(n: usize) -> usize {
    let v: Vec<usize> = Vec::new();
    v[n]
}

fn main() {
    let bt = outer_frame();
    println!("status: {:?}", bt.status() == BacktraceStatus::Captured);
    let s = format!("{bt}");
    let frames = s.lines().filter(|l| l.trim_start().split(':').next().is_some_and(|n| n.parse::<u32>().is_ok())).count();
    println!("frames > 3: {}", frames > 3);
    for f in ["backtrace::inner_frame", "backtrace::outer_frame", "backtrace::main"] {
        println!("has {f}: {}", s.contains(f));
    }
    println!("has file:line: {}", s.contains("examples/src/bin/backtrace.rs:"));
    let (i, o) = (s.find("backtrace::inner_frame"), s.find("backtrace::outer_frame"));
    println!("inner before outer: {}", matches!((i, o), (Some(i), Some(o)) if i < o));
    if std::env::args().count() > 1 {
        let n = std::env::args().count() + 10;
        println!("{}", panicking_frame(n));
    }
}
