use core::cell::{Cell, RefCell};
use core::error::Error;
use core::fmt;

use crate::sys::tls::Key;

/// A thread-local storage key, created by [`thread_local!`](crate::thread_local).
pub struct LocalKey<T: 'static> {
    key: Key,
    init: fn() -> T,
}

unsafe impl<T: 'static> Sync for LocalKey<T> {}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AccessError(pub(crate) ());

impl fmt::Debug for AccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccessError").finish()
    }
}

impl fmt::Display for AccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt("already destroyed", f)
    }
}

impl Error for AccessError {}

impl<T: 'static> LocalKey<T> {
    #[doc(hidden)]
    pub const fn new(init: fn() -> T) -> LocalKey<T> {
        LocalKey { key: Key::new(), init }
    }

    pub fn with<F, R>(&'static self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        match self.try_with(f) {
            Ok(r) => r,
            Err(_) => panic!("cannot access a Thread Local Storage value during or after destruction"),
        }
    }

    pub fn try_with<F, R>(&'static self, f: F) -> Result<R, AccessError>
    where
        F: FnOnce(&T) -> R,
    {
        let p = self.key.get(self.init)?;
        Ok(f(unsafe { &*p }))
    }
}

impl<T: 'static> LocalKey<Cell<T>> {
    pub fn set(&'static self, value: T) {
        self.with(|c| c.set(value))
    }

    pub fn get(&'static self) -> T
    where
        T: Copy,
    {
        self.with(Cell::get)
    }

    pub fn take(&'static self) -> T
    where
        T: Default,
    {
        self.with(Cell::take)
    }

    pub fn replace(&'static self, value: T) -> T {
        self.with(|c| c.replace(value))
    }
}

impl<T: 'static> LocalKey<RefCell<T>> {
    pub fn with_borrow<F, R>(&'static self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        self.with(|c| f(&c.borrow()))
    }

    pub fn with_borrow_mut<F, R>(&'static self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        self.with(|c| f(&mut c.borrow_mut()))
    }

    pub fn set(&'static self, value: T) {
        self.with(|c| *c.borrow_mut() = value)
    }

    pub fn take(&'static self) -> T
    where
        T: Default,
    {
        self.with(RefCell::take)
    }

    pub fn replace(&'static self, value: T) -> T {
        self.with(|c| c.replace(value))
    }
}

impl<T: 'static> fmt::Debug for LocalKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalKey").finish_non_exhaustive()
    }
}

/// Declare a new thread local storage key of type [`LocalKey`].
#[macro_export]
macro_rules! thread_local {
    () => {};

    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = const $init:block; $($rest:tt)*) => {
        $crate::thread_local!($(#[$attr])* $vis static $name: $t = $init; $($rest)*);
    };
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = const $init:block) => {
        $crate::thread_local!($(#[$attr])* $vis static $name: $t = $init);
    };

    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr; $($rest:tt)*) => {
        $crate::thread_local!($(#[$attr])* $vis static $name: $t = $init);
        $crate::thread_local!($($rest)*);
    };
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr) => {
        $(#[$attr])* $vis static $name: $crate::thread::LocalKey<$t> = {
            fn __init() -> $t {
                $init
            }
            $crate::thread::LocalKey::new(__init)
        };
    };
}
