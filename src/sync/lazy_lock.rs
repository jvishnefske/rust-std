use core::cell::Cell;
use core::fmt;
use core::ops::Deref;

use super::OnceLock;

pub struct LazyLock<T, F = fn() -> T> {
    cell: OnceLock<T>,
    init: Cell<Option<F>>,
}

unsafe impl<T: Sync + Send, F: Send> Sync for LazyLock<T, F> {}

impl<T, F: FnOnce() -> T> LazyLock<T, F> {
    pub const fn new(f: F) -> LazyLock<T, F> {
        LazyLock { cell: OnceLock::new(), init: Cell::new(Some(f)) }
    }

    pub fn force(this: &LazyLock<T, F>) -> &T {
        this.cell.get_or_init(|| match this.init.take() {
            Some(f) => f(),
            None => panic!("LazyLock instance has previously been poisoned"),
        })
    }

    pub fn force_mut(this: &mut LazyLock<T, F>) -> &mut T {
        LazyLock::force(this);
        this.cell.get_mut().unwrap()
    }

    pub fn get(this: &LazyLock<T, F>) -> Option<&T> {
        this.cell.get()
    }

    pub fn get_mut(this: &mut LazyLock<T, F>) -> Option<&mut T> {
        this.cell.get_mut()
    }

    pub fn into_inner(this: Self) -> Result<T, F> {
        let LazyLock { cell, init } = this;
        match cell.into_inner() {
            Some(v) => Ok(v),
            None => Err(init.into_inner().unwrap()),
        }
    }
}

impl<T: core::panic::RefUnwindSafe + core::panic::UnwindSafe, F: core::panic::UnwindSafe> core::panic::RefUnwindSafe
    for LazyLock<T, F>
{
}
impl<T: core::panic::UnwindSafe, F: core::panic::UnwindSafe> core::panic::UnwindSafe for LazyLock<T, F> {}

impl<T, F: FnOnce() -> T> Deref for LazyLock<T, F> {
    type Target = T;
    fn deref(&self) -> &T {
        LazyLock::force(self)
    }
}

impl<T: Default> Default for LazyLock<T> {
    fn default() -> LazyLock<T> {
        LazyLock::new(T::default)
    }
}

impl<T: fmt::Debug, F> fmt::Debug for LazyLock<T, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_tuple("LazyLock");
        match self.cell.get() {
            Some(v) => d.field(v),
            None => d.field(&format_args!("<uninit>")),
        };
        d.finish()
    }
}
