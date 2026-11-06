//! Traits, helpers, and type definitions for core I/O functionality.

use alloc_crate::boxed::Box;
use alloc_crate::collections::VecDeque;
use alloc_crate::string::String;
use alloc_crate::vec::Vec;
use core::fmt;

mod buffered;
mod cursor;
mod error;
pub(crate) mod stdio;

pub use buffered::{BufReader, BufWriter, LineWriter};
pub use cursor::Cursor;
pub use error::{Error, ErrorKind, IntoInnerError, Result};
#[doc(hidden)]
pub use error::const_error;
pub use stdio::{
    IsTerminal, Stderr, StderrLock, Stdin, StdinLock, Stdout, StdoutLock, _eprint, _print, stderr, stdin, stdout,
};

pub mod prelude {
    pub use super::{BufRead, Read, Seek, Write};
}

pub(crate) const DEFAULT_BUF_SIZE: usize = 8 * 1024;

fn utf8_error() -> Error {
    const_error!(ErrorKind::InvalidData, "stream did not contain valid UTF-8")
}

pub(crate) fn default_read_to_end<R: Read + ?Sized>(r: &mut R, buf: &mut Vec<u8>) -> Result<usize> {
    let start = buf.len();
    let mut chunk = 32usize;
    loop {
        if buf.capacity() - buf.len() < chunk {
            buf.reserve(chunk);
        }
        let len = buf.len();
        let spare = buf.capacity() - len;
        buf.resize(len + spare, 0);
        match r.read(&mut buf[len..]) {
            Ok(0) => {
                buf.truncate(len);
                return Ok(len - start);
            }
            Ok(n) => {
                buf.truncate(len + n);
                if n == spare && chunk < DEFAULT_BUF_SIZE * 8 {
                    chunk *= 2;
                }
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => buf.truncate(len),
            Err(e) => {
                buf.truncate(len);
                return Err(e);
            }
        }
    }
}

pub(crate) fn default_read_to_string<R: Read + ?Sized>(r: &mut R, buf: &mut String) -> Result<usize> {
    let mut bytes = core::mem::take(buf).into_bytes();
    let start = bytes.len();
    let ret = default_read_to_end(r, &mut bytes);
    match core::str::from_utf8(&bytes[start..]) {
        Ok(_) => {
            *buf = unsafe { String::from_utf8_unchecked(bytes) };
            ret
        }
        Err(_) => {
            bytes.truncate(start);
            *buf = unsafe { String::from_utf8_unchecked(bytes) };
            ret.and_then(|_| Err(utf8_error()))
        }
    }
}

pub trait Read {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;

    fn read_vectored(&mut self, bufs: &mut [IoSliceMut<'_>]) -> Result<usize> {
        match bufs.iter_mut().find(|b| !b.is_empty()) {
            Some(b) => self.read(b),
            None => Ok(0),
        }
    }

    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        default_read_to_end(self, buf)
    }

    fn read_to_string(&mut self, buf: &mut String) -> Result<usize> {
        default_read_to_string(self, buf)
    }

    fn read_exact(&mut self, mut buf: &mut [u8]) -> Result<()> {
        while !buf.is_empty() {
            match self.read(buf) {
                Ok(0) => break,
                Ok(n) => buf = &mut buf[n..],
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        if buf.is_empty() {
            Ok(())
        } else {
            Err(const_error!(ErrorKind::UnexpectedEof, "failed to fill whole buffer"))
        }
    }

    fn by_ref(&mut self) -> &mut Self
    where
        Self: Sized,
    {
        self
    }

    fn bytes(self) -> Bytes<Self>
    where
        Self: Sized,
    {
        Bytes { inner: self }
    }

    fn chain<R: Read>(self, next: R) -> Chain<Self, R>
    where
        Self: Sized,
    {
        Chain { first: self, second: next, done_first: false }
    }

    fn take(self, limit: u64) -> Take<Self>
    where
        Self: Sized,
    {
        Take { inner: self, limit }
    }
}

/// Read all bytes from `reader` into a new String.
pub fn read_to_string<R: Read>(mut reader: R) -> Result<String> {
    let mut buf = String::new();
    reader.read_to_string(&mut buf)?;
    Ok(buf)
}

pub trait Write {
    fn write(&mut self, buf: &[u8]) -> Result<usize>;

    fn write_vectored(&mut self, bufs: &[IoSlice<'_>]) -> Result<usize> {
        match bufs.iter().find(|b| !b.is_empty()) {
            Some(b) => self.write(b),
            None => Ok(0),
        }
    }

    fn flush(&mut self) -> Result<()>;

    fn write_all(&mut self, mut buf: &[u8]) -> Result<()> {
        while !buf.is_empty() {
            match self.write(buf) {
                Ok(0) => return Err(const_error!(ErrorKind::WriteZero, "failed to write whole buffer")),
                Ok(n) => buf = &buf[n..],
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> Result<()> {
        // fast path for plain string literals
        if let Some(s) = args.as_str() {
            return self.write_all(s.as_bytes());
        }
        struct Adapter<'a, T: ?Sized> {
            inner: &'a mut T,
            error: Result<()>,
        }
        impl<T: Write + ?Sized> fmt::Write for Adapter<'_, T> {
            fn write_str(&mut self, s: &str) -> fmt::Result {
                match self.inner.write_all(s.as_bytes()) {
                    Ok(()) => Ok(()),
                    Err(e) => {
                        self.error = Err(e);
                        Err(fmt::Error)
                    }
                }
            }
        }
        let mut out = Adapter { inner: self, error: Ok(()) };
        match fmt::write(&mut out, args) {
            Ok(()) => Ok(()),
            Err(_) => {
                if out.error.is_err() {
                    out.error
                } else {
                    Err(const_error!(ErrorKind::Uncategorized, "formatter error"))
                }
            }
        }
    }

    fn by_ref(&mut self) -> &mut Self
    where
        Self: Sized,
    {
        self
    }
}

#[derive(Copy, PartialEq, Eq, Clone, Debug)]
pub enum SeekFrom {
    Start(u64),
    End(i64),
    Current(i64),
}

pub trait Seek {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64>;

    fn rewind(&mut self) -> Result<()> {
        self.seek(SeekFrom::Start(0))?;
        Ok(())
    }

    fn stream_position(&mut self) -> Result<u64> {
        self.seek(SeekFrom::Current(0))
    }

    fn stream_len(&mut self) -> Result<u64> {
        let old = self.stream_position()?;
        let len = self.seek(SeekFrom::End(0))?;
        if old != len {
            self.seek(SeekFrom::Start(old))?;
        }
        Ok(len)
    }

    fn seek_relative(&mut self, offset: i64) -> Result<()> {
        self.seek(SeekFrom::Current(offset))?;
        Ok(())
    }
}

fn read_until<R: BufRead + ?Sized>(r: &mut R, delim: u8, buf: &mut Vec<u8>) -> Result<usize> {
    let mut read = 0;
    loop {
        let (done, used) = {
            let available = match r.fill_buf() {
                Ok(n) => n,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            match available.iter().position(|&b| b == delim) {
                Some(i) => {
                    buf.extend_from_slice(&available[..=i]);
                    (true, i + 1)
                }
                None => {
                    buf.extend_from_slice(available);
                    (available.is_empty(), available.len())
                }
            }
        };
        r.consume(used);
        read += used;
        if done {
            return Ok(read);
        }
    }
}

pub trait BufRead: Read {
    fn fill_buf(&mut self) -> Result<&[u8]>;

    fn consume(&mut self, amt: usize);

    fn has_data_left(&mut self) -> Result<bool> {
        self.fill_buf().map(|b| !b.is_empty())
    }

    fn read_until(&mut self, byte: u8, buf: &mut Vec<u8>) -> Result<usize> {
        read_until(self, byte, buf)
    }

    fn skip_until(&mut self, byte: u8) -> Result<usize> {
        let mut v = Vec::new();
        read_until(self, byte, &mut v)
    }

    fn read_line(&mut self, buf: &mut String) -> Result<usize> {
        let mut bytes = core::mem::take(buf).into_bytes();
        let start = bytes.len();
        let ret = read_until(self, b'\n', &mut bytes);
        if core::str::from_utf8(&bytes[start..]).is_err() {
            bytes.truncate(start);
            *buf = unsafe { String::from_utf8_unchecked(bytes) };
            return ret.and_then(|_| Err(utf8_error()));
        }
        *buf = unsafe { String::from_utf8_unchecked(bytes) };
        ret
    }

    fn split(self, byte: u8) -> Split<Self>
    where
        Self: Sized,
    {
        Split { buf: self, delim: byte }
    }

    fn lines(self) -> Lines<Self>
    where
        Self: Sized,
    {
        Lines { buf: self }
    }
}

// ---- iterator adapters ----

#[derive(Debug)]
pub struct Bytes<R> {
    inner: R,
}

impl<R: Read> Iterator for Bytes<R> {
    type Item = Result<u8>;
    fn next(&mut self) -> Option<Result<u8>> {
        let mut b = 0u8;
        loop {
            return match self.inner.read(core::slice::from_mut(&mut b)) {
                Ok(0) => None,
                Ok(_) => Some(Ok(b)),
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => Some(Err(e)),
            };
        }
    }
}

#[derive(Debug)]
pub struct Lines<B> {
    buf: B,
}

impl<B: BufRead> Iterator for Lines<B> {
    type Item = Result<String>;
    fn next(&mut self) -> Option<Result<String>> {
        let mut buf = String::new();
        match self.buf.read_line(&mut buf) {
            Ok(0) => None,
            Ok(_) => {
                if buf.ends_with('\n') {
                    buf.pop();
                    if buf.ends_with('\r') {
                        buf.pop();
                    }
                }
                Some(Ok(buf))
            }
            Err(e) => Some(Err(e)),
        }
    }
}

#[derive(Debug)]
pub struct Split<B> {
    buf: B,
    delim: u8,
}

impl<B: BufRead> Iterator for Split<B> {
    type Item = Result<Vec<u8>>;
    fn next(&mut self) -> Option<Result<Vec<u8>>> {
        let mut buf = Vec::new();
        match self.buf.read_until(self.delim, &mut buf) {
            Ok(0) => None,
            Ok(_) => {
                if buf.last() == Some(&self.delim) {
                    buf.pop();
                }
                Some(Ok(buf))
            }
            Err(e) => Some(Err(e)),
        }
    }
}

#[derive(Debug)]
pub struct Chain<T, U> {
    first: T,
    second: U,
    done_first: bool,
}

impl<T, U> Chain<T, U> {
    pub fn into_inner(self) -> (T, U) {
        (self.first, self.second)
    }
    pub fn get_ref(&self) -> (&T, &U) {
        (&self.first, &self.second)
    }
    pub fn get_mut(&mut self) -> (&mut T, &mut U) {
        (&mut self.first, &mut self.second)
    }
}

impl<T: Read, U: Read> Read for Chain<T, U> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if !self.done_first {
            match self.first.read(buf)? {
                0 if !buf.is_empty() => self.done_first = true,
                n => return Ok(n),
            }
        }
        self.second.read(buf)
    }
}

impl<T: BufRead, U: BufRead> BufRead for Chain<T, U> {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        if !self.done_first {
            match self.first.fill_buf()? {
                [] => self.done_first = true,
                buf => return Ok(buf),
            }
        }
        self.second.fill_buf()
    }
    fn consume(&mut self, amt: usize) {
        if !self.done_first { self.first.consume(amt) } else { self.second.consume(amt) }
    }
}

#[derive(Debug)]
pub struct Take<T> {
    inner: T,
    limit: u64,
}

impl<T> Take<T> {
    pub fn limit(&self) -> u64 {
        self.limit
    }
    pub fn set_limit(&mut self, limit: u64) {
        self.limit = limit;
    }
    pub fn into_inner(self) -> T {
        self.inner
    }
    pub fn get_ref(&self) -> &T {
        &self.inner
    }
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T: Read> Read for Take<T> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if self.limit == 0 {
            return Ok(0);
        }
        let max = (buf.len() as u64).min(self.limit) as usize;
        let n = self.inner.read(&mut buf[..max])?;
        self.limit -= n as u64;
        Ok(n)
    }
}

impl<T: BufRead> BufRead for Take<T> {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        if self.limit == 0 {
            return Ok(&[]);
        }
        let buf = self.inner.fill_buf()?;
        let cap = (buf.len() as u64).min(self.limit) as usize;
        Ok(&buf[..cap])
    }
    fn consume(&mut self, amt: usize) {
        let amt = (amt as u64).min(self.limit) as usize;
        self.limit -= amt as u64;
        self.inner.consume(amt);
    }
}

// ---- IoSlice ----

#[derive(Copy, Clone, Debug)]
#[repr(transparent)]
pub struct IoSlice<'a>(&'a [u8]);

