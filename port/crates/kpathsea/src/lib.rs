//! # texrs-kpathsea
//!
//! A pure-Rust port of **kpathsea 6.4.2**, the path-searching library of
//! TeX Live 2026, as built for Windows (`kpathsealibw64.dll`).  It is meant
//! to be used by a Rust port of pdfTeX and must find exactly the same
//! files, read exactly the same `texmf.cnf` variables and accept exactly
//! the same output names as the C library.  Every function notes the C
//! function it mirrors.
//!
//! Strings are byte strings (`&[u8]` / `Vec<u8>`), exactly like the C
//! `char *` they replace: TeX file names are bytes, and on Windows the
//! encoding is the ANSI code page or UTF-8 depending on
//! `command_line_encoding` (see [`Kpse::init_command_line_encoding`]).
//! `&str`, `String` and `Vec<u8>` arguments are all accepted through
//! `AsRef<[u8]>`.
//!
//! ```no_run
//! use texrs_kpathsea::{Kpse, FileFormat};
//! let mut k = Kpse::new("pdftex", Some("pdflatex"));
//! let tfm = k.find_file("cmr10", FileFormat::Tfm, true);
//! ```

#[macro_use]
pub(crate) mod util;
pub(crate) mod cnf;
pub(crate) mod db;
pub(crate) mod debug;
pub(crate) mod eltdirs;
pub(crate) mod env;
pub(crate) mod expand;
pub(crate) mod fontmap;
pub(crate) mod glyph;
pub(crate) mod hash;
pub(crate) mod make;
pub(crate) mod nameok;
pub(crate) mod pathsearch;
pub(crate) mod progname;
pub(crate) mod sys;
pub(crate) mod texfile;
pub(crate) mod variable;

#[cfg(test)]
mod tests;
mod dbcache;

use std::collections::HashMap;

pub use debug::{
    KPSE_DEBUG_EXPAND, KPSE_DEBUG_FOPEN, KPSE_DEBUG_HASH, KPSE_DEBUG_PATHS, KPSE_DEBUG_SEARCH,
    KPSE_DEBUG_STAT, KPSE_DEBUG_VARS,
};
pub use glyph::{GlyphFileInfo, GlyphSource};
pub use texfile::{FileFormat, SrcLevel, LAST_FORMAT};

/// `kpathsea_version_string`.
pub const VERSION_STRING: &str = "kpathsea version 6.4.2";
/// `kpathsea_bug_address`.
pub const BUG_ADDRESS: &str = "Email bug reports to tex-k@tug.org.\n";

/// Recorder callback type (`p_record_input` / `p_record_output`).
pub type RecordFn = Box<dyn FnMut(&[u8])>;

/// One kpathsea instance (`struct kpathsea_instance`).
pub struct Kpse {
    // xputenv.c / getenv
    pub(crate) env: env::Env,
    // debug.c
    pub(crate) debug: u32,
    /// True when this instance plays the role of the C global `kpse_def`
    /// (as in pdfTeX), false for a private instance (as in kpsewhich).
    /// Some C tracing (hash_lookup, fopen) always goes to `kpse_def`.
    pub(crate) compat_def: bool,
    pub(crate) err: debug::ErrSink,
    // cnf.c
    pub(crate) record_input: Option<RecordFn>,
    pub(crate) record_output: Option<RecordFn>,
    pub(crate) cnf_hash: Option<hash::CHash>,
    pub(crate) doing_cnf_init: bool,
    // db.c
    pub(crate) db: Option<db::Db>,
    pub(crate) alias_db: Option<hash::CHash>,
    pub(crate) db_dir_list: Vec<Vec<u8>>,
    // dir.c
    pub(crate) link_table: HashMap<Vec<u8>, i64>,
    // elt-dirs.c
    pub(crate) elt_cache: Vec<eltdirs::DirList>,
    pub(crate) elt_cache_index: HashMap<Vec<u8>, usize>,
    // fontmap.c
    pub(crate) map: Option<hash::CHash>,
    pub(crate) map_path: Option<Vec<u8>>,
    // pathsearch.c
    pub(crate) followup_search: bool,
    pub(crate) log_file: Option<std::fs::File>,
    pub(crate) log_opened: bool,
    // progname.c
    pub(crate) invocation_name: Vec<u8>,
    pub(crate) invocation_short_name: Vec<u8>,
    pub(crate) program_name: Vec<u8>,
    // tex-file.c
    pub(crate) fallback_font: Option<Vec<u8>>,
    pub(crate) fallback_resolutions_string: Option<Vec<u8>>,
    pub(crate) fallback_resolutions: Option<Vec<u32>>,
    pub(crate) format_info: Vec<texfile::FormatInfo>,
    // tex-make.c
    pub(crate) make_tex_discard_errors: bool,
    pub(crate) missfont: Option<std::fs::File>,
    // variable.c
    pub(crate) expansions: Vec<(Vec<u8>, bool)>,
    // win32
    pub(crate) suffixlist: Vec<Vec<u8>>,
    pub(crate) file_system_codepage: u32,
    pub(crate) win32_codepage: u32,
    pub(crate) is_cp932_system: u32,
    pub(crate) passwd_name: Vec<u8>,
    pub(crate) passwd_dir: Vec<u8>,
    // Performance caches that cannot change results (see README).
    pub(crate) longpath_dir_cache: HashMap<Vec<u8>, Vec<u8>>,
}

