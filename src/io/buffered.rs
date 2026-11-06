use alloc_crate::boxed::Box;
use alloc_crate::vec::Vec;
use core::fmt;

use super::{BufRead, DEFAULT_BUF_SIZE, IntoInnerError, Read, Result, Seek, SeekFrom, Write};

pub struct BufReader<R: ?Sized> {
    buf: Box<[u8]>,
    pos: usize,
    filled: usize,
    inner: R,
}

impl<R: Read> BufReader<R> {
    pub fn new(inner: R) -> BufReader<R> {
        BufReader::with_capacity(DEFAULT_BUF_SIZE, inner)
    }

    pub fn with_capacity(capacity: usize, inner: R) -> BufReader<R> {
        BufReader { buf: alloc_crate::vec![0; capacity].into_boxed_slice(), pos: 0, filled: 0, inner }
    }
}

impl<R> BufReader<R> {
    pub fn into_inner(self) -> R {
        self.inner
    }
}

impl<R: ?Sized> BufReader<R> {
    pub fn get_ref(&self) -> &R {
        &self.inner
    }
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.inner
    }
    pub fn buffer(&self) -> &[u8] {
        &self.buf[self.pos..self.filled]
    }
    pub fn capacity(&self) -> usize {
        self.buf.len()
    }
    fn discard_buffer(&mut self) {
        self.pos = 0;
        self.filled = 0;
    }
}

impl<R: ?Sized + Read> Read for BufReader<R> {
    fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        // large reads bypass the buffer when it is empty
        if self.pos == self.filled && out.len() >= self.buf.len() {
            self.discard_buffer();
            return self.inner.read(out);
        }
        let n = {
            let mut rem = self.fill_buf()?;
            rem.read(out)?
        };
        self.consume(n);
        Ok(n)
    }

    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        let n = self.filled - self.pos;
        buf.extend_from_slice(&self.buf[self.pos..self.filled]);
        self.discard_buffer();
        Ok(n + self.inner.read_to_end(buf)?)
    }
}

impl<R: ?Sized + Read> BufRead for BufReader<R> {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        if self.pos >= self.filled {
            self.filled = self.inner.read(&mut self.buf)?;
            self.pos = 0;
        }
        Ok(&self.buf[self.pos..self.filled])
    }

    fn consume(&mut self, amt: usize) {
        self.pos = (self.pos + amt).min(self.filled);
    }
}

impl<R: ?Sized + Seek> Seek for BufReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        let result = if let SeekFrom::Current(n) = pos {
            let remainder = (self.filled - self.pos) as i64;
            self.inner.seek(SeekFrom::Current(n - remainder))?
        } else {
            self.inner.seek(pos)?
        };
        self.discard_buffer();
        Ok(result)
    }
}

impl<R: ?Sized + fmt::Debug> fmt::Debug for BufReader<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BufReader")
            .field("reader", &&self.inner)
            .field("buffer", &format_args!("{}/{}", self.filled - self.pos, self.buf.len()))
            .finish()
    }
}

pub struct BufWriter<W: ?Sized + Write> {
    buf: Vec<u8>,
    panicked: bool,
    inner: W,
}

impl<W: Write> BufWriter<W> {
    pub fn new(inner: W) -> BufWriter<W> {
        BufWriter::with_capacity(DEFAULT_BUF_SIZE, inner)
    }

    pub fn with_capacity(capacity: usize, inner: W) -> BufWriter<W> {
        BufWriter { buf: Vec::with_capacity(capacity), panicked: false, inner }
    }

    pub fn into_inner(mut self) -> core::result::Result<W, IntoInnerError<BufWriter<W>>> {
        match self.flush_buf() {
            Err(e) => Err(IntoInnerError::new(self, e)),
            Ok(()) => {
                let me = core::mem::ManuallyDrop::new(self);
                // SAFETY: fields are read exactly once and `me` is never dropped
                let inner = unsafe { core::ptr::read(&me.inner) };
                drop(unsafe { core::ptr::read(&me.buf) });
                Ok(inner)
            }
        }
    }
}

impl<W: ?Sized + Write> BufWriter<W> {
    fn flush_buf(&mut self) -> Result<()> {
        let mut written = 0;
        let mut ret = Ok(());
        while written < self.buf.len() {
            self.panicked = true;
            let r = self.inner.write(&self.buf[written..]);
            self.panicked = false;
            match r {
                Ok(0) => {
                    ret = Err(super::const_error!(super::ErrorKind::WriteZero, "failed to write the buffered data"));
                    break;
                }
                Ok(n) => written += n,
                Err(e) if e.kind() == super::ErrorKind::Interrupted => {}
                Err(e) => {
                    ret = Err(e);
                    break;
                }
            }
        }
        self.buf.drain(..written);
        ret
    }

    /// Buffer as much of `data` as fits in the spare capacity.
    fn write_to_buf(&mut self, data: &[u8]) -> usize {
        let n = data.len().min(self.buf.capacity() - self.buf.len());
        self.buf.extend_from_slice(&data[..n]);
        n
    }

