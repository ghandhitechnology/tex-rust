//! Port of `regcomp.c` (non-I18N, non-_LIBC configuration).

use crate::ctype;
use crate::internal::*;

/// Upper bound on parse-tree / NFA nodes.  The C engine returns REG_ESPACE
/// when malloc fails, which happens at a machine-dependent size; we fail at a
/// fixed size instead of letting the process abort on allocation failure.
pub(crate) const NODE_LIMIT: usize = 1 << 24;

/// The compiled pattern (`regex_t` + `re_dfa_t`).
pub(crate) struct Compiled {
    pub dfa: Dfa,
    pub cache: StateCache,
    pub re_nsub: usize,
    pub syntax: RegSyntax,
    pub no_sub: bool,
    pub newline_anchor: bool,
    pub fastmap: [u8; SBC_MAX],
    pub fastmap_accurate: bool,
    pub can_be_null: bool,
}

/// `bin_tree_t`, stored in an arena (indices instead of pointers).
#[derive(Clone, Copy, Debug)]
struct BinTree {
    parent: Option<usize>,
    left: Option<usize>,
    right: Option<usize>,
    first: Option<usize>,
    next: Option<usize>,
    token: Token,
    node_idx: i32,
}

#[derive(Clone, Copy)]
enum Visit {
    OptimizeSubexps,
    LowerSubexps,
    CalcFirst,
    CalcNext,
    LinkNfaNodes,
    MarkOptSubexp(i32),
    FreeTree,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BracketElemType {
    SbChar,
    EquivClass,
    CollSym,
    CharClass,
}

struct BracketElem {
    ty: BracketElemType,
    ch: u8,
    name: Vec<u8>,
}

const BRACKET_NAME_BUF_SIZE: i32 = 32;

struct Compiler<'p> {
    dfa: Dfa,
    cache: StateCache,
    tree: Vec<BinTree>,
    str_tree: Option<usize>,
    re_nsub: usize,
    no_sub: bool,
    regexp: PatString<'p>,
    /// Recursion depth of calc_eclosure_iter / duplicate_node_closure.
    depth: u32,
    /// See REG_NEED_STACK.
    depth_limit: u32,
}

/// Internal pseudo error: the pattern needs more recursion than the
/// caller's stack is assumed to allow; lib.rs retries on a big stack.
pub(crate) const REG_NEED_STACK: RegErrcode = -100;

/// regcomp's core: `re_compile_internal` followed (on success) by
/// `re_compile_fastmap`.  Returns the raw reg_errcode_t on failure (before
/// regcomp's REG_ERPAREN -> REG_EPAREN mapping).
pub(crate) fn re_compile_internal(
    pattern: &[u8],
    syntax: RegSyntax,
    no_sub: bool,
    newline_anchor: bool,
    depth_limit: u32,
) -> Result<Compiled, RegErrcode> {
    let length = pattern.len();
    // init_dfa
    let mut dfa = Dfa::new();
    dfa.mb_cur_max = 1;
    let cache = StateCache::new(length);

    let regexp = PatString::construct(pattern, syntax & RE_ICASE != 0);
    let mut c = Compiler { dfa, cache, tree: Vec::new(), str_tree: None, re_nsub: 0, no_sub, regexp, depth: 0, depth_limit };

    let mut err = REG_NOERROR;
    c.str_tree = c.parse(syntax, &mut err);
    if c.str_tree.is_none() {
        return Err(err);
    }
    let err = c.analyze();
    if err != REG_NOERROR {
        return Err(err);
    }
    let err = c.create_initial_state();
    if err != REG_NOERROR {
        return Err(err);
    }

    let mut comp = Compiled {
        dfa: c.dfa,
        cache: c.cache,
        re_nsub: c.re_nsub,
        syntax,
        no_sub,
        newline_anchor,
        fastmap: [0; SBC_MAX],
        fastmap_accurate: false,
        can_be_null: false,
    };
    re_compile_fastmap(&mut comp);
    Ok(comp)
}

/// re_compile_fastmap
fn re_compile_fastmap(comp: &mut Compiled) {
    comp.fastmap = [0; SBC_MAX];
    let init = comp.dfa.init_state;
    re_compile_fastmap_iter(comp, init);
    if comp.dfa.init_state != comp.dfa.init_state_word {
        let s = comp.dfa.init_state_word;
        re_compile_fastmap_iter(comp, s);
    }
    if comp.dfa.init_state != comp.dfa.init_state_nl {
        let s = comp.dfa.init_state_nl;
        re_compile_fastmap_iter(comp, s);
    }
    if comp.dfa.init_state != comp.dfa.init_state_begbuf {
        let s = comp.dfa.init_state_begbuf;
        re_compile_fastmap_iter(comp, s);
    }
    comp.fastmap_accurate = true;
}

#[inline]
fn re_set_fastmap(fastmap: &mut [u8; SBC_MAX], icase: bool, ch: i32) {
    fastmap[ch as usize] = 1;
    if icase {
        fastmap[ctype::tolower(ch) as usize] = 1;
    }
}

/// re_compile_fastmap_iter
fn re_compile_fastmap_iter(comp: &mut Compiled, init_state: StatePtr) {
    let sid = init_state.expect("initial state");
    let icase = comp.dfa.mb_cur_max == 1 && (comp.syntax & RE_ICASE) != 0;
    let nodes = comp.cache.st(sid).nodes.clone();
    for &node in nodes.as_slice() {
        let tok = comp.dfa.nodes[node as usize];
        let ty = tok.ty;
        if ty == CHARACTER {
            re_set_fastmap(&mut comp.fastmap, icase, tok.c as i32);
        } else if ty == SIMPLE_BRACKET {
            let set = comp.dfa.sbcsets[tok.sbcset as usize];
            let mut ch: i32 = 0;
            for i in 0..BITSET_WORDS {
                let w = set[i];
                for j in 0..BITSET_WORD_BITS {
                    if w & (1u32 << j) != 0 {
                        re_set_fastmap(&mut comp.fastmap, icase, ch);
                    }
                    ch += 1;
                }
            }
        } else if ty == OP_PERIOD || ty == END_OF_RE {
            comp.fastmap = [1; SBC_MAX];
            if ty == END_OF_RE {
                comp.can_be_null = true;
            }
            return;
        }
    }
}

impl<'p> Compiler<'p> {
    // -----------------------------------------------------------------------
    // Tree helpers.

    /// create_token_tree
    fn create_token_tree(&mut self, left: Option<usize>, right: Option<usize>, token: &Token) -> Option<usize> {
        if self.tree.len() >= NODE_LIMIT {
            return None;
        }
        let mut t = *token;
        t.duplicated = false;
        t.opt_subexp = false;
        let id = self.tree.len();
        self.tree.push(BinTree { parent: None, left, right, first: None, next: None, token: t, node_idx: -1 });
        if let Some(l) = left {
            self.tree[l].parent = Some(id);
        }
        if let Some(r) = right {
            self.tree[r].parent = Some(id);
        }
        Some(id)
    }

    /// create_tree
    fn create_tree(&mut self, left: Option<usize>, right: Option<usize>, ty: TokenType) -> Option<usize> {
        let t = Token { ty, ..Token::default() };
        self.create_token_tree(left, right, &t)
    }

    /// duplicate_tree
    fn duplicate_tree(&mut self, root: usize) -> Option<usize> {
        #[derive(Clone, Copy)]
        enum Slot {
            Root,
            Left(usize),
            Right(usize),
        }
        let mut dup_root: Option<usize> = None;
        let mut p_new = Slot::Root;
        let mut dup_node: Option<usize> = self.tree[root].parent;
        let mut node = root;
        loop {
            let tok = self.tree[node].token;
            let newn = self.create_token_tree(None, None, &tok)?;
            match p_new {
                Slot::Root => dup_root = Some(newn),
                Slot::Left(p) => self.tree[p].left = Some(newn),
                Slot::Right(p) => self.tree[p].right = Some(newn),
            }
            self.tree[newn].parent = dup_node;
            self.tree[newn].token.duplicated = true;
            dup_node = Some(newn);

            if let Some(l) = self.tree[node].left {
                node = l;
                p_new = Slot::Left(dup_node.unwrap());
            } else {
                let mut prev: Option<usize> = None;
                while self.tree[node].right == prev || self.tree[node].right.is_none() {
                    prev = Some(node);
                    let parent = self.tree[node].parent;
                    dup_node = dup_node.and_then(|d| self.tree[d].parent);
                    match parent {
                        None => return dup_root,
                        Some(p) => node = p,
                    }
                }
                node = self.tree[node].right.unwrap();
                p_new = Slot::Right(dup_node.unwrap());
            }
        }
    }