impl<'a> IoSlice<'a> {
    pub fn new(buf: &'a [u8]) -> IoSlice<'a> {
        IoSlice(buf)
    }
}

impl core::ops::Deref for IoSlice<'_> {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.0
    }
}

#[derive(Debug)]
#[repr(transparent)]
pub struct IoSliceMut<'a>(&'a mut [u8]);

impl<'a> IoSliceMut<'a> {
    pub fn new(buf: &'a mut [u8]) -> IoSliceMut<'a> {
        IoSliceMut(buf)
    }
}

impl core::ops::Deref for IoSliceMut<'_> {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.0
    }
}

impl core::ops::DerefMut for IoSliceMut<'_> {
    fn deref_mut(&mut self) -> &mut [u8] {
        self.0
    }
}

// ---- blanket and std-type impls ----

impl<R: Read + ?Sized> Read for &mut R {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        (**self).read(buf)
    }
    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        (**self).read_to_end(buf)
    }
    fn read_to_string(&mut self, buf: &mut String) -> Result<usize> {
        (**self).read_to_string(buf)
    }
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        (**self).read_exact(buf)
    }
}

impl<R: Read + ?Sized> Read for Box<R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        (**self).read(buf)
    }
    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        (**self).read_to_end(buf)
    }
    fn read_to_string(&mut self, buf: &mut String) -> Result<usize> {
        (**self).read_to_string(buf)
    }
}

