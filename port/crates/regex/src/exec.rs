//! Port of `regexec.c` (non-I18N configuration).

use crate::compile::Compiled;
use crate::internal::*;

/// `regmatch_t` (regoff_t is `int`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RegMatch {
    pub rm_so: i32,
    pub rm_eo: i32,
}

const UNSET: RegMatch = RegMatch { rm_so: -1, rm_eo: -1 };

/// Maximum nesting of the back-reference sifting recursion.  The C engine
/// in pdftex.dll (5 MB stack) runs out of stack at roughly 13000-14000
/// levels, where malloc starts failing and regexec returns REG_ESPACE; past
/// this depth the port returns REG_ESPACE as well.
const SIFT_DEPTH_LIMIT: u32 = 12000;

/// A point where the C engine (as built into TeX Live's pdfTeX) does not
/// return: pdfTeX aborts, crashes, or (for [`ExecAbort::StepLimit`]) may run
/// forever.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecAbort {
    /// A live `assert()` in regexec.c fails; the C program calls `abort()`.
    Assertion {
        /// The asserted expression.
        expr: &'static str,
        /// Line in TeX Live's regexec.c.
        line: u32,
    },
    /// The C code dereferences a NULL `re_dfastate_t *` (a crash).
    NullState {
        /// The C expression that is dereferenced.
        site: &'static str,
    },
    /// The step budget passed to [`crate::Regex::try_exec_limited`] was
    /// exhausted.  The C engine would keep going (it loops forever on some
    /// back-reference patterns).
    StepLimit,
}

impl std::fmt::Display for ExecAbort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecAbort::Assertion { expr, line } => write!(f, "Assertion failed: {expr}, file regexec.c, line {line}"),
            ExecAbort::NullState { site } => write!(f, "NULL pointer dereference ({site}) in regexec.c"),
            ExecAbort::StepLimit => write!(f, "regex step limit exceeded"),
        }
    }
}

impl std::error::Error for ExecAbort {}

#[derive(Clone, Debug, Default)]
struct BkrefEnt {
    node: i32,
    str_idx: i32,
    subexp_from: i32,
    subexp_to: i32,
    more: bool,
    /// `unsigned short` in C.
    eps_reachable_subexps_map: u16,
}

#[derive(Clone, Debug, Default)]
struct StateArray {
    next_idx: i32,
    alloc: i32,
    array: Vec<StatePtr>,
}

#[derive(Debug, Default)]
struct SubLast {
    node: i32,
    str_idx: i32,
    path: StateArray,
}

#[derive(Debug, Default)]
struct SubTop {
    str_idx: i32,
    node: i32,
    path: Option<StateArray>,
    lasts: Vec<SubLast>,
}

struct MatchCtx<'a> {
    dfa: &'a Dfa,
    cache: &'a mut StateCache,
    input: InputString<'a>,
    eflags: i32,
    match_last: i32,
    last_node: i32,
    state_log: Option<Vec<StatePtr>>,
    state_log_top: i32,
    bkref_ents: Vec<BkrefEnt>,
    max_mb_elem_len: i32,
    sub_tops: Vec<SubTop>,
    /// Nesting of sift_states_bkref -> sift_states_backward (not in C; see
    /// SIFT_DEPTH_LIMIT).
    sift_depth: u32,
    /// Work counter for the optional step budget (not part of the C code).
    steps: u64,
    step_limit: u64,
}

struct SiftCtx {
    last_node: i32,
    last_str_idx: i32,
    limits: NodeSet,
}

struct FailStackEnt {
    idx: i32,
    node: i32,
    regs: Vec<RegMatch>,
    eps_via_nodes: NodeSet,
}

#[inline]
fn arr_get(a: &[StatePtr], i: i32) -> StatePtr {
    if i < 0 {
        None
    } else {
        a.get(i as usize).copied().flatten()
    }
}

#[inline]
fn arr_set(a: &mut Vec<StatePtr>, i: i32, v: StatePtr) {
    let i = i as usize;
    if i >= a.len() {
        a.resize(i + 1, None);
    }
    a[i] = v;
}

#[inline]
fn reg_get(regs: &[RegMatch], i: i32) -> RegMatch {
    // Reading past the caller's pmatch array is undefined behaviour in C;
    // treat such registers as unset.
    if i < 0 {
        UNSET
    } else {
        regs.get(i as usize).copied().unwrap_or(UNSET)
    }
}

impl<'a> MatchCtx<'a> {
    /// Count work against the optional step budget.
    #[inline]
    fn tick_n(&mut self, n: u64) -> Result<(), ExecAbort> {
        self.steps = self.steps.saturating_add(n);
        if self.steps > self.step_limit {
            Err(ExecAbort::StepLimit)
        } else {
            Ok(())
        }
    }

    #[inline]
    fn sl(&self, i: i32) -> StatePtr {
        match &self.state_log {
            Some(v) => arr_get(v, i),
            None => None,
        }
    }

    #[inline]
    fn sl_set(&mut self, i: i32, v: StatePtr) {
        if let Some(log) = &mut self.state_log {
            arr_set(log, i, v);
        }
    }

    #[inline]
    fn st(&self, id: StateId) -> &State {
        self.cache.st(id)
    }

    #[inline]
    fn context_at(&self, idx: i32) -> u32 {
        self.input.context_at(idx, self.eflags)
    }

    #[inline]
    fn acquire_state(&mut self, nodes: &NodeSet) -> StatePtr {
        self.cache.acquire_state(self.dfa, nodes)
    }

    #[inline]
    fn acquire_state_context(&mut self, nodes: &NodeSet, context: u32) -> StatePtr {
        self.cache.acquire_state_context(self.dfa, nodes, context)
    }
}

/// re_search_internal.  Returns the reg_errcode_t.
#[allow(clippy::too_many_arguments)]
pub(crate) fn re_search_internal(
    preg: &Compiled,
    cache: &mut StateCache,
    string: &[u8],
    length: i32,
    start: i32,
    range: i32,
    stop: i32,
    nmatch: usize,
    pmatch: &mut [RegMatch],
    eflags: i32,
    step_limit: u64,
) -> Result<RegErrcode, ExecAbort> {
    let dfa = &preg.dfa;
    let mut start = start;
    let mut range = range;
    let mut match_last;
    let mut nmatch = nmatch;

    let fastmap: Option<&[u8; SBC_MAX]> =
        if preg.fastmap_accurate && range != 0 && !preg.can_be_null { Some(&preg.fastmap) } else { None };

    let extra_nmatch = if nmatch > preg.re_nsub { nmatch - (preg.re_nsub + 1) } else { 0 };
    nmatch -= extra_nmatch;

    // Check if the DFA haven't been compiled.
    let (is, isw, isn, isb) = match (dfa.init_state, dfa.init_state_word, dfa.init_state_nl, dfa.init_state_begbuf) {
        (Some(a), Some(b), Some(c), Some(d)) => (a, b, c, d),
        _ => return Ok(REG_NOMATCH),
    };
    let _ = isb;

    // If initial states with non-begbuf contexts have no elements, the regex
    // must be anchored.
    if cache.st(is).nodes.nelem == 0
        && cache.st(isw).nodes.nelem == 0
        && (cache.st(isn).nodes.nelem == 0 || !preg.newline_anchor)
    {
        if start != 0 && start + range != 0 {
            return Ok(REG_NOMATCH);
        }
        start = 0;
        range = 0;
    }

    // We must check the longest matching, if nmatch > 0.
    let fl_longest_match = nmatch != 0 || dfa.nbackref != 0;

    let mut input = InputString::allocate(string, length, dfa.nodes.len() as i32 + 1, preg.syntax & RE_ICASE != 0, dfa);
    input.stop = stop;
    input.raw_stop = stop;
    input.newline_anchor = preg.newline_anchor;

    let mut mctx = MatchCtx {
        dfa,
        cache,
        input,
        eflags,
        match_last: -1,
        last_node: 0,
        state_log: None,
        state_log_top: 0,
        bkref_ents: Vec::new(),
        max_mb_elem_len: 1,
        sub_tops: Vec::new(),
        sift_depth: 0,
        steps: 0,
        step_limit,
    };

    if nmatch > 1 || dfa.has_mb_node {
        mctx.state_log = Some(vec![None; (mctx.input.bufs_len + 1) as usize]);
    }

    let mut match_first = start;
    mctx.input.tip_context =
        if eflags & REG_NOTBOL != 0 { CONTEXT_BEGBUF } else { CONTEXT_NEWLINE | CONTEXT_BEGBUF };

    // Check incrementally whether of not the input string match.
    let incr: i32 = if range < 0 { -1 } else { 1 };
    let left_lim = if range < 0 { start + range } else { start };
    let right_lim = if range < 0 { start } else { start + range };
    let sb = dfa.mb_cur_max == 1;
    let t_present = false; // preg->translate is always NULL after regcomp
    let match_kind: i32 = if fastmap.is_some() {
        (if sb || !(preg.syntax & RE_ICASE != 0 || t_present) { 4 } else { 0 })
            | (if range >= 0 { 2 } else { 0 })
            | (if t_present { 1 } else { 0 })
    } else {
        8
    };
    let raw = |i: i32| -> u8 { string.get(i as usize).copied().unwrap_or(0) };

    let mut err;
    'outer: loop {
        err = REG_NOMATCH;
        if match_first < left_lim || right_lim < match_first {
            break 'outer;
        }

        match match_kind {
            8 => {}
            6 | 7 => {
                let fm = fastmap.unwrap();
                while match_first < right_lim && fm[raw(match_first) as usize] == 0 {
                    match_first += 1;
                }
                if match_first == right_lim {
                    let ch = if match_first >= length { 0 } else { raw(match_first) };
                    if fm[ch as usize] == 0 {
                        break 'outer;
                    }
                }
            }
            4 | 5 => {
                let fm = fastmap.unwrap();
                while match_first >= left_lim {
                    let ch = if match_first >= length { 0 } else { raw(match_first) };
                    if fm[ch as usize] != 0 {
                        break;
                    }
                    match_first -= 1;
                }
                if match_first < left_lim {
                    break 'outer;
                }
            }
            _ => {
                let fm = fastmap.unwrap();
                loop {
                    let mut offset = (match_first - mctx.input.raw_mbs_idx) as u32;
                    if offset >= mctx.input.valid_raw_len as u32 {
                        mctx.input.reconstruct(match_first, eflags);
                        offset = (match_first - mctx.input.raw_mbs_idx) as u32;
                    }
                    let ch = if match_first >= length { 0 } else { mctx.input.byte_at(offset as i32) };
                    if fm[ch as usize] != 0 {
                        break;
                    }
                    match_first += incr;
                    if match_first < left_lim || match_first > right_lim {
                        err = REG_NOMATCH;
                        break 'outer;
                    }
                }
            }
        }

        // Reconstruct the buffers so that the matcher can assume that the
        // matching starts from the beginning of the buffer.
        mctx.input.reconstruct(match_first, eflags);

        mctx.state_log_top = 0;
        mctx.bkref_ents.clear();
        mctx.max_mb_elem_len = 0;
        match_last = check_matching(&mut mctx, fl_longest_match, if range >= 0 { Some(&mut match_first) } else { None });
        if match_last != -1 {
            if match_last == -2 {
                err = REG_ESPACE;
                break 'outer;
            } else {
                mctx.match_last = match_last;
                if (!preg.no_sub && nmatch > 1) || dfa.nbackref != 0 {
                    let pstate = mctx.sl(match_last).ok_or(ExecAbort::NullState { site: "mctx.state_log[match_last]" })?;
                    mctx.last_node = check_halt_state_context(&mctx, pstate, match_last);
                }
                if (!preg.no_sub && nmatch > 1 && dfa.has_plural_match) || dfa.nbackref != 0 {
                    err = prune_impossible_nodes(&mut mctx);
                    if err == REG_NOERROR {
                        break 'outer;
                    }
                    if err != REG_NOMATCH {
                        break 'outer;
                    }
                } else {
                    err = REG_NOERROR;
                    break 'outer; // We found a match.
                }
            }
        }

        match_ctx_clean(&mut mctx);
        match_first += incr;
    }

    if err != REG_NOERROR {
        return Ok(err);
    }

    // Set pmatch[] if we need.
    if nmatch > 0 {
        for reg in pmatch.iter_mut().take(nmatch).skip(1) {
            *reg = UNSET;
        }
        pmatch[0].rm_so = 0;
        pmatch[0].rm_eo = mctx.match_last;

        if !preg.no_sub && nmatch > 1 {
            let e = set_regs(&mut mctx, nmatch, pmatch, dfa.has_plural_match && dfa.nbackref > 0)?;
            if e != REG_NOERROR {
                return Ok(e);
            }
        }

        // At last, add the offset to the each registers.
        for reg in pmatch.iter_mut().take(nmatch) {
            if reg.rm_so != -1 {
                reg.rm_so += match_first;
                reg.rm_eo += match_first;
            }
        }
        for reg_idx in 0..extra_nmatch {
            pmatch[nmatch + reg_idx] = UNSET;
        }

        if let Some(map) = &dfa.subexp_map {
            let mut reg_idx = 0usize;
            while reg_idx + 1 < nmatch {
                if map[reg_idx] != reg_idx as i32 {
                    pmatch[reg_idx + 1] = pmatch[map[reg_idx] as usize + 1];
                }
                reg_idx += 1;
            }
        }
    }
    Ok(REG_NOERROR)
}

