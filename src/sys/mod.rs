//! Platform layer: the syscall runtime is `linux_none_rt`; this adds the
//! io::Result helpers and the slot-based thread locals rust_std still uses.

pub use linux_none_rt::{alloc, arch, futex, locks, nr};
pub use linux_none_rt::sys as linux;

pub mod fd;
pub mod path;
pub mod tls;
