//! The stack protector and `_FORTIFY_SOURCE`'s checked functions, for C code
//! built with hardening flags (Nix's defaults) against glibc's headers.
//!
//! `-fstack-protector` compares a canary on return from a function: on
//! x86_64 it reads it from `%fs:0x28` (the TCB, see [`crate::tls`]), on
//! aarch64 from `__stack_chk_guard`. Both hold the same value, from the
//! kernel's `AT_RANDOM` bytes, with a zero low byte (as glibc's) so that
//! string functions cannot read it out or write it back.
//!
//! `_FORTIFY_SOURCE` makes glibc's headers call `__memcpy_chk(dst, src, n,
//! dstlen)` and friends where the compiler knows the destination's size:
//! they abort if the write would overflow it, as glibc does, and otherwise
//! do what the unchecked function does.

use core::ffi::{VaList, c_char, c_int, c_long, c_uint, c_void};

use super::{FILE, size_t, ssize_t};
use crate::{start, sys};

/// The stack canary (aarch64's `-fstack-protector` reads it here).
#[unsafe(no_mangle)]
pub static mut __stack_chk_guard: usize = 0;

/// Set the canary from `AT_RANDOM`, before any C code runs.
pub(crate) unsafe fn init_stack_guard() {
    let Some(p) = start::auxv(start::AT_RANDOM).filter(|&p| p != 0) else { return };
    let v = unsafe { (p as *const usize).read_unaligned() } & !0xff;
    unsafe { *(&raw mut __stack_chk_guard) = v };
}

/// The canary, for new threads' TCBs.
pub(crate) fn stack_guard() -> usize {
    unsafe { *(&raw const __stack_chk_guard) }
}

fn die(msg: &[u8]) -> ! {
    start::write_stderr(msg);
    start::abort()
}

#[unsafe(no_mangle)]
pub extern "C" fn __stack_chk_fail() -> ! {
    die(b"*** stack smashing detected ***: terminated\n")
}

#[unsafe(no_mangle)]
pub extern "C" fn __stack_chk_fail_local() -> ! {
    __stack_chk_fail()
}

#[unsafe(no_mangle)]
pub extern "C" fn __chk_fail() -> ! {
    die(b"*** buffer overflow detected ***: terminated\n")
}

fn check(n: size_t, room: size_t) {
    if n > room {
        __chk_fail();
    }
}

