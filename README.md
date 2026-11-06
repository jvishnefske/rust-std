# std on `*-unknown-linux-none`

The real Rust standard library on Linux **without a libc**: static, non-PIE
executables for x86_64 and aarch64 that talk to the kernel with raw syscalls.
Ordinary `fn main()` programs, crates.io crates and libtest work unchanged.

```rust
fn main() {
    let names: Vec<String> = std::env::args().collect();
    std::thread::spawn(move || println!("{names:?}")).join().unwrap();
}
```

```console
$ cargo xtask sysroot            # once: build the patched std sources
$ cd apps && cargo build -p examples
$ file target/x86_64-unknown-linux-none/debug/hello
... ELF 64-bit LSB executable, x86-64, ..., statically linked ...
```

## How it works

Rust has a built-in target `x86_64-unknown-linux-none` (Linux, no libc, no
std). This repository gives it a std, the way the `motor` target has one: each
`std::sys` subsystem dispatches to a `linux_none` implementation built on a
small runtime crate instead of libc.

- **`rt/`** (`linux_none_rt`): everything libc would provide to std, on raw
  syscalls, using only `core`. The ELF entry point (`_start` calls the C
  `main` rustc generates), argc/argv/envp/auxv, ELF TLS from `PT_TLS`
  (variant II on x86_64, variant I on aarch64), threads on `clone` with
  `CLONE_CHILD_CLEARTID` join, process spawning (`clone(CLONE_VM|CLONE_VFORK)`
  + `execve`, or a full fork for `pre_exec` callbacks), an mmap size-class
  allocator, futexes and `getdents64` parsing. Kani proofs cover the
  allocator's size classes, the TLS layout, syscall error decoding and the
  dirent parser.
- **`std-patches/`**: a patch series against `library/` of the pinned
  nightly's `rust-src`. It adds `cfg(linux_none)` (linux with an empty
  `target_env`), a `linux_none` arm ahead of the `unix` one in each `sys`
  dispatcher, and the new files `sys/*/linux_none.rs`. Where upstream's unix
  code is libc-free or nearly so it is reused (args, stdio, time, path,
  `os_str`, futex-based sync, `process/unix/common.rs`, the generic socket
  layer); fs, fd, thread, process spawning and sockets have their own files.
  `os::unix`, `os::linux` and `os::fd` compile unmodified apart from a few
  import lines.
- **libc** stays a dependency of std, for the types and constants `os::unix`
  exposes. libc has no definitions for linux with an empty `target_env`, so
  `cargo xtask sysroot` patches std's copy to use musl's (whose layouts are
  the kernel's) and to link nothing. No libc *function* is ever called: the
  harness checks that every binary has no undefined symbols.
- **`targets/aarch64-unknown-linux-none.json`**: there is no built-in aarch64
  linux-none target; this is the x86_64 one with aarch64's arch, data layout
  and features.
- **`xtask/`**: `cargo xtask sysroot` copies the pinned `rust-src` into
  `target/linux-none/std-src`, patches libc, applies `std-patches/` as git
  commits, links `rt/` into the library workspace, and registers a
  `linux-none` rustup toolchain (a hard-linked copy of the pinned nightly
  whose `rust-src` is the patched tree). `cargo xtask export-patches` turns
  commits made in `target/linux-none/std-src` back into `std-patches/`.

## Building programs

`apps/` is a workspace already set up for this: its `rust-toolchain.toml`
selects the `linux-none` toolchain and `.cargo/config.toml` sets the target,
`-Zbuild-std` and the static, non-PIE link flags.

```console
$ cd apps
$ cargo build                                   # x86_64
$ cargo build --target ../targets/aarch64-unknown-linux-none.json
$ cargo test -p stdtests -Zbuild-std=std,panic_abort,test -Zpanic-abort-tests
```

For your own crate, copy `apps/.cargo/config.toml` and
`apps/rust-toolchain.toml` (or use `cargo +linux-none`). Panics abort
(`panic = "abort"` is required), and tests need `-Zpanic-abort-tests`.

Cargo does not notice changes to `-Zbuild-std` sources. After re-running
`cargo xtask sysroot`, `cargo clean` your project. When editing std or `rt`,
build with the pinned nightly and
`__CARGO_TESTS_ONLY_SRC_ROOT=$PWD/target/linux-none/std-src/library`. The
harness also passes a hash of the sources in `--cfg linux_none_src=...`, so
that its builds are rebuilt.

## What differs from std on linux-gnu

- **Panics abort**: there is no unwinder. `catch_unwind` cannot catch, and
  backtraces are empty.
- **No DNS**: name lookup knows numeric addresses and `/etc/hosts`.
- **No passwd database**: `env::home_dir` uses `$HOME` only.
- **`fs::canonicalize`** asks `/proc/self/fd`, so it needs `/proc`.
- **`io::copy`** copies through a buffer: there is no
  `copy_file_range`/`sendfile`/`splice` fast path yet.
- **`thread::available_parallelism`** counts the affinity mask and ignores
  cgroup CPU quotas.
- **`os::unix::thread::JoinHandleExt`** is not implemented, since there are
  no pthreads to hand out.
- **Stack overflows** are a plain SIGSEGV on the guard page, without std's
  "has overflowed its stack" message.

## Testing

`cargo test` (in the repository root) runs the harness. It builds the
patched sources if needed, then for x86_64 and for aarch64:

- **Examples**: it builds `apps/examples` and checks what each program
  prints, and that every binary is a static ELF with no `PT_INTERP`,
  `PT_DYNAMIC` or undefined symbol.
- **std's own tests**: it runs upstream `library/std/tests` (the
  `apps/stdtests` crate) under libtest with `-Zpanic-abort-tests`, so every
  test runs in a child process. Skipped are the ten sync tests that need
  catching a panic. They fail the same way with std on linux-gnu under
  `panic=abort`. `switch-stdout.rs` is left out because it calls libc
  itself.
- **Differential fuzzing**: `apps/fuzzee/ops.rs` runs both in-process
  against the host's std and in the `fuzzee` binary against linux-none's
  std, and the transcripts must match (`FUZZ_SEED`, `FUZZ_ITERS`,
  `FUZZ_SECS`).

aarch64 programs run natively on an aarch64 host, or under qemu when
`$QEMU_AARCH64` or `qemu-aarch64` on PATH is available. Otherwise they are
only built and checked statically. std's tests and the smoke example start
copies of themselves, so under qemu they also need qemu registered with
binfmt_misc (`qemu-user-static` does that). Without binfmt the tests are
only built. std's in-crate unit tests
(`library/std/src/**/tests.rs`) are not run yet.

The Kani proofs run with `cd rt && cargo kani`.

## Requirements

- rustup, with the nightly pinned in `rust-toolchain.toml` and its
  `rust-src` (rustup installs both).
- git (`cargo xtask sysroot` applies the patches with `git am`).
- Network access on first build, for std's crates.io dependencies.
- Linux 4.11 or later on the target (for `statx`). Newer interfaces
  (`GRND_INSECURE`, `fchmodat2`, pidfds) are used when available, with
  fallbacks or errors otherwise.
- Optionally `qemu-user`, to run the aarch64 tests on x86_64.

Updating the pinned nightly means bumping `rust-toolchain.toml`, running
`cargo xtask sysroot`, fixing whatever no longer applies in
`target/linux-none/std-src`, committing there and running
`cargo xtask export-patches`.
