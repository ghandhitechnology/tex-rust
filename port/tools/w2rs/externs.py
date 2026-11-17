"""web2c externals: how each identifier the Pascal uses but does not declare becomes Rust.

Semantics come from texk/web2c/cpascal.h, texmfmp.h, coerce.h and pdftexdir/ptexlib.h.
Most are plain calls to runtime methods on `Globals` (implemented by hand in the
engine's rt modules); the ones that take a Pascal lvalue (file opens, dump/undump,
date/time) get custom code so no Rust borrow of `self` is held across the call.
"""

I32 = ("int", "i32")
I64 = ("int", "i64")
U8 = ("int", "u8")
BOOL = ("bool",)
REAL = ("real",)
STR = ("str",)
CSTR = ("ptr", ("int", "u8"))
UNIT = ("unit",)
FILE = ("file", None)

# kpathsea's kpse_file_format_type numbering (kpathsea/types.h)
KPSE_FORMATS = [
    "gf", "pk", "any_glyph", "tfm", "afm", "base", "bib", "bst", "cnf", "db", "fmt", "fontmap",
    "mem", "mf", "mfpool", "mft", "mp", "mppool", "mpsupport", "ocp", "ofm", "opl", "otp", "ovf",
    "ovp", "pict", "tex", "texdoc", "texpool", "texsource", "tex_ps_header", "troff_font",
    "type1", "vf", "dvips_config", "ist", "truetype", "type42", "web2c", "program_text",
    "program_binary", "miscfonts", "web", "cweb", "enc_files", "cmap_files", "sfd_files",
    "opentype", "pdftex_config", "lig_files", "texmfscripts", "lua", "fea", "cid", "mlbib",
    "mlbst", "clua", "ris", "bltxml", "last",
]

EXTERN_CONSTS = {
    "TEXMFPOOLNAME": "pdftex.pool",
    "TEXMFENGINENAME": "pdftex",
    "maxint": 2147483647,
    "kpsesrccompile": 1,  # kpse_src_compile in kpathsea/types.h (enum kpse_src_type)
    "kpsesrccmdline": 6,
}
for i, n in enumerate(KPSE_FORMATS):
    EXTERN_CONSTS["kpse" + n.replace("_", "") + "format"] = i

EXTERN_VARS = {
    "mem": ("ptr", ("word", "MemoryWord")),
    "eqtb": ("ptr", ("word", "MemoryWord")),
    "tfmtemp": I32,
    "texinputtype": I32,
    "translatefilename": CSTR,
    "versionstring": CSTR,
    "kpsemaketexdiscarderrors": I32,
}

# Pascal assignments to these C globals also call a runtime method that hands the new value
# to the Rust component that owns the C variable (kpathsea's make_tex_discard_errors).
WRITE_HOOKS = {
    "self.kpsemaketexdiscarderrors": "sync_kpse_discard_errors",
}

# Routines whose generated body is emitted as `<name>_full`: a hand-written `<name>` in
# rt/fast.rs handles the hot common case with exactly the same effects and calls
# `<name>_full` for everything else (hot/cold splitting of the inner loop).
FAST_PATHS = {"getnext", "macrocall", "idlookup", "endtokenlist", "dividescaled", "passtext", "effectivechar", "advcharwidth", "effectivecharinfo", "readbuffer"}

# Records whose assignments are emitted as field-by-field copies (rt/fast.rs copy_<name>).
# A whole-struct copy becomes 16- and 8-byte loads; when the fields were just stored
# separately (cur_input before push_input) the CPU cannot forward those stores into the
# wider loads and stalls until they retire.
FIELDWISE_RECORDS: set = set()  # tried (volatile field copies, and narrow-read/8-byte-write copies): slower

# Small routines on the inner loop that are emitted with #[inline(always)].
INLINE_ALWAYS: set = {"gettoken"}  # tried endtokenlist, flushlist, deletetokenref, getavail: no measurable gain