/// prune_impossible_nodes
fn prune_impossible_nodes(mctx: &mut MatchCtx) -> RegErrcode {
    let mut match_last = mctx.match_last;
    let mut halt_node = mctx.last_node;
    let mut sifted_states: Vec<StatePtr> = vec![None; (match_last + 1) as usize];
    if mctx.dfa.nbackref != 0 {
        let mut lim_states: Vec<StatePtr> = vec![None; (match_last + 1) as usize];
        loop {
            for e in lim_states.iter_mut().take((match_last + 1) as usize) {
                *e = None;
            }
            let mut sctx = SiftCtx { last_node: halt_node, last_str_idx: match_last, limits: NodeSet::new_empty() };
            let ret = sift_states_backward(mctx, &mut sctx, &mut sifted_states, Some(&mut lim_states));
            if ret != REG_NOERROR {
                return ret;
            }
            if arr_get(&sifted_states, 0).is_some() || arr_get(&lim_states, 0).is_some() {
                break;
            }
            loop {
                match_last -= 1;
                if match_last < 0 {
                    return REG_NOMATCH;
                }
                match mctx.sl(match_last) {
                    None => continue,
                    Some(s) => {
                        if !mctx.st(s).halt {
                            continue;
                        }
                    }
                }
                break;
            }
            let s = mctx.sl(match_last).unwrap();
            halt_node = check_halt_state_context(mctx, s, match_last);
        }
        let ret = merge_state_array(mctx, &mut sifted_states, &lim_states, match_last + 1);
        if ret != REG_NOERROR {
            return ret;
        }
    } else {
        let mut sctx = SiftCtx { last_node: halt_node, last_str_idx: match_last, limits: NodeSet::new_empty() };
        let ret = sift_states_backward(mctx, &mut sctx, &mut sifted_states, None);
        if ret != REG_NOERROR {
            return ret;
        }
    }
    mctx.state_log = Some(sifted_states);
    mctx.last_node = halt_node;
    mctx.match_last = match_last;
    REG_NOERROR
}

/// acquire_init_state_context
fn acquire_init_state_context(mctx: &mut MatchCtx, idx: i32) -> StatePtr {
    let dfa = mctx.dfa;
    let init_state = dfa.init_state.unwrap();
    if mctx.st(init_state).has_constraint {
        let context = mctx.context_at(idx - 1);
        if is_word_context(context) {
            dfa.init_state_word
        } else if is_ordinary_context(context) {
            dfa.init_state
        } else if is_begbuf_context(context) && is_newline_context(context) {
            dfa.init_state_begbuf
        } else if is_newline_context(context) {
            dfa.init_state_nl
        } else if is_begbuf_context(context) {
            // It is relatively rare case, then calculate on demand.
            let ent = mctx.st(init_state).entrance().clone();
            mctx.acquire_state_context(&ent, context)
        } else {
            dfa.init_state
        }
    } else {
        dfa.init_state
    }
}

/// check_matching
fn check_matching(mctx: &mut MatchCtx, fl_longest_match: bool, p_match_first: Option<&mut i32>) -> i32 {
    let mut p_match_first = p_match_first;
    let mut matched = false;
    let mut match_last: i32 = -1;
    let cur_str_idx = mctx.input.cur_idx;
    let mut at_init_state = p_match_first.is_some();
    let mut next_start_idx = cur_str_idx;

    let mut cur_state = match acquire_init_state_context(mctx, cur_str_idx) {
        Some(s) => s,
        None => return -2,
    };

    if mctx.state_log.is_some() {
        mctx.sl_set(cur_str_idx, Some(cur_state));

        // Check OP_OPEN_SUBEXP in the initial state in case that we use them
        // later.  E.g. Processing back references.
        if mctx.dfa.nbackref != 0 {
            at_init_state = false;
            let nodes = mctx.st(cur_state).nodes.clone();
            check_subexp_matching_top(mctx, &nodes, 0);
            if mctx.st(cur_state).has_backref {
                transit_state_bkref(mctx, &nodes);
            }
        }
    }

    // If the RE accepts NULL string.
    if mctx.st(cur_state).halt
        && (!mctx.st(cur_state).has_constraint || check_halt_state_context(mctx, cur_state, cur_str_idx) != 0)
    {
        if !fl_longest_match {
            return cur_str_idx;
        } else {
            match_last = cur_str_idx;
            matched = true;
        }
    }

    while !mctx.input.eoi() {
        let old_state = cur_state;
        let next_char_idx = mctx.input.cur_idx + 1;

        if next_char_idx >= mctx.input.bufs_len
            || (next_char_idx >= mctx.input.valid_len && mctx.input.valid_len < mctx.input.len)
        {
            extend_buffers(mctx);
        }

        let mut next = transit_state(mctx, cur_state);
        if mctx.state_log.is_some() {
            next = merge_state_with_log(mctx, next);
        }

        let cs = match next {
            Some(s) => s,
            None => {
                // Reached the invalid state or an error.  Try to recover a
                // valid state using the state log, if available and if we have
                // not already found a valid (even if not the longest) match.
                if mctx.state_log.is_none() || (matched && !fl_longest_match) {
                    break;
                }
                match find_recover_state(mctx) {
                    Some(s) => s,
                    None => break,
                }
            }
        };
        cur_state = cs;

        if at_init_state {
            if old_state == cur_state {
                next_start_idx = next_char_idx;
            } else {
                at_init_state = false;
            }
        }

        if mctx.st(cur_state).halt {
            // Reached a halt state.  Check the halt state can satisfy the
            // current context.
            let ci = mctx.input.cur_idx;
            if !mctx.st(cur_state).has_constraint || check_halt_state_context(mctx, cur_state, ci) != 0 {
                // We found an appropriate halt state.
                match_last = mctx.input.cur_idx;
                matched = true;
                // We found a match, do not modify match_first below.
                p_match_first = None;
                if !fl_longest_match {
                    break;
                }
            }
        }
    }

    if let Some(p) = p_match_first {
        *p += next_start_idx;
    }
    match_last
}

/// check_halt_node_context
fn check_halt_node_context(dfa: &Dfa, node: i32, context: u32) -> bool {
    let ty = dfa.nodes[node as usize].ty;
    let constraint = dfa.nodes[node as usize].constraint;
    if ty != END_OF_RE {
        return false;
    }
    if constraint == 0 {
        return true;
    }
    !not_satisfy_next_constraint(constraint, context)
}

