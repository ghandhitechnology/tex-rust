//! Hand-written runtime: everything web2c and pdfTeX implement in C, ported to Rust.

pub mod alloc;
pub mod dump;
pub mod fast;
pub mod fxhash;
pub mod fast_id;
pub mod fast_macro;
pub mod fast_macro_gen;
pub mod io;
pub mod kpse;
pub mod md5;
pub mod pdf;
pub mod phase;
pub mod sampler;
#[cfg(feature = "prof")]
pub mod prof;
pub mod ptr;
pub mod stubs;
pub mod synctex;
pub mod texmfmp;
pub mod time;
pub mod words;

pub use io::{bytes_to_string, FileRef};
pub use ptr::*;
pub use texmfmp::{w2c_abs_real, w2c_fabs, w2c_strcpy, zround};
pub use words::*;

/// State of the C side of the program (texmfmp.c / openclose.c / pdfTeX's C library).
pub struct Runtime {
    pub files: io::FileTable,
    pub argv: Vec<String>,
    pub optind: usize,
    pub argv_consumed: bool,
    pub user_progname: Option<String>,
    pub user_cnf_lines: Vec<String>,
    pub c_job_name: Option<String>,
    pub output_directory: Option<String>,
    pub default_translate_filename: Option<String>,
    pub cmdlist: Option<Vec<String>>,
    pub recorder: std::rc::Rc<std::cell::RefCell<io::Recorder>>,
    pub fullnameoffile: Option<Vec<u8>>,
    pub last_source_name: Option<String>,
    pub last_lineno: i32,
    pub srcspecialsoption: bool,
    pub kpathsea_debug: u32,
    pub start_time: i64,
    pub start_time_set: bool,
    pub sde_set: bool,
    pub fsd_set: bool,
    pub start_time_str: String,
    pub cstrbuf: Ptr<u8>,
    pub dump_file: FileRef,
    /// in-memory read position of the format being loaded (rt/dump.rs)
    pub fmt_cursor: Option<dump::FmtCursor>,
    /// auxiliary index for id_lookup (rt/fast_id.rs)
    pub csindex: fast_id::CsIndex,
    /// macro parameter list bookkeeping (rt/fast_macro.rs): nodes in the parameter being
    /// built, (tail, length) of each pstack entry, and of each param_stack entry
    pub pcount: i32,
    pub pstack_meta: [(i32, i32); 10],
    pub pmeta: Vec<(i32, i32)>,
    /// lazily initialized hash_extra part of eqtb (rt/fast.rs eqtb_lazy_from)
    pub eqtb_template: MemoryWord,
    pub eqtb_ready: i32,
    pub eqtb_lazy_hi: i32,
    /// parallel compression of PDF streams (rt/pdf/parzip.rs)
    pub parzip: pdf::parzip::ParZip,
    /// divisor and reciprocal for rt/fast.rs udivrem
    pub divcache: [(u32, u64); 8],
    /// some non-frozen control sequence has (had) a meaning >= outer_call (rt/fast_macro.rs)
    pub user_outer: bool,
    /// memoized adv_char_width results per font and character (rt/fast.rs)
    pub advcache: Vec<Option<Box<[(i32, i32, i32, i32); 256]>>>,
    pub pdfu: pdf::utils::UtilsState,
    pub fonts: pdf::fonts::FontState,
    pub synctex: synctex::SynctexState,
    pub core: pdf::core::CoreState,
    pub dump_offset: u64,
    pub zip: pdf::writezip::ZipState,
    pub images: pdf::images::ImageState,
}

impl Default for Runtime {
    fn default() -> Self {
        Runtime {
            files: io::FileTable::default(),
            argv: Vec::new(),
            optind: 1,
            argv_consumed: false,
            user_progname: None,
            user_cnf_lines: Vec::new(),
            c_job_name: None,
            output_directory: None,
            default_translate_filename: None,
            cmdlist: None,
            recorder: Default::default(),
            fullnameoffile: None,
            last_source_name: None,
            last_lineno: 0,
            srcspecialsoption: false,
            kpathsea_debug: 0,
            start_time: 0,
            start_time_set: false,
            sde_set: false,
            fsd_set: false,
            start_time_str: String::new(),
            cstrbuf: Ptr::null(),
            dump_file: FileRef::NULL,
            fmt_cursor: None,
            csindex: Default::default(),
            pcount: 0,
            pstack_meta: [(-268435455, 0); 10],
            pmeta: Vec::new(),
            eqtb_template: MemoryWord::default(),
            eqtb_ready: i32::MAX,
            eqtb_lazy_hi: -1,
            parzip: Default::default(),
            divcache: [(0, 0); 8],
            user_outer: false,
            advcache: Vec::new(),
            pdfu: Default::default(),
            fonts: Default::default(),
            synctex: Default::default(),
            core: Default::default(),
            dump_offset: 0,
            zip: Default::default(),
            images: Default::default(),
        }
    }
}

impl Runtime {
    pub fn kpse(&mut self) -> &mut kpse::Kpse {
        self.files.kpse.as_mut().expect("kpathsea not initialized")
    }
    pub fn invocation_name(&mut self) -> String {
        self.kpse().invocation_name().to_string()
    }
}
