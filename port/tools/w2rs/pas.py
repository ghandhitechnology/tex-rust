"""Lexer, preprocessor and parser for the Pascal that TeX Live's tangle writes for pdfTeX.

The input is the subset of Pascal that web2c accepts (see texk/web2c/web2c/web2c-parser.y),
plus web2c's conditional-compilation pseudo calls ifdef('X') / ifndef('X') / endif('X').
The parser builds a plain AST (tuples tagged by node kind) that emit.py turns into Rust.
"""
import re

# --------------------------------------------------------------------------- lexer

KEYWORDS = {
    "and", "array", "begin", "case", "const", "div", "do", "downto", "else", "end", "file",
    "for", "forward", "function", "goto", "if", "in", "label", "mod", "nil", "not", "of", "or",
    "others", "packed", "procedure", "program", "record", "repeat", "then", "to", "type",
    "until", "var", "while", "with",
}

TOKEN_RE = re.compile(
    r"""
    (?P<ws>\s+)
  | (?P<comment>\{[^}]*\})
  | (?P<include>\#include[^\n;]*;?)
  | (?P<real>\d+\.\d+(?:[eE][-+]?\d+)?|\d+[eE][-+]?\d+)
  | (?P<int>\d+)
  | (?P<str>'(?:[^']|'')*')
  | (?P<id>[A-Za-z_][A-Za-z0-9_]*)
  | (?P<op>:=|<>|<=|>=|\.\.|[-+*/=<>()\[\],;:.^@])
    """,
    re.X,
)


class Tok:
    __slots__ = ("kind", "val", "pos")

    def __init__(self, kind, val, pos):
        self.kind, self.val, self.pos = kind, val, pos

    def __repr__(self):
        return f"{self.kind}:{self.val!r}@{self.pos}"


def lex(text):
    out = []
    i, n = 0, len(text)
    while i < n:
        m = TOKEN_RE.match(text, i)
        if not m:
            raise SyntaxError(f"bad character {text[i]!r} at {i}: {text[i-40:i+40]!r}")
        kind = m.lastgroup
        s = m.group(kind)
        if kind == "id":
            low = s.lower()
            if low in KEYWORDS:
                out.append(Tok("kw", low, i))
            else:
                out.append(Tok("id", s, i))
        elif kind == "int":
            out.append(Tok("int", int(s), i))
        elif kind == "real":
            out.append(Tok("real", s, i))
        elif kind == "str":
            out.append(Tok("str", s[1:-1].replace("''", "'"), i))
        elif kind == "op":
            out.append(Tok("op", s, i))
        # ws, comment, include: dropped
        i = m.end()
    out.append(Tok("eof", None, n))
    return out


def preprocess(toks, defined):
    """Resolve ifdef('X')/ifndef('X')/endif('X') against the set of defined symbols."""
    out = []
    stack = []  # (name, active)
    active = True
    i = 0
    while i < len(toks):
        t = toks[i]
        if t.kind == "id" and t.val in ("ifdef", "ifndef", "endif", "endifn") and toks[i + 1].val == "(":
            name = toks[i + 2].val
            assert toks[i + 2].kind == "str" and toks[i + 3].val == ")", toks[i : i + 4]
            if t.val in ("endif", "endifn"):
                top, prev_active = stack.pop()
                assert top == name, (top, name)
                active = prev_active
            else:
                cond = (name in defined) == (t.val == "ifdef")
                stack.append((name, active))
                active = active and cond
            i += 4
            continue
        if active:
            out.append(t)
        i += 1
    assert not stack, stack
    return out