    fn visit(&mut self, v: Visit, node: usize) -> RegErrcode {
        match v {
            Visit::OptimizeSubexps => self.optimize_subexps(node),
            Visit::LowerSubexps => self.lower_subexps(node),
            Visit::CalcFirst => self.calc_first(node),
            Visit::CalcNext => self.calc_next(node),
            Visit::LinkNfaNodes => self.link_nfa_nodes(node),
            Visit::MarkOptSubexp(idx) => {
                if self.tree[node].token.ty == SUBEXP && self.tree[node].token.idx == idx {
                    self.tree[node].token.opt_subexp = true;
                }
                REG_NOERROR
            }
            Visit::FreeTree => REG_NOERROR,
        }
    }

    /// postorder
    fn postorder(&mut self, root: usize, v: Visit) -> RegErrcode {
        let mut node = root;
        loop {
            while self.tree[node].left.is_some() || self.tree[node].right.is_some() {
                node = match self.tree[node].left {
                    Some(l) => l,
                    None => self.tree[node].right.unwrap(),
                };
            }
            loop {
                let err = self.visit(v, node);
                if err != REG_NOERROR {
                    return err;
                }
                let parent = match self.tree[node].parent {
                    None => return REG_NOERROR,
                    Some(p) => p,
                };
                let prev = node;
                node = parent;
                if !(self.tree[node].right == Some(prev) || self.tree[node].right.is_none()) {
                    break;
                }
            }
            node = self.tree[node].right.unwrap();
        }
    }

    /// preorder
    fn preorder(&mut self, root: usize, v: Visit) -> RegErrcode {
        let mut node = root;
        loop {
            let err = self.visit(v, node);
            if err != REG_NOERROR {
                return err;
            }
            if let Some(l) = self.tree[node].left {
                node = l;
            } else {
                let mut prev: Option<usize> = None;
                while self.tree[node].right == prev || self.tree[node].right.is_none() {
                    prev = Some(node);
                    match self.tree[node].parent {
                        None => return REG_NOERROR,
                        Some(p) => node = p,
                    }
                }
                node = self.tree[node].right.unwrap();
            }
        }
    }

    // -----------------------------------------------------------------------
    // analyze and its passes.

    /// analyze
    fn analyze(&mut self) -> RegErrcode {
        let root = self.str_tree.unwrap();
        if self.re_nsub > 0 || true {
            // dfa->subexp_map = re_malloc (int, preg->re_nsub);  (malloc(0) may
            // return a non-NULL pointer; either way the map is dropped below
            // when it is the identity, which it always is for re_nsub == 0.)
            let mut map: Vec<i32> = (0..self.re_nsub as i32).collect();
            self.dfa.subexp_map = Some(std::mem::take(&mut map));
            self.preorder(root, Visit::OptimizeSubexps);
            let map = self.dfa.subexp_map.as_ref().unwrap();
            let mut i = 0usize;
            while i < self.re_nsub {
                if map[i] != i as i32 {
                    break;
                }
                i += 1;
            }
            if i == self.re_nsub {
                self.dfa.subexp_map = None;
            }
        }

        let ret = self.postorder(root, Visit::LowerSubexps);
        if ret != REG_NOERROR {
            return ret;
        }
        let ret = self.postorder(root, Visit::CalcFirst);
        if ret != REG_NOERROR {
            return ret;
        }
        self.preorder(root, Visit::CalcNext);
        let ret = self.preorder(root, Visit::LinkNfaNodes);
        if ret != REG_NOERROR {
            return ret;
        }
        let ret = self.calc_eclosure();
        if ret != REG_NOERROR {
            return ret;
        }

        if (!self.no_sub && self.re_nsub > 0 && self.dfa.has_plural_match) || self.dfa.nbackref != 0 {
            self.calc_inveclosure();
        }
        REG_NOERROR
    }

    /// optimize_subexps (preorder)
    fn optimize_subexps(&mut self, node: usize) -> RegErrcode {
        let tok = self.tree[node].token;
        if tok.ty == OP_BACK_REF && self.dfa.subexp_map.is_some() {
            let idx = tok.idx;
            let mapped = self.dfa.subexp_map.as_ref().unwrap()[idx as usize];
            self.tree[node].token.idx = mapped;
            // `1 << idx` with a 32-bit int: x86 masks the count to 5 bits.
            self.dfa.used_bkref_map |= 1u32.wrapping_shl(mapped as u32);
        } else if tok.ty == SUBEXP {
            if let Some(left) = self.tree[node].left {
                if self.tree[left].token.ty == SUBEXP {
                    let other_idx = self.tree[left].token.idx;
                    let ll = self.tree[left].left;
                    self.tree[node].left = ll;
                    if let Some(ll) = ll {
                        self.tree[ll].parent = Some(node);
                    }
                    let map = self.dfa.subexp_map.as_mut().unwrap();
                    map[other_idx as usize] = map[tok.idx as usize];
                    if other_idx < BITSET_WORD_BITS {
                        self.dfa.used_bkref_map &= !(1u32 << other_idx);
                    }
                }
            }
        }
        REG_NOERROR
    }

    /// lower_subexps (postorder)
    fn lower_subexps(&mut self, node: usize) -> RegErrcode {
        let mut err = REG_NOERROR;
        if let Some(l) = self.tree[node].left {
            if self.tree[l].token.ty == SUBEXP {
                let nl = self.lower_subexp(&mut err, l);
                self.tree[node].left = nl;
                if let Some(nl) = nl {
                    self.tree[nl].parent = Some(node);
                }
            }
        }
        if let Some(r) = self.tree[node].right {
            if self.tree[r].token.ty == SUBEXP {
                let nr = self.lower_subexp(&mut err, r);
                self.tree[node].right = nr;
                if let Some(nr) = nr {
                    self.tree[nr].parent = Some(node);
                }
            }
        }
        err
    }

    /// lower_subexp
    fn lower_subexp(&mut self, err: &mut RegErrcode, node: usize) -> Option<usize> {
        let body = self.tree[node].left;
        let idx = self.tree[node].token.idx;
        if self.no_sub
            && body.is_some()
            && (idx >= BITSET_WORD_BITS || (self.dfa.used_bkref_map & (1u32 << idx)) == 0)
        {
            return body;
        }
        let op = self.create_tree(None, None, OP_OPEN_SUBEXP);
        let cls = self.create_tree(None, None, OP_CLOSE_SUBEXP);
        let tree1 = match (body, cls) {
            (Some(b), Some(c)) => self.create_tree(Some(b), Some(c), CONCAT),
            (None, c) => c,
            (Some(_), None) => None,
        };
        let tree = match (op, tree1) {
            (Some(o), Some(t1)) => self.create_tree(Some(o), Some(t1), CONCAT),
            _ => None,
        };
        let (op, cls, tree) = match (op, cls, tree, tree1) {
            (Some(o), Some(c), Some(t), Some(_)) => (o, c, t),
            _ => {
                *err = REG_ESPACE;
                return None;
            }
        };
        let opt = self.tree[node].token.opt_subexp;
        self.tree[op].token.idx = idx;
        self.tree[cls].token.idx = idx;
        self.tree[op].token.opt_subexp = opt;
        self.tree[cls].token.opt_subexp = opt;
        Some(tree)
    }