/// check_halt_state_context
fn check_halt_state_context(mctx: &MatchCtx, state: StateId, idx: i32) -> i32 {
    let context = mctx.context_at(idx);
    for &n in mctx.st(state).nodes.as_slice() {
        if check_halt_node_context(mctx.dfa, n, context) {
            return n;
        }
    }
    0
}

/// proceed_next_node.  Returns the next node, -1 for failure, -2 for error.
fn proceed_next_node(
    mctx: &MatchCtx,
    nregs: usize,
    regs: &mut [RegMatch],
    pidx: &mut i32,
    node: i32,
    eps_via_nodes: &mut NodeSet,
    fs: &mut Option<Vec<FailStackEnt>>,
) -> Result<i32, ExecAbort> {
    let dfa = mctx.dfa;
    let n = node as usize;
    if is_epsilon_node(dfa.nodes[n].ty) {
        let cur_state = mctx.sl(*pidx).ok_or(ExecAbort::NullState { site: "mctx->state_log[*pidx]->nodes" })?;
        let cur_nodes = &mctx.st(cur_state).nodes;
        let edests = &dfa.edests[n];
        eps_via_nodes.insert(node);
        let mut dest_node = -1;
        for i in 0..edests.nelem as usize {
            let candidate = edests.elems[i];
            if cur_nodes.contains(candidate) == 0 {
                continue;
            }
            if dest_node == -1 {
                dest_node = candidate;
            } else {
                // In order to avoid infinite loop like "(a*)*", return the
                // second epsilon-transition if the first was already
                // considered.
                if eps_via_nodes.contains(dest_node) != 0 {
                    return Ok(candidate);
                } else if let Some(stack) = fs.as_mut() {
                    // Otherwise, push the second epsilon-transition on the
                    // fail stack.
                    push_fail_stack(stack, *pidx, candidate, nregs, regs, eps_via_nodes);
                }
                // We know we are going to exit.
                break;
            }
        }
        Ok(dest_node)
    } else {
        let mut naccepted: i32 = 0;
        let ty = dfa.nodes[n].ty;
        if ty == OP_BACK_REF {
            let subexp_idx = dfa.nodes[n].idx + 1;
            let r = reg_get(regs, subexp_idx);
            naccepted = r.rm_eo.wrapping_sub(r.rm_so);
            if fs.is_some() {
                if r.rm_so == -1 || r.rm_eo == -1 {
                    return Ok(-1);
                } else if naccepted != 0 && !mctx.input.bytes_equal(r.rm_so, *pidx, naccepted) {
                    return Ok(-1);
                }
            }

            if naccepted == 0 {
                eps_via_nodes.insert(node);
                let dest_node = dfa.edests[n].elems[0];
                let cs = mctx.sl(*pidx).ok_or(ExecAbort::NullState { site: "mctx->state_log[*pidx]->nodes" })?;
                if mctx.st(cs).nodes.contains(dest_node) != 0 {
                    return Ok(dest_node);
                }
            }
        }

        if naccepted != 0 || check_node_accept(mctx, &dfa.nodes[n], *pidx) {
            let dest_node = dfa.nexts[n];
            *pidx = if naccepted == 0 { *pidx + 1 } else { *pidx + naccepted };
            if fs.is_some() {
                let bad = *pidx > mctx.match_last
                    || match mctx.sl(*pidx) {
                        None => true,
                        Some(s) => mctx.st(s).nodes.contains(dest_node) == 0,
                    };
                if bad {
                    return Ok(-1);
                }
            }
            eps_via_nodes.empty();
            return Ok(dest_node);
        }
        Ok(-1)
    }
}

/// push_fail_stack
fn push_fail_stack(
    fs: &mut Vec<FailStackEnt>,
    str_idx: i32,
    dest_node: i32,
    nregs: usize,
    regs: &[RegMatch],
    eps_via_nodes: &NodeSet,
) {
    fs.push(FailStackEnt {
        idx: str_idx,
        node: dest_node,
        regs: regs[..nregs].to_vec(),
        eps_via_nodes: NodeSet::init_copy(eps_via_nodes),
    });
}

/// pop_fail_stack
fn pop_fail_stack(
    fs: &mut Vec<FailStackEnt>,
    pidx: &mut i32,
    nregs: usize,
    regs: &mut [RegMatch],
    eps_via_nodes: &mut NodeSet,
) -> Result<i32, ExecAbort> {
    let ent = match fs.pop() {
        Some(e) => e,
        None => {
            return Err(ExecAbort::Assertion { expr: "num >= 0", line: 1344 });
        }
    };
    *pidx = ent.idx;
    regs[..nregs].copy_from_slice(&ent.regs[..nregs]);
    *eps_via_nodes = ent.eps_via_nodes;
    Ok(ent.node)
}

/// set_regs
fn set_regs(
    mctx: &mut MatchCtx,
    nmatch: usize,
    pmatch: &mut [RegMatch],
    fl_backtrack: bool,
) -> Result<RegErrcode, ExecAbort> {
    let dfa = mctx.dfa;
    let mut fs: Option<Vec<FailStackEnt>> = if fl_backtrack { Some(Vec::new()) } else { None };
    let mut cur_node = dfa.init_node;
    let mut eps_via_nodes = NodeSet::new_empty();
    let mut prev_idx_match: Vec<RegMatch> = pmatch[..nmatch].to_vec();

    let mut idx = pmatch[0].rm_so;
    while idx <= pmatch[0].rm_eo {
        update_regs(dfa, pmatch, &mut prev_idx_match, cur_node, idx, nmatch);

        if idx == pmatch[0].rm_eo && cur_node == mctx.last_node {
            if let Some(stack) = fs.as_mut() {
                let mut reg_idx = 0;
                while reg_idx < nmatch {
                    if pmatch[reg_idx].rm_so > -1 && pmatch[reg_idx].rm_eo == -1 {
                        break;
                    }
                    reg_idx += 1;
                }
                if reg_idx == nmatch {
                    return Ok(REG_NOERROR);
                }
                cur_node = pop_fail_stack(stack, &mut idx, nmatch, pmatch, &mut eps_via_nodes)?;
            } else {
                return Ok(REG_NOERROR);
            }
        }

        // Proceed to next node.
        let fs_len = fs.as_ref().map_or(0, |f| f.len());
        cur_node = proceed_next_node(mctx, nmatch, pmatch, &mut idx, cur_node, &mut eps_via_nodes, &mut fs)?;
        // Step budget: one unit per transition plus the work proportional to
        // EPS_VIA_NODES (inserted into, and copied onto the fail stack);
        // that set grows without bound on the patterns where the C engine
        // never finishes.
        let pushed = fs.as_ref().map_or(0, |f| if f.len() > fs_len { f[f.len() - 1].eps_via_nodes.nelem as u64 } else { 0 });
        mctx.tick_n(1 + pushed + eps_via_nodes.nelem as u64)?;

        if cur_node < 0 {
            if cur_node == -2 {
                return Ok(REG_ESPACE);
            }
            if let Some(stack) = fs.as_mut() {
                cur_node = pop_fail_stack(stack, &mut idx, nmatch, pmatch, &mut eps_via_nodes)?;
            } else {
                return Ok(REG_NOMATCH);
            }
        }
    }
    Ok(REG_NOERROR)
}

