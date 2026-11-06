//! Cross-platform path manipulation (Unix semantics: `/` separated bytes).

use alloc_crate::borrow::{Cow, ToOwned};
use alloc_crate::boxed::Box;
use alloc_crate::rc::Rc;
use alloc_crate::string::String;
use alloc_crate::sync::Arc;
use core::borrow::Borrow;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::iter::FusedIterator;
use core::ops::Deref;

use crate::ffi::{OsStr, OsString};
use crate::fs;
use crate::io;

pub const MAIN_SEPARATOR: char = '/';
pub const MAIN_SEPARATOR_STR: &str = "/";

pub fn is_separator(c: char) -> bool {
    c == '/'
}

#[repr(transparent)]
pub struct Path {
    inner: OsStr,
}

#[derive(Clone, Default)]
pub struct PathBuf {
    inner: OsString,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Component<'a> {
    Prefix(PrefixComponent<'a>),
    RootDir,
    CurDir,
    ParentDir,
    Normal(&'a OsStr),
}

/// Windows path prefixes never occur on Linux; kept for API compatibility.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PrefixComponent<'a> {
    raw: &'a OsStr,
}

impl<'a> PrefixComponent<'a> {
    pub fn as_os_str(&self) -> &'a OsStr {
        self.raw
    }
}

impl<'a> Component<'a> {
    pub fn as_os_str(self) -> &'a OsStr {
        match self {
            Component::Prefix(p) => p.as_os_str(),
            Component::RootDir => OsStr::new("/"),
            Component::CurDir => OsStr::new("."),
            Component::ParentDir => OsStr::new(".."),
            Component::Normal(p) => p,
        }
    }
}

impl AsRef<OsStr> for Component<'_> {
    fn as_ref(&self) -> &OsStr {
        self.as_os_str()
    }
}

impl AsRef<Path> for Component<'_> {
    fn as_ref(&self) -> &Path {
        self.as_os_str().as_ref()
    }
}

/// Iterator over the components of a path.
#[derive(Clone)]
pub struct Components<'a> {
    path: &'a [u8],
    // whether the root has been yielded (front) and whether the path has one
    has_root: bool,
    front_done_root: bool,
    // a leading "." is the only CurDir that is reported
    front_cur_dir_checked: bool,
}

impl<'a> Components<'a> {
    fn new(path: &'a [u8]) -> Components<'a> {
        let has_root = path.first() == Some(&b'/');
        Components { path, has_root, front_done_root: false, front_cur_dir_checked: false }
    }

    fn parse(seg: &'a [u8]) -> Option<Component<'a>> {
        match seg {
            b"" | b"." => None,
            b".." => Some(Component::ParentDir),
            _ => Some(Component::Normal(OsStr::from_bytes(seg))),
        }
    }

    fn strip_root(&mut self) {
        if self.has_root && !self.front_done_root {
            self.front_done_root = true;
            while self.path.first() == Some(&b'/') {
                self.path = &self.path[1..];
            }
        }
    }

    /// The remaining path as a `Path`. Like std, empty and `.` components
    /// are trimmed from the right, and from the left once iteration has
    /// moved past the root / leading `.`.
    pub fn as_path(&self) -> &'a Path {
        let mut p = self.path;
        let root_pending = self.has_root && !self.front_done_root;
        let cur_pending = !self.front_cur_dir_checked && !self.has_root && (p == b"." || p.starts_with(b"./"));
        let front_in_body = if self.has_root { self.front_done_root } else { self.front_cur_dir_checked };
        if front_in_body {
            loop {
                if p.first() == Some(&b'/') || p == b"." || p.starts_with(b"./") {
                    p = &p[1..];
                } else {
                    break;
                }
            }
        }
        // as in std, the root is one byte: extra leading slashes are empty
        // components that get trimmed like any others
        let before_body = if root_pending || cur_pending { 1 } else { 0 };
        while p.len() > before_body {
            let n = p.len();
            let dot_segment = p[n - 1] == b'.' && (n - 1 == before_body || p[n - 2] == b'/');
            if p[n - 1] == b'/' || dot_segment {
                p = &p[..n - 1];
            } else {
                break;
            }
        }
        Path::from_bytes(p)
    }
}

impl<'a> Iterator for Components<'a> {
    type Item = Component<'a>;