unsafe extern "C" {
    fn memcpy(d: *mut c_void, s: *const c_void, n: size_t) -> *mut c_void;
    fn memmove(d: *mut c_void, s: *const c_void, n: size_t) -> *mut c_void;
    fn memset(d: *mut c_void, c: c_int, n: size_t) -> *mut c_void;
    fn strlen(s: *const c_char) -> size_t;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __memcpy_chk(d: *mut c_void, s: *const c_void, n: size_t, room: size_t) -> *mut c_void {
    check(n, room);
    unsafe { memcpy(d, s, n) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __memmove_chk(d: *mut c_void, s: *const c_void, n: size_t, room: size_t) -> *mut c_void {
    check(n, room);
    unsafe { memmove(d, s, n) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __mempcpy_chk(d: *mut c_void, s: *const c_void, n: size_t, room: size_t) -> *mut c_void {
    check(n, room);
    unsafe { memcpy(d, s, n).cast::<u8>().add(n).cast() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __memset_chk(d: *mut c_void, c: c_int, n: size_t, room: size_t) -> *mut c_void {
    check(n, room);
    unsafe { memset(d, c, n) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __explicit_bzero_chk(d: *mut c_void, n: size_t, room: size_t) {
    check(n, room);
    // volatile, so the zeroing of a dying buffer is not optimized out
    for i in 0..n {
        unsafe { d.cast::<u8>().add(i).write_volatile(0) };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __strcpy_chk(d: *mut c_char, s: *const c_char, room: size_t) -> *mut c_char {
    let n = unsafe { strlen(s) } + 1;
    check(n, room);
    unsafe { memcpy(d.cast(), s.cast(), n) };
    d
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stpcpy_chk(d: *mut c_char, s: *const c_char, room: size_t) -> *mut c_char {
    let n = unsafe { strlen(s) };
    check(n + 1, room);
    unsafe {
        memcpy(d.cast(), s.cast(), n + 1);
        d.add(n)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __strncpy_chk(d: *mut c_char, s: *const c_char, n: size_t, room: size_t) -> *mut c_char {
    check(n, room);
    unsafe { super::string::strncpy(d, s, n) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stpncpy_chk(d: *mut c_char, s: *const c_char, n: size_t, room: size_t) -> *mut c_char {
    check(n, room);
    let len = unsafe { super::string::strnlen(s, n) };
    unsafe {
        memcpy(d.cast(), s.cast(), len);
        memset(d.add(len).cast(), 0, n - len);
        d.add(len)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __strcat_chk(d: *mut c_char, s: *const c_char, room: size_t) -> *mut c_char {
    let (dl, sl) = unsafe { (strlen(d), strlen(s)) };
    check(dl + sl + 1, room);
    unsafe { memcpy(d.add(dl).cast(), s.cast(), sl + 1) };
    d
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __strncat_chk(d: *mut c_char, s: *const c_char, n: size_t, room: size_t) -> *mut c_char {
    let (dl, sl) = unsafe { (strlen(d), super::string::strnlen(s, n)) };
    check(dl + sl + 1, room);
    unsafe {
        memcpy(d.add(dl).cast(), s.cast(), sl);
        *d.add(dl + sl) = 0;
    }
    d
}

// ---- printf: `flag` asks for extra checks of the format (%n in writable
// memory), which there is no need for: this printf has no %n ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vsnprintf_chk(s: *mut c_char, n: size_t, _flag: c_int, room: size_t, fmt: *const c_char, ap: VaList) -> c_int {
    check(n, room);
    unsafe { super::stdio::vsnprintf(s, n, fmt, ap) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __snprintf_chk(s: *mut c_char, n: size_t, flag: c_int, room: size_t, fmt: *const c_char, ap: ...) -> c_int {
    unsafe { __vsnprintf_chk(s, n, flag, room, fmt, ap) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vsprintf_chk(s: *mut c_char, _flag: c_int, room: size_t, fmt: *const c_char, ap: VaList) -> c_int {
    // glibc formats at most `room` bytes, and fails if that was not all
    let n = unsafe { super::stdio::vsnprintf(s, room, fmt, ap) };
    if n < 0 || n as size_t >= room {
        __chk_fail();
    }
    n
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sprintf_chk(s: *mut c_char, flag: c_int, room: size_t, fmt: *const c_char, ap: ...) -> c_int {
    unsafe { __vsprintf_chk(s, flag, room, fmt, ap) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vfprintf_chk(f: *mut FILE, _flag: c_int, fmt: *const c_char, ap: VaList) -> c_int {
    unsafe { super::stdio::vfprintf(f, fmt, ap) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __fprintf_chk(f: *mut FILE, flag: c_int, fmt: *const c_char, ap: ...) -> c_int {
    unsafe { __vfprintf_chk(f, flag, fmt, ap) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vprintf_chk(flag: c_int, fmt: *const c_char, ap: VaList) -> c_int {
    unsafe { __vfprintf_chk(super::stdio::stdout, flag, fmt, ap) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __printf_chk(flag: c_int, fmt: *const c_char, ap: ...) -> c_int {
    unsafe { __vprintf_chk(flag, fmt, ap) }
}

// ---- file descriptors ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __read_chk(fd: c_int, buf: *mut c_void, n: size_t, room: size_t) -> ssize_t {
    check(n, room);
    unsafe { super::read(fd, buf, n) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __pread_chk(fd: c_int, buf: *mut c_void, n: size_t, off: i64, room: size_t) -> ssize_t {
    check(n, room);
    unsafe { super::pread(fd, buf, n, off) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __readlink_chk(path: *const c_char, buf: *mut c_char, n: size_t, room: size_t) -> ssize_t {
    check(n, room);
    unsafe { super::readlink(path, buf, n) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __realpath_chk(path: *const c_char, resolved: *mut c_char, room: size_t) -> *mut c_char {
    // realpath writes up to PATH_MAX bytes
    if !resolved.is_null() {
        check(4096, room);
    }
    unsafe { super::realpath(path, resolved) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __poll_chk(fds: *mut c_void, n: core::ffi::c_ulong, timeout: c_int, room: size_t) -> c_int {
    // struct pollfd is 8 bytes
    check(n as size_t * 8, room);
    unsafe { super::poll(fds, n, timeout) }
}

/// What `open` and `openat` without a mode check: the flags must not need
/// one.
fn check_no_mode(flags: c_int, call: &[u8]) {
    let tmpfile = 0o20000000 | sys::O_DIRECTORY as c_int;
    if flags & sys::O_CREAT as c_int != 0 || flags & tmpfile == tmpfile {
        start::write_stderr(b"*** invalid ");
        die(call);
    }
}

/// The `open` that `_FORTIFY_SOURCE` calls when no mode is passed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __open_2(path: *const c_char, flags: c_int) -> c_int {
    check_no_mode(flags, b"open call: O_CREAT or O_TMPFILE without mode ***: terminated\n");
    unsafe { super::open(path, flags, 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __openat_2(dirfd: c_int, path: *const c_char, flags: c_int) -> c_int {
    check_no_mode(flags, b"openat call: O_CREAT or O_TMPFILE without mode ***: terminated\n");
    unsafe { super::openat(dirfd, path, flags, 0 as c_uint) }
}

/// `FD_SET` and friends: the index of `fd`'s word in an `fd_set`, which
/// holds 1024 descriptors.
#[unsafe(no_mangle)]
pub extern "C" fn __fdelt_chk(fd: c_long) -> c_long {
    if !(0..1024).contains(&fd) {
        __chk_fail();
    }
    fd / 64
}
