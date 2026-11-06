use core::cell::UnsafeCell;
use core::fmt;

use super::Once;

pub struct OnceLock<T> {
    once: Once,
    // an Option rather than MaybeUninit: drop glue then handles the value, so
    // no Drop impl is needed and dropck is as lenient as std's #[may_dangle]
    value: UnsafeCell<Option<T>>,
}

unsafe impl<T: Sync + Send> Sync for OnceLock<T> {}
unsafe impl<T: Send> Send for OnceLock<T> {}

impl<T> OnceLock<T> {
    pub const fn new() -> OnceLock<T> {
        OnceLock { once: Once::new(), value: UnsafeCell::new(None) }
    }

    pub fn get(&self) -> Option<&T> {
        if self.once.is_completed() { Some(unsafe { self.get_unchecked() }) } else { None }
    }

    pub fn get_mut(&mut self) -> Option<&mut T> {
        if self.once.is_completed() { self.value.get_mut().as_mut() } else { None }
    }

    pub fn wait(&self) -> &T {
        self.once.wait();
        unsafe { self.get_unchecked() }
    }

    pub fn set(&self, value: T) -> Result<(), T> {
        match self.try_insert(value) {
            Ok(_) => Ok(()),
            Err((_, value)) => Err(value),
        }
    }

    pub fn try_insert(&self, value: T) -> Result<&T, (&T, T)> {
        let mut value = Some(value);
        let res = self.get_or_init(|| value.take().unwrap());
        match value {
            None => Ok(res),
            Some(value) => Err((res, value)),
        }
    }

    pub fn get_or_init<F: FnOnce() -> T>(&self, f: F) -> &T {
        match self.get_or_try_init(|| Ok::<T, core::convert::Infallible>(f())) {
            Ok(v) => v,
            Err(e) => match e {},
        }
    }

    pub fn get_or_try_init<F, E>(&self, f: F) -> Result<&T, E>
    where
        F: FnOnce() -> Result<T, E>,
    {
        if let Some(v) = self.get() {
            return Ok(v);
        }
        let mut res = Ok(());
        let mut f = Some(f);
        let slot = &self.value;
        self.once.call_fallible(&mut || match (f.take().unwrap())() {
            Ok(v) => {
                unsafe { *slot.get() = Some(v) };
                true
            }
            Err(e) => {
                res = Err(e);
                false
            }
        });
        res?;
        // either we initialized it, or another thread completed it first
        Ok(self.get().expect("OnceLock initialized"))
    }

    pub fn into_inner(mut self) -> Option<T> {
        self.take()
    }

    pub fn take(&mut self) -> Option<T> {
        self.once = Once::new();
        self.value.get_mut().take()
    }

    unsafe fn get_unchecked(&self) -> &T {
        unsafe { (*self.value.get()).as_ref().unwrap_unchecked() }
    }
}

impl<T: core::panic::RefUnwindSafe + core::panic::UnwindSafe> core::panic::RefUnwindSafe for OnceLock<T> {}
impl<T: core::panic::UnwindSafe> core::panic::UnwindSafe for OnceLock<T> {}

impl<T> Default for OnceLock<T> {
    fn default() -> OnceLock<T> {
        OnceLock::new()
    }
}

impl<T: fmt::Debug> fmt::Debug for OnceLock<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_tuple("OnceLock");
        match self.get() {
            Some(v) => d.field(v),
            None => d.field(&format_args!("<uninit>")),
        };
        d.finish()
    }
}

impl<T: Clone> Clone for OnceLock<T> {
    fn clone(&self) -> OnceLock<T> {
        let cell = Self::new();
        if let Some(v) = self.get() {
            let _ = cell.set(v.clone());
        }
        cell
    }
}

impl<T> From<T> for OnceLock<T> {
    fn from(value: T) -> Self {
        let cell = Self::new();
        let _ = cell.set(value);
        cell
    }
}

impl<T: PartialEq> PartialEq for OnceLock<T> {
    fn eq(&self, other: &OnceLock<T>) -> bool {
        self.get() == other.get()
    }
}

impl<T: Eq> Eq for OnceLock<T> {}
