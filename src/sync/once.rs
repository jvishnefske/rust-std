use core::fmt;
use core::sync::atomic::AtomicU32;
use core::sync::atomic::Ordering::{Acquire, Release};

use crate::sys::futex;

const INCOMPLETE: u32 = 0;
const RUNNING: u32 = 1;
const COMPLETE: u32 = 2;

pub struct Once {
    state: AtomicU32,
}

pub struct OnceState {
    _priv: (),
}

impl OnceState {
    pub fn is_poisoned(&self) -> bool {
        false
    }
}

impl Once {
    pub const fn new() -> Once {
        Once { state: AtomicU32::new(INCOMPLETE) }
    }

    #[inline]
    pub fn is_completed(&self) -> bool {
        self.state.load(Acquire) == COMPLETE
    }

    pub fn call_once<F: FnOnce()>(&self, f: F) {
        if self.is_completed() {
            return;
        }
        let mut f = Some(f);
        self.call_inner(&mut || (f.take().unwrap())());
    }

    pub fn call_once_force<F: FnOnce(&OnceState)>(&self, f: F) {
        if self.is_completed() {
            return;
        }
        let mut f = Some(f);
        self.call_inner(&mut || (f.take().unwrap())(&OnceState { _priv: () }));
    }

    pub fn wait(&self) {
        loop {
            let s = self.state.load(Acquire);
            if s == COMPLETE {
                return;
            }
            futex::wait(&self.state, s, None);
        }
    }

    /// Same as [`Once::wait`]: a Once is never poisoned here.
    pub fn wait_force(&self) {
        self.wait()
    }

    fn call_inner(&self, f: &mut dyn FnMut()) {
        self.call_fallible(&mut || {
            f();
            true
        });
    }

    /// Run `f` unless already complete. If `f` returns false the Once goes
    /// back to incomplete, so a later call can try again.
    #[cold]
    pub(crate) fn call_fallible(&self, f: &mut dyn FnMut() -> bool) {
        loop {
            match self.state.compare_exchange(INCOMPLETE, RUNNING, Acquire, Acquire) {
                Ok(_) => {
                    let done = f();
                    self.state.store(if done { COMPLETE } else { INCOMPLETE }, Release);
                    futex::wake_all(&self.state);
                    return;
                }
                Err(COMPLETE) => return,
                Err(_) => {
                    futex::wait(&self.state, RUNNING, None);
                }
            }
        }
    }
}

impl core::panic::UnwindSafe for Once {}
impl core::panic::RefUnwindSafe for Once {}

impl Default for Once {
    fn default() -> Self {
        Once::new()
    }
}

impl fmt::Debug for Once {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Once").finish_non_exhaustive()
    }
}

impl fmt::Debug for OnceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OnceState").field("poisoned", &false).finish()
    }
}