    fn next(&mut self) -> Option<Component<'a>> {
        if self.has_root && !self.front_done_root {
            self.strip_root();
            return Some(Component::RootDir);
        }
        if !self.front_cur_dir_checked {
            self.front_cur_dir_checked = true;
            if !self.has_root && (self.path == b"." || self.path.starts_with(b"./")) {
                self.path = &self.path[1..];
                return Some(Component::CurDir);
            }
        }
        loop {
            if self.path.is_empty() {
                return None;
            }
            let end = self.path.iter().position(|&b| b == b'/').unwrap_or(self.path.len());
            let seg = &self.path[..end];
            self.path = if end < self.path.len() { &self.path[end + 1..] } else { &[] };
            if let Some(c) = Self::parse(seg) {
                return Some(c);
            }
        }
    }
}

impl<'a> DoubleEndedIterator for Components<'a> {
    fn next_back(&mut self) -> Option<Component<'a>> {
        loop {
            let root_pending = self.has_root && !self.front_done_root;
            let mut offset = 0;
            if root_pending {
                while self.path.get(offset) == Some(&b'/') {
                    offset += 1;
                }
            }
            let rest = &self.path[offset..];
            let cur_pending = !self.front_cur_dir_checked && !self.has_root && (rest == b"." || rest.starts_with(b"./"));
            if cur_pending {
                offset += 1;
            }
            let body = &self.path[offset..];
            let end = body.iter().rposition(|&b| b != b'/').map_or(0, |i| i + 1);
            if end == 0 {
                self.path = &[];
                if cur_pending {
                    self.front_cur_dir_checked = true;
                    return Some(Component::CurDir);
                }
                if root_pending {
                    self.front_done_root = true;
                    return Some(Component::RootDir);
                }
                return None;
            }
            let start = body[..end].iter().rposition(|&b| b == b'/').map_or(0, |i| i + 1);
            let seg = &body[start..end];
            self.path = &self.path[..offset + start];
            if let Some(c) = Self::parse(seg) {
                return Some(c);
            }
        }
    }
}

impl FusedIterator for Components<'_> {}

struct DebugComponents<'a>(&'a Path);

impl fmt::Debug for DebugComponents<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.0.components()).finish()
    }
}

struct DebugIter<'a>(&'a Path);

impl fmt::Debug for DebugIter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.0.iter()).finish()
    }
}

impl fmt::Debug for Components<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Components").field(&DebugComponents(self.as_path())).finish()
    }
}

impl PartialEq for Components<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.clone().eq(other.clone())
    }
}

/// Iterator over path components as `&OsStr`.
#[derive(Clone)]
pub struct Iter<'a> {
    inner: Components<'a>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a OsStr;
    fn next(&mut self) -> Option<&'a OsStr> {
        self.inner.next().map(Component::as_os_str)
    }
}

impl<'a> DoubleEndedIterator for Iter<'a> {
    fn next_back(&mut self) -> Option<&'a OsStr> {
        self.inner.next_back().map(Component::as_os_str)
    }
}

impl<'a> Iter<'a> {
    pub fn as_path(&self) -> &'a Path {
        self.inner.as_path()
    }
}

impl fmt::Debug for Iter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Iter").field(&DebugIter(self.as_path())).finish()
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Ancestors<'a> {
    next: Option<&'a Path>,
}

impl<'a> Iterator for Ancestors<'a> {
    type Item = &'a Path;
    fn next(&mut self) -> Option<&'a Path> {
        let next = self.next;
        self.next = next.and_then(Path::parent);
        next
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripPrefixError(());

impl fmt::Display for StripPrefixError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("prefix not found")
    }
}

impl core::error::Error for StripPrefixError {}

impl Path {
    pub fn new<S: AsRef<OsStr> + ?Sized>(s: &S) -> &Path {
        unsafe { &*(s.as_ref() as *const OsStr as *const Path) }
    }

    fn from_bytes(b: &[u8]) -> &Path {
        Path::new(OsStr::from_bytes(b))
    }

    fn bytes(&self) -> &[u8] {
        self.inner.as_bytes()
    }

    pub fn as_os_str(&self) -> &OsStr {
        &self.inner
    }

