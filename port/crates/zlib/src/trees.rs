//! Port of trees.c (zlib 1.3.2): Huffman tree construction and block output.
//!
//! Function names and control flow follow the C file one-to-one. The
//! tables in `tables.rs` are copied verbatim from trees.h.

use crate::deflate::{DeflateState, D_CODES, HEAP_SIZE, L_CODES, LITERALS, MAX_BITS};
use crate::tables::{BASE_DIST, BASE_LENGTH, DIST_CODE, LENGTH_CODE, STATIC_DTREE, STATIC_LTREE};
use crate::{Z_BINARY, Z_FIXED, Z_TEXT, Z_UNKNOWN};

/// `ct_data`: `fc` is Freq or Code, `dl` is Dad or Len.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct CtData {
    pub fc: u16,
    pub dl: u16,
}

pub(crate) const MAX_BL_BITS: i32 = 7;
pub(crate) const END_BLOCK: usize = 256;
const REP_3_6: usize = 16;
const REPZ_3_10: usize = 17;
const REPZ_11_138: usize = 18;

const STORED_BLOCK: u32 = 0;
const STATIC_TREES: u32 = 1;
const DYN_TREES: u32 = 2;

pub(crate) const BL_CODES: usize = 19;
const BUF_SIZE: i32 = 16;

pub(crate) static EXTRA_LBITS: [i32; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
pub(crate) static EXTRA_DBITS: [i32; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
static EXTRA_BLBITS: [i32; BL_CODES] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 3, 7];
static BL_ORDER: [u8; BL_CODES] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

pub(crate) struct StaticTreeDesc {
    pub static_tree: Option<&'static [CtData]>,
    pub extra_bits: &'static [i32],
    pub extra_base: i32,
    pub elems: i32,
    pub max_length: i32,
}

pub(crate) static STATIC_L_DESC: StaticTreeDesc = StaticTreeDesc {
    static_tree: Some(&STATIC_LTREE),
    extra_bits: &EXTRA_LBITS,
    extra_base: LITERALS as i32 + 1,
    elems: L_CODES as i32,
    max_length: MAX_BITS as i32,
};
pub(crate) static STATIC_D_DESC: StaticTreeDesc = StaticTreeDesc {
    static_tree: Some(&STATIC_DTREE),
    extra_bits: &EXTRA_DBITS,
    extra_base: 0,
    elems: D_CODES as i32,
    max_length: MAX_BITS as i32,
};
pub(crate) static STATIC_BL_DESC: StaticTreeDesc = StaticTreeDesc {
    static_tree: None,
    extra_bits: &EXTRA_BLBITS,
    extra_base: 0,
    elems: BL_CODES as i32,
    max_length: MAX_BL_BITS,
};

/// `d_code(dist)`
#[inline(always)]
pub(crate) fn d_code(dist: u32) -> u8 {
    if dist < 256 {
        DIST_CODE[dist as usize]
    } else {
        DIST_CODE[256 + (dist >> 7) as usize]
    }
}

/// Reverse the first len bits of a code (1 <= len <= 15).
fn bi_reverse(mut code: u32, mut len: i32) -> u32 {
    let mut res: u32 = 0;
    loop {
        res |= code & 1;
        code >>= 1;
        res <<= 1;
        len -= 1;
        if len <= 0 {
            break;
        }
    }
    res >> 1
}

/// Generate the codes for a given tree and bit counts.
fn gen_codes(tree: &mut [CtData], max_code: i32, bl_count: &[u16; MAX_BITS + 1]) {
    let mut next_code = [0u16; MAX_BITS + 1];
    let mut code: u32 = 0;
    for bits in 1..=MAX_BITS {
        code = (code + bl_count[bits - 1] as u32) << 1;
        next_code[bits] = code as u16;
    }
    let mut n = 0i32;
    while n <= max_code {
        let len = tree[n as usize].dl as usize;
        if len != 0 {
            tree[n as usize].fc = bi_reverse(next_code[len] as u32, len as i32) as u16;
            next_code[len] = next_code[len].wrapping_add(1);
        }
        n += 1;
    }
}

/// Which dynamic tree a descriptor refers to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TreeKind {
    L,
    D,
}

