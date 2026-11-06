//! # rust_std
//!
//! A `std`-compatible runtime for Linux on x86_64 and aarch64 that talks to
//! the kernel with raw syscalls: no glibc, no musl, no libc at all. It builds
//! on stable Rust as an ordinary `#![no_std]` crate.
//!
//! ## Using it
//!
//! ```ignore
//! #![no_std]
//! #![no_main]
//! extern crate rust_std as std;
//! use std::prelude::*;
//!
//! std::main!(main);
//!
//! fn main() -> std::io::Result<()> {
//!     let mut names: Vec<String> = std::env::args().collect();
//!     names.sort();
//!     println!("{names:?}");
//!     Ok(())
//! }
//! ```
//!
//! The binary must be built with `panic = "abort"` and linked statically
//! without crt files or libc:
//!
//! * x86_64 (via `cc`): `-C link-arg=-nostartfiles -C link-arg=-nostdlib -C link-arg=-static -C link-arg=-no-pie`
//! * aarch64 (via `rust-lld`): `-C linker=rust-lld -C linker-flavor=ld.lld -C link-arg=-static -C link-arg=--no-pie`
//!
//! (The runtime does not relocate itself, so the executable must not be PIE.)
//!
//! ## Differences from std
//!
//! * Panics always abort (there is no unwinder), so locks are never poisoned,
//!   `JoinHandle::join` never returns `Err`, and `catch_unwind` does not exist.
//! * `ToSocketAddrs` resolves IP literals and `localhost` only (no DNS).
//! * Float math methods that std implements via libm (`sqrt`, `sin`, ...) are
//!   unavailable unless `core` provides them.
//! * `HashMap`/`HashSet` deref to hashbrown's types, so method names match but
//!   the iterator/entry types are hashbrown's.

#![no_std]
#![allow(clippy::new_without_default)]

#[doc(hidden)]
pub extern crate alloc as __alloc;
// renamed so that `std::alloc` can be this crate's own module, as in std
extern crate alloc as alloc_crate;
// provides memcpy & co., which libc would otherwise supply
extern crate rust_std_mem;

// ---- re-exports of core and alloc, laid out like std ----

pub use alloc_crate::{borrow, boxed, fmt, format, rc, slice, str, string, vec};
pub use core::{
    any, arch, array, ascii, cell, char, clone, cmp, convert, default, error, f32, f64, future, hint, iter,
    marker, mem, num, ops, option, panic, pin, primitive, ptr, result, task,
};

pub use core::{
    assert, assert_eq, assert_ne, cfg, column, compile_error, concat, debug_assert, debug_assert_eq,
    debug_assert_ne, env, file, format_args, include, include_bytes, include_str, line, matches, module_path,
    option_env, stringify, todo, unimplemented, unreachable, write, writeln,
};

pub mod alloc;
pub mod collections;
pub mod env;
pub mod ffi;
pub mod fs;
pub mod io;
pub mod net;
pub mod os;
pub mod path;
pub mod process;
#[doc(hidden)]
pub mod rt;
pub mod sync;
mod sys;
pub mod thread;
pub mod time;

pub mod prelude;

pub mod hash {
    pub use crate::collections::hash_map::{DefaultHasher, RandomState};
    pub use core::hash::*;
}

// Programs that disable the `global-allocator` feature can install
// `std::alloc::System` (or any allocator) themselves.
// (not under Kani, which verifies against its own std and allocator)
#[cfg(all(feature = "global-allocator", not(kani)))]
#[global_allocator]
static GLOBAL: alloc::System = alloc::System;

// ---- macros ----

/// Define the program entry point: `rust_std::main!(main);` where `main`
/// returns any [`process::Termination`] type.
#[macro_export]
macro_rules! main {
    ($main:path) => {
        #[doc(hidden)]
        #[unsafe(no_mangle)]
        pub fn __rust_std_main() -> i32 {
            $crate::process::Termination::report($main()).to_i32()
        }

        // keep the runtime's `_start` in the link
        #[doc(hidden)]
        #[used]
        static __RUST_STD_START: unsafe extern "C" fn() -> ! = $crate::rt::_start;
    };
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::io::_print($crate::format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! println {
    () => {
        $crate::print!("\n")
    };
    ($($arg:tt)*) => {
        $crate::io::_print($crate::format_args!("{}\n", $crate::format_args!($($arg)*)))
    };
}

#[macro_export]
macro_rules! eprint {
    ($($arg:tt)*) => {
        $crate::io::_eprint($crate::format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! eprintln {
    () => {
        $crate::eprint!("\n")
    };
    ($($arg:tt)*) => {
        $crate::io::_eprint($crate::format_args!("{}\n", $crate::format_args!($($arg)*)))
    };
}

#[macro_export]
macro_rules! dbg {
    () => {
        $crate::eprintln!("[{}:{}:{}]", $crate::file!(), $crate::line!(), $crate::column!())
    };
    ($val:expr $(,)?) => {
        match $val {
            tmp => {
                $crate::eprintln!("[{}:{}:{}] {} = {:#?}",
                    $crate::file!(), $crate::line!(), $crate::column!(), $crate::stringify!($val), &tmp);
                tmp
            }
        }
    };
    ($($val:expr),+ $(,)?) => {
        ($($crate::dbg!($val)),+,)
    };
}
