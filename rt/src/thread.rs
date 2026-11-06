//! Native threads created with `clone(2)`.
//!
//! One anonymous mapping per thread holds, from the bottom: a guard page, the
//! stack, the thread's TLS area and its [`Control`] block. The kernel clears
//! `Control::tid` and wakes it as a futex when the thread exits
//! (`CLONE_CHILD_CLEARTID`), which is what [`Thread::join`] waits for. A
//! detached thread unmaps its own mapping on the way out.

use core::mem::size_of;
use core::sync::atomic::AtomicU32;
use core::sync::atomic::Ordering::{AcqRel, Acquire};

use crate::sys::{self, PAGE_SIZE};
use crate::{arch, futex, tls};

const RUNNING: u32 = 0;
const DETACHED: u32 = 1;
const FINISHED: u32 = 2;

/// The smallest stack [`spawn`] hands out.
pub const MIN_STACK: usize = 64 * 1024;

#[repr(C)]
struct Control {
    tid: AtomicU32,
    state: AtomicU32,
    map_base: usize,
    map_len: usize,
    entry: unsafe fn(*mut u8),
    arg: *mut u8,
}

/// A spawned thread: [`join`](Thread::join) or [`detach`](Thread::detach) it
/// exactly once (dropping it leaks the thread's mapping).
pub struct Thread {
    ctl: *mut Control,
}

unsafe impl Send for Thread {}
unsafe impl Sync for Thread {}

const fn round_up(n: usize, to: usize) -> usize {
    (n + to - 1) & !(to - 1)
}

/// Start a thread running `entry(arg)` on a stack of at least `stack_size`
/// bytes. `entry` must return normally; the thread then exits.
///
/// # Safety
/// `entry` must be safe to call with `arg` on the new thread.
pub unsafe fn spawn(stack_size: usize, entry: unsafe fn(*mut u8), arg: *mut u8) -> Result<Thread, i32> {
    let stack_size = round_up(stack_size.max(MIN_STACK), PAGE_SIZE);
    let tls_len = tls::Layout::new(&tls::image()).reserve();
    let top_len = round_up(tls_len + size_of::<Control>() + 64, PAGE_SIZE);
    let map_len = PAGE_SIZE + stack_size + top_len;
    let base = unsafe {
        sys::mmap(
            0,
            map_len,
            sys::PROT_READ | sys::PROT_WRITE,
            sys::MAP_PRIVATE | sys::MAP_ANONYMOUS | sys::MAP_STACK,
            -1,
            0,
        )?
    };
    // guard page below the stack
    let _ = unsafe { sys::mprotect(base, PAGE_SIZE, sys::PROT_NONE) };

    let ctl_addr = (base + map_len - size_of::<Control>()) & !63;
    let ctl = ctl_addr as *mut Control;
    let top = base + PAGE_SIZE + stack_size;
    let tp = unsafe {
        ctl.write(Control {
            tid: AtomicU32::new(0),
            state: AtomicU32::new(RUNNING),
            map_base: base,
            map_len,
            entry,
            arg,
        });
        tls::init_area(top as *mut u8)
    };

    // the trampoline pops `thread_start` then `ctl` off the new stack
    let sp = ((top & !15) - 16) as *mut usize;
    unsafe {
        *sp = thread_start as extern "C" fn(*mut u8) -> ! as usize;
        *sp.add(1) = ctl_addr;
    }

    let flags = sys::CLONE_VM
        | sys::CLONE_FS
        | sys::CLONE_FILES
        | sys::CLONE_SIGHAND
        | sys::CLONE_THREAD
        | sys::CLONE_SYSVSEM
        | sys::CLONE_SETTLS
        | sys::CLONE_PARENT_SETTID
        | sys::CLONE_CHILD_CLEARTID;
    let tid = unsafe { (*ctl).tid.as_ptr() };
    let r = unsafe { arch::clone_thread(flags, sp, tid, tid, tp) };
    if r < 0 {
        let _ = unsafe { sys::munmap(base, map_len) };
        return Err(-r as i32);
    }
    Ok(Thread { ctl })
}

extern "C" fn thread_start(arg: *mut u8) -> ! {
    let ctl = arg as *mut Control;
    unsafe {
        ((*ctl).entry)((*ctl).arg);
        let (base, len) = ((*ctl).map_base, (*ctl).map_len);
        if (*ctl).state.swap(FINISHED, AcqRel) == DETACHED {
            // Nobody will join us: free our own mapping on the way out. Our
            // clear-tid word lives in it, so unregister that first; otherwise
            // the kernel's write of 0 at exit could land in memory another
            // thread has since mapped at the same address.
            let _ = sys::set_tid_address(core::ptr::null_mut());
            arch::unmap_stack_and_exit(base, len);
        }
        sys::exit_thread(0)
    }
}

impl Thread {
    /// The kernel thread id (0 once the thread has exited).
    pub fn tid(&self) -> u32 {
        unsafe { (*self.ctl).tid.load(Acquire) }
    }

    /// Wait for the thread to exit, then free its mapping.
    pub fn join(self) {
        let ctl = unsafe { &*self.ctl };
        loop {
            let t = ctl.tid.load(Acquire);
            if t == 0 {
                break;
            }
            futex::wait_shared(&ctl.tid, t);
        }
        let (base, len) = (ctl.map_base, ctl.map_len);
        let _ = unsafe { sys::munmap(base, len) };
    }

    /// Let the thread free its own mapping when it finishes.
    pub fn detach(self) {
        let ctl = unsafe { &*self.ctl };
        if ctl.state.swap(DETACHED, AcqRel) == FINISHED {
            // it already finished and will not free its mapping itself
            self.join();
        }
    }
}