/// update_regs
fn update_regs(
    dfa: &Dfa,
    pmatch: &mut [RegMatch],
    prev_idx_match: &mut [RegMatch],
    cur_node: i32,
    cur_idx: i32,
    nmatch: usize,
) {
    let tok = &dfa.nodes[cur_node as usize];
    let ty = tok.ty;
    if ty == OP_OPEN_SUBEXP {
        let reg_num = (tok.idx + 1) as usize;
        // We are at the first node of this sub expression.
        if reg_num < nmatch {
            pmatch[reg_num].rm_so = cur_idx;
            pmatch[reg_num].rm_eo = -1;
        }
    } else if ty == OP_CLOSE_SUBEXP {
        let reg_num = (tok.idx + 1) as usize;
        if reg_num < nmatch {
            // We are at the last node of this sub expression.
            if pmatch[reg_num].rm_so < cur_idx {
                pmatch[reg_num].rm_eo = cur_idx;
                // This is a non-empty match or we are not inside an optional
                // subexpression.  Accept this right away.
                prev_idx_match[..nmatch].copy_from_slice(&pmatch[..nmatch]);
            } else if tok.opt_subexp && prev_idx_match[reg_num].rm_so != -1 {
                // We transited through an empty match for an optional
                // subexpression, like (a?)*, and this is not the subexp's
                // first match.  Copy back the old content of the registers so
                // that matches of an inner subexpression are undone as well,
                // like in ((a?))*.
                pmatch[..nmatch].copy_from_slice(&prev_idx_match[..nmatch]);
            } else {
                // We completed a subexpression, but it may be part of an
                // optional one, so do not update PREV_IDX_MATCH.
                pmatch[reg_num].rm_eo = cur_idx;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Sifting (prune_impossible_nodes helpers).

/// sift_states_backward
fn sift_states_backward(
    mctx: &mut MatchCtx,
    sctx: &mut SiftCtx,
    sifted: &mut Vec<StatePtr>,
    mut limited: Option<&mut Vec<StatePtr>>,
) -> RegErrcode {
    let mut null_cnt = 0;
    let mut str_idx = sctx.last_str_idx;

    // Build sifted state_log[str_idx].  It has the nodes which can epsilon
    // transit to the last_node and the last_node itself.
    let mut cur_dest = NodeSet::init_1(sctx.last_node);
    let err = update_cur_sifted_state(mctx, sctx, sifted, limited.as_deref_mut(), str_idx, &mut cur_dest);
    if err != REG_NOERROR {
        return err;
    }

    // Then check each states in the state_log.
    while str_idx > 0 {
        // Update counters.
        null_cnt = if arr_get(sifted, str_idx).is_none() { null_cnt + 1 } else { 0 };
        if null_cnt > mctx.max_mb_elem_len {
            for e in sifted.iter_mut().take(str_idx as usize) {
                *e = None;
            }
            return REG_NOERROR;
        }
        cur_dest.empty();
        str_idx -= 1;

        if mctx.sl(str_idx).is_some() {
            build_sifted_states(mctx, sctx, sifted, str_idx, &mut cur_dest);
        }

        // Add all the nodes which satisfy the following conditions:
        //  - It can epsilon transit to a node in CUR_DEST.
        //  - It is in CUR_SRC.
        // And update state_log.
        let err = update_cur_sifted_state(mctx, sctx, sifted, limited.as_deref_mut(), str_idx, &mut cur_dest);
        if err != REG_NOERROR {
            return err;
        }
    }
    REG_NOERROR
}

/// build_sifted_states
fn build_sifted_states(mctx: &mut MatchCtx, sctx: &SiftCtx, sifted: &[StatePtr], str_idx: i32, cur_dest: &mut NodeSet) {
    let dfa = mctx.dfa;
    let cur_src = mctx.st(mctx.sl(str_idx).unwrap()).non_eps_nodes.clone();
    for &prev_node in cur_src.as_slice() {
        let mut naccepted = 0;
        // We don't check backreferences here.  See update_cur_sifted_state().
        if naccepted == 0 && check_node_accept(mctx, &dfa.nodes[prev_node as usize], str_idx) {
            let s = arr_get(sifted, str_idx + 1);
            if let Some(s) = s {
                if mctx.st(s).nodes.contains(dfa.nexts[prev_node as usize]) != 0 {
                    naccepted = 1;
                }
            }
        }
        if naccepted == 0 {
            continue;
        }
        if sctx.limits.nelem != 0 {
            let to_idx = str_idx + naccepted;
            if check_dst_limits(mctx, &sctx.limits, dfa.nexts[prev_node as usize], to_idx, prev_node, str_idx) {
                continue;
            }
        }
        cur_dest.insert(prev_node);
    }
}

/// clean_state_log_if_needed
fn clean_state_log_if_needed(mctx: &mut MatchCtx, next_state_log_idx: i32) -> RegErrcode {
    let top = mctx.state_log_top;
    if next_state_log_idx >= mctx.input.bufs_len
        || (next_state_log_idx >= mctx.input.valid_len && mctx.input.valid_len < mctx.input.len)
    {
        extend_buffers(mctx);
    }
    if top < next_state_log_idx {
        if let Some(log) = &mut mctx.state_log {
            for i in (top + 1)..=next_state_log_idx {
                arr_set(log, i, None);
            }
        }
        mctx.state_log_top = next_state_log_idx;
    }
    REG_NOERROR
}

/// merge_state_array
fn merge_state_array(mctx: &mut MatchCtx, dst: &mut Vec<StatePtr>, src: &[StatePtr], num: i32) -> RegErrcode {
    for st_idx in 0..num {
        match arr_get(dst, st_idx) {
            None => {
                let v = arr_get(src, st_idx);
                arr_set(dst, st_idx, v);
            }
            Some(d) => {
                if let Some(s) = arr_get(src, st_idx) {
                    let merged = NodeSet::init_union(Some(&mctx.st(d).nodes), Some(&mctx.st(s).nodes));
                    let ns = mctx.acquire_state(&merged);
                    arr_set(dst, st_idx, ns);
                }
            }
        }
    }
    REG_NOERROR
}

/// update_cur_sifted_state
fn update_cur_sifted_state(
    mctx: &mut MatchCtx,
    sctx: &mut SiftCtx,
    sifted: &mut Vec<StatePtr>,
    limited: Option<&mut Vec<StatePtr>>,
    str_idx: i32,
    dest_nodes: &mut NodeSet,
) -> RegErrcode {
    let candidates: Option<NodeSet> = mctx.sl(str_idx).map(|s| mctx.st(s).nodes.clone());

    if dest_nodes.nelem == 0 {
        arr_set(sifted, str_idx, None);
    } else {
        if let Some(cand) = &candidates {
            // At first, add the nodes which can epsilon transit to a node in
            // DEST_NODE.
            add_epsilon_src_nodes(mctx, dest_nodes, cand);

            // Then, check the limitations in the current sift_context.
            if sctx.limits.nelem != 0 {
                check_subexp_limits(mctx, dest_nodes, cand, &sctx.limits, str_idx);
            }
        }
        let s = mctx.acquire_state(dest_nodes);
        arr_set(sifted, str_idx, s);
    }

    if let Some(cand) = &candidates {
        let has_backref = mctx.st(mctx.sl(str_idx).unwrap()).has_backref;
        if has_backref {
            let err = sift_states_bkref(mctx, sctx, sifted, limited, str_idx, cand);
            if err != REG_NOERROR {
                return err;
            }
        }
    }
    REG_NOERROR
}

/// add_epsilon_src_nodes
fn add_epsilon_src_nodes(mctx: &mut MatchCtx, dest_nodes: &mut NodeSet, candidates: &NodeSet) {
    let dfa = mctx.dfa;
    let state = match mctx.acquire_state(dest_nodes) {
        Some(s) => s,
        None => return,
    };
    if mctx.st(state).inveclosure.alloc == 0 {
        let mut inv = NodeSet::alloc_with(dest_nodes.nelem);
        for &e in dest_nodes.as_slice() {
            inv.merge(&dfa.inveclosures[e as usize]);
        }
        mctx.cache.st_mut(state).inveclosure = inv;
    }
    let inv = &mctx.cache.st(state).inveclosure;
    dest_nodes.add_intersect(candidates, inv);
}

/// sub_epsilon_src_nodes
fn sub_epsilon_src_nodes(dfa: &Dfa, node: i32, dest_nodes: &mut NodeSet, candidates: &NodeSet) {
    let inv_eclosure = &dfa.inveclosures[node as usize];
    let mut except_nodes = NodeSet::new_empty();
    for &cur_node in inv_eclosure.as_slice() {
        if cur_node == node {
            continue;
        }
        if is_epsilon_node(dfa.nodes[cur_node as usize].ty) {
            let ed = &dfa.edests[cur_node as usize];
            let edst1 = ed.elems[0];
            let edst2 = if ed.nelem > 1 { ed.elems[1] } else { -1 };
            if (inv_eclosure.contains(edst1) == 0 && dest_nodes.contains(edst1) != 0)
                || (edst2 > 0 && inv_eclosure.contains(edst2) == 0 && dest_nodes.contains(edst2) != 0)
            {
                except_nodes.add_intersect(candidates, &dfa.inveclosures[cur_node as usize]);
            }
        }
    }
    for &cur_node in inv_eclosure.as_slice() {
        if except_nodes.contains(cur_node) == 0 {
            let idx = dest_nodes.contains(cur_node) - 1;
            dest_nodes.remove_at(idx);
        }
    }
}

/// check_dst_limits
fn check_dst_limits(mctx: &mut MatchCtx, limits: &NodeSet, dst_node: i32, dst_idx: i32, src_node: i32, src_idx: i32) -> bool {
    let dfa = mctx.dfa;
    let dst_bkref_idx = search_cur_bkref_entry(mctx, dst_idx);
    let src_bkref_idx = search_cur_bkref_entry(mctx, src_idx);
    for lim_idx in 0..limits.nelem as usize {
        let lim = limits.elems[lim_idx];
        let ent_node = mctx.bkref_ents[lim as usize].node;
        let subexp_idx = dfa.nodes[ent_node as usize].idx;

        let dst_pos = check_dst_limits_calc_pos(mctx, lim, subexp_idx, dst_node, dst_idx, dst_bkref_idx);
        let src_pos = check_dst_limits_calc_pos(mctx, lim, subexp_idx, src_node, src_idx, src_bkref_idx);

        // In case of:
        // <src> <dst> ( <subexp> )
        // ( <subexp> ) <src> <dst>
        // ( <subexp1> <src> <subexp2> <dst> <subexp3> )
        if src_pos == dst_pos {
            continue; // This is unrelated limitation.
        } else {
            return true;
        }
    }
    false
}

/// check_dst_limits_calc_pos_1
fn check_dst_limits_calc_pos_1(mctx: &mut MatchCtx, boundaries: i32, subexp_idx: i32, from_node: i32, bkref_idx: i32) -> i32 {
    let dfa = mctx.dfa;
    let eclosures = &dfa.eclosures[from_node as usize];
    for &node in eclosures.as_slice() {
        match dfa.nodes[node as usize].ty {
            OP_BACK_REF => {
                if bkref_idx != -1 {
                    let mut ei = bkref_idx as usize;
                    loop {
                        // do { ... } while (ent++->more);  `continue` jumps to
                        // the condition.
                        'body: {
                            let ent_node = mctx.bkref_ents[ei].node;
                            if ent_node != node {
                                break 'body;
                            }
                            if subexp_idx < BITSET_WORD_BITS
                                && (mctx.bkref_ents[ei].eps_reachable_subexps_map as u32 & (1u32 << subexp_idx)) == 0
                            {
                                break 'body;
                            }
                            // Recurse trying to reach the OP_OPEN_SUBEXP and
                            // OP_CLOSE_SUBEXP cases below.  But, if the
                            // destination node is the same node as the source
                            // node, don't recurse because it would cause an
                            // infinite loop: a regex that exhibits this
                            // behavior is ()\1*\1*
                            let dst = dfa.edests[node as usize].elems[0];
                            if dst == from_node {
                                if boundaries & 1 != 0 {
                                    return -1;
                                } else {
                                    return 0;
                                }
                            }
                            let cpos = check_dst_limits_calc_pos_1(mctx, boundaries, subexp_idx, dst, bkref_idx);
                            if cpos == -1 {
                                return -1;
                            }
                            if cpos == 0 && boundaries & 2 != 0 {
                                return 0;
                            }
                            if subexp_idx < BITSET_WORD_BITS {
                                let m = mctx.bkref_ents[ei].eps_reachable_subexps_map as u32 & !(1u32 << subexp_idx);
                                mctx.bkref_ents[ei].eps_reachable_subexps_map = m as u16;
                            }
                        }
                        let more = mctx.bkref_ents[ei].more;
                        ei += 1;
                        if !more {
                            break;
                        }
                    }
                }
            }
            OP_OPEN_SUBEXP => {
                if boundaries & 1 != 0 && subexp_idx == dfa.nodes[node as usize].idx {
                    return -1;
                }
            }
            OP_CLOSE_SUBEXP => {
                if boundaries & 2 != 0 && subexp_idx == dfa.nodes[node as usize].idx {
                    return 0;
                }
            }
            _ => {}
        }
    }
    if boundaries & 2 != 0 {
        1
    } else {
        0
    }
}

/// check_dst_limits_calc_pos
fn check_dst_limits_calc_pos(mctx: &mut MatchCtx, limit: i32, subexp_idx: i32, from_node: i32, str_idx: i32, bkref_idx: i32) -> i32 {
    let lim = &mctx.bkref_ents[limit as usize];
    // If we are outside the range of the subexpression, return -1 or 1.
    if str_idx < lim.subexp_from {
        return -1;
    }
    if lim.subexp_to < str_idx {
        return 1;
    }
    // If we are within the subexpression, return 0.
    let mut boundaries = (str_idx == lim.subexp_from) as i32;
    boundaries |= ((str_idx == lim.subexp_to) as i32) << 1;
    if boundaries == 0 {
        return 0;
    }
    // Else, examine epsilon closure.
    check_dst_limits_calc_pos_1(mctx, boundaries, subexp_idx, from_node, bkref_idx)
}

/// check_subexp_limits
fn check_subexp_limits(mctx: &MatchCtx, dest_nodes: &mut NodeSet, candidates: &NodeSet, limits: &NodeSet, str_idx: i32) {
    let dfa = mctx.dfa;
    for lim_idx in 0..limits.nelem as usize {
        let ent = &mctx.bkref_ents[limits.elems[lim_idx] as usize];
        if str_idx <= ent.subexp_from || ent.str_idx < str_idx {
            continue; // This is unrelated limitation.
        }
        let subexp_idx = dfa.nodes[ent.node as usize].idx;
        if ent.subexp_to == str_idx {
            let mut ops_node = -1;
            let mut cls_node = -1;
            for &node in dest_nodes.as_slice() {
                let ty = dfa.nodes[node as usize].ty;
                if ty == OP_OPEN_SUBEXP && subexp_idx == dfa.nodes[node as usize].idx {
                    ops_node = node;
                } else if ty == OP_CLOSE_SUBEXP && subexp_idx == dfa.nodes[node as usize].idx {
                    cls_node = node;
                }
            }

            // Check the limitation of the open subexpression.
            // Note that (ent->subexp_to = str_idx != ent->subexp_from).
            if ops_node >= 0 {
                sub_epsilon_src_nodes(dfa, ops_node, dest_nodes, candidates);
            }

            // Check the limitation of the close subexpression.
            if cls_node >= 0 {
                let mut node_idx: i32 = 0;
                while node_idx < dest_nodes.nelem {
                    let node = dest_nodes.elems[node_idx as usize];
                    if dfa.inveclosures[node as usize].contains(cls_node) == 0
                        && dfa.eclosures[node as usize].contains(cls_node) == 0
                    {
                        // It is against this limitation.  Remove it form the
                        // current sifted state.
                        sub_epsilon_src_nodes(dfa, node, dest_nodes, candidates);
                        node_idx -= 1;
                    }
                    node_idx += 1;
                }
            }
        } else {
            let mut node_idx: i32 = 0;
            while node_idx < dest_nodes.nelem {
                let node = dest_nodes.elems[node_idx as usize];
                let ty = dfa.nodes[node as usize].ty;
                if ty == OP_CLOSE_SUBEXP || ty == OP_OPEN_SUBEXP {
                    if subexp_idx != dfa.nodes[node as usize].idx {
                        node_idx += 1;
                        continue;
                    }
                    // It is against this limitation.  Remove it form the
                    // current sifted state.
                    sub_epsilon_src_nodes(dfa, node, dest_nodes, candidates);
                }
                node_idx += 1;
            }
        }
    }
}

/// sift_states_bkref
fn sift_states_bkref(
    mctx: &mut MatchCtx,
    sctx: &mut SiftCtx,
    sifted: &mut Vec<StatePtr>,
    mut limited: Option<&mut Vec<StatePtr>>,
    str_idx: i32,
    candidates: &NodeSet,
) -> RegErrcode {
    let dfa = mctx.dfa;
    let first_idx = search_cur_bkref_entry(mctx, str_idx);
    if first_idx == -1 {
        return REG_NOERROR;
    }

    // local_sctx.sifted_states == NULL  <=>  local is None.
    let mut local: Option<SiftCtx> = None;

    for &node in candidates.as_slice() {
        let ty = dfa.nodes[node as usize].ty;
        // Avoid infinite loop for the REs like "()\1+".
        if node == sctx.last_node && str_idx == sctx.last_str_idx {
            continue;
        }
        if ty != OP_BACK_REF {
            continue;
        }

        let mut enabled_idx = first_idx;
        loop {
            'body: {
                let ent = mctx.bkref_ents[enabled_idx as usize].clone();
                if ent.node != node {
                    break 'body;
                }
                let subexp_len = ent.subexp_to - ent.subexp_from;
                let to_idx = str_idx + subexp_len;
                let dst_node = if subexp_len != 0 { dfa.nexts[node as usize] } else { dfa.edests[node as usize].elems[0] };

                if to_idx > sctx.last_str_idx {
                    break 'body;
                }
                let st = match arr_get(sifted, to_idx) {
                    None => break 'body,
                    Some(s) => s,
                };
                if mctx.st(st).nodes.contains(dst_node) == 0 {
                    break 'body;
                }
                if check_dst_limits(mctx, &sctx.limits, node, str_idx, dst_node, to_idx) {
                    break 'body;
                }

                if local.is_none() {
                    local = Some(SiftCtx {
                        last_node: sctx.last_node,
                        last_str_idx: sctx.last_str_idx,
                        limits: NodeSet::init_copy(&sctx.limits),
                    });
                }
                let lsctx = local.as_mut().unwrap();
                lsctx.last_node = node;
                lsctx.last_str_idx = str_idx;
                lsctx.limits.insert(enabled_idx);
                let cur_state = arr_get(sifted, str_idx);
                // The C code recurses here without bound on some patterns
                // (e.g. ERE "()\1+*"); on Windows the C stack then runs out
                // inside malloc, which returns NULL, so regexec fails with
                // REG_ESPACE (pdfTeX reports "no match").  Mirror that.
                mctx.sift_depth += 1;
                if mctx.sift_depth > SIFT_DEPTH_LIMIT {
                    mctx.sift_depth -= 1;
                    return REG_ESPACE;
                }
                let err = sift_states_backward(mctx, lsctx, sifted, limited.as_deref_mut());
                mctx.sift_depth -= 1;
                if err != REG_NOERROR {
                    return err;
                }
                if let Some(lim) = limited.as_deref_mut() {
                    let err = merge_state_array(mctx, lim, sifted, str_idx + 1);
                    if err != REG_NOERROR {
                        return err;
                    }
                }
                arr_set(sifted, str_idx, cur_state);
                local.as_mut().unwrap().limits.remove(enabled_idx);
            }
            let more = mctx.bkref_ents[enabled_idx as usize].more;
            enabled_idx += 1;
            if !more {
                break;
            }
        }
    }
    REG_NOERROR
}

// ---------------------------------------------------------------------------
// State transition.

/// transit_state
fn transit_state(mctx: &mut MatchCtx, state: StateId) -> StatePtr {
    let ch = mctx.input.fetch_byte();
    loop {
        if let Some(t) = &mctx.st(state).trtable {
            return t[ch as usize];
        }
        // (word_trtable is never built: mb_cur_max == 1.)
        build_trtable(mctx.dfa, mctx.cache, state);
    }
}

/// merge_state_with_log
fn merge_state_with_log(mctx: &mut MatchCtx, next_state: StatePtr) -> StatePtr {
    let dfa = mctx.dfa;
    let cur_idx = mctx.input.cur_idx;
    let mut next_state = next_state;

    if cur_idx > mctx.state_log_top {
        mctx.sl_set(cur_idx, next_state);
        mctx.state_log_top = cur_idx;
    } else if mctx.sl(cur_idx).is_none() {
        mctx.sl_set(cur_idx, next_state);
    } else {
        // If (state_log[cur_idx] != 0), it implies that cur_idx is the
        // destination of a multibyte char/collating element/back reference.
        // Then the next state is the union set of these destinations and the
        // results of the transition table.
        let pstate = mctx.sl(cur_idx).unwrap();
        let log_nodes = mctx.st(pstate).entrance().clone();
        let next_nodes = match next_state {
            Some(ns) => {
                let table_nodes = mctx.st(ns).entrance().clone();
                NodeSet::init_union(Some(&table_nodes), Some(&log_nodes))
            }
            None => log_nodes,
        };
        let context = mctx.context_at(mctx.input.cur_idx - 1);
        next_state = mctx.acquire_state_context(&next_nodes, context);
        mctx.sl_set(cur_idx, next_state);
    }

    if dfa.nbackref != 0 {
        if let Some(ns) = next_state {
            // Check OP_OPEN_SUBEXP in the current state in case that we use
            // them later.  We must check them here, since the back references
            // in the next state might use them.
            let nodes = mctx.st(ns).nodes.clone();
            check_subexp_matching_top(mctx, &nodes, cur_idx);

            // If the next state has back references.
            if mctx.st(ns).has_backref {
                transit_state_bkref(mctx, &nodes);
                next_state = mctx.sl(cur_idx);
            }
        }
    }
    next_state
}

/// find_recover_state
fn find_recover_state(mctx: &mut MatchCtx) -> StatePtr {
    loop {
        let max = mctx.state_log_top;
        let mut cur_str_idx = mctx.input.cur_idx;
        loop {
            cur_str_idx += 1;
            if cur_str_idx > max {
                return None;
            }
            mctx.input.skip_bytes(1);
            if mctx.sl(cur_str_idx).is_some() {
                break;
            }
        }
        let cur_state = merge_state_with_log(mctx, None);
        if cur_state.is_some() {
            return cur_state;
        }
    }
}

/// check_subexp_matching_top
fn check_subexp_matching_top(mctx: &mut MatchCtx, cur_nodes: &NodeSet, str_idx: i32) {
    let dfa = mctx.dfa;
    for &node in cur_nodes.as_slice() {
        let tok = &dfa.nodes[node as usize];
        if tok.ty == OP_OPEN_SUBEXP && tok.idx < BITSET_WORD_BITS && (dfa.used_bkref_map & (1u32 << tok.idx)) != 0 {
            match_ctx_add_subtop(mctx, node, str_idx);
        }
    }
}

/// transit_state_bkref
fn transit_state_bkref(mctx: &mut MatchCtx, nodes: &NodeSet) {
    let dfa = mctx.dfa;
    let cur_str_idx = mctx.input.cur_idx;

    for &node_idx in nodes.as_slice() {
        let node = &dfa.nodes[node_idx as usize];
        // Check whether `node' is a backreference or not.
        if node.ty != OP_BACK_REF {
            continue;
        }
        if node.constraint != 0 {
            let context = mctx.context_at(cur_str_idx);
            if not_satisfy_next_constraint(node.constraint, context) {
                continue;
            }
        }

        // `node' is a backreference.  Check the substring which the substring
        // matched.
        let mut bkc_idx = mctx.bkref_ents.len();
        get_subexp(mctx, node_idx, cur_str_idx);

        // And add the epsilon closures (which is `new_dest_nodes') of the
        // backreference to appropriate state_log.
        while bkc_idx < mctx.bkref_ents.len() {
            let ent = mctx.bkref_ents[bkc_idx].clone();
            bkc_idx += 1;
            if ent.node != node_idx || ent.str_idx != cur_str_idx {
                continue;
            }
            let subexp_len = ent.subexp_to - ent.subexp_from;
            let new_dest_nodes = if subexp_len == 0 {
                &dfa.eclosures[dfa.edests[node_idx as usize].elems[0] as usize]
            } else {
                &dfa.eclosures[dfa.nexts[node_idx as usize] as usize]
            };
            let dest_str_idx = cur_str_idx + ent.subexp_to - ent.subexp_from;
            let context = mctx.context_at(dest_str_idx - 1);
            let dest_state = mctx.sl(dest_str_idx);
            let prev_nelem = match mctx.sl(cur_str_idx) {
                None => 0,
                Some(s) => mctx.st(s).nodes.nelem,
            };
            // Add `new_dest_node' to state_log.
            match dest_state {
                None => {
                    let s = mctx.acquire_state_context(new_dest_nodes, context);
                    mctx.sl_set(dest_str_idx, s);
                }
                Some(ds) => {
                    let ent_nodes = mctx.st(ds).entrance().clone();
                    let dest_nodes = NodeSet::init_union(Some(&ent_nodes), Some(new_dest_nodes));
                    let s = mctx.acquire_state_context(&dest_nodes, context);
                    mctx.sl_set(dest_str_idx, s);
                }
            }
            // We need to check recursively if the backreference can epsilon
            // transit.
            if subexp_len == 0 {
                let now = match mctx.sl(cur_str_idx) {
                    None => 0,
                    Some(s) => mctx.st(s).nodes.nelem,
                };
                if now > prev_nelem {
                    check_subexp_matching_top(mctx, new_dest_nodes, cur_str_idx);
                    transit_state_bkref(mctx, new_dest_nodes);
                }
            }
        }
    }
}

/// get_subexp
fn get_subexp(mctx: &mut MatchCtx, bkref_node: i32, bkref_str_idx: i32) -> RegErrcode {
    let dfa = mctx.dfa;
    // Return if we have already checked BKREF_NODE at BKREF_STR_IDX.
    let cache_idx = search_cur_bkref_entry(mctx, bkref_str_idx);
    if cache_idx != -1 {
        let mut i = cache_idx as usize;
        loop {
            if mctx.bkref_ents[i].node == bkref_node {
                return REG_NOERROR; // We already checked it.
            }
            let more = mctx.bkref_ents[i].more;
            i += 1;
            if !more {
                break;
            }
        }
    }

    let subexp_num = dfa.nodes[bkref_node as usize].idx;

    // For each sub expression
    let mut sub_top_idx = 0usize;
    while sub_top_idx < mctx.sub_tops.len() {
        let st_node = mctx.sub_tops[sub_top_idx].node;
        if dfa.nodes[st_node as usize].idx != subexp_num {
            sub_top_idx += 1;
            continue; // It isn't related.
        }
        let st_str_idx = mctx.sub_tops[sub_top_idx].str_idx;
        let mut sl_str = st_str_idx;
        let mut bkref_str_off = bkref_str_idx;
        // At first, check the last node of sub expressions we already
        // evaluated.
        let mut sub_last_idx = 0usize;
        let mut broke = false;
        while sub_last_idx < mctx.sub_tops[sub_top_idx].lasts.len() {
            let sl_str_idx = mctx.sub_tops[sub_top_idx].lasts[sub_last_idx].str_idx;
            let sl_str_diff = sl_str_idx - sl_str;
            // The matched string by the sub expression match with the
            // substring at the back reference?
            if sl_str_diff > 0 {
                if bkref_str_off + sl_str_diff > mctx.input.valid_len {
                    // Not enough chars for a successful match.
                    if bkref_str_off + sl_str_diff > mctx.input.len {
                        broke = true;
                        break;
                    }
                    clean_state_log_if_needed(mctx, bkref_str_off + sl_str_diff);
                }
                if !mctx.input.bytes_equal(bkref_str_off, sl_str, sl_str_diff) {
                    // We don't need to search this sub expression any more.
                    broke = true;
                    break;
                }
            }
            bkref_str_off += sl_str_diff;
            sl_str += sl_str_diff;
            let err = get_subexp_sub(mctx, sub_top_idx, sub_last_idx, bkref_node, bkref_str_idx);
            sub_last_idx += 1;
            if err == REG_NOMATCH {
                continue;
            }
            if err != REG_NOERROR {
                return err;
            }
        }
        if broke {
            // `sub_last_idx < sub_top->nlasts`
            sub_top_idx += 1;
            continue;
        }
        if sub_last_idx > 0 {
            sl_str += 1;
        }
        // Then, search for the other last nodes of the sub expression.
        while sl_str <= bkref_str_idx {
            let sl_str_off = sl_str - st_str_idx;
            // The matched string by the sub expression match with the
            // substring at the back reference?
            if sl_str_off > 0 {
                if bkref_str_off >= mctx.input.valid_len {
                    // If we are at the end of the input, we cannot match.
                    if bkref_str_off >= mctx.input.len {
                        break;
                    }
                    extend_buffers(mctx);
                }
                let a = mctx.input.byte_at(bkref_str_off);
                bkref_str_off += 1;
                if a != mctx.input.byte_at(sl_str - 1) {
                    break; // We don't need to search this sub expression any more.
                }
            }
            let s = match mctx.sl(sl_str) {
                None => {
                    sl_str += 1;
                    continue;
                }
                Some(s) => s,
            };
            // Does this state have a ')' of the sub expression?
            let cls_node = find_subexp_node(dfa, &mctx.st(s).nodes, subexp_num, OP_CLOSE_SUBEXP);
            if cls_node == -1 {
                sl_str += 1;
                continue; // No.
            }
            if mctx.sub_tops[sub_top_idx].path.is_none() {
                mctx.sub_tops[sub_top_idx].path = Some(StateArray::default());
            }
            // Can the OP_OPEN_SUBEXP node arrive the OP_CLOSE_SUBEXP node in
            // the current context?
            let mut path = mctx.sub_tops[sub_top_idx].path.take().unwrap();
            let err = check_arrival(mctx, &mut path, st_node, st_str_idx, cls_node, sl_str, OP_CLOSE_SUBEXP);
            mctx.sub_tops[sub_top_idx].path = Some(path);
            if err == REG_NOMATCH {
                sl_str += 1;
                continue;
            }
            if err != REG_NOERROR {
                return err;
            }
            mctx.sub_tops[sub_top_idx].lasts.push(SubLast { node: cls_node, str_idx: sl_str, path: StateArray::default() });
            let last_idx = mctx.sub_tops[sub_top_idx].lasts.len() - 1;
            let _ = get_subexp_sub(mctx, sub_top_idx, last_idx, bkref_node, bkref_str_idx);
            sl_str += 1;
        }
        sub_top_idx += 1;
    }
    REG_NOERROR
}

/// get_subexp_sub
fn get_subexp_sub(mctx: &mut MatchCtx, sub_top_idx: usize, sub_last_idx: usize, bkref_node: i32, bkref_str: i32) -> RegErrcode {
    let top_str = mctx.sub_tops[sub_top_idx].str_idx;
    let (sl_node, sl_str) = {
        let sl = &mctx.sub_tops[sub_top_idx].lasts[sub_last_idx];
        (sl.node, sl.str_idx)
    };
    // Can the subexpression arrive the back reference?
    let mut path = std::mem::take(&mut mctx.sub_tops[sub_top_idx].lasts[sub_last_idx].path);
    let err = check_arrival(mctx, &mut path, sl_node, sl_str, bkref_node, bkref_str, OP_OPEN_SUBEXP);
    mctx.sub_tops[sub_top_idx].lasts[sub_last_idx].path = path;
    if err != REG_NOERROR {
        return err;
    }
    match_ctx_add_entry(mctx, bkref_node, bkref_str, top_str, sl_str);
    let to_idx = bkref_str + sl_str - top_str;
    clean_state_log_if_needed(mctx, to_idx)
}

/// find_subexp_node
fn find_subexp_node(dfa: &Dfa, nodes: &NodeSet, subexp_idx: i32, ty: TokenType) -> i32 {
    for &cls_node in nodes.as_slice() {
        let node = &dfa.nodes[cls_node as usize];
        if node.ty == ty && node.idx == subexp_idx {
            return cls_node;
        }
    }
    -1
}

/// check_arrival
#[allow(clippy::too_many_arguments)]
fn check_arrival(
    mctx: &mut MatchCtx,
    path: &mut StateArray,
    top_node: i32,
    top_str: i32,
    last_node: i32,
    last_str: i32,
    ty: TokenType,
) -> RegErrcode {
    let dfa = mctx.dfa;
    let subexp_num = dfa.nodes[top_node as usize].idx;
    // Extend the buffer if we need.
    if path.alloc < last_str + mctx.max_mb_elem_len + 1 {
        path.alloc += last_str + mctx.max_mb_elem_len + 1;
        path.array.resize(path.alloc as usize, None);
    }

    let mut str_idx = if path.next_idx != 0 { path.next_idx } else { top_str };

    // Temporary modify MCTX.
    let backup_state_log = mctx.state_log.replace(std::mem::take(&mut path.array));
    let backup_cur_idx = mctx.input.cur_idx;
    mctx.input.cur_idx = str_idx;

    // Setup initial node set.
    let mut context = mctx.context_at(str_idx - 1);
    let mut next_nodes;
    let mut cur_state: StatePtr = None;
    if str_idx == top_str {
        next_nodes = NodeSet::init_1(top_node);
        check_arrival_expand_ecl(dfa, &mut next_nodes, subexp_num, ty);
    } else {
        cur_state = mctx.sl(str_idx);
        match cur_state {
            Some(cs) if mctx.st(cs).has_backref => {
                next_nodes = NodeSet::init_copy(&mctx.st(cs).nodes);
            }
            _ => next_nodes = NodeSet::new_empty(),
        }
    }
    if str_idx == top_str || cur_state.map_or(false, |cs| mctx.st(cs).has_backref) {
        if next_nodes.nelem != 0 {
            expand_bkref_cache(mctx, &mut next_nodes, str_idx, subexp_num, ty);
        }
        cur_state = mctx.acquire_state_context(&next_nodes, context);
        mctx.sl_set(str_idx, cur_state);
    }

    let mut null_cnt = 0;
    while str_idx < last_str && null_cnt <= mctx.max_mb_elem_len {
        next_nodes.empty();
        if let Some(s) = mctx.sl(str_idx + 1) {
            let n = mctx.st(s).nodes.clone();
            next_nodes.merge(&n);
        }
        if let Some(cs) = cur_state {
            let non_eps = mctx.st(cs).non_eps_nodes.clone();
            check_arrival_add_next_nodes(mctx, str_idx, &non_eps, &mut next_nodes);
        }
        str_idx += 1;
        if next_nodes.nelem != 0 {
            check_arrival_expand_ecl(dfa, &mut next_nodes, subexp_num, ty);
            expand_bkref_cache(mctx, &mut next_nodes, str_idx, subexp_num, ty);
        }
        context = mctx.context_at(str_idx - 1);
        cur_state = mctx.acquire_state_context(&next_nodes, context);
        mctx.sl_set(str_idx, cur_state);
        null_cnt = if cur_state.is_none() { null_cnt + 1 } else { 0 };
    }
    let found = match mctx.sl(last_str) {
        None => false,
        Some(s) => mctx.st(s).nodes.contains(last_node) != 0,
    };
    path.next_idx = str_idx;

    // Fix MCTX.
    path.array = std::mem::replace(&mut mctx.state_log, backup_state_log).unwrap_or_default();
    mctx.input.cur_idx = backup_cur_idx;

    // Then check the current node set has the node LAST_NODE.
    if found {
        REG_NOERROR
    } else {
        REG_NOMATCH
    }
}

/// check_arrival_add_next_nodes
fn check_arrival_add_next_nodes(mctx: &MatchCtx, str_idx: i32, cur_nodes: &NodeSet, next_nodes: &mut NodeSet) {
    let dfa = mctx.dfa;
    for &cur_node in cur_nodes.as_slice() {
        if check_node_accept(mctx, &dfa.nodes[cur_node as usize], str_idx) {
            next_nodes.insert(dfa.nexts[cur_node as usize]);
        }
    }
}

/// check_arrival_expand_ecl
fn check_arrival_expand_ecl(dfa: &Dfa, cur_nodes: &mut NodeSet, ex_subexp: i32, ty: TokenType) {
    let mut new_nodes = NodeSet::alloc_with(cur_nodes.nelem);
    // Create a new node set NEW_NODES with the nodes which are epsilon
    // closures of the node in CUR_NODES.
    for &cur_node in cur_nodes.as_slice() {
        let eclosure = &dfa.eclosures[cur_node as usize];
        let outside_node = find_subexp_node(dfa, eclosure, ex_subexp, ty);
        if outside_node == -1 {
            // There are no problematic nodes, just merge them.
            new_nodes.merge(eclosure);
        } else {
            // There are problematic nodes, re-calculate incrementally.
            check_arrival_expand_ecl_sub(dfa, &mut new_nodes, cur_node, ex_subexp, ty);
        }
    }
    *cur_nodes = new_nodes;
}

/// check_arrival_expand_ecl_sub
fn check_arrival_expand_ecl_sub(dfa: &Dfa, dst_nodes: &mut NodeSet, target: i32, ex_subexp: i32, ty: TokenType) {
    let mut cur_node = target;
    while dst_nodes.contains(cur_node) == 0 {
        let cn = cur_node as usize;
        if dfa.nodes[cn].ty == ty && dfa.nodes[cn].idx == ex_subexp {
            if ty == OP_CLOSE_SUBEXP {
                dst_nodes.insert(cur_node);
            }
            break;
        }
        dst_nodes.insert(cur_node);
        if dfa.edests[cn].nelem == 0 {
            break;
        }
        if dfa.edests[cn].nelem == 2 {
            check_arrival_expand_ecl_sub(dfa, dst_nodes, dfa.edests[cn].elems[1], ex_subexp, ty);
        }
        cur_node = dfa.edests[cn].elems[0];
    }
}

/// expand_bkref_cache
fn expand_bkref_cache(mctx: &mut MatchCtx, cur_nodes: &mut NodeSet, cur_str: i32, subexp_num: i32, ty: TokenType) {
    let dfa = mctx.dfa;
    let cache_idx_start = search_cur_bkref_entry(mctx, cur_str);
    if cache_idx_start == -1 {
        return;
    }
    'restart: loop {
        let mut ei = cache_idx_start as usize;
        loop {
            'body: {
                let ent = mctx.bkref_ents[ei].clone();
                // Is this entry ENT is appropriate?
                if cur_nodes.contains(ent.node) == 0 {
                    break 'body; // No.
                }
                let to_idx = cur_str + ent.subexp_to - ent.subexp_from;
                // Calculate the destination of the back reference, and append
                // it to MCTX->STATE_LOG.
                if to_idx == cur_str {
                    // The backreference did epsilon transit, we must re-check
                    // all the node in the current state.
                    let next_node = dfa.edests[ent.node as usize].elems[0];
                    if cur_nodes.contains(next_node) != 0 {
                        break 'body;
                    }
                    let mut new_dests = NodeSet::init_1(next_node);
                    check_arrival_expand_ecl(dfa, &mut new_dests, subexp_num, ty);
                    cur_nodes.merge(&new_dests);
                    continue 'restart;
                } else {
                    let next_node = dfa.nexts[ent.node as usize];
                    let union_set;
                    if let Some(s) = mctx.sl(to_idx) {
                        if mctx.st(s).nodes.contains(next_node) != 0 {
                            break 'body;
                        }
                        let mut u = NodeSet::init_copy(&mctx.st(s).nodes);
                        u.insert(next_node);
                        union_set = u;
                    } else {
                        union_set = NodeSet::init_1(next_node);
                    }
                    let ns = mctx.acquire_state(&union_set);
                    mctx.sl_set(to_idx, ns);
                }
            }
            let more = mctx.bkref_ents[ei].more;
            ei += 1;
            if !more {
                break;
            }
        }
        break;
    }
}