    /// calc_first (postorder)
    fn calc_first(&mut self, node: usize) -> RegErrcode {
        if self.tree[node].token.ty == CONCAT {
            let left = self.tree[node].left.unwrap();
            self.tree[node].first = self.tree[left].first;
            self.tree[node].node_idx = self.tree[left].node_idx;
        } else {
            self.tree[node].first = Some(node);
            if self.dfa.nodes.len() >= NODE_LIMIT {
                return REG_ESPACE;
            }
            let tok = self.tree[node].token;
            self.tree[node].node_idx = self.dfa.add_node(tok);
        }
        REG_NOERROR
    }

    /// calc_next (preorder)
    fn calc_next(&mut self, node: usize) -> RegErrcode {
        let ty = self.tree[node].token.ty;
        match ty {
            OP_DUP_ASTERISK => {
                let l = self.tree[node].left.unwrap();
                self.tree[l].next = Some(node);
            }
            CONCAT => {
                let l = self.tree[node].left.unwrap();
                let r = self.tree[node].right.unwrap();
                self.tree[l].next = self.tree[r].first;
                self.tree[r].next = self.tree[node].next;
            }
            _ => {
                let next = self.tree[node].next;
                if let Some(l) = self.tree[node].left {
                    self.tree[l].next = next;
                }
                if let Some(r) = self.tree[node].right {
                    self.tree[r].next = next;
                }
            }
        }
        REG_NOERROR
    }

    fn first_idx(&self, n: usize) -> i32 {
        let f = self.tree[n].first.unwrap();
        self.tree[f].node_idx
    }

    /// link_nfa_nodes (preorder)
    fn link_nfa_nodes(&mut self, node: usize) -> RegErrcode {
        let idx = self.tree[node].node_idx;
        let ty = self.tree[node].token.ty;
        match ty {
            CONCAT => {}
            END_OF_RE => {
                debug_assert!(self.tree[node].next.is_none());
            }
            OP_DUP_ASTERISK | OP_ALT => {
                self.dfa.has_plural_match = true;
                let left = match self.tree[node].left {
                    Some(l) => self.first_idx(l),
                    None => self.tree[self.tree[node].next.unwrap()].node_idx,
                };
                let right = match self.tree[node].right {
                    Some(r) => self.first_idx(r),
                    None => self.tree[self.tree[node].next.unwrap()].node_idx,
                };
                self.dfa.edests[idx as usize] = NodeSet::init_2(left, right);
            }
            ANCHOR | OP_OPEN_SUBEXP | OP_CLOSE_SUBEXP => {
                let n = self.tree[self.tree[node].next.unwrap()].node_idx;
                self.dfa.edests[idx as usize] = NodeSet::init_1(n);
            }
            OP_BACK_REF => {
                let n = self.tree[self.tree[node].next.unwrap()].node_idx;
                self.dfa.nexts[idx as usize] = n;
                self.dfa.edests[idx as usize] = NodeSet::init_1(n);
            }
            _ => {
                debug_assert!(!is_epsilon_node(ty));
                let n = self.tree[self.tree[node].next.unwrap()].node_idx;
                self.dfa.nexts[idx as usize] = n;
            }
        }
        REG_NOERROR
    }

    // -----------------------------------------------------------------------
    // Epsilon closures.

    /// duplicate_node_closure
    fn duplicate_node_closure(
        &mut self,
        top_org_node: i32,
        top_clone_node: i32,
        root_node: i32,
        init_constraint: u32,
    ) -> RegErrcode {
        let mut constraint = init_constraint;
        let mut org_node = top_org_node;
        let mut clone_node = top_clone_node;
        loop {
            let org_dest;
            let clone_dest;
            let on = org_node as usize;
            let cn = clone_node as usize;
            if self.dfa.nodes[on].ty == OP_BACK_REF {
                org_dest = self.dfa.nexts[on];
                self.dfa.edests[cn].empty();
                clone_dest = match self.duplicate_node(org_dest, constraint) {
                    Some(d) => d,
                    None => return REG_ESPACE,
                };
                self.dfa.nexts[cn] = self.dfa.nexts[on];
                self.dfa.edests[cn].insert(clone_dest);
            } else if self.dfa.edests[on].nelem == 0 {
                self.dfa.nexts[cn] = self.dfa.nexts[on];
                break;
            } else if self.dfa.edests[on].nelem == 1 {
                org_dest = self.dfa.edests[on].elems[0];
                self.dfa.edests[cn].empty();
                if self.dfa.nodes[on].ty == ANCHOR {
                    if org_node == root_node && clone_node != org_node {
                        self.dfa.edests[cn].insert(org_dest);
                        break;
                    }
                    constraint |= self.dfa.nodes[on].ctx_type;
                }
                clone_dest = match self.duplicate_node(org_dest, constraint) {
                    Some(d) => d,
                    None => return REG_ESPACE,
                };
                self.dfa.edests[cn].insert(clone_dest);
            } else {
                // edests[org_node].nelem == 2
                let first_dest = self.dfa.edests[on].elems[0];
                self.dfa.edests[cn].empty();
                let found = self.search_duplicated_node(first_dest, constraint);
                if found == -1 {
                    let cd = match self.duplicate_node(first_dest, constraint) {
                        Some(d) => d,
                        None => return REG_ESPACE,
                    };
                    self.dfa.edests[cn].insert(cd);
                    self.depth += 1;
                    if self.depth > self.depth_limit {
                        return REG_NEED_STACK;
                    }
                    let err = self.duplicate_node_closure(first_dest, cd, root_node, constraint);
                    self.depth -= 1;
                    if err != REG_NOERROR {
                        return err;
                    }
                } else {
                    self.dfa.edests[cn].insert(found);
                }

                org_dest = self.dfa.edests[on].elems[1];
                clone_dest = match self.duplicate_node(org_dest, constraint) {
                    Some(d) => d,
                    None => return REG_ESPACE,
                };
                self.dfa.edests[cn].insert(clone_dest);
            }
            org_node = org_dest;
            clone_node = clone_dest;
        }
        REG_NOERROR
    }

    /// search_duplicated_node
    fn search_duplicated_node(&self, org_node: i32, constraint: u32) -> i32 {
        let mut idx = self.dfa.nodes.len() as i32 - 1;
        while self.dfa.nodes[idx as usize].duplicated && idx > 0 {
            if org_node == self.dfa.org_indices[idx as usize] && constraint == self.dfa.nodes[idx as usize].constraint {
                return idx;
            }
            idx -= 1;
        }
        -1
    }

    /// duplicate_node
    fn duplicate_node(&mut self, org_idx: i32, constraint: u32) -> Option<i32> {
        if self.dfa.nodes.len() >= NODE_LIMIT {
            return None;
        }
        let tok = self.dfa.nodes[org_idx as usize];
        let dup_idx = self.dfa.add_node(tok);
        let d = dup_idx as usize;
        self.dfa.nodes[d].constraint = constraint;
        if self.dfa.nodes[org_idx as usize].ty == ANCHOR {
            self.dfa.nodes[d].constraint |= self.dfa.nodes[org_idx as usize].ctx_type;
        }
        // The C bit-field is 10 bits wide.
        self.dfa.nodes[d].constraint &= 0x3ff;
        self.dfa.nodes[d].duplicated = true;
        self.dfa.org_indices[d] = org_idx;
        Some(dup_idx)
    }

    /// calc_inveclosure
    fn calc_inveclosure(&mut self) {
        let n = self.dfa.nodes.len();
        self.dfa.inveclosures = vec![NodeSet::new_empty(); n];
        for src in 0..n {
            let ecl = self.dfa.eclosures[src].clone();
            for &e in ecl.as_slice() {
                self.dfa.inveclosures[e as usize].insert_last(src as i32);
            }
        }
    }

    /// calc_eclosure
    fn calc_eclosure(&mut self) -> RegErrcode {
        let mut incomplete = false;
        let mut node_idx: usize = 0;
        loop {
            if node_idx == self.dfa.nodes.len() {
                if !incomplete {
                    break;
                }
                incomplete = false;
                node_idx = 0;
            }
            if self.dfa.eclosures[node_idx].nelem != 0 {
                node_idx += 1;
                continue;
            }
            if let Err(err) = self.calc_eclosure_iter(node_idx as i32, true) {
                return err;
            }
            if self.dfa.eclosures[node_idx].nelem == 0 {
                incomplete = true;
            }
            node_idx += 1;
        }
        REG_NOERROR
    }