impl<W: Write + ?Sized> Write for &mut W {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        (**self).write(buf)
    }
    fn flush(&mut self) -> Result<()> {
        (**self).flush()
    }
    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        (**self).write_all(buf)
    }
    fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> Result<()> {
        (**self).write_fmt(args)
    }
}

impl<W: Write + ?Sized> Write for Box<W> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        (**self).write(buf)
    }
    fn flush(&mut self) -> Result<()> {
        (**self).flush()
    }
    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        (**self).write_all(buf)
    }
}

impl<B: BufRead + ?Sized> BufRead for &mut B {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        (**self).fill_buf()
    }
    fn consume(&mut self, amt: usize) {
        (**self).consume(amt)
    }
}

impl<B: BufRead + ?Sized> BufRead for Box<B> {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        (**self).fill_buf()
    }
    fn consume(&mut self, amt: usize) {
        (**self).consume(amt)
    }
}

impl<S: Seek + ?Sized> Seek for &mut S {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        (**self).seek(pos)
    }
}

impl<S: Seek + ?Sized> Seek for Box<S> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        (**self).seek(pos)
    }
}

impl Read for &[u8] {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let n = buf.len().min(self.len());
        buf[..n].copy_from_slice(&self[..n]);
        *self = &self[n..];
        Ok(n)
    }
    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        let n = self.len();
        buf.extend_from_slice(self);
        *self = &[];
        Ok(n)
    }
}

