//! hash.c: kpathsea's chained hash table, reproduced bucket-for-bucket.
//!
//! The exact bucket layout matters for behaviour (not just speed) in two
//! places: keys inserted with `hash_insert_normalized` (ls-R aliases,
//! texfonts.map) are hashed *without* case folding but looked up *with*
//! it, so an alias containing upper-case letters is only found when the
//! two hash values happen to agree.  We reproduce that by keeping C's
//! buckets.  Look-ups return all values for the key in insertion order.

use std::sync::OnceLock;

use crate::sys;

static HIGH: OnceLock<[u8; 128]> = OnceLock::new();

/// The `TRANSFORM` macro of config.h: `CharLower` on a single byte.
/// ASCII is lower-cased directly; the high half is computed once by
/// calling `CharLowerA` (only if a non-ASCII byte is ever transformed).
#[inline]
pub(crate) fn transform(c: u8) -> u8 {
    if c < 0x80 {
        c.to_ascii_lowercase()
    } else {
        HIGH.get_or_init(sys::charlower_high_table)[(c - 0x80) as usize]
    }
}

/// `hash()`: uses `TRANSFORM`, unsigned bytes.
#[inline]
pub(crate) fn c_hash(size: u32, key: &[u8]) -> u32 {
    let mut n: u32 = 0;
    for &c in key {
        n = (n + n + transform(c) as u32) % size;
    }
    n
}

/// `hash_normalized()`: no `TRANSFORM`, and `char` is signed with MSVC.
#[inline]
pub(crate) fn c_hash_normalized(size: u32, key: &[u8]) -> u32 {
    let mut n: u32 = 0;
    for &c in key {
        n = n.wrapping_add(n).wrapping_add(c as i8 as i32 as u32) % size;
    }
    n
}

pub(crate) struct CHash {
    size: u32,
    buckets: Vec<Vec<(Vec<u8>, Vec<u8>)>>,
}

impl CHash {
    /// `hash_create`.
    pub(crate) fn new(size: u32) -> Self {
        CHash { size, buckets: (0..size).map(|_| Vec::new()).collect() }
    }

    /// `hash_insert`.
    pub(crate) fn insert(&mut self, key: Vec<u8>, value: Vec<u8>) {
        let n = c_hash(self.size, &key) as usize;
        self.buckets[n].push((key, value));
    }

    /// `hash_insert_normalized`.
    pub(crate) fn insert_normalized(&mut self, key: Vec<u8>, value: Vec<u8>) {
        let n = c_hash_normalized(self.size, &key) as usize;
        self.buckets[n].push((key, value));
    }

    /// `hash_lookup`: all values whose key matches case-insensitively in
    /// the bucket of `hash(key)`, in insertion order.
    pub(crate) fn lookup(&self, key: &[u8]) -> Vec<&[u8]> {
        let n = c_hash(self.size, key) as usize;
        self.buckets[n]
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_slice())
            .collect()
    }

    /// First value for `key`, without building a list.
    pub(crate) fn lookup_first(&self, key: &[u8]) -> Option<&[u8]> {
        let n = c_hash(self.size, key) as usize;
        self.buckets[n]
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_slice())
    }

    /// `hash_print (table, true)` summary line.
    pub(crate) fn summary(&self) -> Vec<u8> {
        let mut total_buckets = 0u32;
        let mut total_elements = 0u32;
        for b in &self.buckets {
            if !b.is_empty() {
                total_buckets += 1;
                total_elements += b.len() as u32;
            }
        }
        hash_summary_line(self.size, total_buckets, total_elements)
    }
}

/// The summary line printed by `hash_print`.
pub(crate) fn hash_summary_line(size: u32, nonempty: u32, entries: u32) -> Vec<u8> {
    let avg = if nonempty != 0 { entries as f64 / nonempty as f64 } else { 0.0 };
    format!(
        "{} buckets, {} nonempty ({}%); {} entries, average chain {}.\n",
        size,
        nonempty,
        100 * nonempty / size,
        entries,
        crate::debug::fmt_f1(avg)
    )
    .into_bytes()
}