/// Heap / bit-length working state shared by all tree builds (fields of
/// `deflate_state` in C).
#[derive(Clone)]
pub(crate) struct TreeWork {
    pub bl_count: [u16; MAX_BITS + 1],
    pub heap: [i32; 2 * L_CODES + 1],
    pub heap_len: i32,
    pub heap_max: i32,
    pub depth: [u8; 2 * L_CODES + 1],
    pub opt_len: u64,
    pub static_len: u64,
}

impl TreeWork {
    pub fn new() -> Self {
        TreeWork {
            bl_count: [0; MAX_BITS + 1],
            heap: [0; 2 * L_CODES + 1],
            heap_len: 0,
            heap_max: 0,
            depth: [0; 2 * L_CODES + 1],
            opt_len: 0,
            static_len: 0,
        }
    }
}

const SMALLEST: usize = 1;

#[inline(always)]
fn smaller(tree: &[CtData], n: i32, m: i32, depth: &[u8]) -> bool {
    let (n, m) = (n as usize, m as usize);
    tree[n].fc < tree[m].fc || (tree[n].fc == tree[m].fc && depth[n] <= depth[m])
}

/// Restore the heap property by moving down the tree starting at node k.
fn pqdownheap(w: &mut TreeWork, tree: &[CtData], mut k: usize) {
    let v = w.heap[k];
    let mut j = k << 1; // left son of k
    while j as i32 <= w.heap_len {
        // Set j to the smallest of the two sons:
        if (j as i32) < w.heap_len && smaller(tree, w.heap[j + 1], w.heap[j], &w.depth) {
            j += 1;
        }
        // Exit if v is smaller than both sons
        if smaller(tree, v, w.heap[j], &w.depth) {
            break;
        }
        // Exchange v with the smallest son
        w.heap[k] = w.heap[j];
        k = j;
        // And continue down the tree, setting j to the left son of k
        j <<= 1;
    }
    w.heap[k] = v;
}

/// Compute the optimal bit lengths for a tree and update the total bit length
/// for the current block.
fn gen_bitlen(w: &mut TreeWork, tree: &mut [CtData], max_code: i32, stat: &StaticTreeDesc) {
    let stree = stat.static_tree;
    let extra = stat.extra_bits;
    let base = stat.extra_base;
    let max_length = stat.max_length;
    let mut overflow = 0i32;

    for bits in 0..=MAX_BITS {
        w.bl_count[bits] = 0;
    }

    // In a first pass, compute the optimal bit lengths (which may overflow in
    // the case of the bit length tree).
    tree[w.heap[w.heap_max as usize] as usize].dl = 0; // root of the heap

    let mut h = w.heap_max + 1;
    while (h as usize) < HEAP_SIZE {
        let n = w.heap[h as usize];
        let mut bits = tree[tree[n as usize].dl as usize].dl as i32 + 1;
        if bits > max_length {
            bits = max_length;
            overflow += 1;
        }
        tree[n as usize].dl = bits as u16;
        // We overwrite tree[n].Dad which is no longer needed

        if n > max_code {
            h += 1;
            continue; // not a leaf node
        }

        w.bl_count[bits as usize] += 1;
        let mut xbits = 0i32;
        if n >= base {
            xbits = extra[(n - base) as usize];
        }
        let f = tree[n as usize].fc;
        w.opt_len = w.opt_len.wrapping_add(f as u64 * (bits + xbits) as u32 as u64);
        if let Some(st) = stree {
            w.static_len = w
                .static_len
                .wrapping_add(f as u64 * (st[n as usize].dl as i32 + xbits) as u32 as u64);
        }
        h += 1;
    }
    if overflow == 0 {
        return;
    }

    // Find the first bit length which could increase:
    loop {
        let mut bits = max_length - 1;
        while w.bl_count[bits as usize] == 0 {
            bits -= 1;
        }
        w.bl_count[bits as usize] -= 1; // move one leaf down the tree
        w.bl_count[bits as usize + 1] += 2; // move one overflow item as its brother
        w.bl_count[max_length as usize] = w.bl_count[max_length as usize].wrapping_sub(1);
        // The brother of the overflow item also moves one step up, but this
        // does not affect bl_count[max_length]
        overflow -= 2;
        if overflow <= 0 {
            break;
        }
    }

    // Now recompute all bit lengths, scanning in increasing frequency. h is
    // still equal to HEAP_SIZE.
    let mut bits = max_length;
    while bits != 0 {
        let mut n = w.bl_count[bits as usize] as i32;
        while n != 0 {
            h -= 1;
            let m = w.heap[h as usize];
            if m > max_code {
                continue;
            }
            if tree[m as usize].dl as u32 != bits as u32 {
                w.opt_len = w.opt_len.wrapping_add(
                    ((bits as u64).wrapping_sub(tree[m as usize].dl as u64))
                        .wrapping_mul(tree[m as usize].fc as u64),
                );
                tree[m as usize].dl = bits as u16;
            }
            n -= 1;
        }
        bits -= 1;
    }
}

