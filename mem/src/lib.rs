//! The C memory routines LLVM emits calls to (`memcpy`, `memset`, ...).
//!
//! Normally libc provides these. `#![no_builtins]` stops LLVM from recognising
//! the loops below as memcpy/memset idioms and turning them back into calls to
//! themselves.
//!
//! Under Kani the symbols are not exported (the verifier has its own C
//! runtime); `proofs` checks each function against a byte-wise model.
#![no_std]
#![no_builtins]

use core::ffi::{c_char, c_int, c_void};
use core::mem::size_of;

const WORD: usize = size_of::<usize>();

#[inline(always)]
fn word_aligned(a: usize, b: usize) -> bool {
    (a | b).is_multiple_of(WORD)
}

/// # Safety
/// Standard C `memcpy` contract.
#[cfg_attr(not(kani), unsafe(no_mangle))]
pub unsafe extern "C" fn memcpy(dest: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    unsafe { copy_forward(dest as *mut u8, src as *const u8, n) };
    dest
}

/// # Safety
/// Standard C `memmove` contract.
#[cfg_attr(not(kani), unsafe(no_mangle))]
pub unsafe extern "C" fn memmove(dest: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let (dest, src) = (dest as *mut u8, src as *const u8);
    if (dest as usize).wrapping_sub(src as usize) >= n {
        // no overlap, or dest is before src: forward copy is safe
        unsafe { copy_forward(dest, src, n) };
    } else {
        unsafe { copy_backward(dest, src, n) };
    }
    dest as *mut c_void
}

#[inline(always)]
unsafe fn copy_forward(dest: *mut u8, src: *const u8, n: usize) {
    let mut i = 0;
    unsafe {
        if word_aligned(dest as usize, src as usize) {
            while i + WORD <= n {
                *(dest.add(i) as *mut usize) = *(src.add(i) as *const usize);
                i += WORD;
            }
        }
        while i < n {
            *dest.add(i) = *src.add(i);
            i += 1;
        }
    }
}

#[inline(always)]
unsafe fn copy_backward(dest: *mut u8, src: *const u8, n: usize) {
    let mut i = n;
    unsafe {
        if word_aligned(dest as usize + n, src as usize + n) {
            while i >= WORD {
                i -= WORD;
                *(dest.add(i) as *mut usize) = *(src.add(i) as *const usize);
            }
        }
        while i > 0 {
            i -= 1;
            *dest.add(i) = *src.add(i);
        }
    }
}

/// # Safety
/// Standard C `memset` contract.
#[cfg_attr(not(kani), unsafe(no_mangle))]
pub unsafe extern "C" fn memset(dest: *mut c_void, c: c_int, n: usize) -> *mut c_void {
    let s = dest as *mut u8;
    let b = c as u8;
    let mut i = 0;
    unsafe {
        if (s as usize).is_multiple_of(WORD) {
            // (not `[b; WORD]`: rustc lowers array repeats to a memset call)
            let w = (b as usize) * (usize::MAX / 0xff);
            while i + WORD <= n {
                *(s.add(i) as *mut usize) = w;
                i += WORD;
            }
        }
        while i < n {
            *s.add(i) = b;
            i += 1;
        }
    }
    dest
}

/// # Safety
/// Standard C `memcmp` contract.
#[cfg_attr(not(kani), unsafe(no_mangle))]
pub unsafe extern "C" fn memcmp(a: *const c_void, b: *const c_void, n: usize) -> c_int {
    let (a, b) = (a as *const u8, b as *const u8);
    let mut i = 0;
    while i < n {
        let (x, y) = unsafe { (*a.add(i), *b.add(i)) };
        if x != y {
            return x as i32 - y as i32;
        }
        i += 1;
    }
    0
}

/// # Safety
/// Standard C `bcmp` contract.
#[cfg_attr(not(kani), unsafe(no_mangle))]
pub unsafe extern "C" fn bcmp(a: *const c_void, b: *const c_void, n: usize) -> c_int {
    unsafe { memcmp(a, b, n) }
}

