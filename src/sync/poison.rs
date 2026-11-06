use core::error::Error;
use core::fmt;

pub struct PoisonError<T> {
    data: T,
}

impl<T> PoisonError<T> {
    pub fn new(data: T) -> PoisonError<T> {
        PoisonError { data }
    }
    pub fn into_inner(self) -> T {
        self.data
    }
    pub fn get_ref(&self) -> &T {
        &self.data
    }
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.data
    }
}

impl<T> fmt::Debug for PoisonError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PoisonError").finish_non_exhaustive()
    }
}

impl<T> fmt::Display for PoisonError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("poisoned lock: another task failed inside")
    }
}

impl<T> Error for PoisonError<T> {}

pub enum TryLockError<T> {
    Poisoned(PoisonError<T>),
    WouldBlock,
}

impl<T> From<PoisonError<T>> for TryLockError<T> {
    fn from(err: PoisonError<T>) -> TryLockError<T> {
        TryLockError::Poisoned(err)
    }
}

impl<T> fmt::Debug for TryLockError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TryLockError::Poisoned(..) => f.write_str("Poisoned(..)"),
            TryLockError::WouldBlock => f.write_str("WouldBlock"),
        }
    }
}

impl<T> fmt::Display for TryLockError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TryLockError::Poisoned(..) => f.write_str("poisoned lock: another task failed inside"),
            TryLockError::WouldBlock => f.write_str("try_lock failed because the operation would block"),
        }
    }
}

impl<T> Error for TryLockError<T> {}

pub type LockResult<T> = Result<T, PoisonError<T>>;
pub type TryLockResult<T> = Result<T, TryLockError<T>>;
