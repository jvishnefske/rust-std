# Draft: Zulip post

Not posted. For `#t-libs` (cross-post a link in `#t-compiler`), topic
"std for *-unknown-linux-none".

---

Hi all. I have a working std backend for `x86_64-unknown-linux-none`
(plus an aarch64 counterpart as a target JSON), and before writing an MCP I
want to ask whether this is something std would take.

**What it is:** std's `sys/` gets a `linux_none` arm in each subsystem,
modelled on how the `motor` target is wired in. It is backed by a
`#![no_std]` runtime crate, `linux_none_rt`, that makes raw syscalls: no
libc anywhere. It covers threads (futex-based sync, ELF TLS), fs, process
spawn, net, a DNS stub resolver, vDSO time, and unwinding and backtraces
through backtrace-rs's gimli path. The `libc` crate is still used for
types and constants only, treated as musl.

**How it's tested:** upstream `library/std/tests` and std's own unit tests
pass on both arches (aarch64 under qemu), along with a differential
fuzzer against std on linux-gnu. Kani proofs cover parts of the runtime.
Separately, 40 nixpkgs Rust packages (ripgrep, fd, bat, a zenoh-based
workspace with 367 tests, ...) build and pass smoke tests as fully static
binaries with no undefined symbols.

**Size:** 10 patches to `library/`, about 3.8k added lines, plus the
runtime crate (about 8k lines, including a small C-function layer that
only matters for crates with C code).

**Branch:** https://github.com/jvishnefske/rust/tree/linux-none-std, on
`cea272f`, plus a backtrace-rs branch at
https://github.com/jvishnefske/backtrace-rs/tree/linux-none.

**Questions:**

1. Would t-libs consider std on `*-linux-none` at all, or is the target
   meant to stay `no_std`?
2. If yes: should the runtime live in-tree under `library/`, or be a
   crates.io dependency like `moto-rt`?
3. Is an MCP the right next step, and is there someone who'd be willing
   to shepherd it?
4. The libc change (linux with an empty `target_env` treated as musl) and
   the backtrace-rs change would go to their repos first. Anything that
   should be done differently there?