/// # Safety
/// `s` must point to a NUL-terminated string.
#[cfg_attr(not(kani), unsafe(no_mangle))]
pub unsafe extern "C" fn strlen(s: *const c_char) -> usize {
    let mut n = 0;
    while unsafe { *s.add(n) } != 0 {
        n += 1;
    }
    n
}

#[cfg(kani)]
mod proofs {
    use super::*;

    // Buffers longer than three words, with arbitrary offsets, cover the
    // unaligned prologue/epilogue and the word loop of every function.
    const N: usize = 24;

    fn region() -> (usize, usize) {
        let off: usize = kani::any_where(|&o| o <= N);
        let len: usize = kani::any_where(|&l| l <= N - off);
        (off, len)
    }

    #[kani::proof]
    #[kani::unwind(26)]
    fn memcpy_copies_exactly() {
        let src: [u8; N] = kani::any();
        let mut dst: [u8; N] = kani::any();
        let orig = dst;
        let (s_off, len) = region();
        let d_off: usize = kani::any_where(|&o| o <= N - len);
        let r = unsafe { memcpy(dst.as_mut_ptr().add(d_off).cast(), src.as_ptr().add(s_off).cast(), len) };
        assert_eq!(r as usize, dst.as_ptr() as usize + d_off);
        let i: usize = kani::any_where(|&i| i < N);
        if i >= d_off && i < d_off + len {
            assert_eq!(dst[i], src[s_off + i - d_off]);
        } else {
            assert_eq!(dst[i], orig[i]);
        }
    }

    #[kani::proof]
    #[kani::unwind(26)]
    fn memmove_handles_overlap() {
        let mut buf: [u8; N] = kani::any();
        let orig = buf;
        let (s_off, len) = region();
        let d_off: usize = kani::any_where(|&o| o <= N - len);
        let p = buf.as_mut_ptr();
        unsafe { memmove(p.add(d_off).cast(), p.add(s_off).cast(), len) };
        let i: usize = kani::any_where(|&i| i < N);
        if i >= d_off && i < d_off + len {
            assert_eq!(buf[i], orig[s_off + i - d_off]);
        } else {
            assert_eq!(buf[i], orig[i]);
        }
    }

    #[kani::proof]
    #[kani::unwind(26)]
    fn memset_fills_exactly() {
        let mut buf: [u8; N] = kani::any();
        let orig = buf;
        let (off, len) = region();
        let c: i32 = kani::any();
        unsafe { memset(buf.as_mut_ptr().add(off).cast(), c, len) };
        let i: usize = kani::any_where(|&i| i < N);
        if i >= off && i < off + len {
            assert_eq!(buf[i], c as u8);
        } else {
            assert_eq!(buf[i], orig[i]);
        }
    }

    #[kani::proof]
    #[kani::unwind(10)]
    fn memcmp_orders_like_slices() {
        let a: [u8; 8] = kani::any();
        let b: [u8; 8] = kani::any();
        let len: usize = kani::any_where(|&l| l <= 8);
        let r = unsafe { memcmp(a.as_ptr().cast(), b.as_ptr().cast(), len) };
        assert_eq!(r.cmp(&0), a[..len].cmp(&b[..len]));
        let r = unsafe { bcmp(a.as_ptr().cast(), b.as_ptr().cast(), len) };
        assert_eq!(r == 0, a[..len] == b[..len]);
    }

    #[kani::proof]
    #[kani::unwind(10)]
    fn strlen_finds_first_nul() {
        let mut s: [u8; 8] = kani::any();
        let nul: usize = kani::any_where(|&i| i < 8);
        s[nul] = 0;
        let n = unsafe { strlen(s.as_ptr().cast()) };
        assert!(n <= nul && s[n] == 0);
        assert!(s[..n].iter().all(|&b| b != 0));
    }
}
