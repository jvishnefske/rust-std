//! Synchronization primitives built on Linux futexes.
//!
//! The runtime aborts on panic, so locks are never poisoned; the
//! `LockResult` signatures are kept for source compatibility with std.

pub use alloc_crate::sync::{Arc, Weak};
pub use core::sync::atomic;

mod barrier;
mod condvar;
mod lazy_lock;
pub mod mpsc;
mod mutex;
mod once;
mod once_lock;
mod poison;
mod rwlock;

pub use barrier::{Barrier, BarrierWaitResult};
pub use condvar::{Condvar, WaitTimeoutResult};
pub use lazy_lock::LazyLock;
pub use mutex::{Mutex, MutexGuard};
pub use once::{Once, OnceState};
pub use once_lock::OnceLock;
pub use poison::{LockResult, PoisonError, TryLockError, TryLockResult};
pub use rwlock::{RwLock, RwLockReadGuard, RwLockWriteGuard};
