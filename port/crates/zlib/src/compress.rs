//! Port of compress.c and uncompr.c (zlib 1.3.2).

use crate::deflate::{DeflateState, Io, DEF_MEM_LEVEL, MAX_WBITS};
use crate::inflate::InflateState;
use crate::{
    Z_BUF_ERROR, Z_DATA_ERROR, Z_DEFAULT_COMPRESSION, Z_DEFAULT_STRATEGY, Z_DEFLATED, Z_FINISH,
    Z_NEED_DICT, Z_NO_FLUSH, Z_OK, Z_STREAM_END,
};

const UINT_MAX: usize = u32::MAX as usize;

/// `compress2_z()`: compress `source` into `dest`. Returns the zlib return
/// code and the number of bytes written to `dest` (`*destLen`).
pub fn compress2(dest: &mut [u8], source: &[u8], level: i32) -> (i32, usize) {
    let mut left = dest.len();
    let mut s = match DeflateState::init2(level, Z_DEFLATED, MAX_WBITS, DEF_MEM_LEVEL, Z_DEFAULT_STRATEGY)
    {
        Ok(s) => s,
        Err(e) => return (e, 0),
    };
    let mut next_out = 0usize;
    let mut avail_out = 0usize;
    let mut next_in = 0usize;
    let mut avail_in = 0usize;
    let mut source_len = source.len();
    let mut err;
    loop {
        if avail_out == 0 {
            avail_out = left.min(UINT_MAX);
            left -= avail_out;
        }
        if avail_in == 0 {
            avail_in = source_len.min(UINT_MAX);
            source_len -= avail_in;
        }
        let flush = if source_len != 0 { Z_NO_FLUSH } else { Z_FINISH };
        let mut io = Io::new(
            &source[next_in..next_in + avail_in],
            &mut dest[next_out..next_out + avail_out],
        );
        err = s.deflate(&mut io, flush);
        next_in += io.next_in;
        avail_in = io.avail_in as usize;
        next_out += io.next_out;
        avail_out = io.avail_out as usize;
        if err != Z_OK {
            break;
        }
    }
    // deflateEnd(&stream)
    (if err == Z_STREAM_END { Z_OK } else { err }, next_out)
}

/// `compress_z()`: `compress2()` with `Z_DEFAULT_COMPRESSION`.
pub fn compress(dest: &mut [u8], source: &[u8]) -> (i32, usize) {
    compress2(dest, source, Z_DEFAULT_COMPRESSION)
}

/// `compressBound_z()`
pub fn compress_bound(source_len: u64) -> u64 {
    let bound = source_len
        .wrapping_add(source_len >> 12)
        .wrapping_add(source_len >> 14)
        .wrapping_add(source_len >> 25)
        .wrapping_add(13);
    if bound < source_len {
        u64::MAX
    } else {
        bound
    }
}

/// Convenience: `compress2()` into a vector sized with `compressBound()`.
pub fn compress_to_vec(source: &[u8], level: i32) -> Result<Vec<u8>, i32> {
    let mut out = vec![0u8; compress_bound(source.len() as u64) as usize];
    let (r, n) = compress2(&mut out, source, level);
    if r != Z_OK {
        return Err(r);
    }
    out.truncate(n);
    Ok(out)
}

/// `uncompress2_z()`: returns (code, bytes written to dest, bytes of
/// source consumed).
pub fn uncompress2(dest: &mut [u8], source: &[u8]) -> (i32, usize, usize) {
    let mut len = source.len();
    let mut left = dest.len();
    let mut s = match InflateState::init2(15) {
        Ok(s) => s,
        Err(e) => return (e, 0, 0),
    };
    let mut next_in = 0usize;
    let mut avail_in = 0usize;
    let mut next_out = 0usize;
    let mut avail_out = 0usize;
    let mut err;
    // next_out cannot be NULL: a zero-length dest is handled naturally here.
    loop {
        if avail_out == 0 {
            avail_out = left.min(UINT_MAX);
            left -= avail_out;
        }
        if avail_in == 0 {
            avail_in = len.min(UINT_MAX);
            len -= avail_in;
        }
        let mut io = Io::new(
            &source[next_in..next_in + avail_in],
            &mut dest[next_out..next_out + avail_out],
        );
        err = s.inflate(&mut io, Z_NO_FLUSH);
        next_in += io.next_in;
        avail_in = io.avail_in as usize;
        next_out += io.next_out;
        avail_out = io.avail_out as usize;
        if err != Z_OK {
            break;
        }
    }

    // Set len and left to the unused input data and unused output space.
    len += avail_in;
    left += avail_out;
    let source_used = source.len() - len;
    let dest_len = dest.len() - left;
    let ret = if err == Z_STREAM_END {
        Z_OK
    } else if err == Z_NEED_DICT {
        Z_DATA_ERROR
    } else if err == Z_BUF_ERROR && len == 0 {
        Z_DATA_ERROR
    } else {
        err
    };
    (ret, dest_len, source_used)
}

/// `uncompress_z()`: returns (code, bytes written to dest).
pub fn uncompress(dest: &mut [u8], source: &[u8]) -> (i32, usize) {
    let (r, d, _) = uncompress2(dest, source);
    (r, d)
}

/// Convenience: decompress a complete zlib stream whose decompressed size
/// is known (or bounded) by `max_len`.
pub fn uncompress_to_vec(source: &[u8], max_len: usize) -> Result<Vec<u8>, i32> {
    let mut out = vec![0u8; max_len];
    let (r, n, _) = uncompress2(&mut out, source);
    if r != Z_OK {
        return Err(r);
    }
    out.truncate(n);
    Ok(out)
}
