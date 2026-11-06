//! std's own integration tests (`library/std/tests`), compiled against
//! rust_std. `build.rs` adapts the verbatim copies in `upstream/`.
//!
//! Usage: `stdtests --list` prints `name\tshould_panic\tignore\texpected`
//! lines; `stdtests --skipped` prints removed tests and why; `stdtests NAME`
//! runs one test (exit 0 on success). Each test runs in its own process
//! because a panic aborts the whole process; see harness/tests/stdtests.rs.
#![no_std]
#![no_main]
#![allow(unused_attributes, unused_imports, dead_code, unused_macros, unexpected_cfgs)]
// upstream builds these tests with an older edition, where this is a warning
#![allow(static_mut_refs)]
#![allow(clippy::all)]

extern crate rust_std as std;
use std::prelude::*;
use std::process::ExitCode;

std::main!(main);

pub struct Test {
    pub name: &'static str,
    pub f: fn(),
    pub should_panic: Option<&'static str>,
    pub ignore: bool,
}

include!(concat!(env!("OUT_DIR"), "/registry.rs"));

/// Tests may return `()` or a `Result`, like `#[test]` functions.
fn report<T: std::process::Termination>(t: T) {
    assert!(t.report() == ExitCode::SUCCESS, "test returned an error");
}

fn main() -> ExitCode {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        None | Some("--list") => {
            for t in tests() {
                println!("{}\t{}\t{}\t{}", t.name, t.should_panic.is_some() as u8, t.ignore as u8, t.should_panic.unwrap_or(""));
            }
            ExitCode::SUCCESS
        }
        Some("--skipped") => {
            for (name, why) in SKIPPED {
                println!("{name}\t{why}");
            }
            ExitCode::SUCCESS
        }
        Some(name) => match tests().into_iter().find(|t| t.name == name) {
            Some(t) => {
                (t.f)();
                ExitCode::SUCCESS
            }
            None => {
                eprintln!("no such test: {name}");
                ExitCode::from(2)
            }
        },
    }
}

/// Stand-in for upstream's `tests/common/mod.rs` (which needs the `rand` crate).
pub mod common {
    use std::prelude::*;
    use std::hash::{BuildHasher, Hash, Hasher};
    use std::path::{Path, PathBuf};
    use std::{env, fs, thread};

    pub struct TestRng(u64);

    impl TestRng {
        pub fn next_u64(&mut self) -> u64 {
            // xorshift64*
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545F4914F6CDD1D)
        }
        pub fn next_u32(&mut self) -> u32 {
            (self.next_u64() >> 32) as u32
        }
    }

    #[track_caller]
    pub fn test_rng() -> TestRng {
        let mut hasher = std::hash::RandomState::new().build_hasher();
        core::panic::Location::caller().hash(&mut hasher);
        TestRng(hasher.finish() | 1)
    }

    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn join(&self, path: &str) -> PathBuf {
            self.0.join(path)
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let result = fs::remove_dir_all(&self.0);
            if !thread::panicking() {
                result.unwrap();
            }
        }
    }

    #[track_caller]
    pub fn tmpdir() -> TempDir {
        let ret = env::temp_dir().join(format!("rust-{}", test_rng().next_u32()));
        fs::create_dir(&ret).unwrap();
        TempDir(ret)
    }
}

/// The few `rand` items the upstream tests use.
pub mod rand {
    pub trait Rng {
        fn random_bool(&mut self, p: f64) -> bool;
    }

    impl Rng for crate::common::TestRng {
        fn random_bool(&mut self, p: f64) -> bool {
            ((self.next_u64() >> 11) as f64 / (1u64 << 53) as f64) < p
        }
    }

    pub mod distr {
        use std::prelude::*;
        use crate::common::TestRng;

        pub struct Alphanumeric;

        pub trait SampleString {
            fn sample_string(&self, rng: &mut TestRng, len: usize) -> String;
        }

        impl SampleString for Alphanumeric {
            fn sample_string(&self, rng: &mut TestRng, len: usize) -> String {
                const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
                (0..len).map(|_| CHARS[(rng.next_u32() as usize) % CHARS.len()] as char).collect()
            }
        }
    }
}

macro_rules! suite {
    ($name:ident, $file:literal) => {
        pub mod $name {
            use std::prelude::*;
            include!(concat!(env!("OUT_DIR"), "/", $file));
        }
    };
}

suite!(path, "path.rs");
suite!(env, "env.rs");
suite!(env_modify, "env_modify.rs");
suite!(time, "time.rs");
suite!(thread, "thread.rs");
suite!(thread_local, "thread_local.rs");
suite!(istr, "istr.rs");
suite!(seq_compare, "seq_compare.rs");
suite!(builtin_clone, "builtin_clone.rs");
suite!(create_dir_all_bare, "create_dir_all_bare.rs");
suite!(num, "num.rs");

/// Mirrors upstream's `tests/sync/lib.rs`.
pub mod sync {
    #[track_caller]
    fn result_unwrap<T, E: std::fmt::Debug>(x: Result<T, E>) -> T {
        x.unwrap()
    }

    /// Upstream generates a `nonpoison_` and a `poison_` variant of each test.
    /// rust_std has only the (never poisoned) std locks, so this generates the
    /// `poison_` variant under the plain test name.
    macro_rules! nonpoison_and_poison_unwrap_test {
        (name: $name:ident, test_body: {$($test_body:tt)*}) => {
            pub fn $name() {
                #[allow(unused_imports)]
                use super::result_unwrap as maybe_unwrap;
                use ::std::sync as locks;

                $($test_body)*
            }
        };
    }
    pub(crate) use nonpoison_and_poison_unwrap_test;

    suite!(barrier, "sync__barrier.rs");
    suite!(condvar, "sync__condvar.rs");
    suite!(lazy_lock, "sync__lazy_lock.rs");
    suite!(mpsc, "sync__mpsc.rs");
    suite!(mpsc_sync, "sync__mpsc_sync.rs");
    suite!(mutex, "sync__mutex.rs");
    suite!(once, "sync__once.rs");
    suite!(once_lock, "sync__once_lock.rs");
    suite!(rwlock, "sync__rwlock.rs");
}
