//! Platform layer: raw Linux syscalls, no libc.

#[cfg(target_arch = "aarch64")]
#[path = "arch/aarch64.rs"]
#[allow(dead_code)]
pub mod arch;
#[cfg(target_arch = "x86_64")]
#[path = "arch/x86_64.rs"]
#[allow(dead_code)]
pub mod arch;
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
compile_error!("rust_std supports x86_64 and aarch64 Linux only");

#[cfg(not(target_os = "linux"))]
compile_error!("rust_std targets Linux only");

pub mod alloc;
pub mod fd;
pub mod futex;
pub mod linux;
pub mod locks;
pub mod path;
pub mod tls;

pub use arch::nr;
