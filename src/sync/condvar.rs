use core::fmt;
use core::time::Duration;

use super::{LockResult, MutexGuard};
use crate::sys::locks::RawCondvar;
use crate::time::Instant;

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub struct WaitTimeoutResult(bool);

impl WaitTimeoutResult {
    pub fn timed_out(&self) -> bool {
        self.0
    }
}

pub struct Condvar {
    raw: RawCondvar,
}

impl Condvar {
    pub const fn new() -> Condvar {
        Condvar { raw: RawCondvar::new() }
    }

    pub fn wait<'a, T>(&self, guard: MutexGuard<'a, T>) -> LockResult<MutexGuard<'a, T>> {
        unsafe { self.raw.wait(&guard.lock.raw, None) };
        Ok(guard)
    }

    pub fn wait_while<'a, T, F>(&self, mut guard: MutexGuard<'a, T>, mut condition: F) -> LockResult<MutexGuard<'a, T>>
    where
        F: FnMut(&mut T) -> bool,
    {
        while condition(&mut *guard) {
            guard = self.wait(guard)?;
        }
        Ok(guard)
    }

    pub fn wait_timeout<'a, T>(&self, guard: MutexGuard<'a, T>, dur: Duration) -> LockResult<(MutexGuard<'a, T>, WaitTimeoutResult)> {
        let ok = unsafe { self.raw.wait(&guard.lock.raw, Some(dur)) };
        Ok((guard, WaitTimeoutResult(!ok)))
    }

    pub fn wait_timeout_ms<'a, T>(&self, guard: MutexGuard<'a, T>, ms: u32) -> LockResult<(MutexGuard<'a, T>, bool)> {
        let ok = unsafe { self.raw.wait(&guard.lock.raw, Some(Duration::from_millis(ms as u64))) };
        Ok((guard, ok))
    }

    pub fn wait_timeout_while<'a, T, F>(
        &self,
        mut guard: MutexGuard<'a, T>,
        dur: Duration,
        mut condition: F,
    ) -> LockResult<(MutexGuard<'a, T>, WaitTimeoutResult)>
    where
        F: FnMut(&mut T) -> bool,
    {
        let start = Instant::now();
        loop {
            if !condition(&mut *guard) {
                return Ok((guard, WaitTimeoutResult(false)));
            }
            let timeout = match dur.checked_sub(start.elapsed()) {
                Some(t) => t,
                None => return Ok((guard, WaitTimeoutResult(true))),
            };
            guard = self.wait_timeout(guard, timeout)?.0;
        }
    }

    pub fn notify_one(&self) {
        self.raw.notify_one()
    }

    pub fn notify_all(&self) {
        self.raw.notify_all()
    }
}

impl core::panic::UnwindSafe for Condvar {}
impl core::panic::RefUnwindSafe for Condvar {}

impl Default for Condvar {
    fn default() -> Condvar {
        Condvar::new()
    }
}

impl fmt::Debug for Condvar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Condvar").finish_non_exhaustive()
    }
}
