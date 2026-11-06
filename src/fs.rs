//! Filesystem manipulation operations.

use alloc_crate::string::String;
use alloc_crate::sync::Arc;
use alloc_crate::vec::Vec;
use core::fmt;

use crate::ffi::OsString;
use crate::io::{self, Read, Seek, SeekFrom, Write};
use crate::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use crate::path::{Path, PathBuf};
use crate::sys::fd::{self as sfd, cvt, cvt_r};
use crate::sys::linux::{self, AT_FDCWD, Statx};
use crate::sys::path::with_path;
use crate::time::SystemTime;

pub struct File {
    fd: OwnedFd,
}

crate::os::fd::impl_owned_fd!(File, fd);

#[derive(Clone)]
pub struct Metadata {
    pub(crate) stat: Statx,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct FileType {
    mode: u32,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Permissions {
    pub(crate) mode: u32,
}

#[derive(Clone, Debug)]
pub struct OpenOptions {
    read: bool,
    write: bool,
    append: bool,
    truncate: bool,
    create: bool,
    create_new: bool,
    pub(crate) mode: u32,
    pub(crate) custom_flags: u32,
}

// ---- File ----

impl File {
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<File> {
        OpenOptions::new().read(true).open(path.as_ref())
    }

    pub fn create<P: AsRef<Path>>(path: P) -> io::Result<File> {
        OpenOptions::new().write(true).create(true).truncate(true).open(path.as_ref())
    }

    pub fn create_new<P: AsRef<Path>>(path: P) -> io::Result<File> {
        OpenOptions::new().read(true).write(true).create_new(true).open(path.as_ref())
    }

    pub fn options() -> OpenOptions {
        OpenOptions::new()
    }

    fn raw(&self) -> i32 {
        self.fd.as_raw_fd()
    }

    pub fn sync_all(&self) -> io::Result<()> {
        cvt_r(|| linux::fsync(self.raw())).map(drop)
    }

    pub fn sync_data(&self) -> io::Result<()> {
        cvt_r(|| linux::fdatasync(self.raw())).map(drop)
    }

    pub fn set_len(&self, size: u64) -> io::Result<()> {
        cvt_r(|| linux::ftruncate(self.raw(), size)).map(drop)
    }

    pub fn metadata(&self) -> io::Result<Metadata> {
        fstatx(self.raw())
    }

    pub fn try_clone(&self) -> io::Result<File> {
        Ok(File { fd: self.fd.try_clone()? })
    }

    pub fn set_permissions(&self, perm: Permissions) -> io::Result<()> {
        cvt_r(|| linux::fchmod(self.raw(), perm.mode)).map(drop)
    }
}

fn file_seek(fd: i32, pos: SeekFrom) -> io::Result<u64> {
    let (off, whence) = match pos {
        SeekFrom::Start(n) => (n as i64, 0),
        SeekFrom::Current(n) => (n, 1),
        SeekFrom::End(n) => (n, 2),
    };
    cvt(linux::lseek(fd, off, whence)).map(|n| n as u64)
}

fn read_to_end_sized(fd: i32, buf: &mut Vec<u8>) -> io::Result<usize> {
    // pre-size the buffer from the file length to avoid repeated growth
    if let Ok(m) = fstatx(fd) {
        let pos = linux::lseek(fd, 0, 1).unwrap_or(0) as u64;
        let hint = m.len().saturating_sub(pos) as usize;
        buf.try_reserve(hint)?;
    }
    io::default_read_to_end(&mut FdReader(fd), buf)
}

struct FdReader(i32);

impl Read for FdReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        sfd::read(self.0, buf)
    }
}

impl Read for &File {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        sfd::read(self.raw(), buf)
    }
    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> io::Result<usize> {
        read_to_end_sized(self.raw(), buf)
    }
    fn read_to_string(&mut self, buf: &mut String) -> io::Result<usize> {
        io::default_read_to_string(self, buf)
    }
}

impl Write for &File {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        sfd::write(self.raw(), buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for &File {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        file_seek(self.raw(), pos)
    }
}

impl Read for File {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&*self).read(buf)
    }
    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> io::Result<usize> {
        (&*self).read_to_end(buf)
    }
    fn read_to_string(&mut self, buf: &mut String) -> io::Result<usize> {
        (&*self).read_to_string(buf)
    }
}