# functions: name -> (return type, [param types]); None param = pass natural type
FUNCS = {
    "getc": (I32, [FILE]),
    "eof": (BOOL, [FILE]),
    "feof": (BOOL, [FILE]),
    "extxnoverd": (I32, [I32, I32, I32]),
    "packetbyte": (U8, []),
    "inputln": (BOOL, [FILE, BOOL]),
    "getefcode": (I32, [I32, I32]), "getlpcode": (I32, [I32, I32]), "getrpcode": (I32, [I32, I32]),
    "getknbscode": (I32, [I32, I32]), "getstbscode": (I32, [I32, I32]), "getshbscode": (I32, [I32, I32]),
    "getknbccode": (I32, [I32, I32]), "getknaccode": (I32, [I32, I32]),
    "imagewidth": (I32, [I32]), "imageheight": (I32, [I32]), "imagerotate": (I32, [I32]),
    "imagexres": (I32, [I32]), "imageyres": (I32, [I32]), "imagepages": (I32, [I32]),
    "imagecolordepth": (I32, [I32]), "getimagegroupref": (I32, [I32]),
    "ispdfimage": (BOOL, [I32]), "ispngimage": (BOOL, [I32]),
    "epdforigx": (I32, [I32]), "epdforigy": (I32, [I32]),
    "matrixused": (BOOL, []),
    "isscalable": (BOOL, [I32]),
    "colorstackused": (I32, []),
    "colorstackcurrent": (I32, [I32]), "colorstackset": (I32, [I32, I32]),
    "colorstackpush": (I32, [I32, I32]), "colorstackpop": (I32, [I32]),
    "colorstackskippagestart": (I32, [I32]), "newcolorstack": (I32, [I32, I32, BOOL]),
    "getjobname": (I32, [I32]),
    "kpseinnameok": (BOOL, [CSTR]), "kpseoutnameok": (BOOL, [CSTR]),
    "strlen": (I32, [CSTR]),
    "newvfpacket": (I32, [I32]),
    "getllx": (I32, []), "getlly": (I32, []), "geturx": (I32, []), "getury": (I32, []),
    "isnewsource": (BOOL, [I32, I32]), "makesrcspecial": (I32, [I32, I32]),
    "loadpoolstrings": (I32, [I32]),
    "avlfindobj": (I32, [I32, I32, I32]),
    "ISDIRSEP": (BOOL, [I32]),
    "makefullnamestring": (I32, []),
    "runsystem": (I32, [CSTR]),
    "texmfyesno": (BOOL, [STR]),
    "getpkcharwidth": (I32, [I32, I32]),
    "hasspacechar": (BOOL, [I32]),
    "pdfsetmatrix": (I32, [I32, I32, I32]),
    "getresnameprefix": (I32, []),
    "checkimageb": (BOOL, [I32]), "checkimagec": (BOOL, [I32]), "checkimagei": (BOOL, [I32]),
    "readimage": (I32, [I32, I32, I32, I32, I32, I32, I32, I32]),
    "strcmp": (I32, [CSTR, CSTR]),
    "hasfmentry": (BOOL, [I32]),
    "pdfcharmarked": (BOOL, [I32, I32]),
    "makecstring": (CSTR, [I32]),
    "fabs": (REAL, [REAL]),
}