/// Construct one Huffman tree and assign the code bit strings and lengths.
/// Returns the new max_code.
fn build_tree(w: &mut TreeWork, tree: &mut [CtData], stat: &StaticTreeDesc) -> i32 {
    let stree = stat.static_tree;
    let elems = stat.elems;
    let mut max_code: i32 = -1;

    // Construct the initial heap, with least frequent element in
    // heap[SMALLEST]. heap[0] is not used.
    w.heap_len = 0;
    w.heap_max = HEAP_SIZE as i32;

    for n in 0..elems {
        if tree[n as usize].fc != 0 {
            w.heap_len += 1;
            max_code = n;
            w.heap[w.heap_len as usize] = n;
            w.depth[n as usize] = 0;
        } else {
            tree[n as usize].dl = 0;
        }
    }

    // The pkzip format requires that at least one distance code exists, and
    // that at least one bit should be sent even if there is only one possible
    // code. So to avoid special checks later on we force at least two codes of
    // non zero frequency.
    while w.heap_len < 2 {
        let node = if max_code < 2 {
            max_code += 1;
            max_code
        } else {
            0
        };
        w.heap_len += 1;
        w.heap[w.heap_len as usize] = node;
        tree[node as usize].fc = 1;
        w.depth[node as usize] = 0;
        w.opt_len = w.opt_len.wrapping_sub(1);
        if let Some(st) = stree {
            w.static_len = w.static_len.wrapping_sub(st[node as usize].dl as u64);
        }
        // node is 0 or 1 so it does not have extra bits
    }

    // The elements heap[heap_len/2 + 1 .. heap_len] are leaves of the tree,
    // establish sub-heaps of increasing lengths:
    let mut n = w.heap_len / 2;
    while n >= 1 {
        pqdownheap(w, tree, n as usize);
        n -= 1;
    }

    // Construct the Huffman tree by repeatedly combining the least two
    // frequent nodes.
    let mut node = elems; // next internal node of the tree
    loop {
        // pqremove(s, tree, n)
        let n = w.heap[SMALLEST];
        w.heap[SMALLEST] = w.heap[w.heap_len as usize];
        w.heap_len -= 1;
        pqdownheap(w, tree, SMALLEST);
        let m = w.heap[SMALLEST]; // m = node of next least frequency

        w.heap_max -= 1;
        w.heap[w.heap_max as usize] = n; // keep the nodes sorted by frequency
        w.heap_max -= 1;
        w.heap[w.heap_max as usize] = m;

        // Create a new node father of n and m
        tree[node as usize].fc = tree[n as usize].fc.wrapping_add(tree[m as usize].fc);
        let dn = w.depth[n as usize];
        let dm = w.depth[m as usize];
        w.depth[node as usize] = (if dn >= dm { dn } else { dm }).wrapping_add(1);
        tree[n as usize].dl = node as u16;
        tree[m as usize].dl = node as u16;
        // and insert the new node in the heap
        w.heap[SMALLEST] = node;
        node += 1;
        pqdownheap(w, tree, SMALLEST);

        if w.heap_len < 2 {
            break;
        }
    }

    w.heap_max -= 1;
    w.heap[w.heap_max as usize] = w.heap[SMALLEST];

    // At this point, the fields freq and dad are set. We can now generate the
    // bit lengths.
    gen_bitlen(w, tree, max_code, stat);

    // The field len is now set, we can generate the bit codes
    let bl_count = w.bl_count;
    gen_codes(tree, max_code, &bl_count);
    max_code
}

