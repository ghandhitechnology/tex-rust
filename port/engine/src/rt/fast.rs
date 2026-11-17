//! Hot/cold splitting of TeX's inner loop.
//!
//! The translator emits the routines listed in `externs.FAST_PATHS` as `<name>_full`
//! (`#[inline(never)]`). The `<name>` defined here is inlined into every caller and handles
//! the most frequent cases itself, with exactly the effects the generated code would have;
//! anything else goes to `<name>_full` before any state has been changed, so the full
//! routine sees precisely the state it would have seen without the fast path.
//!
//! Why: `get_next` runs about half a billion times while building pdflatex.fmt, nearly
//! always for the cases below, but as one large function it pays a full prologue and
//! epilogue (eight saved registers) on every call.

use super::*;
use crate::gen::globals::Globals;

/// `dst := src` for TeX's in_state_record (cur_input and the input stack), one field at a
/// time. Pushing cur_input happens right after its fields were stored one by one; a
/// whole-struct copy would read them back with 16- and 8-byte loads, which the CPU cannot
/// serve from those smaller pending stores (store-forwarding stall). The loads are
/// volatile only so that LLVM cannot merge them back into one wide load.
#[inline(always)]
pub fn copy_instaterecord(dst: &mut crate::gen::globals::Rinstaterecord, src: &crate::gen::globals::Rinstaterecord) {
    // Store forwarding: a load is served from a pending store only if the store covers
    // it. cur_input's fields are stored one at a time (get_next updates loc) and the
    // stack slots are written by this function, so: read field by field (each narrow
    // load is covered by whatever store wrote it), write three 8-byte words (any later
    // field read is covered by one of them). The reads are volatile only so that LLVM
    // does not merge them back into wide loads.
    unsafe {
        let st = core::ptr::read_volatile(&src.statefield) as u64;
        let ix = core::ptr::read_volatile(&src.indexfield) as u64;
        let start = core::ptr::read_volatile(&src.startfield) as u32 as u64;
        let loc = core::ptr::read_volatile(&src.locfield) as u32 as u64;
        let limit = core::ptr::read_volatile(&src.limitfield) as u32 as u64;
        let name = core::ptr::read_volatile(&src.namefield) as u32 as u64;
        let tag = core::ptr::read_volatile(&src.synctextagfield) as u32 as u64;
        let d = dst as *mut crate::gen::globals::Rinstaterecord as *mut u64;
        // repr(C) layout: state(0) index(1) pad(2..4) start(4) loc(8) limit(12) name(16) tag(20)
        d.write_unaligned(st | (ix << 8) | (start << 32));
        d.add(1).write_unaligned(loc | (limit << 32));
        d.add(2).write_unaligned(name | (tag << 32));
    }
}

/// Hint the CPU to start loading `p[i]` into the cache. Has no other effect: a prefetch
/// never faults, and `i` = null (a large negative number) is skipped.
#[inline(always)]
pub fn prefetch<T>(p: Ptr<T>, i: i32) {
    #[cfg(target_arch = "x86_64")]
    if i >= 0 {
        unsafe {
            core::arch::x86_64::_mm_prefetch::<{ core::arch::x86_64::_MM_HINT_T0 }>(
                p.raw().offset(i as isize) as *const i8,
            )
        }
    }
}

