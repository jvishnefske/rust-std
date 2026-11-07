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
  allocator, futexes, errno, `getdents64` parsing, the alternate signal stacks
  and guard-page lookup behind std's stack overflow message, the vDSO's
  `clock_gettime`, a DNS stub resolver, `/etc/passwd` lookup, and the
  program headers from `AT_PHDR` (what `dl_iterate_phdr` gives a static
  executable). Kani proofs cover the
  allocator's size classes, the TLS layout, syscall error decoding, the
  dirent parser, the `/proc/self/maps` parser, the vDSO parser, the
  `PT_GNU_EH_FRAME` lookup, DNS name decoding and the passwd parser.
- **`std-patches/`**: a patch series against `library/` of the pinned
  nightly's `rust-src`. It adds `cfg(linux_none)` (linux with an empty
  `target_env`), a `linux_none` arm ahead of the `unix` one in each `sys`
  dispatcher, and the new files `sys/*/linux_none.rs`. Where upstream's unix
  code is libc-free or nearly so it is reused (args, stdio, time, path,
  `os_str`, futex-based sync, `process/unix/common.rs`, the generic socket
  layer); fs, fd, thread, process spawning and sockets have their own files.
  `os::unix`, `os::linux` and `os::fd` compile unmodified apart from a few
  import lines.
- **Unwinding** uses the pure-Rust [`unwinding`] crate (core only, already
  std's unwinder on Xous) in place of libgcc_s/libunwind. std registers an
  FDE finder that hands it the executable's `.eh_frame_hdr`, found through
  `PT_GNU_EH_FRAME`. Backtraces walk the stack with it under either panic
  strategy and are symbolized by std's backtrace-rs (gimli) from
  `current_exe`'s debug info, with `mmap` and the library list on rt
  instead of libc.
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
`apps/rust-toolchain.toml` (or use `cargo +linux-none`). With that
configuration panics abort, and tests need `-Zpanic-abort-tests`.

### panic=unwind

The targets default to `panic=abort` (the built-in x86_64 spec says so, and
the aarch64 JSON copies it), but rustc accepts `-Cpanic=unwind` for them.
Cargo never passes that flag itself, since it assumes unwinding is the
default, so put it in the rustflags, and build `panic_unwind` with std's
`panic-unwind` feature:

```console
$ cd apps    # --config arrays add to .cargo/config.toml's rustflags
$ cargo build --config 'profile.dev.panic="unwind"' \
    --config 'target.x86_64-unknown-linux-none.rustflags=["-Cpanic=unwind"]' \
    -Zbuild-std=std,panic_abort,panic_unwind,test \
    -Zbuild-std-features=compiler-builtins-mem,backtrace,panic-unwind
```

`catch_unwind`, panics crossing `JoinHandle::join`, lock poisoning and
plain libtest (no `-Zpanic-abort-tests`) then work as on linux-gnu.
`RUST_BACKTRACE=1` prints symbolized frames under both strategies (std's
`backtrace` feature, on in `apps/.cargo/config.toml`).

[`unwinding`]: https://github.com/nbdd0121/unwinding

Cargo does not notice changes to `-Zbuild-std` sources. After re-running
`cargo xtask sysroot`, `cargo clean` your project. When editing std or `rt`,
build with the pinned nightly and
`__CARGO_TESTS_ONLY_SRC_ROOT=$PWD/target/linux-none/std-src/library`. The
harness also passes a hash of the sources in `--cfg linux_none_src=...`, so
that its builds are rebuilt.

## What differs from std on linux-gnu

- **Panics abort by default**: unwinding needs `-Cpanic=unwind` (above).
  The unwinder only knows the executable's own unwind tables (there are no
  shared objects), and does not unwind through signal frames.
- **Name lookup** reads `/etc/hosts`, then asks the DNS servers in
  `/etc/resolv.conf` (A and AAAA, TCP for truncated answers): there is no
  NSS, so no mDNS, `systemd-resolved` varlink or LDAP. Of `options`, only
  `timeout`, `attempts` and `ndots` count; IPv4 addresses come first.
- **`env::home_dir`** without `$HOME` reads `/etc/passwd` only (no NSS).
- **`os::unix::thread::JoinHandleExt`** hands out the address of the
  runtime's control block for the thread as its `pthread_t` (there are no
  pthreads). `into_pthread_t` detaches the thread, since nothing could join
  it through that value.

