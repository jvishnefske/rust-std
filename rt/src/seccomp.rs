//! Self-sandboxing: a seccomp filter that `_start` installs before `main`.
//!
//! Build with `LINUX_NONE_SECCOMP=/abs/path/to/allowlist` and the program
//! can make only the syscalls the allowlist names (plus `exit`,
//! `exit_group`, `rt_sigreturn` and `restart_syscall`). Any other syscall
//! gets the allowlist's `action:` (`kill` by default: the process dies of
//! SIGSYS). `build.rs` describes the format and compiles it to BPF, and
//! `cargo xtask syscalls` writes one from a traced run.
//!
//! Since every syscall a linux-none program makes goes through rt (there is
//! no libc underneath), a trace of rt's wrappers is the whole surface. The
//! filter is inherited by threads and child processes, and across `execve`.

use crate::sys;

/// `struct sock_filter`: one classic-BPF instruction.
#[repr(C)]
pub(crate) struct SockFilter {
    pub code: u16,
    pub jt: u8,
    pub jf: u8,
    pub k: u32,
}

#[repr(C)]
struct SockFprog {
    len: u16,
    filter: *const SockFilter,
}

include!(concat!(env!("OUT_DIR"), "/seccomp.rs"));

const PR_SET_NO_NEW_PRIVS: usize = 38;
const SECCOMP_SET_MODE_FILTER: usize = 1;

/// Whether the program was built with an allowlist.
pub fn enabled() -> bool {
    !FILTER.is_empty()
}

/// Install the filter built from `LINUX_NONE_SECCOMP`, if any (`_start` does,
/// before `main`). A program built to run sandboxed does not run without its
/// sandbox: failure aborts.
pub fn install() {
    if FILTER.is_empty() {
        return;
    }
    let prog = SockFprog { len: FILTER.len() as u16, filter: FILTER.as_ptr() };
    let r = unsafe { sys::prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) }
        .and_then(|_| unsafe { sys::syscall(crate::nr::SECCOMP, [SECCOMP_SET_MODE_FILTER, 0, &raw const prog as usize, 0, 0, 0]) });
    if r.is_err() {
        crate::start::write_stderr(b"linux_none_rt: could not install the LINUX_NONE_SECCOMP filter\n");
        crate::start::abort();
    }
}