/// Scan a literal or distance tree to determine the frequencies of the codes
/// in the bit length tree.
fn scan_tree(bl_tree: &mut [CtData], tree: &mut [CtData], max_code: i32) {
    let mut prevlen: i32 = -1;
    let mut nextlen: i32 = tree[0].dl as i32;
    let mut count: i32 = 0;
    let mut max_count: i32 = 7;
    let mut min_count: i32 = 4;

    if nextlen == 0 {
        max_count = 138;
        min_count = 3;
    }
    tree[(max_code + 1) as usize].dl = 0xffff; // guard

    for n in 0..=max_code {
        let curlen = nextlen;
        nextlen = tree[(n + 1) as usize].dl as i32;
        count += 1;
        if count < max_count && curlen == nextlen {
            continue;
        } else if count < min_count {
            bl_tree[curlen as usize].fc = bl_tree[curlen as usize].fc.wrapping_add(count as u16);
        } else if curlen != 0 {
            if curlen != prevlen {
                bl_tree[curlen as usize].fc = bl_tree[curlen as usize].fc.wrapping_add(1);
            }
            bl_tree[REP_3_6].fc = bl_tree[REP_3_6].fc.wrapping_add(1);
        } else if count <= 10 {
            bl_tree[REPZ_3_10].fc = bl_tree[REPZ_3_10].fc.wrapping_add(1);
        } else {
            bl_tree[REPZ_11_138].fc = bl_tree[REPZ_11_138].fc.wrapping_add(1);
        }
        count = 0;
        prevlen = curlen;
        if nextlen == 0 {
            max_count = 138;
            min_count = 3;
        } else if curlen == nextlen {
            max_count = 6;
            min_count = 3;
        } else {
            max_count = 7;
            min_count = 4;
        }
    }
}

impl DeflateState {
    /// `put_byte()`
    #[inline(always)]
    pub(crate) fn put_byte(&mut self, c: u8) {
        let p = self.pending;
        self.pending_buf[p] = c;
        self.pending = p + 1;
    }

    /// `put_short()`: LSB first.
    #[inline(always)]
    pub(crate) fn put_short(&mut self, w: u16) {
        let p = self.pending;
        self.pending_buf[p] = (w & 0xff) as u8;
        self.pending_buf[p + 1] = (w >> 8) as u8;
        self.pending = p + 2;
    }

    /// `send_bits()`: value must fit in `length` bits, length <= 16.
    #[inline(always)]
    pub(crate) fn send_bits(&mut self, value: u32, length: i32) {
        let len = length;
        if self.bi_valid > BUF_SIZE - len {
            let val = value as u16 as u32;
            self.bi_buf |= (val << self.bi_valid) as u16;
            let b = self.bi_buf;
            self.put_short(b);
            self.bi_buf = (val >> (BUF_SIZE - self.bi_valid)) as u16;
            self.bi_valid += len - BUF_SIZE;
        } else {
            self.bi_buf |= ((value as u16 as u32) << self.bi_valid) as u16;
            self.bi_valid += len;
        }
    }

    /// Flush the bit buffer, keeping at most 7 bits in it.
    #[inline]
    pub(crate) fn bi_flush(&mut self) {
        if self.bi_valid == 16 {
            let b = self.bi_buf;
            self.put_short(b);
            self.bi_buf = 0;
            self.bi_valid = 0;
        } else if self.bi_valid >= 8 {
            let b = self.bi_buf as u8;
            self.put_byte(b);
            self.bi_buf >>= 8;
            self.bi_valid -= 8;
        }
    }

    /// Flush the bit buffer and align the output on a byte boundary.
    pub(crate) fn bi_windup(&mut self) {
        if self.bi_valid > 8 {
            let b = self.bi_buf;
            self.put_short(b);
        } else if self.bi_valid > 0 {
            let b = self.bi_buf as u8;
            self.put_byte(b);
        }
        self.bi_used = ((self.bi_valid - 1) & 7) + 1;
        self.bi_buf = 0;
        self.bi_valid = 0;
    }

    /// Initialize a new block.
    pub(crate) fn init_block(&mut self) {
        for n in 0..L_CODES {
            self.dyn_ltree[n].fc = 0;
        }
        for n in 0..D_CODES {
            self.dyn_dtree[n].fc = 0;
        }
        for n in 0..BL_CODES {
            self.bl_tree[n].fc = 0;
        }
        self.dyn_ltree[END_BLOCK].fc = 1;
        self.tw.opt_len = 0;
        self.tw.static_len = 0;
        self.sym_next = 0;
        self.matches = 0;
    }