PROCS = {
    "fflush": [FILE],
    "pdfassert": [BOOL],
    "aclose": [FILE], "bclose": [FILE], "wclose": [FILE],
    "uexit": [I32],
    "pdfmarkchar": [I32, I32],
    "writedvi": [I32, I32], "writepdf": [I32, I32],
    "synctexvoidvlist": [I32, I32], "synctexvoidhlist": [I32, I32], "synctexhlist": [I32],
    "synctexcurrent": [], "synctexkern": [I32, I32], "synctexmath": [I32, I32],
    "synctexhorizontalruleorglue": [I32, I32], "synctextsilh": [I32], "synctexvlist": [I32],
    "synctextsilv": [I32], "synctexsheet": [I32], "synctexteehs": [], "synctexstartinput": [],
    "synctexpdfrefxform": [I32], "synctexpdfxform": [I32], "synctexmrofxfdp": [],
    "synctexabort": [BOOL], "synctexterminate": [BOOL], "synctexinitcommand": [],
    "allocvffnts": [],
    "writezip": [BOOL],
    "storepacket": [I32, I32, I32],
    "remembersourceinfo": [I32, I32],
    "makepdftexbanner": [], "printIDalt": [I32], "printID": [I32],
    "readtcxfile": [], "initstarttime": [], "topenin": [],
    "avlputobj": [I32, I32],
    "escapestring": [I32], "escapename": [I32], "escapehex": [I32], "unescapehex": [I32],
    "getcreationdate": [], "getfilemoddate": [I32], "getfilesize": [I32],
    "getmd5sum": [I32, BOOL], "getfiledump": [I32, I32, I32],
    "matchstrings": [I32, I32, I32, BOOL], "getmatch": [I32],
    "recorderchangefilename": [CSTR],
    "writestreamlength": [I64, I64],
    "pdfmaplinesp": [], "pushpacketstate": [], "poppacketstate": [], "startpacket": [I32, I32],
    "checkpdfsave": [I32, I32], "checkpdfrestore": [I32, I32],
    "writeimage": [I32], "deleteimage": [I32],
    "matrixtransformrect": [I32, I32, I32, I32], "matrixtransformpoint": [I32, I32],
    "matrixrecalculate": [I32],
    "setimagegroupref": [I32, I32],
    "kpsesetprogramenabled": [I32, I32, I32],
    "setjobid": [I32, I32, I32, I32],
    "pdfshipoutbegin": [BOOL], "pdfshipoutend": [BOOL],
    "updateimageprocset": [I32],
    "printcreationdate": [], "printmoddate": [],
    "dumpimagemeta": [], "dumptounicode": [], "undumpimagemeta": [I32, I32, I32], "undumptounicode": [],
    "deftounicode": [I32, I32],
    "pdfmapfile": [I32], "pdfmapline": [I32],
    "removepdffile": [], "garbagewarning": [], "flushjbig2page0objects": [],
    "dopdffont": [I32, I32], "writefontstuff": [], "libpdffinish": [],
    "pdfinitmapfile": [STR],
    "kpseinitprog": [STR, I32, CSTR, CSTR],
    "printcstring": [CSTR],
    "putbyte": [I32, FILE],
    "fputs": [CSTR, FILE],
}

FILE_OPENS = {
    # name -> (runtime method, file kind, extra args)
    "aopenin": ("open_in_or_pipe", "alpha"),
    "aopenout": ("open_out_or_pipe", "alpha"),
    "bopenin": ("bopenin", "byte"),
    "bopenout": ("bopenout", "byte"),
    "wopenin": ("wopenin", "word"),
    "wopenout": ("wopenout", "word"),
    "texbopenin": ("texbopenin", "byte"),
    "vfbopenin": ("vfbopenin", "byte"),
}

WRITE_CHAR_BASES = ("xchr", "months", "nameoffile", "ASCII04", "ASCII1", "ASCIIall")
WRITE_STR_IDS = ("versionstring", "poolname", "formatengine", "dumpname")


def _Expr(em, code, ty, calls=False):
    from emit import Expr
    return Expr(code, ty, calls)


def conv_arg(em, a, pt):
    ex = em.ex(a)
    if pt is None:
        return ex.code, ex.calls
    if pt == FILE:
        if ex.ty[0] != "file":
            raise em_error(f"expected file, got {ex.ty}: {ex.code}")
        return ex.code, ex.calls
    if pt == STR:
        if ex.ty == STR:
            return ex.code, ex.calls
        if ex.ty == I32 and a[0] == "str":  # one-character literal
            return f'b"{em.escape(a[1])}"', False
        raise em_error(f"expected string literal, got {ex.code}")
    if pt == CSTR and ex.ty == STR:
        return f'Ptr::<u8>::cstr_lit(b"{em.bytes_lit(ex.code)}\\0")', ex.calls
    if pt == CSTR and ex.ty == ("nil",):
        return "Ptr::<u8>::null()", False
    return em.conv(ex, pt), ex.calls


