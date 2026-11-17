//! tex-file.c: the file format table, search path initialization
//! (`kpathsea_init_format`), `kpathsea_find_file[_generic]`, and the
//! mktex enable/disable switches.

use crate::debug::{KPSE_DEBUG_PATHS, KPSE_DEBUG_SEARCH};
use crate::expand::expand_default;
use crate::util::*;
use crate::Kpse;

/// `kpse_file_format_type`, in the order of kpathsea/types.h.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
#[repr(u32)]
pub enum FileFormat {
    Gf = 0,
    Pk,
    AnyGlyph,
    Tfm,
    Afm,
    Base,
    Bib,
    Bst,
    Cnf,
    Db,
    Fmt,
    Fontmap,
    Mem,
    Mf,
    Mfpool,
    Mft,
    Mp,
    Mppool,
    Mpsupport,
    Ocp,
    Ofm,
    Opl,
    Otp,
    Ovf,
    Ovp,
    Pict,
    Tex,
    Texdoc,
    Texpool,
    Texsource,
    TexPsHeader,
    TroffFont,
    Type1,
    Vf,
    DvipsConfig,
    Ist,
    Truetype,
    Type42,
    Web2c,
    ProgramText,
    ProgramBinary,
    Miscfonts,
    Web,
    Cweb,
    Enc,
    Cmap,
    Sfd,
    Opentype,
    PdftexConfig,
    Lig,
    Texmfscripts,
    Lua,
    Fea,
    Cid,
    Mlbib,
    Mlbst,
    Clua,
    Ris,
    Bltxml,
}

/// `kpse_last_format`.
pub const LAST_FORMAT: usize = 59;

const ALL_FORMATS: [FileFormat; LAST_FORMAT] = {
    use FileFormat::*;
    [
        Gf, Pk, AnyGlyph, Tfm, Afm, Base, Bib, Bst, Cnf, Db, Fmt, Fontmap, Mem, Mf, Mfpool, Mft, Mp,
        Mppool, Mpsupport, Ocp, Ofm, Opl, Otp, Ovf, Ovp, Pict, Tex, Texdoc, Texpool, Texsource,
        TexPsHeader, TroffFont, Type1, Vf, DvipsConfig, Ist, Truetype, Type42, Web2c, ProgramText,
        ProgramBinary, Miscfonts, Web, Cweb, Enc, Cmap, Sfd, Opentype, PdftexConfig, Lig,
        Texmfscripts, Lua, Fea, Cid, Mlbib, Mlbst, Clua, Ris, Bltxml,
    ]
};

impl FileFormat {
    /// Format with numeric value I.
    pub fn from_index(i: usize) -> Option<FileFormat> {
        ALL_FORMATS.get(i).copied()
    }
    /// Numeric value (as in C).
    pub fn index(self) -> usize {
        self as usize
    }
    /// All formats in order.
    pub fn all() -> &'static [FileFormat] {
        &ALL_FORMATS
    }
}

/// `kpse_src_type`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord, Default)]
#[repr(u32)]
pub enum SrcLevel {
    #[default]
    Implicit = 0,
    Compile,
    TexmfCnf,
    ClientCnf,
    Env,
    X,
    Cmdline,
}

/// `kpse_format_info_type`.
#[derive(Default, Clone)]
pub(crate) struct FormatInfo {
    pub(crate) type_: Option<&'static str>,
    pub(crate) path: Option<Vec<u8>>,
    pub(crate) raw_path: Option<Vec<u8>>,
    pub(crate) path_source: Option<Vec<u8>>,
    pub(crate) override_path: Option<Vec<u8>>,
    pub(crate) client_path: Option<Vec<u8>>,
    pub(crate) cnf_path: Option<Vec<u8>>,
    pub(crate) default_path: Option<Vec<u8>>,
    pub(crate) suffix: Option<Vec<Vec<u8>>>,
    pub(crate) alt_suffix: Option<Vec<Vec<u8>>>,
    pub(crate) suffix_search_only: bool,
    pub(crate) program: Option<Vec<u8>>,
    pub(crate) argv: Option<Vec<Vec<u8>>>,
    pub(crate) program_enabled_p: bool,
    pub(crate) program_enable_level: SrcLevel,
    pub(crate) binmode: bool,
}