    /// `_tr_init()`: initialize the tree data structures for a new zlib stream.
    pub(crate) fn tr_init(&mut self) {
        self.bi_buf = 0;
        self.bi_valid = 0;
        self.bi_used = 0;
        // Initialize the first block of the first file:
        self.init_block();
    }

    /// Send a literal or distance tree in compressed form, using the codes in
    /// bl_tree.
    fn send_tree(&mut self, which: TreeKind, max_code: i32) {
        let mut prevlen: i32 = -1;
        let mut nextlen: i32 = self.tree_len(which, 0) as i32;
        let mut count: i32 = 0;
        let mut max_count: i32 = 7;
        let mut min_count: i32 = 4;

        // tree[max_code + 1].Len = -1;  -- guard already set
        if nextlen == 0 {
            max_count = 138;
            min_count = 3;
        }

        for n in 0..=max_code {
            let curlen = nextlen;
            nextlen = self.tree_len(which, (n + 1) as usize) as i32;
            count += 1;
            if count < max_count && curlen == nextlen {
                continue;
            } else if count < min_count {
                loop {
                    self.send_bl_code(curlen as usize);
                    count -= 1;
                    if count == 0 {
                        break;
                    }
                }
            } else if curlen != 0 {
                if curlen != prevlen {
                    self.send_bl_code(curlen as usize);
                    count -= 1;
                }
                self.send_bl_code(REP_3_6);
                self.send_bits((count - 3) as u32, 2);
            } else if count <= 10 {
                self.send_bl_code(REPZ_3_10);
                self.send_bits((count - 3) as u32, 3);
            } else {
                self.send_bl_code(REPZ_11_138);
                self.send_bits((count - 11) as u32, 7);
            }
            count = 0;
            prevlen = curlen;
            if nextlen == 0 {
                max_count = 138;
                min_count = 3;
            } else if curlen == nextlen {
                max_count = 6;
                min_count = 3;
            } else {
                max_count = 7;
                min_count = 4;
            }
        }
    }

    #[inline(always)]
    fn tree_len(&self, which: TreeKind, i: usize) -> u16 {
        match which {
            TreeKind::L => self.dyn_ltree[i].dl,
            TreeKind::D => self.dyn_dtree[i].dl,
        }
    }

    #[inline(always)]
    fn send_bl_code(&mut self, c: usize) {
        let ct = self.bl_tree[c];
        self.send_bits(ct.fc as u32, ct.dl as i32);
    }

    /// Construct the Huffman tree for the bit lengths and return the index in
    /// bl_order of the last bit length code to send.
    fn build_bl_tree(&mut self) -> i32 {
        // Determine the bit length frequencies for literal and distance trees
        scan_tree(&mut self.bl_tree, &mut self.dyn_ltree, self.l_max_code);
        scan_tree(&mut self.bl_tree, &mut self.dyn_dtree, self.d_max_code);

        // Build the bit length tree:
        self.bl_max_code = build_tree(&mut self.tw, &mut self.bl_tree, &STATIC_BL_DESC);
        // opt_len now includes the length of the tree representations, except
        // the lengths of the bit lengths codes and the 5 + 5 + 4 bits for the
        // counts.

        // Determine the number of bit length codes to send. The pkzip format
        // requires that at least 4 bit length codes be sent.
        let mut max_blindex = BL_CODES as i32 - 1;
        while max_blindex >= 3 {
            if self.bl_tree[BL_ORDER[max_blindex as usize] as usize].dl != 0 {
                break;
            }
            max_blindex -= 1;
        }
        // Update opt_len to include the bit length tree and counts
        self.tw.opt_len = self
            .tw
            .opt_len
            .wrapping_add(3 * (max_blindex as u64 + 1) + 5 + 5 + 4);
        max_blindex
    }

    /// Send the header for a block using dynamic Huffman trees.
    fn send_all_trees(&mut self, lcodes: i32, dcodes: i32, blcodes: i32) {
        self.send_bits((lcodes - 257) as u32, 5); // not +255 as stated in appnote.txt
        self.send_bits((dcodes - 1) as u32, 5);
        self.send_bits((blcodes - 4) as u32, 4); // not -3 as stated in appnote.txt
        for rank in 0..blcodes {
            let l = self.bl_tree[BL_ORDER[rank as usize] as usize].dl;
            self.send_bits(l as u32, 3);
        }
        self.send_tree(TreeKind::L, lcodes - 1); // literal tree
        self.send_tree(TreeKind::D, dcodes - 1); // distance tree
    }

