//! Built with an allowlist that leaves out `openat` (harness/tests/seccomp.rs
//! sets `LINUX_NONE_SECCOMP`), the open below is denied by the filter rt
//! installs before `main`. Built without one, it succeeds.

fn main() {
    println!("before open");
    match std::fs::File::open("/") {
        Ok(_) => println!("open: ok"),
        Err(e) => println!("open: {:?} errno={}", e.kind(), e.raw_os_error().unwrap_or(0)),
    }
}