impl Kpse {
    /// An uninitialised instance (`kpathsea_new`).  Call
    /// [`Kpse::set_program_name`] before anything else.
    pub fn blank() -> Kpse {
        let acp = sys::acp();
        Kpse {
            env: env::Env::new(acp),
            debug: 0,
            compat_def: true,
            err: debug::ErrSink::stderr(),
            record_input: None,
            record_output: None,
            cnf_hash: None,
            doing_cnf_init: false,
            db: None,
            alias_db: None,
            db_dir_list: Vec::new(),
            link_table: HashMap::new(),
            elt_cache: Vec::new(),
            elt_cache_index: HashMap::new(),
            map: None,
            map_path: None,
            followup_search: false,
            log_file: None,
            log_opened: false,
            invocation_name: Vec::new(),
            invocation_short_name: Vec::new(),
            program_name: Vec::new(),
            fallback_font: None,
            fallback_resolutions_string: None,
            fallback_resolutions: None,
            format_info: (0..LAST_FORMAT).map(|_| texfile::FormatInfo::default()).collect(),
            make_tex_discard_errors: false,
            missfont: None,
            expansions: Vec::new(),
            suffixlist: Vec::new(),
            file_system_codepage: 0,
            win32_codepage: 0,
            is_cp932_system: 0,
            passwd_name: Vec::new(),
            passwd_dir: Vec::new(),
            longpath_dir_cache: HashMap::new(),
        }
    }

    /// `kpse_set_program_name (argv0, progname)` on a fresh instance.
    ///
    /// The executable location (for `SELFAUTOLOC` etc.) is found like the
    /// Windows C code does (`SearchPath` on `argv0`, then long-name
    /// conversion), except that the environment variable
    /// `TEXRS_SELFAUTOLOC` (a directory), if set, overrides the directory.
    pub fn new(argv0: impl AsRef<[u8]>, progname: Option<&str>) -> Kpse {
        let mut k = Kpse::blank();
        k.set_program_name(argv0.as_ref(), progname.map(|s| s.as_bytes()));
        k
    }

    /// Like [`Kpse::new`] but with an explicit executable path instead of
    /// searching for `argv0` (the directory part becomes `SELFAUTOLOC`).
    pub fn with_exe_path(exe_path: impl AsRef<[u8]>, progname: Option<&str>) -> Kpse {
        let mut k = Kpse::blank();
        k.set_program_name_from_path(exe_path.as_ref(), progname.map(|s| s.as_bytes()));
        k
    }

    /// Select whether this instance behaves like the C global `kpse_def`
    /// (pdfTeX; the default) or a private instance (kpsewhich).  This only
    /// affects which debug traces are printed.
    pub fn set_compat_def(&mut self, v: bool) {
        self.compat_def = v;
    }

    /// `kpse_program_name`.
    pub fn program_name(&self) -> &[u8] {
        &self.program_name
    }

    /// `kpse_invocation_name`.
    pub fn invocation_name(&self) -> &[u8] {
        &self.invocation_name
    }

    /// `kpse_invocation_short_name`.
    pub fn invocation_short_name(&self) -> &[u8] {
        &self.invocation_short_name
    }

    /// `kpathsea_debug |= flags`.
    pub fn set_debug(&mut self, flags: u32) {
        self.debug |= flags;
    }

    /// Current `kpathsea_debug` value.
    pub fn debug_flags(&self) -> u32 {
        self.debug
    }

    /// `kpse_record_input = f`.
    pub fn set_record_input(&mut self, f: Option<RecordFn>) {
        self.record_input = f;
    }

    /// `kpse_record_output = f`.
    pub fn set_record_output(&mut self, f: Option<RecordFn>) {
        self.record_output = f;
    }

    /// Call the output recorder, if any (for clients mirroring
    /// `recorder_record_output`).
    pub fn record_output(&mut self, name: &[u8]) {
        if let Some(f) = self.record_output.as_mut() {
            f(name);
        }
    }

    /// `kpse_make_tex_discard_errors = v`.
    pub fn set_make_tex_discard_errors(&mut self, v: bool) {
        self.make_tex_discard_errors = v;
    }

    /// `kpse_fallback_resolutions_string = s` (before `init_prog`).
    pub fn set_fallback_resolutions_string(&mut self, s: Option<&[u8]>) {
        self.fallback_resolutions_string = s.map(|s| s.to_vec());
    }

    /// `kpse_fallback_font = s`.
    pub fn set_fallback_font(&mut self, s: Option<&[u8]>) {
        self.fallback_font = s.map(|s| s.to_vec());
    }

    /// The `File_system_codepage` (65001 once UTF-8 command lines are on).
    pub fn file_system_codepage(&self) -> u32 {
        self.file_system_codepage
    }

    /// Set `File_system_codepage` directly.
    pub fn set_file_system_codepage(&mut self, cp: u32) {
        self.file_system_codepage = cp;
    }

