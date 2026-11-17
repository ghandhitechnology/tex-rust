//! Port of inffast.c (zlib 1.3.2): decode literals and length/distance pairs
//! while at least 6 input bytes and 258 output bytes are available.

use crate::deflate::Io;
use crate::inflate::{InflateState, Mode};

/// Copy `len` bytes from `out - dist` to `out` in `buf` with byte-by-byte
/// semantics (the source may overlap the destination, as in C's loops).
#[inline(always)]
pub(crate) fn copy_match(buf: &mut [u8], out: usize, dist: usize, len: usize) {
    assert!(dist >= 1 && dist <= out && out + len <= buf.len());
    let from = out - dist;
    // SAFETY: the assertion bounds every access: reads are in
    // [from, from + len) and writes in [out, out + len), both inside buf.
    unsafe {
        let p = buf.as_mut_ptr();
        if dist >= 8 {
            // 8-byte steps: each read ends at or before the next write
            // position, so this equals the byte-wise forward copy.
            let mut i = 0;
            while i + 8 <= len {
                let v = (p.add(from + i) as *const u64).read_unaligned();
                (p.add(out + i) as *mut u64).write_unaligned(v);
                i += 8;
            }
            while i < len {
                *p.add(out + i) = *p.add(from + i);
                i += 1;
            }
        } else if dist == 1 {
            std::ptr::write_bytes(p.add(out), *p.add(from), len);
        } else {
            for i in 0..len {
                *p.add(out + i) = *p.add(from + i);
            }
        }
    }
}