    pub fn as_mut_os_str(&mut self) -> &mut OsStr {
        &mut self.inner
    }

    pub fn to_str(&self) -> Option<&str> {
        self.inner.to_str()
    }

    pub fn to_string_lossy(&self) -> Cow<'_, str> {
        self.inner.to_string_lossy()
    }

    pub fn to_path_buf(&self) -> PathBuf {
        PathBuf { inner: self.inner.to_os_string() }
    }

    pub fn is_absolute(&self) -> bool {
        self.has_root()
    }

    pub fn is_relative(&self) -> bool {
        !self.is_absolute()
    }

    pub fn has_root(&self) -> bool {
        self.bytes().first() == Some(&b'/')
    }

    pub fn parent(&self) -> Option<&Path> {
        let mut comps = self.components();
        let comp = comps.next_back();
        comp.and_then(|p| match p {
            Component::Normal(_) | Component::CurDir | Component::ParentDir => Some(comps.as_path()),
            _ => None,
        })
    }

    pub fn ancestors(&self) -> Ancestors<'_> {
        Ancestors { next: Some(self) }
    }

    pub fn file_name(&self) -> Option<&OsStr> {
        self.components().next_back().and_then(|p| match p {
            Component::Normal(p) => Some(p),
            _ => None,
        })
    }

    pub fn strip_prefix<P: AsRef<Path>>(&self, base: P) -> Result<&Path, StripPrefixError> {
        let mut iter = self.components();
        let mut prefix = base.as_ref().components();
        loop {
            let mut next = iter.clone();
            match (next.next(), prefix.next()) {
                (Some(ref x), Some(ref y)) if x == y => iter = next,
                (Some(_), Some(_)) | (None, Some(_)) => return Err(StripPrefixError(())),
                (_, None) => return Ok(iter.as_path()),
            }
        }
    }

    pub fn starts_with<P: AsRef<Path>>(&self, base: P) -> bool {
        self.strip_prefix(base).is_ok()
    }

    pub fn ends_with<P: AsRef<Path>>(&self, child: P) -> bool {
        let mut a = self.components();
        let mut b = child.as_ref().components();
        loop {
            match (a.next_back(), b.next_back()) {
                (_, None) => return true,
                (Some(x), Some(y)) if x == y => {}
                _ => return false,
            }
        }
    }

    fn split_file_at_dot(file: &[u8]) -> (Option<&[u8]>, Option<&[u8]>) {
        if file == b".." {
            return (Some(file), None);
        }
        let mut iter = file.rsplitn(2, |b| *b == b'.');
        let after = iter.next();
        let before = iter.next();
        if before == Some(b"") { (Some(file), None) } else { (before, after) }
    }

    pub fn file_stem(&self) -> Option<&OsStr> {
        self.file_name().map(|n| Self::split_file_at_dot(n.as_bytes())).and_then(|(b, a)| b.or(a)).map(OsStr::from_bytes)
    }

    pub fn file_prefix(&self) -> Option<&OsStr> {
        self.file_name().map(|name| {
            let b = name.as_bytes();
            let start = if b.first() == Some(&b'.') { 1 } else { 0 };
            let end = b[start..].iter().position(|&c| c == b'.').map_or(b.len(), |i| i + start);
            OsStr::from_bytes(&b[..end])
        })
    }

    pub fn extension(&self) -> Option<&OsStr> {
        self.file_name().map(|n| Self::split_file_at_dot(n.as_bytes())).and_then(|(b, a)| b.and(a)).map(OsStr::from_bytes)
    }

    pub fn join<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        let mut buf = self.to_path_buf();
        buf.push(path);
        buf
    }

    pub fn with_file_name<S: AsRef<OsStr>>(&self, file_name: S) -> PathBuf {
        let mut buf = self.to_path_buf();
        buf.set_file_name(file_name);
        buf
    }

    pub fn with_added_extension<S: AsRef<OsStr>>(&self, extension: S) -> PathBuf {
        let mut buf = self.to_path_buf();
        buf.add_extension(extension);
        buf
    }

    pub fn with_extension<S: AsRef<OsStr>>(&self, extension: S) -> PathBuf {
        // as std: drop the old extension first, then set the new one (so e.g.
        // "/..a" with "" gives "/..", whose file name is then None)
        let bytes = self.bytes();
        let keep = match self.extension() {
            None => bytes,
            Some(old) => &bytes[..bytes.len() - old.len()],
        };
        let mut buf = PathBuf::from(OsString::from_vec(keep.to_vec()));
        buf.set_extension(extension);
        buf
    }

    pub fn components(&self) -> Components<'_> {
        Components::new(self.bytes())
    }

    pub fn iter(&self) -> Iter<'_> {
        Iter { inner: self.components() }
    }

    pub fn display(&self) -> Display<'_> {
        Display { path: self }
    }

    pub fn metadata(&self) -> io::Result<fs::Metadata> {
        fs::metadata(self)
    }

    pub fn symlink_metadata(&self) -> io::Result<fs::Metadata> {
        fs::symlink_metadata(self)
    }

    pub fn canonicalize(&self) -> io::Result<PathBuf> {
        fs::canonicalize(self)
    }

    pub fn read_link(&self) -> io::Result<PathBuf> {
        fs::read_link(self)
    }

    pub fn read_dir(&self) -> io::Result<fs::ReadDir> {
        fs::read_dir(self)
    }

    pub fn exists(&self) -> bool {
        fs::metadata(self).is_ok()
    }

    pub fn try_exists(&self) -> io::Result<bool> {
        fs::exists(self)
    }

    pub fn is_file(&self) -> bool {
        fs::metadata(self).map(|m| m.is_file()).unwrap_or(false)
    }

    pub fn is_dir(&self) -> bool {
        fs::metadata(self).map(|m| m.is_dir()).unwrap_or(false)
    }

    pub fn is_symlink(&self) -> bool {
        fs::symlink_metadata(self).map(|m| m.is_symlink()).unwrap_or(false)
    }

    pub fn into_path_buf(self: Box<Path>) -> PathBuf {
        let raw = Box::into_raw(self) as *mut OsStr;
        PathBuf { inner: unsafe { Box::from_raw(raw) }.into_os_string() }
    }
}