    /// calc_eclosure_iter.  In C the epsilon closure is returned through
    /// NEW_SET and, unless it is incomplete, also stored in
    /// `dfa->eclosures[node]` (both share the element array).  Here the set
    /// is returned only when it was *not* stored; otherwise the caller reads
    /// `dfa.eclosures[node]`.  This avoids copying closures, nothing else.
    fn calc_eclosure_iter(&mut self, node: i32, root: bool) -> Result<Option<NodeSet>, RegErrcode> {
        let n = node as usize;
        let mut incomplete = false;
        let mut eclosure = NodeSet::alloc_with(self.dfa.edests[n].nelem + 1);

        // This indicates that we are calculating this node now.
        self.dfa.eclosures[n].nelem = -1;

        let constraint = if self.dfa.nodes[n].ty == ANCHOR { self.dfa.nodes[n].ctx_type } else { 0 };
        if constraint != 0
            && self.dfa.edests[n].nelem != 0
            && !self.dfa.nodes[self.dfa.edests[n].elems[0] as usize].duplicated
        {
            let err = self.duplicate_node_closure(node, node, node, constraint);
            if err != REG_NOERROR {
                return Err(err);
            }
        }

        if is_epsilon_node(self.dfa.nodes[n].ty) {
            let mut i = 0;
            while i < self.dfa.edests[n].nelem {
                let edest = self.dfa.edests[n].elems[i as usize];
                let e = edest as usize;
                // If calculating the epsilon closure of `edest' is in
                // progress, return intermediate result.
                if self.dfa.eclosures[e].nelem == -1 {
                    incomplete = true;
                    i += 1;
                    continue;
                }
                // If we haven't calculated the epsilon closure of `edest'
                // yet, calculate now.  Otherwise use calculated epsilon
                // closure.
                if self.dfa.eclosures[e].nelem == 0 {
                    self.depth += 1;
                    if self.depth > self.depth_limit {
                        return Err(REG_NEED_STACK);
                    }
                    let r = self.calc_eclosure_iter(edest, false)?;
                    self.depth -= 1;
                    match r {
                        Some(elem) => eclosure.merge(&elem),
                        None => eclosure.merge(&self.dfa.eclosures[e]),
                    }
                } else {
                    eclosure.merge(&self.dfa.eclosures[e]);
                }
                // If the epsilon closure of `edest' is incomplete, the
                // epsilon closure of this node is also incomplete.
                if self.dfa.eclosures[e].nelem == 0 {
                    incomplete = true;
                }
                i += 1;
            }
        }

        // Epsilon closures include itself.
        eclosure.insert(node);
        if incomplete && !root {
            self.dfa.eclosures[n].nelem = 0;
            Ok(Some(eclosure))
        } else {
            self.dfa.eclosures[n] = eclosure;
            Ok(None)
        }
    }

    // -----------------------------------------------------------------------
    // Tokenizer.

    /// fetch_token
    fn fetch_token(&mut self, result: &mut Token, syntax: RegSyntax) {
        let n = self.peek_token(result, syntax);
        self.regexp.skip_bytes(n);
    }

    /// peek_token
    fn peek_token(&mut self, token: &mut Token, syntax: RegSyntax) -> i32 {
        let input = &mut self.regexp;
        if input.eoi() {
            token.ty = END_OF_RE;
            return 0;
        }
        let c = input.peek_byte(0);
        token.c = c;
        token.word_char = false;
        if c == b'\\' {
            if input.cur_idx + 1 >= input.len {
                token.ty = BACK_SLASH;
                return 1;
            }
            let c2 = input.peek_byte_case(1);
            token.c = c2;
            token.ty = CHARACTER;
            token.word_char = is_word_char(c2 as i32);
            match c2 {
                b'|' => {
                    if syntax & RE_LIMITED_OPS == 0 && syntax & RE_NO_BK_VBAR == 0 {
                        token.ty = OP_ALT;
                    }
                }
                b'1'..=b'9' => {
                    if syntax & RE_NO_BK_REFS == 0 {
                        token.ty = OP_BACK_REF;
                        token.idx = (c2 - b'1') as i32;
                    }
                }
                b'<' => {
                    if syntax & RE_NO_GNU_OPS == 0 {
                        token.ty = ANCHOR;
                        token.ctx_type = WORD_FIRST;
                    }
                }
                b'>' => {
                    if syntax & RE_NO_GNU_OPS == 0 {
                        token.ty = ANCHOR;
                        token.ctx_type = WORD_LAST;
                    }
                }
                b'b' => {
                    if syntax & RE_NO_GNU_OPS == 0 {
                        token.ty = ANCHOR;
                        token.ctx_type = WORD_DELIM;
                    }
                }
                b'B' => {
                    if syntax & RE_NO_GNU_OPS == 0 {
                        token.ty = ANCHOR;
                        token.ctx_type = NOT_WORD_DELIM;
                    }
                }
                b'w' => {
                    if syntax & RE_NO_GNU_OPS == 0 {
                        token.ty = OP_WORD;
                    }
                }
                b'W' => {
                    if syntax & RE_NO_GNU_OPS == 0 {
                        token.ty = OP_NOTWORD;
                    }
                }
                b's' => {
                    if syntax & RE_NO_GNU_OPS == 0 {
                        token.ty = OP_SPACE;
                    }
                }
                b'S' => {
                    if syntax & RE_NO_GNU_OPS == 0 {
                        token.ty = OP_NOTSPACE;
                    }
                }
                b'`' => {
                    if syntax & RE_NO_GNU_OPS == 0 {
                        token.ty = ANCHOR;
                        token.ctx_type = BUF_FIRST;
                    }
                }
                b'\'' => {
                    if syntax & RE_NO_GNU_OPS == 0 {
                        token.ty = ANCHOR;
                        token.ctx_type = BUF_LAST;
                    }
                }
                b'(' => {
                    if syntax & RE_NO_BK_PARENS == 0 {
                        token.ty = OP_OPEN_SUBEXP;
                    }
                }
                b')' => {
                    if syntax & RE_NO_BK_PARENS == 0 {
                        token.ty = OP_CLOSE_SUBEXP;
                    }
                }
                b'+' => {
                    if syntax & RE_LIMITED_OPS == 0 && syntax & RE_BK_PLUS_QM != 0 {
                        token.ty = OP_DUP_PLUS;
                    }
                }
                b'?' => {
                    if syntax & RE_LIMITED_OPS == 0 && syntax & RE_BK_PLUS_QM != 0 {
                        token.ty = OP_DUP_QUESTION;
                    }
                }
                b'{' => {
                    if syntax & RE_INTERVALS != 0 && syntax & RE_NO_BK_BRACES == 0 {
                        token.ty = OP_OPEN_DUP_NUM;
                    }
                }
                b'}' => {
                    if syntax & RE_INTERVALS != 0 && syntax & RE_NO_BK_BRACES == 0 {
                        token.ty = OP_CLOSE_DUP_NUM;
                    }
                }
                _ => {}
            }
            return 2;
        }

        token.ty = CHARACTER;
        token.word_char = is_word_char(token.c as i32);

        match c {
            b'\n' => {
                if syntax & RE_NEWLINE_ALT != 0 {
                    token.ty = OP_ALT;
                }
            }
            b'|' => {
                if syntax & RE_LIMITED_OPS == 0 && syntax & RE_NO_BK_VBAR != 0 {
                    token.ty = OP_ALT;
                }
            }
            b'*' => {
                token.ty = OP_DUP_ASTERISK;
            }
            b'+' => {
                if syntax & RE_LIMITED_OPS == 0 && syntax & RE_BK_PLUS_QM == 0 {
                    token.ty = OP_DUP_PLUS;
                }
            }
            b'?' => {
                if syntax & RE_LIMITED_OPS == 0 && syntax & RE_BK_PLUS_QM == 0 {
                    token.ty = OP_DUP_QUESTION;
                }
            }
            b'{' => {
                if syntax & RE_INTERVALS != 0 && syntax & RE_NO_BK_BRACES != 0 {
                    token.ty = OP_OPEN_DUP_NUM;
                }
            }
            b'}' => {
                if syntax & RE_INTERVALS != 0 && syntax & RE_NO_BK_BRACES != 0 {
                    token.ty = OP_CLOSE_DUP_NUM;
                }
            }
            b'(' => {
                if syntax & RE_NO_BK_PARENS != 0 {
                    token.ty = OP_OPEN_SUBEXP;
                }
            }
            b')' => {
                if syntax & RE_NO_BK_PARENS != 0 {
                    token.ty = OP_CLOSE_SUBEXP;
                }
            }
            b'[' => {
                token.ty = OP_OPEN_BRACKET;
            }
            b'.' => {
                token.ty = OP_PERIOD;
            }
            b'^' => {
                if syntax & (RE_CONTEXT_INDEP_ANCHORS | RE_CARET_ANCHORS_HERE) == 0 && input.cur_idx != 0 {
                    let prev = input.peek_byte(-1);
                    if syntax & RE_NEWLINE_ALT == 0 || prev != b'\n' {
                        return 1;
                    }
                }
                token.ty = ANCHOR;
                token.ctx_type = LINE_FIRST;
            }
            b'$' => {
                if syntax & RE_CONTEXT_INDEP_ANCHORS == 0 && input.cur_idx + 1 != input.len {
                    let mut next = Token::default();
                    self.regexp.skip_bytes(1);
                    self.peek_token(&mut next, syntax);
                    self.regexp.skip_bytes(-1);
                    if next.ty != OP_ALT && next.ty != OP_CLOSE_SUBEXP {
                        return 1;
                    }
                }
                token.ty = ANCHOR;
                token.ctx_type = LINE_LAST;
            }
            _ => {}
        }
        1
    }