def em_error(msg):
    from emit import TranslateError
    return TranslateError(msg)


def call_default(em, name, args, params):
    if len(args) != len(params):
        raise em_error(f"{name}: {len(args)} args, expected {len(params)}")
    codes, calls = [], False
    for a, pt in zip(args, params):
        c, cl = conv_arg(em, a, pt)
        codes.append(c)
        calls = calls or cl
    return em.call_with_temps(f"self.{name}", codes, calls)


def extern_expr(em, name, args, stmt=False):
    """Emit an external used as an expression (args None: bare identifier)."""
    if args is None:
        if name in EXTERN_CONSTS:
            return em.const_expr(EXTERN_CONSTS[name])
        if name in ("stdout", "stderr", "stdin"):
            return _Expr(em, f"FileRef::{name.upper()}", ("file", "alpha"))
        if name in ("true", "false"):
            return _Expr(em, name, BOOL)
        if name == "promptfilenamehelpmsg":
            return _Expr(em, 'Ptr::<u8>::cstr_lit(b"(Press Enter to retry, or Control-Z to exit\\0")', CSTR)
        if name in FUNCS and not FUNCS[name][1]:
            return _Expr(em, f"self.{name}()", FUNCS[name][0], True)
        if name in PROCS and not PROCS[name] and stmt:
            return _Expr(em, f"self.{name}()", UNIT, True)
        return None
    n = name
    if n in ("chr", "ord"):
        return em.ex(args[0])
    if n == "odd":
        a = em.ex(args[0])
        return _Expr(em, f"(({em.as_i32(a)} & 1) != 0)", BOOL, a.calls)
    if n == "abs":
        a = em.ex(args[0])
        if a.ty == REAL:
            return _Expr(em, f"w2c_abs_real({a.code})", I32, a.calls)
        if a.ty == I64:
            return _Expr(em, f"({a.code}).wrapping_abs()", I64, a.calls)
        return _Expr(em, f"({em.as_i32(a)}).wrapping_abs()", I32, a.calls)
    if n == "fabs":
        a = em.ex(args[0])
        return _Expr(em, f"w2c_fabs({em.conv(a, REAL)})", REAL, a.calls)
    if n == "round":
        a = em.ex(args[0])
        return _Expr(em, f"zround({em.conv(a, REAL)})", I32, a.calls)
    if n == "trunc":
        a = em.ex(args[0])
        return _Expr(em, f"({em.conv(a, REAL)}) as i32", I32, a.calls)
    if n == "ucharcast":
        a = em.ex(args[0])
        return _Expr(em, f"({em.as_i32(a)} as u8)", U8, a.calls)
    if n in ("stringcast", "conststringcast"):
        a = em.ex(args[0])
        if a.ty[0] != "ptr":
            raise em_error(f"{n} of {a.ty}")
        return _Expr(em, f"{a.code}.cast::<u8>()", CSTR, a.calls)
    if n == "addressof":
        lv = em.lv(args[0])
        t = em.lv_type(lv)
        return _Expr(em, f"Ptr::<{em.rty(t)}>::from_raw(&raw mut {lv.place})", ("ptr", t), lv.calls)
    if n == "xmallocarray":
        t = em.resolve_type(("name", args[0][1]))
        sz = em.ex(args[1])
        return _Expr(em, f"Ptr::<{em.rty(t)}>::xmalloc_array({em.as_i32(sz)})", ("ptr", t), sz.calls)
    if n == "xreallocarray":
        p = em.ex(args[0])
        t = em.resolve_type(("name", args[1][1]))
        sz = em.ex(args[2])
        return _Expr(em, f"{p.code}.xrealloc_array({em.as_i32(sz)})", ("ptr", t), p.calls or sz.calls)
    if n in FILE_OPENS:
        method, kind = FILE_OPENS[n]
        lv = em.lv(args[0])
        extra = []
        for a in args[1:]:
            extra.append(em.as_i32(em.ex(a)))
        t = em.fresh("f")
        code = (f"{{ let ({t}, __ok) = self.{method}({', '.join(extra)}); {lv.place} = {t}; __ok }}")
        return _Expr(em, code, BOOL, True)
    if n in FUNCS:
        ret, params = FUNCS[n]
        return _Expr(em, call_default(em, n, args, params), ret, True)
    if n in PROCS and stmt:
        return _Expr(em, call_default(em, n, args, PROCS[n]), UNIT, True)
    return None


