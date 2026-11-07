//! The C library functions that crates call through the `libc` crate, for
//! programs on linux-none, where there is no libc to provide them.
//!
//! The libc crate gives such crates musl's types and constants (see
//! `cargo xtask libc-fixup`); these are its functions, as musl defines them,
//! on the runtime's syscalls. They cover what common crates use (terminals,
//! mmap, CPU counts, epoll, signals, semaphores, users and groups, fork and
//! exec), not all of libc: C code still needs a C library.
//!
//! Variadic functions (`open`, `ioctl`, `syscall`) take their full argument
//! list: on x86_64 and aarch64 Linux, variadic integer arguments are passed
//! as fixed ones are, and the unused ones are never read meaningfully.
//!
//! The environment (`getenv`, `execv`, `execvp`) is the initial one, from
//! the process's stack: changes made through std's `env::set_var` are not
//! seen.

#![allow(non_camel_case_types)]

use core::ffi::{CStr, c_char, c_int, c_long, c_uint, c_ulong, c_void};
use core::sync::atomic::{AtomicU32, Ordering};

use crate::arch::{KSigaction, nr};
use crate::sys::{self, AT_FDCWD, SysResult};
use crate::{elf, errno, futex, start};

type size_t = usize;
type ssize_t = isize;
type pid_t = i32;
type uid_t = u32;
type gid_t = u32;

fn set_errno(e: i32) {
    unsafe { *errno::location() = e };
}

/// The C convention: the value, or -1 with `errno` set.
fn ret(r: SysResult) -> isize {
    match r {
        Ok(v) => v as isize,
        Err(e) => {
            set_errno(e);
            -1
        }
    }
}

fn ret_int(r: SysResult) -> c_int {
    ret(r) as c_int
}

#[unsafe(no_mangle)]
pub extern "C" fn __errno_location() -> *mut c_int {
    errno::location()
}

