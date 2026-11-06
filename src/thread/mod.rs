//! Native threads, created with `clone(2)` directly.

use alloc_crate::boxed::Box;
use alloc_crate::string::String;
use alloc_crate::sync::Arc;
use core::any::Any;
use core::cell::UnsafeCell;
use core::fmt;
use core::marker::PhantomData;
use core::mem::size_of;
use core::num::{NonZeroU64, NonZeroUsize};
use core::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed, Release};
use core::sync::atomic::{AtomicU32, AtomicU64};
use core::time::Duration;

use crate::io;
use crate::sys::linux::{self, PAGE_SIZE};
use crate::sys::tls::{self, ThreadData};
use crate::sys::{arch, futex};

mod local;
mod scoped;

pub use local::{AccessError, LocalKey};
pub use scoped::{Scope, ScopedJoinHandle, scope};

pub type Result<T> = core::result::Result<T, Box<dyn Any + Send + 'static>>;

const DEFAULT_STACK_SIZE: usize = 2 << 20;

// ---- ThreadId / Thread ----

#[derive(Eq, PartialEq, Clone, Copy, Hash, Debug, Ord, PartialOrd)]
pub struct ThreadId(NonZeroU64);

impl ThreadId {
    fn new() -> ThreadId {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        ThreadId(NonZeroU64::new(COUNTER.fetch_add(1, Relaxed)).unwrap())
    }

    pub fn as_u64(&self) -> NonZeroU64 {
        self.0
    }
}

const PARK_EMPTY: u32 = 0;
const PARK_NOTIFIED: u32 = 1;
const PARK_PARKED: u32 = u32::MAX;

struct Inner {
    name: Option<String>,
    id: ThreadId,
    parker: AtomicU32,
}

/// A handle to a thread.
#[derive(Clone)]
pub struct Thread {
    inner: Arc<Inner>,
}

impl Thread {
    fn new(name: Option<String>) -> Thread {
        Thread { inner: Arc::new(Inner { name, id: ThreadId::new(), parker: AtomicU32::new(PARK_EMPTY) }) }
    }

    pub(crate) fn new_main() -> Thread {
        Thread::new(Some(String::from("main")))
    }

    pub fn id(&self) -> ThreadId {
        self.inner.id
    }

    pub fn name(&self) -> Option<&str> {
        self.inner.name.as_deref()
    }

    pub fn unpark(&self) {
        if self.inner.parker.swap(PARK_NOTIFIED, Release) == PARK_PARKED {
            futex::wake_one(&self.inner.parker);
        }
    }

    fn park(&self, timeout: Option<Duration>) {
        let p = &self.inner.parker;
        // NOTIFIED -> EMPTY and return, or EMPTY -> PARKED and sleep
        if p.fetch_sub(1, Acquire) == PARK_NOTIFIED {
            return;
        }
        match timeout {
            None => loop {
                futex::wait(p, PARK_PARKED, None);
                if p.compare_exchange(PARK_NOTIFIED, PARK_EMPTY, Acquire, Relaxed).is_ok() {
                    return;
                }
            },
            Some(d) => {
                futex::wait(p, PARK_PARKED, Some(d));
                p.swap(PARK_EMPTY, Acquire);
            }
        }
    }
}

impl fmt::Debug for Thread {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Thread").field("id", &self.id()).field("name", &self.name()).finish_non_exhaustive()
    }
}

pub fn current() -> Thread {
    let td = tls::current();
    if let Some(t) = td.thread() {
        return t.clone();
    }
    let t = Thread::new(None);
    td.set_thread(t.clone());
    t
}

pub fn park() {
    current().park(None)
}

pub fn park_timeout(dur: Duration) {
    current().park(Some(dur))
}

pub fn sleep(dur: Duration) {
    let ts = linux::Timespec { sec: dur.as_secs().min(i64::MAX as u64) as i64, nsec: dur.subsec_nanos() as i64 };
    let _ = linux::nanosleep(&ts);
}

pub fn sleep_until(deadline: crate::time::Instant) {
    let now = crate::time::Instant::now();
    if let Some(d) = deadline.checked_duration_since(now) {
        sleep(d);
    }
}

pub fn yield_now() {
    linux::sched_yield()
}

/// Always false: the runtime aborts on panic instead of unwinding.
pub fn panicking() -> bool {
    false
}

pub fn available_parallelism() -> io::Result<NonZeroUsize> {
    let mut set = [0u8; 128];
    let n = crate::sys::fd::cvt(unsafe { linux::sched_getaffinity(set.as_mut_ptr(), set.len()) })?;
    let count = set[..n].iter().map(|b| b.count_ones() as usize).sum();
    NonZeroUsize::new(count).ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
}

