//! Inspection and manipulation of the process's environment.

use alloc_crate::string::String;
use alloc_crate::vec::Vec;
use core::error::Error;
use core::fmt;

use crate::ffi::{CStr, OsStr, OsString};
use crate::io;
use crate::path::{Path, PathBuf};
use crate::sync::RwLock;
use crate::sys::{fd::cvt, linux, path::with_path};

pub mod consts {
    pub const ARCH: &str = if cfg!(target_arch = "x86_64") { "x86_64" } else { "aarch64" };
    pub const FAMILY: &str = "unix";
    pub const OS: &str = "linux";
    pub const DLL_PREFIX: &str = "lib";
    pub const DLL_SUFFIX: &str = ".so";
    pub const DLL_EXTENSION: &str = "so";
    pub const EXE_SUFFIX: &str = "";
    pub const EXE_EXTENSION: &str = "";
}

// ---- arguments ----

pub struct ArgsOs {
    inner: alloc_crate::vec::IntoIter<OsString>,
}

pub struct Args {
    inner: ArgsOs,
}

pub fn args_os() -> ArgsOs {
    let v: Vec<OsString> = crate::rt::argv()
        .iter()
        .map(|&p| OsString::from_vec(unsafe { CStr::from_ptr(p as *const _) }.to_bytes().to_vec()))
        .collect();
    ArgsOs { inner: v.into_iter() }
}

pub fn args() -> Args {
    Args { inner: args_os() }
}

impl Iterator for ArgsOs {
    type Item = OsString;
    fn next(&mut self) -> Option<OsString> {
        self.inner.next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl DoubleEndedIterator for ArgsOs {
    fn next_back(&mut self) -> Option<OsString> {
        self.inner.next_back()
    }
}

impl ExactSizeIterator for ArgsOs {}

impl Iterator for Args {
    type Item = String;
    fn next(&mut self) -> Option<String> {
        self.inner.next().map(|s| s.into_string().expect("argument is not valid Unicode"))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl DoubleEndedIterator for Args {
    fn next_back(&mut self) -> Option<String> {
        self.inner.next_back().map(|s| s.into_string().expect("argument is not valid Unicode"))
    }
}

impl ExactSizeIterator for Args {}

impl fmt::Debug for Args {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Args").field("inner", &self.inner.inner.as_slice()).finish()
    }
}

impl fmt::Debug for ArgsOs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArgsOs").field("inner", &self.inner.as_slice()).finish()
    }
}

// ---- environment variables ----

/// The environment, copied out of the initial stack on first use so that
/// `set_var`/`remove_var` can modify it.
static ENV: RwLock<Option<Vec<(OsString, OsString)>>> = RwLock::new(None);

fn with_env<R>(f: impl FnOnce(&Vec<(OsString, OsString)>) -> R) -> R {
    {
        let env = ENV.read().unwrap();
        if let Some(e) = &*env {
            return f(e);
        }
    }
    let mut env = ENV.write().unwrap();
    f(env.get_or_insert_with(initial_env))
}

fn with_env_mut<R>(f: impl FnOnce(&mut Vec<(OsString, OsString)>) -> R) -> R {
    let mut env = ENV.write().unwrap();
    f(env.get_or_insert_with(initial_env))
}

fn initial_env() -> Vec<(OsString, OsString)> {
    let mut out = Vec::new();
    let mut p = crate::rt::envp();
    if p.is_null() {
        return out;
    }
    unsafe {
        while !(*p).is_null() {
            let entry = CStr::from_ptr(*p as *const _).to_bytes();
            if let Some((k, v)) = split_entry(entry) {
                out.push((OsString::from_vec(k.to_vec()), OsString::from_vec(v.to_vec())));
            }
            p = p.add(1);
        }
    }
    out
}

/// Split a `KEY=VALUE` environment entry. A leading '=' is part of the name
/// (matches std's parsing); entries without a separator are ignored.
fn split_entry(entry: &[u8]) -> Option<(&[u8], &[u8])> {
    let i = entry.iter().skip(1).position(|&b| b == b'=')? + 1;
    Some((&entry[..i], &entry[i + 1..]))
}

/// The environment as `KEY=VALUE` C strings, for `execve`.
pub(crate) fn env_snapshot() -> Vec<(OsString, OsString)> {
    with_env(|e| e.clone())
}

pub struct VarsOs {
    inner: alloc_crate::vec::IntoIter<(OsString, OsString)>,
}

pub struct Vars {
    inner: VarsOs,
}

pub fn vars_os() -> VarsOs {
    VarsOs { inner: env_snapshot().into_iter() }
}

pub fn vars() -> Vars {
    Vars { inner: vars_os() }
}

impl Iterator for VarsOs {
    type Item = (OsString, OsString);
    fn next(&mut self) -> Option<(OsString, OsString)> {
        self.inner.next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl Iterator for Vars {
    type Item = (String, String);
    fn next(&mut self) -> Option<(String, String)> {
        self.inner.next().map(|(k, v)| (k.into_string().unwrap(), v.into_string().unwrap()))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl fmt::Debug for Vars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Vars").field("inner", &self.inner.inner.as_slice()).finish()
    }
}

impl fmt::Debug for VarsOs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VarsOs").field("inner", &self.inner.as_slice()).finish()
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum VarError {
    NotPresent,
    NotUnicode(OsString),
}

impl fmt::Display for VarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VarError::NotPresent => f.write_str("environment variable not found"),
            VarError::NotUnicode(s) => write!(f, "environment variable was not valid unicode: {s:?}"),
        }
    }
}