def write_args(em, args, ln):
    """fixwrites: classify each argument as char, string or integer."""
    if not args:
        raise em_error("write without args")
    first = args[0]
    fexpr = None
    rest = args
    is_value = first[0] == "str" or (first[0] in ("index", "id") and _base_name(first) in
                                     ("buffer", "xchr", "dig", "HEX", "versionstring", "kpathseaversionstring"))
    if not is_value:
        fe = em.ex(first)
        if fe.ty[0] == "file":
            fexpr = fe.code
            rest = args[1:]
    if fexpr is None:
        fexpr = "FileRef::STDOUT"
    lines = [f"let __wf = {fexpr};"]
    for a in rest:
        width = None
        if a[0] == "fmt":
            a, width = a[1], a[2]
        bn = _base_name(a)
        if a[0] == "str" and len(a[1]) == 1:
            lines.append(f"self.w_char(__wf, {ord(a[1][0])});")
        elif a[0] == "str":
            lines.append(f'self.w_str(__wf, b"{em.escape(a[1])}");')
        elif bn in WRITE_CHAR_BASES and a[0] == "index":
            ex = em.ex(a)
            lines.append(f"{{ let __c = {em.as_i32(ex)}; self.w_char(__wf, __c); }}")
        elif bn in WRITE_STR_IDS or (a[0] == "call" and a[1] in ("stringcast", "conststringcast")):
            ex = em.ex(a)
            if ex.ty == STR:
                lines.append(f"self.w_str(__wf, {ex.code});")
            else:
                lines.append(f"{{ let __s = {ex.code}; self.w_cstr(__wf, __s); }}")
        else:
            ex = em.ex(a)
            if ex.ty == REAL:
                raise em_error("write of real")
            v = ex.code if ex.ty == I64 else f"({em.as_i32(ex)} as i64)"
            if width is not None:
                w = em.ex(width)
                lines.append(f"{{ let __v = {v}; self.w_int_width(__wf, __v, {em.as_i32(w)}); }}")
            else:
                lines.append(f"{{ let __v = {v}; self.w_int(__wf, __v); }}")
    if ln:
        lines.append("self.w_ln(__wf);")
    return "{ " + " ".join(lines) + " }"


def _base_name(a):
    while a[0] in ("index", "field", "call") and a[0] != "call":
        a = a[1]
    if a[0] == "id":
        return a[1]
    if a[0] == "call":
        return a[1]
    return None


