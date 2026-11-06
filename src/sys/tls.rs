//! Thread-local storage without compiler TLS support.
//!
//! Each thread has a [`ThreadData`] block whose address is the hardware thread
//! pointer (`fs` on x86_64, `tpidr_el0` on aarch64). `thread_local!` keys are
//! small integers indexing a per-thread vector of boxed values.

use alloc_crate::boxed::Box;
use alloc_crate::vec::Vec;
use core::cell::{Cell, UnsafeCell};
use core::ptr::null_mut;
use core::sync::atomic::{AtomicUsize, Ordering};

use super::arch;
use crate::thread::{AccessError, Thread};

const ALIVE: u8 = 0;
const DESTROYING: u8 = 1;
const DESTROYED: u8 = 2;

#[derive(Copy, Clone)]
struct Slot {
    ptr: *mut u8,
    dtor: unsafe fn(*mut u8),
}

#[derive(Copy, Clone)]
enum SlotState {
    Empty,
    Alive(Slot),
    /// destructor ran or is running: further access is an `AccessError`
    Destroyed,
}

#[repr(C)]
pub struct ThreadData {
    /// Must be the first field: x86_64 reads the thread pointer from `fs:[0]`.
    self_ptr: *mut ThreadData,
    thread: UnsafeCell<Option<Thread>>,
    slots: UnsafeCell<Vec<SlotState>>,
    state: Cell<u8>,
}

impl ThreadData {
    pub const fn new() -> Self {
        ThreadData {
            self_ptr: null_mut(),
            thread: UnsafeCell::new(None),
            slots: UnsafeCell::new(Vec::new()),
            state: Cell::new(ALIVE),
        }
    }

    /// Point the thread pointer at `this`. Used for the main thread; spawned
    /// threads get theirs from `clone(CLONE_SETTLS)` after [`Self::prepare`].
    pub unsafe fn install(this: *mut ThreadData) {
        unsafe {
            Self::prepare(this);
            arch::set_thread_pointer(this as usize);
        }
    }

    pub unsafe fn prepare(this: *mut ThreadData) {
        unsafe { (*this).self_ptr = this };
    }

    pub fn thread(&self) -> &Option<Thread> {
        unsafe { &*self.thread.get() }
    }

    pub fn set_thread(&self, t: Thread) {
        unsafe { *self.thread.get() = Some(t) };
    }

    fn get(&self, idx: usize) -> SlotState {
        let slots = unsafe { &*self.slots.get() };
        slots.get(idx).copied().unwrap_or(SlotState::Empty)
    }

    fn set(&self, idx: usize, state: SlotState) {
        let slots = unsafe { &mut *self.slots.get() };
        if slots.len() <= idx {
            slots.resize(idx + 1, SlotState::Empty);
        }
        slots[idx] = state;
    }

    /// Run destructors for this thread's locals. A key whose destructor has
    /// started can no longer be accessed; keys first initialized by another
    /// destructor are destroyed in a later round.
    pub fn run_dtors(&self) {
        self.state.set(DESTROYING);
        loop {
            let mut ran = false;
            let mut idx = 0;
            // index afresh each time: a destructor may grow the slot vector
            while idx < unsafe { (*self.slots.get()).len() } {
                if let SlotState::Alive(s) = self.get(idx) {
                    self.set(idx, SlotState::Destroyed);
                    unsafe { (s.dtor)(s.ptr) };
                    ran = true;
                }
                idx += 1;
            }
            if !ran {
                break;
            }
        }
        self.state.set(DESTROYED);
        drop(core::mem::take(unsafe { &mut *self.slots.get() }));
        unsafe { *self.thread.get() = None };
    }
}

#[inline]
pub fn current() -> &'static ThreadData {
    unsafe { &*(arch::thread_pointer() as *const ThreadData) }
}

/// Backing implementation of `thread::LocalKey`.
pub struct Key {
    index: AtomicUsize, // 0 = not yet assigned, else index + 1
}

static NEXT_KEY: AtomicUsize = AtomicUsize::new(1);

unsafe fn drop_box<T>(p: *mut u8) {
    drop(unsafe { Box::from_raw(p as *mut T) });
}

impl Key {
    pub const fn new() -> Self {
        Key { index: AtomicUsize::new(0) }
    }

    fn index(&self) -> usize {
        let k = self.index.load(Ordering::Acquire);
        if k != 0 {
            return k - 1;
        }
        let n = NEXT_KEY.fetch_add(1, Ordering::Relaxed);
        match self.index.compare_exchange(0, n, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => n - 1,
            Err(existing) => existing - 1,
        }
    }

    /// Pointer to this thread's value, initializing it with `init` on first use.
    pub fn get<T: 'static>(&'static self, init: impl FnOnce() -> T) -> Result<*const T, AccessError> {
        let td = current();
        let idx = self.index();
        match td.get(idx) {
            SlotState::Alive(s) => return Ok(s.ptr as *const T),
            SlotState::Destroyed => return Err(AccessError(())),
            SlotState::Empty if td.state.get() == DESTROYED => return Err(AccessError(())),
            SlotState::Empty => {}
        }
        // `init` may itself use thread locals (and even this one), so no
        // borrow of the slot vector is held across it.
        let p = Box::into_raw(Box::new(init()));
        match td.get(idx) {
            SlotState::Empty => {
                td.set(idx, SlotState::Alive(Slot { ptr: p as *mut u8, dtor: drop_box::<T> }));
                Ok(p)
            }
            SlotState::Alive(s) => {
                // initialized recursively by `init`: keep that value
                drop(unsafe { Box::from_raw(p) });
                Ok(s.ptr as *const T)
            }
            SlotState::Destroyed => {
                drop(unsafe { Box::from_raw(p) });
                Err(AccessError(()))
            }
        }
    }
}
