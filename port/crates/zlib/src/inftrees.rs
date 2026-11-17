//! Port of inftrees.c (zlib 1.3.2): build decoding tables.

/// Decoding table entry (`code` in inftrees.h).
///
/// op values as set by inflate_table():
/// 00000000 - literal
/// 0000tttt - table link, tttt != 0 is the number of table index bits
/// 0001eeee - length or distance, eeee is the number of extra bits
/// 01100000 - end of block
/// 01000000 - invalid code
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct Code {
    pub op: u8,
    pub bits: u8,
    pub val: u16,
}

pub(crate) const ENOUGH_LENS: usize = 852;
pub(crate) const ENOUGH_DISTS: usize = 592;
pub(crate) const ENOUGH: usize = ENOUGH_LENS + ENOUGH_DISTS;

const MAXBITS: usize = 15;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CodeType {
    Codes,
    Lens,
    Dists,
}

static LBASE: [u16; 31] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258, 0, 0,
];
static LEXT: [u16; 31] = [
    16, 16, 16, 16, 16, 16, 16, 16, 17, 17, 17, 17, 18, 18, 18, 18, 19, 19, 19, 19, 20, 20, 20, 20,
    21, 21, 21, 21, 16, 199, 75,
];
static DBASE: [u16; 32] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577, 0, 0,
];
static DEXT: [u16; 32] = [
    16, 16, 16, 16, 17, 17, 18, 18, 19, 19, 20, 20, 21, 21, 22, 22, 23, 23, 24, 24, 25, 25, 26, 26,
    27, 27, 28, 28, 29, 29, 64, 64,
];

