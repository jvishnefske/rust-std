//! Memory allocation APIs, as in `std::alloc`.

pub use alloc_crate::alloc::{
    GlobalAlloc, Layout, LayoutError, alloc, alloc_zeroed, dealloc, handle_alloc_error, realloc,
};

/// The runtime's own allocator (`mmap`-backed size classes). In std, `System`
/// is the platform allocator, which on Linux means libc `malloc`; here it is
/// the allocator rust_std ships instead.
#[derive(Debug, Default, Clone, Copy)]
pub struct System;

static SYSTEM: crate::sys::alloc::Allocator = crate::sys::alloc::Allocator::new();

unsafe impl GlobalAlloc for System {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { SYSTEM.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { SYSTEM.dealloc(ptr, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        unsafe { SYSTEM.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        unsafe { SYSTEM.realloc(ptr, layout, new_size) }
    }
}
