use alloc_crate::string::String;
use alloc_crate::vec::Vec;
use core::fmt;

use super::{BufRead, BufReader, Lines, LineWriter, Read, Result, Write};
use crate::sync::{Mutex, MutexGuard};
use crate::sys::{fd, linux};

/// Raw fd handle that treats EBADF as success (closed std streams are
/// silently ignored, like std does).
struct Raw(i32);

fn handle_ebadf<T>(r: Result<T>, default: T) -> Result<T> {
    match r {
        Err(e) if e.raw_os_error() == Some(linux::EBADF) => Ok(default),
        r => r,
    }
}

impl Read for Raw {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        handle_ebadf(fd::read(self.0, buf), 0)
    }
}

impl Write for Raw {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        handle_ebadf(fd::write(self.0, buf), buf.len())
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

pub(crate) fn isatty(fd: i32) -> bool {
    const TCGETS: usize = 0x5401;
    let mut termios = [0u8; 64];
    unsafe { linux::syscall(crate::sys::nr::IOCTL, [fd as usize, TCGETS, termios.as_mut_ptr() as usize, 0, 0, 0]) }.is_ok()
}

pub trait IsTerminal {
    fn is_terminal(&self) -> bool;
}

// ---- stdout ----

static STDOUT: Mutex<Option<LineWriter<Raw>>> = Mutex::new(None);

pub struct Stdout {
    _priv: (),
}

pub struct StdoutLock<'a> {
    guard: MutexGuard<'a, Option<LineWriter<Raw>>>,
}

pub fn stdout() -> Stdout {
    Stdout { _priv: () }
}

impl Stdout {
    pub fn lock(&self) -> StdoutLock<'static> {
        let mut guard = STDOUT.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = Some(LineWriter::new(Raw(1)));
        }
        StdoutLock { guard }
    }
}

impl StdoutLock<'_> {
    fn w(&mut self) -> &mut LineWriter<Raw> {
        self.guard.as_mut().unwrap()
    }
}

impl Write for Stdout {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.lock().write(buf)
    }
    fn flush(&mut self) -> Result<()> {
        self.lock().flush()
    }
    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        self.lock().write_all(buf)
    }
    fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> Result<()> {
        self.lock().write_fmt(args)
    }
}

impl Write for &Stdout {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.lock().write(buf)
    }
    fn flush(&mut self) -> Result<()> {
        self.lock().flush()
    }
    fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> Result<()> {
        self.lock().write_fmt(args)
    }
}

impl Write for StdoutLock<'_> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.w().write(buf)
    }
    fn flush(&mut self) -> Result<()> {
        self.w().flush()
    }
}

impl fmt::Debug for Stdout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Stdout { .. }")
    }
}

impl IsTerminal for Stdout {
    fn is_terminal(&self) -> bool {
        isatty(1)
    }
}

// ---- stderr (unbuffered) ----

static STDERR: Mutex<()> = Mutex::new(());

pub struct Stderr {
    _priv: (),
}

pub struct StderrLock<'a> {
    _guard: MutexGuard<'a, ()>,
}

pub fn stderr() -> Stderr {
    Stderr { _priv: () }
}

impl Stderr {
    pub fn lock(&self) -> StderrLock<'static> {
        StderrLock { _guard: STDERR.lock().unwrap_or_else(|e| e.into_inner()) }
    }
}

impl Write for Stderr {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.lock().write(buf)
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
    fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> Result<()> {
        self.lock().write_fmt(args)
    }
}

impl Write for &Stderr {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.lock().write(buf)
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
    fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> Result<()> {
        self.lock().write_fmt(args)
    }
}

impl Write for StderrLock<'_> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        Raw(2).write(buf)
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

impl fmt::Debug for Stderr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Stderr { .. }")
    }
}

impl IsTerminal for Stderr {
    fn is_terminal(&self) -> bool {
        isatty(2)
    }
}

// ---- stdin ----

static STDIN: Mutex<Option<BufReader<Raw>>> = Mutex::new(None);

pub struct Stdin {
    _priv: (),
}

pub struct StdinLock<'a> {
    guard: MutexGuard<'a, Option<BufReader<Raw>>>,
}

pub fn stdin() -> Stdin {
    Stdin { _priv: () }
}

impl Stdin {
    pub fn lock(&self) -> StdinLock<'static> {
        let mut guard = STDIN.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = Some(BufReader::new(Raw(0)));
        }
        StdinLock { guard }
    }

    pub fn read_line(&self, buf: &mut String) -> Result<usize> {
        self.lock().read_line(buf)
    }

    pub fn lines(self) -> Lines<StdinLock<'static>> {
        self.lock().lines()
    }
}

impl StdinLock<'_> {
    fn r(&mut self) -> &mut BufReader<Raw> {
        self.guard.as_mut().unwrap()
    }
}

impl Read for Stdin {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.lock().read(buf)
    }
    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        self.lock().read_to_end(buf)
    }
    fn read_to_string(&mut self, buf: &mut String) -> Result<usize> {
        self.lock().read_to_string(buf)
    }
}

impl Read for StdinLock<'_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.r().read(buf)
    }
    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        self.r().read_to_end(buf)
    }
}

impl BufRead for StdinLock<'_> {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        self.r().fill_buf()
    }
    fn consume(&mut self, amt: usize) {
        self.r().consume(amt)
    }
}

impl fmt::Debug for Stdin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Stdin { .. }")
    }
}

impl IsTerminal for Stdin {
    fn is_terminal(&self) -> bool {
        isatty(0)
    }
}

// ---- print! support ----

#[doc(hidden)]
pub fn _print(args: fmt::Arguments<'_>) {
    if let Err(e) = stdout().write_fmt(args) {
        panic!("failed printing to stdout: {e}");
    }
}

#[doc(hidden)]
pub fn _eprint(args: fmt::Arguments<'_>) {
    if let Err(e) = stderr().write_fmt(args) {
        panic!("failed printing to stderr: {e}");
    }
}

/// Flush stdout at process exit.
pub(crate) fn cleanup() {
    if let Ok(mut guard) = STDOUT.try_lock()
        && let Some(w) = guard.as_mut() {
            let _ = w.flush();
        }
}