pub struct Display<'a> {
    path: &'a Path,
}

impl fmt::Display for Display<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.path.inner.display(), f)
    }
}

impl fmt::Debug for Display<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.path, f)
    }
}

impl PathBuf {
    pub fn new() -> PathBuf {
        PathBuf { inner: OsString::new() }
    }

    pub fn with_capacity(capacity: usize) -> PathBuf {
        PathBuf { inner: OsString::with_capacity(capacity) }
    }

    pub fn as_path(&self) -> &Path {
        self
    }

    pub fn push<P: AsRef<Path>>(&mut self, path: P) {
        let path = path.as_ref();
        let v = self.inner.as_vec_mut();
        if path.is_absolute() {
            v.clear();
        } else if !v.is_empty() && v.last() != Some(&b'/') {
            v.push(b'/');
        }
        v.extend_from_slice(path.bytes());
    }

    pub fn pop(&mut self) -> bool {
        match self.parent().map(|p| p.bytes().len()) {
            Some(len) => {
                self.inner.as_vec_mut().truncate(len);
                true
            }
            None => false,
        }
    }

    pub fn set_file_name<S: AsRef<OsStr>>(&mut self, file_name: S) {
        if self.file_name().is_some() {
            self.pop();
        }
        self.push(Path::new(file_name.as_ref()));
    }

    pub fn set_extension<S: AsRef<OsStr>>(&mut self, extension: S) -> bool {
        let new = extension.as_ref().as_bytes();
        validate_extension(new);
        let stem_end = match self.file_stem() {
            None => return false,
            Some(stem) => stem.as_bytes().as_ptr() as usize + stem.len() - self.bytes().as_ptr() as usize,
        };
        let v = self.inner.as_vec_mut();
        v.truncate(stem_end);
        if !new.is_empty() {
            v.push(b'.');
            v.extend_from_slice(new);
        }
        true
    }

    pub fn add_extension<S: AsRef<OsStr>>(&mut self, extension: S) -> bool {
        let new = extension.as_ref().as_bytes();
        validate_extension(new);
        let name_end = match self.file_name() {
            None => return false,
            Some(name) => name.as_bytes().as_ptr() as usize + name.len() - self.bytes().as_ptr() as usize,
        };
        if !new.is_empty() {
            // truncating after the file name also drops trailing separators
            let v = self.inner.as_vec_mut();
            v.truncate(name_end);
            v.push(b'.');
            v.extend_from_slice(new);
        }
        true
    }

