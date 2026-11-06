//! Owned and borrowed Unix file descriptors.

use core::fmt;
use core::marker::PhantomData;
use core::mem::ManuallyDrop;

use crate::io;
use crate::sys::{fd, linux};

pub type RawFd = i32;

pub trait AsRawFd {
    fn as_raw_fd(&self) -> RawFd;
}

pub trait FromRawFd {
    /// # Safety
    /// `fd` must be an open file descriptor that the caller owns.
    unsafe fn from_raw_fd(fd: RawFd) -> Self;
}

pub trait IntoRawFd {
    fn into_raw_fd(self) -> RawFd;
}

pub trait AsFd {
    fn as_fd(&self) -> BorrowedFd<'_>;
}

#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct BorrowedFd<'fd> {
    fd: RawFd,
    _phantom: PhantomData<&'fd OwnedFd>,
}

#[repr(transparent)]
pub struct OwnedFd {
    fd: RawFd,
}

impl BorrowedFd<'_> {
    /// # Safety
    /// `fd` must stay open for the returned lifetime.
    pub const unsafe fn borrow_raw(fd: RawFd) -> Self {
        assert!(fd != -1);
        BorrowedFd { fd, _phantom: PhantomData }
    }

    pub fn try_clone_to_owned(&self) -> io::Result<OwnedFd> {
        Ok(OwnedFd { fd: fd::dup_cloexec(self.fd)? })
    }
}

impl OwnedFd {
    pub fn try_clone(&self) -> io::Result<OwnedFd> {
        self.as_fd().try_clone_to_owned()
    }
}

impl Drop for OwnedFd {
    fn drop(&mut self) {
        let _ = linux::close(self.fd);
    }
}

impl AsRawFd for BorrowedFd<'_> {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

impl AsRawFd for OwnedFd {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

impl IntoRawFd for OwnedFd {
    fn into_raw_fd(self) -> RawFd {
        ManuallyDrop::new(self).fd
    }
}

impl FromRawFd for OwnedFd {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        assert!(fd != -1);
        OwnedFd { fd }
    }
}

impl AsFd for BorrowedFd<'_> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        *self
    }
}

impl AsFd for OwnedFd {
    fn as_fd(&self) -> BorrowedFd<'_> {
        unsafe { BorrowedFd::borrow_raw(self.fd) }
    }
}

impl<T: AsFd + ?Sized> AsFd for &T {
    fn as_fd(&self) -> BorrowedFd<'_> {
        T::as_fd(self)
    }
}

impl<T: AsFd + ?Sized> AsFd for &mut T {
    fn as_fd(&self) -> BorrowedFd<'_> {
        T::as_fd(self)
    }
}

impl<T: AsFd + ?Sized> AsFd for alloc_crate::boxed::Box<T> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        (**self).as_fd()
    }
}

impl<T: AsFd + ?Sized> AsFd for alloc_crate::sync::Arc<T> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        (**self).as_fd()
    }
}

impl AsRawFd for RawFd {
    fn as_raw_fd(&self) -> RawFd {
        *self
    }
}

impl IntoRawFd for RawFd {
    fn into_raw_fd(self) -> RawFd {
        self
    }
}

impl FromRawFd for RawFd {
    unsafe fn from_raw_fd(fd: RawFd) -> RawFd {
        fd
    }
}

impl fmt::Debug for BorrowedFd<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BorrowedFd").field("fd", &self.fd).finish()
    }
}

impl fmt::Debug for OwnedFd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnedFd").field("fd", &self.fd).finish()
    }
}

macro_rules! stdio_fd {
    ($t:ty, $lock:ty, $fd:expr) => {
        impl AsRawFd for $t {
            fn as_raw_fd(&self) -> RawFd {
                $fd
            }
        }
        impl AsFd for $t {
            fn as_fd(&self) -> BorrowedFd<'_> {
                unsafe { BorrowedFd::borrow_raw($fd) }
            }
        }
        impl AsRawFd for $lock {
            fn as_raw_fd(&self) -> RawFd {
                $fd
            }
        }
        impl AsFd for $lock {
            fn as_fd(&self) -> BorrowedFd<'_> {
                unsafe { BorrowedFd::borrow_raw($fd) }
            }
        }
    };
}

stdio_fd!(io::Stdin, io::StdinLock<'_>, 0);
stdio_fd!(io::Stdout, io::StdoutLock<'_>, 1);
stdio_fd!(io::Stderr, io::StderrLock<'_>, 2);

/// Implement the fd traits for a type that wraps an `OwnedFd` field.
macro_rules! impl_owned_fd {
    ($t:ty, $field:ident) => {
        impl $crate::os::fd::AsRawFd for $t {
            fn as_raw_fd(&self) -> $crate::os::fd::RawFd {
                $crate::os::fd::AsRawFd::as_raw_fd(&self.$field)
            }
        }
        impl $crate::os::fd::AsFd for $t {
            fn as_fd(&self) -> $crate::os::fd::BorrowedFd<'_> {
                $crate::os::fd::AsFd::as_fd(&self.$field)
            }
        }
        impl $crate::os::fd::IntoRawFd for $t {
            fn into_raw_fd(self) -> $crate::os::fd::RawFd {
                $crate::os::fd::IntoRawFd::into_raw_fd(self.$field)
            }
        }
        impl $crate::os::fd::FromRawFd for $t {
            unsafe fn from_raw_fd(fd: $crate::os::fd::RawFd) -> Self {
                Self { $field: unsafe { $crate::os::fd::FromRawFd::from_raw_fd(fd) } }
            }
        }
        impl From<$t> for $crate::os::fd::OwnedFd {
            fn from(v: $t) -> $crate::os::fd::OwnedFd {
                v.$field
            }
        }
        impl From<$crate::os::fd::OwnedFd> for $t {
            fn from(fd: $crate::os::fd::OwnedFd) -> Self {
                Self { $field: fd }
            }
        }
    };
}
pub(crate) use impl_owned_fd;
