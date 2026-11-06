//! Platform-specific string types. On Linux an OS string is arbitrary bytes.

use alloc_crate::borrow::{Cow, ToOwned};
use alloc_crate::boxed::Box;
use alloc_crate::string::String;
use alloc_crate::vec::Vec;
use core::borrow::Borrow;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::ops::Deref;

pub use alloc_crate::ffi::{CString, FromVecWithNulError, IntoStringError, NulError};
pub use core::ffi::*;

#[repr(transparent)]
pub struct OsStr {
    inner: [u8],
}

#[derive(Clone, Default)]
pub struct OsString {
    inner: Vec<u8>,
}

impl OsStr {
    pub fn new<S: AsRef<OsStr> + ?Sized>(s: &S) -> &OsStr {
        s.as_ref()
    }

    pub(crate) fn from_bytes(b: &[u8]) -> &OsStr {
        // SAFETY: OsStr is a transparent wrapper around [u8]
        unsafe { &*(b as *const [u8] as *const OsStr) }
    }

    pub(crate) fn from_bytes_mut(b: &mut [u8]) -> &mut OsStr {
        unsafe { &mut *(b as *mut [u8] as *mut OsStr) }
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.inner
    }

    pub fn as_encoded_bytes(&self) -> &[u8] {
        &self.inner
    }

    /// # Safety
    /// On this platform any bytes are valid.
    pub unsafe fn from_encoded_bytes_unchecked(bytes: &[u8]) -> &OsStr {
        Self::from_bytes(bytes)
    }

    pub fn to_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.inner).ok()
    }

    pub fn to_string_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.inner)
    }

    pub fn to_os_string(&self) -> OsString {
        OsString { inner: self.inner.to_vec() }
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn into_os_string(self: Box<OsStr>) -> OsString {
        let raw = Box::into_raw(self) as *mut [u8];
        OsString { inner: unsafe { Box::from_raw(raw) }.into_vec() }
    }

    pub fn make_ascii_lowercase(&mut self) {
        self.inner.make_ascii_lowercase()
    }

    pub fn make_ascii_uppercase(&mut self) {
        self.inner.make_ascii_uppercase()
    }

    pub fn to_ascii_lowercase(&self) -> OsString {
        OsString { inner: self.inner.to_ascii_lowercase() }
    }

    pub fn to_ascii_uppercase(&self) -> OsString {
        OsString { inner: self.inner.to_ascii_uppercase() }
    }

    pub fn is_ascii(&self) -> bool {
        self.inner.is_ascii()
    }

    pub fn eq_ignore_ascii_case<S: AsRef<OsStr>>(&self, other: S) -> bool {
        self.inner.eq_ignore_ascii_case(&other.as_ref().inner)
    }

    pub fn display(&self) -> Display<'_> {
        Display { os_str: self }
    }
}

pub struct Display<'a> {
    os_str: &'a OsStr,
}

impl fmt::Display for Display<'_> {
    // Mirrors core's ByteStr Display, which std uses: invalid sequences print
    // as U+FFFD and count as one character for width and precision.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn emit(bytes: &[u8], f: &mut fmt::Formatter<'_>) -> fmt::Result {
            for chunk in bytes.utf8_chunks() {
                f.write_str(chunk.valid())?;
                if !chunk.invalid().is_empty() {
                    f.write_str("\u{FFFD}")?;
                }
            }
            Ok(())
        }

        let bytes = &self.os_str.inner;
        let requested = f.width().unwrap_or(0);
        if requested == 0 && f.precision().is_none() {
            return emit(bytes, f);
        }
        // (bytes to print, their width in chars)
        let (shown, width) = match f.precision() {
            Some(precision) => {
                let mut left = precision;
                let mut end = 0;
                let mut width = 0;
                'outer: for chunk in bytes.utf8_chunks() {
                    for c in chunk.valid().chars() {
                        if left == 0 {
                            break 'outer;
                        }
                        end += c.len_utf8();
                        width += 1;
                        left -= 1;
                    }
                    if !chunk.invalid().is_empty() {
                        if left == 0 {
                            break;
                        }
                        end += chunk.invalid().len();
                        width += 1;
                        left -= 1;
                    }
                }
                (&bytes[..end], width)
            }
            None => {
                let width = bytes
                    .utf8_chunks()
                    .map(|c| c.valid().chars().count() + usize::from(!c.invalid().is_empty()))
                    .sum();
                (&bytes[..], width)
            }
        };
        let pad = requested.saturating_sub(width);
        let (pre, post) = match f.align() {
            None | Some(fmt::Alignment::Left) => (0, pad),
            Some(fmt::Alignment::Right) => (pad, 0),
            Some(fmt::Alignment::Center) => (pad / 2, pad - pad / 2),
        };
        let fill = f.fill();
        for _ in 0..pre {
            fmt::Write::write_char(f, fill)?;
        }
        emit(shown, f)?;
        for _ in 0..post {
            fmt::Write::write_char(f, fill)?;
        }
        Ok(())
    }
}