impl Globals {
    /// `get_next` (tex.web §341), fast path for §357, input from a token list
    /// (state = token_list). Handled here:
    /// - the end of the list (loc = null): end_token_list, then restart;
    /// - a control sequence whose meaning is below outer_call (no check_outer_validity,
    ///   no dont_expand), with encTeX's \noexpand-in-\write mode (write_noexpanding) off;
    /// - a character token; out_param (insert a macro parameter) pushes the parameter's
    ///   token list and restarts;
    /// in each case unless the token ends an alignment entry (§342: tab_mark or car_ret
    /// with align_state = 0). The generated code for these cases (gen/r04.rs,
    /// getnext_full) is:
    ///
    /// ```text
    /// restart: cur_cs:=0;
    /// if loc<>null then begin t:=info(loc); loc:=link(loc);
    ///   if t>=cs_token_flag then begin cur_cs:=t-cs_token_flag;
    ///     cur_cmd:=eq_type(cur_cs); cur_chr:=equiv(cur_cs) end
    ///   else begin cur_cmd:=t div 256; cur_chr:=t mod 256;
    ///     left_brace: incr(align_state); right_brace: decr(align_state);
    ///     out_param: begin begin_token_list(param_stack[limit+cur_chr-1],parameter);
    ///       goto restart end
    ///   end end
    /// else begin end_token_list; goto restart end
    /// ```
    #[inline(always)]
    pub fn getnext(&mut self) {
        const NULL: i32 = -268435455;
        const CS_TOKEN_FLAG: i32 = 4095;
        const OUTER_CALL: i32 = 116;
        const LEFT_BRACE: i32 = 1;
        const RIGHT_BRACE: i32 = 2;
        const TAB_MARK: i32 = 4;
        const CAR_RET: i32 = 5; // = out_param for character tokens
        const PARAMETER: u8 = 0;
        loop {
            if self.curinput.statefield != 0 {
                // input from a file line (state mid_line, skip_blanks or new_line): a letter
                // or other_char needs only §344's `state:=mid_line` (add_delims_to); with
                // encTeX's \mubytein = 0, read_buffer is buffer[loc] plus its resets
                let loc = self.curinput.locfield;
                if loc <= self.curinput.limitfield && ix(self.eqtb, 29338).int() == 0 {
                    let c = *ix(self.buffer, loc);
                    let cat = ix(self.eqtb, 27741 + c as i32).hh_rh();
                    if cat == 11 || cat == 12 {
                        self.mubyteskip = 0;
                        self.mubytetoken = 0;
                        if self.mubytekeep > 0 {
                            self.mubytekeep = 0;
                        }
                        self.curcs = 0;
                        self.curinput.locfield = loc + 1;
                        self.curchr = c as i32;
                        self.curcmd = cat as u8;
                        self.curinput.statefield = 1; // mid_line
                        return;
                    }
                }
                break;
            }
            let loc = self.curinput.locfield;
            if loc == NULL {
                self.curcs = 0;
                self.endtokenlist();
                continue;
            }
            let w = *ix(self.mem, loc);
            let t = w.hh_lh();
            if t >= CS_TOKEN_FLAG {
                let cs = t - CS_TOKEN_FLAG;
                let e = *ix(self.eqtb, cs);
                let cmd = (e.hh_b0()) as u8;
                let c = cmd as i32;
                if c < OUTER_CALL
                    && !self.writenoexpanding
                    && !(c <= CAR_RET && c >= TAB_MARK && self.alignstate == 0)
                {
                    self.curinput.locfield = w.hh_rh();
                    self.curcs = cs;
                    self.curcmd = cmd;
                    self.curchr = e.hh_rh();
                    return;
                }
                break;
            }
            let c = ((t / 256) as u8) as i32;
            if c == CAR_RET {
                self.curcs = 0;
                self.curinput.locfield = w.hh_rh();
                self.curcmd = (t / 256) as u8;
                self.curchr = t % 256;
                let p = *ix(self.paramstack, (self.curinput.limitfield + self.curchr) - 1);
                self.begintokenlist(p, PARAMETER);
                continue;
            }
            if c == TAB_MARK && self.alignstate == 0 {
                break;
            }
            self.curinput.locfield = w.hh_rh();
            self.curcs = 0;
            self.curcmd = (t / 256) as u8;
            self.curchr = t % 256;
            if c == LEFT_BRACE {
                self.alignstate += 1;
            } else if c == RIGHT_BRACE {
                self.alignstate -= 1;
            }
            return;
        }
        self.getnext_full()
    }
}

