//! A small mmap-backed allocator: power-of-two size classes with free lists
//! for blocks up to 32 KiB, direct `mmap` for everything larger.
//!
//! The free lists and the bump region are one heap for the process, behind
//! a lock. On linux-none each thread keeps a cache of free blocks per class
//! in front of it, so that most allocations and frees take no lock: a
//! thread refills an empty class, and hands back half of a full one, a
//! batch at a time under one lock. A thread's cache goes back to the heap
//! when it exits ([`flush_thread_cache`]).

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::ptr::null_mut;

use crate::sys::{self as linux, MAP_ANONYMOUS, MAP_PRIVATE, PAGE_SIZE, PROT_READ, PROT_WRITE};
use crate::locks::RawMutex;

const MIN_SHIFT: u32 = 4; // 16 bytes
const MAX_SHIFT: u32 = 15; // 32 KiB
const NCLASSES: usize = (MAX_SHIFT - MIN_SHIFT + 1) as usize;
const CHUNK: usize = 1 << 20;

/// A handle on the process's heap (std's global allocator and the C
/// library's `malloc` share it).
pub struct Allocator;

struct Heap {
    lock: RawMutex,
    state: UnsafeCell<State>,
}

struct State {
    free: [*mut u8; NCLASSES],
    bump: usize,
    end: usize,
}

unsafe impl Sync for Heap {}

static HEAP: Heap = Heap {
    lock: RawMutex::new(),
    state: UnsafeCell::new(State { free: [null_mut(); NCLASSES], bump: 0, end: 0 }),
};

/// Size class index for a layout, or None if it goes straight to mmap.
#[inline]
fn class_of(layout: &Layout) -> Option<usize> {
    let size = layout.size().max(layout.align()).max(1 << MIN_SHIFT);
    if size > 1 << MAX_SHIFT || layout.align() > PAGE_SIZE {
        return None;
    }
    let shift = usize::BITS - (size - 1).leading_zeros();
    Some((shift - MIN_SHIFT) as usize)
}

#[inline]
fn round_up(n: usize, to: usize) -> usize {
    (n + to - 1) & !(to - 1)
}

unsafe fn map(len: usize) -> *mut u8 {
    match unsafe { linux::mmap(0, len, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0) } {
        Ok(p) => p as *mut u8,
        Err(_) => null_mut(),
    }
}

/// Blocks a thread caches of class `class`: 64 KiB worth, 4 to 64 of them.
#[cfg_attr(not(linux_none), allow(dead_code))]
const fn cache_limit(class: usize) -> usize {
    let n = (64 << 10) >> (class as u32 + MIN_SHIFT);
    if n < 4 { 4 } else if n > 64 { 64 } else { n }
}

impl State {
    /// A block of `class` from the free list or the bump region (null if
    /// out of memory). The heap's lock must be held.
    unsafe fn take(&mut self, class: usize) -> *mut u8 {
        let size = 1usize << (class as u32 + MIN_SHIFT);
        let head = self.free[class];
        if !head.is_null() {
            self.free[class] = unsafe { *(head as *mut *mut u8) };
            return head;
        }
        // every block of a class is aligned to min(size, page) so that any
        // layout mapping to the class is satisfied by any of its blocks
        let align = size.min(PAGE_SIZE);
        let mut start = round_up(self.bump, align);
        if self.bump == 0 || start + size > self.end {
            let chunk = unsafe { map(CHUNK) } as usize;
            if chunk == 0 {
                return null_mut();
            }
            self.bump = chunk;
            self.end = chunk + CHUNK;
            start = chunk;
        }
        self.bump = start + size;
        start as *mut u8
    }

    /// Put `p` on `class`'s free list. The heap's lock must be held.
    unsafe fn put(&mut self, class: usize, p: *mut u8) {
        unsafe { *(p as *mut *mut u8) = self.free[class] };
        self.free[class] = p;
    }
}

impl Heap {
    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        self.lock.lock();
        let r = f(unsafe { &mut *self.state.get() });
        unsafe { self.lock.unlock() };
        r
    }
}

/// A thread's free blocks, per class: a list linked through the blocks.
#[cfg(linux_none)]
struct Cache {
    free: [*mut u8; NCLASSES],
    count: [usize; NCLASSES],
}

#[cfg(linux_none)]
#[thread_local]
static mut CACHE: Cache = Cache { free: [null_mut(); NCLASSES], count: [0; NCLASSES] };

#[cfg(linux_none)]
fn cache() -> &'static mut Cache {
    unsafe { &mut *(&raw mut CACHE) }
}

#[cfg(linux_none)]
unsafe fn alloc_small(class: usize) -> *mut u8 {
    let c = cache();
    let p = c.free[class];
    if !p.is_null() {
        c.free[class] = unsafe { *(p as *mut *mut u8) };
        c.count[class] -= 1;
        return p;
    }
    // refill: one block to return, half the limit more for the cache
    HEAP.with(|st| unsafe {
        let p = st.take(class);
        for _ in 0..cache_limit(class) / 2 {
            let q = st.take(class);
            if q.is_null() {
                break;
            }
            *(q as *mut *mut u8) = c.free[class];
            c.free[class] = q;
            c.count[class] += 1;
        }
        p
    })
}

#[cfg(linux_none)]
unsafe fn dealloc_small(class: usize, p: *mut u8) {
    let c = cache();
    unsafe { *(p as *mut *mut u8) = c.free[class] };
    c.free[class] = p;
    c.count[class] += 1;
    if c.count[class] > cache_limit(class) {
        // hand half back
        HEAP.with(|st| unsafe {
            for _ in 0..cache_limit(class) / 2 {
                let q = c.free[class];
                c.free[class] = *(q as *mut *mut u8);
                c.count[class] -= 1;
                st.put(class, q);
            }
        });
    }
}