    pub fn get_ref(&self) -> &W {
        &self.inner
    }
    pub fn get_mut(&mut self) -> &mut W {
        &mut self.inner
    }
    pub fn buffer(&self) -> &[u8] {
        &self.buf
    }
    pub fn capacity(&self) -> usize {
        self.buf.capacity()
    }
}

impl<W: ?Sized + Write> Write for BufWriter<W> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        if self.buf.len() + buf.len() > self.buf.capacity() {
            self.flush_buf()?;
        }
        if buf.len() >= self.buf.capacity() {
            self.panicked = true;
            let r = self.inner.write(buf);
            self.panicked = false;
            r
        } else {
            self.buf.extend_from_slice(buf);
            Ok(buf.len())
        }
    }

    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        if self.buf.len() + buf.len() > self.buf.capacity() {
            self.flush_buf()?;
        }
        if buf.len() >= self.buf.capacity() {
            self.panicked = true;
            let r = self.inner.write_all(buf);
            self.panicked = false;
            r
        } else {
            self.buf.extend_from_slice(buf);
            Ok(())
        }
    }

    fn flush(&mut self) -> Result<()> {
        self.flush_buf()?;
        self.inner.flush()
    }
}

impl<W: ?Sized + Write + Seek> Seek for BufWriter<W> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        self.flush_buf()?;
        self.inner.seek(pos)
    }
}

impl<W: ?Sized + Write> Drop for BufWriter<W> {
    fn drop(&mut self) {
        if !self.panicked {
            let _ = self.flush_buf();
        }
    }
}

impl<W: ?Sized + Write + fmt::Debug> fmt::Debug for BufWriter<W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BufWriter")
            .field("writer", &&self.inner)
            .field("buffer", &format_args!("{}/{}", self.buf.len(), self.buf.capacity()))
            .finish()
    }
}

/// Buffers output and flushes it whenever a newline is written.
pub struct LineWriter<W: ?Sized + Write> {
    inner: BufWriter<W>,
}

impl<W: Write> LineWriter<W> {
    pub fn new(inner: W) -> LineWriter<W> {
        LineWriter::with_capacity(1024, inner)
    }

    pub fn with_capacity(capacity: usize, inner: W) -> LineWriter<W> {
        LineWriter { inner: BufWriter::with_capacity(capacity, inner) }
    }

    pub fn into_inner(self) -> core::result::Result<W, IntoInnerError<LineWriter<W>>> {
        self.inner.into_inner().map_err(|e| {
            let (w, err) = e.into_parts();
            IntoInnerError::new(LineWriter { inner: w }, err)
        })
    }
}

impl<W: ?Sized + Write> LineWriter<W> {
    pub fn get_ref(&self) -> &W {
        self.inner.get_ref()
    }
    pub fn get_mut(&mut self) -> &mut W {
        self.inner.get_mut()
    }
}

impl<W: ?Sized + Write> LineWriter<W> {
    fn flush_if_completed_line(&mut self) -> Result<()> {
        match self.inner.buffer().last() {
            Some(b'\n') => self.inner.flush_buf(),
            _ => Ok(()),
        }
    }
}

// The same algorithm as std's LineWriterShim, so that return values (how
// much of a write was accepted) match std's exactly.
impl<W: ?Sized + Write> Write for LineWriter<W> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        let newline_idx = match buf.iter().rposition(|&b| b == b'\n') {
            None => {
                self.flush_if_completed_line()?;
                return self.inner.write(buf);
            }
            Some(i) => i + 1,
        };
        self.inner.flush_buf()?;
        // complete lines go to the inner writer in a single write
        let flushed = self.inner.get_mut().write(&buf[..newline_idx])?;
        if flushed == 0 {
            return Ok(0);
        }
        let cap = self.inner.capacity();
        let tail = if flushed >= newline_idx {
            let tail = &buf[flushed..];
            if tail.len() >= cap {
                return Ok(flushed);
            }
            tail
        } else if newline_idx - flushed <= cap {
            &buf[flushed..newline_idx]
        } else {
            let scan = &buf[flushed..][..cap];
            match scan.iter().rposition(|&b| b == b'\n') {
                Some(i) => &scan[..i + 1],
                None => scan,
            }
        };
        Ok(flushed + self.inner.write_to_buf(tail))
    }

    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        match buf.iter().rposition(|&b| b == b'\n') {
            None => {
                self.flush_if_completed_line()?;
                self.inner.write_all(buf)
            }
            Some(i) => {
                let (lines, tail) = buf.split_at(i + 1);
                if self.inner.buffer().is_empty() {
                    self.inner.get_mut().write_all(lines)?;
                } else {
                    self.inner.write_all(lines)?;
                    self.inner.flush_buf()?;
                }
                self.inner.write_all(tail)
            }
        }
    }

    fn flush(&mut self) -> Result<()> {
        self.inner.flush()
    }
}

impl<W: ?Sized + Write + fmt::Debug> fmt::Debug for LineWriter<W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LineWriter").field("writer", &self.inner.get_ref()).finish()
    }
}