impl fmt::Debug for Display<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.os_str, f)
    }
}

impl OsString {
    pub fn new() -> OsString {
        OsString { inner: Vec::new() }
    }

    pub fn with_capacity(capacity: usize) -> OsString {
        OsString { inner: Vec::with_capacity(capacity) }
    }

    pub(crate) fn from_vec(v: Vec<u8>) -> OsString {
        OsString { inner: v }
    }

    pub(crate) fn into_vec(self) -> Vec<u8> {
        self.inner
    }

    pub fn as_os_str(&self) -> &OsStr {
        OsStr::from_bytes(&self.inner)
    }

    pub fn into_encoded_bytes(self) -> Vec<u8> {
        self.inner
    }

    /// # Safety
    /// On this platform any bytes are valid.
    pub unsafe fn from_encoded_bytes_unchecked(bytes: Vec<u8>) -> OsString {
        OsString { inner: bytes }
    }

    pub fn into_string(self) -> Result<String, OsString> {
        String::from_utf8(self.inner).map_err(|e| OsString { inner: e.into_bytes() })
    }

    pub fn push<T: AsRef<OsStr>>(&mut self, s: T) {
        self.inner.extend_from_slice(&s.as_ref().inner)
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

    pub fn shrink_to_fit(&mut self) {
        self.inner.shrink_to_fit()
    }

    pub fn into_boxed_os_str(self) -> Box<OsStr> {
        let raw = Box::into_raw(self.inner.into_boxed_slice()) as *mut OsStr;
        unsafe { Box::from_raw(raw) }
    }

    pub(crate) fn as_vec_mut(&mut self) -> &mut Vec<u8> {
        &mut self.inner
    }
}

impl Deref for OsString {
    type Target = OsStr;
    fn deref(&self) -> &OsStr {
        self.as_os_str()
    }
}

impl core::ops::DerefMut for OsString {
    fn deref_mut(&mut self) -> &mut OsStr {
        OsStr::from_bytes_mut(&mut self.inner)
    }
}

impl Borrow<OsStr> for OsString {
    fn borrow(&self) -> &OsStr {
        self
    }
}

impl ToOwned for OsStr {
    type Owned = OsString;
    fn to_owned(&self) -> OsString {
        self.to_os_string()
    }
    fn clone_into(&self, target: &mut OsString) {
        target.inner.clear();
        target.inner.extend_from_slice(&self.inner);
    }
}

impl fmt::Debug for OsStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // valid UTF-8 prints exactly like a str
        if let Ok(s) = core::str::from_utf8(&self.inner) {
            return fmt::Debug::fmt(s, f);
        }
        f.write_str("\"")?;
        for chunk in self.inner.utf8_chunks() {
            for c in chunk.valid().chars() {
                // as in str's Debug, a single quote needs no escape
                if c == '\'' {
                    fmt::Write::write_char(f, c)?;
                } else {
                    for e in c.escape_debug() {
                        fmt::Write::write_char(f, e)?;
                    }
                }
            }
            for b in chunk.invalid() {
                write!(f, "\\x{b:02X}")?;
            }
        }
        f.write_str("\"")
    }
}

impl fmt::Debug for OsString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_os_str(), f)
    }
}

impl PartialEq for OsStr {
    fn eq(&self, other: &OsStr) -> bool {
        self.inner == other.inner
    }
}
impl Eq for OsStr {}

impl PartialOrd for OsStr {
    fn partial_cmp(&self, other: &OsStr) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for OsStr {
    fn cmp(&self, other: &OsStr) -> core::cmp::Ordering {
        self.inner.cmp(&other.inner)
    }
}
impl Hash for OsStr {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inner.hash(state)
    }
}

impl PartialEq for OsString {
    fn eq(&self, other: &OsString) -> bool {
        self.inner == other.inner
    }
}
impl Eq for OsString {}
impl PartialOrd for OsString {
    fn partial_cmp(&self, other: &OsString) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for OsString {
    fn cmp(&self, other: &OsString) -> core::cmp::Ordering {
        self.inner.cmp(&other.inner)
    }
}
impl Hash for OsString {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_os_str().hash(state)
    }
}