    /// peek_token_bracket
    fn peek_token_bracket(&mut self, token: &mut Token, syntax: RegSyntax) -> i32 {
        let input = &mut self.regexp;
        if input.eoi() {
            token.ty = END_OF_RE;
            return 0;
        }
        let c = input.peek_byte(0);
        token.c = c;

        if c == b'\\' && syntax & RE_BACKSLASH_ESCAPE_IN_LISTS != 0 && input.cur_idx + 1 < input.len {
            input.skip_bytes(1);
            let c2 = input.peek_byte(0);
            token.c = c2;
            token.ty = CHARACTER;
            return 1;
        }
        if c == b'[' {
            let c2 = if input.cur_idx + 1 < input.len { input.peek_byte(1) } else { 0 };
            token.c = c2;
            let mut token_len = 2;
            match c2 {
                b'.' => token.ty = OP_OPEN_COLL_ELEM,
                b'=' => token.ty = OP_OPEN_EQUIV_CLASS,
                b':' if syntax & RE_CHAR_CLASSES != 0 => token.ty = OP_OPEN_CHAR_CLASS,
                _ => {
                    token.ty = CHARACTER;
                    token.c = c;
                    token_len = 1;
                }
            }
            return token_len;
        }
        token.ty = match c {
            b'-' => OP_CHARSET_RANGE,
            b']' => OP_CLOSE_BRACKET,
            b'^' => OP_NON_MATCH_LIST,
            _ => CHARACTER,
        };
        1
    }

    // -----------------------------------------------------------------------
    // Parser.

    /// parse
    fn parse(&mut self, syntax: RegSyntax, err: &mut RegErrcode) -> Option<usize> {
        self.dfa.syntax = syntax;
        let mut current_token = Token::default();
        self.fetch_token(&mut current_token, syntax | RE_CARET_ANCHORS_HERE);
        let tree = self.parse_reg_exp(&mut current_token, syntax, 0, err);
        if *err != REG_NOERROR && tree.is_none() {
            return None;
        }
        let eor = self.create_tree(None, None, END_OF_RE);
        let root = match (tree, eor) {
            (Some(t), Some(e)) => self.create_tree(Some(t), Some(e), CONCAT),
            (None, e) => e,
            _ => None,
        };
        if eor.is_none() || root.is_none() {
            *err = REG_ESPACE;
            return None;
        }
        root
    }

    /// parse_reg_exp
    fn parse_reg_exp(&mut self, token: &mut Token, syntax: RegSyntax, nest: i32, err: &mut RegErrcode) -> Option<usize> {
        if nest as u32 > self.depth_limit / 4 {
            *err = REG_NEED_STACK;
            return None;
        }
        let mut tree = self.parse_branch(token, syntax, nest, err);
        if *err != REG_NOERROR && tree.is_none() {
            return None;
        }
        while token.ty == OP_ALT {
            self.fetch_token(token, syntax | RE_CARET_ANCHORS_HERE);
            let branch;
            if token.ty != OP_ALT && token.ty != END_OF_RE && (nest == 0 || token.ty != OP_CLOSE_SUBEXP) {
                branch = self.parse_branch(token, syntax, nest, err);
                if *err != REG_NOERROR && branch.is_none() {
                    return None;
                }
            } else {
                branch = None;
            }
            tree = self.create_tree(tree, branch, OP_ALT);
            if tree.is_none() {
                *err = REG_ESPACE;
                return None;
            }
        }
        tree
    }

    /// parse_branch
    fn parse_branch(&mut self, token: &mut Token, syntax: RegSyntax, nest: i32, err: &mut RegErrcode) -> Option<usize> {
        let mut tree = self.parse_expression(token, syntax, nest, err);
        if *err != REG_NOERROR && tree.is_none() {
            return None;
        }
        while token.ty != OP_ALT && token.ty != END_OF_RE && (nest == 0 || token.ty != OP_CLOSE_SUBEXP) {
            let exp = self.parse_expression(token, syntax, nest, err);
            if *err != REG_NOERROR && exp.is_none() {
                return None;
            }
            if tree.is_some() && exp.is_some() {
                tree = self.create_tree(tree, exp, CONCAT);
                if tree.is_none() {
                    *err = REG_ESPACE;
                    return None;
                }
            } else if tree.is_none() {
                tree = exp;
            }
        }
        tree
    }

