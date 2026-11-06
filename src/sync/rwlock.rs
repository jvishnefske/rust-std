use core::cell::UnsafeCell;
use core::fmt;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;

use super::{LockResult, TryLockError, TryLockResult};
use crate::sys::locks::RawRwLock;

pub struct RwLock<T: ?Sized> {
    raw: RawRwLock,
    data: UnsafeCell<T>,
}

unsafe impl<T: ?Sized + Send> Send for RwLock<T> {}
unsafe impl<T: ?Sized + Send + Sync> Sync for RwLock<T> {}

#[must_use = "if unused the RwLock will immediately unlock"]
pub struct RwLockReadGuard<'a, T: ?Sized + 'a> {
    // a pointer, not &RwLock<T>, so the guard is covariant in T like std's
    data: NonNull<T>,
    raw: &'a RawRwLock,
}

#[must_use = "if unused the RwLock will immediately unlock"]
pub struct RwLockWriteGuard<'a, T: ?Sized + 'a> {
    lock: &'a RwLock<T>,
    _not_send: PhantomData<*const ()>,
}

unsafe impl<T: ?Sized + Sync> Sync for RwLockReadGuard<'_, T> {}
unsafe impl<T: ?Sized + Sync> Sync for RwLockWriteGuard<'_, T> {}

impl<T> RwLock<T> {
    pub const fn new(t: T) -> RwLock<T> {
        RwLock { raw: RawRwLock::new(), data: UnsafeCell::new(t) }
    }

    pub fn into_inner(self) -> LockResult<T> {
        Ok(self.data.into_inner())
    }
}

impl<T: ?Sized> RwLock<T> {
    pub fn read(&self) -> LockResult<RwLockReadGuard<'_, T>> {
        self.raw.read();
        Ok(self.read_guard())
    }

    pub fn try_read(&self) -> TryLockResult<RwLockReadGuard<'_, T>> {
        if self.raw.try_read() { Ok(self.read_guard()) } else { Err(TryLockError::WouldBlock) }
    }

    pub fn write(&self) -> LockResult<RwLockWriteGuard<'_, T>> {
        self.raw.write();
        Ok(RwLockWriteGuard { lock: self, _not_send: PhantomData })
    }

    pub fn try_write(&self) -> TryLockResult<RwLockWriteGuard<'_, T>> {
        if self.raw.try_write() { Ok(RwLockWriteGuard { lock: self, _not_send: PhantomData }) } else { Err(TryLockError::WouldBlock) }
    }

    fn read_guard(&self) -> RwLockReadGuard<'_, T> {
        RwLockReadGuard { data: unsafe { NonNull::new_unchecked(self.data.get()) }, raw: &self.raw }
    }

    pub fn is_poisoned(&self) -> bool {
        false
    }

    pub fn clear_poison(&self) {}

    pub fn get_mut(&mut self) -> LockResult<&mut T> {
        Ok(self.data.get_mut())
    }
}

impl<T: ?Sized> core::panic::UnwindSafe for RwLock<T> {}
impl<T: ?Sized> core::panic::RefUnwindSafe for RwLock<T> {}

impl<T: Default> Default for RwLock<T> {
    fn default() -> RwLock<T> {
        RwLock::new(Default::default())
    }
}

impl<T> From<T> for RwLock<T> {
    fn from(t: T) -> Self {
        RwLock::new(t)
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for RwLock<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("RwLock");
        match self.try_read() {
            Ok(guard) => d.field("data", &&*guard),
            Err(_) => d.field("data", &format_args!("<locked>")),
        };
        d.field("poisoned", &false).finish_non_exhaustive()
    }
}

impl<T: ?Sized> Deref for RwLockReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { self.data.as_ref() }
    }
}

impl<T: ?Sized> Deref for RwLockWriteGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> DerefMut for RwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for RwLockReadGuard<'_, T> {
    fn drop(&mut self) {
        unsafe { self.raw.read_unlock() }
    }
}

impl<T: ?Sized> Drop for RwLockWriteGuard<'_, T> {
    fn drop(&mut self) {
        unsafe { self.lock.raw.write_unlock() }
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for RwLockReadGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for RwLockWriteGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}
