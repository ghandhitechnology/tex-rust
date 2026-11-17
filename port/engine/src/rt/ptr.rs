//! C-style pointers for the translated Pascal.
//!
//! web2c turns every Pascal array into a C array or a malloc'ed block and does pointer
//! arithmetic on them (`hash := yhash - hashoffset`, `stringcast(nameoffile+1)`). The
//! generated Rust keeps exactly that model with `Ptr<T>`: a copyable raw pointer.
//! Allocations carry a small header with their size so `free` and `realloc` work.

use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::marker::PhantomData;

#[repr(transparent)]
pub struct Ptr<T> {
    p: *mut T,
    _m: PhantomData<T>,
}

impl<T> Clone for Ptr<T> {
    #[inline(always)]
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Ptr<T> {}
impl<T> Default for Ptr<T> {
    fn default() -> Self {
        Self::null()
    }
}
impl<T> std::fmt::Debug for Ptr<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Ptr({:p})", self.p)
    }
}

const HDR: usize = 16;

impl<T> Ptr<T> {
    #[inline(always)]
    pub const fn null() -> Self {
        Ptr { p: std::ptr::null_mut(), _m: PhantomData }
    }
    #[inline(always)]
    pub fn from_raw(p: *mut T) -> Self {
        Ptr { p, _m: PhantomData }
    }
    #[inline(always)]
    pub fn raw(self) -> *mut T {
        self.p
    }
    #[inline(always)]
    pub fn is_null(self) -> bool {
        self.p.is_null()
    }
    #[inline(always)]
    pub fn addr(self) -> usize {
        self.p as usize
    }
    /// Pointer arithmetic in elements (C `p + k`).
    #[inline(always)]
    pub fn off(self, k: i32) -> Self {
        Ptr { p: self.p.wrapping_offset(k as isize), _m: PhantomData }
    }
    #[inline(always)]
    pub fn cast<U>(self) -> Ptr<U> {
        Ptr { p: self.p as *mut U, _m: PhantomData }
    }

    fn alloc_bytes(bytes: usize) -> *mut u8 {
        unsafe {
            let base = if bytes + HDR >= LARGE {
                // TeX's big arrays (mem, font_info, the trie, ...) are sized from texmf.cnf
                // and mostly never touched in a run: fresh pages from the OS are zero, cost
                // nothing until touched, and need no explicit zeroing (which the process
                // heap would do for alloc_zeroed)
                let p = os::alloc(bytes + HDR);
                if !p.is_null() {
                    (p as *mut usize).add(1).write(KIND_OS);
                }
                p
            } else {
                let p = alloc_zeroed(Layout::from_size_align(bytes + HDR, 16).expect("allocation too large"));
                if !p.is_null() {
                    (p as *mut usize).add(1).write(KIND_HEAP);
                }
                p
            };
            if base.is_null() {
                crate::rt::texmfmp::fatal("memory exhausted");
            }
            (base as *mut usize).write(bytes);
            base.add(HDR)
        }
    }

    /// `count` zeroed elements (static Pascal arrays; web2c's C arrays are zero-initialized).
    pub fn alloc_count(count: i64) -> Self {
        let bytes = (count.max(1) as usize) * std::mem::size_of::<T>().max(1);
        Ptr { p: Self::alloc_bytes(bytes) as *mut T, _m: PhantomData }
    }

    /// web2c `xmallocarray(type, size)`: room for size+1 elements.
    pub fn xmalloc_array(size: i32) -> Self {
        Self::alloc_count(size as i64 + 1)
    }

    /// web2c `xreallocarray(ptr, type, size)`: size+1 elements, old contents kept.
    pub fn xrealloc_array(self, size: i32) -> Self {
        let new = Self::alloc_count(size as i64 + 1);
        if !self.p.is_null() {
            unsafe {
                let old_bytes = *((self.p as *mut u8).sub(HDR) as *mut usize);
                let new_bytes = (size as usize + 1) * std::mem::size_of::<T>();
                std::ptr::copy_nonoverlapping(self.p as *const u8, new.p as *mut u8, old_bytes.min(new_bytes));
            }
            self.free();
        }
        new
    }

    /// C `free`. Only valid on pointers returned by the allocators above.
    pub fn free(self) {
        if self.p.is_null() {
            return;
        }
        unsafe {
            let base = (self.p as *mut u8).sub(HDR);
            let bytes = *(base as *mut usize);
            if *(base as *mut usize).add(1) == KIND_OS {
                os::free(base);
            } else {
                dealloc(base, Layout::from_size_align_unchecked(bytes + HDR, 16));
            }
        }
    }
}

/// Allocations of at least this many bytes come straight from the OS (see alloc_bytes).
const LARGE: usize = 1 << 20;
const KIND_HEAP: usize = 0x6865_6170;
const KIND_OS: usize = 0x6f73_6f73;

mod os {
    #[link(name = "kernel32")]
    extern "system" {
        fn VirtualAlloc(addr: *mut u8, size: usize, kind: u32, protect: u32) -> *mut u8;
        fn VirtualFree(addr: *mut u8, size: usize, kind: u32) -> i32;
    }
    const MEM_COMMIT: u32 = 0x1000;
    const MEM_RESERVE: u32 = 0x2000;
    const MEM_RELEASE: u32 = 0x8000;
    const PAGE_READWRITE: u32 = 0x04;

    /// Zero-filled, page-aligned memory (null on failure).
    pub unsafe fn alloc(bytes: usize) -> *mut u8 {
        VirtualAlloc(std::ptr::null_mut(), bytes, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE)
    }
    pub unsafe fn free(p: *mut u8) {
        VirtualFree(p, 0, MEM_RELEASE);
    }
}

impl Ptr<u8> {
    /// A pointer to a NUL-terminated static byte string (never freed or written).
    pub fn cstr_lit(s: &'static [u8]) -> Ptr<u8> {
        debug_assert!(s.last() == Some(&0));
        Ptr { p: s.as_ptr() as *mut u8, _m: PhantomData }
    }
    /// Copy bytes into a fresh malloc'ed NUL-terminated C string.
    pub fn from_bytes(s: &[u8]) -> Ptr<u8> {
        let p = Self::alloc_count(s.len() as i64 + 1);
        unsafe { std::ptr::copy_nonoverlapping(s.as_ptr(), p.p, s.len()) };
        p
    }
    /// The bytes of a NUL-terminated C string (without the NUL).
    pub fn cstr_bytes<'a>(self) -> &'a [u8] {
        if self.p.is_null() {
            return &[];
        }
        unsafe {
            let mut n = 0;
            while *self.p.add(n) != 0 {
                n += 1;
            }
            std::slice::from_raw_parts(self.p, n)
        }
    }
}

/// Element access, C semantics (no bounds check in release builds).
#[inline(always)]
pub fn ix<T>(p: Ptr<T>, i: i32) -> &'static mut T {
    #[cfg(debug_assertions)]
    if p.p.is_null() {
        panic!("index {} into a null Ptr<{}>", i, std::any::type_name::<T>());
    }
    unsafe { &mut *p.p.offset(i as isize) }
}

/// A slice view of `n` elements starting at index `i`.
pub fn slice<'a, T>(p: Ptr<T>, i: i32, n: usize) -> &'a mut [T] {
    unsafe { std::slice::from_raw_parts_mut(p.p.offset(i as isize), n) }
}