/// Return the calling thread's cached blocks to the heap (as it exits).
pub fn flush_thread_cache() {
    #[cfg(linux_none)]
    {
        let c = cache();
        if c.count.iter().all(|&n| n == 0) {
            return;
        }
        HEAP.with(|st| unsafe {
            for class in 0..NCLASSES {
                while !c.free[class].is_null() {
                    let q = c.free[class];
                    c.free[class] = *(q as *mut *mut u8);
                    st.put(class, q);
                }
                c.count[class] = 0;
            }
        });
    }
}

// elsewhere (host tests, Kani): straight to the heap
#[cfg(not(linux_none))]
unsafe fn alloc_small(class: usize) -> *mut u8 {
    HEAP.with(|st| unsafe { st.take(class) })
}

#[cfg(not(linux_none))]
unsafe fn dealloc_small(class: usize, p: *mut u8) {
    HEAP.with(|st| unsafe { st.put(class, p) })
}

impl Allocator {
    pub const fn new() -> Self {
        Allocator
    }

    unsafe fn alloc_large(layout: Layout) -> *mut u8 {
        let len = round_up(layout.size(), PAGE_SIZE);
        if layout.align() <= PAGE_SIZE {
            return unsafe { map(len) };
        }
        // over-allocate, then trim to the requested alignment
        let total = len + layout.align();
        let base = unsafe { map(total) } as usize;
        if base == 0 {
            return null_mut();
        }
        let start = round_up(base, layout.align());
        unsafe {
            if start > base {
                let _ = linux::munmap(base, start - base);
            }
            let tail = start + len;
            if base + total > tail {
                let _ = linux::munmap(tail, base + total - tail);
            }
        }
        start as *mut u8
    }
}

unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        match class_of(&layout) {
            Some(c) => unsafe { alloc_small(c) },
            None => unsafe { Self::alloc_large(layout) },
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        match class_of(&layout) {
            Some(c) => unsafe { dealloc_small(c, ptr) },
            None => unsafe {
                let _ = linux::munmap(ptr as usize, round_up(layout.size(), PAGE_SIZE));
            },
        }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        match class_of(&layout) {
            Some(c) => {
                let p = unsafe { alloc_small(c) };
                if !p.is_null() {
                    unsafe { core::ptr::write_bytes(p, 0, layout.size()) };
                }
                p
            }
            // fresh anonymous mappings are already zeroed
            None => unsafe { Self::alloc_large(layout) },
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
        let old = class_of(&layout);
        if old.is_some() && old == class_of(&new_layout) {
            return ptr;
        }
        if old.is_none() && class_of(&new_layout).is_none() && layout.align() <= PAGE_SIZE
            && round_up(layout.size(), PAGE_SIZE) == round_up(new_size, PAGE_SIZE)
        {
            return ptr;
        }
        let new = unsafe { self.alloc(new_layout) };
        if !new.is_null() {
            unsafe {
                core::ptr::copy_nonoverlapping(ptr, new, layout.size().min(new_size));
                self.dealloc(ptr, layout);
            }
        }
        new
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// Every layout either goes to mmap or gets a size class whose blocks are
    /// big enough and aligned enough (blocks of class `c` are aligned to
    /// min(block size, page)), and no smaller class would do.
    #[kani::proof]
    fn size_class_fits_layout() {
        let size: usize = kani::any_where(|&s| s <= 1 << 40);
        let shift: u32 = kani::any_where(|&k| k <= 20);
        let layout = Layout::from_size_align(size, 1 << shift).unwrap();
        let need = size.max(layout.align()).max(1 << MIN_SHIFT);
        match class_of(&layout) {
            Some(c) => {
                assert!(c < NCLASSES);
                let block = 1usize << (c as u32 + MIN_SHIFT);
                assert!(block >= need);
                assert!(block.min(PAGE_SIZE) % layout.align() == 0);
                assert!(c == 0 || block / 2 < need);
            }
            None => assert!(need > 1 << MAX_SHIFT || layout.align() > PAGE_SIZE),
        }
    }

    /// `realloc` keeps the block when the class is unchanged: then the block
    /// must also hold the new size.
    #[kani::proof]
    fn same_class_realloc_fits() {
        let (old, new): (usize, usize) = (kani::any_where(|&s| s <= 1 << 20), kani::any_where(|&s| s <= 1 << 20));
        let align = 1usize << kani::any_where(|&k: &u32| k <= 13);
        let (a, b) = (Layout::from_size_align(old, align).unwrap(), Layout::from_size_align(new, align).unwrap());
        if let (Some(c), Some(d)) = (class_of(&a), class_of(&b)) {
            if c == d {
                assert!(1usize << (c as u32 + MIN_SHIFT) >= new);
            }
        }
    }

    /// A thread caches 4 to 64 blocks of each class, and a refill or a
    /// hand-back moves at least two.
    #[kani::proof]
    fn cache_limits() {
        let c: usize = kani::any_where(|&c| c < NCLASSES);
        assert!((4..=64).contains(&cache_limit(c)) && cache_limit(c) / 2 >= 2);
    }

    #[kani::proof]
    fn round_up_is_least_multiple() {
        let n: usize = kani::any_where(|&n| n <= usize::MAX / 2);
        let to = 1usize << kani::any_where(|&k: &u32| k <= 12);
        let r = round_up(n, to);
        assert!(r >= n && r % to == 0 && r - n < to);
    }
}