/// `inflate_fast()`. Entry assumptions: mode == LEN, avail_in >= 6,
/// avail_out >= 258, start >= avail_out, bits < 8.
pub(crate) fn inflate_fast(state: &mut InflateState, io: &mut Io, start: u32) {
    let input = io.input;
    let mut inp = io.next_in; // local strm->next_in
    let last = inp + (io.avail_in as usize - 5); // have enough input while in < last
    let mut out = io.next_out; // local strm->next_out
    let beg = out - (start - io.avail_out) as usize; // inflate()'s initial next_out
    let end = out + (io.avail_out as usize - 257); // while out < end, enough space
    let wsize = state.wsize as usize;
    let whave = state.whave as usize;
    let wnext = state.wnext as usize;
    let mut hold = state.hold;
    let mut bits = state.bits;
    let lmask = (1u64 << state.lenbits) - 1;
    let dmask = (1u64 << state.distbits) - 1;

    let mut new_mode: Option<Mode> = None;
    let mut msg: Option<&'static str> = None;
    {
        let (lcode, dcode) = state.tables();
        let window: &[u8] = &state.window;
        let output = &mut *io.output;
        // Loop invariants (as in C): at the top of each iteration at least 6
        // input bytes remain (in < last) and at least 258 output bytes
        // (out < end); one iteration uses at most 6 and 258.
        assert!(last + 5 <= input.len() && end + 257 <= output.len());
        macro_rules! inb {
            () => {{
                debug_assert!(inp < input.len());
                // SAFETY: see the loop invariants above.
                let b = unsafe { *input.get_unchecked(inp) } as u64;
                inp += 1;
                b
            }};
        }

        // decode literals and length/distances until end-of-block or not
        // enough input data or output space
        'outer: loop {
            if bits < 15 {
                hold += inb!() << bits;
                bits += 8;
                hold += inb!() << bits;
                bits += 8;
            }
            let mut here = lcode[(hold & lmask) as usize];
            // dolen:
            loop {
                let op = here.bits as u32;
                hold >>= op;
                bits -= op;
                let op = here.op as u32;
                if op == 0 {
                    // literal
                    debug_assert!(out < output.len());
                    // SAFETY: out < end (loop invariant), end + 257 <= len.
                    unsafe { *output.get_unchecked_mut(out) = here.val as u8 };
                    out += 1;
                    break;
                } else if op & 16 != 0 {
                    // length base
                    let mut len = here.val as usize;
                    let op = op & 15; // number of extra bits
                    if op != 0 {
                        if bits < op {
                            hold += inb!() << bits;
                            bits += 8;
                        }
                        len += (hold as u32 & ((1u32 << op) - 1)) as usize;
                        hold >>= op;
                        bits -= op;
                    }
                    if bits < 15 {
                        hold += inb!() << bits;
                        bits += 8;
                        hold += inb!() << bits;
                        bits += 8;
                    }
                    let mut here = dcode[(hold & dmask) as usize];
                    // dodist:
                    loop {
                        let op = here.bits as u32;
                        hold >>= op;
                        bits -= op;
                        let op = here.op as u32;
                        if op & 16 != 0 {
                            // distance base
                            let mut dist = here.val as usize;
                            let op = op & 15; // number of extra bits
                            if bits < op {
                                hold += inb!() << bits;
                                bits += 8;
                                if bits < op {
                                    hold += inb!() << bits;
                                    bits += 8;
                                }
                            }
                            dist += (hold as u32 & ((1u32 << op) - 1)) as usize;
                            hold >>= op;
                            bits -= op;
                            let op = out - beg; // max distance in output
                            if dist > op {
                                // see if copy from window
                                let mut op = dist - op; // distance back in window
                                if op > whave {
                                    // state->sane is always 1 in this build
                                    msg = Some("invalid distance too far back");
                                    new_mode = Some(Mode::Bad);
                                    break 'outer;
                                }
                                // Bytes come from the window (one or two
                                // segments), then possibly from the output.
                                let mut wfrom: usize;
                                let mut from_output = false;
                                if wnext == 0 {
                                    // very common case
                                    wfrom = wsize - op;
                                    if op < len {
                                        // some from window
                                        len -= op;
                                        output[out..out + op]
                                            .copy_from_slice(&window[wfrom..wfrom + op]);
                                        out += op;
                                        from_output = true; // rest from output
                                    }
                                } else if wnext < op {
                                    // wrap around window
                                    wfrom = wsize + wnext - op;
                                    op -= wnext;
                                    if op < len {
                                        // some from end of window
                                        len -= op;
                                        output[out..out + op]
                                            .copy_from_slice(&window[wfrom..wfrom + op]);
                                        out += op;
                                        wfrom = 0;
                                        if wnext < len {
                                            // some from start of window
                                            let op = wnext;
                                            len -= op;
                                            output[out..out + op].copy_from_slice(&window[..op]);
                                            out += op;
                                            from_output = true; // rest from output
                                        }
                                    }
                                } else {
                                    // contiguous in window
                                    wfrom = wnext - op;
                                    if op < len {
                                        // some from window
                                        len -= op;
                                        output[out..out + op]
                                            .copy_from_slice(&window[wfrom..wfrom + op]);
                                        out += op;
                                        from_output = true; // rest from output
                                    }
                                }
                                if from_output {
                                    copy_match(output, out, dist, len);
                                } else {
                                    output[out..out + len]
                                        .copy_from_slice(&window[wfrom..wfrom + len]);
                                }
                                out += len;
                            } else {
                                // copy direct from output
                                copy_match(output, out, dist, len);
                                out += len;
                            }
                            break;
                        } else if op & 64 == 0 {
                            // 2nd level distance code
                            here = dcode
                                [here.val as usize + (hold as u32 & ((1u32 << op) - 1)) as usize];
                            continue;
                        } else {
                            msg = Some("invalid distance code");
                            new_mode = Some(Mode::Bad);
                            break 'outer;
                        }
                    }
                    break;
                } else if op & 64 == 0 {
                    // 2nd level length code
                    here = lcode[here.val as usize + (hold as u32 & ((1u32 << op) - 1)) as usize];
                    continue;
                } else if op & 32 != 0 {
                    // end-of-block
                    new_mode = Some(Mode::Type);
                    break 'outer;
                } else {
                    msg = Some("invalid literal/length code");
                    new_mode = Some(Mode::Bad);
                    break 'outer;
                }
            }
            if !(inp < last && out < end) {
                break;
            }
        }
    }
    if let Some(m) = msg {
        state.msg = Some(m);
    }
    if let Some(m) = new_mode {
        state.mode = m;
    }

    // return unused bytes (on entry, bits < 8, so in won't go too far back)
    let len = bits >> 3;
    inp -= len as usize;
    bits -= len << 3;
    hold &= (1u64 << bits) - 1;

    // update state and return
    io.next_in = inp;
    io.next_out = out;
    io.avail_in = if inp < last {
        (5 + (last - inp)) as u32
    } else {
        (5 - (inp - last)) as u32
    };
    io.avail_out = if out < end {
        (257 + (end - out)) as u32
    } else {
        (257 - (out - end)) as u32
    };
    state.hold = hold;
    state.bits = bits;
}
