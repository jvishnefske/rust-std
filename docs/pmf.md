# Product-market fit: std on `*-unknown-linux-none`

An evaluation as of 2026-11. The drafts for the next steps are
[zulip-post.md](zulip-post.md) and
[compiler-team-proposal.md](compiler-team-proposal.md).

**Verdict:** the engineering is strong, but there is no product-market fit
yet, and the market as it stands is narrow. Real std, crates.io crates
and libtest run unchanged, and linux-none-pkgs builds 40 of 42 nixpkgs
packages. But "Rust without libc" isn't something many people are trying
to buy, and musl already meets most of the need for free.

## The problem, and who has it

What most people want is "one static binary I can copy anywhere."
`x86_64-unknown-linux-musl` with `+crt-static` already does that on stable
Rust, with no patched std. That covers containers built `FROM scratch`,
CLI tools distributed as one file, and most embedded Linux.

What's left is the smaller group who care that there is no C at all:

| Segment | Why they'd care | How strong |
|---|---|---|
| Security-sensitive or sandboxed services | Memory-safe all the way down; a known, small syscall set for tight seccomp profiles; Kani proofs on the runtime | **The strongest wedge** |
| initramfs, early boot, rescue tools | No libc, no loader, small and predictable | Real but tiny |
| Supply-chain and audit teams | No C toolchain in the trust base | Real, but rarely early adopters |
| Hermetic, reproducible builds (Nix) | linux-none-pkgs shows it working | Enthusiasts, not a market |
| Anyone hurt by musl itself | musl's slow malloc and weaker DNS | Weak: the allocator can be swapped, and musl 1.2.4 added TCP fallback for DNS |

## Competition

- **musl static.** Stable Rust, zero setup, good enough for nearly
  everyone. To beat it this needs a measurable win, not "no C" alone.
- **Eyra (c-ward and origin).** The closest alternative: libc
  reimplemented in Rust, with std left unpatched. Nearly the same pitch,
  with prior mindshare. "Why not Eyra?" needs a clear answer: no libc ABI
  layer, a native std backend, verified pieces, and a path upstream.
- **Upstream doing nothing.** The `linux-none` target exists without std,
  so whatever upstream decides about std there sets the ceiling.

## What blocks adoption

1. **A pinned nightly plus a patched std.** Users take on the cost of
   rebasing the patches whenever they move nightlies. That alone rules
   out most production users.
2. **`__CARGO_TESTS_ONLY_SRC_ROOT`.** Pointing cargo at the patched std
   goes through a cargo-internal testing variable, which reads as
   "experimental, don't depend on this."
3. **C dependencies.** Crates with C code (OpenSSL and similar) only work
   as far as rt's clib reaches. That's the rustls-only world: fine for new
   services, not for existing ones.
4. **No proof of benefit.** There are no published numbers against musl:
   binary size, startup time, allocator throughput, syscall count.

## Options

**Upstreaming** is the only route to fit at scale. If std on
`*-linux-none` becomes an official rustup target, the patching and the
pinned nightly go away. The compiler-team proposal matters more than any
new feature. Without it this stays a strong reference implementation that
people admire and don't depend on.

Short of that, a **niche wedge**: "auditable, seccomp-tight static Rust
services." For example, a tool that prints a binary's exact syscall set and
generates a seccomp profile from it.

## Validation before investing more

1. **Benchmarks** against musl-static and Eyra on ripgrep, a rustls web
   server and middleware: size, cold start, RSS, an allocator-heavy
   workload, the syscall surface. If none is clearly better, the "no C"
   story stands alone.
2. **Interviews** with 10 people from container security, Nix, embedded
   Linux and rust-lang's target maintainers. Ask something concrete: "would
   you run this in CI next month?"
3. **Upstream temperature:** an informal Zulip post on t-compiler and
   t-libs before the formal proposal. A "no" changes the strategy.
4. **Stopping rule:** after one quarter with no upstream interest and no
   outside users, keep it as a research project and stop adding features.

## Scores

| Dimension | Score |
|---|---|
| Problem severity | 4/10 |
| Market size | 3/10 |
| Solution quality | 9/10 |
| Differentiation vs musl | 4/10 |
| Differentiation vs Eyra | 6/10 |
| Distribution and adoptability today | 2/10 |
| PMF today | 2/10 |
| PMF if upstreamed | 6/10 |