impl PartialEq<str> for OsStr {
    fn eq(&self, other: &str) -> bool {
        &self.inner == other.as_bytes()
    }
}
impl PartialEq<OsStr> for str {
    fn eq(&self, other: &OsStr) -> bool {
        self.as_bytes() == &other.inner
    }
}
impl PartialEq<str> for OsString {
    fn eq(&self, other: &str) -> bool {
        self.inner == other.as_bytes()
    }
}
impl PartialEq<&str> for OsString {
    fn eq(&self, other: &&str) -> bool {
        self.inner == other.as_bytes()
    }
}
impl PartialEq<OsString> for str {
    fn eq(&self, other: &OsString) -> bool {
        self.as_bytes() == other.inner
    }
}
impl PartialEq<OsString> for &str {
    fn eq(&self, other: &OsString) -> bool {
        self.as_bytes() == other.inner
    }
}
impl PartialEq<OsStr> for OsString {
    fn eq(&self, other: &OsStr) -> bool {
        self.inner == other.inner
    }
}
impl PartialEq<OsString> for OsStr {
    fn eq(&self, other: &OsString) -> bool {
        self.inner == other.inner
    }
}

impl AsRef<OsStr> for OsStr {
    fn as_ref(&self) -> &OsStr {
        self
    }
}
impl AsRef<OsStr> for OsString {
    fn as_ref(&self) -> &OsStr {
        self
    }
}
impl AsRef<OsStr> for str {
    fn as_ref(&self) -> &OsStr {
        OsStr::from_bytes(self.as_bytes())
    }
}
impl AsRef<OsStr> for String {
    fn as_ref(&self) -> &OsStr {
        OsStr::from_bytes(self.as_bytes())
    }
}

impl From<String> for OsString {
    fn from(s: String) -> OsString {
        OsString { inner: s.into_bytes() }
    }
}
impl<T: ?Sized + AsRef<OsStr>> From<&T> for OsString {
    fn from(s: &T) -> OsString {
        s.as_ref().to_os_string()
    }
}
impl From<Box<OsStr>> for OsString {
    fn from(b: Box<OsStr>) -> OsString {
        b.into_os_string()
    }
}
impl From<&OsStr> for Box<OsStr> {
    fn from(s: &OsStr) -> Box<OsStr> {
        s.to_os_string().into_boxed_os_str()
    }
}
impl Clone for Box<OsStr> {
    fn clone(&self) -> Self {
        self.to_os_string().into_boxed_os_str()
    }
}
impl From<&OsStr> for alloc_crate::sync::Arc<OsStr> {
    fn from(s: &OsStr) -> Self {
        let arc: alloc_crate::sync::Arc<[u8]> = alloc_crate::sync::Arc::from(s.as_bytes());
        unsafe { alloc_crate::sync::Arc::from_raw(alloc_crate::sync::Arc::into_raw(arc) as *const OsStr) }
    }
}
impl From<&OsStr> for alloc_crate::rc::Rc<OsStr> {
    fn from(s: &OsStr) -> Self {
        let rc: alloc_crate::rc::Rc<[u8]> = alloc_crate::rc::Rc::from(s.as_bytes());
        unsafe { alloc_crate::rc::Rc::from_raw(alloc_crate::rc::Rc::into_raw(rc) as *const OsStr) }
    }
}
impl From<OsString> for alloc_crate::sync::Arc<OsStr> {
    fn from(s: OsString) -> Self {
        alloc_crate::sync::Arc::from(s.as_os_str())
    }
}
impl From<OsString> for alloc_crate::rc::Rc<OsStr> {
    fn from(s: OsString) -> Self {
        alloc_crate::rc::Rc::from(s.as_os_str())
    }
}

impl From<OsString> for Box<OsStr> {
    fn from(s: OsString) -> Box<OsStr> {
        s.into_boxed_os_str()
    }
}
impl<'a> From<&'a OsStr> for Cow<'a, OsStr> {
    fn from(s: &'a OsStr) -> Cow<'a, OsStr> {
        Cow::Borrowed(s)
    }
}
impl<'a> From<OsString> for Cow<'a, OsStr> {
    fn from(s: OsString) -> Cow<'a, OsStr> {
        Cow::Owned(s)
    }
}

impl core::str::FromStr for OsString {
    type Err = core::convert::Infallible;
    fn from_str(s: &str) -> Result<OsString, Self::Err> {
        Ok(OsString::from(s))
    }
}

impl<T: AsRef<OsStr>> Extend<T> for OsString {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for s in iter {
            self.push(s);
        }
    }
}

impl fmt::Write for OsString {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push(s);
        Ok(())
    }
}
