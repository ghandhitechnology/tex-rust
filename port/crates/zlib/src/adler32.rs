//! Port of adler32.c (zlib 1.3.2).
//!
//! `uLong` is 32 bits on Windows (LLP64), which is the platform the TeX Live
//! build we match uses, so the arithmetic here is done in `u32` with the same
//! wrap-around behaviour as the C code for out-of-range starting values.

const BASE: u32 = 65521; // largest prime smaller than 65536
const NMAX: usize = 5552;
// NMAX is the largest n such that 255n(n+1)/2 + (n+1)(BASE-1) <= 2^32-1

/// `adler32_z()`: update a running Adler-32 checksum with `buf`.
///
/// `buf == None` corresponds to passing `Z_NULL`, which returns the required
/// initial value (1) unless `len == 1` semantics apply (see C source); with a
/// slice API that case cannot arise, so `None` simply returns 1.
pub fn adler32_z(adler: u32, buf: Option<&[u8]>) -> u32 {
    let Some(buf) = buf else {
        return 1;
    };
    let mut sum2: u32 = (adler >> 16) & 0xffff;
    let mut adler: u32 = adler & 0xffff;
    let mut len = buf.len();

    // in case user likes doing a byte at a time, keep it fast
    if len == 1 {
        adler = adler.wrapping_add(buf[0] as u32);
        if adler >= BASE {
            adler -= BASE;
        }
        sum2 = sum2.wrapping_add(adler);
        if sum2 >= BASE {
            sum2 -= BASE;
        }
        return adler | (sum2 << 16);
    }

    // in case short lengths are provided, keep it somewhat fast
    if len < 16 {
        for &b in buf {
            adler = adler.wrapping_add(b as u32);
            sum2 = sum2.wrapping_add(adler);
        }
        if adler >= BASE {
            adler -= BASE;
        }
        sum2 %= BASE; // only added so many BASE's
        return adler | (sum2 << 16);
    }

    let mut p = buf;
    // do length NMAX blocks -- requires just one modulo operation
    while len >= NMAX {
        len -= NMAX;
        let (block, rest) = p.split_at(NMAX);
        for chunk in block.chunks_exact(16) {
            for &b in chunk {
                adler = adler.wrapping_add(b as u32);
                sum2 = sum2.wrapping_add(adler);
            }
        }
        adler %= BASE;
        sum2 %= BASE;
        p = rest;
    }

    // do remaining bytes (less than NMAX, still just one modulo)
    if len != 0 {
        for &b in p {
            adler = adler.wrapping_add(b as u32);
            sum2 = sum2.wrapping_add(adler);
        }
        adler %= BASE;
        sum2 %= BASE;
    }

    // return recombined sums
    adler | (sum2 << 16)
}

/// `adler32()`.
pub fn adler32(adler: u32, buf: &[u8]) -> u32 {
    adler32_z(adler, Some(buf))
}

/// `adler32_combine64()` / `adler32_combine()`.
pub fn adler32_combine(adler1: u32, adler2: u32, len2: i64) -> u32 {
    // for negative len, return invalid adler32 as a clue for debugging
    if len2 < 0 {
        return 0xffff_ffff;
    }
    // the derivation of this formula is left as an exercise for the reader
    let rem = (len2 % BASE as i64) as u32;
    let mut sum1: u32 = adler1 & 0xffff;
    let mut sum2: u32 = rem.wrapping_mul(sum1);
    sum2 %= BASE;
    sum1 = sum1.wrapping_add((adler2 & 0xffff) + BASE - 1);
    sum2 = sum2
        .wrapping_add((adler1 >> 16) & 0xffff)
        .wrapping_add((adler2 >> 16) & 0xffff)
        .wrapping_add(BASE)
        .wrapping_sub(rem);
    if sum1 >= BASE {
        sum1 -= BASE;
    }
    if sum1 >= BASE {
        sum1 -= BASE;
    }
    if sum2 >= (BASE << 1) {
        sum2 -= BASE << 1;
    }
    if sum2 >= BASE {
        sum2 -= BASE;
    }
    sum1 | (sum2 << 16)
}