    /// `_tr_stored_block()`: send a stored block. `buf` is an offset into
    /// the window (or `None` for `(char *)0`).
    pub(crate) fn tr_stored_block(&mut self, buf: Option<usize>, stored_len: u32, last: bool) {
        self.send_bits((STORED_BLOCK << 1) + last as u32, 3); // send block type
        self.bi_windup(); // align on byte boundary
        self.put_short(stored_len as u16);
        self.put_short(!(stored_len as u16));
        if stored_len != 0 {
            let src = buf.expect("stored block without buffer");
            let p = self.pending;
            let n = stored_len as usize;
            self.pending_buf[p..p + n].copy_from_slice(&self.window[src..src + n]);
        }
        self.pending += stored_len as usize;
    }

    /// `_tr_flush_bits()`
    #[inline]
    pub(crate) fn tr_flush_bits(&mut self) {
        self.bi_flush();
    }

    /// `_tr_align()`: send one empty static block to give enough lookahead
    /// for inflate.
    pub(crate) fn tr_align(&mut self) {
        self.send_bits(STATIC_TREES << 1, 3);
        let c = STATIC_LTREE[END_BLOCK];
        self.send_bits(c.fc as u32, c.dl as i32);
        self.bi_flush();
    }

    /// Send the block data compressed using the given Huffman trees.
    fn compress_block(&mut self, use_static: bool) {
        let mut sx: usize = 0;
        let sym_buf = self.sym_buf;
        if self.sym_next != 0 {
            loop {
                let mut dist = self.pending_buf[sym_buf + sx] as u32;
                sx += 1;
                dist += (self.pending_buf[sym_buf + sx] as u32) << 8;
                sx += 1;
                let mut lc = self.pending_buf[sym_buf + sx] as i32;
                sx += 1;
                if dist == 0 {
                    // send a literal byte
                    let ct = if use_static {
                        STATIC_LTREE[lc as usize]
                    } else {
                        self.dyn_ltree[lc as usize]
                    };
                    self.send_bits(ct.fc as u32, ct.dl as i32);
                } else {
                    // Here, lc is the match length - MIN_MATCH
                    let mut code = LENGTH_CODE[lc as usize] as usize;
                    let ct = if use_static {
                        STATIC_LTREE[code + LITERALS + 1]
                    } else {
                        self.dyn_ltree[code + LITERALS + 1]
                    };
                    self.send_bits(ct.fc as u32, ct.dl as i32); // send length code
                    let mut extra = EXTRA_LBITS[code];
                    if extra != 0 {
                        lc -= BASE_LENGTH[code];
                        self.send_bits(lc as u32, extra); // send the extra length bits
                    }
                    dist -= 1; // dist is now the match distance - 1
                    code = d_code(dist) as usize;
                    let ct = if use_static {
                        STATIC_DTREE[code]
                    } else {
                        self.dyn_dtree[code]
                    };
                    self.send_bits(ct.fc as u32, ct.dl as i32); // send the distance code
                    extra = EXTRA_DBITS[code];
                    if extra != 0 {
                        dist = dist.wrapping_sub(BASE_DIST[code] as u32);
                        self.send_bits(dist, extra); // send the extra distance bits
                    }
                }
                if sx >= self.sym_next as usize {
                    break;
                }
            }
        }
        let ct = if use_static {
            STATIC_LTREE[END_BLOCK]
        } else {
            self.dyn_ltree[END_BLOCK]
        };
        self.send_bits(ct.fc as u32, ct.dl as i32);
    }

    /// Check if the data type is TEXT or BINARY.
    fn detect_data_type(&self) -> i32 {
        // block_mask is the bit mask of block-listed bytes
        // set bits 0..6, 14..25, and 28..31
        let mut block_mask: u32 = 0xf3ff_c07f;
        for n in 0..=31usize {
            if (block_mask & 1) != 0 && self.dyn_ltree[n].fc != 0 {
                return Z_BINARY;
            }
            block_mask >>= 1;
        }
        // Check for textual ("allow-listed") bytes.
        if self.dyn_ltree[9].fc != 0 || self.dyn_ltree[10].fc != 0 || self.dyn_ltree[13].fc != 0 {
            return Z_TEXT;
        }
        for n in 32..LITERALS {
            if self.dyn_ltree[n].fc != 0 {
                return Z_TEXT;
            }
        }
        Z_BINARY
    }