impl Write for File {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&*self).write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for File {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        (&*self).seek(pos)
    }
}

impl io::IsTerminal for File {
    fn is_terminal(&self) -> bool {
        io::stdio::isatty(self.raw())
    }
}

impl fmt::Debug for File {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("File");
        d.field("fd", &self.raw());
        let link = PathBuf::from(alloc_crate::format!("/proc/self/fd/{}", self.raw()));
        if let Ok(path) = read_link(&link) {
            d.field("path", &path);
        }
        d.finish()
    }
}

// ---- OpenOptions ----

impl OpenOptions {
    pub fn new() -> OpenOptions {
        OpenOptions {
            read: false,
            write: false,
            append: false,
            truncate: false,
            create: false,
            create_new: false,
            mode: 0o666,
            custom_flags: 0,
        }
    }

    pub fn read(&mut self, read: bool) -> &mut OpenOptions {
        self.read = read;
        self
    }
    pub fn write(&mut self, write: bool) -> &mut OpenOptions {
        self.write = write;
        self
    }
    pub fn append(&mut self, append: bool) -> &mut OpenOptions {
        self.append = append;
        self
    }
    pub fn truncate(&mut self, truncate: bool) -> &mut OpenOptions {
        self.truncate = truncate;
        self
    }
    pub fn create(&mut self, create: bool) -> &mut OpenOptions {
        self.create = create;
        self
    }
    pub fn create_new(&mut self, create_new: bool) -> &mut OpenOptions {
        self.create_new = create_new;
        self
    }

    fn flags(&self) -> io::Result<u32> {
        let invalid = || Err(io::Error::from_raw_os_error(linux::EINVAL));
        let access = match (self.read, self.write, self.append) {
            (true, false, false) => linux::O_RDONLY,
            (false, true, false) => linux::O_WRONLY,
            (true, true, false) => linux::O_RDWR,
            (false, _, true) => linux::O_WRONLY | linux::O_APPEND,
            (true, _, true) => linux::O_RDWR | linux::O_APPEND,
            (false, false, false) => return invalid(),
        };
        if !self.write && !self.append && (self.truncate || self.create || self.create_new) {
            return invalid();
        }
        if self.append && self.truncate && !self.create_new {
            return invalid();
        }
        let creation = match (self.create, self.truncate, self.create_new) {
            (false, false, false) => 0,
            (true, false, false) => linux::O_CREAT,
            (false, true, false) => linux::O_TRUNC,
            (true, true, false) => linux::O_CREAT | linux::O_TRUNC,
            (_, _, true) => linux::O_CREAT | linux::O_EXCL,
        };
        Ok(access | creation | linux::O_CLOEXEC | (self.custom_flags & !linux::O_APPEND))
    }

    pub fn open<P: AsRef<Path>>(&self, path: P) -> io::Result<File> {
        let flags = self.flags()?;
        with_path(path.as_ref(), |p| {
            let fd = cvt_r(|| unsafe { linux::openat(AT_FDCWD, p, flags, self.mode) })?;
            Ok(File { fd: unsafe { OwnedFd::from_raw_fd(fd) } })
        })
    }
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self::new()
    }
}

// ---- Metadata ----

fn statx_at(dirfd: i32, p: *const u8, flags: u32) -> io::Result<Metadata> {
    let mut st = Statx::default();
    cvt(unsafe { linux::statx(dirfd, p, flags, linux::STATX_BASIC_STATS | linux::STATX_BTIME, &mut st) })?;
    Ok(Metadata { stat: st })
}

fn fstatx(fd: i32) -> io::Result<Metadata> {
    statx_at(fd, c"".as_ptr().cast::<u8>(), linux::AT_EMPTY_PATH)
}

fn ts(t: &linux::StatxTimestamp) -> SystemTime {
    SystemTime::from_parts(t.sec, t.nsec)
}

