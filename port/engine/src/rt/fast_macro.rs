//! Register-resident inner loop of macro_call (tex.web §399, "Contribute an entire group
//! to the current parameter"): copying a braced macro argument token by token.
//!
//! LaTeX3 code passes most code as braced arguments through many layers of macros, so
//! this loop runs for a large share of all tokens. The generated version keeps `loc`,
//! `avail`, `dyn_used` and `align_state` in Globals fields, so every token costs several
//! store/reload round trips on the two pointer chains (the input list and the free list).
//! This version holds them in registers while the tokens come from the current token
//! list, and writes everything back (including cur_cmd/cur_chr/cur_cs/cur_tok of the last
//! token) before any other code runs: before the general get_token, before get_avail when
//! the free list is empty, and on exit. Node allocation order, dyn_used and every other
//! effect are exactly those of the generated loop.
//!
//! The rest of macro_call is the generated code itself, copied by tools/gen_fast_macro.py
//! into rt/fast_macro_gen.rs with this loop substituted.

use super::*;
use crate::gen::globals::Globals;

const NULL: i32 = -268435455;
const CS_TOKEN_FLAG: i32 = 4095;
const OUTER_CALL: i32 = 116;
const LONG_CALL: i32 = 115;
const LEFT_BRACE: i32 = 1;
const RIGHT_BRACE: i32 = 2;
const TAB_MARK: i32 = 4;
const CAR_RET: i32 = 5; // = out_param for character tokens
const LEFT_BRACE_LIMIT: i32 = 512;
const FROZEN_CONTROL_SEQUENCE: i32 = 15514;
const EQTB_SIZE: i32 = 30192;

impl Globals {
    /// eq_type(p) := t was just done (eq_define, geq_define, primitive): remember whether a
    /// control sequence outside the frozen ones now has a meaning >= outer_call
    #[inline]
    pub fn note_eq_type(&mut self, p: i32, t: i32) {
        if t >= OUTER_CALL && (p < FROZEN_CONTROL_SEQUENCE || p > EQTB_SIZE) {
            self.rt.user_outer = true;
        }
    }

    /// After loading a format: the same for every meaning it contains.
    pub fn scan_outer_meanings(&mut self) {
        let hi = EQTB_SIZE + self.hashhigh;
        let found = (1..FROZEN_CONTROL_SEQUENCE)
            .chain(EQTB_SIZE + 1..=hi)
            .any(|p| ix(self.eqtb, p).hh_b0() as i32 >= OUTER_CALL);
        if found {
            self.rt.user_outer = true;
        }
    }
}
const RIGHT_BRACE_LIMIT: i32 = 768;

