use alloc_crate::vec::Vec;

use crate::io;

const STACK_BUF: usize = 512;

/// Call `f` with `bytes` as a NUL-terminated C string, avoiding an allocation
/// for short paths. Interior NULs are an `InvalidInput` error.
pub fn with_cstr<T>(bytes: &[u8], f: impl FnOnce(*const u8) -> io::Result<T>) -> io::Result<T> {
    if bytes.contains(&0) {
        return Err(io::const_error!(io::ErrorKind::InvalidInput, "path contains an interior nul byte"));
    }
    if bytes.len() < STACK_BUF {
        let mut buf = [0u8; STACK_BUF];
        buf[..bytes.len()].copy_from_slice(bytes);
        f(buf.as_ptr())
    } else {
        let mut v = Vec::with_capacity(bytes.len() + 1);
        v.extend_from_slice(bytes);
        v.push(0);
        f(v.as_ptr())
    }
}

pub fn with_path<T>(p: &crate::path::Path, f: impl FnOnce(*const u8) -> io::Result<T>) -> io::Result<T> {
    with_cstr(p.as_os_str().as_bytes(), f)
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// The callback sees a NUL-terminated copy of the bytes, unless they
    /// contain an interior NUL, which is an `InvalidInput` error.
    #[kani::proof]
    #[kani::unwind(6)]
    fn with_cstr_terminates_or_rejects() {
        let bytes: [u8; 4] = kani::any();
        let len: usize = kani::any_where(|&l| l <= 4);
        let b = &bytes[..len];
        let r = with_cstr(b, |p| {
            for (i, &c) in b.iter().enumerate() {
                assert_eq!(unsafe { *p.add(i) }, c);
            }
            assert_eq!(unsafe { *p.add(b.len()) }, 0);
            Ok(())
        });
        match r {
            Ok(()) => assert!(!b.contains(&0)),
            Err(e) => {
                assert!(b.contains(&0));
                assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
            }
        }
    }
}