impl Globals {
    /// pdftex.web divide_scaled(s, m, dd): s/m rounded to dd decimal digits, also setting
    /// scaled_out. The generated routine does the long division one digit at a time (dd
    /// divisions); for the usual arguments the result is exactly floor(s*10^dd/m) with
    /// the same i32 wrap-around, computed here with one 64-bit division. Anything unusual
    /// (s or m = i32::MIN, m = 0 or too large, which report pdfTeX errors, dd > 9) goes to
    /// the generated routine (emitted as dividescaled_full).
    pub fn dividescaled(&mut self, s0: i32, m0: i32, dd: i32) -> i32 {
        const POW10: [i64; 10] = [1, 10, 100, 1000, 10_000, 100_000, 1_000_000, 10_000_000, 100_000_000, 1_000_000_000];
        if s0 == 0 && m0 > 0 && m0 < 2147483647 / 10 && (0..=9).contains(&dd) {
            // 0 / m: quotient and remainder 0 (pdf_begin_string's usual case: no kern
            // between consecutive characters)
            self.scaledout = 0;
            return 0;
        }
        let (mut s, mut m, mut sign) = (s0, m0, 1i32);
        if s < 0 {
            sign = -sign;
            s = s.wrapping_neg();
        }
        if m < 0 {
            sign = -sign;
            m = m.wrapping_neg();
        }
        if s < 0 || m <= 0 || m >= 2147483647 / 10 || !(0..=9).contains(&dd) {
            return self.dividescaled_full(s0, m0, dd);
        }
        let x = s as i64 * POW10[dd as usize];
        // q_k = 10*q_{k-1} + digit wraps mod 2^32 exactly like the truncated exact quotient
        let (qq, rr) = self.udivrem(x as u64, m as u32);
        let mut q = qq as i32;
        let mut r = rr as i32;
        if 2 * r >= m {
            q = q.wrapping_add(1);
            r -= m;
        }
        // r div ten_pow[dd] (ten_pow[k] = 10^k), as divisions by constants
        let rq = match dd {
            0 => r,
            1 => r / 10,
            2 => r / 100,
            3 => r / 1000,
            4 => r / 10_000,
            5 => r / 100_000,
            6 => r / 1_000_000,
            7 => r / 10_000_000,
            8 => r / 100_000_000,
            _ => r / 1_000_000_000,
        };
        debug_assert_eq!(rq, r / *ix(self.tenpow, dd));
        self.scaledout = sign.wrapping_mul(s.wrapping_sub(rq));
        debug_assert_eq!(sign.wrapping_mul(q), {
            let save = self.scaledout;
            let v = self.dividescaled_full(s0, m0, dd);
            assert_eq!(save, self.scaledout);
            v
        });
        sign.wrapping_mul(q)
    }
}

impl Globals {
    /// a[lo..=hi] := a[src], for a range of freshly allocated (so all-zero) memory, as
    /// load_fmt_file initializes the hash_extra parts of hash and eqtb
    /// (tools/gen_patches.py). A zero template needs no writes at all (the pages stay
    /// untouched); otherwise one tight fill instead of the generated per-element loop.
    pub fn fill_fresh_from<T: Copy + 'static>(&mut self, a: Ptr<T>, src: i32, lo: i32, hi: i32) {
        if hi < lo {
            return;
        }
        let v = *ix(a, src);
        let n = (hi - lo + 1) as usize;
        let dst = slice(a, lo, n);
        let vb = unsafe { std::slice::from_raw_parts(&v as *const T as *const u8, std::mem::size_of::<T>()) };
        if vb.iter().all(|&b| b == 0) {
            debug_assert!(unsafe { std::slice::from_raw_parts(dst.as_ptr() as *const u8, std::mem::size_of_val(dst)) }
                .iter()
                .all(|&b| b == 0));
            return;
        }
        dst.fill(v);
    }
}

impl Globals {
    /// eqtb[lo..=hi] := eqtb[src] for the hash_extra part of eqtb (beyond eqtb_size), done
    /// lazily: an entry there is only ever used after id_lookup has made it the eqtb entry
    /// of a new control sequence (hash_high += 1), and id_lookup then calls
    /// eqtb_lazy_ensure for exactly that index. Entries undumped from a format
    /// (eqtb_size+1..eqtb_size+hash_high) are below the first index ever ensured, so they
    /// are never overwritten. Saves writing hash_extra words (4.8 MB) per run.
    pub fn eqtb_lazy_from(&mut self, src: i32, lo: i32, hi: i32) {
        self.rt.eqtb_template = *ix(self.eqtb, src);
        self.rt.eqtb_ready = lo - 1;
        self.rt.eqtb_lazy_hi = hi;
    }

    #[inline]
    pub fn eqtb_lazy_ensure(&mut self, i: i32) {
        if i > self.rt.eqtb_ready && i <= self.rt.eqtb_lazy_hi {
            *ix(self.eqtb, i) = self.rt.eqtb_template;
            self.rt.eqtb_ready = i;
        }
    }
}

/// zround(s / 10.0) for an integer s (round half away from zero), in integers.
#[inline]
pub fn round_div10(s: i32) -> i32 {
    let s = s as i64;
    (if s >= 0 { (s + 5) / 10 } else { -((-s + 5) / 10) }) as i32
}

impl Globals {
    /// effective_char(err_p, f, c) (pdftex.web): c itself unless MLTeX is enabled; the
    /// generated routine (effectivechar_full) handles MLTeX's character substitution.
    #[inline(always)]
    pub fn effectivechar(&mut self, errp: bool, f: i32, c: u8) -> i32 {
        if !self.mltexenabledp {
            return c as i32;
        }
        self.effectivechar_full(errp, f, c)
    }

