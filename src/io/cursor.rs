use alloc_crate::boxed::Box;
use alloc_crate::vec::Vec;

use super::{BufRead, ErrorKind, Read, Result, Seek, SeekFrom, Write, const_error};

#[derive(Debug, Default, Eq, PartialEq, Clone)]
pub struct Cursor<T> {
    inner: T,
    pos: u64,
}

impl<T> Cursor<T> {
    pub const fn new(inner: T) -> Cursor<T> {
        Cursor { inner, pos: 0 }
    }
    pub fn into_inner(self) -> T {
        self.inner
    }
    pub const fn get_ref(&self) -> &T {
        &self.inner
    }
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }
    pub const fn position(&self) -> u64 {
        self.pos
    }
    pub fn set_position(&mut self, pos: u64) {
        self.pos = pos;
    }
}

impl<T: AsRef<[u8]>> Cursor<T> {
    fn remaining(&self) -> &[u8] {
        let data = self.inner.as_ref();
        let start = (self.pos as usize).min(data.len());
        &data[start..]
    }
}

impl<T: AsRef<[u8]>> Seek for Cursor<T> {
    fn seek(&mut self, style: SeekFrom) -> Result<u64> {
        let (base, offset) = match style {
            SeekFrom::Start(n) => {
                self.pos = n;
                return Ok(n);
            }
            SeekFrom::End(n) => (self.inner.as_ref().len() as u64, n),
            SeekFrom::Current(n) => (self.pos, n),
        };
        match base.checked_add_signed(offset) {
            Some(n) => {
                self.pos = n;
                Ok(n)
            }
            None => Err(const_error!(ErrorKind::InvalidInput, "invalid seek to a negative or overflowing position")),
        }
    }
}

impl<T: AsRef<[u8]>> Read for Cursor<T> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let n = Read::read(&mut self.remaining(), buf)?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl<T: AsRef<[u8]>> BufRead for Cursor<T> {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        Ok(self.remaining())
    }
    fn consume(&mut self, amt: usize) {
        self.pos += amt as u64;
    }
}

fn slice_write(pos: &mut u64, slice: &mut [u8], buf: &[u8]) -> Result<usize> {
    let start = (*pos as usize).min(slice.len());
    let n = (&mut slice[start..]).write(buf)?;
    *pos += n as u64;
    Ok(n)
}

fn vec_write(pos: &mut u64, vec: &mut Vec<u8>, buf: &[u8]) -> Result<usize> {
    let p = usize::try_from(*pos).map_err(|_| const_error!(ErrorKind::InvalidInput, "cursor position exceeds maximum possible vector length"))?;
    if p > vec.len() {
        vec.resize(p, 0);
    }
    let overlap = (vec.len() - p).min(buf.len());
    vec[p..p + overlap].copy_from_slice(&buf[..overlap]);
    vec.extend_from_slice(&buf[overlap..]);
    *pos += buf.len() as u64;
    Ok(buf.len())
}

impl Write for Cursor<&mut [u8]> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        slice_write(&mut self.pos, self.inner, buf)
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

impl<const N: usize> Write for Cursor<[u8; N]> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        slice_write(&mut self.pos, &mut self.inner, buf)
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

impl Write for Cursor<Box<[u8]>> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        slice_write(&mut self.pos, &mut self.inner, buf)
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

impl Write for Cursor<&mut Vec<u8>> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        vec_write(&mut self.pos, self.inner, buf)
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

impl Write for Cursor<Vec<u8>> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        vec_write(&mut self.pos, &mut self.inner, buf)
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// Seeking matches checked arithmetic on (base, offset), never panics,
    /// and leaves the position unchanged on error.
    #[kani::proof]
    fn seek_matches_model() {
        let data = [0u8; 3];
        let mut c = Cursor::new(&data[..]);
        let start: u64 = kani::any();
        c.set_position(start);
        let off: i64 = kani::any();
        let (pos, expected) = match kani::any::<u8>() % 3 {
            0 => {
                let n: u64 = kani::any();
                (SeekFrom::Start(n), Some(n))
            }
            1 => (SeekFrom::End(off), 3u64.checked_add_signed(off)),
            _ => (SeekFrom::Current(off), start.checked_add_signed(off)),
        };
        match c.seek(pos) {
            Ok(n) => {
                assert_eq!(Some(n), expected);
                assert_eq!(c.position(), n);
            }
            Err(e) => {
                assert_eq!(expected, None);
                assert_eq!(e.kind(), ErrorKind::InvalidInput);
                assert_eq!(c.position(), start);
            }
        }
    }
}
