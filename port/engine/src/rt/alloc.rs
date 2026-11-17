//! The engine's global allocator: size-class free lists carved from large OS chunks.
//!
//! The Rust side of the engine (kpathsea, map files, fonts, the PDF writer, ...) makes
//! many small, short-lived allocations, and the Windows process heap costs roughly
//! 100 ns for each malloc/free pair. A TeX run is one short-lived process, so a simple
//! segregated-fit allocator that never returns small blocks to the OS is both much faster
//! and safe: freed blocks go on a per-size free list and are reused.
//!
//! Requests above MAX_SMALL bytes, or aligned to more than 16 bytes, go to the system
//! allocator. Which allocator owns a block is decided by its Layout alone, so dealloc
//! (which gets the same Layout) always returns a block to the allocator it came from.
//! A spin lock guards the state: the engine thread does nearly all allocation, so the
//! lock is practically never contended. Nothing in here allocates.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, Ordering};

const CLASSES: [usize; 40] = [
    16, 32, 48, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 448, 512, 640, 768, 896, 1024, 1280, 1536,
    1792, 2048, 2560, 3072, 3584, 4096, 5120, 6144, 7168, 8192, 10240, 12288, 14336, 16384, 20480, 24576, 28672,
    32768,
];
const MAX_SMALL: usize = 32768;
const CHUNK: usize = 4 << 20;

/// Size class index for sizes 1..=1024 in 16-byte steps (index = (size - 1) / 16).
const SMALL_INDEX: [u8; 64] = {
    let mut t = [0u8; 64];
    let mut i = 0;
    while i < 64 {
        let size = (i + 1) * 16;
        let mut c = 0;
        while CLASSES[c] < size {
            c += 1;
        }
        t[i] = c as u8;
        i += 1;
    }
    t
};

#[inline(always)]
fn class_of(size: usize) -> usize {
    if size <= 1024 {
        SMALL_INDEX[(size.max(1) - 1) >> 4] as usize
    } else {
        let mut c = 19; // CLASSES[19] = 1024
        while CLASSES[c] < size {
            c += 1;
        }
        c
    }
}

struct State {
    free: [*mut u8; CLASSES.len()],
    bump: usize,
    bump_end: usize,
}

pub struct TexAlloc {
    lock: AtomicBool,
    st: UnsafeCell<State>,
}

unsafe impl Sync for TexAlloc {}

impl TexAlloc {
    pub const fn new() -> TexAlloc {
        TexAlloc {
            lock: AtomicBool::new(false),
            st: UnsafeCell::new(State { free: [std::ptr::null_mut(); CLASSES.len()], bump: 0, bump_end: 0 }),
        }
    }

    #[inline(always)]
    fn lock(&self) {
        while self.lock.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            while self.lock.load(Ordering::Relaxed) {
                std::hint::spin_loop();
            }
        }
    }

    #[inline(always)]
    fn unlock(&self) {
        self.lock.store(false, Ordering::Release);
    }

    #[inline(always)]
    fn small(layout: &Layout) -> bool {
        layout.size() <= MAX_SMALL && layout.align() <= 16
    }

    #[inline(always)]
    unsafe fn alloc_small(&self, size: usize) -> *mut u8 {
        let c = class_of(size);
        self.lock();
        let st = &mut *self.st.get();
        let p = st.free[c];
        if !p.is_null() {
            st.free[c] = *(p as *mut *mut u8);
            self.unlock();
            return p;
        }
        let sz = CLASSES[c];
        if st.bump + sz > st.bump_end {
            let chunk = os_alloc(CHUNK);
            if chunk.is_null() {
                self.unlock();
                return std::ptr::null_mut();
            }
            st.bump = chunk as usize;
            st.bump_end = chunk as usize + CHUNK;
        }
        let p = st.bump as *mut u8;
        st.bump += sz;
        self.unlock();
        p
    }

    #[inline(always)]
    unsafe fn free_small(&self, p: *mut u8, size: usize) {
        let c = class_of(size);
        self.lock();
        let st = &mut *self.st.get();
        *(p as *mut *mut u8) = st.free[c];
        st.free[c] = p;
        self.unlock();
    }
}

unsafe impl GlobalAlloc for TexAlloc {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if Self::small(&layout) {
            self.alloc_small(layout.size())
        } else {
            System.alloc(layout)
        }
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if Self::small(&layout) {
            let p = self.alloc_small(layout.size());
            if !p.is_null() {
                std::ptr::write_bytes(p, 0, layout.size());
            }
            p
        } else {
            System.alloc_zeroed(layout)
        }
    }

    #[inline]
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        if Self::small(&layout) {
            self.free_small(p, layout.size());
        } else {
            System.dealloc(p, layout)
        }
    }

    #[inline]
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_layout = Layout::from_size_align_unchecked(new_size, layout.align());
        let old_small = Self::small(&layout);
        let new_small = Self::small(&new_layout);
        if old_small && new_small && class_of(layout.size()) == class_of(new_size) {
            return p;
        }
        if !old_small && !new_small {
            return System.realloc(p, layout, new_size);
        }
        let q = self.alloc(new_layout);
        if !q.is_null() {
            std::ptr::copy_nonoverlapping(p, q, layout.size().min(new_size));
            self.dealloc(p, layout);
        }
        q
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn VirtualAlloc(addr: *mut u8, size: usize, kind: u32, protect: u32) -> *mut u8;
}

unsafe fn os_alloc(bytes: usize) -> *mut u8 {
    VirtualAlloc(std::ptr::null_mut(), bytes, 0x1000 | 0x2000, 0x04)
}