#[allow(clippy::len_without_is_empty)] // matches std's API
impl Metadata {
    pub fn file_type(&self) -> FileType {
        FileType { mode: self.stat.mode as u32 }
    }
    pub fn is_dir(&self) -> bool {
        self.file_type().is_dir()
    }
    pub fn is_file(&self) -> bool {
        self.file_type().is_file()
    }
    pub fn is_symlink(&self) -> bool {
        self.file_type().is_symlink()
    }
    pub fn len(&self) -> u64 {
        self.stat.size
    }
    pub fn permissions(&self) -> Permissions {
        Permissions { mode: self.stat.mode as u32 & 0o7777 }
    }
    pub fn modified(&self) -> io::Result<SystemTime> {
        Ok(ts(&self.stat.mtime))
    }
    pub fn accessed(&self) -> io::Result<SystemTime> {
        Ok(ts(&self.stat.atime))
    }
    pub fn created(&self) -> io::Result<SystemTime> {
        if self.stat.mask & linux::STATX_BTIME != 0 {
            Ok(ts(&self.stat.btime))
        } else {
            Err(io::const_error!(io::ErrorKind::Unsupported, "creation time is not available for the filesystem"))
        }
    }
}

impl fmt::Debug for Metadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Metadata")
            .field("file_type", &self.file_type())
            .field("permissions", &self.permissions())
            .field("len", &self.len())
            .field("modified", &self.modified().ok())
            .finish_non_exhaustive()
    }
}

impl FileType {
    pub(crate) fn is(&self, fmt: u32) -> bool {
        self.mode & linux::S_IFMT == fmt
    }
    pub fn is_dir(&self) -> bool {
        self.is(linux::S_IFDIR)
    }
    pub fn is_file(&self) -> bool {
        self.is(linux::S_IFREG)
    }
    pub fn is_symlink(&self) -> bool {
        self.is(linux::S_IFLNK)
    }
}

impl fmt::Debug for FileType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileType")
            .field("is_file", &self.is_file())
            .field("is_dir", &self.is_dir())
            .field("is_symlink", &self.is_symlink())
            .finish_non_exhaustive()
    }
}

impl Permissions {
    pub fn readonly(&self) -> bool {
        self.mode & 0o222 == 0
    }
    pub fn set_readonly(&mut self, readonly: bool) {
        if readonly { self.mode &= !0o222 } else { self.mode |= 0o222 }
    }
}

// ---- directories ----

pub struct ReadDir {
    fd: OwnedFd,
    root: Arc<PathBuf>,
    buf: Vec<u8>,
    pos: usize,
    len: usize,
    end: bool,
}

pub struct DirEntry {
    root: Arc<PathBuf>,
    name: OsString,
    pub(crate) ino: u64,
    d_type: u8,
}

pub fn read_dir<P: AsRef<Path>>(path: P) -> io::Result<ReadDir> {
    let path = path.as_ref();
    let fd = with_path(path, |p| {
        cvt_r(|| unsafe { linux::openat(AT_FDCWD, p, linux::O_RDONLY | linux::O_DIRECTORY | linux::O_CLOEXEC, 0) })
    })?;
    Ok(ReadDir {
        fd: unsafe { OwnedFd::from_raw_fd(fd) },
        root: Arc::new(path.to_path_buf()),
        buf: alloc_crate::vec![0; 8192],
        pos: 0,
        len: 0,
        end: false,
    })
}

impl Iterator for ReadDir {
    type Item = io::Result<DirEntry>;

    fn next(&mut self) -> Option<io::Result<DirEntry>> {
        loop {
            if self.pos >= self.len {
                if self.end {
                    return None;
                }
                match cvt(unsafe { linux::getdents64(self.fd.as_raw_fd(), self.buf.as_mut_ptr(), self.buf.len()) }) {
                    Ok(0) => {
                        self.end = true;
                        return None;
                    }
                    Ok(n) => {
                        self.len = n;
                        self.pos = 0;
                    }
                    Err(e) => {
                        self.end = true;
                        return Some(Err(e));
                    }
                }
            }
            // struct linux_dirent64 { u64 ino; i64 off; u16 reclen; u8 type; char name[]; }
            let rec = &self.buf[self.pos..self.len];
            let ino = u64::from_ne_bytes(rec[0..8].try_into().unwrap());
            let reclen = u16::from_ne_bytes(rec[16..18].try_into().unwrap()) as usize;
            let d_type = rec[18];
            let name_bytes = &rec[19..reclen];
            let name_len = name_bytes.iter().position(|&b| b == 0).unwrap_or(name_bytes.len());
            let name = &name_bytes[..name_len];
            self.pos += reclen;
            if name == b"." || name == b".." {
                continue;
            }
            return Some(Ok(DirEntry {
                root: self.root.clone(),
                name: OsString::from_vec(name.to_vec()),
                ino,
                d_type,
            }));
        }
    }
}