def extern_stmt(em, name, args):
    n = name
    if n in ("incr", "decr"):
        lv = em.lv(args[0])
        t = em.lv_type(lv)
        op = "+" if n == "incr" else "-"
        from emit import Expr
        if lv.kind == "word" and lv.path:
            stem, ft = em.word_field(lv.wname, lv.path)
            tv = em.fresh("w")
            return (f"{{ let {tv} = &mut {lv.place}; {tv}.set_{stem}((({tv}.{stem}() as i32) {op} 1) as {em.rty(ft)}); }}")
        if t[0] == "int":
            if t[1] in ("i32", "i64"):
                return f"{lv.place} {op}= 1;"
            return f"{lv.place} = (({lv.place} as i32) {op} 1) as {t[1]};"
        if t[0] == "ptr":
            return f"{lv.place} = {lv.place}.off({'1' if n == 'incr' else '-1'});"
        raise em_error(f"{n} on {t}")
    if n in ("write", "writeln"):
        if n == "writeln" and len(args) == 1:
            fe = em.ex(args[0])
            return f"self.w_ln({fe.code});"
        return write_args(em, args, n == "writeln")
    if n in ("dumpthings", "undumpthings"):
        lv = em.lv(args[0])
        cnt = em.ex(args[1])
        if n == "dumpthings":
            label = _base_name(args[0]) or "?"
            return f"{{ let __p = &raw mut {lv.place}; let __n = {em.as_i32(cnt)}; self.dump_things_l(__p, __n, \"{label}\"); }}"
        return f"{{ let __p = &raw mut {lv.place}; let __n = {em.as_i32(cnt)}; self.undump_things(__p, __n); }}"
    if n in ("dumpwd", "dumphh", "dumpqqqq", "undumpwd", "undumphh", "undumpqqqq"):
        lv = em.lv(args[0])
        meth = "dump_things" if n.startswith("dump") else "undump_things"
        return f"{{ let __p = &raw mut {lv.place}; self.{meth}(__p, 1); }}"
    if n == "dumpint":
        ex = em.ex(args[0])
        return f"{{ let __v: i32 = {em.as_i32(ex)}; self.dump_int(__v); }}"
    if n == "undumpint":
        lv = em.lv(args[0])
        from emit import Expr
        return em.assign(lv, Expr("self.undump_int()", I32, True))
    if n == "undumpcheckedthings":
        lo, hi = em.ex(args[0]), em.ex(args[1])
        lv = em.lv(args[2])
        cnt = em.ex(args[3])
        return (f"{{ let __lo = {em.as_i32(lo)}; let __hi = {em.as_i32(hi)}; let __p = &raw mut {lv.place}; "
                f"let __n = {em.as_i32(cnt)}; self.undump_checked_things(__lo, __hi, __p, __n); }}")
    if n == "undumpuppercheckthings":
        hi = em.ex(args[0])
        lv = em.lv(args[1])
        cnt = em.ex(args[2])
        return (f"{{ let __hi = {em.as_i32(hi)}; let __p = &raw mut {lv.place}; "
                f"let __n = {em.as_i32(cnt)}; self.undump_upper_check_things(__hi, __p, __n); }}")
    if n == "setupboundvariable":
        lv = em.lv(args[0][2][0])  # addressof(x)
        name_ = em.ex(args[1])
        dflt = em.ex(args[2])
        return (f"{{ let __n = {name_.code}; let __d = {em.as_i32(dflt)}; let __v = self.setup_bound_var(__n, __d); "
                f"{lv.place} = __v; }}")
    if n == "libcfree":
        ex = em.ex(args[0])
        return f"{ex.code}.free();"
    if n in ("secondsandmicros",):
        a, b = em.lv(args[0]), em.lv(args[1])
        return f"{{ let (__s, __m) = self.seconds_and_micros(); {a.place} = __s; {b.place} = __m; }}"
    if n == "dateandtime":
        lvs = [em.lv(x) for x in args]
        return ("{ let (__t, __d, __mo, __y) = self.date_and_time(); "
                + " ".join(f"{lv.place} = {v};" for lv, v in zip(lvs, ("__t", "__d", "__mo", "__y"))) + " }")
    if n == "strcpy":
        dst = em.ex(args[0])
        src = em.ex(args[1])
        return f'{{ let __d = {dst.code}; w2c_strcpy(__d, {conv_arg(em, args[1], CSTR)[0]}); }}'
    if n == "calledit":
        ex = [em.ex(a) for a in args]
        return f"self.calledit({ex[0].code}, {em.as_i32(ex[1])}, {em.as_i32(ex[2])}, {em.as_i32(ex[3])});"
    if n in PROCS:
        return call_default(em, n, args, PROCS[n]) + ";"
    if n in FUNCS:  # function called as a statement
        return call_default(em, n, args, FUNCS[n][1]) + ";"
    if n in FILE_OPENS:
        ex = extern_expr(em, n, args)
        return ex.code + ";"
    return None
