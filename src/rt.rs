//! Process startup and shutdown, panic handling.

use core::cell::UnsafeCell;
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};

use crate::sys::linux;
use crate::sys::tls::ThreadData;

pub use crate::sys::arch::_start;

static ARGC: AtomicUsize = AtomicUsize::new(0);
static ARGV: AtomicPtr<*const u8> = AtomicPtr::new(core::ptr::null_mut());
static ENVP: AtomicPtr<*const u8> = AtomicPtr::new(core::ptr::null_mut());
static AUXV: AtomicPtr<usize> = AtomicPtr::new(core::ptr::null_mut());
static TLS_READY: AtomicBool = AtomicBool::new(false);

struct MainThreadData(UnsafeCell<ThreadData>);
unsafe impl Sync for MainThreadData {}
static MAIN_THREAD: MainThreadData = MainThreadData(UnsafeCell::new(ThreadData::new()));

unsafe extern "Rust" {
    /// Defined by the application through [`crate::main!`].
    fn __rust_std_main() -> i32;
}

/// Called from `_start` with the initial stack pointer.
pub(crate) unsafe extern "C" fn start(sp: *const usize) -> ! {
    unsafe {
        let argc = *sp;
        let argv = sp.add(1) as *mut *const u8;
        ARGC.store(argc, Ordering::Relaxed);
        ARGV.store(argv, Ordering::Relaxed);
        let envp = argv.add(argc + 1);
        ENVP.store(envp, Ordering::Relaxed);
        let mut p = envp;
        while !(*p).is_null() {
            p = p.add(1);
        }
        AUXV.store(p.add(1) as *mut usize, Ordering::Relaxed);

        ThreadData::install(MAIN_THREAD.0.get());
        TLS_READY.store(true, Ordering::Relaxed);

        run_init_array();
        (*MAIN_THREAD.0.get()).set_thread(crate::thread::Thread::new_main());

        // like std: writes to a closed pipe return EPIPE instead of killing us
        let act = crate::sys::arch::KSigaction { handler: linux::SIG_IGN, flags: 0, restorer: 0, mask: 0 };
        let _ = linux::rt_sigaction(linux::SIGPIPE, &act, core::ptr::null_mut());

        let code = __rust_std_main();
        exit(code)
    }
}

/// Flush stdio and terminate the process.
pub(crate) fn exit(code: i32) -> ! {
    crate::io::stdio::cleanup();
    linux::exit_group(code)
}

/// Run static constructors (`.init_array`), as crt startup code would.
/// compiler-builtins uses one on aarch64 to detect LSE atomics.
unsafe fn run_init_array() {
    unsafe extern "C" {
        static __init_array_start: [unsafe extern "C" fn(); 0];
        static __init_array_end: [unsafe extern "C" fn(); 0];
    }
    unsafe {
        let mut f = &raw const __init_array_start as *const unsafe extern "C" fn();
        let end = &raw const __init_array_end as *const unsafe extern "C" fn();
        while f < end {
            (*f)();
            f = f.add(1);
        }
    }
}

/// Look up an entry of the ELF auxiliary vector the kernel passed us.
pub(crate) fn auxv(key: usize) -> Option<usize> {
    let mut p = AUXV.load(Ordering::Relaxed) as *const usize;
    if p.is_null() {
        return None;
    }
    unsafe {
        while *p != 0 {
            if *p == key {
                return Some(*p.add(1));
            }
            p = p.add(2);
        }
    }
    None
}

/// libc's `getauxval`, which compiler-builtins calls on aarch64.
#[cfg(all(target_arch = "aarch64", not(kani)))]
#[unsafe(no_mangle)]
extern "C" fn getauxval(key: core::ffi::c_ulong) -> core::ffi::c_ulong {
    auxv(key as usize).unwrap_or(0) as core::ffi::c_ulong
}

pub(crate) fn argv() -> &'static [*const u8] {
    let argv = ARGV.load(Ordering::Relaxed);
    if argv.is_null() {
        return &[];
    }
    unsafe { core::slice::from_raw_parts(argv, ARGC.load(Ordering::Relaxed)) }
}

pub(crate) fn envp() -> *const *const u8 {
    ENVP.load(Ordering::Relaxed)
}

/// Raise SIGABRT with the default disposition, as `std::process::abort` does.
pub(crate) fn abort() -> ! {
    unsafe {
        let act = crate::sys::arch::KSigaction { handler: linux::SIG_DFL, flags: 0, restorer: 0, mask: 0 };
        let _ = linux::rt_sigaction(linux::SIGABRT, &act, core::ptr::null_mut());
        let set: u64 = 1 << (linux::SIGABRT - 1);
        let _ = linux::rt_sigprocmask(linux::SIG_UNBLOCK, &set, core::ptr::null_mut());
    }
    let _ = linux::tgkill(linux::getpid(), linux::gettid(), linux::SIGABRT);
    linux::exit_group(134)
}

#[cfg_attr(test, allow(dead_code))]
pub(crate) struct RawStderr;

impl Write for RawStderr {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        linux::write_all_raw(2, s.as_bytes());
        Ok(())
    }
}

#[cfg(not(any(test, kani)))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    static PANICKING: AtomicUsize = AtomicUsize::new(0);
    if PANICKING.fetch_add(1, Ordering::Relaxed) > 0 {
        // panicked while panicking (or two threads at once): don't recurse
        let _ = RawStderr.write_str("thread panicked while processing panic. aborting.\n");
        abort();
    }
    let mut out = RawStderr;
    let name = if TLS_READY.load(Ordering::Relaxed) {
        crate::sys::tls::current().thread().as_ref().and_then(|t| t.name().map(|n| unsafe { &*(n as *const str) }))
    } else {
        Some("main")
    };
    let _ = write!(out, "thread '{}' panicked at ", name.unwrap_or("<unnamed>"));
    if let Some(loc) = info.location() {
        let _ = write!(out, "{loc}");
    }
    let _ = writeln!(out, ":\n{}", info.message());
    let _ = out.write_str("note: rust_std is built with panic=abort; aborting\n");
    abort()
}

// The prebuilt `core`/`alloc` are compiled with unwinding tables that name
// these symbols. With panic=abort nothing ever unwinds, so they are never
// called; they only need to exist for the link to succeed.

#[cfg(not(kani))]
#[unsafe(no_mangle)]
extern "C" fn rust_eh_personality() {}

#[cfg(not(kani))]
#[unsafe(no_mangle)]
extern "C" fn _Unwind_Resume() -> ! {
    abort()
}