    /// `getenv` as the C runtime would see it (including kpathsea's own
    /// `xputenv` changes).
    pub fn getenv(&mut self, name: impl AsRef<[u8]>) -> Option<Vec<u8>> {
        self.env.get(name.as_ref())
    }

    /// `kpathsea_xputenv`: set an environment variable for kpathsea and for
    /// any child process it starts.  An empty value removes the variable
    /// (MSVC `putenv` semantics).
    pub fn xputenv(&mut self, var: impl AsRef<[u8]>, value: impl AsRef<[u8]>) {
        self.env.put(var.as_ref(), value.as_ref());
    }

    /// `kpathsea_xputenv_int`.
    pub fn xputenv_int(&mut self, var: impl AsRef<[u8]>, value: i32) {
        self.env.put(var.as_ref(), value.to_string().as_bytes());
    }

    /// Environment changes made through `xputenv`, as (name, value or
    /// removal) pairs, for clients that spawn processes themselves (e.g.
    /// `\write18`).
    pub fn env_changes(&self) -> Vec<(Vec<u8>, Option<Vec<u8>>)> {
        self.env.changes().map(|(k, v)| (k.to_vec(), v.map(|v| v.to_vec()))).collect()
    }

    /// `kpse_absolute_p (name, relative_ok)`.
    pub fn absolute_p(name: impl AsRef<[u8]>, relative_ok: bool) -> bool {
        util::absolute_p(name.as_ref(), relative_ok)
    }

    /// `kpse_program_basename`: basename with a trailing `.exe` removed.
    pub fn program_basename(argv0: impl AsRef<[u8]>) -> Vec<u8> {
        let base = util::xbasename(argv0.as_ref()).to_vec();
        if let Some(d) = base.iter().rposition(|&c| c == b'.') {
            if base[d..].eq_ignore_ascii_case(b".exe") {
                return base[..d].to_vec();
            }
        }
        base
    }

    /// `find_suffix`.
    pub fn find_suffix(name: &[u8]) -> Option<&[u8]> {
        util::find_suffix(name).map(|i| &name[i..])
    }

    /// `remove_suffix`.
    pub fn remove_suffix(name: &[u8]) -> Vec<u8> {
        util::remove_suffix(name)
    }

    /// `kpse_magstep_fix (dpi, bdpi, &m)`: returns (true dpi, magstep).
    pub fn magstep_fix(dpi: u32, bdpi: u32) -> (u32, i32) {
        glyph::magstep_fix(dpi, bdpi)
    }

    /// `kpse_bitmap_tolerance (dpi1, dpi2)`.
    pub fn bitmap_tolerance(dpi1: f64, dpi2: f64) -> bool {
        glyph::bitmap_tolerance(dpi1, dpi2)
    }

    /// Convert a byte string from this instance's file-system code page to
    /// an OS path (for opening files that kpathsea returned).
    pub fn to_os_path(&self, name: &[u8]) -> std::path::PathBuf {
        std::path::PathBuf::from(sys::bytes_to_os(self.fs_cp(), name))
    }

    /// Convert an OS string to bytes the way the C program would see it
    /// (UTF-8 once `command_line_encoding` is in effect, ANSI otherwise).
    pub fn os_to_bytes(&self, s: &std::ffi::OsStr) -> Vec<u8> {
        sys::os_to_bytes(self.fs_cp(), s)
    }

    pub(crate) fn fs_cp(&self) -> u32 {
        if self.file_system_codepage == 0 {
            sys::acp()
        } else {
            self.file_system_codepage
        }
    }

    /// `kpathsea_IS_KANJI`.
    #[inline]
    pub(crate) fn is_kanji(&self, s: &[u8], i: usize) -> bool {
        if self.is_cp932_system == 0 {
            return false;
        }
        let c = util::at(s, i);
        let c2 = util::at(s, i + 1);
        let k1 = match self.is_cp932_system {
            932 => (0x81..=0x9f).contains(&c) || (0xe0..=0xfc).contains(&c),
            936 => (0x81..=0xfe).contains(&c),
            950 => (0xa1..=0xc6).contains(&c) || (0xc9..=0xf9).contains(&c),
            _ => false,
        };
        let k2 = match self.is_cp932_system {
            932 => (0x40..=0xfc).contains(&c2) && c2 != 0x7f,
            936 => (0x40..=0xfe).contains(&c2) && c2 != 0x7f,
            950 => (0x40..=0x7e).contains(&c2) || (0xa1..=0xfe).contains(&c2),
            _ => false,
        };
        k1 && k2
    }

    /// Slashify in place (`\` -> `/`), skipping double-byte characters.
    pub(crate) fn slashify(&self, s: &mut [u8]) {
        let mut i = 0;
        while i < s.len() {
            if s[i] == b'\\' {
                s[i] = b'/';
            } else if self.is_kanji(s, i) {
                i += 1;
            }
            i += 1;
        }
    }
}

/// Lossy conversion helper for callers that want `String`s.
pub fn lossy(v: &[u8]) -> String {
    String::from_utf8_lossy(v).into_owned()
}