    /// parse_expression
    fn parse_expression(&mut self, token: &mut Token, syntax: RegSyntax, nest: i32, err: &mut RegErrcode) -> Option<usize> {
        let mut tree: Option<usize>;
        let mut ty = token.ty;
        // The C switch with fall-through is expressed as a small state machine.
        loop {
            match ty {
                CHARACTER => {
                    tree = self.create_token_tree(None, None, token);
                    if tree.is_none() {
                        *err = REG_ESPACE;
                        return None;
                    }
                    break;
                }
                OP_OPEN_SUBEXP => {
                    tree = self.parse_sub_exp(token, syntax, nest + 1, err);
                    if *err != REG_NOERROR && tree.is_none() {
                        return None;
                    }
                    break;
                }
                OP_OPEN_BRACKET => {
                    tree = self.parse_bracket_exp(token, syntax, err);
                    if *err != REG_NOERROR && tree.is_none() {
                        return None;
                    }
                    break;
                }
                OP_BACK_REF => {
                    if self.dfa.completed_bkref_map & 1u32.wrapping_shl(token.idx as u32) == 0 {
                        *err = REG_ESUBREG;
                        return None;
                    }
                    self.dfa.used_bkref_map |= 1u32.wrapping_shl(token.idx as u32);
                    tree = self.create_token_tree(None, None, token);
                    if tree.is_none() {
                        *err = REG_ESPACE;
                        return None;
                    }
                    self.dfa.nbackref += 1;
                    self.dfa.has_mb_node = true;
                    break;
                }
                OP_OPEN_DUP_NUM => {
                    if syntax & RE_CONTEXT_INVALID_DUP != 0 {
                        *err = REG_BADRPT;
                        return None;
                    }
                    ty = OP_DUP_ASTERISK; // fall through
                    continue;
                }
                OP_DUP_ASTERISK | OP_DUP_PLUS | OP_DUP_QUESTION => {
                    if syntax & RE_CONTEXT_INVALID_OPS != 0 {
                        *err = REG_BADRPT;
                        return None;
                    } else if syntax & RE_CONTEXT_INDEP_OPS != 0 {
                        self.fetch_token(token, syntax);
                        return self.parse_expression(token, syntax, nest, err);
                    }
                    ty = OP_CLOSE_SUBEXP; // fall through
                    continue;
                }
                OP_CLOSE_SUBEXP => {
                    if token.ty == OP_CLOSE_SUBEXP && syntax & RE_UNMATCHED_RIGHT_PAREN_ORD == 0 {
                        *err = REG_ERPAREN;
                        return None;
                    }
                    ty = OP_CLOSE_DUP_NUM; // fall through
                    continue;
                }
                OP_CLOSE_DUP_NUM => {
                    token.ty = CHARACTER;
                    tree = self.create_token_tree(None, None, token);
                    if tree.is_none() {
                        *err = REG_ESPACE;
                        return None;
                    }
                    break;
                }
                ANCHOR => {
                    if token.ctx_type & (WORD_DELIM | NOT_WORD_DELIM | WORD_FIRST | WORD_LAST) != 0
                        && !self.dfa.word_ops_used
                    {
                        self.init_word_char();
                    }
                    if token.ctx_type == WORD_DELIM || token.ctx_type == NOT_WORD_DELIM {
                        let tree_first;
                        if token.ctx_type == WORD_DELIM {
                            token.ctx_type = WORD_FIRST;
                            tree_first = self.create_token_tree(None, None, token);
                            token.ctx_type = WORD_LAST;
                        } else {
                            token.ctx_type = INSIDE_WORD;
                            tree_first = self.create_token_tree(None, None, token);
                            token.ctx_type = INSIDE_NOTWORD;
                        }
                        let tree_last = self.create_token_tree(None, None, token);
                        let t = match (tree_first, tree_last) {
                            (Some(a), Some(b)) => self.create_tree(Some(a), Some(b), OP_ALT),
                            _ => None,
                        };
                        if t.is_none() {
                            *err = REG_ESPACE;
                            return None;
                        }
                        tree = t;
                    } else {
                        tree = self.create_token_tree(None, None, token);
                        if tree.is_none() {
                            *err = REG_ESPACE;
                            return None;
                        }
                    }
                    // ANCHORs can't be followed by repetition operators.
                    self.fetch_token(token, syntax);
                    return tree;
                }
                OP_PERIOD => {
                    tree = self.create_token_tree(None, None, token);
                    if tree.is_none() {
                        *err = REG_ESPACE;
                        return None;
                    }
                    if self.dfa.mb_cur_max > 1 {
                        self.dfa.has_mb_node = true;
                    }
                    break;
                }
                OP_WORD | OP_NOTWORD => {
                    tree = self.build_charclass_op(b"alnum", b"_", token.ty == OP_NOTWORD, err);
                    if *err != REG_NOERROR && tree.is_none() {
                        return None;
                    }
                    break;
                }
                OP_SPACE | OP_NOTSPACE => {
                    tree = self.build_charclass_op(b"space", b"", token.ty == OP_NOTSPACE, err);
                    if *err != REG_NOERROR && tree.is_none() {
                        return None;
                    }
                    break;
                }
                OP_ALT | END_OF_RE => return None,
                BACK_SLASH => {
                    *err = REG_EESCAPE;
                    return None;
                }
                _ => return None,
            }
        }
        self.fetch_token(token, syntax);

        while token.ty == OP_DUP_ASTERISK
            || token.ty == OP_DUP_PLUS
            || token.ty == OP_DUP_QUESTION
            || token.ty == OP_OPEN_DUP_NUM
        {
            tree = self.parse_dup_op(tree, token, syntax, err);
            if *err != REG_NOERROR && tree.is_none() {
                return None;
            }
            if syntax & RE_CONTEXT_INVALID_DUP != 0 && (token.ty == OP_DUP_ASTERISK || token.ty == OP_OPEN_DUP_NUM) {
                *err = REG_BADRPT;
                return None;
            }
        }
        tree
    }

    /// parse_sub_exp
    fn parse_sub_exp(&mut self, token: &mut Token, syntax: RegSyntax, nest: i32, err: &mut RegErrcode) -> Option<usize> {
        let cur_nsub = self.re_nsub;
        self.re_nsub += 1;
        self.fetch_token(token, syntax | RE_CARET_ANCHORS_HERE);

        let tree;
        if token.ty == OP_CLOSE_SUBEXP {
            tree = None;
        } else {
            tree = self.parse_reg_exp(token, syntax, nest, err);
            if *err == REG_NOERROR && token.ty != OP_CLOSE_SUBEXP {
                *err = REG_EPAREN;
            }
            if *err != REG_NOERROR {
                return None;
            }
        }

        if cur_nsub <= (b'9' - b'1') as usize {
            self.dfa.completed_bkref_map |= 1u32 << cur_nsub;
        }

        let t = self.create_tree(tree, None, SUBEXP);
        match t {
            None => {
                *err = REG_ESPACE;
                None
            }
            Some(t) => {
                self.tree[t].token.idx = cur_nsub as i32;
                Some(t)
            }
        }
    }

    /// parse_dup_op
    fn parse_dup_op(&mut self, elem: Option<usize>, token: &mut Token, syntax: RegSyntax, err: &mut RegErrcode) -> Option<usize> {
        let mut elem = elem;
        let start_idx = self.regexp.cur_idx;
        let start_token = *token;
        let start: i32;
        let end: i32;

        if token.ty == OP_OPEN_DUP_NUM {
            let mut e = 0;
            let mut s = self.fetch_number(token, syntax);
            if s == -1 {
                if token.ty == CHARACTER && token.c == b',' {
                    s = 0; // We treat "{,m}" as "{0,m}".
                } else {
                    *err = REG_BADBR; // <re>{} is invalid.
                    return None;
                }
            }
            if s != -2 {
                // We treat "{n}" as "{n,n}".
                e = if token.ty == OP_CLOSE_DUP_NUM {
                    s
                } else if token.ty == CHARACTER && token.c == b',' {
                    self.fetch_number(token, syntax)
                } else {
                    -2
                };
            }
            if s == -2 || e == -2 {
                // Invalid sequence.
                if syntax & RE_INVALID_INTERVAL_ORD == 0 {
                    *err = if token.ty == END_OF_RE { REG_EBRACE } else { REG_BADBR };
                    return None;
                }
                // If the syntax bit is set, rollback.
                self.regexp.cur_idx = start_idx;
                *token = start_token;
                token.ty = CHARACTER;
                return elem;
            }
            if e != -1 && s > e {
                *err = REG_BADBR;
                return None;
            }
            start = s;
            end = e;
        } else {
            start = if token.ty == OP_DUP_PLUS { 1 } else { 0 };
            end = if token.ty == OP_DUP_QUESTION { 1 } else { -1 };
        }

        self.fetch_token(token, syntax);

        let el = elem?;
        if start == 0 && end == 0 {
            self.postorder(el, Visit::FreeTree);
            return None;
        }

        let mut tree: Option<usize>;
        let old_tree: Option<usize>;
        // Extract "<re>{n,m}" to "<re><re>...<re><re>{0,<m-n>}".
        if start > 0 {
            tree = elem;
            let mut i = 2;
            while i <= start {
                elem = self.duplicate_tree(elem.unwrap());
                tree = match (tree, elem) {
                    (Some(t), Some(e)) => self.create_tree(Some(t), Some(e), CONCAT),
                    _ => None,
                };
                if elem.is_none() || tree.is_none() {
                    *err = REG_ESPACE;
                    return None;
                }
                i += 1;
            }
            if start == end {
                return tree;
            }
            // Duplicate ELEM before it is marked optional.
            elem = self.duplicate_tree(elem.unwrap());
            if elem.is_none() {
                *err = REG_ESPACE;
                return None;
            }
            old_tree = tree;
        } else {
            old_tree = None;
        }

        let el = elem.unwrap();
        if self.tree[el].token.ty == SUBEXP {
            let idx = self.tree[el].token.idx;
            self.postorder(el, Visit::MarkOptSubexp(idx));
        }

        tree = self.create_tree(Some(el), None, if end == -1 { OP_DUP_ASTERISK } else { OP_ALT });
        if tree.is_none() {
            *err = REG_ESPACE;
            return None;
        }

        // This loop is actually executed only when end != -1.
        let mut i = start + 2;
        while i <= end {
            elem = self.duplicate_tree(elem.unwrap());
            tree = match (tree, elem) {
                (Some(t), Some(e)) => self.create_tree(Some(t), Some(e), CONCAT),
                _ => None,
            };
            if elem.is_none() || tree.is_none() {
                *err = REG_ESPACE;
                return None;
            }
            tree = self.create_tree(tree, None, OP_ALT);
            if tree.is_none() {
                *err = REG_ESPACE;
                return None;
            }
            i += 1;
        }

        if old_tree.is_some() {
            tree = self.create_tree(old_tree, tree, CONCAT);
            if tree.is_none() {
                *err = REG_ESPACE;
                return None;
            }
        }
        tree
    }

