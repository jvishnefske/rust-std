use core::fmt;

use super::{Condvar, Mutex};

pub struct Barrier {
    lock: Mutex<(usize, usize)>, // (arrived, generation)
    cvar: Condvar,
    n: usize,
}

pub struct BarrierWaitResult(bool);

impl BarrierWaitResult {
    pub fn is_leader(&self) -> bool {
        self.0
    }
}

impl fmt::Debug for BarrierWaitResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BarrierWaitResult").field("is_leader", &self.0).finish()
    }
}

impl Barrier {
    pub const fn new(n: usize) -> Barrier {
        Barrier { lock: Mutex::new((0, 0)), cvar: Condvar::new(), n }
    }

    pub fn wait(&self) -> BarrierWaitResult {
        let mut st = self.lock.lock().unwrap();
        let generation = st.1;
        st.0 += 1;
        if st.0 < self.n {
            let _st = self.cvar.wait_while(st, |st| st.1 == generation).unwrap();
            BarrierWaitResult(false)
        } else {
            st.0 = 0;
            st.1 = st.1.wrapping_add(1);
            self.cvar.notify_all();
            BarrierWaitResult(true)
        }
    }
}

impl fmt::Debug for Barrier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Barrier").finish_non_exhaustive()
    }
}
