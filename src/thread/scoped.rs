use alloc_crate::sync::Arc;
use core::fmt;
use core::marker::PhantomData;
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering::{Acquire, Relaxed, Release};

use super::{Builder, JoinInner, Result, Thread, current, park};
use crate::io;

pub(crate) struct ScopeData {
    num_running: AtomicUsize,
    main_thread: Thread,
}

impl ScopeData {
    pub(crate) fn increment_running(&self) {
        self.num_running.fetch_add(1, Relaxed);
    }

    pub(crate) fn decrement_running(&self) {
        if self.num_running.fetch_sub(1, Release) == 1 {
            self.main_thread.unpark();
        }
    }
}

pub struct Scope<'scope, 'env: 'scope> {
    data: Arc<ScopeData>,
    scope: PhantomData<&'scope mut &'scope ()>,
    env: PhantomData<&'env mut &'env ()>,
}

pub struct ScopedJoinHandle<'scope, T>(JoinInner<'scope, T>);

unsafe impl<T> Send for ScopedJoinHandle<'_, T> {}
unsafe impl<T> Sync for ScopedJoinHandle<'_, T> {}

/// Create a scope for spawning threads that may borrow non-'static data.
/// All threads spawned in the scope are joined before this returns.
pub fn scope<'env, F, T>(f: F) -> T
where
    F: for<'scope> FnOnce(&'scope Scope<'scope, 'env>) -> T,
{
    let scope = Scope {
        data: Arc::new(ScopeData { num_running: AtomicUsize::new(0), main_thread: current() }),
        env: PhantomData,
        scope: PhantomData,
    };
    let result = f(&scope);
    while scope.data.num_running.load(Acquire) != 0 {
        park();
    }
    result
}

impl<'scope, 'env> Scope<'scope, 'env> {
    pub fn spawn<F, T>(&'scope self, f: F) -> ScopedJoinHandle<'scope, T>
    where
        F: FnOnce() -> T + Send + 'scope,
        T: Send + 'scope,
    {
        Builder::new().spawn_scoped(self, f).expect("failed to spawn thread")
    }
}

impl Builder {
    pub fn spawn_scoped<'scope, 'env, F, T>(self, scope: &'scope Scope<'scope, 'env>, f: F) -> io::Result<ScopedJoinHandle<'scope, T>>
    where
        F: FnOnce() -> T + Send + 'scope,
        T: Send + 'scope,
    {
        // SAFETY: `scope` joins every thread before 'scope ends
        Ok(ScopedJoinHandle(unsafe { self.spawn_inner(f, Some(scope.data.clone())) }?))
    }
}

impl<T> ScopedJoinHandle<'_, T> {
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

impl fmt::Debug for Scope<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Scope").field("num_running_threads", &self.data.num_running.load(Relaxed)).finish_non_exhaustive()
    }
}

impl<T> fmt::Debug for ScopedJoinHandle<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopedJoinHandle").finish_non_exhaustive()
    }
}