impl BufRead for &[u8] {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        Ok(*self)
    }
    fn consume(&mut self, amt: usize) {
        *self = &self[amt..];
    }
}

impl Write for &mut [u8] {
    fn write(&mut self, data: &[u8]) -> Result<usize> {
        let n = data.len().min(self.len());
        let (a, b) = core::mem::take(self).split_at_mut(n);
        a.copy_from_slice(&data[..n]);
        *self = b;
        Ok(n)
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

impl Write for Vec<u8> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        self.extend_from_slice(buf);
        Ok(())
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

impl Read for VecDeque<u8> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let (front, _) = self.as_slices();
        let n = buf.len().min(front.len());
        buf[..n].copy_from_slice(&front[..n]);
        self.drain(..n);
        Ok(n)
    }
}

impl Write for VecDeque<u8> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.extend(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

// ---- utilities ----

pub fn copy<R: Read + ?Sized, W: Write + ?Sized>(reader: &mut R, writer: &mut W) -> Result<u64> {
    let mut buf = [0u8; DEFAULT_BUF_SIZE];
    let mut written = 0u64;
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => return Ok(written),
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        writer.write_all(&buf[..n])?;
        written += n as u64;
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Empty;

pub const fn empty() -> Empty {
    Empty
}

impl Read for Empty {
    fn read(&mut self, _: &mut [u8]) -> Result<usize> {
        Ok(0)
    }
}
impl BufRead for Empty {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        Ok(&[])
    }
    fn consume(&mut self, _: usize) {}
}
impl Write for Empty {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}
impl Seek for Empty {
    fn seek(&mut self, _: SeekFrom) -> Result<u64> {
        Ok(0)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Sink;

pub const fn sink() -> Sink {
    Sink
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Repeat {
    byte: u8,
}

pub const fn repeat(byte: u8) -> Repeat {
    Repeat { byte }
}

impl Read for Repeat {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        buf.fill(self.byte);
        Ok(buf.len())
    }
}