/// build_trtable
fn build_trtable(dfa: &Dfa, cache: &mut StateCache, state: StateId) {
    // At first, group all nodes belonging to `state' into several
    // destinations.
    let (dests_node, dests_ch) = group_nodes_into_dfa_states(dfa, cache.st(state));
    let ndests = dests_node.len();
    if ndests == 0 {
        cache.st_mut(state).trtable = Some(Box::new([None; SBC_MAX]));
        return;
    }

    let mut follows = NodeSet::alloc_with(ndests as i32 + 1);
    let mut dest_states: Vec<StatePtr> = vec![None; ndests];
    let mut dest_states_word: Vec<StatePtr> = vec![None; ndests];
    let mut dest_states_nl: Vec<StatePtr> = vec![None; ndests];
    let mut acceptable: Bitset = [0; BITSET_WORDS];

    // Then build the states for all destinations.
    for i in 0..ndests {
        follows.empty();
        // Merge the follows of this destination states.
        for &n in dests_node[i].as_slice() {
            let next_node = dfa.nexts[n as usize];
            if next_node != -1 {
                follows.merge(&dfa.eclosures[next_node as usize]);
            }
        }
        dest_states[i] = cache.acquire_state_context(dfa, &follows, 0);
        let ds = dest_states[i].expect("dest_states[i] (NULL dereference in C)");
        // If the new state has context constraint, build appropriate states
        // for these contexts.
        if cache.st(ds).has_constraint {
            dest_states_word[i] = cache.acquire_state_context(dfa, &follows, CONTEXT_WORD);
            dest_states_nl[i] = cache.acquire_state_context(dfa, &follows, CONTEXT_NEWLINE);
        } else {
            dest_states_word[i] = dest_states[i];
            dest_states_nl[i] = dest_states[i];
        }
        bitset_merge(&mut acceptable, &dests_ch[i]);
    }

    // need_word_trtable is never set when mb_cur_max == 1: allocate a
    // 256-entry transition table.
    let mut trtable: Box<[StatePtr; SBC_MAX]> = Box::new([None; SBC_MAX]);
    for i in 0..BITSET_WORDS {
        let mut ch = i * BITSET_WORD_BITS as usize;
        let mut elem = acceptable[i];
        let mut mask: u32 = 1;
        while elem != 0 {
            if elem & 1 != 0 {
                // There must be exactly one destination which accepts
                // character ch.  See group_nodes_into_DFAstates.
                let mut j = 0;
                while dests_ch[j][i] & mask == 0 {
                    j += 1;
                }
                // j-th destination accepts the word character ch.
                if dfa.word_char[i] & mask != 0 {
                    trtable[ch] = dest_states_word[j];
                } else {
                    trtable[ch] = dest_states[j];
                }
            }
            mask <<= 1;
            elem >>= 1;
            ch += 1;
        }
    }

    // new line
    if bitset_contain(&acceptable, NEWLINE_CHAR as usize) {
        // The current state accepts newline character.
        for j in 0..ndests {
            if bitset_contain(&dests_ch[j], NEWLINE_CHAR as usize) {
                // k-th destination accepts newline character.
                trtable[NEWLINE_CHAR as usize] = dest_states_nl[j];
                // There must be only one destination which accepts newline.
                // See group_nodes_into_DFAstates.
                break;
            }
        }
    }

    cache.st_mut(state).trtable = Some(trtable);
}