impl fmt::Debug for ReadDir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ReadDir").field(&*self.root).finish()
    }
}

impl DirEntry {
    pub fn path(&self) -> PathBuf {
        self.root.join(&self.name)
    }

    pub fn metadata(&self) -> io::Result<Metadata> {
        symlink_metadata(self.path())
    }

    pub fn file_type(&self) -> io::Result<FileType> {
        const DT_FIFO: u8 = 1;
        const DT_CHR: u8 = 2;
        const DT_DIR: u8 = 4;
        const DT_BLK: u8 = 6;
        const DT_REG: u8 = 8;
        const DT_LNK: u8 = 10;
        const DT_SOCK: u8 = 12;
        let mode = match self.d_type {
            DT_FIFO => linux::S_IFIFO,
            DT_CHR => linux::S_IFCHR,
            DT_DIR => linux::S_IFDIR,
            DT_BLK => linux::S_IFBLK,
            DT_REG => linux::S_IFREG,
            DT_LNK => linux::S_IFLNK,
            DT_SOCK => linux::S_IFSOCK,
            _ => return self.metadata().map(|m| m.file_type()),
        };
        Ok(FileType { mode })
    }

    pub fn file_name(&self) -> OsString {
        self.name.clone()
    }
}

impl fmt::Debug for DirEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DirEntry").field(&self.path()).finish()
    }
}

#[derive(Debug)]
pub struct DirBuilder {
    mode: u32,
    recursive: bool,
}

impl DirBuilder {
    pub fn new() -> DirBuilder {
        DirBuilder { mode: 0o777, recursive: false }
    }

    pub fn recursive(&mut self, recursive: bool) -> &mut Self {
        self.recursive = recursive;
        self
    }

    pub fn mode(&mut self, mode: u32) -> &mut Self {
        self.mode = mode;
        self
    }

    pub fn create<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        if self.recursive { self.create_all(path.as_ref()) } else { self.mkdir(path.as_ref()) }
    }

    fn mkdir(&self, path: &Path) -> io::Result<()> {
        with_path(path, |p| cvt(unsafe { linux::mkdirat(AT_FDCWD, p, self.mode) })).map(drop)
    }

    fn create_all(&self, path: &Path) -> io::Result<()> {
        if path == Path::new("") {
            return Ok(());
        }
        match self.mkdir(path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) if path.is_dir() => return Ok(()),
            Err(e) => return Err(e),
        }
        match path.parent() {
            Some(p) => self.create_all(p)?,
            None => return Err(io::const_error!(io::ErrorKind::Uncategorized, "failed to create whole tree")),
        }
        match self.mkdir(path) {
            Ok(()) => Ok(()),
            Err(_) if path.is_dir() => Ok(()),
            Err(e) => Err(e),
        }
    }
}

impl Default for DirBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ---- free functions ----

pub fn read<P: AsRef<Path>>(path: P) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

pub fn read_to_string<P: AsRef<Path>>(path: P) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut s = String::new();
    file.read_to_string(&mut s)?;
    Ok(s)
}

pub fn write<P: AsRef<Path>, C: AsRef<[u8]>>(path: P, contents: C) -> io::Result<()> {
    File::create(path)?.write_all(contents.as_ref())
}

pub fn metadata<P: AsRef<Path>>(path: P) -> io::Result<Metadata> {
    with_path(path.as_ref(), |p| statx_at(AT_FDCWD, p, 0))
}

pub fn symlink_metadata<P: AsRef<Path>>(path: P) -> io::Result<Metadata> {
    with_path(path.as_ref(), |p| statx_at(AT_FDCWD, p, linux::AT_SYMLINK_NOFOLLOW))
}