// ---- file descriptors ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn read(fd: c_int, buf: *mut c_void, n: size_t) -> ssize_t {
    ret(unsafe { sys::read(fd, buf.cast(), n) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn write(fd: c_int, buf: *const c_void, n: size_t) -> ssize_t {
    ret(unsafe { sys::write(fd, buf.cast(), n) })
}

#[unsafe(no_mangle)]
pub extern "C" fn close(fd: c_int) -> c_int {
    ret_int(sys::close(fd))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn open(path: *const c_char, flags: c_int, mode: c_uint) -> c_int {
    unsafe { openat(AT_FDCWD, path, flags, mode) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openat(dirfd: c_int, path: *const c_char, flags: c_int, mode: c_uint) -> c_int {
    ret_int(unsafe { sys::openat(dirfd, path.cast(), flags as u32, mode) }.map(|fd| fd as usize))
}

#[unsafe(no_mangle)]
pub extern "C" fn dup(fd: c_int) -> c_int {
    ret_int(sys::dup(fd).map(|fd| fd as usize))
}

#[unsafe(no_mangle)]
pub extern "C" fn dup2(old: c_int, new: c_int) -> c_int {
    if old == new {
        // dup3 rejects equal descriptors; dup2 checks `old` is open
        return ret_int(sys::fcntl(old, sys::F_GETFD, 0).map(|_| new as usize));
    }
    ret_int(sys::dup3(old, new, 0).map(|fd| fd as usize))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pipe(fds: *mut c_int) -> c_int {
    unsafe { pipe2(fds, 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pipe2(fds: *mut c_int, flags: c_int) -> c_int {
    match sys::pipe2(flags as u32) {
        Ok(p) => {
            unsafe { fds.cast::<[i32; 2]>().write(p) };
            0
        }
        Err(e) => ret_int(Err(e)),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn splice(
    fd_in: c_int,
    off_in: *mut i64,
    fd_out: c_int,
    off_out: *mut i64,
    len: size_t,
    flags: c_uint,
) -> ssize_t {
    let args = [fd_in as usize, off_in as usize, fd_out as usize, off_out as usize, len, flags as usize];
    ret(unsafe { sys::syscall(nr::SPLICE, args) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ioctl(fd: c_int, req: c_ulong, arg: usize) -> c_int {
    ret_int(unsafe { sys::ioctl(fd, req as usize, arg) })
}

const TCGETS: usize = 0x5401;

#[unsafe(no_mangle)]
pub extern "C" fn isatty(fd: c_int) -> c_int {
    // a `struct termios`, which the kernel fills for terminals only
    let mut termios = [0u8; 64];
    match unsafe { sys::ioctl(fd, TCGETS, termios.as_mut_ptr() as usize) } {
        Ok(_) => 1,
        Err(e) => {
            set_errno(e);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn access(path: *const c_char, mode: c_int) -> c_int {
    unsafe { faccessat(AT_FDCWD, path, mode, 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn faccessat(dirfd: c_int, path: *const c_char, mode: c_int, flags: c_int) -> c_int {
    ret_int(if flags == 0 {
        unsafe { sys::faccessat(dirfd, path.cast(), mode as u32) }
    } else {
        unsafe { sys::syscall(nr::FACCESSAT2, [dirfd as usize, path as usize, mode as usize, flags as usize, 0, 0]) }
    })
}

/// `syscall(n, ...)`: the raw syscall, with the C convention for errors.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall(n: c_long, a: c_long, b: c_long, c: c_long, d: c_long, e: c_long, f: c_long) -> c_long {
    let args = [a, b, c, d, e, f].map(|x| x as usize);
    ret(unsafe { sys::syscall(n as usize, args) }) as c_long
}

#[unsafe(no_mangle)]
pub extern "C" fn _exit(code: c_int) -> ! {
    sys::exit_group(code)
}

// ---- identity ----

#[unsafe(no_mangle)]
pub extern "C" fn getpid() -> pid_t {
    sys::getpid() as pid_t
}

#[unsafe(no_mangle)]
pub extern "C" fn getppid() -> pid_t {
    sys::getppid() as pid_t
}

#[unsafe(no_mangle)]
pub extern "C" fn getuid() -> uid_t {
    sys::getuid()
}

#[unsafe(no_mangle)]
pub extern "C" fn geteuid() -> uid_t {
    sys::geteuid()
}

#[unsafe(no_mangle)]
pub extern "C" fn getgid() -> gid_t {
    sys::getgid()
}

#[unsafe(no_mangle)]
pub extern "C" fn getegid() -> gid_t {
    sys::getegid()
}

/// `struct utsname`: six 65-byte fields, as the kernel's `new_utsname`.
const UTS_LEN: usize = 65;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uname(buf: *mut c_void) -> c_int {
    ret_int(unsafe { sys::uname(buf.cast()) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gethostname(name: *mut c_char, len: size_t) -> c_int {
    let mut uts = [0u8; 6 * UTS_LEN];
    if let Err(e) = unsafe { sys::uname(uts.as_mut_ptr()) } {
        return ret_int(Err(e));
    }
    let node = &uts[UTS_LEN..2 * UTS_LEN];
    let n = node.iter().position(|&b| b == 0).unwrap_or(UTS_LEN - 1);
    if n >= len {
        return ret_int(Err(sys::ENAMETOOLONG));
    }
    unsafe {
        core::ptr::copy_nonoverlapping(node.as_ptr(), name.cast(), n);
        *name.add(n) = 0;
    }
    0
}

/// The value in `entry` (`NAME=value`) if it is `name`'s.
unsafe fn env_value(entry: *const u8, name: &[u8]) -> Option<*mut c_char> {
    let e = unsafe { CStr::from_ptr(entry.cast()) }.to_bytes();
    (e.len() > name.len() && e.starts_with(name) && e[name.len()] == b'=').then(|| unsafe { entry.add(name.len() + 1) } as *mut c_char)
}

/// The entries `putenv` added, which `getenv` consults before the initial
/// environment (latest first).
struct PutEnv {
    lock: crate::locks::RawMutex,
    entries: core::cell::UnsafeCell<([usize; 64], usize)>,
}

// SAFETY: `entries` is only used with `lock` held
unsafe impl Sync for PutEnv {}

impl PutEnv {
    fn with<R>(&self, f: impl FnOnce(&mut [usize; 64], &mut usize) -> R) -> R {
        self.lock.lock();
        let (entries, n) = unsafe { &mut *self.entries.get() };
        let r = f(entries, n);
        unsafe { self.lock.unlock() };
        r
    }
}

static PUTENV: PutEnv = PutEnv { lock: crate::locks::RawMutex::new(), entries: core::cell::UnsafeCell::new(([0; 64], 0)) };

/// `putenv`: `string` (`NAME=value`, kept, not copied) is seen by later
/// `getenv` calls, not by std's `env::var` or child processes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn putenv(string: *mut c_char) -> c_int {
    let s = unsafe { CStr::from_ptr(string) }.to_bytes();
    let Some(eq) = s.iter().position(|&b| b == b'=') else { return ret_int(Err(sys::EINVAL)) };
    PUTENV.with(|entries, n| {
        // replace an entry for the same name
        for e in entries[..*n].iter_mut() {
            if unsafe { env_value(*e as *const u8, &s[..eq]) }.is_some() {
                *e = string as usize;
                return 0;
            }
        }
        if *n == entries.len() {
            return ret_int(Err(sys::ENOMEM));
        }
        entries[*n] = string as usize;
        *n += 1;
        0
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getenv(name: *const c_char) -> *mut c_char {
    let name = unsafe { CStr::from_ptr(name) }.to_bytes();
    if name.is_empty() || name.contains(&b'=') {
        return core::ptr::null_mut();
    }
    let added = PUTENV.with(|entries, n| entries[..*n].iter().rev().find_map(|&e| unsafe { env_value(e as *const u8, name) }));
    if let Some(v) = added {
        return v;
    }
    let mut p = start::envp();
    if p.is_null() {
        return core::ptr::null_mut();
    }
    unsafe {
        while !(*p).is_null() {
            if let Some(v) = env_value(*p, name) {
                return v;
            }
            p = p.add(1);
        }
    }
    core::ptr::null_mut()
}

// ---- sysconf ----

const _SC_ARG_MAX: c_int = 0;
const _SC_CHILD_MAX: c_int = 1;
const _SC_CLK_TCK: c_int = 2;
const _SC_NGROUPS_MAX: c_int = 3;
const _SC_OPEN_MAX: c_int = 4;
const _SC_PAGESIZE: c_int = 30;
const _SC_GETGR_R_SIZE_MAX: c_int = 69;
const _SC_GETPW_R_SIZE_MAX: c_int = 70;
const _SC_NPROCESSORS_CONF: c_int = 83;
const _SC_NPROCESSORS_ONLN: c_int = 84;
const _SC_PHYS_PAGES: c_int = 85;
const _SC_HOST_NAME_MAX: c_int = 180;

const RLIMIT_NPROC: usize = 6;
const RLIMIT_NOFILE: usize = 7;
const RLIM_INFINITY: u64 = !0;

/// The soft limit of `resource`, as a `sysconf` value (-1 for unlimited).
fn soft_limit(resource: usize) -> c_long {
    let mut lim = [0u64; 2];
    match unsafe { sys::syscall(nr::PRLIMIT64, [0, resource, 0, lim.as_mut_ptr() as usize, 0, 0]) } {
        Ok(_) if lim[0] != RLIM_INFINITY => lim[0].min(c_long::MAX as u64) as c_long,
        _ => -1,
    }
}

/// The number of CPUs the process may run on.
fn cpus() -> c_long {
    let mut mask = [0u8; 128];
    match unsafe { sys::sched_getaffinity(mask.as_mut_ptr(), mask.len()) } {
        Ok(n) => mask[..n].iter().map(|b| b.count_ones() as c_long).sum::<c_long>().max(1),
        Err(_) => 1,
    }
}

fn page_size() -> usize {
    start::auxv(start::AT_PAGESZ).unwrap_or(sys::PAGE_SIZE)
}

#[unsafe(no_mangle)]
pub extern "C" fn sysconf(name: c_int) -> c_long {
    match name {
        _SC_ARG_MAX => 131072,
        _SC_CHILD_MAX => soft_limit(RLIMIT_NPROC),
        _SC_CLK_TCK => 100,
        _SC_NGROUPS_MAX => 65536,
        _SC_OPEN_MAX => soft_limit(RLIMIT_NOFILE),
        _SC_PAGESIZE => page_size() as c_long,
        // no limit: callers pick a buffer size and grow it on ERANGE
        _SC_GETGR_R_SIZE_MAX | _SC_GETPW_R_SIZE_MAX => -1,
        _SC_NPROCESSORS_CONF | _SC_NPROCESSORS_ONLN => cpus(),
        _SC_PHYS_PAGES => {
            // `struct sysinfo`: totalram at 32, mem_unit at 104
            let mut si = [0u64; 16];
            match unsafe { sys::syscall(nr::SYSINFO, [si.as_mut_ptr() as usize, 0, 0, 0, 0, 0]) } {
                Ok(_) => {
                    let unit = (si[13] & 0xffff_ffff).max(1);
                    (si[4].saturating_mul(unit) / page_size() as u64) as c_long
                }
                Err(e) => ret(Err(e)) as c_long,
            }
        }
        _SC_HOST_NAME_MAX => 64,
        _ => ret(Err(sys::EINVAL)) as c_long,
    }
}

// ---- memory ----

const MAP_FAILED: *mut c_void = !0usize as *mut c_void;
const POSIX_MADV_DONTNEED: c_int = 4;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mmap(addr: *mut c_void, len: size_t, prot: c_int, flags: c_int, fd: c_int, off: i64) -> *mut c_void {
    match unsafe { sys::mmap(addr as usize, len, prot as usize, flags as usize, fd, off as usize) } {
        Ok(p) => p as *mut c_void,
        Err(e) => {
            set_errno(e);
            MAP_FAILED
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn munmap(addr: *mut c_void, len: size_t) -> c_int {
    ret_int(unsafe { sys::munmap(addr as usize, len) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mprotect(addr: *mut c_void, len: size_t, prot: c_int) -> c_int {
    ret_int(unsafe { sys::mprotect(addr as usize, len, prot as usize) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn madvise(addr: *mut c_void, len: size_t, advice: c_int) -> c_int {
    ret_int(unsafe { sys::syscall(nr::MADVISE, [addr as usize, len, advice as usize, 0, 0, 0]) })
}

/// Returns the error rather than setting `errno`, as POSIX says. Like
/// glibc, `POSIX_MADV_DONTNEED` does nothing: Linux's `MADV_DONTNEED`
/// discards the pages, which POSIX's does not allow.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_madvise(addr: *mut c_void, len: size_t, advice: c_int) -> c_int {
    if advice == POSIX_MADV_DONTNEED {
        return 0;
    }
    match unsafe { sys::syscall(nr::MADVISE, [addr as usize, len, advice as usize, 0, 0, 0]) } {
        Ok(_) => 0,
        Err(e) => e,
    }
}

// ---- scheduling ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sched_getaffinity(pid: pid_t, size: size_t, mask: *mut c_void) -> c_int {
    match unsafe { sys::syscall(nr::SCHED_GETAFFINITY, [pid as usize, size, mask as usize, 0, 0, 0]) } {
        Ok(n) => {
            // the kernel writes its mask size; the rest is ours to clear
            unsafe { core::ptr::write_bytes(mask.cast::<u8>().add(n), 0, size - n) };
            0
        }
        Err(e) => ret_int(Err(e)),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn sched_yield() -> c_int {
    sys::sched_yield();
    0
}

// ---- polling ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn poll(fds: *mut c_void, n: c_ulong, timeout_ms: c_int) -> c_int {
    let ts = sys::Timespec { sec: (timeout_ms / 1000) as i64, nsec: (timeout_ms % 1000) as i64 * 1_000_000 };
    let tsp = if timeout_ms < 0 { core::ptr::null() } else { &raw const ts };
    ret_int(unsafe { sys::ppoll(fds.cast(), n as usize, tsp) })
}

#[unsafe(no_mangle)]
pub extern "C" fn epoll_create1(flags: c_int) -> c_int {
    ret_int(unsafe { sys::syscall(nr::EPOLL_CREATE1, [flags as usize, 0, 0, 0, 0, 0]) })
}

#[unsafe(no_mangle)]
pub extern "C" fn epoll_create(size: c_int) -> c_int {
    if size <= 0 {
        return ret_int(Err(sys::EINVAL));
    }
    epoll_create1(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn epoll_ctl(epfd: c_int, op: c_int, fd: c_int, event: *mut c_void) -> c_int {
    ret_int(unsafe { sys::syscall(nr::EPOLL_CTL, [epfd as usize, op as usize, fd as usize, event as usize, 0, 0]) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn epoll_wait(epfd: c_int, events: *mut c_void, max: c_int, timeout_ms: c_int) -> c_int {
    let args = [epfd as usize, events as usize, max as usize, timeout_ms as usize, 0, 8];
    ret_int(unsafe { sys::syscall(nr::EPOLL_PWAIT, args) })
}

// ---- sockets ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn send(fd: c_int, buf: *const c_void, len: size_t, flags: c_int) -> ssize_t {
    ret(unsafe { sys::syscall(nr::SENDTO, [fd as usize, buf as usize, len, flags as usize, 0, 0]) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn recv(fd: c_int, buf: *mut c_void, len: size_t, flags: c_int) -> ssize_t {
    ret(unsafe { sys::syscall(nr::RECVFROM, [fd as usize, buf as usize, len, flags as usize, 0, 0]) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn socketpair(domain: c_int, ty: c_int, proto: c_int, fds: *mut c_int) -> c_int {
    ret_int(unsafe { sys::socketpair(domain as u16, ty as u32, proto as u32, fds.cast()) })
}

// ---- signals ----

/// musl's `sigset_t`: 1024 bits, of which the kernel uses the first 64.
type sigset_t = [u64; 16];

/// musl's `struct sigaction`.
#[repr(C)]
pub struct sigaction {
    pub sa_sigaction: usize,
    pub sa_mask: sigset_t,
    pub sa_flags: c_int,
    pub sa_restorer: usize,
}

const SA_RESTART: usize = 0x1000_0000;
const NSIG: c_int = 65;

fn valid_signal(sig: c_int) -> bool {
    (1..NSIG).contains(&sig)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigemptyset(set: *mut sigset_t) -> c_int {
    unsafe { set.write([0; 16]) };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigfillset(set: *mut sigset_t) -> c_int {
    unsafe { set.write([0; 16]) };
    unsafe { (*set)[0] = !0 };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigaddset(set: *mut sigset_t, sig: c_int) -> c_int {
    if !valid_signal(sig) {
        return ret_int(Err(sys::EINVAL));
    }
    unsafe { (*set)[0] |= 1 << (sig - 1) };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigdelset(set: *mut sigset_t, sig: c_int) -> c_int {
    if !valid_signal(sig) {
        return ret_int(Err(sys::EINVAL));
    }
    unsafe { (*set)[0] &= !(1 << (sig - 1)) };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigismember(set: *const sigset_t, sig: c_int) -> c_int {
    if !valid_signal(sig) {
        return ret_int(Err(sys::EINVAL));
    }
    (unsafe { (*set)[0] } >> (sig - 1) & 1) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigaction(sig: c_int, act: *const sigaction, old: *mut sigaction) -> c_int {
    if !valid_signal(sig) {
        return ret_int(Err(sys::EINVAL));
    }
    let kact = unsafe { act.as_ref() }.map(|a| KSigaction {
        handler: a.sa_sigaction,
        // the kernel returns from handlers through our `rt_sigreturn`
        flags: a.sa_flags as c_uint as usize | sys::SA_RESTORER,
        restorer: crate::arch::restore_rt as unsafe extern "C" fn() -> ! as usize,
        mask: a.sa_mask[0],
    });
    let mut kold = KSigaction { handler: 0, flags: 0, restorer: 0, mask: 0 };
    let kact_ptr = kact.as_ref().map_or(core::ptr::null(), |k| k as *const KSigaction);
    if let Err(e) = unsafe { sys::rt_sigaction(sig as usize, kact_ptr, &mut kold) } {
        return ret_int(Err(e));
    }
    if let Some(old) = unsafe { old.as_mut() } {
        old.sa_sigaction = kold.handler;
        old.sa_mask = [0; 16];
        old.sa_mask[0] = kold.mask;
        old.sa_flags = kold.flags as c_int;
        old.sa_restorer = kold.restorer;
    }
    0
}

const SIG_ERR: usize = !0;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn signal(sig: c_int, handler: usize) -> usize {
    let act = sigaction { sa_sigaction: handler, sa_mask: [0; 16], sa_flags: SA_RESTART as c_int, sa_restorer: 0 };
    let mut old = sigaction { sa_sigaction: 0, sa_mask: [0; 16], sa_flags: 0, sa_restorer: 0 };
    if unsafe { sigaction(sig, &act, &mut old) } < 0 { SIG_ERR } else { old.sa_sigaction }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigprocmask(how: c_int, set: *const sigset_t, old: *mut sigset_t) -> c_int {
    let mut kold = 0u64;
    let kset = unsafe { set.as_ref() }.map(|s| s[0]);
    let kset_ptr = kset.as_ref().map_or(core::ptr::null(), |k| k as *const u64);
    if let Err(e) = unsafe { sys::rt_sigprocmask(how as usize, kset_ptr, &mut kold) } {
        return ret_int(Err(e));
    }
    if let Some(old) = unsafe { old.as_mut() } {
        *old = [0; 16];
        old[0] = kold;
    }
    0
}

/// Returns the error rather than setting `errno`, as POSIX says.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_sigmask(how: c_int, set: *const sigset_t, old: *mut sigset_t) -> c_int {
    if unsafe { sigprocmask(how, set, old) } < 0 { unsafe { *errno::location() } } else { 0 }
}

#[unsafe(no_mangle)]
pub extern "C" fn raise(sig: c_int) -> c_int {
    ret_int(sys::tgkill(sys::getpid(), sys::gettid(), sig as usize))
}

#[unsafe(no_mangle)]
pub extern "C" fn kill(pid: pid_t, sig: c_int) -> c_int {
    ret_int(sys::kill(pid, sig as usize))
}

#[unsafe(no_mangle)]
pub extern "C" fn abort() -> ! {
    start::abort()
}

// ---- semaphores ----

/// musl's `sem_t` is eight ints; the first is the count.
#[repr(C)]
pub struct sem_t {
    count: AtomicU32,
    _rest: [c_int; 7],
}

const SEM_VALUE_MAX: u32 = i32::MAX as u32;
const EOVERFLOW: i32 = 75;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sem_init(sem: *mut sem_t, _pshared: c_int, value: c_uint) -> c_int {
    if value > SEM_VALUE_MAX {
        return ret_int(Err(sys::EINVAL));
    }
    unsafe { sem.write(sem_t { count: AtomicU32::new(value), _rest: [0; 7] }) };
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn sem_destroy(_sem: *mut sem_t) -> c_int {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sem_post(sem: *mut sem_t) -> c_int {
    let count = unsafe { &(*sem).count };
    if count.try_update(Ordering::Release, Ordering::Relaxed, |c| (c < SEM_VALUE_MAX).then_some(c + 1)).is_err() {
        return ret_int(Err(EOVERFLOW));
    }
    // shared futexes, so that process-shared semaphores work too
    let _ = unsafe { sys::futex(count.as_ptr(), sys::FUTEX_WAKE, 1, core::ptr::null()) };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sem_trywait(sem: *mut sem_t) -> c_int {
    let count = unsafe { &(*sem).count };
    match count.try_update(Ordering::Acquire, Ordering::Relaxed, |c| c.checked_sub(1)) {
        Ok(_) => 0,
        Err(_) => ret_int(Err(sys::EAGAIN)),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sem_wait(sem: *mut sem_t) -> c_int {
    let count = unsafe { &(*sem).count };
    loop {
        if count.try_update(Ordering::Acquire, Ordering::Relaxed, |c| c.checked_sub(1)).is_ok() {
            return 0;
        }
        futex::wait_shared(count, 0);
    }
}

// ---- users and groups ----

/// musl's `struct passwd`.
#[repr(C)]
pub struct passwd {
    pub pw_name: *mut c_char,
    pub pw_passwd: *mut c_char,
    pub pw_uid: uid_t,
    pub pw_gid: gid_t,
    pub pw_gecos: *mut c_char,
    pub pw_dir: *mut c_char,
    pub pw_shell: *mut c_char,
}

/// musl's `struct group`.
#[repr(C)]
pub struct group {
    pub gr_name: *mut c_char,
    pub gr_passwd: *mut c_char,
    pub gr_gid: gid_t,
    pub gr_mem: *mut *mut c_char,
}

/// Run `f` on the contents of the file at `path`, mapped read-only.
fn with_file<R>(path: &CStr, f: impl FnOnce(&[u8]) -> R) -> SysResult<R> {
    let fd = unsafe { sys::openat(AT_FDCWD, path.as_ptr().cast(), sys::O_RDONLY | sys::O_CLOEXEC, 0)? };
    let size = sys::lseek(fd, 0, 2 /* SEEK_END */);
    let r = size.and_then(|size| {
        if size == 0 {
            return Ok(f(&[]));
        }
        let p = unsafe { sys::mmap(0, size, sys::PROT_READ, sys::MAP_PRIVATE, fd, 0)? };
        let r = f(unsafe { core::slice::from_raw_parts(p as *const u8, size) });
        let _ = unsafe { sys::munmap(p, size) };
        Ok(r)
    });
    let _ = sys::close(fd);
    r
}

use crate::passwd::parse_uid as parse_id;

/// Which entry to look up: by name or by id.
#[derive(Clone, Copy)]
enum Key<'a> {
    Name(&'a [u8]),
    Id(u32),
}

/// The `:`-separated fields of the first line of `file` with `N` fields
/// whose name (field 0) or id (field 2) matches `key`.
fn find_entry<'f, const N: usize>(file: &'f [u8], key: Key<'_>) -> Option<[&'f [u8]; N]> {
    file.split(|&b| b == b'\n').find_map(|line| {
        let mut fields = [&[][..]; N];
        let mut it = line.split(|&b| b == b':');
        for f in fields.iter_mut() {
            *f = it.next()?;
        }
        let hit = match key {
            Key::Name(n) => fields[0] == n,
            Key::Id(id) => parse_id(fields[2]) == Some(id),
        };
        hit.then_some(fields)
    })
}

/// Copies strings into a caller's buffer, NUL-terminated.
struct Buf {
    p: *mut u8,
    left: usize,
}

impl Buf {
    /// Align the next allocation to `align`.
    fn align(&mut self, align: usize) -> Option<()> {
        let pad = (self.p as usize).next_multiple_of(align) - self.p as usize;
        self.left = self.left.checked_sub(pad)?;
        self.p = unsafe { self.p.add(pad) };
        Some(())
    }

    fn take(&mut self, n: usize) -> Option<*mut u8> {
        self.left = self.left.checked_sub(n)?;
        let p = self.p;
        self.p = unsafe { self.p.add(n) };
        Some(p)
    }

    fn str(&mut self, s: &[u8]) -> Option<*mut c_char> {
        let p = self.take(s.len() + 1)?;
        unsafe {
            core::ptr::copy_nonoverlapping(s.as_ptr(), p, s.len());
            *p.add(s.len()) = 0;
        }
        Some(p.cast())
    }
}

const ERANGE: c_int = sys::ERANGE;

/// The `getpw*_r` and `getgr*_r` convention: 0 with `*result` set (or null
/// when there is no such entry), or an error number.
fn lookup_r<T>(file: &CStr, fill: impl FnOnce(&[u8]) -> Option<Option<T>>, out: *mut T, result: *mut *mut T) -> c_int {
    unsafe { *result = core::ptr::null_mut() };
    match with_file(file, fill) {
        Ok(Some(Some(v))) => {
            unsafe {
                out.write(v);
                *result = out;
            }
            0
        }
        Ok(Some(None)) => 0,
        Ok(None) => ERANGE,
        // no database: no such entry
        Err(sys::ENOENT) => 0,
        Err(e) => e,
    }
}

/// Some(None) when there is no entry, None when `buf` is too small.
fn fill_passwd(file: &[u8], key: Key<'_>, buf: &mut Buf) -> Option<Option<passwd>> {
    let Some([name, pw, uid, gid, gecos, dir, shell]) = find_entry::<7>(file, key) else { return Some(None) };
    Some(Some(passwd {
        pw_name: buf.str(name)?,
        pw_passwd: buf.str(pw)?,
        pw_uid: parse_id(uid).unwrap_or(0),
        pw_gid: parse_id(gid).unwrap_or(0),
        pw_gecos: buf.str(gecos)?,
        pw_dir: buf.str(dir)?,
        pw_shell: buf.str(shell)?,
    }))
}

fn fill_group(file: &[u8], key: Key<'_>, buf: &mut Buf) -> Option<Option<group>> {
    let Some([name, pw, gid, members]) = find_entry::<4>(file, key) else { return Some(None) };
    let members = || members.split(|&b| b == b',').filter(|m| !m.is_empty());
    buf.align(align_of::<*mut c_char>())?;
    let n = members().count();
    let mem = buf.take((n + 1) * size_of::<*mut c_char>())? as *mut *mut c_char;
    for (i, m) in members().enumerate() {
        unsafe { *mem.add(i) = buf.str(m)? };
    }
    unsafe { *mem.add(n) = core::ptr::null_mut() };
    Some(Some(group { gr_name: buf.str(name)?, gr_passwd: buf.str(pw)?, gr_gid: parse_id(gid).unwrap_or(0), gr_mem: mem }))
}

unsafe fn pw_r(key: Key<'_>, pwd: *mut passwd, buf: *mut c_char, len: size_t, result: *mut *mut passwd) -> c_int {
    let mut b = Buf { p: buf.cast(), left: len };
    lookup_r(c"/etc/passwd", |f| fill_passwd(f, key, &mut b), pwd, result)
}

unsafe fn gr_r(key: Key<'_>, grp: *mut group, buf: *mut c_char, len: size_t, result: *mut *mut group) -> c_int {
    let mut b = Buf { p: buf.cast(), left: len };
    lookup_r(c"/etc/group", |f| fill_group(f, key, &mut b), grp, result)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getpwuid_r(uid: uid_t, pwd: *mut passwd, buf: *mut c_char, len: size_t, result: *mut *mut passwd) -> c_int {
    unsafe { pw_r(Key::Id(uid), pwd, buf, len, result) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getpwnam_r(
    name: *const c_char,
    pwd: *mut passwd,
    buf: *mut c_char,
    len: size_t,
    result: *mut *mut passwd,
) -> c_int {
    unsafe { pw_r(Key::Name(CStr::from_ptr(name).to_bytes()), pwd, buf, len, result) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getgrgid_r(gid: gid_t, grp: *mut group, buf: *mut c_char, len: size_t, result: *mut *mut group) -> c_int {
    unsafe { gr_r(Key::Id(gid), grp, buf, len, result) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getgrnam_r(
    name: *const c_char,
    grp: *mut group,
    buf: *mut c_char,
    len: size_t,
    result: *mut *mut group,
) -> c_int {
    unsafe { gr_r(Key::Name(CStr::from_ptr(name).to_bytes()), grp, buf, len, result) }
}

/// The static storage of `getpwnam`, `getpwuid`, `getgrnam` and `getgrgid`,
/// which (as in C) later calls overwrite.
static mut PW: passwd = passwd {
    pw_name: core::ptr::null_mut(),
    pw_passwd: core::ptr::null_mut(),
    pw_uid: 0,
    pw_gid: 0,
    pw_gecos: core::ptr::null_mut(),
    pw_dir: core::ptr::null_mut(),
    pw_shell: core::ptr::null_mut(),
};
static mut GR: group = group { gr_name: core::ptr::null_mut(), gr_passwd: core::ptr::null_mut(), gr_gid: 0, gr_mem: core::ptr::null_mut() };
static mut STATIC_BUF: [u64; 512] = [0; 512];

/// The non-reentrant form of a `_r` lookup, on the static storage.
unsafe fn non_reentrant<T>(r: impl FnOnce(*mut c_char, size_t, *mut *mut T) -> c_int) -> *mut T {
    let mut result = core::ptr::null_mut();
    match r((&raw mut STATIC_BUF).cast(), size_of::<[u64; 512]>(), &mut result) {
        0 => result,
        e => {
            set_errno(e);
            core::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getpwuid(uid: uid_t) -> *mut passwd {
    unsafe { non_reentrant(|b, l, r| getpwuid_r(uid, &raw mut PW, b, l, r)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getpwnam(name: *const c_char) -> *mut passwd {
    unsafe { non_reentrant(|b, l, r| getpwnam_r(name, &raw mut PW, b, l, r)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getgrgid(gid: gid_t) -> *mut group {
    unsafe { non_reentrant(|b, l, r| getgrgid_r(gid, &raw mut GR, b, l, r)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getgrnam(name: *const c_char) -> *mut group {
    unsafe { non_reentrant(|b, l, r| getgrnam_r(name, &raw mut GR, b, l, r)) }
}

// ---- processes ----

#[unsafe(no_mangle)]
pub extern "C" fn fork() -> pid_t {
    // a child with a copy of the address space, like the kernel's fork
    ret_int(unsafe { sys::syscall(nr::CLONE, [sys::SIGCHLD, 0, 0, 0, 0, 0]) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waitpid(pid: pid_t, status: *mut c_int, options: c_int) -> pid_t {
    ret_int(unsafe { sys::syscall(nr::WAIT4, [pid as usize, status as usize, options as usize, 0, 0, 0]) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wait4(pid: pid_t, status: *mut c_int, options: c_int, rusage: *mut c_void) -> pid_t {
    ret_int(unsafe { sys::syscall(nr::WAIT4, [pid as usize, status as usize, options as usize, rusage as usize, 0, 0]) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn execve(path: *const c_char, argv: *const *const c_char, envp: *const *const c_char) -> c_int {
    ret_int(unsafe { sys::syscall(nr::EXECVE, [path as usize, argv as usize, envp as usize, 0, 0, 0]) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn execv(path: *const c_char, argv: *const *const c_char) -> c_int {
    unsafe { execve(path, argv, start::envp().cast()) }
}

/// `execvp`: `file` as a path if it has a `/`, else searched for in `$PATH`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn execvp(file: *const c_char, argv: *const *const c_char) -> c_int {
    let name = unsafe { CStr::from_ptr(file) }.to_bytes();
    if name.is_empty() {
        return ret_int(Err(sys::ENOENT));
    }
    if name.contains(&b'/') {
        return unsafe { execv(file, argv) };
    }
    let path = unsafe { getenv(c"PATH".as_ptr()) };
    let path = if path.is_null() { b"/usr/local/bin:/bin:/usr/bin" } else { unsafe { CStr::from_ptr(path) }.to_bytes() };
    let mut err = sys::ENOENT;
    let mut full = [0u8; 4096];
    for dir in path.split(|&b| b == b':') {
        let dir = if dir.is_empty() { &b"."[..] } else { dir };
        let len = dir.len() + 1 + name.len();
        if len >= full.len() {
            err = sys::ENAMETOOLONG;
            continue;
        }
        full[..dir.len()].copy_from_slice(dir);
        full[dir.len()] = b'/';
        full[dir.len() + 1..len].copy_from_slice(name);
        full[len] = 0;
        unsafe { execv(full.as_ptr().cast(), argv) };
        // keep looking past directories without it; remember why it failed
        match unsafe { *errno::location() } {
            sys::ENOENT | sys::ENOTDIR => {}
            e => err = e,
        }
    }
    ret_int(Err(err))
}

// ---- dynamic linking (there is none) ----

/// glibc's and musl's `struct dl_phdr_info`.
#[repr(C)]
pub struct dl_phdr_info {
    pub dlpi_addr: usize,
    pub dlpi_name: *const c_char,
    pub dlpi_phdr: *const elf::Phdr,
    pub dlpi_phnum: u16,
    pub dlpi_adds: u64,
    pub dlpi_subs: u64,
    pub dlpi_tls_modid: usize,
    pub dlpi_tls_data: *mut c_void,
}

/// Calls `callback` for the one object loaded: the executable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dl_iterate_phdr(
    callback: Option<unsafe extern "C" fn(*mut dl_phdr_info, size_t, *mut c_void) -> c_int>,
    data: *mut c_void,
) -> c_int {
    let Some(callback) = callback else { return 0 };
    let phdrs = elf::program_headers();
    let mut info = dl_phdr_info {
        dlpi_addr: elf::executable_bias(),
        dlpi_name: c"".as_ptr(),
        dlpi_phdr: phdrs.as_ptr(),
        dlpi_phnum: phdrs.len() as u16,
        dlpi_adds: 1,
        dlpi_subs: 0,
        dlpi_tls_modid: 0,
        dlpi_tls_data: core::ptr::null_mut(),
    };
    unsafe { callback(&mut info, size_of::<dl_phdr_info>(), data) }
}

/// No shared objects, so no symbols to find: callers fall back as they do
/// when an optional symbol is missing.
#[unsafe(no_mangle)]
pub extern "C" fn dlsym(_handle: *mut c_void, _name: *const c_char) -> *mut c_void {
    core::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn dlopen(_path: *const c_char, _flags: c_int) -> *mut c_void {
    core::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn dlclose(_handle: *mut c_void) -> c_int {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn dlerror() -> *mut c_char {
    c"dynamic linking is not supported".as_ptr() as *mut c_char
}

// ---- more file descriptors ----

#[unsafe(no_mangle)]
pub extern "C" fn fcntl(fd: c_int, cmd: c_int, arg: usize) -> c_int {
    ret_int(sys::fcntl(fd, cmd as u32, arg))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pread(fd: c_int, buf: *mut c_void, n: size_t, off: i64) -> ssize_t {
    ret(unsafe { sys::pread(fd, buf.cast(), n, off as u64) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pwrite(fd: c_int, buf: *const c_void, n: size_t, off: i64) -> ssize_t {
    ret(unsafe { sys::pwrite(fd, buf.cast(), n, off as u64) })
}

#[unsafe(no_mangle)]
pub extern "C" fn fchmod(fd: c_int, mode: c_uint) -> c_int {
    ret_int(sys::fchmod(fd, mode))
}

#[unsafe(no_mangle)]
pub extern "C" fn umask(mask: c_uint) -> c_uint {
    (unsafe { sys::syscall(nr::UMASK, [mask as usize, 0, 0, 0, 0, 0]) }).unwrap_or(0) as c_uint
}

const AT_SYMLINK_NOFOLLOW: c_int = 0x100;
const AT_EMPTY_PATH: c_int = 0x1000;

/// `struct stat` is the kernel's on x86_64 and aarch64, in musl as here.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fstatat(dirfd: c_int, path: *const c_char, st: *mut c_void, flags: c_int) -> c_int {
    ret_int(unsafe { sys::syscall(nr::NEWFSTATAT, [dirfd as usize, path as usize, st as usize, flags as usize, 0, 0]) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stat(path: *const c_char, st: *mut c_void) -> c_int {
    unsafe { fstatat(AT_FDCWD, path, st, 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lstat(path: *const c_char, st: *mut c_void) -> c_int {
    unsafe { fstatat(AT_FDCWD, path, st, AT_SYMLINK_NOFOLLOW) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fstat(fd: c_int, st: *mut c_void) -> c_int {
    unsafe { fstatat(fd, c"".as_ptr(), st, AT_EMPTY_PATH) }
}

/// musl's `struct statvfs`.
#[repr(C)]
pub struct statvfs {
    pub f_bsize: c_ulong,
    pub f_frsize: c_ulong,
    pub f_blocks: u64,
    pub f_bfree: u64,
    pub f_bavail: u64,
    pub f_files: u64,
    pub f_ffree: u64,
    pub f_favail: u64,
    pub f_fsid: c_ulong,
    pub f_flag: c_ulong,
    pub f_namemax: c_ulong,
    __f_spare: [c_int; 6],
}

/// The kernel's `struct statfs` on 64-bit targets.
#[repr(C)]
struct KStatfs {
    f_type: i64,
    f_bsize: i64,
    f_blocks: u64,
    f_bfree: u64,
    f_bavail: u64,
    f_files: u64,
    f_ffree: u64,
    f_fsid: [i32; 2],
    f_namelen: i64,
    f_frsize: i64,
    f_flags: i64,
    f_spare: [i64; 4],
}

/// `statvfs` from `statfs`, as musl converts it.
fn to_statvfs(k: &KStatfs, out: *mut statvfs) {
    const ST_VALID: i64 = 0x20;
    let v = statvfs {
        f_bsize: k.f_bsize as c_ulong,
        f_frsize: if k.f_frsize != 0 { k.f_frsize } else { k.f_bsize } as c_ulong,
        f_blocks: k.f_blocks,
        f_bfree: k.f_bfree,
        f_bavail: k.f_bavail,
        f_files: k.f_files,
        f_ffree: k.f_ffree,
        f_favail: k.f_ffree,
        f_fsid: k.f_fsid[0] as u32 as c_ulong,
        f_flag: (k.f_flags & !ST_VALID) as c_ulong,
        f_namemax: k.f_namelen as c_ulong,
        __f_spare: [0; 6],
    };
    unsafe { out.write(v) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn statvfs(path: *const c_char, out: *mut statvfs) -> c_int {
    let mut k = unsafe { core::mem::zeroed::<KStatfs>() };
    if let Err(e) = unsafe { sys::syscall(nr::STATFS, [path as usize, &raw mut k as usize, 0, 0, 0, 0]) } {
        return ret_int(Err(e));
    }
    to_statvfs(&k, out);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fstatvfs(fd: c_int, out: *mut statvfs) -> c_int {
    let mut k = unsafe { core::mem::zeroed::<KStatfs>() };
    if let Err(e) = unsafe { sys::syscall(nr::FSTATFS, [fd as usize, &raw mut k as usize, 0, 0, 0, 0]) } {
        return ret_int(Err(e));
    }
    to_statvfs(&k, out);
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn eventfd(initval: c_uint, flags: c_int) -> c_int {
    ret_int(unsafe { sys::syscall(nr::EVENTFD2, [initval as usize, flags as usize, 0, 0, 0, 0]) })
}

#[unsafe(no_mangle)]
pub extern "C" fn inotify_init1(flags: c_int) -> c_int {
    ret_int(unsafe { sys::syscall(nr::INOTIFY_INIT1, [flags as usize, 0, 0, 0, 0, 0]) })
}

#[unsafe(no_mangle)]
pub extern "C" fn inotify_init() -> c_int {
    inotify_init1(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn inotify_add_watch(fd: c_int, path: *const c_char, mask: u32) -> c_int {
    ret_int(unsafe { sys::syscall(nr::INOTIFY_ADD_WATCH, [fd as usize, path as usize, mask as usize, 0, 0, 0]) })
}

#[unsafe(no_mangle)]
pub extern "C" fn inotify_rm_watch(fd: c_int, wd: c_int) -> c_int {
    ret_int(unsafe { sys::syscall(nr::INOTIFY_RM_WATCH, [fd as usize, wd as usize, 0, 0, 0, 0]) })
}

// ---- terminals ----

const TCSETS: usize = 0x5402;
const TCFLSH: usize = 0x540B;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tcgetattr(fd: c_int, termios: *mut c_void) -> c_int {
    ret_int(unsafe { sys::ioctl(fd, TCGETS, termios as usize) })
}

/// `optional_actions` is `TCSANOW` (0), `TCSADRAIN` (1) or `TCSAFLUSH` (2),
/// which select `TCSETS`, `TCSETSW` and `TCSETSF`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tcsetattr(fd: c_int, optional_actions: c_int, termios: *const c_void) -> c_int {
    if !(0..=2).contains(&optional_actions) {
        return ret_int(Err(sys::EINVAL));
    }
    ret_int(unsafe { sys::ioctl(fd, TCSETS + optional_actions as usize, termios as usize) })
}

#[unsafe(no_mangle)]
pub extern "C" fn tcflush(fd: c_int, queue: c_int) -> c_int {
    ret_int(unsafe { sys::ioctl(fd, TCFLSH, queue as usize) })
}

// ---- more sockets ----

#[unsafe(no_mangle)]
pub extern "C" fn socket(domain: c_int, ty: c_int, proto: c_int) -> c_int {
    ret_int(unsafe { sys::syscall(nr::SOCKET, [domain as usize, ty as usize, proto as usize, 0, 0, 0]) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn connect(fd: c_int, addr: *const c_void, len: u32) -> c_int {
    ret_int(unsafe { sys::syscall(nr::CONNECT, [fd as usize, addr as usize, len as usize, 0, 0, 0]) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bind(fd: c_int, addr: *const c_void, len: u32) -> c_int {
    ret_int(unsafe { sys::syscall(nr::BIND, [fd as usize, addr as usize, len as usize, 0, 0, 0]) })
}

#[unsafe(no_mangle)]
pub extern "C" fn listen(fd: c_int, backlog: c_int) -> c_int {
    ret_int(unsafe { sys::syscall(nr::LISTEN, [fd as usize, backlog as usize, 0, 0, 0, 0]) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn accept4(fd: c_int, addr: *mut c_void, len: *mut u32, flags: c_int) -> c_int {
    ret_int(unsafe { sys::syscall(nr::ACCEPT4, [fd as usize, addr as usize, len as usize, flags as usize, 0, 0]) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn accept(fd: c_int, addr: *mut c_void, len: *mut u32) -> c_int {
    unsafe { accept4(fd, addr, len, 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn setsockopt(fd: c_int, level: c_int, name: c_int, val: *const c_void, len: u32) -> c_int {
    let args = [fd as usize, level as usize, name as usize, val as usize, len as usize, 0];
    ret_int(unsafe { sys::syscall(nr::SETSOCKOPT, args) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getsockopt(fd: c_int, level: c_int, name: c_int, val: *mut c_void, len: *mut u32) -> c_int {
    let args = [fd as usize, level as usize, name as usize, val as usize, len as usize, 0];
    ret_int(unsafe { sys::syscall(nr::GETSOCKOPT, args) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sendmsg(fd: c_int, msg: *const c_void, flags: c_int) -> ssize_t {
    ret(unsafe { sys::sendmsg(fd, msg.cast(), flags as u32) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn recvmsg(fd: c_int, msg: *mut c_void, flags: c_int) -> ssize_t {
    ret(unsafe { sys::recvmsg(fd, msg.cast(), flags as u32) })
}

// ---- more signals, time and resources ----

/// Returns the error rather than setting `errno`, as POSIX says.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigwait(set: *const sigset_t, sig: *mut c_int) -> c_int {
    loop {
        let args = [set as usize, 0, 0, 8, 0, 0];
        match unsafe { sys::syscall(nr::RT_SIGTIMEDWAIT, args) } {
            Ok(s) => {
                unsafe { *sig = s as c_int };
                return 0;
            }
            Err(sys::EINTR) => {}
            Err(e) => return e,
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn clock_gettime(clock: c_int, ts: *mut sys::Timespec) -> c_int {
    ret_int(unsafe { sys::syscall(nr::CLOCK_GETTIME, [clock as usize, ts as usize, 0, 0, 0, 0]) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getrusage(who: c_int, usage: *mut c_void) -> c_int {
    ret_int(unsafe { sys::syscall(nr::GETRUSAGE, [who as usize, usage as usize, 0, 0, 0, 0]) })
}

/// `struct sysinfo` is the kernel's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getrlimit(resource: c_int, rlim: *mut [u64; 2]) -> c_int {
    ret_int(unsafe { sys::syscall(nr::PRLIMIT64, [0, resource as usize, 0, rlim as usize, 0, 0]) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn setrlimit(resource: c_int, rlim: *const [u64; 2]) -> c_int {
    ret_int(unsafe { sys::syscall(nr::PRLIMIT64, [0, resource as usize, rlim as usize, 0, 0, 0]) })
}

/// Listing interfaces takes a netlink client, which there is not: callers
/// get `ENOSYS`, as from a kernel without the feature.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getifaddrs(ifap: *mut *mut c_void) -> c_int {
    unsafe { *ifap = core::ptr::null_mut() };
    ret_int(Err(sys::ENOSYS))
}

#[unsafe(no_mangle)]
pub extern "C" fn freeifaddrs(_ifa: *mut c_void) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sysinfo(info: *mut c_void) -> c_int {
    ret_int(unsafe { sys::syscall(nr::SYSINFO, [info as usize, 0, 0, 0, 0, 0]) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn prctl(op: c_int, a: c_ulong, b: c_ulong, c: c_ulong, d: c_ulong) -> c_int {
    ret_int(unsafe { sys::prctl(op as usize, a as usize, b as usize, c as usize, d as usize) })
}

// ---- mount tables ----

/// A `FILE` for reading lines, enough for the `*mntent` functions: other
/// stdio functions are not provided.
pub struct FILE {
    fd: c_int,
    pos: usize,
    len: usize,
    buf: [u8; 4096],
    line: [u8; 4096],
}

/// Room for a `FILE`, from the kernel: there is no allocator here.
const FILE_PAGES: usize = size_of::<FILE>().next_multiple_of(sys::PAGE_SIZE);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fopen(path: *const c_char, mode: *const c_char) -> *mut FILE {
    // only reading is supported
    if unsafe { *mode } != b'r' as c_char || unsafe { CStr::from_ptr(mode) }.to_bytes().contains(&b'+') {
        set_errno(sys::EINVAL);
        return core::ptr::null_mut();
    }
    let fd = match unsafe { sys::openat(AT_FDCWD, path.cast(), sys::O_RDONLY | sys::O_CLOEXEC, 0) } {
        Ok(fd) => fd,
        Err(e) => {
            set_errno(e);
            return core::ptr::null_mut();
        }
    };
    let rw = sys::PROT_READ | sys::PROT_WRITE;
    match unsafe { sys::mmap(0, FILE_PAGES, rw, sys::MAP_PRIVATE | sys::MAP_ANONYMOUS, -1, 0) } {
        Ok(p) => {
            let f = p as *mut FILE;
            unsafe {
                (*f).fd = fd;
                (*f).pos = 0;
                (*f).len = 0;
            }
            f
        }
        Err(e) => {
            let _ = sys::close(fd);
            set_errno(e);
            core::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fclose(f: *mut FILE) -> c_int {
    let r = sys::close(unsafe { (*f).fd });
    let _ = unsafe { sys::munmap(f as usize, FILE_PAGES) };
    ret_int(r)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fileno(f: *mut FILE) -> c_int {
    unsafe { (*f).fd }
}

/// The next line of `f` without its newline (a longer one is cut short),
/// or None at the end of the file.
fn next_line(f: &mut FILE) -> Option<&[u8]> {
    let mut n = 0;
    loop {
        if f.pos == f.len {
            f.pos = 0;
            f.len = unsafe { sys::read(f.fd, f.buf.as_mut_ptr(), f.buf.len()) }.unwrap_or(0);
            if f.len == 0 {
                return (n > 0).then_some(&f.line[..n]);
            }
        }
        let b = f.buf[f.pos];
        f.pos += 1;
        if b == b'\n' {
            return Some(&f.line[..n]);
        }
        if n < f.line.len() {
            f.line[n] = b;
            n += 1;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fgets(s: *mut c_char, size: c_int, f: *mut FILE) -> *mut c_char {
    let Some(line) = next_line(unsafe { &mut *f }) else { return core::ptr::null_mut() };
    if size <= 0 {
        return core::ptr::null_mut();
    }
    // the newline is kept, as far as there is room
    let n = line.len().min(size as usize - 1);
    unsafe {
        core::ptr::copy_nonoverlapping(line.as_ptr(), s.cast(), n);
        let mut end = n;
        if end < size as usize - 1 {
            *s.add(end) = b'\n' as c_char;
            end += 1;
        }
        *s.add(end) = 0;
    }
    s
}

/// glibc's and musl's `struct mntent`.
#[repr(C)]
pub struct mntent {
    pub mnt_fsname: *mut c_char,
    pub mnt_dir: *mut c_char,
    pub mnt_type: *mut c_char,
    pub mnt_opts: *mut c_char,
    pub mnt_freq: c_int,
    pub mnt_passno: c_int,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn setmntent(path: *const c_char, mode: *const c_char) -> *mut FILE {
    unsafe { fopen(path, mode) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn endmntent(f: *mut FILE) -> c_int {
    if !f.is_null() {
        unsafe { fclose(f) };
    }
    1
}

/// Copy a field of a mount table line into `buf`, decoding the `\ooo`
/// escapes of spaces, tabs, newlines and backslashes.
fn mnt_field(field: &[u8], buf: &mut Buf) -> Option<*mut c_char> {
    let p = buf.take(field.len() + 1)?;
    let mut n = 0;
    let mut i = 0;
    while i < field.len() {
        let oct = field.get(i + 1..i + 4).filter(|o| field[i] == b'\\' && o.iter().all(|d| (b'0'..=b'7').contains(d)));
        let b = match oct {
            Some(o) => {
                i += 4;
                (o[0] - b'0') << 6 | (o[1] - b'0') << 3 | (o[2] - b'0')
            }
            None => {
                i += 1;
                field[i - 1]
            }
        };
        unsafe { *p.add(n) = b };
        n += 1;
    }
    unsafe { *p.add(n) = 0 };
    Some(p.cast())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getmntent_r(f: *mut FILE, ent: *mut mntent, buf: *mut c_char, len: c_int) -> *mut mntent {
    let f = unsafe { &mut *f };
    loop {
        let Some(line) = next_line(f) else { return core::ptr::null_mut() };
        let mut fields = line.split(|b| b.is_ascii_whitespace()).filter(|s| !s.is_empty());
        let Some(fsname) = fields.next() else { continue };
        if fsname.starts_with(b"#") {
            continue;
        }
        let (dir, ty, opts) = (fields.next().unwrap_or(b""), fields.next().unwrap_or(b""), fields.next().unwrap_or(b""));
        let num = |s: Option<&[u8]>| s.and_then(parse_id).unwrap_or(0) as c_int;
        let (freq, passno) = (num(fields.next()), num(fields.next()));
        let mut b = Buf { p: buf.cast(), left: len.max(0) as usize };
        let filled = (|| {
            Some(mntent {
                mnt_fsname: mnt_field(fsname, &mut b)?,
                mnt_dir: mnt_field(dir, &mut b)?,
                mnt_type: mnt_field(ty, &mut b)?,
                mnt_opts: mnt_field(opts, &mut b)?,
                mnt_freq: freq,
                mnt_passno: passno,
            })
        })();
        // a line too long for `buf` is skipped, as in musl
        if let Some(m) = filled {
            unsafe { ent.write(m) };
            return ent;
        }
    }
}

static mut MNT: mntent = mntent {
    mnt_fsname: core::ptr::null_mut(),
    mnt_dir: core::ptr::null_mut(),
    mnt_type: core::ptr::null_mut(),
    mnt_opts: core::ptr::null_mut(),
    mnt_freq: 0,
    mnt_passno: 0,
};
static mut MNT_BUF: [u8; 4096] = [0; 4096];

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getmntent(f: *mut FILE) -> *mut mntent {
    unsafe { getmntent_r(f, &raw mut MNT, (&raw mut MNT_BUF).cast(), 4096) }
}