/// group_nodes_into_DFAstates
fn group_nodes_into_dfa_states(dfa: &Dfa, state: &State) -> (Vec<NodeSet>, Vec<Bitset>) {
    let mut dests_node: Vec<NodeSet> = Vec::new();
    let mut dests_ch: Vec<Bitset> = Vec::new();
    let cur_nodes = &state.nodes;
    let mut accepts: Bitset = [0; BITSET_WORDS];

    // For all the nodes belonging to `state',
    for i in 0..cur_nodes.nelem as usize {
        let node = &dfa.nodes[cur_nodes.elems[i] as usize];
        let ty = node.ty;
        let constraint = node.constraint;

        // Enumerate all single byte character this node can accept.
        if ty == CHARACTER {
            bitset_set(&mut accepts, node.c as usize);
        } else if ty == SIMPLE_BRACKET {
            bitset_merge(&mut accepts, &dfa.sbcsets[node.sbcset as usize]);
        } else if ty == OP_PERIOD {
            accepts = [u32::MAX; BITSET_WORDS];
            if dfa.syntax & RE_DOT_NEWLINE == 0 {
                bitset_clear(&mut accepts, b'\n' as usize);
            }
            if dfa.syntax & RE_DOT_NOT_NULL != 0 {
                bitset_clear(&mut accepts, 0);
            }
        } else {
            continue;
        }

        // Check the `accepts' and sift the characters which are not match it
        // the context.
        if constraint != 0 {
            if constraint & NEXT_NEWLINE_CONSTRAINT != 0 {
                let accepts_newline = bitset_contain(&accepts, NEWLINE_CHAR as usize);
                accepts = [0; BITSET_WORDS];
                if accepts_newline {
                    bitset_set(&mut accepts, NEWLINE_CHAR as usize);
                } else {
                    continue;
                }
            }
            if constraint & NEXT_ENDBUF_CONSTRAINT != 0 {
                accepts = [0; BITSET_WORDS];
                continue;
            }
            if constraint & NEXT_WORD_CONSTRAINT != 0 {
                let mut any_set: u32 = 0;
                if ty == CHARACTER && !node.word_char {
                    accepts = [0; BITSET_WORDS];
                    continue;
                }
                for j in 0..BITSET_WORDS {
                    accepts[j] &= dfa.word_char[j];
                    any_set |= accepts[j];
                }
                if any_set == 0 {
                    continue;
                }
            }
            if constraint & NEXT_NOTWORD_CONSTRAINT != 0 {
                let mut any_set: u32 = 0;
                if ty == CHARACTER && node.word_char {
                    accepts = [0; BITSET_WORDS];
                    continue;
                }
                for j in 0..BITSET_WORDS {
                    accepts[j] &= !dfa.word_char[j];
                    any_set |= accepts[j];
                }
                if any_set == 0 {
                    continue;
                }
            }
        }

        // Then divide `accepts' into DFA states, or create a new state.
        // Above, we make sure that accepts is not empty.
        let mut j = 0;
        while j < dests_node.len() {
            // Optimization, skip if this state doesn't accept the character.
            if ty == CHARACTER && !bitset_contain(&dests_ch[j], node.c as usize) {
                j += 1;
                continue;
            }
            // Enumerate the intersection set of this state and `accepts'.
            let mut intersec: Bitset = [0; BITSET_WORDS];
            let mut has_intersec: u32 = 0;
            for k in 0..BITSET_WORDS {
                intersec[k] = accepts[k] & dests_ch[j][k];
                has_intersec |= intersec[k];
            }
            // And skip if the intersection set is empty.
            if has_intersec == 0 {
                j += 1;
                continue;
            }
            // Then check if this state is a subset of `accepts'.
            let mut remains: Bitset = [0; BITSET_WORDS];
            let mut not_subset: u32 = 0;
            let mut not_consumed: u32 = 0;
            for k in 0..BITSET_WORDS {
                remains[k] = !accepts[k] & dests_ch[j][k];
                not_subset |= remains[k];
                accepts[k] &= !dests_ch[j][k];
                not_consumed |= accepts[k];
            }
            // If this state isn't a subset of `accepts', create a new group
            // state, which has the `remains'.
            if not_subset != 0 {
                dests_ch.push(remains);
                dests_ch[j] = intersec;
                let copy = NodeSet::init_copy(&dests_node[j]);
                dests_node.push(copy);
            }
            // Put the position in the current group.
            dests_node[j].insert(cur_nodes.elems[i]);
            // If all characters are consumed, go to next node.
            if not_consumed == 0 {
                break;
            }
            j += 1;
        }
        // Some characters remain, create a new group.
        if j == dests_node.len() {
            dests_ch.push(accepts);
            dests_node.push(NodeSet::init_1(cur_nodes.elems[i]));
            accepts = [0; BITSET_WORDS];
        }
    }
    (dests_node, dests_ch)
}

