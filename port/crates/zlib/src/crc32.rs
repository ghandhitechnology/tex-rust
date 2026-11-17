//! Port of crc32.c (zlib 1.3.2).
//!
//! The C file selects between several equivalent implementations (byte-wise,
//! braided, hardware). They all compute the same reflected CRC-32 with
//! polynomial 0xedb88320; this port uses slicing-by-16 tables built at compile
//! time. `crc32_combine*` use the same polynomial arithmetic as the C code.

const POLY: u32 = 0xedb8_8320; // p(x) reflected, with x^32 implied

const fn make_tables() -> [[u32; 256]; 16] {
    let mut t = [[0u32; 256]; 16];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { (c >> 1) ^ POLY } else { c >> 1 };
            k += 1;
        }
        t[0][n] = c;
        n += 1;
    }
    let mut n = 0;
    while n < 256 {
        let mut c = t[0][n];
        let mut k = 1;
        while k < 16 {
            c = t[0][(c & 0xff) as usize] ^ (c >> 8);
            t[k][n] = c;
            k += 1;
        }
        n += 1;
    }
    t
}

static TABLES: [[u32; 256]; 16] = make_tables();

/// `get_crc_table()`: the byte-wise CRC table.
pub fn get_crc_table() -> &'static [u32; 256] {
    &TABLES[0]
}

#[inline]
fn crc_update(mut crc: u32, buf: &[u8]) -> u32 {
    let t = &TABLES;
    let mut chunks = buf.chunks_exact(16);
    for c in &mut chunks {
        let a = u32::from_le_bytes([c[0], c[1], c[2], c[3]]) ^ crc;
        crc = t[15][(a & 0xff) as usize]
            ^ t[14][((a >> 8) & 0xff) as usize]
            ^ t[13][((a >> 16) & 0xff) as usize]
            ^ t[12][(a >> 24) as usize]
            ^ t[11][c[4] as usize]
            ^ t[10][c[5] as usize]
            ^ t[9][c[6] as usize]
            ^ t[8][c[7] as usize]
            ^ t[7][c[8] as usize]
            ^ t[6][c[9] as usize]
            ^ t[5][c[10] as usize]
            ^ t[4][c[11] as usize]
            ^ t[3][c[12] as usize]
            ^ t[2][c[13] as usize]
            ^ t[1][c[14] as usize]
            ^ t[0][c[15] as usize];
    }
    for &b in chunks.remainder() {
        crc = t[0][((crc ^ b as u32) & 0xff) as usize] ^ (crc >> 8);
    }
    crc
}

/// `crc32_z()`: update a running CRC-32. `None` corresponds to `Z_NULL` and
/// returns the initial value 0.
pub fn crc32_z(crc: u32, buf: Option<&[u8]>) -> u32 {
    let Some(buf) = buf else {
        return 0;
    };
    !crc_update(!crc, buf)
}

/// `crc32()`.
pub fn crc32(crc: u32, buf: &[u8]) -> u32 {
    crc32_z(crc, Some(buf))
}

/// Return a(x) multiplied by b(x) modulo p(x), where p(x) is the CRC
/// polynomial, reflected. For speed, this requires that a not be zero.
fn multmodp(a: u32, mut b: u32) -> u32 {
    let mut m: u32 = 1 << 31;
    let mut p: u32 = 0;
    loop {
        if a & m != 0 {
            p ^= b;
            if a & (m - 1) == 0 {
                break;
            }
        }
        m >>= 1;
        b = if b & 1 != 0 { (b >> 1) ^ POLY } else { b >> 1 };
    }
    p
}

const fn make_x2n() -> [u32; 32] {
    // x2n_table[n] = x^2^n mod p(x)
    let mut t = [0u32; 32];
    let mut p: u32 = 1 << 30; // x^1
    t[0] = p;
    let mut n = 1;
    while n < 32 {
        // p = multmodp(p, p) (const version)
        let a = p;
        let mut b = p;
        let mut m: u32 = 1 << 31;
        let mut r: u32 = 0;
        loop {
            if a & m != 0 {
                r ^= b;
                if a & (m - 1) == 0 {
                    break;
                }
            }
            m >>= 1;
            b = if b & 1 != 0 { (b >> 1) ^ POLY } else { b >> 1 };
        }
        p = r;
        t[n] = p;
        n += 1;
    }
    t
}

static X2N_TABLE: [u32; 32] = make_x2n();

/// Return x^(n * 2^k) modulo p(x).
fn x2nmodp(mut n: i64, mut k: u32) -> u32 {
    let mut p: u32 = 1 << 31; // x^0 == 1
    while n != 0 {
        if n & 1 != 0 {
            p = multmodp(X2N_TABLE[(k & 31) as usize], p);
        }
        n >>= 1;
        k += 1;
    }
    p
}

/// `crc32_combine_gen64()`.
pub fn crc32_combine_gen(len2: i64) -> u32 {
    if len2 < 0 {
        return 0;
    }
    x2nmodp(len2, 3)
}

/// `crc32_combine_op()`.
pub fn crc32_combine_op(crc1: u32, crc2: u32, op: u32) -> u32 {
    if op == 0 {
        return 0;
    }
    multmodp(op, crc1) ^ crc2
}

/// `crc32_combine64()` / `crc32_combine()`.
pub fn crc32_combine(crc1: u32, crc2: u32, len2: i64) -> u32 {
    crc32_combine_op(crc1, crc2, crc32_combine_gen(len2))
}