impl Globals {
    /// tex.web §399 with `p` the tail of the parameter being built and cur_tok the left
    /// brace just read. Returns true when the matching right brace has been read (it is
    /// in cur_tok, not yet stored), false when a \par token ended the group in a macro
    /// that is not \long (cur_tok = par_token; the caller reports the runaway argument).
    /// In both cases all state is back in Globals.
    pub fn scan_group_fast(&mut self, p: &mut i32, unbalance: &mut i32) -> bool {
        let mem = self.mem;
        let eqtb = self.eqtb;
        let mut avail = self.avail;
        let mut dynused = self.dynused;
        let mut align = self.alignstate;
        let mut loc = self.curinput.locfield;
        let mut tok = self.curtok;
        let mut partoken = self.partoken;
        let mut notlong = self.longstate as i32 != LONG_CALL;
        let mut wne = self.writenoexpanding;
        let no_outer = !self.rt.user_outer;
        // the last token was read here (true) or by get_token (false); in the first case
        // cur_cmd/cur_chr/cur_cs/cur_tok still have to be stored
        let mut last_fast = false;
        let mut pp = *p;
        let mut appended = 0i32;
        *unbalance = 1;
        loop {
            // fast_store_new_token(cur_tok)
            let q;
            if avail == NULL {
                self.curinput.locfield = loc;
                self.alignstate = align;
                self.avail = avail;
                self.dynused = dynused;
                if last_fast {
                    self.set_cur_from_token(tok);
                    last_fast = false;
                }
                q = self.getavail();
                avail = self.avail;
                dynused = self.dynused;
            } else {
                q = avail;
                avail = ix(mem, q).hh_rh();
                ix(mem, q).set_hh_rh(NULL);
                dynused += 1;
            }
            ix(mem, pp).set_hh_rh(q);
            ix(mem, q).set_hh_lh(tok);
            pp = q;
            appended += 1;

            // get_token, inline for the common token-list cases (see rt/fast.rs getnext)
            let mut fast = false;
            if self.curinput.statefield == 0 && loc != NULL {
                let w = ix(mem, loc);
                let t = w.hh_lh();
                if t > CS_TOKEN_FLAG {
                    let cs = t - CS_TOKEN_FLAG;
                    // the meaning is needed only to catch outer meanings (only the frozen
                    // control sequences have them unless some definition made one) and,
                    // with align_state = 0, alignment entries; otherwise skip the lookup
                    if no_outer && align != 0 && !wne && (cs < FROZEN_CONTROL_SEQUENCE || cs > EQTB_SIZE) {
                        loc = w.hh_rh();
                        tok = t;
                        fast = true;
                    } else {
                        let cmd = (ix(eqtb, cs).hh_b0() as u8) as i32;
                        if cmd < OUTER_CALL && !wne && !(cmd <= CAR_RET && cmd >= TAB_MARK && align == 0) {
                            loc = w.hh_rh();
                            tok = t;
                            fast = true;
                        }
                    }
                } else if t < CS_TOKEN_FLAG {
                    let c = ((t / 256) as u8) as i32;
                    if c != CAR_RET && !(c == TAB_MARK && align == 0) {
                        loc = w.hh_rh();
                        tok = t;
                        fast = true;
                        if c == LEFT_BRACE {
                            align += 1;
                        } else if c == RIGHT_BRACE {
                            align -= 1;
                        }
                    }
                }
            }
            if fast {
                last_fast = true;
            } else {
                self.curinput.locfield = loc;
                self.alignstate = align;
                self.avail = avail;
                self.dynused = dynused;
                if last_fast {
                    self.set_cur_from_token(tok);
                    last_fast = false;
                }
                self.gettoken();
                loc = self.curinput.locfield;
                align = self.alignstate;
                avail = self.avail;
                dynused = self.dynused;
                tok = self.curtok;
                partoken = self.partoken;
                notlong = self.longstate as i32 != LONG_CALL;
                wne = self.writenoexpanding;
            }

            let done = if tok == partoken && notlong {
                Some(false)
            } else if tok < RIGHT_BRACE_LIMIT {
                if tok < LEFT_BRACE_LIMIT {
                    *unbalance += 1;
                    None
                } else {
                    *unbalance -= 1;
                    if *unbalance == 0 {
                        Some(true)
                    } else {
                        None
                    }
                }
            } else {
                None
            };
            if let Some(r) = done {
                self.curinput.locfield = loc;
                self.alignstate = align;
                self.avail = avail;
                self.dynused = dynused;
                if last_fast {
                    self.set_cur_from_token(tok);
                }
                *p = pp;
                self.rt.pcount += appended;
                return r;
            }
        }
    }

    /// cur_cs, cur_cmd, cur_chr and cur_tok as get_token leaves them after reading the
    /// token `t` from a token list (only for tokens the fast paths accept: t <> cs_token_flag).
    #[inline(always)]
    fn set_cur_from_token(&mut self, t: i32) {
        if t > CS_TOKEN_FLAG {
            let cs = t - CS_TOKEN_FLAG;
            let e = ix(self.eqtb, cs);
            self.curcs = cs;
            self.curcmd = e.hh_b0() as u8;
            self.curchr = e.hh_rh();
        } else {
            self.curcs = 0;
            self.curcmd = (t / 256) as u8;
            self.curchr = t % 256;
        }
        self.curtok = t;
    }
}

// ---------------------------------------------------------------------------------------
// Parameter lists: macro_call records the tail and node count of every parameter it
// builds (pstack_meta, then pmeta alongside param_stack), so that end_token_list can free
// a parameter in O(1) instead of flush_list's walk, with the same result: the list goes
// to the head of the free list unchanged and dyn_used drops by its length. Parameter
// lists are never changed between macro_call and end_token_list (they are only read,
// through begin_token_list(..., parameter)). Verify builds walk every list to check.

impl Globals {
    pub fn set_pmeta(&mut self, k: i32, meta: (i32, i32)) {
        let k = k as usize;
        if k >= self.rt.pmeta.len() {
            self.rt.pmeta.resize(k + 1, (NULL, 0));
        }
        self.rt.pmeta[k] = meta;
    }