    /// (x div d, x mod d) for x < 2^63, 0 < d < 2^31, with a cached reciprocal of d
    /// instead of a hardware 64-bit division (divide_scaled divides by the same font
    /// size for every character).
    #[inline]
    fn udivrem(&mut self, x: u64, d: u32) -> (u64, u64) {
        // a few divisors alternate (font sizes, one_hundred_bp): small direct-mapped cache
        let slot = ((d as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 61) as usize;
        let c = &mut self.rt.divcache[slot];
        if c.0 != d {
            *c = (d, u64::MAX / d as u64);
        }
        let inv = c.1;
        let d = d as u64;
        // q <= x div d and x div d - q <= 2, so at most two corrections
        let mut q = ((x as u128 * inv as u128) >> 64) as u64;
        let mut r = x - q * d;
        while r >= d {
            q += 1;
            r -= d;
        }
        (q, r)
    }
}

impl Globals {
    /// pdftex.web adv_char_width(f, c). With pdf_cur_Tma = 0 (no horizontal scaling) its
    /// result is divide_scaled(w, pdf_font_size[f], 4) for the character's width w, the
    /// same for every occurrence of a character, so it is memoized per (font, character)
    /// and validated against w and the font size on every use. All effects are those of
    /// the generated routine (advcharwidth_full): pdf_delta_h, adv_char_width_s,
    /// adv_char_width_s_out and scaled_out (set by divide_scaled).
    pub fn advcharwidth(&mut self, f: i32, c: u8) {
        // the same order of evaluation as the generated routine: the width (through
        // effective_char, which may warn under MLTeX), then is_scalable
        let ec = self.effectivechar(true, f, c);
        let ci = ix(self.fontinfo, *ix(self.charbase, f) + ec).qqqq_b0() as i32;
        let w = ix(self.fontinfo, *ix(self.widthbase, f) + ci).int();
        if !self.isscalable(f) {
            self.pdfdeltah += self.getpkcharwidth(f, w);
            return;
        }
        let size = *ix(self.pdffontsize, f);
        if self.pdfcurTma != 0 {
            let a3 = self.roundxnoverd(w, 1000, 1000 + self.pdfcurTma);
            let s = self.dividescaled(a3, size, 4);
            let a6 = self.roundxnoverd(size, s.wrapping_abs(), 10000);
            let mut sout = self.roundxnoverd(a6, 1000 + self.pdfcurTma, 1000);
            if s < 0 {
                sout = -sout;
            }
            self.pdfdeltah += sout;
            self.advcharwidths = round_div10(s);
            self.advcharwidthsout = sout;
            return;
        }
        let fi = f as usize;
        let hit = match self.rt.advcache.get(fi).and_then(|t| t.as_ref()) {
            Some(t) => {
                let e = t[c as usize];
                if e.1 == size && e.0 == w && size != 0 {
                    Some((e.2, e.3))
                } else {
                    None
                }
            }
            None => None,
        };
        let (s, sout) = match hit {
            Some(v) => {
                self.scaledout = v.1;
                v
            }
            None => {
                let s = self.dividescaled(w, size, 4);
                let sout = self.scaledout;
                if self.rt.advcache.len() <= fi {
                    self.rt.advcache.resize_with(fi + 1, || None);
                }
                let t = self.rt.advcache[fi].get_or_insert_with(|| Box::new([(0, 0, 0, 0); 256]));
                t[c as usize] = (w, size, s, sout);
                (s, sout)
            }
        };
        self.pdfdeltah += sout;
        self.advcharwidths = round_div10(s);
        self.advcharwidthsout = sout;
    }
}

impl Globals {
    /// effective_char_info(f, c) (pdftex.web): char_info(f)(c) unless MLTeX is enabled
    /// (the generated routine, effectivecharinfo_full, handles character substitution).
    #[inline(always)]
    pub fn effectivecharinfo(&mut self, f: i32, c: u8) -> FourQuarters {
        if !self.mltexenabledp {
            return ix(self.fontinfo, *ix(self.charbase, f) + c as i32).qqqq();
        }
        self.effectivecharinfo_full(f, c)
    }
}

impl Globals {
    /// encTeX's read_buffer(i): with \mubytein = 0 (always, unless encTeX is used) just
    /// buffer[i], after the same resets as the generated routine (readbuffer_full).
    #[inline(always)]
    pub fn readbuffer(&mut self, i: *mut i32) -> u8 {
        if ix(self.eqtb, 29338).int() == 0 {
            self.mubyteskip = 0;
            self.mubytetoken = 0;
            if self.mubytekeep > 0 {
                self.mubytekeep = 0;
            }
            return *ix(self.buffer, unsafe { *i });
        }
        self.readbuffer_full(i)
    }
}