pub fn exists<P: AsRef<Path>>(path: P) -> io::Result<bool> {
    match metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

pub fn create_dir<P: AsRef<Path>>(path: P) -> io::Result<()> {
    DirBuilder::new().create(path)
}

pub fn create_dir_all<P: AsRef<Path>>(path: P) -> io::Result<()> {
    DirBuilder::new().recursive(true).create(path)
}

pub fn remove_file<P: AsRef<Path>>(path: P) -> io::Result<()> {
    with_path(path.as_ref(), |p| cvt(unsafe { linux::unlinkat(AT_FDCWD, p, 0) })).map(drop)
}

pub fn remove_dir<P: AsRef<Path>>(path: P) -> io::Result<()> {
    with_path(path.as_ref(), |p| cvt(unsafe { linux::unlinkat(AT_FDCWD, p, linux::AT_REMOVEDIR) })).map(drop)
}

/// Removes a directory and everything inside it. Symlinks are removed, never
/// followed.
pub fn remove_dir_all<P: AsRef<Path>>(path: P) -> io::Result<()> {
    let path = path.as_ref();
    if symlink_metadata(path)?.is_symlink() {
        return remove_file(path);
    }
    for entry in read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            remove_dir_all(entry.path())?;
        } else {
            remove_file(entry.path())?;
        }
    }
    remove_dir(path)
}

pub fn rename<P: AsRef<Path>, Q: AsRef<Path>>(from: P, to: Q) -> io::Result<()> {
    with_path(from.as_ref(), |f| {
        with_path(to.as_ref(), |t| cvt(unsafe { linux::renameat2(AT_FDCWD, f, AT_FDCWD, t, 0) }))
    })
    .map(drop)
}

pub fn hard_link<P: AsRef<Path>, Q: AsRef<Path>>(original: P, link: Q) -> io::Result<()> {
    with_path(original.as_ref(), |o| {
        with_path(link.as_ref(), |l| cvt(unsafe { linux::linkat(AT_FDCWD, o, AT_FDCWD, l, 0) }))
    })
    .map(drop)
}

pub(crate) fn symlink_inner(original: &Path, link: &Path) -> io::Result<()> {
    with_path(original, |o| with_path(link, |l| cvt(unsafe { linux::symlinkat(o, AT_FDCWD, l) }))).map(drop)
}

#[deprecated(note = "replaced with std::os::unix::fs::symlink")]
pub fn soft_link<P: AsRef<Path>, Q: AsRef<Path>>(original: P, link: Q) -> io::Result<()> {
    symlink_inner(original.as_ref(), link.as_ref())
}

pub fn read_link<P: AsRef<Path>>(path: P) -> io::Result<PathBuf> {
    with_path(path.as_ref(), |p| {
        let mut buf: Vec<u8> = Vec::with_capacity(256);
        loop {
            let n = cvt(unsafe { linux::readlinkat(AT_FDCWD, p, buf.as_mut_ptr(), buf.capacity()) })?;
            if n < buf.capacity() {
                unsafe { buf.set_len(n) };
                return Ok(PathBuf::from(OsString::from_vec(buf)));
            }
            buf.reserve(buf.capacity());
        }
    })
}

pub fn canonicalize<P: AsRef<Path>>(path: P) -> io::Result<PathBuf> {
    // open the path itself (O_PATH) and ask the kernel where it ended up
    let fd = with_path(path.as_ref(), |p| {
        cvt_r(|| unsafe { linux::openat(AT_FDCWD, p, linux::O_PATH | linux::O_CLOEXEC, 0) })
    })?;
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    read_link(alloc_crate::format!("/proc/self/fd/{}", fd.as_raw_fd()))
}

pub fn set_permissions<P: AsRef<Path>>(path: P, perm: Permissions) -> io::Result<()> {
    with_path(path.as_ref(), |p| cvt(unsafe { linux::fchmodat(AT_FDCWD, p, perm.mode) })).map(drop)
}

pub fn copy<P: AsRef<Path>, Q: AsRef<Path>>(from: P, to: Q) -> io::Result<u64> {
    let mut reader = File::open(from.as_ref())?;
    let meta = reader.metadata()?;
    if !meta.is_file() {
        return Err(io::const_error!(io::ErrorKind::InvalidInput, "the source path is neither a regular file nor a symlink to a regular file"));
    }
    let mut writer = OpenOptions::new().write(true).create(true).truncate(true).open(to.as_ref())?;
    let n = io::copy(&mut reader, &mut writer)?;
    writer.set_permissions(meta.permissions())?;
    Ok(n)
}
