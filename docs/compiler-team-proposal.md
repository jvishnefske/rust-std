# Draft outline: compiler-team proposal

Not filed. For rust-lang/compiler-team, after the Zulip post
([zulip-post.md](zulip-post.md)) gets a positive answer.

## Title

std support for `*-unknown-linux-none`

## Proposal

- Mark `x86_64-unknown-linux-none` as having std (still tier 3), with a
  native std backend that makes raw syscalls and links no libc.
- Add `aarch64-unknown-linux-none`, tier 3, from the JSON spec in
  `targets/`.
- Name a target maintainer (per the tier 3 target policy).

## Motivation

- Static, non-PIE executables with no C in the trust base.
- A known, minimal syscall surface, which makes tight seccomp profiles
  possible.
- Early boot and initramfs tools without a C toolchain.
- An honest note: musl static covers most "static binary" needs. This is
  for the cases where having no libc at all is the point.

## Design

- Each `sys/*` subsystem gets a `linux_none` `cfg_select!` arm placed
  before the `unix` arm, the same way the `motor` target is wired in.
- The runtime is `linux_none_rt`, a `rustc-dep-of-std` crate. Open
  question: in-tree under `library/`, or on crates.io.
- `libc`: for linux with an empty `target_env`, use musl's types and
  constants. Upstream that to rust-lang/libc first.
- backtrace-rs: gimli symbolization from the executable's own ELF image.
  Upstream that to backtrace-rs, then bump the submodule.
- `panic=abort` by default. `-Cpanic=unwind` works too, with std's
  `panic-unwind` feature; backtraces symbolize through gimli.

## Tier 3 policy checklist

To fill in point by point: maintainer, naming, no legal issues, no
host-tools requirement, and the "no impact on other targets" argument
(new arms only, gated on `target_os = "linux"` with an empty
`target_env`).

## Testing

- The `library/std/tests` suites and std's unit tests on both targets
  (aarch64 under qemu), currently run by an out-of-tree harness.
- Plan for `x.py`: what can run in rust's CI for a tier 3 target,
  probably build-only.

## Alternatives

- Keep `linux-none` as `no_std`. People who want this use Eyra or musl.
- Eyra's approach: libc reimplemented in Rust, with std unchanged.
- Out-of-tree only: the status quo, with a patched std on a pinned
  nightly.

## Mentors and reviewers

Ask on Zulip; leave blank until then.
