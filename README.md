# rust_std

A `std`-compatible runtime for Linux on **x86_64** and **aarch64** that talks
to the kernel through raw syscalls. It uses no glibc, no musl and no libc crate.

It is an ordinary `#![no_std]` crate that builds on **stable Rust**. Programs
alias it as `std` and get familiar APIs: `println!`, files, threads, channels,
processes, sockets, `HashMap`, and more. The result is a small, fully static
executable: a release "hello world" is about 44 KB and has no dynamic section
and no undefined symbols.

```rust
#![no_std]
#![no_main]
extern crate rust_std as std;
use std::prelude::*;

std::main!(main);

fn main() -> std::io::Result<()> {
    let names: Vec<String> = std::env::args().skip(1).collect();
    std::fs::write("/tmp/names.txt", names.join("\n"))?;
    let handle = std::thread::spawn(move || names.len());
    println!("wrote {} names", handle.join().unwrap());
    Ok(())
}
```

## What's provided

| Module | Contents |
|---|---|
| `io` | `Read`/`Write`/`BufRead`/`Seek`, `Error`/`ErrorKind` (mapped from errno), `stdin`/`stdout`/`stderr` (stdout line-buffered), `BufReader`, `BufWriter`, `LineWriter`, `Cursor`, `copy`, `empty`, `sink`, `repeat`, `IsTerminal` |
| `fs` | `File`, `OpenOptions`, `Metadata` (via `statx`), `Permissions`, `read_dir`, `create_dir_all`, `remove_dir_all`, `copy`, `rename`, `read_link`, `canonicalize`, `DirBuilder`, … |
| `path`, `ffi` | `Path`/`PathBuf` with std's component semantics; byte-based `OsStr`/`OsString`; `CStr`/`CString` |
| `env` | `args`, `vars`, `var`, `set_var`/`remove_var`, `current_dir`, `current_exe`, `temp_dir`, `split_paths`/`join_paths` |
| `process` | `Command` (fork via `clone`, `execve`, `PATH` lookup, piped/null/inherited stdio, env and cwd), `Child`, `Output`, `ExitStatus`, `ExitCode`, `Termination`, `exit`, `abort` |
| `thread` | `spawn`, `Builder`, `JoinHandle`, `scope`, `park`/`unpark`, `sleep`, `current`, `available_parallelism`, `thread_local!` |
| `sync` | Futex-based `Mutex`, `RwLock`, `Condvar`, `Once`, `OnceLock`, `LazyLock`, `Barrier`, `mpsc` (including rendezvous `sync_channel(0)`), plus `Arc` and `atomic` |
| `net` | `TcpStream`, `TcpListener`, `UdpSocket`, `ToSocketAddrs` |
| `time` | `Instant` (`CLOCK_MONOTONIC`), `SystemTime`, `UNIX_EPOCH`, `Duration` |
| `collections`, `hash` | All of `alloc::collections`, plus `HashMap`/`HashSet` (hashbrown, with randomly keyed SipHash) |
| `os` | `os::fd` (`OwnedFd`, `AsRawFd`, …) and `os::unix` extension traits (`OsStrExt`, `PermissionsExt`, `MetadataExt`, `ExitStatusExt`, `symlink`, …) |

Everything else std re-exports from `core` and `alloc` (`fmt`, `vec`, `string`,
`boxed`, `rc`, `cell`, `iter`, …) is re-exported the same way.

## Using it in your own crate

```toml
# Cargo.toml
[dependencies]
rust_std = { path = "../rust-std" }

[profile.dev]
panic = "abort"
[profile.release]
panic = "abort"
```

The binary must be linked **statically**, **without crt startup files or
libc**, and **not as PIE**, because the runtime does not relocate itself. A
`build.rs` like this one sets the flags for your binaries only:

```rust
// build.rs
fn main() {
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let args: &[&str] = if arch == "aarch64" {
        &["-static", "--no-pie"] // linked with rust-lld, see below
    } else {
        &["-nostartfiles", "-nostdlib", "-static", "-no-pie"] // linked via cc
    };
    for a in args {
        println!("cargo:rustc-link-arg-bins={a}");
    }
}
```