# --------------------------------------------------------------------------- AST
# Expressions:   ('num', int) ('real', str) ('str', str) ('nil',) ('id', name)
#                ('call', name, [args]) ('index', base, [idx]) ('field', base, name)
#                ('deref', base) ('un', op, e) ('bin', op, l, r) ('fmt', e, width)  [write args]
# Statements:    ('assign', lhs, rhs) ('pcall', name, [args]) ('block', [stmts])
#                ('if', c, then, else|None) ('case', e, [([labels], stmt)], others|None)
#                ('while', c, body) ('repeat', [stmts], c) ('for', var, a, dir, b, body)
#                ('goto', n) ('label', n, stmt) ('empty',)
# Types:         ('name', id) ('sub', lo_expr, hi_expr) ('array', [index_types], elem, packed)
#                ('record', [(names, type)]) ('ptr', type) ('file', elem)


class Parser:
    def __init__(self, toks):
        self.t = toks
        self.i = 0

    # -- helpers
    def peek(self, k=0):
        return self.t[self.i + k]

    def at(self, val, k=0):
        t = self.t[self.i + k]
        return t.val == val and t.kind in ("kw", "op")

    def take(self, val=None):
        t = self.t[self.i]
        if val is not None and not (t.val == val and t.kind in ("kw", "op")):
            ctx = " ".join(str(x.val) for x in self.t[max(0, self.i - 15) : self.i + 10])
            raise SyntaxError(f"expected {val!r}, got {t!r}; context: {ctx}")
        self.i += 1
        return t

    def take_id(self):
        t = self.t[self.i]
        if t.kind != "id":
            ctx = " ".join(str(x.val) for x in self.t[max(0, self.i - 15) : self.i + 10])
            raise SyntaxError(f"expected identifier, got {t!r}; context: {ctx}")
        self.i += 1
        return t.val

    def accept(self, val):
        if self.at(val):
            self.i += 1
            return True
        return False

    # -- program
    def program(self):
        self.take("program")
        name = self.take_id()
        self.take(";")
        decls = self.declarations(top=True)
        body = None
        if self.at("begin"):
            body = self.compound()
        self.accept(".")
        assert self.peek().kind == "eof", self.peek()
        return {"name": name, **decls, "body": body}

    def declarations(self, top=False):
        labels, consts, types, vars_, procs = [], [], [], [], []
        while True:
            if self.accept("label"):
                labels.append(self.peek().val)
                self.i += 1
                while self.accept(","):
                    labels.append(self.peek().val)
                    self.i += 1
                self.take(";")
            elif self.accept("const"):
                while self.peek().kind == "id" and self.at("=", 1):
                    name = self.take_id()
                    self.take("=")
                    consts.append((name, self.expr()))
                    self.take(";")
            elif self.accept("type"):
                while self.peek().kind == "id" and self.at("=", 1):
                    name = self.take_id()
                    self.take("=")
                    types.append((name, self.type_()))
                    self.take(";")
            elif self.accept("var"):
                while self.peek().kind == "id":
                    names = [self.take_id()]
                    while self.accept(","):
                        names.append(self.take_id())
                    self.take(":")
                    ty = self.type_()
                    self.take(";")
                    for n in names:
                        vars_.append((n, ty))
            elif self.at("procedure") or self.at("function"):
                procs.append(self.routine())
            elif self.peek().kind == "id" and self.peek().val == "noreturn" and self.at("procedure", 1):
                self.i += 1
                r = self.routine()
                r["noreturn"] = True
                procs.append(r)
            else:
                break
        return {"labels": labels, "consts": consts, "types": types, "vars": vars_, "procs": procs}

    def routine(self):
        kind = self.take().val
        name = self.take_id()
        params = []
        if self.accept("("):
            while True:
                byref = self.accept("var")
                names = [self.take_id()]
                while self.accept(","):
                    names.append(self.take_id())
                self.take(":")
                ty = self.type_()
                for n in names:
                    params.append((n, ty, byref))
                if not self.accept(";"):
                    break
            self.take(")")
        ret = None
        if kind == "function" and self.accept(":"):
            ret = self.type_()
        self.take(";")
        if self.accept("forward"):
            self.take(";")
            return {"kind": kind, "name": name, "params": params, "ret": ret, "forward": True}
        decls = self.declarations()
        assert not decls["procs"], f"nested routine in {name}"
        body = self.compound()
        self.take(";")
        return {"kind": kind, "name": name, "params": params, "ret": ret, "forward": False,
                "labels": decls["labels"], "consts": decls["consts"], "types": decls["types"],
                "vars": decls["vars"], "body": body}

    # -- types
    def type_(self):
        packed = self.accept("packed")
        if self.accept("array"):
            self.take("[")
            idx = [self.simple_type()]
            while self.accept(","):
                idx.append(self.simple_type())
            self.take("]")
            self.take("of")
            return ("array", idx, self.type_(), packed)
        if self.accept("file"):
            self.take("of")
            return ("file", self.type_())
        if self.accept("record"):
            fields = []
            while not self.at("end"):
                names = [self.take_id()]
                while self.accept(","):
                    names.append(self.take_id())
                self.take(":")
                fields.append((names, self.type_()))
                if not self.accept(";"):
                    break
            self.take("end")
            return ("record", fields)
        if self.accept("^"):
            return ("ptr", self.type_())
        return self.simple_type()

    def simple_type(self):
        # an identifier alone is a type name; otherwise a subrange lo..hi
        if self.peek().kind == "id" and not self.at("..", 1):
            return ("name", self.take_id())
        lo = self.simple_expr()
        self.take("..")
        hi = self.simple_expr()
        return ("sub", lo, hi)

    # -- statements
    def compound(self):
        self.take("begin")
        stmts = self.stmt_list(("end",))
        self.take("end")
        return ("block", stmts)

    def stmt_list(self, terminators):
        stmts = [self.statement()]
        while self.accept(";"):
            stmts.append(self.statement())
        if not any(self.at(t) for t in terminators):
            ctx = " ".join(str(x.val) for x in self.t[max(0, self.i - 20) : self.i + 10])
            raise SyntaxError(f"expected one of {terminators}, got {self.peek()!r}; context: {ctx}")
        return [s for s in stmts if s[0] != "empty" or False]

    def statement(self):
        t = self.peek()
        if t.kind == "int" and self.at(":", 1):
            self.i += 2
            return ("label", t.val, self.statement())
        if t.kind == "kw":
            v = t.val
            if v == "begin":
                return self.compound()
            if v == "if":
                self.i += 1
                c = self.expr()
                self.take("then")
                th = self.statement()
                el = None
                if self.accept("else"):
                    el = self.statement()
                return ("if", c, th, el)
            if v == "case":
                self.i += 1
                e = self.expr()
                self.take("of")
                arms, others = [], None
                while not self.at("end"):
                    if self.accept("others"):
                        self.take(":")
                        others = ("block", self.stmt_list(("end",)))
                        break
                    labels = [self.expr()]
                    while self.accept(","):
                        labels.append(self.expr())
                    self.take(":")
                    arms.append((labels, self.statement()))
                    if not self.accept(";"):
                        break
                self.take("end")
                return ("case", e, arms, others)
            if v == "while":
                self.i += 1
                c = self.expr()
                self.take("do")
                return ("while", c, self.statement())
            if v == "repeat":
                self.i += 1
                body = self.stmt_list(("until",))
                self.take("until")
                return ("repeat", body, self.expr())
            if v == "for":
                self.i += 1
                var = self.take_id()
                self.take(":=")
                a = self.expr()
                d = self.take().val
                assert d in ("to", "downto"), d
                b = self.expr()
                self.take("do")
                return ("for", var, a, d, b, self.statement())
            if v == "goto":
                self.i += 1
                n = self.take()
                return ("goto", n.val)
            if v in ("end", "until", "else"):
                return ("empty",)
            raise SyntaxError(f"unexpected keyword {t!r}")
        if t.kind == "op" and t.val == ";":
            return ("empty",)
        if t.kind == "id":
            lhs = self.designator()
            if self.accept(":="):
                return ("assign", lhs, self.expr())
            if lhs[0] == "id":
                return ("pcall", lhs[1], [])
            if lhs[0] == "call":
                return ("pcall", lhs[1], lhs[2])
            raise SyntaxError(f"bad statement {lhs!r}")
        raise SyntaxError(f"unexpected token {t!r}")

    # -- expressions
    def designator(self):
        name = self.take_id()
        if self.at("("):
            self.take("(")
            args = self.args()
            self.take(")")
            node = ("call", name, args)
        else:
            node = ("id", name)
        while True:
            if self.accept("["):
                idx = [self.expr()]
                while self.accept(","):
                    idx.append(self.expr())
                self.take("]")
                node = ("index", node, idx)
            elif self.at(".") and self.peek(1).kind == "id":
                self.take(".")
                node = ("field", node, self.take_id())
            elif self.accept("^"):
                node = ("deref", node)
            else:
                return node

    def args(self):
        out = [self.arg()]
        while self.accept(","):
            out.append(self.arg())
        return out

    def arg(self):
        e = self.expr()
        if self.accept(":"):  # write(f, x:width)
            e = ("fmt", e, self.expr())
        return e

    # web2c copies expression tokens straight into C (and -> &&, or -> ||, = -> ==, ...),
    # so the compiled pdfTeX evaluates them with C precedence, not Pascal's. We parse the
    # same way: || < && < ==,!= < relational < additive < multiplicative < unary.
    def _binary_level(self, ops, sub):
        left = sub()
        while True:
            t = self.peek()
            if t.kind in ("op", "kw") and t.val in ops:
                self.i += 1
                left = ("bin", t.val, left, sub())
            else:
                return left

    def expr(self):
        return self._binary_level(("or",), self.and_expr)

    def and_expr(self):
        return self._binary_level(("and",), self.eq_expr)

    def eq_expr(self):
        return self._binary_level(("=", "<>"), self.rel_expr)

    def rel_expr(self):
        return self._binary_level(("<", "<=", ">", ">=", "in"), self.simple_expr)

    def simple_expr(self):
        return self._binary_level(("+", "-"), self.term)

    def term(self):
        return self._binary_level(("*", "/", "div", "mod"), self.unary)

    def unary(self):
        if self.at("not"):
            self.i += 1
            return ("un", "not", self.unary())
        if self.at("-"):
            self.i += 1
            return ("un", "-", self.unary())
        if self.at("+"):
            self.i += 1
            return self.unary()
        return self.factor()

    def factor(self):
        t = self.peek()
        if t.kind == "int":
            self.i += 1
            return ("num", t.val)
        if t.kind == "real":
            self.i += 1
            return ("real", t.val)
        if t.kind == "str":
            self.i += 1
            return ("str", t.val)
        if t.kind == "kw" and t.val == "nil":
            self.i += 1
            return ("nil",)
        if t.kind == "kw" and t.val == "not":
            self.i += 1
            return ("un", "not", self.factor())
        if t.kind == "op" and t.val == "(":
            self.i += 1
            e = self.expr()
            self.take(")")
            return e
        if t.kind == "op" and t.val == "-":  # lenient: x * -y
            self.i += 1
            return ("un", "-", self.factor())
        if t.kind == "op" and t.val == "+":
            self.i += 1
            return self.factor()
        if t.kind == "id":
            return self.designator()
        raise SyntaxError(f"unexpected token in expression {t!r}")


def parse_file(path, defined):
    text = open(path, encoding="latin-1").read()
    toks = preprocess(lex(text), defined)
    return Parser(toks).program()


if __name__ == "__main__":
    import sys, collections
    prog = parse_file(sys.argv[1], {"INITEX", "STAT"})
    print("consts", len(prog["consts"]), "types", len(prog["types"]), "vars", len(prog["vars"]),
          "procs", len(prog["procs"]))
    kinds = collections.Counter()

    def walk(n):
        if isinstance(n, tuple) and n and isinstance(n[0], str):
            kinds[n[0]] += 1
        if isinstance(n, (tuple, list)):
            for x in n:
                walk(x)
        elif isinstance(n, dict):
            for x in n.values():
                walk(x)

    walk(prog)
    print(kinds.most_common())
