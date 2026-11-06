//! OS-specific functionality.

pub mod fd;

pub mod raw {
    pub use core::ffi::{
        c_char, c_double, c_float, c_int, c_long, c_longlong, c_schar, c_short, c_uchar, c_uint, c_ulong,
        c_ulonglong, c_ushort, c_void,
    };
}

pub mod unix {
    pub use super::fd as io;

    pub mod ffi {
        use alloc_crate::vec::Vec;

        use crate::ffi::{OsStr, OsString};

        pub trait OsStrExt {
            fn from_bytes(slice: &[u8]) -> &Self;
            fn as_bytes(&self) -> &[u8];
        }

        impl OsStrExt for OsStr {
            fn from_bytes(slice: &[u8]) -> &OsStr {
                OsStr::from_bytes(slice)
            }
            fn as_bytes(&self) -> &[u8] {
                OsStr::as_bytes(self)
            }
        }

        pub trait OsStringExt {
            fn from_vec(vec: Vec<u8>) -> Self;
            fn into_vec(self) -> Vec<u8>;
        }

        impl OsStringExt for OsString {
            fn from_vec(vec: Vec<u8>) -> OsString {
                OsString::from_vec(vec)
            }
            fn into_vec(self) -> Vec<u8> {
                OsString::into_vec(self)
            }
        }
    }

    pub mod fs {
        use crate::fs::{DirEntry, File, FileType, Metadata, OpenOptions, Permissions};
        use crate::io;
        use crate::path::Path;
        use crate::sys::linux;

        pub trait FileTypeExt {
            fn is_block_device(&self) -> bool;
            fn is_char_device(&self) -> bool;
            fn is_fifo(&self) -> bool;
            fn is_socket(&self) -> bool;
        }

        impl FileTypeExt for FileType {
            fn is_block_device(&self) -> bool {
                self.is(linux::S_IFBLK)
            }
            fn is_char_device(&self) -> bool {
                self.is(linux::S_IFCHR)
            }
            fn is_fifo(&self) -> bool {
                self.is(linux::S_IFIFO)
            }
            fn is_socket(&self) -> bool {
                self.is(linux::S_IFSOCK)
            }
        }

        pub trait PermissionsExt {
            fn mode(&self) -> u32;
            fn set_mode(&mut self, mode: u32);
            fn from_mode(mode: u32) -> Self;
        }

        impl PermissionsExt for Permissions {
            fn mode(&self) -> u32 {
                self.mode
            }
            fn set_mode(&mut self, mode: u32) {
                self.mode = mode;
            }
            fn from_mode(mode: u32) -> Permissions {
                Permissions { mode }
            }
        }

        pub trait OpenOptionsExt {
            fn mode(&mut self, mode: u32) -> &mut Self;
            fn custom_flags(&mut self, flags: i32) -> &mut Self;
        }

        impl OpenOptionsExt for OpenOptions {
            fn mode(&mut self, mode: u32) -> &mut OpenOptions {
                self.mode = mode;
                self
            }
            fn custom_flags(&mut self, flags: i32) -> &mut OpenOptions {
                self.custom_flags = flags as u32;
                self
            }
        }

        pub trait MetadataExt {
            fn dev(&self) -> u64;
            fn ino(&self) -> u64;
            fn mode(&self) -> u32;
            fn nlink(&self) -> u64;
            fn uid(&self) -> u32;
            fn gid(&self) -> u32;
            fn rdev(&self) -> u64;
            fn size(&self) -> u64;
            fn atime(&self) -> i64;
            fn atime_nsec(&self) -> i64;
            fn mtime(&self) -> i64;
            fn mtime_nsec(&self) -> i64;
            fn ctime(&self) -> i64;
            fn ctime_nsec(&self) -> i64;
            fn blksize(&self) -> u64;
            fn blocks(&self) -> u64;
        }

