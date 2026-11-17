//! id_lookup (tex.web §259) with an auxiliary index over the control-sequence names.
//!
//! web2c's TeX hashes names into hash_prime = 8501 buckets, but a LaTeX format holds
//! tens of thousands of names, so every lookup walks a chain of several slots scattered
//! over megabytes (about 7% of building pdflatex.fmt). The index finds a name's slot
//! directly, with exactly TeX's result:
//! - a name occurs at most once in the chains (TeX inserts only names it did not find,
//!   and the pool strings of names are never changed), so a hit is the slot TeX's walk
//!   would return;
//! - a miss returns undefined_control_sequence when no_new_control_sequence is set, as
//!   TeX does, and otherwise runs the generated id_lookup (emitted as idlookup_full),
//!   which inserts the name into TeX's table in the usual way; the new slot is then added
//!   to the index.
//! The index is built from the hash table only after INDEX_AFTER lookups (short runs
//! never pay for it). Debug builds cross-check every answer with the generated routine.

use super::*;
use crate::gen::globals::Globals;

const INDEX_AFTER: u32 = 50_000;
const HASH_BASE: i32 = 514;
const HASH_SIZE: i32 = 15000;
const EQTB_SIZE: i32 = 30192;
const UNDEFINED_CONTROL_SEQUENCE: i32 = 26627;

#[derive(Default)]
pub struct CsIndex {
    /// lookups so far, until the index is built
    count: u32,
    built: bool,
    /// open addressing: (fx hash of the name, hash slot p); p = 0 is empty
    table: Vec<(u32, i32)>,
    used: usize,
}

#[inline]
fn fx(key: &[u8]) -> u32 {
    const K: u64 = 0x517c_c1b7_2722_0a95;
    let mut h: u64 = key.len() as u64;
    let mut chunks = key.chunks_exact(8);
    for c in &mut chunks {
        h = (h.rotate_left(5) ^ u64::from_le_bytes(c.try_into().unwrap())).wrapping_mul(K);
    }
    let r = chunks.remainder();
    if !r.is_empty() {
        let mut b = [0u8; 8];
        b[..r.len()].copy_from_slice(r);
        h = (h.rotate_left(5) ^ u64::from_le_bytes(b)).wrapping_mul(K);
    }
    ((h ^ (h >> 29)).wrapping_mul(K) >> 32) as u32
}

impl CsIndex {
    fn insert_raw(&mut self, h: u32, p: i32) {
        if (self.used + 1) * 2 > self.table.len() {
            self.grow();
        }
        let mask = self.table.len() - 1;
        let mut i = h as usize & mask;
        while self.table[i].1 != 0 {
            i = (i + 1) & mask;
        }
        self.table[i] = (h, p);
        self.used += 1;
    }

    fn grow(&mut self) {
        let n = (self.table.len() * 2).max(1 << 16);
        let old = std::mem::replace(&mut self.table, vec![(0, 0); n]);
        self.used = 0;
        for (h, p) in old {
            if p != 0 {
                self.insert_raw(h, p);
            }
        }
    }
}

impl Globals {
    /// The bytes of the name of hash slot p (text(p) in the string pool).
    #[inline]
    fn cs_name_bytes(&self, p: i32) -> &[u8] {
        let t = ix(self.hash, p).rh();
        let a = *ix(self.strstart, t);
        let b = *ix(self.strstart, t + 1);
        slice(self.strpool, a, (b - a) as usize)
    }

    fn build_cs_index(&mut self) {
        let mut idx = std::mem::take(&mut self.rt.csindex);
        let ranges = [(HASH_BASE, HASH_BASE + HASH_SIZE), (EQTB_SIZE + 1, EQTB_SIZE + 1 + self.hashhigh)];
        for (lo, hi) in ranges {
            for p in lo..hi {
                if ix(self.hash, p).rh() > 0 {
                    let h = fx(self.cs_name_bytes(p));
                    idx.insert_raw(h, p);
                }
            }
        }
        idx.built = true;
        self.rt.csindex = idx;
    }

    /// tex.web id_lookup(j, l): the hash slot of the name buffer[j..j+l-1].
    pub fn idlookup(&mut self, j: i32, l: i32) -> i32 {
        if !self.rt.csindex.built && self.rt.csindex.count >= INDEX_AFTER {
            // from now on every name is in the index, so misses are answered too
            self.rt.csindex = CsIndex::default();
            self.build_cs_index();
        }
        if self.rt.csindex.table.is_empty() {
            self.rt.csindex.grow();
        }
        let name = slice(self.buffer, j, l.max(0) as usize);
        let h = fx(name);
        let found = {
            let t = &self.rt.csindex.table;
            let mask = t.len() - 1;
            let mut i = h as usize & mask;
            let mut found = None;
            loop {
                let (eh, p) = t[i];
                if p == 0 {
                    break;
                }
                if eh == h && self.cs_name_bytes(p) == name {
                    found = Some(p);
                    break;
                }
                i = (i + 1) & mask;
            }
            found
        };
        if let Some(p) = found {
            debug_assert_eq!(p, self.idlookup_full(j, l));
            return p;
        }
        if self.rt.csindex.built {
            if self.nonewcontrolsequence {
                debug_assert_eq!(UNDEFINED_CONTROL_SEQUENCE, self.idlookup_full(j, l));
                return UNDEFINED_CONTROL_SEQUENCE;
            }
            let p = self.idlookup_full(j, l);
            self.rt.csindex.insert_raw(h, p);
            return p;
        }
        // before the full index: TeX's own lookup, and remember every name it finds or
        // creates (names never move, so later lookups of it can skip the hash chain)
        self.rt.csindex.count += 1;
        let p = self.idlookup_full(j, l);
        if p != UNDEFINED_CONTROL_SEQUENCE {
            self.rt.csindex.insert_raw(h, p);
        }
        p
    }
}