## Testing

`cargo test` (in the repository root) runs the harness. It builds the
patched sources if needed, then for x86_64 and for aarch64:

- **Examples**: it builds `apps/examples` and checks what each program
  prints, and that every binary is a static ELF with no `PT_INTERP`,
  `PT_DYNAMIC` or undefined symbol; once with `panic=abort` and once with
  `panic=unwind` (in `apps/target/unwind`). `backtrace` and `unwind`
  check symbolized backtraces, `catch_unwind`, destructors during
  unwinding, panics across `join`, poisoning and `resume_unwind`. It
  also overflows the stack of the main and of a spawned thread (std's
  message, then SIGABRT), and checks
  `available_parallelism` under a two-core quota from
  `systemd-run --user -p CPUQuota=200%` (skipped without it). On x86_64
  with `strace` installed it also checks that `kcopy`'s `io::copy` calls
  reach `copy_file_range`, `sendfile` and `splice`, and that `time` makes
  no `clock_gettime` syscall. (qemu-user's vDSO functions make the syscall
  themselves, so under qemu they are only counted.)
- **The vDSO parser**: `harness/tests/vdso.rs` resolves `clock_gettime` in
  the host's vDSO and in a copy of qemu-user's aarch64 one (with
  `DT_GNU_HASH` or `DT_HASH` alone, with and without version tables), and
  feeds both through every truncation and thousands of corruptions
  (`VDSO_FUZZ_ITERS`).
- **std's own tests**: it runs upstream `library/std/tests` (the
  `apps/stdtests` crate) under libtest with `-Zpanic-abort-tests`, so every
  test runs in a child process. Skipped are the ten sync tests that need
  catching a panic. They fail the same way with std on linux-gnu under
  `panic=abort`. `switch-stdout.rs` is left out because it calls libc
  itself. The tests run again with `panic=unwind` and plain libtest
  (each test on a thread), where nothing is skipped and those ten pass.
  Since that needs no child processes, it also runs under qemu without
  binfmt, minus the two test files that start copies of themselves and
  `rwlock::test_rwlock_max_readers` (2^30 read locks, too slow emulated).
- **std's unit tests**: it builds std's in-crate tests
  (`library/std/src/**/tests.rs`) the way `x.py test library/std` does
  (`harness/src/unittests.rs`): `cargo build -p sysroot` in the patched
  library workspace makes a sysroot for the target, then `cargo test -p
  std --lib` builds std as the test crate against it (the sysroot's std
  provides libtest, the panic handler and the other lang items), with
  bootstrap's `-Zforce-unstable-if-unmarked`, `panic=abort` and
  `-Zpanic-abort-tests`. One test is skipped: it fails the same way with
  std on linux-gnu built like this (`harness/tests/std_unittests.rs` lists
  it with its reason, apart from linux-none gaps, of which there are
  none). These builds add a few minutes; `SKIP_STD_UNITTESTS=1` leaves
  them out.
- **DNS**: `tests/dns.rs` runs `rt`'s resolver on the host against a fake
  server on 127.0.0.1 (CNAME chains, TCP fallback, forged replies,
  timeouts, search domains). The `resolve` example checks lookups through
  std with the host's `/etc/resolv.conf`, if the host can resolve names.
- **Differential fuzzing**: `apps/fuzzee/ops.rs` runs both in-process
  against the host's std and in the `fuzzee` binary against linux-none's
  std, and the transcripts must match (`FUZZ_SEED`, `FUZZ_ITERS`,
  `FUZZ_SECS`).

aarch64 programs run natively on an aarch64 host, or under qemu when
`$QEMU_AARCH64` or `qemu-aarch64` on PATH is available. Otherwise they are
only built and checked statically. std's tests and the smoke example start
copies of themselves, so under qemu they also need qemu registered with
binfmt_misc (`qemu-user-static` does that). Without binfmt those are
only built, except for std's tests under `panic=unwind` (see above) and
std's unit tests: the harness then starts each unit test under qemu
itself, as libtest's parent process would (with the test's name in
`__RUST_TEST_INVOKE`).

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
