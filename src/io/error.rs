use alloc_crate::boxed::Box;
use core::error;
use core::fmt;

use crate::sys::linux;

pub type Result<T> = core::result::Result<T, Error>;

/// The error type for I/O operations, mirroring `std::io::Error`.
pub struct Error {
    repr: Repr,
}

enum Repr {
    Os(i32),
    Simple(ErrorKind),
    SimpleMessage(ErrorKind, &'static str),
    Custom(Box<Custom>),
}

struct Custom {
    kind: ErrorKind,
    error: Box<dyn error::Error + Send + Sync>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum ErrorKind {
    NotFound,
    PermissionDenied,
    ConnectionRefused,
    ConnectionReset,
    HostUnreachable,
    NetworkUnreachable,
    ConnectionAborted,
    NotConnected,
    AddrInUse,
    AddrNotAvailable,
    NetworkDown,
    BrokenPipe,
    AlreadyExists,
    WouldBlock,
    NotADirectory,
    IsADirectory,
    DirectoryNotEmpty,
    ReadOnlyFilesystem,
    FilesystemLoop,
    StaleNetworkFileHandle,
    InvalidInput,
    InvalidData,
    TimedOut,
    WriteZero,
    StorageFull,
    NotSeekable,
    QuotaExceeded,
    FileTooLarge,
    ResourceBusy,
    ExecutableFileBusy,
    Deadlock,
    CrossesDevices,
    TooManyLinks,
    InvalidFilename,
    ArgumentListTooLong,
    Interrupted,
    Unsupported,
    UnexpectedEof,
    OutOfMemory,
    InProgress,
    Other,
    Uncategorized,
}

impl ErrorKind {
    fn as_str(&self) -> &'static str {
        use ErrorKind::*;
        match *self {
            NotFound => "entity not found",
            PermissionDenied => "permission denied",
            ConnectionRefused => "connection refused",
            ConnectionReset => "connection reset",
            HostUnreachable => "host unreachable",
            NetworkUnreachable => "network unreachable",
            ConnectionAborted => "connection aborted",
            NotConnected => "not connected",
            AddrInUse => "address in use",
            AddrNotAvailable => "address not available",
            NetworkDown => "network down",
            BrokenPipe => "broken pipe",
            AlreadyExists => "entity already exists",
            WouldBlock => "operation would block",
            NotADirectory => "not a directory",
            IsADirectory => "is a directory",
            DirectoryNotEmpty => "directory not empty",
            ReadOnlyFilesystem => "read-only filesystem or storage medium",
            FilesystemLoop => "filesystem loop or indirection limit (e.g. symlink loop)",
            StaleNetworkFileHandle => "stale network file handle",
            InvalidInput => "invalid input parameter",
            InvalidData => "invalid data",
            TimedOut => "timed out",
            WriteZero => "write zero",
            StorageFull => "no storage space",
            NotSeekable => "seek on unseekable file",
            QuotaExceeded => "quota exceeded",
            FileTooLarge => "file too large",
            ResourceBusy => "resource busy",
            ExecutableFileBusy => "executable file busy",
            Deadlock => "deadlock",
            CrossesDevices => "cross-device link or rename",
            TooManyLinks => "too many links",
            InvalidFilename => "invalid filename",
            ArgumentListTooLong => "argument list too long",
            Interrupted => "operation interrupted",
            Unsupported => "unsupported",
            UnexpectedEof => "unexpected end of file",
            OutOfMemory => "out of memory",
            InProgress => "in progress",
            Other => "other error",
            Uncategorized => "uncategorized error",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

fn decode_errno(e: i32) -> ErrorKind {
    use ErrorKind::*;
    match e {
        linux::EPERM | linux::EACCES => PermissionDenied,
        linux::ENOENT => NotFound,
        linux::EINTR => Interrupted,
        linux::EAGAIN => WouldBlock,
        linux::ENOMEM => OutOfMemory,
        linux::EBUSY => ResourceBusy,
        linux::EEXIST => AlreadyExists,
        linux::EXDEV => CrossesDevices,
        linux::ENOTDIR => NotADirectory,
        linux::EISDIR => IsADirectory,
        linux::EINVAL => InvalidInput,
        linux::ETXTBSY => ExecutableFileBusy,
        linux::EFBIG => FileTooLarge,
        linux::ENOSPC => StorageFull,
        linux::ESPIPE => NotSeekable,
        linux::EROFS => ReadOnlyFilesystem,
        linux::EMLINK => TooManyLinks,
        linux::EPIPE => BrokenPipe,
        linux::EDEADLK => Deadlock,
        linux::ENAMETOOLONG => InvalidFilename,
        linux::ENOSYS => Unsupported,
        linux::ENOTEMPTY => DirectoryNotEmpty,
        linux::ELOOP => FilesystemLoop,
        linux::EADDRINUSE => AddrInUse,
        linux::EADDRNOTAVAIL => AddrNotAvailable,
        linux::ENETDOWN => NetworkDown,
        linux::ENETUNREACH => NetworkUnreachable,
        linux::ECONNABORTED => ConnectionAborted,
        linux::ECONNRESET => ConnectionReset,
        linux::ENOTCONN => NotConnected,
        linux::ETIMEDOUT => TimedOut,
        linux::ECONNREFUSED => ConnectionRefused,
        linux::EHOSTUNREACH => HostUnreachable,
        linux::EINPROGRESS => InProgress,
        7 => ArgumentListTooLong,
        116 => StaleNetworkFileHandle,
        122 => QuotaExceeded,
        _ => Uncategorized,
    }
}

/// Build an `io::Error` from a kind and a static message without allocating.
#[macro_export]
#[doc(hidden)]
macro_rules! __io_const_error {
    ($kind:expr, $msg:expr $(,)?) => {
        $crate::io::Error::__from_static($kind, $msg)
    };
}
#[doc(hidden)]
pub use crate::__io_const_error as const_error;

impl Error {
    pub fn new<E>(kind: ErrorKind, error: E) -> Error
    where
        E: Into<Box<dyn error::Error + Send + Sync>>,
    {
        Error { repr: Repr::Custom(Box::new(Custom { kind, error: error.into() })) }
    }

    pub fn other<E>(error: E) -> Error
    where
        E: Into<Box<dyn error::Error + Send + Sync>>,
    {
        Self::new(ErrorKind::Other, error)
    }

    #[doc(hidden)]
    pub const fn __from_static(kind: ErrorKind, msg: &'static str) -> Error {
        Error { repr: Repr::SimpleMessage(kind, msg) }
    }

    pub fn last_os_error() -> Error {
        // there is no errno variable without libc; syscall wrappers return it
        Error::from(ErrorKind::Uncategorized)
    }

    pub fn from_raw_os_error(code: i32) -> Error {
        Error { repr: Repr::Os(code) }
    }

    pub fn raw_os_error(&self) -> Option<i32> {
        match self.repr {
            Repr::Os(c) => Some(c),
            _ => None,
        }
    }

    pub fn get_ref(&self) -> Option<&(dyn error::Error + Send + Sync + 'static)> {
        match &self.repr {
            Repr::Custom(c) => Some(&*c.error),
            _ => None,
        }
    }

    pub fn get_mut(&mut self) -> Option<&mut (dyn error::Error + Send + Sync + 'static)> {
        match &mut self.repr {
            Repr::Custom(c) => Some(&mut *c.error),
            _ => None,
        }
    }

    pub fn into_inner(self) -> Option<Box<dyn error::Error + Send + Sync>> {
        match self.repr {
            Repr::Custom(c) => Some(c.error),
            _ => None,
        }
    }

    pub fn kind(&self) -> ErrorKind {
        match &self.repr {
            Repr::Os(c) => decode_errno(*c),
            Repr::Simple(k) | Repr::SimpleMessage(k, _) => *k,
            Repr::Custom(c) => c.kind,
        }
    }
}

impl From<ErrorKind> for Error {
    fn from(kind: ErrorKind) -> Error {
        Error { repr: Repr::Simple(kind) }
    }
}

impl From<alloc_crate::ffi::NulError> for Error {
    fn from(_: alloc_crate::ffi::NulError) -> Error {
        const_error!(ErrorKind::InvalidInput, "data provided contains a nul byte")
    }
}

impl From<alloc_crate::collections::TryReserveError> for Error {
    fn from(_: alloc_crate::collections::TryReserveError) -> Error {
        ErrorKind::OutOfMemory.into()
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.repr {
            Repr::Os(code) => f
                .debug_struct("Os")
                .field("code", code)
                .field("kind", &decode_errno(*code))
                .field("message", &linux::strerror(*code))
                .finish(),
            Repr::Simple(kind) => f.debug_tuple("Kind").field(kind).finish(),
            Repr::SimpleMessage(kind, msg) => {
                f.debug_struct("Error").field("kind", kind).field("message", msg).finish()
            }
            Repr::Custom(c) => f.debug_struct("Custom").field("kind", &c.kind).field("error", &c.error).finish(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.repr {
            Repr::Os(code) => write!(f, "{} (os error {code})", linux::strerror(*code)),
            Repr::Simple(kind) => f.write_str(kind.as_str()),
            Repr::SimpleMessage(_, msg) => f.write_str(msg),
            Repr::Custom(c) => c.error.fmt(f),
        }
    }
}

impl error::Error for Error {
    #[allow(deprecated)]
    fn description(&self) -> &str {
        match &self.repr {
            Repr::Os(_) | Repr::Simple(_) => self.kind().as_str(),
            Repr::SimpleMessage(_, msg) => msg,
            Repr::Custom(c) => c.error.description(),
        }
    }

    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.repr {
            Repr::Custom(c) => c.error.source(),
            _ => None,
        }
    }
}

/// Error wrapper returned by `BufWriter::into_inner`.
#[derive(Debug)]
pub struct IntoInnerError<W>(W, Error);

impl<W> IntoInnerError<W> {
    pub(crate) fn new(w: W, e: Error) -> Self {
        IntoInnerError(w, e)
    }
    pub(crate) fn into_parts(self) -> (W, Error) {
        (self.0, self.1)
    }
    pub fn error(&self) -> &Error {
        &self.1
    }
    pub fn into_inner(self) -> W {
        self.0
    }
    pub fn into_error(self) -> Error {
        self.1
    }
}

impl<W> fmt::Display for IntoInnerError<W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.1.fmt(f)
    }
}

impl<W: fmt::Debug> error::Error for IntoInnerError<W> {}