/// Static description of one format (the `case` arms of
/// `kpathsea_init_format_return_varlist`).
struct Spec {
    type_: &'static str,
    envs: &'static [&'static str],
    suffixes: Option<&'static [&'static str]>,
    alt: Option<&'static [&'static str]>,
    suffix_only: bool,
    binmode: bool,
    maketex: Option<(&'static str, &'static [&'static str])>,
}

const MKTEXPK_ARGS: &[&str] = &[
    "--mfmode",
    "$MAKETEX_MODE",
    "--bdpi",
    "$MAKETEX_BASE_DPI",
    "--mag",
    "$MAKETEX_MAG",
    "--dpi",
    "$KPATHSEA_DPI",
];
const NOARGS: &[&str] = &[];

/// The compile-time `DEFAULT_TEXMFCNF` of the TeX Live 2026 Windows build
/// (paths.h).  All other compile-time defaults are `/nonesuch`.
pub(crate) const DEFAULT_TEXMFCNF: &str = "{$SELFAUTOLOC,$SELFAUTOLOC/share/texmf-local/web2c,$SELFAUTOLOC/share/texmf-dist/web2c,$SELFAUTOLOC/share/texmf/web2c,$SELFAUTOLOC/texmf-local/web2c,$SELFAUTOLOC/texmf-dist/web2c,$SELFAUTOLOC/texmf/web2c,$SELFAUTODIR,$SELFAUTODIR/share/texmf-local/web2c,$SELFAUTODIR/share/texmf-dist/web2c,$SELFAUTODIR/share/texmf/web2c,$SELFAUTODIR/texmf-local/web2c,$SELFAUTODIR/texmf-dist/web2c,$SELFAUTODIR/texmf/web2c,$SELFAUTOGRANDPARENT/texmf-local/web2c,$SELFAUTOPARENT,$SELFAUTOPARENT/share/texmf-local/web2c,$SELFAUTOPARENT/share/texmf-dist/web2c,$SELFAUTOPARENT/share/texmf/web2c,$SELFAUTOPARENT/texmf-local/web2c,$SELFAUTOPARENT/texmf-dist/web2c,$SELFAUTOPARENT/texmf/web2c}";
const NONESUCH: &str = "/nonesuch";

fn spec(f: FileFormat) -> Spec {
    use FileFormat::*;
    let s = |type_, envs, suffixes, alt, suffix_only, binmode, maketex| Spec {
        type_,
        envs,
        suffixes,
        alt,
        suffix_only,
        binmode,
        maketex,
    };
    const GLYPH: &[&str] = &["GLYPHFONTS", "TEXFONTS"];
    match f {
        Gf => s("gf", &["GFFONTS", "GLYPHFONTS", "TEXFONTS"], Some(&["gf"]), None, true, true, None),
        Pk => s(
            "pk",
            &["PKFONTS", "TEXPKS", "GLYPHFONTS", "TEXFONTS"],
            Some(&["pk"]),
            None,
            true,
            true,
            Some(("mktexpk", MKTEXPK_ARGS)),
        ),
        AnyGlyph => s("bitmap font", GLYPH, None, None, true, true, Some(("mktexpk", MKTEXPK_ARGS))),
        Tfm => s(
            "tfm",
            &["TFMFONTS", "TEXFONTS"],
            Some(&[".tfm"]),
            None,
            true,
            true,
            Some(("mktextfm", NOARGS)),
        ),
        Afm => s("afm", &["AFMFONTS", "TEXFONTS"], Some(&[".afm"]), None, false, false, None),
        Base => s(
            "base",
            &["MFBASES", "TEXMFINI"],
            Some(&[".base"]),
            None,
            false,
            true,
            Some(("mktexfmt", NOARGS)),
        ),
        Bib => s("bib", &["BIBINPUTS", "TEXBIB"], Some(&[".bib"]), None, true, false, None),
        Bst => s("bst", &["BSTINPUTS"], Some(&[".bst"]), None, false, false, None),
        Cnf => s("cnf", &["TEXMFCNF"], Some(&[".cnf"]), None, false, false, None),
        Db => s("ls-R", &["TEXMFDBS"], Some(&["ls-R", "ls-r"]), None, false, false, None),
        Fmt => s(
            "fmt",
            &["TEXFORMATS", "TEXMFINI"],
            Some(&[".fmt"]),
            None,
            false,
            true,
            Some(("mktexfmt", NOARGS)),
        ),
        Fontmap => s("map", &["TEXFONTMAPS", "TEXFONTS"], Some(&[".map"]), None, false, false, None),
        Mem => s(
            "mem",
            &["MPMEMS", "TEXMFINI"],
            Some(&[".mem"]),
            None,
            false,
            true,
            Some(("mktexfmt", NOARGS)),
        ),
        Mf => s("mf", &["MFINPUTS"], Some(&[".mf"]), None, false, false, Some(("mktexmf", NOARGS))),
        Mfpool => s("mfpool", &["MFPOOL", "TEXMFINI"], Some(&[".pool"]), None, false, false, None),
        Mft => s("mft", &["MFTINPUTS"], Some(&[".mft"]), None, false, false, None),
        Mp => s("mp", &["MPINPUTS"], Some(&[".mp"]), None, false, false, None),
        Mppool => s("mppool", &["MPPOOL", "TEXMFINI"], Some(&[".pool"]), None, false, false, None),
        Mpsupport => s("MetaPost support", &["MPSUPPORT"], None, None, false, false, None),
        Ocp => s("ocp", &["OCPINPUTS"], Some(&[".ocp"]), None, true, true, Some(("mkocp", NOARGS))),
        Ofm => s(
            "ofm",
            &["OFMFONTS", "TEXFONTS"],
            Some(&[".ofm", ".tfm"]),
            None,
            true,
            true,
            Some(("mkofm", NOARGS)),
        ),
        Opl => s("opl", &["OPLFONTS", "TEXFONTS"], Some(&[".opl"]), Some(&[".pl"]), true, false, None),
        Otp => s("otp", &["OTPINPUTS"], Some(&[".otp"]), None, true, false, None),
        Ovf => s("ovf", &["OVFFONTS", "TEXFONTS"], Some(&[".ovf", ".vf"]), None, true, true, None),
        Ovp => s("ovp", &["OVPFONTS", "TEXFONTS"], Some(&[".ovp"]), Some(&[".vpl"]), true, false, None),
        Pict => s(
            "graphic/figure",
            &["TEXPICTS", "TEXINPUTS"],
            None,
            Some(&[".eps", ".epsi"]),
            false,
            true,
            None,
        ),
        Tex => s(
            "tex",
            &["TEXINPUTS"],
            Some(&[".tex"]),
            Some(&[".sty", ".cls", ".fd", ".aux", ".bbl", ".def", ".clo", ".ldf"]),
            false,
            false,
            Some(("mktextex", NOARGS)),
        ),
        TexPsHeader => s(
            "PostScript header",
            &["TEXPSHEADERS", "PSHEADERS"],
            None,
            Some(&[".pro"]),
            false,
            true,
            None,
        ),
        Texdoc => s("TeX system documentation", &["TEXDOCS"], None, None, false, false, None),
        Texpool => s("texpool", &["TEXPOOL", "TEXMFINI"], Some(&[".pool"]), None, false, false, None),
        Texsource => s(
            "TeX system sources",
            &["TEXSOURCES"],
            None,
            Some(&[".dtx", ".ins"]),
            false,
            false,
            None,
        ),
        TroffFont => s("Troff fonts", &["TRFONTS"], None, None, false, true, None),
        Type1 => s(
            "type1 fonts",
            &["T1FONTS", "T1INPUTS", "TEXFONTS", "TEXPSHEADERS", "PSHEADERS"],
            Some(&[".pfa", ".pfb"]),
            None,
            false,
            true,
            None,
        ),
        Vf => s("vf", &["VFFONTS", "TEXFONTS"], Some(&[".vf"]), None, true, true, None),
        DvipsConfig => s("dvips config", &["TEXCONFIG"], None, None, false, false, None),
        Ist => s("ist", &["TEXINDEXSTYLE", "INDEXSTYLE"], Some(&[".ist"]), None, false, false, None),
        Truetype => s(
            "truetype fonts",
            &["TTFONTS", "TEXFONTS"],
            Some(&[".ttf", ".ttc", ".TTF", ".TTC", ".dfont"]),
            None,
            false,
            true,
            None,
        ),
        Type42 => s("type42 fonts", &["T42FONTS", "TEXFONTS"], Some(&[".t42", ".T42"]), None, false, true, None),
        Web2c => s("web2c files", &["WEB2C"], None, None, false, false, None),
        ProgramText => s("other text files", &[], None, None, false, false, None),
        ProgramBinary => s("other binary files", &[], None, None, false, true, None),
        Miscfonts => s("misc fonts", &["MISCFONTS", "TEXFONTS"], None, None, false, true, None),
        Web => s("web", &["WEBINPUTS"], Some(&[".web"]), Some(&[".ch"]), false, false, None),
        Cweb => s("cweb", &["CWEBINPUTS"], Some(&[".w", ".web"]), Some(&[".ch"]), false, false, None),
        Enc => s("enc files", &["ENCFONTS", "TEXFONTS"], Some(&[".enc"]), None, true, false, None),
        Cmap => s("cmap files", &["CMAPFONTS", "TEXFONTS"], None, None, false, false, None),
        Sfd => s(
            "subfont definition files",
            &["SFDFONTS", "TEXFONTS"],
            Some(&[".sfd"]),
            None,
            true,
            false,
            None,
        ),
        Opentype => s(
            "opentype fonts",
            &["OPENTYPEFONTS", "TEXFONTS"],
            Some(&[".otf", ".OTF"]),
            None,
            true,
            true,
            None,
        ),
        PdftexConfig => s("pdftex config", &["PDFTEXCONFIG"], None, None, false, false, None),
        Lig => s("lig files", &["LIGFONTS", "TEXFONTS"], Some(&[".lig"]), None, true, false, None),
        Texmfscripts => s("texmfscripts", &["TEXMFSCRIPTS"], None, None, false, false, None),
        Lua => s(
            "lua",
            &["LUAINPUTS"],
            Some(&[".lua", ".luatex", ".luc", ".luctex", ".texlua", ".texluc", ".tlu"]),
            None,
            true,
            false,
            None,
        ),
        Fea => s("font feature files", &["FONTFEATURES"], Some(&[".fea"]), None, true, false, None),
        Cid => s("cid maps", &["FONTCIDMAPS"], Some(&[".cid", ".cidmap"]), None, true, false, None),
        Mlbib => s(
            "mlbib",
            &["MLBIBINPUTS", "BIBINPUTS", "TEXBIB"],
            Some(&[".mlbib", ".bib"]),
            None,
            true,
            false,
            None,
        ),
        Mlbst => s(
            "mlbst",
            &["MLBSTINPUTS", "BSTINPUTS"],
            Some(&[".mlbst", ".bst"]),
            None,
            true,
            false,
            None,
        ),
        Clua => s("clua", &["CLUAINPUTS"], Some(&[".dll", ".so"]), None, true, false, None),
        Ris => s("ris", &["RISINPUTS"], Some(&[".ris"]), None, true, false, None),
        Bltxml => s("bltxml", &["BLTXMLINPUTS"], Some(&[".bltxml"]), None, true, false, None),
    }
}

fn to_list(l: &[&str]) -> Vec<Vec<u8>> {
    l.iter().map(|s| s.as_bytes().to_vec()).collect()
}

impl Kpse {
    /// tex-file.c `kpathsea_set_program_enabled`.
    pub fn set_program_enabled(&mut self, fmt: FileFormat, value: bool, level: SrcLevel) {
        let f = &mut self.format_info[fmt as usize];
        if level >= f.program_enable_level {
            f.program_enabled_p = value;
            f.program_enable_level = level;
        }
    }

    /// tex-file.c `kpathsea_maketex_option` (`-mktex=FMT` / `-no-mktex`).
    pub fn maketex_option(&mut self, fmtname: impl AsRef<[u8]>, value: bool) {
        let n = fmtname.as_ref();
        let fmt = if n.eq_ignore_ascii_case(b"pk") {
            Some(FileFormat::Pk)
        } else if n.eq_ignore_ascii_case(b"mf") {
            Some(FileFormat::Mf)
        } else if n.eq_ignore_ascii_case(b"tex") {
            Some(FileFormat::Tex)
        } else if n.eq_ignore_ascii_case(b"tfm") {
            Some(FileFormat::Tfm)
        } else if n.eq_ignore_ascii_case(b"fmt") {
            Some(FileFormat::Fmt)
        } else if n.eq_ignore_ascii_case(b"ofm") {
            Some(FileFormat::Ofm)
        } else if n.eq_ignore_ascii_case(b"ocp") {
            Some(FileFormat::Ocp)
        } else {
            let m = bcat!(b"\nkpathsea: Unknown mktex format: ", n, b"\n");
            self.err_write(&m);
            None
        };
        if let Some(f) = fmt {
            self.set_program_enabled(f, value, SrcLevel::Cmdline);
        }
    }

    /// `kpathsea_set_suffixes`: replace the (alternate) suffix list.
    pub fn set_suffixes(&mut self, format: FileFormat, alternate: bool, suffixes: &[&[u8]]) {
        let l: Vec<Vec<u8>> = suffixes.iter().map(|s| s.to_vec()).collect();
        let f = &mut self.format_info[format as usize];
        if alternate {
            f.alt_suffix = Some(l);
        } else {
            f.suffix = Some(l);
        }
    }

    /// Set the "client path" of a format (`kpse_format_info[f].client_path`,
    /// e.g. from a program config file).  Takes effect at initialization.
    pub fn set_client_path(&mut self, format: FileFormat, path: Option<&[u8]>) {
        self.format_info[format as usize].client_path = path.map(|p| p.to_vec());
    }

    /// Set the "override path" of a format (`override_path`).
    pub fn set_override_path(&mut self, format: FileFormat, path: Option<&[u8]>) {
        self.format_info[format as usize].override_path = path.map(|p| p.to_vec());
    }

    /// tex-file.c `init_maketex`.
    fn init_maketex(&mut self, fmt: FileFormat, dflt_prog: &str, args: &[&str]) {
        let prog = self.format_info[fmt as usize]
            .program
            .clone()
            .unwrap_or_else(|| dflt_prog.as_bytes().to_vec());
        let prog_up = uppercasify(&prog);
        let progval = self.var_value(&prog_up);
        {
            let f = &mut self.format_info[fmt as usize];
            f.program = Some(prog);
            let mut argv = vec![dflt_prog.as_bytes().to_vec()];
            argv.extend(args.iter().map(|a| a.as_bytes().to_vec()));
            f.argv = Some(argv);
        }
        if let Some(v) = progval {
            if !v.is_empty() {
                self.set_program_enabled(fmt, v[0] == b'1', SrcLevel::ClientCnf);
            }
        }
    }

    /// tex-file.c `init_path`.
    fn init_path(&mut self, fmt: FileFormat, default_path: Vec<u8>, envs: &[Vec<u8>]) {
        let is_cnf = fmt == FileFormat::Cnf;
        self.format_info[fmt as usize].default_path = Some(default_path.clone());
        let mut var: Option<Vec<u8>> = None;
        let mut env_value: Option<Vec<u8>> = None;
        for env_name in envs {
            if var.is_none() {
                let evar = cat(&[env_name, b".", &self.program_name]);
                match self.env.get(&evar).filter(|v| !v.is_empty()) {
                    Some(v) => {
                        env_value = Some(v);
                        var = Some(evar);
                    }
                    None => {
                        let evar = cat(&[env_name, b"_", &self.program_name]);
                        match self.env.get(&evar).filter(|v| !v.is_empty()) {
                            Some(v) => {
                                env_value = Some(v);
                                var = Some(evar);
                            }
                            None => {
                                if let Some(v) = self.env.get(env_name).filter(|v| !v.is_empty()) {
                                    env_value = Some(v);
                                    var = Some(env_name.clone());
                                }
                            }
                        }
                    }
                }
            }
            if self.format_info[fmt as usize].cnf_path.is_none() && !is_cnf {
                let c = self.cnf_get(env_name);
                self.format_info[fmt as usize].cnf_path = c;
            }
            if var.is_some() && self.format_info[fmt as usize].cnf_path.is_some() {
                break;
            }
        }

        let info = &mut self.format_info[fmt as usize];
        info.raw_path = Some(default_path.clone());
        info.path = Some(default_path);
        info.path_source = Some(b"compile-time paths.h".to_vec());

        fn expand_default_into(info: &mut FormatInfo, try_path: Option<Vec<u8>>, source: Vec<u8>) {
            if let Some(tp) = try_path {
                let cur = info.path.take().unwrap_or_default();
                info.path = Some(expand_default(Some(&tp), &cur));
                info.raw_path = Some(tp);
                info.path_source = Some(source);
            }
        }
        let cnf_path = info.cnf_path.clone();
        expand_default_into(info, cnf_path, b"texmf.cnf".to_vec());
        let client = info.client_path.clone();
        expand_default_into(info, client, b"program config file".to_vec());
        if let Some(v) = var {
            let src = cat(&[&v, b" environment variable"]);
            expand_default_into(info, env_value, src);
        }
        let ov = info.override_path.clone();
        expand_default_into(info, ov, b"application override variable".to_vec());
        let p = info.path.clone().unwrap_or_default();
        let bp = self.brace_expand(&p);
        self.format_info[fmt as usize].path = Some(bp);
    }

    /// `kpathsea_init_format`: initialize FORMAT if needed and return its
    /// search path.
    pub fn init_format(&mut self, format: FileFormat) -> Vec<u8> {
        if self.format_info[format as usize].path.is_none() {
            self.init_format_return_varlist(format);
        }
        self.format_info[format as usize].path.clone().unwrap_or_default()
    }

    /// `kpathsea_init_format_return_varlist`: (re)initialize FORMAT and
    /// return the list of variables consulted, separated by spaces.
    pub fn init_format_return_varlist(&mut self, format: FileFormat) -> Vec<u8> {
        let sp = spec(format);
        if let Some((prog, args)) = sp.maketex {
            self.init_maketex(format, prog, args);
        }
        let (default_path, envs): (Vec<u8>, Vec<Vec<u8>>) = match format {
            FileFormat::ProgramText | FileFormat::ProgramBinary => {
                let d = cat(&[b".", &[ENV_SEP], b"$TEXMF", b"/", &self.program_name, b"/", b"/"]);
                let e = cat(&[&uppercasify(&self.program_name), b"INPUTS"]);
                (d, vec![e])
            }
            FileFormat::Cnf => (DEFAULT_TEXMFCNF.as_bytes().to_vec(), to_list(sp.envs)),
            _ => (NONESUCH.as_bytes().to_vec(), to_list(sp.envs)),
        };
        self.format_info[format as usize].type_ = Some(sp.type_);
        self.init_path(format, default_path, &envs);
        let envvar_list = {
            let mut v = Vec::new();
            for (i, e) in envs.iter().enumerate() {
                if i > 0 {
                    v.push(b' ');
                }
                v.extend_from_slice(e);
            }
            v
        };
        {
            let f = &mut self.format_info[format as usize];
            if let Some(s) = sp.suffixes {
                f.suffix = Some(to_list(s));
            }
            if let Some(a) = sp.alt {
                f.alt_suffix = Some(to_list(a));
            }
            if sp.suffix_only {
                f.suffix_search_only = true;
            }
            if format == FileFormat::Truetype {
                f.suffix_search_only = false;
            }
            if sp.binmode {
                f.binmode = true;
            }
            if format == FileFormat::Db {
                f.path = Some(remove_dbonly(f.path.as_deref().unwrap_or(b"")));
            }
        }
        if self.debug_p(KPSE_DEBUG_PATHS) {
            self.describe_format(format, &envvar_list);
        }
        envvar_list
    }

    fn describe_format(&mut self, format: FileFormat, envvar_list: &[u8]) {
        if format == FileFormat::Cnf {
            self.err_write(b"\n");
        }
        let f = self.format_info[format as usize].clone();
        let d = |k: &mut Kpse, m: Vec<u8>| k.debugf(&m);
        d(
            self,
            bcat!(
                b"Search path for ",
                f.type_.unwrap_or(""),
                b" files (from ",
                maybe(&f.path_source),
                b")\n"
            ),
        );
        d(self, bcat!(b"  = ", maybe(&f.path), b"\n"));
        d(self, bcat!(b"  before expansion = ", maybe(&f.raw_path), b"\n"));
        d(self, bcat!(b"  application override path = ", maybe(&f.override_path), b"\n"));
        d(self, bcat!(b"  application config file path = ", maybe(&f.client_path), b"\n"));
        d(self, bcat!(b"  texmf.cnf path = ", maybe(&f.cnf_path), b"\n"));
        d(self, bcat!(b"  compile-time path = ", maybe(&f.default_path), b"\n"));
        d(self, bcat!(b"  environment variables = ", envvar_list, b"\n"));
        self.debugf(b"  default suffixes =");
        match &f.suffix {
            Some(l) => {
                let mut m = Vec::new();
                for s in l {
                    m.push(b' ');
                    m.extend_from_slice(s);
                }
                m.push(b'\n');
                self.err_write(&m);
            }
            None => self.err_write(b" (none)\n"),
        }
        self.debugf(b"  other suffixes =");
        match &f.alt_suffix {
            Some(l) => {
                let mut m = Vec::new();
                for s in l {
                    m.push(b' ');
                    m.extend_from_slice(s);
                }
                m.push(b'\n');
                self.err_write(&m);
            }
            None => self.err_write(b" (none)\n"),
        }
        d(self, bcat!(b"  search only with suffix = ", f.suffix_search_only, b"\n"));
        d(self, bcat!(b"  runtime generation program = ", maybe(&f.program), b"\n"));
        self.debugf(b"  runtime generation command =");
        match &f.argv {
            Some(l) => {
                let mut m = Vec::new();
                for s in l {
                    m.push(b' ');
                    m.extend_from_slice(s);
                }
                m.push(b'\n');
                self.err_write(&m);
            }
            None => self.err_write(b" (none)\n"),
        }
        d(self, bcat!(b"  program enabled = ", f.program_enabled_p, b"\n"));
        d(self, bcat!(b"  program enable level = ", f.program_enable_level as u32, b"\n"));
        d(self, bcat!(b"  open files in binary mode = ", f.binmode, b"\n"));
        d(self, bcat!(b"  numeric format value = ", format as u32, b"\n"));
    }

    /// Search path of FORMAT (initializing it if needed).
    pub fn format_path(&mut self, format: FileFormat) -> Vec<u8> {
        self.init_format(format)
    }

    /// Human-readable type name of FORMAT (`format_info[f].type`), if the
    /// format has been initialized.
    pub fn format_type(&self, format: FileFormat) -> Option<&'static str> {
        self.format_info[format as usize].type_
    }

    /// `(path_source, raw_path)` of an initialized FORMAT.
    pub fn format_origin(&self, format: FileFormat) -> (Vec<u8>, Vec<u8>) {
        let f = &self.format_info[format as usize];
        (f.path_source.clone().unwrap_or_default(), f.raw_path.clone().unwrap_or_default())
    }

    /// Suffix lists of an initialized FORMAT: (suffixes, alternate suffixes).
    pub fn format_suffixes(&self, format: FileFormat) -> (Option<Vec<Vec<u8>>>, Option<Vec<Vec<u8>>>) {
        let f = &self.format_info[format as usize];
        (f.suffix.clone(), f.alt_suffix.clone())
    }

    /// Whether files of FORMAT are opened in binary mode.
    pub fn format_binmode(&self, format: FileFormat) -> bool {
        self.format_info[format as usize].binmode
    }

    /// `target_fontmaps`.
    fn target_fontmaps(&mut self, target: &mut Vec<Vec<u8>>, name: &[u8]) {
        if let Some(mapped) = self.fontmap_lookup(name) {
            for m in mapped {
                target.push(m);
            }
        }
    }

    /// `target_asis_name`.
    fn target_asis_name(
        &mut self,
        target: &mut Vec<Vec<u8>>,
        format: FileFormat,
        name: &[u8],
        use_fontmaps: bool,
        has_potential_suffix: bool,
    ) {
        if has_potential_suffix || !self.format_info[format as usize].suffix_search_only {
            target.push(name.to_vec());
            if use_fontmaps {
                self.target_fontmaps(target, name);
            }
        }
    }

    /// `target_suffixed_names`.
    fn target_suffixed_names(
        &mut self,
        target: &mut Vec<Vec<u8>>,
        format: FileFormat,
        name: &[u8],
        use_fontmaps: bool,
        has_potential_suffix: bool,
    ) {
        let suffixes = match &self.format_info[format as usize].suffix {
            Some(s) if !has_potential_suffix => s.clone(),
            _ => return,
        };
        for ext in suffixes {
            let nws = cat(&[name, &ext]);
            target.push(nws.clone());
            if use_fontmaps {
                self.target_fontmaps(target, &nws);
            }
        }
    }

    /// `kpathsea_find_file`: look up NAME of type FORMAT.
    pub fn find_file(&mut self, name: impl AsRef<[u8]>, format: FileFormat, must_exist: bool) -> Option<Vec<u8>> {
        self.find_file_generic(name, format, must_exist, false).into_iter().next()
    }

    /// `kpse_find_file` returning a lossily-decoded `String`.
    pub fn find_file_str(&mut self, name: &str, format: FileFormat, must_exist: bool) -> Option<String> {
        self.find_file(name.as_bytes(), format, must_exist).map(|v| crate::lossy(&v))
    }

    /// `kpathsea_find_file_generic`: all matches if ALL.
    pub fn find_file_generic(
        &mut self,
        const_name: impl AsRef<[u8]>,
        format: FileFormat,
        must_exist: bool,
        all: bool,
    ) -> Vec<Vec<u8>> {
        let const_name = c_str(const_name.as_ref());
        let use_fontmaps = matches!(
            format,
            FileFormat::Tfm | FileFormat::Gf | FileFormat::Pk | FileFormat::Ofm
        );
        if self.format_info[format as usize].path.is_none() {
            self.init_format(format);
        }
        if self.debug_p(KPSE_DEBUG_SEARCH) {
            let f = &self.format_info[format as usize];
            let m = bcat!(
                b"kpse_find_file: searching for ",
                const_name,
                b" of type ",
                f.type_.unwrap_or(""),
                b" (from ",
                maybe(&f.path_source),
                b")\n"
            );
            self.debugf(&m);
        }
        let name = self.expand(const_name);

        let mut has_any_suffix = name.iter().rposition(|&c| c == b'.').is_some();
        if let Some(d) = name.iter().rposition(|&c| c == b'.') {
            if name[d..].contains(&DIR_SEP) {
                has_any_suffix = false;
            }
        }
        let mut has_potential_suffix = false;
        {
            let f = &self.format_info[format as usize];
            if let Some(sl) = &f.suffix {
                for ext in sl {
                    if name.len() >= ext.len() && file_str_case_eq(ext, &name[name.len() - ext.len()..]) {
                        has_potential_suffix = true;
                        break;
                    }
                }
            }
            if !has_potential_suffix {
                if let Some(sl) = &f.alt_suffix {
                    for ext in sl {
                        if name.len() >= ext.len()
                            && file_str_case_eq(ext, &name[name.len() - ext.len()..])
                        {
                            has_potential_suffix = true;
                            break;
                        }
                    }
                }
            }
        }

        let mut target: Vec<Vec<u8>> = Vec::new();
        let try_std_extension_first = self.var_value(b"try_std_extension_first");
        if has_any_suffix && !cnf_p(try_std_extension_first.as_deref()) {
            self.target_asis_name(&mut target, format, &name, use_fontmaps, has_potential_suffix);
            self.target_suffixed_names(&mut target, format, &name, use_fontmaps, has_potential_suffix);
        } else {
            self.target_suffixed_names(&mut target, format, &name, use_fontmaps, has_potential_suffix);
            self.target_asis_name(&mut target, format, &name, use_fontmaps, has_potential_suffix);
        }

        let path = self.format_info[format as usize].path.clone().unwrap_or_default();
        let mut ret = self.path_search_list_generic(&path, &target, false, all);

        if ret.is_empty() && must_exist {
            let mut target2: Vec<Vec<u8>> = Vec::new();
            let f = &self.format_info[format as usize];
            if !has_potential_suffix && f.suffix_search_only {
                if let Some(sl) = &f.suffix {
                    for ext in sl {
                        target2.push(cat(&[&name, ext]));
                    }
                }
            }
            if has_potential_suffix || !f.suffix_search_only {
                target2.push(name.clone());
            }
            ret = self.path_search_list_generic(&path, &target2, true, all);
        }

        if ret.is_empty() && must_exist {
            if let Some(r) = self.make_tex(format, &name) {
                ret = vec![r];
            }
        }

        // Windows: convert to long names (on-disk case).
        for r in ret.iter_mut() {
            if r.len() < 512 {
                if let Some(l) = self.getlongpath(r, 500) {
                    *r = l;
                }
            }
            if !all {
                break;
            }
        }
        ret
    }

    /// tex-file.c `kpathsea_open_file`: find NAME (with `must_exist`) and
    /// open it; on failure print the C message and exit.
    pub fn open_file(&mut self, name: impl AsRef<[u8]>, format: FileFormat) -> (Vec<u8>, std::fs::File) {
        let name = name.as_ref();
        match self.find_file(name, format, true) {
            Some(full) => match std::fs::File::open(self.to_os_path(&full)) {
                Ok(f) => (full, f),
                Err(e) => {
                    self.perror(&full, &e);
                    std::process::exit(1);
                }
            },
            None => {
                let t = self.format_info[format as usize].type_.unwrap_or("");
                let m = bcat!(t, b" file `", name, b"' not found");
                self.fatal(&m);
            }
        }
    }

    /// The mktex settings of FORMAT: (program, enabled, enable level).
    pub fn program_enabled(&self, format: FileFormat) -> (Option<Vec<u8>>, bool, SrcLevel) {
        let f = &self.format_info[format as usize];
        (f.program.clone(), f.program_enabled_p, f.program_enable_level)
    }

    /// tex-file.c `kpathsea_init_fallback_resolutions`.
    pub fn init_fallback_resolutions(&mut self, envvar: &[u8]) {
        let size_var: Vec<u8> = if self.env.is_set(envvar) { envvar.to_vec() } else { b"TEXSIZES".to_vec() };
        let size_str = self.env.get(&size_var);
        let default_sizes = self.fallback_resolutions_string.clone().unwrap_or_default();
        let size_list = expand_default(size_str.as_deref(), &default_sizes);
        let mut sizes: Vec<u32> = Vec::new();
        let elts: Vec<Vec<u8>> = self.path_elements(&size_list).iter().map(|e| e.to_vec()).collect();
        for size in elts {
            if size.is_empty() {
                continue;
            }
            let s = atoi(&size) as u32;
            if !sizes.is_empty() && s < *sizes.last().unwrap() {
                let m = bcat!(b"kpathsea: last resort size ", &size, b" not in ascending order; ignored");
                self.warning(&m);
            } else {
                sizes.push(s);
            }
        }
        self.fallback_resolutions = Some(sizes);
    }
}

/// tex-file.c `remove_dbonly`: drop `!!` at the start of elements.
fn remove_dbonly(path: &[u8]) -> Vec<u8> {
    let mut ret = Vec::with_capacity(path.len());
    let mut new_elt = true;
    let mut p = 0;
    while p < path.len() {
        if new_elt && path[p] == b'!' && at(path, p + 1) == b'!' {
            p += 2;
        } else {
            new_elt = path[p] == ENV_SEP;
            ret.push(path[p]);
            p += 1;
        }
    }
    ret
}