        fn makedev(major: u32, minor: u32) -> u64 {
            let (major, minor) = (major as u64, minor as u64);
            ((major & 0xfffff000) << 32) | ((major & 0xfff) << 8) | ((minor & 0xffffff00) << 12) | (minor & 0xff)
        }

        impl MetadataExt for Metadata {
            fn dev(&self) -> u64 {
                makedev(self.stat.dev_major, self.stat.dev_minor)
            }
            fn ino(&self) -> u64 {
                self.stat.ino
            }
            fn mode(&self) -> u32 {
                self.stat.mode as u32
            }
            fn nlink(&self) -> u64 {
                self.stat.nlink as u64
            }
            fn uid(&self) -> u32 {
                self.stat.uid
            }
            fn gid(&self) -> u32 {
                self.stat.gid
            }
            fn rdev(&self) -> u64 {
                makedev(self.stat.rdev_major, self.stat.rdev_minor)
            }
            fn size(&self) -> u64 {
                self.stat.size
            }
            fn atime(&self) -> i64 {
                self.stat.atime.sec
            }
            fn atime_nsec(&self) -> i64 {
                self.stat.atime.nsec as i64
            }
            fn mtime(&self) -> i64 {
                self.stat.mtime.sec
            }
            fn mtime_nsec(&self) -> i64 {
                self.stat.mtime.nsec as i64
            }
            fn ctime(&self) -> i64 {
                self.stat.ctime.sec
            }
            fn ctime_nsec(&self) -> i64 {
                self.stat.ctime.nsec as i64
            }
            fn blksize(&self) -> u64 {
                self.stat.blksize as u64
            }
            fn blocks(&self) -> u64 {
                self.stat.blocks
            }
        }

        pub trait DirEntryExt {
            fn ino(&self) -> u64;
        }

        impl DirEntryExt for DirEntry {
            fn ino(&self) -> u64 {
                self.ino
            }
        }

        pub trait FileExt {
            fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<usize>;
            fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<usize>;
        }

        impl FileExt for File {
            fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<usize> {
                crate::sys::fd::read_at(crate::os::fd::AsRawFd::as_raw_fd(self), buf, offset)
            }
            fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<usize> {
                crate::sys::fd::write_at(crate::os::fd::AsRawFd::as_raw_fd(self), buf, offset)
            }
        }

        pub fn symlink<P: AsRef<Path>, Q: AsRef<Path>>(original: P, link: Q) -> io::Result<()> {
            crate::fs::symlink_inner(original.as_ref(), link.as_ref())
        }
    }

    pub mod process {
        use crate::process::{Command, ExitStatus};

        pub trait ExitStatusExt {
            fn from_raw(raw: i32) -> Self;
            fn signal(&self) -> Option<i32>;
            fn core_dumped(&self) -> bool;
            fn into_raw(self) -> i32;
        }

        impl ExitStatusExt for ExitStatus {
            fn from_raw(raw: i32) -> ExitStatus {
                ExitStatus(raw)
            }
            fn signal(&self) -> Option<i32> {
                let sig = self.0 & 0x7f;
                if sig != 0 && sig != 0x7f { Some(sig) } else { None }
            }
            fn core_dumped(&self) -> bool {
                self.signal().is_some() && self.0 & 0x80 != 0
            }
            fn into_raw(self) -> i32 {
                self.0
            }
        }

        pub trait CommandExt {
            fn arg0<S: AsRef<crate::ffi::OsStr>>(&mut self, arg: S) -> &mut Command;
        }

        impl CommandExt for Command {
            fn arg0<S: AsRef<crate::ffi::OsStr>>(&mut self, arg: S) -> &mut Command {
                self.arg0 = Some(arg.as_ref().to_os_string());
                self
            }
        }
    }

    pub mod prelude {
        pub use super::ffi::{OsStrExt, OsStringExt};
        pub use super::fs::{DirEntryExt, FileExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
        pub use super::io::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
        pub use super::process::{CommandExt, ExitStatusExt};
    }
}

pub mod linux {
    pub use super::unix::fs;
}