    /// fetch_number
    fn fetch_number(&mut self, token: &mut Token, syntax: RegSyntax) -> i32 {
        let mut num: i32 = -1;
        loop {
            self.fetch_token(token, syntax);
            let c = token.c;
            if token.ty == END_OF_RE {
                return -2;
            }
            if token.ty == OP_CLOSE_DUP_NUM || c == b',' {
                break;
            }
            num = if token.ty != CHARACTER || c < b'0' || b'9' < c || num == -2 {
                -2
            } else if num == -1 {
                (c - b'0') as i32
            } else {
                num * 10 + (c - b'0') as i32
            };
            num = if num > RE_DUP_MAX { -2 } else { num };
        }
        num
    }

    // -----------------------------------------------------------------------
    // Bracket expressions.

    /// parse_bracket_exp
    fn parse_bracket_exp(&mut self, token: &mut Token, syntax: RegSyntax, err: &mut RegErrcode) -> Option<usize> {
        let mut sbcset: Bitset = [0; BITSET_WORDS];
        let mut non_match = false;
        let mut first_round = true;

        let mut token_len = self.peek_token_bracket(token, syntax);
        if token.ty == END_OF_RE {
            *err = REG_BADPAT;
            return None;
        }
        if token.ty == OP_NON_MATCH_LIST {
            non_match = true;
            if syntax & RE_HAT_LISTS_NOT_NEWLINE != 0 {
                // glibc 2.5 sets '\0' here (later glibc: '\n').
                bitset_set(&mut sbcset, 0);
            }
            self.regexp.skip_bytes(token_len);
            token_len = self.peek_token_bracket(token, syntax);
            if token.ty == END_OF_RE {
                *err = REG_BADPAT;
                return None;
            }
        }

        // We treat the first ']' as a normal character.
        if token.ty == OP_CLOSE_BRACKET {
            token.ty = CHARACTER;
        }

        loop {
            let mut start_elem = BracketElem { ty: BracketElemType::SbChar, ch: 0, name: Vec::new() };
            let mut end_elem = BracketElem { ty: BracketElemType::SbChar, ch: 0, name: Vec::new() };
            let mut token2 = Token::default();
            let mut token_len2 = 0;
            let mut is_range_exp = false;

            let ret = self.parse_bracket_element(&mut start_elem, token, token_len, syntax, first_round);
            if ret != REG_NOERROR {
                *err = ret;
                return None;
            }
            first_round = false;

            // Get information about the next token.  We need it in any case.
            token_len = self.peek_token_bracket(token, syntax);

            // Do not check for ranges if we know they are not allowed.
            if start_elem.ty != BracketElemType::CharClass && start_elem.ty != BracketElemType::EquivClass {
                if token.ty == END_OF_RE {
                    *err = REG_EBRACK;
                    return None;
                }
                if token.ty == OP_CHARSET_RANGE {
                    self.regexp.skip_bytes(token_len); // Skip '-'.
                    token_len2 = self.peek_token_bracket(&mut token2, syntax);
                    if token2.ty == END_OF_RE {
                        *err = REG_EBRACK;
                        return None;
                    }
                    if token2.ty == OP_CLOSE_BRACKET {
                        // We treat the last '-' as a normal character.
                        self.regexp.skip_bytes(-token_len);
                        token.ty = CHARACTER;
                    } else {
                        is_range_exp = true;
                    }
                }
            }

            if is_range_exp {
                let ret = self.parse_bracket_element(&mut end_elem, &mut token2, token_len2, syntax, true);
                if ret != REG_NOERROR {
                    *err = ret;
                    return None;
                }
                token_len = self.peek_token_bracket(token, syntax);
                *err = build_range_exp(&mut sbcset, &start_elem, &end_elem);
                if *err != REG_NOERROR {
                    return None;
                }
            } else {
                match start_elem.ty {
                    BracketElemType::SbChar => bitset_set(&mut sbcset, start_elem.ch as usize),
                    BracketElemType::EquivClass => {
                        *err = build_equiv_class(&mut sbcset, &start_elem.name);
                        if *err != REG_NOERROR {
                            return None;
                        }
                    }
                    BracketElemType::CollSym => {
                        *err = build_collating_symbol(&mut sbcset, &start_elem.name);
                        if *err != REG_NOERROR {
                            return None;
                        }
                    }
                    BracketElemType::CharClass => {
                        *err = build_charclass(&mut sbcset, &start_elem.name, syntax);
                        if *err != REG_NOERROR {
                            return None;
                        }
                    }
                }
            }
            if token.ty == END_OF_RE {
                *err = REG_EBRACK;
                return None;
            }
            if token.ty == OP_CLOSE_BRACKET {
                break;
            }
        }

        self.regexp.skip_bytes(token_len); // Skip a token.

        if non_match {
            bitset_not(&mut sbcset);
        }

        let set_idx = self.dfa.sbcsets.len() as u32;
        self.dfa.sbcsets.push(sbcset);
        let br_token = Token { ty: SIMPLE_BRACKET, sbcset: set_idx, ..Token::default() };
        let t = self.create_token_tree(None, None, &br_token);
        if t.is_none() {
            *err = REG_ESPACE;
        }
        t
    }

    /// parse_bracket_element
    fn parse_bracket_element(
        &mut self,
        elem: &mut BracketElem,
        token: &mut Token,
        token_len: i32,
        syntax: RegSyntax,
        accept_hyphen: bool,
    ) -> RegErrcode {
        self.regexp.skip_bytes(token_len); // Skip a token.
        if token.ty == OP_OPEN_COLL_ELEM || token.ty == OP_OPEN_CHAR_CLASS || token.ty == OP_OPEN_EQUIV_CLASS {
            return self.parse_bracket_symbol(elem, token);
        }
        if token.ty == OP_CHARSET_RANGE && !accept_hyphen {
            // A '-' must only appear as anything but a range indicator before
            // the closing bracket.  Everything else is an error.
            let mut token2 = Token::default();
            self.peek_token_bracket(&mut token2, syntax);
            if token2.ty != OP_CLOSE_BRACKET {
                return REG_ERANGE;
            }
        }
        elem.ty = BracketElemType::SbChar;
        elem.ch = token.c;
        REG_NOERROR
    }

