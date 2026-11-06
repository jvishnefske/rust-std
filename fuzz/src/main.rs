//! The rust_std half of the differential fuzzer (see `harness/src/fuzz.rs`).
//!
//! Protocol on stdin/stdout, repeated until EOF:
//! request `[target: u8][len: u32 LE][input]`, reply `[len: u32 LE][transcript]`.
//! A panic aborts the process, which the driver reports as a crash.
#![no_std]
#![no_main]

extern crate rust_std as std;
use std::io::{Read, Write};
use std::prelude::*;

std::main!(main);

#[allow(dead_code)]
mod ops {
    use std::prelude::*;
    include!("../ops.rs");
}

fn main() {
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    let scratch = std::env::temp_dir().join(format!("fuzzee-{}", std::process::id()));
    for n in 0u64.. {
        let mut header = [0u8; 5];
        if stdin.read_exact(&mut header).is_err() {
            break;
        }
        let len = u32::from_le_bytes(header[1..].try_into().unwrap()) as usize;
        let mut input = vec![0; len];
        stdin.read_exact(&mut input).expect("truncated request");

        let dir = scratch.join(n.to_string());
        std::fs::create_dir_all(&dir).expect("cannot create scratch dir");
        let out = ops::run(header[0] as usize, &input, &dir);
        let _ = std::fs::remove_dir_all(&dir);

        stdout.write_all(&(out.len() as u32).to_le_bytes()).unwrap();
        stdout.write_all(out.as_bytes()).unwrap();
        stdout.flush().unwrap();
    }
    let _ = std::fs::remove_dir_all(&scratch);
}
