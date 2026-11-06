//! Process startup and shutdown, panic handling.

use core::cell::UnsafeCell;
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use linux_none_rt as rt;
use rt::arch::KSigaction;

use crate::sys::linux;
use crate::sys::tls::ThreadData;

pub use rt::arch::_start;

static TLS_READY: AtomicBool = AtomicBool::new(false);

struct MainThreadData(UnsafeCell<ThreadData>);
unsafe impl Sync for MainThreadData {}
static MAIN_THREAD: MainThreadData = MainThreadData(UnsafeCell::new(ThreadData::new()));

/// Called from the C `main` that [`crate::main!`] defines, which the
/// runtime's `_start` calls.
pub fn lang_start(main: impl FnOnce() -> i32) -> i32 {
    unsafe {
        ThreadData::install(MAIN_THREAD.0.get());
        TLS_READY.store(true, Ordering::Relaxed);
        (*MAIN_THREAD.0.get()).set_thread(crate::thread::Thread::new_main());
    }
    rt::start::set_exit_hook(crate::io::stdio::cleanup);

    // like std: writes to a closed pipe return EPIPE instead of killing us
    let act = KSigaction { handler: linux::SIG_IGN, flags: 0, restorer: 0, mask: 0 };
    let _ = unsafe { linux::rt_sigaction(linux::SIGPIPE, &act, core::ptr::null_mut()) };
    main()
}

/// Flush stdio and terminate the process.
pub(crate) fn exit(code: i32) -> ! {
    rt::start::exit(code)
}

pub(crate) fn auxv(key: usize) -> Option<usize> {
    rt::start::auxv(key)
}

pub(crate) fn argv() -> &'static [*const u8] {
    let (argc, argv) = rt::start::args();
    if argv.is_null() {
        return &[];
    }
    unsafe { core::slice::from_raw_parts(argv, argc) }
}

pub(crate) fn envp() -> *const *const u8 {
    rt::start::envp()
}

pub(crate) fn abort() -> ! {
    rt::start::abort()
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