    pub fn as_mut_os_string(&mut self) -> &mut OsString {
        &mut self.inner
    }

    pub fn into_os_string(self) -> OsString {
        self.inner
    }

    pub fn leak<'a>(self) -> &'a mut Path {
        Box::leak(self.into_boxed_path())
    }

    pub fn into_boxed_path(self) -> Box<Path> {
        let raw = Box::into_raw(self.inner.into_boxed_os_str()) as *mut Path;
        unsafe { Box::from_raw(raw) }
    }

    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    pub fn clear(&mut self) {
        self.inner.clear()
    }

    pub fn reserve(&mut self, additional: usize) {
        self.inner.reserve(additional)
    }
}

impl Deref for PathBuf {
    type Target = Path;
    fn deref(&self) -> &Path {
        Path::new(&self.inner)
    }
}

impl core::ops::DerefMut for PathBuf {
    fn deref_mut(&mut self) -> &mut Path {
        unsafe { &mut *(&mut *self.inner as *mut OsStr as *mut Path) }
    }
}

impl Borrow<Path> for PathBuf {
    fn borrow(&self) -> &Path {
        self
    }
}

impl ToOwned for Path {
    type Owned = PathBuf;
    fn to_owned(&self) -> PathBuf {
        self.to_path_buf()
    }
    fn clone_into(&self, target: &mut PathBuf) {
        self.inner.clone_into(&mut target.inner);
    }
}

fn validate_extension(ext: &[u8]) {
    if ext.contains(&b'/') {
        panic!("extension cannot contain path separators: {:?}", OsStr::from_bytes(ext));
    }
}

impl fmt::Debug for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.inner, f)
    }
}

impl fmt::Debug for PathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