    /// `_tr_flush_block()`: determine the best encoding for the current block:
    /// dynamic trees, static trees or store, and write out the encoded block.
    /// `buf` is a window offset or `None` for `(charf *)Z_NULL`.
    pub(crate) fn tr_flush_block(&mut self, buf: Option<usize>, stored_len: u64, last: bool) {
        let mut opt_lenb: u64;
        let static_lenb: u64;
        let mut max_blindex = 0i32;

        // Build the Huffman trees unless a stored block is forced
        if self.level > 0 {
            // Check if the file is binary or text
            if self.data_type == Z_UNKNOWN {
                self.data_type = self.detect_data_type();
            }

            // Construct the literal and distance trees
            self.l_max_code = build_tree(&mut self.tw, &mut self.dyn_ltree, &STATIC_L_DESC);
            self.d_max_code = build_tree(&mut self.tw, &mut self.dyn_dtree, &STATIC_D_DESC);
            // At this point, opt_len and static_len are the total bit lengths
            // of the compressed block data, excluding the tree representations.

            // Build the bit length tree for the above two trees, and get the
            // index in bl_order of the last bit length code to send.
            max_blindex = self.build_bl_tree();

            // Determine the best encoding. Compute the block lengths in bytes.
            opt_lenb = (self.tw.opt_len.wrapping_add(3 + 7) & 0xffff_ffff) >> 3;
            static_lenb = (self.tw.static_len.wrapping_add(3 + 7) & 0xffff_ffff) >> 3;

            if static_lenb <= opt_lenb || self.strategy == Z_FIXED {
                opt_lenb = static_lenb;
            }
        } else {
            opt_lenb = stored_len + 5; // force a stored block
            static_lenb = opt_lenb;
        }

        if stored_len + 4 <= opt_lenb && buf.is_some() {
            // 4: two words for the lengths
            // The test buf != NULL is only necessary if LIT_BUFSIZE > WSIZE.
            self.tr_stored_block(buf, stored_len as u32, last);
        } else if static_lenb == opt_lenb {
            self.send_bits((STATIC_TREES << 1) + last as u32, 3);
            self.compress_block(true);
        } else {
            self.send_bits((DYN_TREES << 1) + last as u32, 3);
            let (l, d) = (self.l_max_code, self.d_max_code);
            self.send_all_trees(l + 1, d + 1, max_blindex + 1);
            self.compress_block(false);
        }
        self.init_block();

        if last {
            self.bi_windup();
        }
    }

    /// `_tr_tally()` (not used by the release-mode C build, which uses the
    /// `_tr_tally_lit`/`_tr_tally_dist` macros; kept for completeness).
    #[inline(always)]
    pub(crate) fn tr_tally_lit(&mut self, c: u8) -> bool {
        let p = self.sym_buf + self.sym_next as usize;
        self.pending_buf[p] = 0;
        self.pending_buf[p + 1] = 0;
        self.pending_buf[p + 2] = c;
        self.sym_next += 3;
        self.dyn_ltree[c as usize].fc = self.dyn_ltree[c as usize].fc.wrapping_add(1);
        self.sym_next == self.sym_end
    }

    /// `_tr_tally_dist()` macro (release build): note that, unlike the
    /// debug-build function `_tr_tally()`, it does not increment `matches`.
    #[inline(always)]
    pub(crate) fn tr_tally_dist(&mut self, distance: u32, length: u32) -> bool {
        let len = length as u8;
        let mut dist = distance as u16;
        let p = self.sym_buf + self.sym_next as usize;
        self.pending_buf[p] = dist as u8;
        self.pending_buf[p + 1] = (dist >> 8) as u8;
        self.pending_buf[p + 2] = len;
        self.sym_next += 3;
        dist = dist.wrapping_sub(1);
        let lc = LENGTH_CODE[len as usize] as usize + LITERALS + 1;
        self.dyn_ltree[lc].fc = self.dyn_ltree[lc].fc.wrapping_add(1);
        let dc = d_code(dist as u32) as usize;
        self.dyn_dtree[dc].fc = self.dyn_dtree[dc].fc.wrapping_add(1);
        self.sym_next == self.sym_end
    }
}
