//! Path searching through texrs-kpathsea (a faithful pure-Rust port of kpathsea 6.4.2).
//! This module only adapts it to the engine: kpathsea format numbers as the Pascal uses
//! them (i32, kpathsea/types.h order), and byte-string file names throughout.

pub use texrs_kpathsea::{FileFormat, SrcLevel};

// kpathsea/types.h kpse_file_format_type
pub const PK_FORMAT: i32 = 1;
pub const TFM_FORMAT: i32 = 3;
pub const CNF_FORMAT: i32 = 8;
pub const FMT_FORMAT: i32 = 10;
pub const FONTMAP_FORMAT: i32 = 11;
pub const OCP_FORMAT: i32 = 19;
pub const OFM_FORMAT: i32 = 20;
pub const OVF_FORMAT: i32 = 23;
pub const PICT_FORMAT: i32 = 25;
pub const TEX_FORMAT: i32 = 26;
pub const TEXPOOL_FORMAT: i32 = 28;
pub const TYPE1_FORMAT: i32 = 32;
pub const VF_FORMAT: i32 = 33;
pub const TRUETYPE_FORMAT: i32 = 36;
pub const WEB2C_FORMAT: i32 = 38;
pub const MISCFONTS_FORMAT: i32 = 41;
pub const ENC_FORMAT: i32 = 44;
pub const CMAP_FORMAT: i32 = 45;
pub const SFD_FORMAT: i32 = 46;
pub const OPENTYPE_FORMAT: i32 = 47;
pub const PDFTEX_CONFIG_FORMAT: i32 = 48;

pub const SRC_COMPILE: i32 = 1;
pub const SRC_CMDLINE: i32 = 6;

fn fmt(i: i32) -> FileFormat {
    FileFormat::from_index(i as usize).unwrap_or_else(|| panic!("bad kpathsea format {}", i))
}

fn level(i: i32) -> SrcLevel {
    match i {
        0 => SrcLevel::Implicit,
        1 => SrcLevel::Compile,
        2 => SrcLevel::TexmfCnf,
        3 => SrcLevel::ClientCnf,
        4 => SrcLevel::Env,
        5 => SrcLevel::X,
        _ => SrcLevel::Cmdline,
    }
}

fn s(b: Vec<u8>) -> String {
    crate::rt::bytes_to_string(&b)
}

/// kpathsea absolute.c kpathsea_absolute_p.
pub fn absolute_p(name: impl AsRef<[u8]>, relative_ok: bool) -> bool {
    texrs_kpathsea::Kpse::absolute_p(name.as_ref(), relative_ok)
}

pub struct Kpse {
    pub k: texrs_kpathsea::Kpse,
}

impl Kpse {
    /// kpse_set_program_name(argv[0], NULL), then texmfmp.c's command_line_encoding setup.
    pub fn new(argv0: &str) -> Kpse {
        let mut k = texrs_kpathsea::Kpse::new(argv0.as_bytes(), None);
        k.init_command_line_encoding();
        Kpse { k }
    }

    pub fn invocation_name(&self) -> String {
        s(self.k.invocation_name().to_vec())
    }
    pub fn program_name(&self) -> String {
        s(self.k.program_name().to_vec())
    }
    pub fn reset_program_name(&mut self, p: &str) {
        self.k.reset_program_name(p);
    }
    pub fn add_cnf_line(&mut self, line: &str) {
        self.k.add_cnf_line(line.as_bytes());
    }
    pub fn xputenv(&mut self, var: &str, value: &str) {
        self.k.xputenv(var.as_bytes(), value.as_bytes());
    }
    pub fn getenv(&mut self, var: &str) -> Option<String> {
        self.k.getenv(var.as_bytes()).map(s)
    }
    pub fn var_value(&mut self, name: &str) -> Option<String> {
        self.k.var_value(name.as_bytes()).map(s)
    }
    pub fn var_expand(&mut self, v: &str) -> String {
        s(self.k.var_expand(v.as_bytes()))
    }
    /// kpse_find_file: the found name as bytes (what C stores in nameoffile).
    pub fn find_file(&mut self, name: impl AsRef<[u8]>, format: i32, must_exist: bool) -> Option<Vec<u8>> {
        self.k.find_file(name.as_ref(), fmt(format), must_exist)
    }
    /// kpse_find_file_generic(name, format, must_exist, all=true)
    pub fn find_file_all(&mut self, name: impl AsRef<[u8]>, format: i32, must_exist: bool) -> Vec<Vec<u8>> {
        self.k.find_file_generic(name.as_ref(), fmt(format), must_exist, true)
    }
    pub fn find_glyph(&mut self, font: impl AsRef<[u8]>, dpi: u32, format: i32) -> Option<(Vec<u8>, texrs_kpathsea::GlyphFileInfo)> {
        self.k.find_glyph(font.as_ref(), dpi, fmt(format))
    }
    pub fn in_name_ok(&mut self, name: impl AsRef<[u8]>) -> bool {
        self.k.in_name_ok(name.as_ref())
    }
    pub fn out_name_ok(&mut self, name: impl AsRef<[u8]>) -> bool {
        self.k.out_name_ok(name.as_ref())
    }
    pub fn set_program_enabled(&mut self, format: i32, value: bool, lvl: i32) {
        self.k.set_program_enabled(fmt(format), value, level(lvl));
    }
    pub fn maketex_option(&mut self, fmtname: &str, value: bool) {
        self.k.maketex_option(fmtname.as_bytes(), value);
    }
    pub fn init_prog(&mut self, prefix: &str, dpi: i32, mode: Option<&str>, fallback: Option<&str>) {
        self.k.init_prog(prefix, dpi as u32, mode, fallback);
    }
    pub fn set_debug(&mut self, flags: u32) {
        self.k.set_debug(flags);
    }
    /// The OS path for a TeX file name (kpathsea's file-system code page rules).
    pub fn os_path(&self, name: &[u8]) -> std::path::PathBuf {
        self.k.to_os_path(name)
    }
    /// Environment changes made through xputenv, for \write18 / pipe children.
    pub fn env_changes(&self) -> Vec<(Vec<u8>, Option<Vec<u8>>)> {
        self.k.env_changes()
    }
    pub fn stderr_write(&mut self, bytes: &[u8]) {
        self.k.stderr_write(bytes);
    }
    pub fn flush_stderr(&mut self) {
        self.k.flush_stderr();
    }
}