/// check_node_accept
fn check_node_accept(mctx: &MatchCtx, node: &Token, idx: i32) -> bool {
    let ch = mctx.input.byte_at(idx);
    match node.ty {
        CHARACTER => {
            if node.c != ch {
                return false;
            }
        }
        SIMPLE_BRACKET => {
            if !bitset_contain(&mctx.dfa.sbcsets[node.sbcset as usize], ch as usize) {
                return false;
            }
        }
        OP_PERIOD => {
            if (ch == b'\n' && mctx.dfa.syntax & RE_DOT_NEWLINE == 0)
                || (ch == 0 && mctx.dfa.syntax & RE_DOT_NOT_NULL != 0)
            {
                return false;
            }
        }
        _ => return false,
    }
    if node.constraint != 0 {
        // The node has constraints.  Check whether the current context
        // satisfies the constraints.
        let context = mctx.context_at(idx);
        if not_satisfy_next_constraint(node.constraint, context) {
            return false;
        }
    }
    true
}

/// extend_buffers
fn extend_buffers(mctx: &mut MatchCtx) {
    // Double the lengthes of the buffers.
    mctx.input.bufs_len *= 2;
    if let Some(log) = &mut mctx.state_log {
        let n = (mctx.input.bufs_len + 1) as usize;
        if log.len() < n {
            log.resize(n, None);
        }
    }
    // Then reconstruct the buffers.
    if mctx.input.icase {
        mctx.input.build_buffers();
    }
}

