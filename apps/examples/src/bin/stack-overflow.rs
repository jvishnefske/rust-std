//! Crash on purpose, as `stack-overflow <how>`: `main` and `thread` recurse
//! forever (std reports the overflow and aborts); `segv` writes to an
//! unmapped address and `kill` has SIGSEGV sent to itself (both must still
//! die by SIGSEGV, without the message).

use std::hint::black_box;

#[allow(unconditional_recursion)]
fn recurse(depth: u64) -> u64 {
    let buf = black_box([depth; 64]);
    recurse(depth + 1) + buf[depth as usize % 64]
}

fn main() {
    let how = std::env::args().nth(1).unwrap_or_default();
    eprintln!("crashing: {how}");
    match how.as_str() {
        "main" => {
            black_box(recurse(0));
        }
        "thread" => {
            let t = std::thread::Builder::new().name("deep".into()).spawn(|| black_box(recurse(0))).unwrap();
            let _ = t.join();
        }
        "segv" => unsafe {
            std::ptr::write_volatile(black_box(0x10usize) as *mut u8, 1);
        },
        "kill" => {
            let pid = std::process::id().to_string();
            let _ = std::process::Command::new("kill").args(["-SEGV", &pid]).status();
            std::thread::sleep(std::time::Duration::from_secs(10));
        }
        _ => panic!("usage: stack-overflow main|thread|segv|kill"),
    }
    eprintln!("still alive");
}
