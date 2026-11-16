//! The stack protector's canary and `_FORTIFY_SOURCE`'s checked functions
//! (rt/src/clib/fortify.rs), called as C code built with hardening flags
//! calls them. Prints `name value` lines; with an argument, makes that
//! checked call overflow (which aborts).

use std::ffi::{c_char, c_int, c_long, c_void};

unsafe extern "C" {
    static __stack_chk_guard: usize;
    fn __memcpy_chk(d: *mut c_void, s: *const c_void, n: usize, room: usize) -> *mut c_void;
    fn __strcpy_chk(d: *mut c_char, s: *const c_char, room: usize) -> *mut c_char;
    fn __strcat_chk(d: *mut c_char, s: *const c_char, room: usize) -> *mut c_char;
    fn __snprintf_chk(s: *mut c_char, n: usize, flag: c_int, room: usize, fmt: *const c_char, ...) -> c_int;
    fn __sprintf_chk(s: *mut c_char, flag: c_int, room: usize, fmt: *const c_char, ...) -> c_int;
    fn __fdelt_chk(fd: c_long) -> c_long;
}

/// What `-fstack-protector` compares against on this thread.
fn tcb_canary() -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        let v: usize;
        unsafe { std::arch::asm!("mov {}, fs:0x28", out(reg) v) };
        v
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        __stack_chk_guard
    }
}

fn main() {
    let mut buf = [0u8; 16];
    match std::env::args().nth(1).as_deref() {
        Some("memcpy") => unsafe {
            __memcpy_chk(buf.as_mut_ptr().cast(), c"0123456789abcdefXYZ".as_ptr().cast(), 19, buf.len());
        },
        Some("sprintf") => unsafe {
            __sprintf_chk(buf.as_mut_ptr().cast(), 1, buf.len(), c"%s-%d".as_ptr(), c"0123456789".as_ptr(), 123456);
        },
        Some("fdelt") => unsafe {
            __fdelt_chk(1024);
        },
        Some(other) => panic!("unknown overflow {other}"),
        None => {}
    }

    let guard = unsafe { __stack_chk_guard };
    println!("guard_set {}", guard != 0 && guard & 0xff == 0);
    println!("tcb_main {}", tcb_canary() == guard);
    println!("tcb_thread {}", std::thread::spawn(tcb_canary).join().unwrap() == guard);

    unsafe {
        __memcpy_chk(buf.as_mut_ptr().cast(), c"0123456789abcdef".as_ptr().cast(), 16, buf.len());
        println!("memcpy {}", std::str::from_utf8(&buf).unwrap());
        __strcpy_chk(buf.as_mut_ptr().cast(), c"hello".as_ptr(), buf.len());
        __strcat_chk(buf.as_mut_ptr().cast(), c", world".as_ptr(), buf.len());
        println!("strcat {}", std::ffi::CStr::from_ptr(buf.as_ptr().cast()).to_str().unwrap());
        let n = __snprintf_chk(buf.as_mut_ptr().cast(), 8, 1, buf.len(), c"%d-%s".as_ptr(), 42, c"abcdefgh".as_ptr());
        println!("snprintf {n} {}", std::ffi::CStr::from_ptr(buf.as_ptr().cast()).to_str().unwrap());
        let n = __sprintf_chk(buf.as_mut_ptr().cast(), 1, buf.len(), c"%x".as_ptr(), 0xbeefu32);
        println!("sprintf {n} {}", std::ffi::CStr::from_ptr(buf.as_ptr().cast()).to_str().unwrap());
        println!("fdelt {} {}", __fdelt_chk(0), __fdelt_chk(1023));
    }
}
