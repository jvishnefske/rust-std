//! io::Result-returning helpers over raw file descriptors.

use crate::io;
use super::linux::{self, EINTR, SysResult};

#[inline]
pub fn cvt<T>(r: SysResult<T>) -> io::Result<T> {
    r.map_err(io::Error::from_raw_os_error)
}

/// Retry a syscall while it fails with EINTR.
#[inline]
pub fn cvt_r<T>(mut f: impl FnMut() -> SysResult<T>) -> io::Result<T> {
    loop {
        match f() {
            Err(EINTR) => {}
            r => return cvt(r),
        }
    }
}

// reads and writes larger than this are clamped, as the kernel would anyway
const MAX_RW: usize = isize::MAX as usize & !4095;

pub fn read(fd: i32, buf: &mut [u8]) -> io::Result<usize> {
    let len = buf.len().min(MAX_RW);
    cvt_r(|| unsafe { linux::read(fd, buf.as_mut_ptr(), len) })
}

pub fn write(fd: i32, buf: &[u8]) -> io::Result<usize> {
    let len = buf.len().min(MAX_RW);
    cvt_r(|| unsafe { linux::write(fd, buf.as_ptr(), len) })
}

pub fn read_at(fd: i32, buf: &mut [u8], off: u64) -> io::Result<usize> {
    let len = buf.len().min(MAX_RW);
    cvt_r(|| unsafe { linux::pread(fd, buf.as_mut_ptr(), len, off) })
}

pub fn write_at(fd: i32, buf: &[u8], off: u64) -> io::Result<usize> {
    let len = buf.len().min(MAX_RW);
    cvt_r(|| unsafe { linux::pwrite(fd, buf.as_ptr(), len, off) })
}

pub fn dup_cloexec(fd: i32) -> io::Result<i32> {
    cvt(linux::fcntl(fd, linux::F_DUPFD_CLOEXEC, 3)).map(|f| f as i32)
}

pub fn set_nonblocking(fd: i32, nonblocking: bool) -> io::Result<()> {
    let flags = cvt(linux::fcntl(fd, linux::F_GETFL, 0))? as u32;
    let new = if nonblocking { flags | linux::O_NONBLOCK } else { flags & !linux::O_NONBLOCK };
    if new != flags {
        cvt(linux::fcntl(fd, linux::F_SETFL, new as usize))?;
    }
    Ok(())
}