/// `inflate_table()`: build a set of tables to decode the provided canonical
/// Huffman code. The tables are written into `codes` starting at `*next`;
/// on return `*next` is advanced past them. `bits` is the requested root
/// table index bits on entry and the actual root bits on return. Returns 0
/// on success, -1 for an invalid code, +1 if ENOUGH isn't enough.
pub(crate) fn inflate_table(
    ty: CodeType,
    lens: &[u16],
    ncodes: usize,
    codes: &mut [Code],
    next_tab: &mut usize,
    bits: &mut u32,
    work: &mut [u16],
) -> i32 {
    let mut count = [0u16; MAXBITS + 1];
    let mut offs = [0u16; MAXBITS + 1];

    // accumulate lengths for codes (assumes lens[] all in 0..MAXBITS)
    for sym in 0..ncodes {
        count[lens[sym] as usize] += 1;
    }

    // bound code lengths, force root to be within code lengths
    let mut root = *bits;
    let mut max = MAXBITS as u32;
    while max >= 1 {
        if count[max as usize] != 0 {
            break;
        }
        max -= 1;
    }
    if root > max {
        root = max;
    }
    if max == 0 {
        // no symbols to code at all
        let here = Code {
            op: 64, // invalid code marker
            bits: 1,
            val: 0,
        };
        codes[*next_tab] = here; // make a table to force an error
        codes[*next_tab + 1] = here;
        *next_tab += 2;
        *bits = 1;
        return 0; // no symbols, but wait for decoding to report error
    }
    let mut min: u32 = 1;
    while min < max {
        if count[min as usize] != 0 {
            break;
        }
        min += 1;
    }
    if root < min {
        root = min;
    }

    // check for an over-subscribed or incomplete set of lengths
    let mut left: i32 = 1;
    for len in 1..=MAXBITS {
        left <<= 1;
        left -= count[len] as i32;
        if left < 0 {
            return -1; // over-subscribed
        }
    }
    if left > 0 && (ty == CodeType::Codes || max != 1) {
        return -1; // incomplete set
    }

    // generate offsets into symbol table for each length for sorting
    offs[1] = 0;
    for len in 1..MAXBITS {
        offs[len + 1] = offs[len] + count[len];
    }

    // sort symbols by length, by symbol order within each length
    for sym in 0..ncodes {
        if lens[sym] != 0 {
            let l = lens[sym] as usize;
            work[offs[l] as usize] = sym as u16;
            offs[l] += 1;
        }
    }

    // set up for code type
    let (base, extra, match_): (&[u16], &[u16], u32) = match ty {
        CodeType::Codes => (&[], &[], 20),
        CodeType::Lens => (&LBASE, &LEXT, 257),
        CodeType::Dists => (&DBASE, &DEXT, 0),
    };

    // initialize state for loop
    let table = *next_tab; // start of this table set
    let mut huff: u32 = 0; // starting code
    let mut sym: usize = 0; // starting code symbol
    let mut len: u32 = min; // starting code length
    let mut next: usize = table; // current table to fill in
    let mut curr: u32 = root; // current table index bits
    let mut drop: u32 = 0; // current bits to drop from code for index
    let mut low: u32 = u32::MAX; // trigger new sub-table when len > root
    let mut used: u32 = 1u32 << root; // use root table entries
    let mask: u32 = used - 1; // mask for comparing low

    // check available table space
    if (ty == CodeType::Lens && used as usize > ENOUGH_LENS)
        || (ty == CodeType::Dists && used as usize > ENOUGH_DISTS)
    {
        return 1;
    }

    // process all codes and make table entries
    loop {
        // create table entry
        let mut here = Code {
            op: 0,
            bits: (len - drop) as u8,
            val: 0,
        };
        let w = work[sym] as u32;
        if w + 1 < match_ {
            here.op = 0;
            here.val = work[sym];
        } else if w >= match_ {
            here.op = extra[(w - match_) as usize] as u8;
            here.val = base[(w - match_) as usize];
        } else {
            here.op = 32 + 64; // end of block
            here.val = 0;
        }

        // replicate for those indices with low len bits equal to huff
        let incr0 = 1u32 << (len - drop);
        let mut fill = 1u32 << curr;
        min = fill; // save offset to next table
        loop {
            fill -= incr0;
            codes[next + ((huff >> drop) + fill) as usize] = here;
            if fill == 0 {
                break;
            }
        }

        // backwards increment the len-bit code huff
        let mut incr = 1u32 << (len - 1);
        while huff & incr != 0 {
            incr >>= 1;
        }
        if incr != 0 {
            huff &= incr - 1;
            huff += incr;
        } else {
            huff = 0;
        }

        // go to next symbol, update count, len
        sym += 1;
        count[len as usize] -= 1;
        if count[len as usize] == 0 {
            if len == max {
                break;
            }
            len = lens[work[sym] as usize] as u32;
        }

        // create new sub-table if needed
        if len > root && (huff & mask) != low {
            // if first time, transition to sub-tables
            if drop == 0 {
                drop = root;
            }

            // increment past last table
            next += min as usize; // here min is 1 << curr

            // determine length of next table
            curr = len - drop;
            let mut left: i32 = 1 << curr;
            while curr + drop < max {
                left -= count[(curr + drop) as usize] as i32;
                if left <= 0 {
                    break;
                }
                curr += 1;
                left <<= 1;
            }

            // check for enough space
            used += 1u32 << curr;
            if (ty == CodeType::Lens && used as usize > ENOUGH_LENS)
                || (ty == CodeType::Dists && used as usize > ENOUGH_DISTS)
            {
                return 1;
            }

            // point entry in root table to sub-table
            low = huff & mask;
            codes[table + low as usize] = Code {
                op: curr as u8,
                bits: root as u8,
                val: (next - table) as u16,
            };
        }
    }

    // fill in remaining table entry if code is incomplete (guaranteed to have
    // at most one remaining entry, since if the code is incomplete, the
    // maximum code length that was allowed to get this far is one bit)
    if huff != 0 {
        codes[next + huff as usize] = Code {
            op: 64, // invalid code marker
            bits: (len - drop) as u8,
            val: 0,
        };
    }

    // set return parameters
    *next_tab += used as usize;
    *bits = root;
    0
}
