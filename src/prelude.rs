//! The prelude: `use std::prelude::*;` brings in what std's implicit prelude
//! provides (beyond what `core`'s prelude already gives a `no_std` crate).

pub use alloc_crate::borrow::ToOwned;
pub use alloc_crate::boxed::Box;
pub use alloc_crate::string::{String, ToString};
pub use alloc_crate::vec::Vec;

pub use crate::{dbg, eprint, eprintln, format, print, println, thread_local, vec};

pub mod v1 {
    pub use super::*;
}
pub mod rust_2015 {
    pub use super::*;
}
pub mod rust_2018 {
    pub use super::*;
}
pub mod rust_2021 {
    pub use super::*;
}
pub mod rust_2024 {
    pub use super::*;
}