    /// parse_bracket_symbol
    fn parse_bracket_symbol(&mut self, elem: &mut BracketElem, token: &Token) -> RegErrcode {
        let delim = token.c;
        let mut i = 0;
        if self.regexp.eoi() {
            return REG_EBRACK;
        }
        elem.name.clear();
        loop {
            if i >= BRACKET_NAME_BUF_SIZE {
                return REG_EBRACK;
            }
            let ch = if token.ty == OP_OPEN_CHAR_CLASS {
                self.regexp.fetch_byte_case()
            } else {
                self.regexp.fetch_byte()
            };
            if self.regexp.eoi() {
                return REG_EBRACK;
            }
            if ch == delim && self.regexp.peek_byte(0) == b']' {
                break;
            }
            elem.name.push(ch);
            i += 1;
        }
        self.regexp.skip_bytes(1);
        match token.ty {
            OP_OPEN_COLL_ELEM => elem.ty = BracketElemType::CollSym,
            OP_OPEN_EQUIV_CLASS => elem.ty = BracketElemType::EquivClass,
            OP_OPEN_CHAR_CLASS => elem.ty = BracketElemType::CharClass,
            _ => {}
        }
        REG_NOERROR
    }

    /// build_charclass_op
    fn build_charclass_op(&mut self, class_name: &[u8], extra: &[u8], non_match: bool, err: &mut RegErrcode) -> Option<usize> {
        let mut sbcset: Bitset = [0; BITSET_WORDS];
        // We don't care the syntax in this case.
        let ret = build_charclass(&mut sbcset, class_name, 0);
        if ret != REG_NOERROR {
            *err = ret;
            return None;
        }
        // \w match '_' also.
        for &e in extra {
            bitset_set(&mut sbcset, e as usize);
        }
        if non_match {
            bitset_not(&mut sbcset);
        }
        let set_idx = self.dfa.sbcsets.len() as u32;
        self.dfa.sbcsets.push(sbcset);
        let br_token = Token { ty: SIMPLE_BRACKET, sbcset: set_idx, ..Token::default() };
        let t = self.create_token_tree(None, None, &br_token);
        if t.is_none() {
            *err = REG_ESPACE;
        }
        t
    }

    /// init_word_char
    fn init_word_char(&mut self) {
        self.dfa.word_ops_used = true;
        let mut ch = 0i32;
        for i in 0..BITSET_WORDS {
            for j in 0..BITSET_WORD_BITS {
                if ctype::isalnum(ch) || ch == b'_' as i32 {
                    self.dfa.word_char[i] |= 1u32 << j;
                }
                ch += 1;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Initial states.

    /// create_initial_state
    fn create_initial_state(&mut self) -> RegErrcode {
        let root = self.str_tree.unwrap();
        let first_tree = self.tree[root].first.unwrap();
        let first = self.tree[first_tree].node_idx;
        self.dfa.init_node = first;
        let mut init_nodes = NodeSet::init_copy(&self.dfa.eclosures[first as usize]);

        // The back-references which are in initial states can epsilon
        // transit, since in this case all of the subexpressions can be null.
        if self.dfa.nbackref > 0 {
            let mut i: i32 = 0;
            while i < init_nodes.nelem {
                let node_idx = init_nodes.elems[i as usize];
                let ty = self.dfa.nodes[node_idx as usize].ty;
                if ty != OP_BACK_REF {
                    i += 1;
                    continue;
                }
                let mut clexp_idx = 0;
                while clexp_idx < init_nodes.nelem {
                    let clexp_node = &self.dfa.nodes[init_nodes.elems[clexp_idx as usize] as usize];
                    if clexp_node.ty == OP_CLOSE_SUBEXP && clexp_node.idx == self.dfa.nodes[node_idx as usize].idx {
                        break;
                    }
                    clexp_idx += 1;
                }
                if clexp_idx == init_nodes.nelem {
                    i += 1;
                    continue;
                }
                if ty == OP_BACK_REF {
                    let dest_idx = self.dfa.edests[node_idx as usize].elems[0];
                    if init_nodes.contains(dest_idx) == 0 {
                        let ecl = self.dfa.eclosures[dest_idx as usize].clone();
                        init_nodes.merge(&ecl);
                        i = 0;
                    }
                }
                i += 1;
            }
        }

        // It must be the first time to invoke acquire_state.
        let dfa = &self.dfa;
        let init_state = self.cache.acquire_state_context(dfa, &init_nodes, 0);
        let init_state = match init_state {
            Some(s) => s,
            None => return REG_ESPACE,
        };
        self.dfa.init_state = Some(init_state);
        if self.cache.st(init_state).has_constraint {
            let dfa = &self.dfa;
            let w = self.cache.acquire_state_context(dfa, &init_nodes, CONTEXT_WORD);
            let nl = self.cache.acquire_state_context(dfa, &init_nodes, CONTEXT_NEWLINE);
            let bb = self.cache.acquire_state_context(dfa, &init_nodes, CONTEXT_NEWLINE | CONTEXT_BEGBUF);
            self.dfa.init_state_word = w;
            self.dfa.init_state_nl = nl;
            self.dfa.init_state_begbuf = bb;
            if w.is_none() || nl.is_none() || bb.is_none() {
                return REG_ESPACE;
            }
        } else {
            self.dfa.init_state_word = Some(init_state);
            self.dfa.init_state_nl = Some(init_state);
            self.dfa.init_state_begbuf = Some(init_state);
        }
        REG_NOERROR
    }
}

/// build_range_exp (non-_LIBC version)
fn build_range_exp(sbcset: &mut Bitset, start_elem: &BracketElem, end_elem: &BracketElem) -> RegErrcode {
    use BracketElemType::*;
    // Equivalence Classes and Character Classes can't be a range start/end.
    if start_elem.ty == EquivClass || start_elem.ty == CharClass || end_elem.ty == EquivClass || end_elem.ty == CharClass {
        return REG_ERANGE;
    }
    // We can handle no multi character collating elements without libc support.
    if (start_elem.ty == CollSym && start_elem.name.len() > 1) || (end_elem.ty == CollSym && end_elem.name.len() > 1) {
        return REG_ECOLLATE;
    }
    let start_ch: u32 = match start_elem.ty {
        SbChar => start_elem.ch as u32,
        CollSym => start_elem.name.first().copied().unwrap_or(0) as u32,
        _ => 0,
    };
    let end_ch: u32 = match end_elem.ty {
        SbChar => end_elem.ch as u32,
        CollSym => end_elem.name.first().copied().unwrap_or(0) as u32,
        _ => 0,
    };
    if start_ch > end_ch {
        return REG_ERANGE;
    }
    for ch in 0..SBC_MAX as u32 {
        if start_ch <= ch && ch <= end_ch {
            bitset_set(sbcset, ch as usize);
        }
    }
    REG_NOERROR
}

/// build_collating_symbol (non-_LIBC version)
fn build_collating_symbol(sbcset: &mut Bitset, name: &[u8]) -> RegErrcode {
    if name.len() != 1 {
        REG_ECOLLATE
    } else {
        bitset_set(sbcset, name[0] as usize);
        REG_NOERROR
    }
}

/// build_equiv_class (non-_LIBC version)
fn build_equiv_class(sbcset: &mut Bitset, name: &[u8]) -> RegErrcode {
    if name.len() != 1 {
        return REG_ECOLLATE;
    }
    bitset_set(sbcset, name[0] as usize);
    REG_NOERROR
}

/// build_charclass (trans == NULL)
fn build_charclass(sbcset: &mut Bitset, class_name: &[u8], syntax: RegSyntax) -> RegErrcode {
    let mut name: &[u8] = class_name;
    // In case of REG_ICASE "upper" and "lower" match the both of upper and
    // lower cases.
    if syntax & RE_ICASE != 0 && (name == b"upper" || name == b"lower") {
        name = b"alpha";
    }
    let f: fn(i32) -> bool = match name {
        b"alnum" => ctype::isalnum,
        b"cntrl" => ctype::iscntrl,
        b"lower" => ctype::islower,
        b"space" => ctype::isspace,
        b"alpha" => ctype::isalpha,
        b"digit" => ctype::isdigit,
        b"print" => ctype::isprint,
        b"upper" => ctype::isupper,
        b"blank" => ctype::isblank,
        b"graph" => ctype::isgraph,
        b"punct" => ctype::ispunct,
        b"xdigit" => ctype::isxdigit,
        _ => return REG_ECTYPE,
    };
    for i in 0..SBC_MAX as i32 {
        if f(i) {
            bitset_set(sbcset, i as usize);
        }
    }
    REG_NOERROR
}