// equality and ordering are component-wise, like std
impl PartialEq for Path {
    fn eq(&self, other: &Path) -> bool {
        self.bytes() == other.bytes() || self.components().eq(other.components())
    }
}
impl Eq for Path {}
impl PartialOrd for Path {
    fn partial_cmp(&self, other: &Path) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Path {
    fn cmp(&self, other: &Path) -> core::cmp::Ordering {
        self.components().cmp(other.components())
    }
}
impl Hash for Path {
    fn hash<H: Hasher>(&self, h: &mut H) {
        for c in self.components() {
            c.hash(h);
        }
    }
}

impl PartialEq for PathBuf {
    fn eq(&self, other: &PathBuf) -> bool {
        **self == **other
    }
}
impl Eq for PathBuf {}
impl PartialOrd for PathBuf {
    fn partial_cmp(&self, other: &PathBuf) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for PathBuf {
    fn cmp(&self, other: &PathBuf) -> core::cmp::Ordering {
        (**self).cmp(&**other)
    }
}
impl Hash for PathBuf {
    fn hash<H: Hasher>(&self, h: &mut H) {
        (**self).hash(h)
    }
}

// Comparisons with other path-like and string-like types, as std provides.
// All compare component-wise, as `Path`s.
macro_rules! impl_cmp {
    ($lhs:ty, $rhs:ty) => {
        impl PartialEq<$rhs> for $lhs {
            fn eq(&self, other: &$rhs) -> bool {
                <Path as PartialEq>::eq(as_path(self), as_path(other))
            }
        }
        impl PartialEq<$lhs> for $rhs {
            fn eq(&self, other: &$lhs) -> bool {
                <Path as PartialEq>::eq(as_path(self), as_path(other))
            }
        }
    };
}

fn as_path<P: AsRef<Path> + ?Sized>(p: &P) -> &Path {
    p.as_ref()
}

impl_cmp!(PathBuf, Path);
impl_cmp!(PathBuf, &Path);
impl_cmp!(Cow<'_, Path>, Path);
impl_cmp!(Cow<'_, Path>, &Path);
impl_cmp!(Cow<'_, Path>, PathBuf);

impl_cmp!(PathBuf, OsStr);
impl_cmp!(PathBuf, &OsStr);
impl_cmp!(PathBuf, Cow<'_, OsStr>);
impl_cmp!(PathBuf, OsString);
impl_cmp!(Path, OsStr);
impl_cmp!(Path, &OsStr);
impl_cmp!(Path, Cow<'_, OsStr>);
impl_cmp!(Path, OsString);
impl_cmp!(&Path, OsStr);
impl_cmp!(&Path, Cow<'_, OsStr>);
impl_cmp!(&Path, OsString);
impl_cmp!(Cow<'_, Path>, OsStr);
impl_cmp!(Cow<'_, Path>, &OsStr);
impl_cmp!(Cow<'_, Path>, OsString);

impl_cmp!(Path, str);
impl_cmp!(Path, String);
impl_cmp!(PathBuf, str);
impl_cmp!(PathBuf, String);

macro_rules! impl_as_ref {
    ($($t:ty),*) => {$(
        impl AsRef<Path> for $t {
            fn as_ref(&self) -> &Path {
                Path::new(self)
            }
        }
    )*};
}

impl_as_ref!(str, String, OsStr, OsString, Cow<'_, OsStr>);

impl AsRef<Path> for Path {
    fn as_ref(&self) -> &Path {
        self
    }
}
impl AsRef<Path> for PathBuf {
    fn as_ref(&self) -> &Path {
        self
    }
}
impl AsRef<OsStr> for Path {
    fn as_ref(&self) -> &OsStr {
        &self.inner
    }
}
impl AsRef<OsStr> for PathBuf {
    fn as_ref(&self) -> &OsStr {
        &self.inner
    }
}

impl<T: ?Sized + AsRef<OsStr>> From<&T> for PathBuf {
    fn from(s: &T) -> PathBuf {
        PathBuf { inner: s.as_ref().to_os_string() }
    }
}
impl From<OsString> for PathBuf {
    fn from(s: OsString) -> PathBuf {
        PathBuf { inner: s }
    }
}
impl From<String> for PathBuf {
    fn from(s: String) -> PathBuf {
        PathBuf { inner: OsString::from(s) }
    }
}
impl From<PathBuf> for OsString {
    fn from(p: PathBuf) -> OsString {
        p.inner
    }
}
impl From<Box<Path>> for PathBuf {
    fn from(b: Box<Path>) -> PathBuf {
        b.into_path_buf()
    }
}
impl From<PathBuf> for Box<Path> {
    fn from(p: PathBuf) -> Box<Path> {
        p.into_boxed_path()
    }
}
impl From<&Path> for Box<Path> {
    fn from(p: &Path) -> Box<Path> {
        p.to_path_buf().into_boxed_path()
    }
}
impl From<Cow<'_, Path>> for Box<Path> {
    fn from(p: Cow<'_, Path>) -> Box<Path> {
        p.into_owned().into_boxed_path()
    }
}
impl Clone for Box<Path> {
    fn clone(&self) -> Self {
        self.to_path_buf().into_boxed_path()
    }
}
impl From<&Path> for Arc<Path> {
    fn from(p: &Path) -> Arc<Path> {
        Arc::from(p.to_path_buf())
    }
}
impl From<&Path> for Rc<Path> {
    fn from(p: &Path) -> Rc<Path> {
        Rc::from(p.to_path_buf())
    }
}

impl From<PathBuf> for Arc<Path> {
    fn from(p: PathBuf) -> Arc<Path> {
        let arc: Arc<OsStr> = Arc::from(p.inner.into_boxed_os_str());
        unsafe { Arc::from_raw(Arc::into_raw(arc) as *const Path) }
    }
}
impl From<PathBuf> for Rc<Path> {
    fn from(p: PathBuf) -> Rc<Path> {
        let rc: Rc<OsStr> = Rc::from(p.inner.into_boxed_os_str());
        unsafe { Rc::from_raw(Rc::into_raw(rc) as *const Path) }
    }
}
impl<'a> From<&'a Path> for Cow<'a, Path> {
    fn from(s: &'a Path) -> Cow<'a, Path> {
        Cow::Borrowed(s)
    }
}
impl<'a> From<PathBuf> for Cow<'a, Path> {
    fn from(s: PathBuf) -> Cow<'a, Path> {
        Cow::Owned(s)
    }
}

impl core::str::FromStr for PathBuf {
    type Err = core::convert::Infallible;
    fn from_str(s: &str) -> Result<PathBuf, Self::Err> {
        Ok(PathBuf::from(s))
    }
}

impl<P: AsRef<Path>> FromIterator<P> for PathBuf {
    fn from_iter<I: IntoIterator<Item = P>>(iter: I) -> PathBuf {
        let mut buf = PathBuf::new();
        buf.extend(iter);
        buf
    }
}

impl<P: AsRef<Path>> Extend<P> for PathBuf {
    fn extend<I: IntoIterator<Item = P>>(&mut self, iter: I) {
        iter.into_iter().for_each(move |p| self.push(p.as_ref()));
    }
}

impl<'a> IntoIterator for &'a PathBuf {
    type Item = &'a OsStr;
    type IntoIter = Iter<'a>;
    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

impl<'a> IntoIterator for &'a Path {
    type Item = &'a OsStr;
    type IntoIter = Iter<'a>;
    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

/// Make a path absolute without touching the filesystem (no symlink
/// resolution), following POSIX like std: `.` components are dropped, `..`
/// kept, exactly two leading slashes preserved, and a trailing slash kept.
pub fn absolute<P: AsRef<Path>>(path: P) -> io::Result<PathBuf> {
    let path = path.as_ref();
    let bytes = path.as_os_str().as_bytes();
    if bytes.is_empty() {
        return Err(io::const_error!(io::ErrorKind::InvalidInput, "cannot make an empty path absolute"));
    }
    let mut components = path.strip_prefix(".").unwrap_or(path).components();
    let mut normalized = if path.is_absolute() {
        if bytes.starts_with(b"//") && !bytes.starts_with(b"///") {
            components.next();
            PathBuf::from("//")
        } else {
            PathBuf::new()
        }
    } else {
        crate::env::current_dir()?
    };
    normalized.extend(components);
    if bytes.ends_with(b"/") {
        normalized.push("");
    }
    Ok(normalized)
}

#[cfg(kani)]
mod proofs {
    use super::*;

    const N: usize = 5;

    /// An arbitrary path of up to N bytes over the bytes that matter to the
    /// parser: separators, dots, and one ordinary byte.
    fn any_path(buf: &mut [u8; N]) -> &Path {
        for b in buf.iter_mut() {
            *b = match kani::any::<u8>() % 3 {
                0 => b'/',
                1 => b'.',
                _ => b'a',
            };
        }
        let len: usize = kani::any_where(|&l| l <= N);
        Path::from_bytes(&buf[..len])
    }

    /// Mixing next() and next_back() in any order yields exactly the forward
    /// sequence, and as_path() always stays inside the original path.
    #[kani::proof]
    #[kani::unwind(8)]
    fn components_front_back_agree() {
        let mut buf = [0u8; N];
        let p = any_path(&mut buf);
        let mut fwd: [Option<Component<'_>>; N + 1] = [None; N + 1];
        let mut count = 0;
        for c in p.components() {
            fwd[count] = Some(c);
            count += 1;
        }
        assert!(count <= N);

        let mut it = p.components();
        let (mut lo, mut hi) = (0, count);
        let base = p.as_os_str().as_bytes().as_ptr() as usize;
        loop {
            let rest = it.as_path().as_os_str().as_bytes();
            let start = rest.as_ptr() as usize;
            assert!(rest.is_empty() || (start >= base && start + rest.len() <= base + p.as_os_str().len()));
            let item = if kani::any() {
                let c = it.next();
                if c.is_some() {
                    assert_eq!(c, fwd[lo]);
                    lo += 1;
                }
                c
            } else {
                let c = it.next_back();
                if c.is_some() {
                    hi -= 1;
                    assert_eq!(c, fwd[hi]);
                }
                c
            };
            if item.is_none() {
                assert_eq!(lo, hi);
                break;
            }
        }
    }

    /// file_name() is the last component when that component is Normal.
    #[kani::proof]
    #[kani::unwind(8)]
    fn file_name_is_last_normal_component() {
        let mut buf = [0u8; N];
        let p = any_path(&mut buf);
        match p.components().next_back() {
            Some(Component::Normal(name)) => assert_eq!(p.file_name(), Some(name)),
            _ => assert_eq!(p.file_name(), None),
        }
    }
}