    /// flush_list(param_stack[k]) using the recorded tail and length.
    #[inline]
    fn flush_param(&mut self, k: i32) {
        let head = *ix(self.paramstack, k);
        if head == NULL {
            return;
        }
        let (tail, len) = self.rt.pmeta[k as usize];
        #[cfg(debug_assertions)]
        {
            let (mut q, mut r, mut n) = (head, head, 0);
            while r != NULL {
                q = r;
                r = ix(self.mem, r).hh_rh();
                n += 1;
            }
            assert_eq!((q, n), (tail, len), "parameter list metadata");
        }
        ix(self.mem, tail).set_hh_rh(self.avail);
        self.avail = head;
        self.dynused -= len;
    }

    /// tex.web §324 end_token_list (the generated code, gen/r04.rs endtokenlist_full,
    /// with the parameter flush done by flush_param)
    pub fn endtokenlist(&mut self) {
        let index = self.curinput.indexfield as i32;
        if index >= 3 {
            if index <= 4 {
                self.flushlist(self.curinput.startfield);
            } else {
                self.deletetokenref(self.curinput.startfield);
                if index == 5 {
                    while self.paramptr > self.curinput.limitfield {
                        self.paramptr -= 1;
                        self.flush_param(self.paramptr);
                    }
                } else if index == 6 && !self.outputcanend {
                    self.fatalerror(688);
                }
            }
        } else if index == 1 {
            if self.alignstate > 500000 {
                self.alignstate = 0;
            } else {
                self.fatalerror(689);
            }
        }
        self.inputptr -= 1;
        self.curinput = *ix(self.inputstack, self.inputptr);
        if self.interrupt != 0 {
            self.pauseforinstructions();
        }
    }
}

impl Globals {
    /// tex.web §494 pass_text: skip tokens without expansion until the \fi or \else (or
    /// \or) that ends the current conditional level. The generated loop calls get_next
    /// for every token; this one reads plain tokens of the current token list directly,
    /// keeping loc, align_state and the nesting level in registers, and lets get_next
    /// handle every token that needs more than that (end of the list, file input, outer
    /// macros, encTeX's write_noexpanding, alignment entries, parameters). State is
    /// written back before any other code runs, so the effects are those of the
    /// generated loop.
    pub fn passtext(&mut self) {
        const FI_OR_ELSE: i32 = 109;
        const IF_TEST: i32 = 108;
        const FI_CODE: i32 = 2;
        let savescannerstatus = self.scannerstatus;
        self.scannerstatus = 1; // skipping
        let mut l: i32 = 0;
        self.skipline = self.line;
        'done: loop {
            if self.curinput.statefield == 0 {
                let mem = self.mem;
                let eqtb = self.eqtb;
                let wne = self.writenoexpanding;
                let mut loc = self.curinput.locfield;
                let mut align = self.alignstate;
                let mut last: Option<i32> = None;
                let mut finished = false;
                while loc != NULL {
                    let w = ix(mem, loc);
                    let t = w.hh_lh();
                    if t > CS_TOKEN_FLAG {
                        let e = ix(eqtb, t - CS_TOKEN_FLAG);
                        let cmd = (e.hh_b0() as u8) as i32;
                        if cmd >= OUTER_CALL || wne || (cmd <= CAR_RET && cmd >= TAB_MARK && align == 0) {
                            break;
                        }
                        loc = w.hh_rh();
                        last = Some(t);
                        if cmd == FI_OR_ELSE {
                            if l == 0 {
                                finished = true;
                                break;
                            }
                            if e.hh_rh() == FI_CODE {
                                l -= 1;
                            }
                        } else if cmd == IF_TEST {
                            l += 1;
                        }
                    } else if t < CS_TOKEN_FLAG {
                        let c = ((t / 256) as u8) as i32;
                        if c == CAR_RET || (c == TAB_MARK && align == 0) {
                            break;
                        }
                        loc = w.hh_rh();
                        last = Some(t);
                        if c == LEFT_BRACE {
                            align += 1;
                        } else if c == RIGHT_BRACE {
                            align -= 1;
                        }
                    } else {
                        break;
                    }
                }
                self.curinput.locfield = loc;
                self.alignstate = align;
                if let Some(t) = last {
                    self.set_cur_from_token(t);
                }
                if finished {
                    break 'done;
                }
            }
            // one token the general way
            self.getnext();
            if self.curcmd as i32 == FI_OR_ELSE {
                if l == 0 {
                    break 'done;
                }
                if self.curchr == FI_CODE {
                    l -= 1;
                }
            } else if self.curcmd as i32 == IF_TEST {
                l += 1;
            }
        }
        self.scannerstatus = savescannerstatus;
        if ix(self.eqtb, 29381).int() > 0 {
            self.showcurcmdchr();
        }
    }
}