// ---------------------------------------------------------------------------
// Matching context.

/// match_ctx_clean
fn match_ctx_clean(mctx: &mut MatchCtx) {
    mctx.sub_tops.clear();
    mctx.bkref_ents.clear();
}

/// match_ctx_add_entry
fn match_ctx_add_entry(mctx: &mut MatchCtx, node: i32, str_idx: i32, from: i32, to: i32) {
    if let Some(last) = mctx.bkref_ents.last_mut() {
        if last.str_idx == str_idx {
            last.more = true;
        }
    }
    mctx.bkref_ents.push(BkrefEnt {
        node,
        str_idx,
        subexp_from: from,
        subexp_to: to,
        more: false,
        // A backreference does not epsilon-transition unless it is empty, so
        // set to all zeros if FROM != TO.
        eps_reachable_subexps_map: if from == to { !0 } else { 0 },
    });
    if mctx.max_mb_elem_len < to - from {
        mctx.max_mb_elem_len = to - from;
    }
}

/// search_cur_bkref_entry
fn search_cur_bkref_entry(mctx: &MatchCtx, str_idx: i32) -> i32 {
    let last = mctx.bkref_ents.len() as i32;
    let mut left = 0;
    let mut right = last;
    while left < right {
        let mid = (left + right) / 2;
        if mctx.bkref_ents[mid as usize].str_idx < str_idx {
            left = mid + 1;
        } else {
            right = mid;
        }
    }
    if left < last && mctx.bkref_ents[left as usize].str_idx == str_idx {
        left
    } else {
        -1
    }
}

/// match_ctx_add_subtop
fn match_ctx_add_subtop(mctx: &mut MatchCtx, node: i32, str_idx: i32) {
    mctx.sub_tops.push(SubTop { str_idx, node, path: None, lasts: Vec::new() });
}