Cross-compiling to aarch64 needs only the rustup target, not a cross GCC or
sysroot. The bundled `rust-lld` does the linking:

```toml
# .cargo/config.toml
[target.aarch64-unknown-linux-gnu]
linker = "rust-lld"
rustflags = ["-C", "linker-flavor=ld.lld"]
```

```sh
rustup target add aarch64-unknown-linux-gnu
cargo build --release --target aarch64-unknown-linux-gnu
```

Each program needs `#![no_std]`, `#![no_main]`,
`extern crate rust_std as std;` and `std::main!(main);`. `main` may return
`()`, `ExitCode`, or `Result<_, E: Debug>`, as with std. Bring the macros
(`println!`, `vec!`, `format!`, …) and the usual prelude types into scope with
`use std::prelude::*;`.

The `global-allocator` feature, on by default, installs the runtime's
allocator. Disable it to provide your own.

## Differences from std

- **Panics always abort.** There is no unwinder, so `catch_unwind` doesn't
  exist and `JoinHandle::join` never returns `Err`. Locks are never poisoned,
  but `lock()` still returns a `LockResult` for source compatibility. A panic
  prints the usual message and raises `SIGABRT`.
- **No DNS.** `ToSocketAddrs` resolves IP literals and `localhost` only.
- **No libm.** Float methods std implements through libm (`sqrt`, `sin`, …)
  are only available if `core` provides them.
- **`HashMap`/`HashSet`** deref to hashbrown's types. Method names match std,
  but the iterator and `Entry` types are hashbrown's.
- **Thread-local destructors** run when spawned threads exit, but not for the
  main thread.
- **Unstable std APIs are not provided**, for example `sync::nonpoison`,
  mapped lock guards, `Mutex::set`, and `SystemTime::MAX`.
- **Built-in `#[test]` doesn't work in rust_std programs.** libtest needs the
  real std. See below for how this repository tests itself.

## How it works

| Path | Purpose |
|---|---|
| `src/sys/arch/{x86_64,aarch64}.rs` | Syscall numbers and `syscall6`, the naked `_start` entry point, the `clone` trampoline for threads, self-unmapping thread exit, and the thread pointer (`fs` / `tpidr_el0`) |
| `src/sys/linux.rs` | Typed syscall wrappers returning `Result<_, errno>` |
| `src/rt.rs` | Startup: reads argc/argv/envp/auxv from the initial stack, sets up main-thread TLS, runs `.init_array`, ignores `SIGPIPE`, calls `main`, flushes stdout. Also holds the panic handler and the `getauxval` that compiler-builtins needs on aarch64. |
| `src/sys/alloc.rs` | Allocator: power-of-two size classes up to 32 KiB carved from `mmap`'d chunks; larger blocks are `mmap`'d directly |
| `src/sys/locks.rs`, `futex.rs` | Futex mutex, condvar and rwlock that every `sync` type is built on |
| `src/sys/tls.rs` | Thread-local storage without compiler TLS: a per-thread block at the thread pointer, indexed by `thread_local!` keys |
| `mem/` | `memcpy`, `memmove`, `memset`, `memcmp`, `bcmp` and `strlen`, in a separate `#![no_builtins]` crate so LLVM can't compile them into calls to themselves |

Each thread's stack, guard page and control block share one `mmap`.
`JoinHandle::join` waits on the futex that `CLONE_CHILD_CLEARTID` releases, then
unmaps the stack. A detached thread unmaps its own stack on exit. Before doing
so it unregisters its clear-tid word with `set_tid_address(NULL)`, so the
kernel's exit-time write can't land in memory reused by another thread.

## Testing

```sh
cargo test                                   # everything below, both arches
cargo build -p rust_std --examples           # just the example programs
cargo build -p stdtests && ./target/debug/stdtests --list
```

`cargo test` runs the host-side `harness` crate, which:

1. **Runs the examples** in `examples/` (hello, args/env, fs, threads, process,
   net, collections, panic, stdin) and checks their output and exit status. It
   also checks that every binary is a static, non-PIE ELF with no interpreter
   or dynamic section.
2. **Runs std's own test suite.** The `stdtests` crate contains verbatim copies
   of 20 files from rust-lang/rust's `library/std/tests/` (in
   `stdtests/upstream/`), adapted at build time by `stdtests/build.rs`. 255
   tests run, each in its own process so panics and `#[should_panic]` work.
   82 are skipped, every one with a recorded reason: 34 need unwinding and the
   rest use unstable APIs. List them with `stdtests --skipped`.

3. **Fuzzes rust_std against std.** `fuzz/ops.rs` is compiled twice: into the
   harness against the real std, and into the `fuzzee` binary against
   rust_std. Random inputs drive seven targets: `path`, `osstr_fmt`,
   `bufread`, `writers`, `cursor`, `alloc` and `fs`. Any difference in the
   two transcripts, or a crash or hang, fails the test, which then reports a
   shrunk reproducer. `FUZZ_SEED` reproduces a run, `FUZZ_ITERS` sets cases
   per target (default 3000), and `FUZZ_SECS=60` runs a longer campaign:
   `FUZZ_SECS=60 cargo test --test fuzz host_fuzz`.

aarch64 runs under qemu user-mode emulation when it's available. Set
`QEMU_AARCH64=/path/to/qemu-aarch64` or put `qemu-aarch64` on `PATH`, and add
the `aarch64-unknown-linux-gnu` rustup target. Without both, the aarch64 tests
are skipped with a message. To keep core dumps from crashing tests, set
`STDTESTS_CORE_DIR=<dir>`.

`cargo test` defaults to the `harness` package because the test profile forces
`panic = "unwind"`, which rust_std binaries can't use. The harness builds the
binaries itself with the release profile.

## Kani proofs

[Kani](https://github.com/model-checking/kani) proves properties for *all*
inputs up to a size bound. The harnesses live in `#[cfg(kani)] mod proofs`
next to the code they check:

- **`mem`**: `memcpy`, `memmove` (any overlap), `memset`, `memcmp`/`bcmp`
  and `strlen` match a byte-wise model for every alignment and length
  within 24 bytes, covering the word-at-a-time fast paths.
- **Allocator**: every layout either goes to `mmap` or gets a size class
  whose blocks are large and aligned enough. `realloc`'s same-class fast
  path is sound, and `round_up` is correct.
- **Syscalls**: return values decode to `Err(errno)` exactly for −4095..=−1.
- **`Path`**: mixing `next()` and `next_back()` in any order yields the
  forward component sequence; `as_path()` stays within the path; and
  `file_name()` matches the last component.
- **Smaller helpers**: C-string conversion, environment-entry parsing, the
  `split_paths`/`join_paths` round-trip, `SystemTime` nanosecond conversion,
  and `Cursor` seeking.

```sh
cargo install --locked kani-verifier && cargo kani setup   # once
cargo kani -p rust_std -p rust_std_mem
```

Kani verifies against its own std, so under `cfg(kani)` the crate drops its
`#[panic_handler]`, global allocator and exported C symbols. Syscalls and
threads (inline asm) are outside what Kani can check; the tests and fuzzer
cover those.

## CI

`.github/workflows/ci.yml` runs on every push and pull request:

- **x86_64 runner**: clippy (including the aarch64 cross target), then
  `cargo test`, with aarch64 cross-built and run under `qemu-user`.
- **Native aarch64 runner** (`ubuntu-24.04-arm`): clippy and `cargo test`.
- **Kani job**: all proofs, using `model-checking/kani-github-action`.

## Requirements

- Linux on x86_64 or aarch64 (a kernel with `statx`, so 4.11 or newer).
- Stable Rust. Developed with rustc 1.99.1, edition 2024.
- For x86_64, a `cc` linker driver; aarch64 uses the bundled `rust-lld`.