// ---- spawning ----

#[derive(Debug, Default)]
pub struct Builder {
    name: Option<String>,
    stack_size: Option<usize>,
}

impl Builder {
    pub fn new() -> Builder {
        Builder::default()
    }

    pub fn name(mut self, name: String) -> Builder {
        self.name = Some(name);
        self
    }

    pub fn stack_size(mut self, size: usize) -> Builder {
        self.stack_size = Some(size);
        self
    }

    pub fn spawn<F, T>(self, f: F) -> io::Result<JoinHandle<T>>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        unsafe { self.spawn_unchecked(f) }
    }

    /// # Safety
    /// The caller must ensure the thread does not outlive borrowed data in `f`.
    pub unsafe fn spawn_unchecked<F, T>(self, f: F) -> io::Result<JoinHandle<T>>
    where
        F: FnOnce() -> T + Send,
        T: Send,
    {
        unsafe { self.spawn_inner(f, None).map(JoinHandle) }
    }

    pub(crate) unsafe fn spawn_inner<'scope, F, T>(
        self,
        f: F,
        scope: Option<Arc<scoped::ScopeData>>,
    ) -> io::Result<JoinInner<'scope, T>>
    where
        F: FnOnce() -> T + Send,
        T: Send,
    {
        let thread = Thread::new(self.name);
        let their_thread = thread.clone();
        let packet: Arc<Packet<'scope, T>> = Arc::new(Packet { scope, result: UnsafeCell::new(None), _marker: PhantomData });
        let their_packet = packet.clone();

        let main = move || {
            tls::current().set_thread(their_thread);
            let r = f();
            unsafe { *their_packet.result.get() = Some(r) };
            drop(their_packet);
        };
        let main: Box<dyn FnOnce() + Send + '_> = Box::new(main);
        // SAFETY: the caller guarantees the closure does not outlive its borrows
        let main: Box<dyn FnOnce() + Send + 'static> = unsafe { core::mem::transmute(main) };

        if let Some(s) = &packet.scope {
            s.increment_running();
        }
        match unsafe { native_spawn(self.stack_size.unwrap_or(DEFAULT_STACK_SIZE), main) } {
            Ok(native) => Ok(JoinInner { native: Some(native), thread, packet }),
            Err(e) => {
                if let Some(s) = &packet.scope {
                    s.decrement_running();
                }
                Err(e)
            }
        }
    }
}

pub fn spawn<F, T>(f: F) -> JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    Builder::new().spawn(f).expect("failed to spawn thread")
}

struct Packet<'scope, T> {
    scope: Option<Arc<scoped::ScopeData>>,
    result: UnsafeCell<Option<T>>,
    _marker: PhantomData<Option<&'scope scoped::ScopeData>>,
}

unsafe impl<T: Send> Sync for Packet<'_, T> {}

impl<T> Drop for Packet<'_, T> {
    fn drop(&mut self) {
        // the spawned thread drops its reference after storing the result;
        // a scope counts threads as finished once that happens
        if let Some(s) = &self.scope {
            // the result must not outlive 'scope: drop it before signalling
            unsafe { *self.result.get() = None };
            s.decrement_running();
        }
    }
}

// ---- native thread control block ----

const RUNNING: u32 = 0;
const DETACHED: u32 = 1;
const FINISHED: u32 = 2;

/// Lives at the top of the thread's stack mapping.
#[repr(C)]
struct Control {
    td: ThreadData, // first: the thread pointer points here
    tid: AtomicU32,
    state: AtomicU32,
    map_base: usize,
    map_len: usize,
    main: Option<Box<dyn FnOnce() + Send>>,
}

struct Native {
    ctl: *mut Control,
}

unsafe impl Send for Native {}
unsafe impl Sync for Native {}

