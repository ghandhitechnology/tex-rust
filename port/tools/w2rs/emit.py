"""Pascal AST (pas.py) -> Rust source for the pdfTeX engine.

Semantics follow web2c (texk/web2c/web2c/web2c-parser.y, fixwrites.c, cpascal.h,
texmfmp.h) so the generated engine behaves exactly like TeX Live's C build:
  * subrange types become the smallest C integer type when both bounds are literal
    numbers, else `integer`; arithmetic is done in C `int` (i32) after promotion;
  * `for` loops run `v := a; if v <= b then repeat body until not (v++ < b)`;
  * write/writeln arguments are classified the way fixwrites does.
Every array becomes a `Ptr<T>` (raw pointer, C indexing semantics) so the Pascal's
pointer arithmetic (hash := yhash - hashoffset, ...) and the C library ports share
the same memory exactly as web2c's C does.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pas  # noqa: E402
import externs  # noqa: E402

RUST_KW = {
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for",
    "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use",
    "where", "while", "async", "await", "dyn", "abstract", "become", "box", "do", "final",
    "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "gen", "union",
}

I32 = ("int", "i32")
I64 = ("int", "i64")
BOOL = ("bool",)
REAL = ("real",)
STR = ("str",)
NIL = ("nil",)
UNIT = ("unit",)
U8 = ("int", "u8")
CSTR = ("ptr", ("int", "u8"))

WORD_TYPES = {"memoryword": "MemoryWord", "twohalves": "TwoHalves", "fourquarters": "FourQuarters",
              "fmemoryword": "FMemoryWord"}

# accessor -> (rust method stem, value type) per word type
WORD_FIELDS = {
    "MemoryWord": {
        "hh": ("hh", ("word", "TwoHalves")), "hh.rh": ("hh_rh", I32), "hh.lh": ("hh_lh", I32),
        "hh.b0": ("hh_b0", ("int", "i16")), "hh.b1": ("hh_b1", ("int", "i16")),
        "int": ("int", I32), "sc": ("int", I32), "gr": ("gr", REAL),
        "qqqq": ("qqqq", ("word", "FourQuarters")), "qqqq.b0": ("qqqq_b0", U8),
        "qqqq.b1": ("qqqq_b1", U8), "qqqq.b2": ("qqqq_b2", U8), "qqqq.b3": ("qqqq_b3", U8),
    },
    "TwoHalves": {"rh": ("rh", I32), "lh": ("lh", I32), "b0": ("b0", ("int", "i16")),
                  "b1": ("b1", ("int", "i16"))},
    "FourQuarters": {"b0": ("b0", U8), "b1": ("b1", U8), "b2": ("b2", U8), "b3": ("b3", U8)},
    "FMemoryWord": {
        "int": ("int", I32), "sc": ("int", I32), "qqqq": ("qqqq", ("word", "FourQuarters")),
        "qqqq.b0": ("qqqq_b0", U8), "qqqq.b1": ("qqqq_b1", U8), "qqqq.b2": ("qqqq_b2", U8),
        "qqqq.b3": ("qqqq_b3", U8),
    },
}
FIELD_ALIASES = {"lhfield": "lh", "hhfield": "hh"}

BUILTIN_TYPES = {
    "integer": I32, "boolean": BOOL, "real": REAL, "char": U8, "longinteger": I64,
    "integer64": I64, "cinttype": I32, "cstring": CSTR, "constcstring": CSTR,
    "gzFile": ("file", "word"), "text": ("file", "alpha"),
}


# web2c's regmem: every routine starts with `memoryword *mem=zmem, *eqtb=zeqtb`, so the
# Pascal names refer to the same arrays; we keep one variable for each pair.
ALIASES = {"zmem": "mem", "zeqtb": "eqtb"}


class TranslateError(Exception):
    pass


def ctype_for_range(lo, hi):
    if 0 <= lo and hi <= 255:
        return "u8"
    if -128 <= lo and hi <= 127:
        return "i8"
    if -32768 <= lo and hi <= 32767:
        return "i16"
    if 0 <= lo and hi <= 65535:
        return "u16"
    return "i32"


def literal_value(e):
    """Value of a literal number, or None if e is not a literal (web2c's lower_sym == -1)."""
    if e[0] == "num":
        return e[1]
    if e[0] == "un" and e[1] == "-" and e[2][0] == "num":
        return -e[2][1]
    return None


class Expr:
    """An emitted rvalue: Rust code, Pascal-level value type, and whether evaluating it calls
    a routine (so it cannot sit inside another &mut self call's argument list)."""
    __slots__ = ("code", "ty", "calls")

    def __init__(self, code, ty, calls=False):
        self.code, self.ty, self.calls = code, ty, calls


class LV:
    """An lvalue. kind 'place': `place` is a Rust place expression of declared type `ty`.
    kind 'word': `place` is a place of word type `wname`, `path` the selected fields."""
    __slots__ = ("kind", "place", "ty", "wname", "path", "calls")

    def __init__(self, kind, place, ty, wname=None, path=(), calls=False):
        self.kind, self.place, self.ty, self.wname, self.path, self.calls = kind, place, ty, wname, tuple(path), calls


class Emitter:
    def __init__(self, prog):
        self.prog = prog
        self.consts = {}
        self.type_ast = {}
        self.types = {}
        self.records = {}  # name -> [(field, type)]
        self.globals = {}  # name -> type
        self.routines = {}
        self.cur = None  # current routine dict
        self.locals = {}
        self.tmp = 0
        self.uses_unknown = set()
        self.prof_names = []
        self.setup()

    # ------------------------------------------------------------------ declarations
    def setup(self):
        for n, e in self.prog["consts"]:
            self.consts[n] = self.const_eval(e)
        for n, t in self.prog["types"]:
            self.type_ast[n] = t
        for n, t in self.prog["types"]:
            self.types[n] = self.resolve_type(t, name=n)
        for n, t in self.prog["vars"]:
            if n in ALIASES:
                continue
            self.globals[n] = self.resolve_type(t)
        for n, t in externs.EXTERN_VARS.items():
            self.globals[n] = t
        for p in self.prog["procs"]:
            if p["forward"] and p["name"] in self.routines:
                continue
            if not p["forward"] or p["name"] not in self.routines:
                self.routines[p["name"]] = p
        # forward declarations carry the parameter list; the body declaration may omit it
        fwd = {p["name"]: p for p in self.prog["procs"] if p["forward"]}
        for p in self.prog["procs"]:
            if not p["forward"]:
                if p["name"] in fwd and not p["params"] and fwd[p["name"]]["params"]:
                    p["params"] = fwd[p["name"]]["params"]
                    p["ret"] = p["ret"] or fwd[p["name"]]["ret"]
                self.routines[p["name"]] = p

    def const_eval(self, e):
        k = e[0]
        if k == "num":
            return e[1]
        if k == "real":
            return float(e[1])
        if k == "str":
            return e[1]
        if k == "un" and e[1] == "-":
            return -self.const_eval(e[2])
        if k == "bin":
            a, b = self.const_eval(e[2]), self.const_eval(e[3])
            return {"+": a + b, "-": a - b, "*": a * b}[e[1]]
        if k == "id":
            if e[1] in self.consts:
                return self.consts[e[1]]
            if e[1] in externs.EXTERN_CONSTS:
                return externs.EXTERN_CONSTS[e[1]]
        raise TranslateError(f"not a constant: {e}")

    def bounds(self, t):
        """(lo, hi) of an index type AST."""
        if t[0] == "sub":
            return self.const_eval(t[1]), self.const_eval(t[2])
        if t[0] == "name":
            ast = self.type_ast.get(t[1])
            if ast is None:
                raise TranslateError(f"unknown index type {t}")
            return self.bounds(ast)
        raise TranslateError(f"bad index type {t}")

    def resolve_type(self, t, name=None):
        k = t[0]
        if k == "name":
            n = t[1]
            if n in BUILTIN_TYPES:
                return BUILTIN_TYPES[n]
            if n in WORD_TYPES:
                return ("word", WORD_TYPES[n])
            if n in self.types:
                return self.types[n]
            if n in self.type_ast:
                return self.resolve_type(self.type_ast[n], n)
            raise TranslateError(f"unknown type {n}")
        if k == "sub":
            lo, hi = literal_value(t[1]), literal_value(t[2])
            if lo is None or hi is None:
                return I32
            return ("int", ctype_for_range(lo, hi))
        if k == "array":
            idx, elem = t[1], t[3 - 1]
            elem_t = self.resolve_type(t[2])
            for it in reversed(idx):
                lo, hi = self.bounds(it)
                elem_t = ("arr", lo, hi, elem_t)
            return elem_t
        if k == "file":
            et = t[1]
            if et == ("name", "ASCIIcode") or et == ("name", "char"):
                return ("file", "alpha")
            return ("file", "byte")
        if k == "ptr":
            return ("ptr", self.resolve_type(t[1]))
        if k == "record":
            if name is None:
                raise TranslateError("anonymous record")
            fields = []
            for names, ft in t[1]:
                rt = self.resolve_type(ft)
                for fn in names:
                    fields.append((fn, rt))
            self.records[name] = fields
            return ("rec", name)
        raise TranslateError(f"bad type {t}")

    # ------------------------------------------------------------------ rust types
    def rty(self, t):
        k = t[0]
        if k == "int":
            return t[1]
        if k == "bool":
            return "bool"
        if k == "real":
            return "f64"
        if k == "word":
            return t[1]
        if k == "rec":
            return "R" + t[1]
        if k == "ptr":
            return f"Ptr<{self.rty(t[1])}>"
        if k == "arr":
            return f"[{self.rty(t[3])}; {t[2] - t[1] + 1}]"
        if k == "file":
            return "FileRef"
        if k == "opaque":
            return t[1]
        raise TranslateError(f"no rust type for {t}")

    def zero(self, t):
        k = t[0]
        if k == "int":
            return "0"
        if k == "bool":
            return "false"
        if k == "real":
            return "0.0"
        return "Default::default()"

    # ------------------------------------------------------------------ names
    def rname(self, n):
        return n + "_" if n in RUST_KW else n

    def fresh(self, stem="t"):
        self.tmp += 1
        return f"__{stem}{self.tmp}"

    # ------------------------------------------------------------------ conversions
    def conv(self, ex, to):
        """Convert an Expr to Rust code of Pascal type `to` (C assignment conversion)."""
        fr = ex.ty
        if to[0] == "int":
            if fr[0] == "int":
                if fr[1] == to[1]:
                    return ex.code
                return f"({ex.code}) as {to[1]}"
            if fr[0] == "bool":
                return f"({ex.code}) as {to[1]}"
            if fr[0] == "real":
                return f"({ex.code}) as {to[1]}"
            if fr == STR:
                raise TranslateError(f"string to int: {ex.code}")
        if to[0] == "real":
            if fr[0] == "int":
                return f"({ex.code}) as f64"
            if fr[0] == "real":
                return ex.code
        if to[0] == "bool":
            if fr[0] == "bool":
                return ex.code
            if fr[0] == "int":
                return f"(({ex.code}) != 0)"
            if fr[0] == "ptr":
                return f"(!({ex.code}).is_null())"
            if fr[0] == "real":
                return f"(({ex.code}) != 0.0)"
        if to[0] == "ptr":
            if fr == NIL or (fr[0] == "int" and ex.code == "0"):
                return "Ptr::null()"
            if fr[0] == "ptr":
                if self.rty(fr) == self.rty(to):
                    return ex.code
                return f"({ex.code}).cast::<{self.rty(to[1])}>()"
            if fr == STR:
                return f"Ptr::<u8>::cstr_lit(b\"{self.bytes_lit(ex.code)}\\0\")"
        if to[0] in ("word", "rec", "file", "arr"):
            if self.rty(fr) == self.rty(to) if fr[0] not in ("str", "nil") else False:
                return ex.code
            if to[0] == "file" and fr == NIL:
                return "FileRef::NULL"
        raise TranslateError(f"cannot convert {fr} to {to}: {ex.code}")

    def bytes_lit(self, code):
        # code is b"..." ; return inner
        assert code.startswith('b"') and code.endswith('"'), code
        return code[2:-1]

    def as_i32(self, ex):
        if ex.ty == I32:
            return ex.code
        if ex.ty[0] == "int":
            return f"(({ex.code}) as i32)"
        if ex.ty[0] == "bool":
            return f"(({ex.code}) as i32)"
        raise TranslateError(f"not an integer: {ex.code} {ex.ty}")

    def promote(self, ex):
        """C integer promotion: narrow ints -> i32."""
        if ex.ty[0] == "int" and ex.ty[1] not in ("i32", "i64"):
            return Expr(f"({ex.code} as i32)", I32, ex.calls)
        return ex

    # ------------------------------------------------------------------ expressions
    def ex(self, e):
        k = e[0]
        if k == "num":
            v = e[1]
            if -2**31 <= v < 2**31:
                return Expr(str(v), I32)
            return Expr(f"{v}i64", I64)
        if k == "real":
            s = e[1]
            if "." not in s and "e" not in s.lower():
                s += ".0"
            return Expr(f"{s}f64", REAL)
        if k == "str":
            s = e[1]
            if len(s) == 1:
                return Expr(f"{ord(s)}", I32)
            return Expr('b"' + self.escape(s) + '"', STR)
        if k == "nil":
            return Expr("Ptr::null()", NIL)
        if k == "un":
            a = self.ex(e[2])
            if e[1] == "not":
                return Expr(f"(!{self.conv(a, BOOL)})", BOOL, a.calls)
            # web2c emits unary minus as `- (integer) x`: reals and 64-bit values are
            # truncated to C int before negation.
            if a.ty in (REAL, I64):
                return Expr(f"(-(({a.code}) as i32))", I32, a.calls)
            if a.ty == BOOL:
                return Expr(f"(-(({a.code}) as i32))", I32, a.calls)
            a = self.promote(a)
            return Expr(f"(-{a.code})", a.ty, a.calls)
        if k == "bin":
            return self.binop(e)
        if k == "id":
            return self.ex_id(e)
        if k == "call":
            return self.ex_call(e[1], e[2])
        if k in ("index", "field"):
            return self.read_lv(self.lv(e))
        raise TranslateError(f"cannot emit expression {e}")

    def escape(self, s):
        out = []
        for ch in s:
            o = ord(ch)
            if ch == '"':
                out.append('\\"')
            elif ch == "\\":
                out.append("\\\\")
            elif 32 <= o < 127:
                out.append(ch)
            else:
                out.append(f"\\x{o:02x}")
        return "".join(out)

    def binop(self, e):
        op = e[1]
        a, b = self.ex(e[2]), self.ex(e[3])
        calls = a.calls or b.calls
        if op in ("and", "or"):
            a_c = self.conv(a, BOOL)
            b_c = self.conv(b, BOOL)
            return Expr(f"({a_c} {'&&' if op == 'and' else '||'} {b_c})", BOOL, calls)
        if op in ("=", "<>", "<", "<=", ">", ">="):
            rop = {"=": "==", "<>": "!=", "<": "<", "<=": "<=", ">": ">", ">=": ">="}[op]
            if a.ty == BOOL and b.ty == BOOL:
                return Expr(f"({a.code} {rop} {b.code})", BOOL, calls)
            if a.ty[0] in ("ptr", "nil") or b.ty[0] in ("ptr", "nil"):
                ac = a.code if a.ty != NIL else "Ptr::null()"
                bc = b.code if b.ty != NIL else "Ptr::null()"
                if a.ty == NIL and b.ty[0] == "ptr":
                    ac = f"Ptr::<{self.rty(b.ty[1])}>::null()"
                if b.ty == NIL and a.ty[0] == "ptr":
                    bc = f"Ptr::<{self.rty(a.ty[1])}>::null()"
                return Expr(f"({ac}.addr() {rop} {bc}.addr())", BOOL, calls)
            if a.ty[0] == "file" or b.ty[0] == "file":
                return Expr(f"({a.code} {rop} {b.code})", BOOL, calls)
            ac, bc, t = self.arith_pair(a, b)
            return Expr(f"({ac} {rop} {bc})", BOOL, calls)
        if op == "/":
            ac = a.code if a.ty == REAL else f"({self.promote(a).code} as f64)"
            bc = b.code if b.ty == REAL else f"({self.promote(b).code} as f64)"
            return Expr(f"({ac} / {bc})", REAL, calls)
        if op in ("+", "-") and (a.ty[0] == "ptr" or b.ty[0] == "ptr"):
            if a.ty[0] == "ptr" and b.ty[0] == "int":
                off = self.as_i32(b) if b.ty != I64 else f"({b.code} as i32)"
                sign = "" if op == "+" else "-"
                return Expr(f"{a.code}.off({sign}{off})", a.ty, calls)
            raise TranslateError(f"pointer arithmetic {e}")
        rop = {"+": "+", "-": "-", "*": "*", "div": "/", "mod": "%"}[op]
        ac, bc, t = self.arith_pair(a, b)
        if t == REAL and op in ("div", "mod"):
            raise TranslateError(f"div/mod on reals {e}")
        return Expr(f"({ac} {rop} {bc})", t, calls)

    def arith_pair(self, a, b):
        if a.ty == BOOL:
            a = Expr(f"(({a.code}) as i32)", I32, a.calls)
        if b.ty == BOOL:
            b = Expr(f"(({b.code}) as i32)", I32, b.calls)
        if a.ty == REAL or b.ty == REAL:
            ac = a.code if a.ty == REAL else f"({self.promote(a).code} as f64)"
            bc = b.code if b.ty == REAL else f"({self.promote(b).code} as f64)"
            return ac, bc, REAL
        if a.ty[0] != "int" or b.ty[0] != "int":
            raise TranslateError(f"arith on {a.ty} {b.ty}: {a.code} / {b.code}")
        if a.ty == I64 or b.ty == I64:
            ac = a.code if a.ty == I64 else f"({self.as_i32(a)} as i64)"
            bc = b.code if b.ty == I64 else f"({self.as_i32(b)} as i64)"
            return ac, bc, I64
        return self.as_i32(a), self.as_i32(b), I32

    def ex_id(self, e):
        n = ALIASES.get(e[1], e[1])
        if n in self.locals:
            t, byref = self.locals[n]
            place = f"(*{self.rname(n)})" if byref else self.rname(n)
            return self.read_place(place, t)
        if self.cur is not None and n == self.cur["name"] and self.cur["kind"] == "function" and not self.routines[n]["params"]:
            return self.ex_call(n, [])
        if n in self.consts:
            v = self.consts[n]
            return self.const_expr(v)
        if n in self.globals:
            return self.read_place(f"self.{self.rname(n)}", self.globals[n])
        if n in self.routines:
            return self.ex_call(n, [])
        if n in ("true", "false"):
            return Expr(n, BOOL)
        r = externs.extern_expr(self, n, None)
        if r is not None:
            return r
        raise TranslateError(f"unknown identifier {n}")

    def const_expr(self, v):
        if isinstance(v, bool):
            return Expr("true" if v else "false", BOOL)
        if isinstance(v, int):
            return self.ex(("num", v))
        if isinstance(v, float):
            return Expr(f"{v!r}f64", REAL)
        if isinstance(v, str):
            return self.ex(("str", v))
        raise TranslateError(f"bad const {v}")

    def read_place(self, place, t):
        if t[0] == "word":
            return Expr(place, t)
        return Expr(place, t)

    # -- calls
    def ex_call(self, name, args, stmt=False):
        if name in self.routines:
            r = self.routines[name]
            params = r["params"]
            if len(params) != len(args):
                raise TranslateError(f"arity {name}: {len(args)} vs {len(params)}")
            codes = []
            calls = False
            for (pn, pt, byref), a in zip(params, args):
                ptype = self.resolve_type(pt)
                if byref:
                    lv = self.lv(a)
                    codes.append(self.addr_of(lv))
                    calls = True  # a raw pointer to a field of self must be taken before the call
                else:
                    ea = self.ex(a)
                    codes.append(self.conv(ea, ptype))
                    calls = calls or ea.calls
            code = self.call_with_temps(f"self.{self.rname(name)}", codes, calls)
            if r["kind"] == "function":
                return Expr(code, self.resolve_type(r["ret"]), True)
            if not stmt:
                raise TranslateError(f"procedure {name} used as value")
            return Expr(code, UNIT, True)
        r = externs.extern_expr(self, name, args, stmt=stmt)
        if r is not None:
            return r
        raise TranslateError(f"unknown routine {name}")

    def call_with_temps(self, fn, codes, any_calls):
        """self.f(args): bind arguments to temporaries when one of them calls back into self."""
        if not any_calls:
            return f"{fn}({', '.join(codes)})"
        lets, names = [], []
        for c in codes:
            t = self.fresh("a")
            lets.append(f"let {t} = {c};")
            names.append(t)
        return "{ " + " ".join(lets) + f" {fn}({', '.join(names)}) }}"

    def addr_of(self, lv):
        if lv.kind != "place":
            raise TranslateError(f"address of word field {lv.place} {lv.path}")
        return f"(&raw mut {lv.place})"

    # ------------------------------------------------------------------ lvalues
    def lv(self, e):
        k = e[0]
        if k == "id":
            n = ALIASES.get(e[1], e[1])
            if n in self.locals:
                t, byref = self.locals[n]
                place = f"(*{self.rname(n)})" if byref else self.rname(n)
                return self.mk_lv(place, t)
            if self.cur is not None and n == self.cur["name"] and self.cur["kind"] == "function":
                return self.mk_lv("__result", self.resolve_type(self.cur["ret"]))
            if n in self.globals:
                return self.mk_lv(f"self.{self.rname(n)}", self.globals[n])
            raise TranslateError(f"not an lvalue: {n}")
        if k == "index":
            base = self.lv(e[1])
            if base.kind != "place":
                raise TranslateError(f"index into word field {e}")
            t = base.ty
            place, calls = base.place, base.calls
            idxs = list(e[2])
            first = True
            while idxs:
                ie = self.ex(idxs.pop(0))
                calls = calls or ie.calls
                icode = self.as_i32(ie) if ie.ty != I64 else f"(({ie.code}) as i32)"
                if (t[0] == "ptr") or (t[0] == "arr" and first and self.is_storage_ptr(base)):
                    elem = t[1] if t[0] == "ptr" else t[3]
                    if ie.calls:
                        tv = self.fresh("i")
                        place = f"(*{{ let {tv} = {icode}; ix({place}, {tv}) }})"
                    else:
                        place = f"(*ix({place}, {icode}))"
                    t = elem
                elif t[0] == "arr":
                    lo = t[1]
                    off = f"({icode} - {lo})" if lo != 0 else icode
                    place = f"{place}[({off}) as usize]"
                    t = t[3]
                else:
                    raise TranslateError(f"index into non-array {e} ({t})")
                first = False
            return self.mk_lv(place, t, calls)
        if k == "field":
            f = FIELD_ALIASES.get(e[2], e[2])
            base = self.lv(e[1])
            if base.kind == "word":
                return LV("word", base.place, None, base.wname, base.path + (f,), base.calls)
            if base.ty[0] == "rec":
                for fn, ft in self.records[base.ty[1]]:
                    if fn == f:
                        return self.mk_lv(f"{base.place}.{self.rname(fn)}", ft, base.calls)
                raise TranslateError(f"no field {f} in {base.ty}")
            raise TranslateError(f"field of non-record {e}")
        if k == "call" and not e[2]:
            return self.lv(("id", e[1]))
        raise TranslateError(f"not an lvalue {e}")

    def is_storage_ptr(self, base):
        # global static arrays are stored as Ptr<T> fields; local arrays as Rust arrays
        return base.place.startswith("self.") and "." not in base.place[5:] and "[" not in base.place

    def mk_lv(self, place, t, calls=False):
        if t[0] == "word":
            return LV("word", place, t, t[1], (), calls)
        return LV("place", place, t, calls=calls)

    def word_field(self, wname, path):
        key = ".".join(path)
        fields = WORD_FIELDS[wname]
        if key not in fields:
            raise TranslateError(f"no field {key} in {wname}")
        return fields[key]

    def read_lv(self, lv):
        if lv.kind == "word":
            if not lv.path:
                return Expr(lv.place, ("word", lv.wname), lv.calls)
            stem, t = self.word_field(lv.wname, lv.path)
            return Expr(f"{lv.place}.{stem}()", t, lv.calls)
        if lv.ty[0] == "arr" and self.is_storage_ptr(lv):
            # a whole global array used as a value: it is a Ptr
            return Expr(lv.place, ("ptr", lv.ty[3]), lv.calls)
        return Expr(lv.place, lv.ty, lv.calls)

    def lv_type(self, lv):
        if lv.kind == "word":
            if not lv.path:
                return ("word", lv.wname)
            return self.word_field(lv.wname, lv.path)[1]
        if lv.ty[0] == "arr" and self.is_storage_ptr(lv):
            return ("ptr", lv.ty[3])
        return lv.ty

    def assign(self, lv, ex):
        t = self.lv_type(lv)
        val = self.conv(ex, t)
        if lv.kind == "word" and lv.path:
            stem, _ = self.word_field(lv.wname, lv.path)
            if ex.calls or lv.calls:
                tv = self.fresh("v")
                return f"{{ let {tv} = {val}; {lv.place}.set_{stem}({tv}); }}"
            return f"{lv.place}.set_{stem}({val});"
        if t[0] == "rec" and t[1] in externs.FIELDWISE_RECORDS:
            # copied field by field (see externs.FIELDWISE_RECORDS)
            return f"crate::rt::fast::copy_{t[1]}(&mut {lv.place}, &{val});"
        hook = externs.WRITE_HOOKS.get(lv.place)
        if hook:
            # a C global that the C side also keeps elsewhere (e.g. inside kpathsea)
            return f"{{ {lv.place} = {val}; self.{hook}(); }}"
        return f"{lv.place} = {val};"

    # ------------------------------------------------------------------ statements
    def stmt(self, s, out, ind):
        k = s[0]
        p = "    " * ind
        if k == "empty":
            return
        if k == "assign":
            lhs = s[1]
            # function result assignment
            if lhs[0] == "id" and self.cur is not None and lhs[1] == self.cur["name"] and lhs[1] not in self.locals:
                ex = self.ex(s[2])
                out.append(p + f"__result = {self.conv(ex, self.resolve_type(self.cur['ret']))};")
                return
            lv = self.lv(lhs)
            ex = self.ex(s[2])
            out.append(p + self.assign(lv, ex))
            return
        if k == "pcall":
            name, args = s[1], s[2]
            if name in self.routines:
                code = self.ex_call(name, args, stmt=True).code
                out.append(p + code + ";")
                return
            code = externs.extern_stmt(self, name, args)
            if code is None:
                raise TranslateError(f"unknown procedure {name}")
            for line in code.split("\n"):
                out.append(p + line)
            return
        if k == "block":
            self.seq(s[1], out, ind)
            return
        if k == "if":
            c = self.ex(s[1])
            out.append(p + f"if {self.conv(c, BOOL)} {{")
            self.seq([s[2]], out, ind + 1)
            if s[3] is not None and s[3][0] != "empty":
                out.append(p + "} else {")
                self.seq([s[3]], out, ind + 1)
            out.append(p + "}")
            return
        if k == "case":
            e = self.ex(s[1])
            if e.ty[0] != "int":
                raise TranslateError(f"case on {e.ty}")
            out.append(p + f"match {self.as_i32(e)} {{")
            seen = set()
            for labels, st in s[2]:
                vals = []
                for l in labels:
                    v = self.case_label(l)
                    if v in seen:
                        continue
                    seen.add(v)
                    vals.append(str(v))
                if not vals:
                    continue
                out.append(p + "    " + " | ".join(vals) + " => {")
                self.seq([st], out, ind + 2)
                out.append(p + "    }")
            out.append(p + "    _ => {")
            if s[3] is not None:
                self.seq([s[3]], out, ind + 2)
            out.append(p + "    }")
            out.append(p + "}")
            return
        if k == "while":
            c = self.ex(s[1])
            out.append(p + f"while {self.conv(c, BOOL)} {{")
            self.seq([s[2]], out, ind + 1)
            out.append(p + "}")
            return
        if k == "repeat":
            out.append(p + "loop {")
            self.seq(s[1], out, ind + 1)
            c = self.ex(s[2])
            out.append(p + f"    if {self.conv(c, BOOL)} {{ break; }}")
            out.append(p + "}")
            return
        if k == "for":
            self.for_stmt(s, out, ind)
            return
        if k == "goto":
            out.append(p + self.goto(s[1]))
            return
        if k == "casex":
            self.casex_stmt(s, out, ind)
            return
        if k == "label":
            self.seq([s], out, ind)
            return
        raise TranslateError(f"cannot emit statement {s}")

    # -- gotos into case arms (legal in web2c's C output, not in Pascal)
    def count_gotos(self, node, L):
        if isinstance(node, tuple):
            if node and node[0] == "goto":
                return 1 if node[1] == L else 0
            return sum(self.count_gotos(x, L) for x in node[1:] if isinstance(x, (tuple, list)))
        if isinstance(node, list):
            return sum(self.count_gotos(x, L) for x in node)
        return 0

    def rewrite_entries(self, node, total):
        if isinstance(node, list):
            return [self.rewrite_entries(x, total) for x in node]
        if not isinstance(node, tuple) or not node or not isinstance(node[0], str):
            return node
        if node[0] == "case":
            e, arms, others = node[1], node[2], node[3]
            new_arms, entries = [], []
            for labels, st in arms:
                st = self.rewrite_entries(st, total)
                entry = None
                if st[0] == "block":
                    items = st[1]
                    for idx, it in enumerate(items):
                        ls, x = [], it
                        while x[0] == "label":
                            ls.append(x[1])
                            x = x[2]
                        outside = [L for L in ls if not self.is_exit_label(L)
                                   and total.get(L, 0) > self.count_gotos(st, L)]
                        if outside:
                            if len(outside) != len([L for L in ls if not self.is_exit_label(L)]):
                                raise TranslateError(f"mixed case-entry labels {ls}")
                            entry = len(entries) + 1
                            entries.append((outside, [x] + list(items[idx + 1:])))
                            st = ("block", list(items[:idx]))
                            break
                new_arms.append((labels, st, entry))
            others2 = self.rewrite_entries(others, total) if others is not None else None
            if not entries:
                return ("case", e, [(l, st) for l, st, _ in new_arms], others2)
            out = ("casex", e, new_arms, others2, entries)
            for ls, _ in entries:
                for L in ls:
                    self.entry_labels.add(L)
                    out = ("label", L, out)
            return out
        return tuple(self.rewrite_entries(x, total) if isinstance(x, (tuple, list)) else x for x in node)

    def all_gotos(self, node, acc):
        if isinstance(node, tuple):
            if node and node[0] == "goto":
                acc[node[1]] = acc.get(node[1], 0) + 1
            else:
                for x in node[1:]:
                    if isinstance(x, (tuple, list)):
                        self.all_gotos(x, acc)
        elif isinstance(node, list):
            for x in node:
                self.all_gotos(x, acc)
        return acc

    def casex_stmt(self, s, out, ind):
        _, e, arms, others, entries = s
        p = "    " * ind
        uid = self.fresh("x")[2:]
        cend = f"cend{uid}"
        blk = {j: f"e{uid}_{j}" for j in range(1, len(entries) + 1)}
        ex = self.ex(e)
        out.append(p + f"'{cend}: {{")
        for j in range(len(entries), 0, -1):
            out.append(p + "    " * (len(entries) - j + 1) + f"'{blk[j]}: {{")
        q = p + "    " * (len(entries) + 1)
        for j, (ls, _) in enumerate(entries, start=1):
            for L in ls:
                out.append(q + f"if __entry == {L} {{ __entry = 0; break '{blk[j]}; }}")
        ctx = {"labels": {}, "caseentry": True, "blk": blk}
        for j, (ls, _) in enumerate(entries, start=1):
            for L in ls:
                ctx["labels"][L] = j
        self.label_ctx.append(ctx)
        out.append(q + f"match {self.as_i32(ex)} {{")
        seen = set()
        ind2 = ind + len(entries) + 1
        for labels, st, entry in arms:
            vals = []
            for l in labels:
                v = self.case_label(l)
                if v not in seen:
                    seen.add(v)
                    vals.append(str(v))
            if not vals:
                continue
            out.append(q + "    " + " | ".join(vals) + " => {")
            self.seq([st], out, ind2 + 2)
            if entry:
                out.append(q + f"        break '{blk[entry]};")
            out.append(q + "    }")
        out.append(q + "    _ => {")
        if others is not None:
            self.seq([others], out, ind2 + 2)
        out.append(q + "    }")
        out.append(q + "}")
        out.append(q + f"break '{cend};")
        self.label_ctx.pop()
        for j, (ls, tail) in enumerate(entries, start=1):
            depth = ind + len(entries) - j + 1
            out.append("    " * depth + "}")
            self.seq(tail, out, depth)
            if j < len(entries):
                out.append("    " * depth + f"break '{cend};")
        out.append(p + "}")

    def case_label(self, l):
        if l[0] == "str" and len(l[1]) == 1:
            return ord(l[1])
        v = self.const_eval(l)
        if not isinstance(v, int):
            raise TranslateError(f"case label {l}")
        return v

    def for_stmt(self, s, out, ind):
        _, var, a, d, b, body = s
        p = "    " * ind
        lv = self.lv(("id", var))
        t = self.lv_type(lv)
        if t[0] != "int":
            raise TranslateError(f"for over {t}")
        ea, eb = self.ex(a), self.ex(b)
        endv = self.fresh("end")
        wide = "i64" if t == I64 else "i32"
        out.append(p + "{")
        out.append(p + f"    let {endv}: {wide} = {self.as_i32(eb) if wide == 'i32' else self.conv(eb, I64)};")
        out.append(p + "    " + self.assign(lv, ea))
        cur = f"({lv.place} as {wide})"
        rel, step = ("<=", "+") if d == "to" else (">=", "-")
        out.append(p + f"    if {cur} {rel} {endv} {{")
        out.append(p + "        loop {")
        self.seq([body], out, ind + 3)
        cont = self.fresh("c")
        lt = "<" if d == "to" else ">"
        out.append(p + f"            let {cont} = {cur} {lt} {endv};")
        out.append(p + f"            {lv.place} = ({cur} {step} 1) as {t[1]};")
        out.append(p + f"            if !{cont} {{ break; }}")
        out.append(p + "        }")
        out.append(p + "    }")
        out.append(p + "}")

    # -- goto machinery
    def gotos_in(self, s):
        key = id(s)
        if key in self._gcache:
            return self._gcache[key]
        res = set()
        if isinstance(s, tuple):
            if s and s[0] == "goto":
                res.add(s[1])
            else:
                for x in s[1:]:
                    if isinstance(x, (tuple, list)):
                        res |= self.gotos_in(x)
        elif isinstance(s, list):
            for x in s:
                res |= self.gotos_in(x)
        self._gcache[key] = res
        return res

    def is_exit_label(self, n):
        """web2c's doreturn(): label 10 means `return` except in these four routines."""
        return n == 10 and self.cur["name"] not in ("macrocall", "hpack", "vpackage", "trybreak")

    def goto(self, n):
        if self.is_exit_label(n):
            return "return __result;" if self.cur["kind"] == "function" else "return;"
        for ctx in reversed(self.label_ctx):
            if n in ctx["labels"]:
                if ctx.get("caseentry"):
                    return f"break '{ctx['blk'][ctx['labels'][n]]};"
                if n in self.entry_labels:
                    return "{ __entry = %d; %s }" % (n, self._goto_in(ctx, n))
                return self._goto_in(ctx, n)
        raise TranslateError(f"goto {n}: label not in an enclosing statement sequence")

    def _goto_in(self, ctx, n):
        if True:
            if True:
                if ctx.get("exit"):
                    return "return __result;" if self.cur["kind"] == "function" else "return;"
                j = ctx["labels"][n]
                if ctx["cur_seg"] < j:
                    return f"break '{ctx['blk'][j]};"
                return f"{{ {ctx['sv']} = {j}; continue '{ctx['loop']}; }}"

    def seq(self, stmts, out, ind, top=False):
        # flatten labels: item = (labels, stmt)
        items = []
        for s in stmts:
            labels = []
            while s[0] == "label":
                if not self.is_exit_label(s[1]):
                    labels.append(s[1])
                s = s[2]
            items.append((labels, s))
        if not any(l for l, _ in items):
            for _, s in items:
                self.stmt(s, out, ind)
            return
        p = "    " * ind
        # exit label at the very end of a routine body
        if top and items[-1][0] and items[-1][1][0] == "empty":
            ctx = {"labels": {l: None for l in items[-1][0]}, "exit": True, "cur_seg": 0}
            self.label_ctx.append(ctx)
            last = items.pop()
            if any(l for l, _ in items):
                self.seq_labeled(items, out, ind)
            else:
                for _, s in items:
                    self.stmt(s, out, ind)
            self.label_ctx.pop()
            return
        self.seq_labeled(items, out, ind)

    def seq_labeled(self, items, out, ind):
        p = "    " * ind
        # segment boundaries: indices of items with labels (that are targeted)
        starts = []
        for idx, (labels, s) in enumerate(items):
            if labels:
                starts.append(idx)
        seg_of_label = {}
        nseg = len(starts) + 1
        for j, idx in enumerate(starts, start=1):
            for l in items[idx][0]:
                seg_of_label[l] = j
        # which labels have backward gotos (goto inside segment >= label's segment)
        seg_items = []
        bounds_ = [0] + starts + [len(items)]
        for j in range(nseg):
            seg_items.append(items[bounds_[j]:bounds_[j + 1]])
        back = set()
        for l, j in seg_of_label.items():
            for jj in range(j, nseg):
                if any(l in self.gotos_in(s) for _, s in seg_items[jj]):
                    back.add(j)
        uid = self.fresh("s")[2:]
        ctx = {"labels": seg_of_label, "cur_seg": 0, "blk": {j: f"b{uid}_{j}" for j in range(1, nseg)},
               "sv": f"__st{uid}", "loop": f"l{uid}"}
        self.label_ctx.append(ctx)
        if back:
            out.append(p + f"let mut {ctx['sv']}: u32 = 0;")
            out.append(p + f"'{ctx['loop']}: loop {{")
            ind += 1
            p = "    " * ind
        # open nested blocks: outermost is segment nseg-1's block
        for j in range(nseg - 1, 0, -1):
            out.append(p + "    " * (nseg - 1 - j) + f"'{ctx['blk'][j]}: {{")
        inner = ind + nseg - 1
        if back:
            arms = " ".join(f"{j} => break '{ctx['blk'][j]}," for j in sorted(back))
            out.append("    " * inner + f"match {ctx['sv']} {{ {arms} _ => {{}} }}")
        for j in range(nseg):
            ctx["cur_seg"] = j
            depth = ind + (nseg - 1 - j)
            for _, s in seg_items[j]:
                self.stmt(s, out, depth)
            if j < nseg - 1:
                out.append("    " * (depth - 1) + "}")
        if back:
            out.append(p + f"break '{ctx['loop']};")
            out.append("    " * (ind - 1) + "}")
        self.label_ctx.pop()

    # ------------------------------------------------------------------ routines
    def routine(self, r):
        self.cur = r
        self.tmp = 0
        self._gcache = {}
        self.label_ctx = []
        self.locals = {}
        params = []
        for pn, pt, byref in r["params"]:
            t = self.resolve_type(pt)
            self.locals[pn] = (t, byref)
            if byref:
                params.append(f"{self.rname(pn)}: *mut {self.rty(t)}")
            else:
                params.append(f"mut {self.rname(pn)}: {self.rty(t)}")
        for c, e in r["consts"]:
            self.consts[c] = self.const_eval(e)
        out = []
        ret = ""
        if r["kind"] == "function":
            rt = self.resolve_type(r["ret"])
            ret = f" -> {self.rty(rt)}"
        if r.get("noreturn"):
            ret = " -> !"
        fname = self.rname(r['name']) + ("_full" if r['name'] in externs.FAST_PATHS else "")
        attr = (["    #[inline(never)]"] if r['name'] in externs.FAST_PATHS
                else ["    #[inline(always)]"] if r['name'] in externs.INLINE_ALWAYS else [])
        out.extend(attr)
        out.append(f"    pub fn {fname}(&mut self{''.join(', ' + x for x in params)}){ret} {{")
        self.prof_names.append(r["name"])
        out.append(f"        #[cfg(feature = \"prof\")] let __pg = crate::rt::prof::enter({len(self.prof_names) - 1});")
        for pn, pt, byref in r["params"]:
            if byref:
                out.append(f"        let {self.rname(pn)} = unsafe {{ &mut *{self.rname(pn)} }};")
        for vn, vt in r["vars"]:
            t = self.resolve_type(vt)
            self.locals[vn] = (t, False)
            out.append(f"        let mut {self.rname(vn)}: {self.rty(t)} = {self.zero_local(t)};")
        if r["kind"] == "function":
            rt = self.resolve_type(r["ret"])
            out.append(f"        let mut __result: {self.rty(rt)} = {self.zero(rt)};")
        self.entry_labels = set()
        body = self.rewrite_entries(r["body"], self.all_gotos(r["body"], {}))
        if self.entry_labels:
            out.append("        let mut __entry: i32 = 0;")
        self.seq(body[1], out, 2, top=True)
        if r["kind"] == "function":
            out.append("        __result")
        elif r.get("noreturn"):
            out.append("        unreachable!()")
        out.append("    }")
        self.cur = None
        return out

    def zero_local(self, t):
        if t[0] == "arr":
            return f"[{self.zero_local(t[3])}; {t[2] - t[1] + 1}]"
        return self.zero(t)

    # ------------------------------------------------------------------ module output
    def emit_globals(self):
        out = ["// Generated by port/tools/w2rs from pdftex.p. Do not edit.",
               "#![allow(non_snake_case, non_camel_case_types, unused_parens, dead_code)]",
               "use crate::rt::*;", ""]
        for name, fields in self.records.items():
            out.append("#[repr(C)]")
            out.append("#[derive(Clone, Copy, Default, Debug)]")
            out.append(f"pub struct R{name} {{")
            for fn, ft in fields:
                out.append(f"    pub {self.rname(fn)}: {self.rty(ft)},")
            out.append("}")
            out.append("")
        out.append("#[derive(Default)]")
        out.append("pub struct Globals {")
        for n, t in self.globals.items():
            if t[0] == "arr":
                out.append(f"    pub {self.rname(n)}: Ptr<{self.rty(t[3])}>,  // [{t[1]}..{t[2]}]")
            else:
                out.append(f"    pub {self.rname(n)}: {self.rty(t)},")
        out.append("    pub rt: Runtime,")
        out.append("}")
        out.append("")
        out.append("impl Globals {")
        out.append("    /// Allocate the statically sized Pascal arrays (web2c: C arrays with offset bases).")
        out.append("    pub fn alloc_static_arrays(&mut self) {")
        for n, t in self.globals.items():
            if t[0] == "arr":
                lo, hi = t[1], t[2]
                base = 0 if lo in (0, 1) else lo
                count = hi - base + 1
                out.append(f"        self.{self.rname(n)} = Ptr::alloc_count({count}).off({-base});")
        out.append("    }")
        out.append("}")
        return out

    def emit_routines(self, chunk=40):
        bodies = []
        failures = []
        for name, r in self.routines.items():
            if r["forward"]:
                continue
            try:
                bodies.append((name, self.routine(r)))
            except TranslateError as ex:
                failures.append((name, str(ex)))
                self.cur = None
        files = []
        for i in range(0, len(bodies), chunk):
            out = ["// Generated by port/tools/w2rs from pdftex.p. Do not edit.",
                   "#![allow(non_snake_case, unused_parens, unused_mut, unused_variables, unused_labels,",
                   "         unused_assignments, unreachable_code, unused_braces, while_true, dead_code)]",
                   "use crate::rt::*;", "use super::globals::*;", "", "impl Globals {"]
            for name, lines in bodies[i:i + chunk]:
                out.extend(lines)
                out.append("")
            out.append("}")
            files.append(out)
        return files, failures


def main():
    src, outdir = sys.argv[1], sys.argv[2]
    prog = pas.parse_file(src, {"INITEX", "STAT"})
    em = Emitter(prog)
    os.makedirs(outdir, exist_ok=True)
    files, failures = em.emit_routines()
    g = em.emit_globals()
    with open(os.path.join(outdir, "globals.rs"), "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(g) + "\n")
    with open(os.path.join(outdir, "prof_names.rs"), "w", encoding="utf-8", newline="\n") as f:
        f.write("// Generated by port/tools/w2rs. Do not edit.\npub const NAMES: &[&str] = &[\n")
        for n in em.prof_names:
            f.write(f"    \"{n}\",\n")
        f.write("];\n")
    mods = ["pub mod globals;", "pub mod prof_names;"]
    for i, lines in enumerate(files):
        with open(os.path.join(outdir, f"r{i:02}.rs"), "w", encoding="utf-8", newline="\n") as f:
            f.write("\n".join(lines) + "\n")
        mods.append(f"pub mod r{i:02};")
    with open(os.path.join(outdir, "mod.rs"), "w", encoding="utf-8", newline="\n") as f:
        f.write("// Generated by port/tools/w2rs. Do not edit.\n" + "\n".join(mods) + "\n")
    for name, err in failures:
        print(f"FAIL {name}: {err}")
    print(f"{len(files)} files, {len(failures)} failures, unknown externs: {sorted(em.uses_unknown)}")


if __name__ == "__main__":
    main()