impl Error for VarError {}

pub fn var_os<K: AsRef<OsStr>>(key: K) -> Option<OsString> {
    let key = key.as_ref();
    with_env(|e| e.iter().find(|(k, _)| **k == *key).map(|(_, v)| v.clone()))
}

pub fn var<K: AsRef<OsStr>>(key: K) -> Result<String, VarError> {
    match var_os(key) {
        Some(s) => s.into_string().map_err(VarError::NotUnicode),
        None => Err(VarError::NotPresent),
    }
}

/// # Safety
/// Kept `unsafe` for parity with std; this runtime's environment is
/// lock-protected, so concurrent use is in fact safe.
pub unsafe fn set_var<K: AsRef<OsStr>, V: AsRef<OsStr>>(key: K, value: V) {
    let (key, value) = (key.as_ref(), value.as_ref());
    assert!(
        !key.is_empty() && !key.as_bytes().contains(&b'=') && !key.as_bytes().contains(&0) && !value.as_bytes().contains(&0),
        "failed to set environment variable `{key:?}` to `{value:?}`"
    );
    with_env_mut(|e| match e.iter_mut().find(|(k, _)| **k == *key) {
        Some(entry) => entry.1 = value.to_os_string(),
        None => e.push((key.to_os_string(), value.to_os_string())),
    })
}

/// # Safety
/// See [`set_var`].
pub unsafe fn remove_var<K: AsRef<OsStr>>(key: K) {
    let key = key.as_ref();
    with_env_mut(|e| e.retain(|(k, _)| **k != *key))
}

// ---- paths ----

pub fn current_dir() -> io::Result<PathBuf> {
    let mut buf: Vec<u8> = Vec::with_capacity(512);
    loop {
        match unsafe { linux::getcwd(buf.as_mut_ptr(), buf.capacity()) } {
            Ok(n) => {
                // n includes the trailing NUL
                unsafe { buf.set_len(n.saturating_sub(1)) };
                return Ok(PathBuf::from(OsString::from_vec(buf)));
            }
            Err(linux::ERANGE) => buf.reserve(buf.capacity()),
            Err(e) => return Err(io::Error::from_raw_os_error(e)),
        }
    }
}

pub fn set_current_dir<P: AsRef<Path>>(path: P) -> io::Result<()> {
    with_path(path.as_ref(), |p| cvt(unsafe { linux::chdir(p) })).map(drop)
}

pub fn temp_dir() -> PathBuf {
    var_os("TMPDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}

pub fn home_dir() -> Option<PathBuf> {
    var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from)
}

pub fn current_exe() -> io::Result<PathBuf> {
    crate::fs::read_link("/proc/self/exe")
}

pub struct SplitPaths<'a> {
    inner: core::slice::Split<'a, u8, fn(&u8) -> bool>,
}

pub fn split_paths<T: AsRef<OsStr> + ?Sized>(unparsed: &T) -> SplitPaths<'_> {
    fn is_colon(b: &u8) -> bool {
        *b == b':'
    }
    SplitPaths { inner: unparsed.as_ref().as_bytes().split(is_colon as fn(&u8) -> bool) }
}

impl Iterator for SplitPaths<'_> {
    type Item = PathBuf;
    fn next(&mut self) -> Option<PathBuf> {
        self.inner.next().map(|b| PathBuf::from(OsString::from_vec(b.to_vec())))
    }
}

impl fmt::Debug for SplitPaths<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SplitPaths").finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct JoinPathsError;

impl fmt::Display for JoinPathsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("path segment contains separator `:`")
    }
}

impl Error for JoinPathsError {}

pub fn join_paths<I, T>(paths: I) -> Result<OsString, JoinPathsError>
where
    I: IntoIterator<Item = T>,
    T: AsRef<OsStr>,
{
    let mut out = Vec::new();
    for (i, p) in paths.into_iter().enumerate() {
        let b = p.as_ref().as_bytes();
        if b.contains(&b':') {
            return Err(JoinPathsError);
        }
        if i > 0 {
            out.push(b':');
        }
        out.extend_from_slice(b);
    }
    Ok(OsString::from_vec(out))
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// Parsing an environment entry is lossless: KEY and VALUE around the
    /// first '=' after the first byte, or nothing when there is none.
    #[kani::proof]
    #[kani::unwind(7)]
    fn split_entry_is_lossless() {
        let buf: [u8; 5] = kani::any();
        let len: usize = kani::any_where(|&l| l <= 5);
        let e = &buf[..len];
        match split_entry(e) {
            Some((k, v)) => {
                assert!(!k.is_empty());
                assert_eq!(k.len() + 1 + v.len(), e.len());
                assert_eq!(e[k.len()], b'=');
                assert!(!k[1..].contains(&b'='));
            }
            None => assert!(e.len() < 2 || !e[1..].contains(&b'=')),
        }
    }

    /// join_paths(split_paths(x)) == x.
    #[kani::proof]
    #[kani::unwind(6)]
    fn split_join_paths_roundtrip() {
        let mut buf = [0u8; 3];
        for b in buf.iter_mut() {
            *b = if kani::any() { b':' } else { b'a' };
        }
        let len: usize = kani::any_where(|&l| l <= 3);
        let x = OsStr::from_bytes(&buf[..len]);
        let joined = join_paths(split_paths(x)).unwrap();
        assert!(joined.as_os_str() == x);
    }
}