unsafe fn native_spawn(stack_size: usize, main: Box<dyn FnOnce() + Send>) -> io::Result<Native> {
    let stack_size = (stack_size.max(64 * 1024) + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let ctl_size = (size_of::<Control>() + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let map_len = PAGE_SIZE + stack_size + ctl_size;
    let base = crate::sys::fd::cvt(unsafe {
        linux::mmap(
            0,
            map_len,
            linux::PROT_READ | linux::PROT_WRITE,
            linux::MAP_PRIVATE | linux::MAP_ANONYMOUS | linux::MAP_STACK,
            -1,
            0,
        )
    })?;
    // guard page below the stack
    let _ = unsafe { linux::mprotect(base, PAGE_SIZE, linux::PROT_NONE) };

    let ctl_addr = (base + map_len - size_of::<Control>()) & !63;
    let ctl = ctl_addr as *mut Control;
    unsafe {
        ctl.write(Control {
            td: ThreadData::new(),
            tid: AtomicU32::new(0),
            state: AtomicU32::new(RUNNING),
            map_base: base,
            map_len,
            main: Some(main),
        });
        ThreadData::prepare(&mut (*ctl).td);
    }

    // the child pops `entry` then `arg` from its new stack
    let sp = ((ctl_addr & !15) - 16) as *mut usize;
    unsafe {
        *sp = thread_start as extern "C" fn(*mut u8) -> ! as usize;
        *sp.add(1) = ctl_addr;
    }

    let flags = linux::CLONE_VM
        | linux::CLONE_FS
        | linux::CLONE_FILES
        | linux::CLONE_SIGHAND
        | linux::CLONE_THREAD
        | linux::CLONE_SYSVSEM
        | linux::CLONE_SETTLS
        | linux::CLONE_PARENT_SETTID
        | linux::CLONE_CHILD_CLEARTID;
    let tid = unsafe { (*ctl).tid.as_ptr() };
    let r = unsafe { arch::clone_thread(flags, sp, tid, tid, ctl_addr) };
    if r < 0 {
        unsafe {
            core::ptr::drop_in_place(ctl);
            let _ = linux::munmap(base, map_len);
        }
        return Err(io::Error::from_raw_os_error(-r as i32));
    }
    Ok(Native { ctl })
}

extern "C" fn thread_start(arg: *mut u8) -> ! {
    let ctl = arg as *mut Control;
    unsafe {
        let main = (*ctl).main.take().unwrap();
        main();
        (*ctl).td.run_dtors();
        let (base, len) = ((*ctl).map_base, (*ctl).map_len);
        if (*ctl).state.swap(FINISHED, AcqRel) == DETACHED {
            // Nobody will join us: free our own stack on the way out. Our
            // clear-tid word lives in that stack mapping, so unregister it
            // first; otherwise the kernel's write of 0 at exit could land in
            // memory another thread has since mapped at the same address.
            let _ = linux::set_tid_address(core::ptr::null_mut());
            arch::unmap_stack_and_exit(base, len);
        }
        linux::exit_thread(0)
    }
}

impl Native {
    /// Wait for the thread to exit (the kernel clears `tid`), then free its stack.
    fn join(self) {
        let ctl = unsafe { &*self.ctl };
        loop {
            let t = ctl.tid.load(Acquire);
            if t == 0 {
                break;
            }
            futex::wait_shared(&ctl.tid, t);
        }
        unsafe {
            let (base, len) = (ctl.map_base, ctl.map_len);
            core::ptr::drop_in_place(self.ctl);
            let _ = linux::munmap(base, len);
        }
    }

    fn detach(self) {
        let ctl = unsafe { &*self.ctl };
        if ctl.state.swap(DETACHED, AcqRel) == FINISHED {
            // it already finished and will not free its own stack
            self.join();
        }
    }
}

// ---- JoinHandle ----

pub(crate) struct JoinInner<'scope, T> {
    native: Option<Native>,
    thread: Thread,
    packet: Arc<Packet<'scope, T>>,
}

impl<T> JoinInner<'_, T> {
    fn join(mut self) -> Result<T> {
        self.native.take().unwrap().join();
        let packet = Arc::get_mut(&mut self.packet).expect("thread result still shared after join");
        Ok(packet.result.get_mut().take().unwrap())
    }

    fn is_finished(&self) -> bool {
        Arc::strong_count(&self.packet) == 1
    }
}

impl<T> Drop for JoinInner<'_, T> {
    fn drop(&mut self) {
        if let Some(n) = self.native.take() {
            n.detach();
        }
    }
}

pub struct JoinHandle<T>(JoinInner<'static, T>);

unsafe impl<T> Send for JoinHandle<T> {}
unsafe impl<T> Sync for JoinHandle<T> {}

impl<T> JoinHandle<T> {
    pub fn thread(&self) -> &Thread {
        &self.0.thread
    }

    pub fn join(self) -> Result<T> {
        self.0.join()
    }

    pub fn is_finished(&self) -> bool {
        self.0.is_finished()
    }
}

impl<T> fmt::Debug for JoinHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JoinHandle").finish_non_exhaustive()
    }
}
