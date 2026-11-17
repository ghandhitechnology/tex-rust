//! SyncTeX writer: texk/web2c/synctexdir/synctex.c (TeX Live 2026) as compiled for pdfTeX
//! on Windows:
//! * synctex-pdftex.h: `SYNCTEX_OUTPUT` and `SYNCTEX_OFFSET_IS_PDF` follow `pdfoutputvalue`,
//!   `SYNCTEX_SUPPORT_PDF_FORM` is on (`<`, `>` and `f` records), `SYNCTEX_interaction` is
//!   `interaction`;
//! * the node layout macros keep synctex.c's defaults (`synchronization_field_size` 2,
//!   `box_node_size` 9, `rule_node_size` 6, `medium_node_size` 4);
//! * `W32UPTEXSYNCTEX`: input file names go through `chgto_oem`, files are opened/removed/
//!   renamed through the `fsyscp_*` wrappers (UTF-8 names), and the final message is
//!   `"\nSyncTeX written on %s.\n"` (trailing newline);
//! * texmfmp.c `generic_synctex_get_current_name` with `W32USYNCTEX`.
//!
//! `.synctex` output (negative `-synctex`) is a `FOPEN_W_MODE` ("wb") stdio stream: it lives in
//! the FileTable, so it is flushed by `uexit` like C's `exit()` flushes stdio buffers.
//! `.synctex.gz` output goes through texrs_zlib's `gzprintf` (zlib 1.3.2 buffering); like a
//! C gzFile, whatever is still buffered is lost if the program exits without closing it.

use std::fs::File;

use crate::gen::globals::Globals;
use crate::rt::io::Handle;
use crate::rt::*;

use ::texrs_zlib::gz::GzWriter;

const SYNCTEX_VERSION: i32 = 1;
const SYNCTEX_UNIT_FACTOR: i32 = 1;
const SYNCTEX_NO_OPTION: i32 = i32::MAX;
const SYNCTEX_NOERR: i32 = 0;

// synctex.c defaults (not overridden by synctex-pdftex.h)
const SYNCHRONIZATION_FIELD_SIZE: i32 = 2;
const BOX_NODE_SIZE: i32 = 7 + SYNCHRONIZATION_FIELD_SIZE;
const SMALL_NODE_SIZE: i32 = 2;
const MEDIUM_NODE_SIZE: i32 = SMALL_NODE_SIZE + SYNCHRONIZATION_FIELD_SIZE;
const RULE_NODE_SIZE: i32 = 4 + SYNCHRONIZATION_FIELD_SIZE;
const GLUE_NODE_SIZE: i32 = MEDIUM_NODE_SIZE;
const KERN_NODE_SIZE: i32 = MEDIUM_NODE_SIZE;
const MATH_NODE_SIZE: i32 = MEDIUM_NODE_SIZE;
const WIDTH_OFFSET: i32 = 1;
const DEPTH_OFFSET: i32 = 2;
const HEIGHT_OFFSET: i32 = 3;
const RULE_NODE: i32 = 2;
const GLUE_NODE: i32 = 10;
const KERN_NODE: i32 = 11;

const SYNCTEX_SUFFIX: &[u8] = b".synctex";
const SYNCTEX_SUFFIX_GZ: &[u8] = b".gz";
const SYNCTEX_SUFFIX_BUSY: &[u8] = b"(busy)";

/// `synctex_ctxt.file` with its `fprintf` (fprintf or gzprintf).
enum SyncFile {
    /// `fopen(name, FOPEN_W_MODE)`: an output slot of the FileTable.
    Plain(FileRef),
    /// `gzopen(name, FOPEN_WBIN_MODE)`
    Gz(Box<GzWriter<File>>),
}

/// `synctex_ctxt.recorder`: the only recorder synctex.c ever stores is
/// `synctex_record_node_kern`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Recorder {
    Kern,
}

/// `synctex_ctxt` plus the static `synctex_tag_counter` of `synctexstartinput`.
pub struct SynctexState {
    file: Option<SyncFile>,
    busy_name: Option<Vec<u8>>,
    root_name: Option<Vec<u8>>,
    count: i32,
    node: i32,
    recorder: Option<Recorder>,
    tag: i32,
    line: i32,
    curh: i32,
    curv: i32,
    magnification: i32,
    unit: i32,
    total_length: i32,
    options: i32,
    lastv: i32,
    form_depth: i32,
    option_read: bool,
    content_ready: bool,
    off: bool,
    no_gz: bool,
    not_void: bool,
    warn: bool,
    quoted: bool,
    output_p: bool,
    tag_counter: u32,
}

impl Default for SynctexState {
    fn default() -> Self {
        SynctexState {
            file: None,
            busy_name: None,
            root_name: None,
            count: 0,
            node: 0,
            recorder: None,
            tag: 0,
            line: 0,
            curh: 0,
            curv: 0,
            magnification: 0,
            unit: 0,
            total_length: 0,
            options: 0,
            lastv: -1, // "last_v_recorded is initialized to -1"
            form_depth: 0,
            option_read: false,
            content_ready: false,
            off: false,
            no_gz: false,
            not_void: false,
            warn: false,
            quoted: false,
            output_p: false,
            tag_counter: 0,
        }
    }
}

/// C string semantics: the bytes up to the first NUL.
fn c_str(mut b: Vec<u8>) -> Vec<u8> {
    if let Some(i) = b.iter().position(|&c| c == 0) {
        b.truncate(i);
    }
    b
}

/// A UTF-8 file name (`file_system_codepage` is CP_UTF8 for the fsyscp_* wrappers) as a path.
fn fs_name(b: &[u8]) -> String {
    bytes_to_string(b)
}

/// fsyscp_remove: true on success; the error tells whether C's errno would be EACCES.
fn fsyscp_remove(name: &[u8]) -> Result<(), bool> {
    match std::fs::remove_file(fs_name(name)) {
        Ok(()) => Ok(()),
        Err(e) => Err(is_eacces(&e)),
    }
}

/// MSVCRT's _dosmaperr: the Windows errors that become EACCES.
fn is_eacces(e: &std::io::Error) -> bool {
    match e.raw_os_error() {
        Some(c) => matches!(c, 5 | 16 | 19..=36 | 65 | 82 | 83 | 108 | 132 | 158 | 167),
        None => e.kind() == std::io::ErrorKind::PermissionDenied,
    }
}

/// fsyscp_rename (_wrename): fails when the target exists (MoveFileW semantics).
fn fsyscp_rename(from: &[u8], to: &[u8]) -> bool {
    let (from, to) = (fs_name(from), fs_name(to));
    if std::fs::symlink_metadata(&to).is_ok() {
        return false;
    }
    std::fs::rename(from, to).is_ok()
}

/// gzprintf of already formatted bytes.
fn gz_print(g: &mut GzWriter<File>, s: &[u8]) -> i32 {
    match std::str::from_utf8(s) {
        Ok(t) => g.gzprintf(t),
        // Non-UTF-8 bytes (file names converted to an ANSI code page): gzwrite feeds deflate
        // the same bytes (gzprintf would need a byte-string entry point in texrs_zlib).
        Err(_) => g.gzwrite(s),
    }
}

/// Windows code page conversions (kpathsea get_wstring_from_mbstring /
/// get_mbstring_from_wstring) used by chgto_oem and generic_synctex_get_current_name.
#[cfg(windows)]
mod cp {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetACP() -> u32;
        fn GetOEMCP() -> u32;
        fn AreFileApisANSI() -> i32;
        fn MultiByteToWideChar(cp: u32, flags: u32, s: *const u8, n: i32, w: *mut u16, wn: i32) -> i32;
        fn WideCharToMultiByte(
            cp: u32,
            flags: u32,
            w: *const u16,
            wn: i32,
            s: *mut u8,
            n: i32,
            def: *const u8,
            used: *mut i32,
        ) -> i32;
    }

    /// `AreFileApisANSI() ? GetACP() : GetOEMCP()`
    pub fn file_apis_cp() -> u32 {
        unsafe {
            if AreFileApisANSI() != 0 {
                GetACP()
            } else {
                GetOEMCP()
            }
        }
    }

    pub fn mb_to_wide(cp: u32, s: &[u8]) -> Vec<u16> {
        if s.is_empty() {
            return Vec::new();
        }
        unsafe {
            let n = MultiByteToWideChar(cp, 0, s.as_ptr(), s.len() as i32, std::ptr::null_mut(), 0);
            if n <= 0 {
                return Vec::new();
            }
            let mut w = vec![0u16; n as usize];
            MultiByteToWideChar(cp, 0, s.as_ptr(), s.len() as i32, w.as_mut_ptr(), n);
            w
        }
    }

    pub fn wide_to_mb(cp: u32, w: &[u16]) -> Vec<u8> {
        if w.is_empty() {
            return Vec::new();
        }
        unsafe {
            let n = WideCharToMultiByte(
                cp,
                0,
                w.as_ptr(),
                w.len() as i32,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
                std::ptr::null_mut(),
            );
            if n <= 0 {
                return Vec::new();
            }
            let mut s = vec![0u8; n as usize];
            WideCharToMultiByte(
                cp,
                0,
                w.as_ptr(),
                w.len() as i32,
                s.as_mut_ptr(),
                n,
                std::ptr::null(),
                std::ptr::null_mut(),
            );
            s
        }
    }
}

#[cfg(not(windows))]
mod cp {
    pub fn file_apis_cp() -> u32 {
        65001
    }
    pub fn mb_to_wide(_cp: u32, s: &[u8]) -> Vec<u16> {
        String::from_utf8_lossy(s).encode_utf16().collect()
    }
    pub fn wide_to_mb(_cp: u32, w: &[u16]) -> Vec<u8> {
        String::from_utf16_lossy(w).into_bytes()
    }
}

const CP_UTF8: u32 = 65001;

/// Convert bytes between two code pages (identity for ASCII, as WideCharToMultiByte is).
fn convert_cp(from: u32, to: u32, s: &[u8]) -> Vec<u8> {
    if s.is_ascii() || from == to {
        return s.to_vec();
    }
    cp::wide_to_mb(to, &cp::mb_to_wide(from, s))
}

impl Globals {
    /// `SYNCTEX_VALUE`: `zeqtb[synctexoffset].cint`
    fn synctex_value(&self) -> i32 {
        ix(self.eqtb, self.synctexoffset).int()
    }
    fn synctex_value_set(&mut self, v: i32) {
        let off = self.synctexoffset;
        ix(self.eqtb, off).set_int(v);
    }

    /// `mem[p].cint`
    #[inline]
    fn sx_mem(&self, p: i32) -> i32 {
        ix(self.mem, p).int()
    }
    /// `SYNCTEX_TAG_MODEL(p, TYPE)` with TYPE's node size.
    #[inline]
    fn sx_tag(&self, p: i32, size: i32) -> i32 {
        self.sx_mem(p + size - SYNCHRONIZATION_FIELD_SIZE)
    }
    /// `SYNCTEX_LINE_MODEL(p, TYPE)`
    #[inline]
    fn sx_line(&self, p: i32, size: i32) -> i32 {
        self.sx_mem(p + size - SYNCHRONIZATION_FIELD_SIZE + 1)
    }
    /// `x UNIT` = `x / synctex_ctxt.unit`
    #[inline]
    fn sx_unit(&self, v: i32) -> i32 {
        v.wrapping_div(self.rt.synctex.unit)
    }

    /// `SYNCTEX_fprintf(SYNCTEX_FILE, ...)`: the number of bytes written, or <= 0 on error.
    fn sx_fprintf(&mut self, s: &[u8]) -> i32 {
        let rt = &mut self.rt;
        match rt.synctex.file.as_mut() {
            // fprintf/gzprintf on a NULL file: no output, an error
            None => -1,
            Some(SyncFile::Plain(f)) => {
                let f = *f;
                rt.files.write(f, s);
                s.len() as i32
            }
            Some(SyncFile::Gz(g)) => gz_print(g, s),
        }
    }

    /// xfclose / gzclose of `SYNCTEX_FILE`, which becomes NULL.
    fn sx_close_file(&mut self) {
        match self.rt.synctex.file.take() {
            Some(SyncFile::Plain(f)) => self.rt.files.close(f),
            Some(SyncFile::Gz(g)) => {
                let _ = g.gzclose();
            }
            None => {}
        }
    }

    /// C `printf` to stdout (the same stream as TeX's terminal output).
    fn sx_printf(&mut self, s: &[u8]) {
        self.rt.files.write(FileRef::STDOUT, s);
    }

    /// `fprintf(stderr, ...)`
    fn sx_eprintf(&mut self, s: &[u8]) {
        self.rt.files.write(FileRef::STDERR, s);
    }

    /// `gettexstring(s)`
    fn sx_gettexstring(&self, s: i32) -> Vec<u8> {
        c_str(self.tex_string(s))
    }

    /// kpathsea `file_system_codepage` (CP_UTF8 when command_line_encoding is utf8/utf-8,
    /// otherwise the file-API code page).
    fn sx_file_system_codepage(&mut self) -> u32 {
        match self.rt.kpse().var_value("command_line_encoding") {
            Some(e) if e == "utf8" || e == "utf-8" => CP_UTF8,
            _ => cp::file_apis_cp(),
        }
    }

    /// synctex.c `chgto_oem` (W32UPTEXSYNCTEX)
    fn sx_chgto_oem(&mut self, src: Vec<u8>) -> Vec<u8> {
        if src.is_ascii() {
            return src;
        }
        let f_codepage = cp::file_apis_cp();
        let fs = self.sx_file_system_codepage();
        if fs == 0 || f_codepage == fs {
            return src;
        }
        convert_cp(fs, f_codepage, &src)
    }

    /// texmfmp.c generic_synctex_get_current_name (W32USYNCTEX)
    pub fn synctex_current_name(&mut self) -> Vec<u8> {
        let Some(full) = self.rt.fullnameoffile.clone() else {
            return Vec::new();
        };
        if kpse::absolute_p(&full, false) {
            return full;
        }
        // xgetcwd: getcwd() in the file-API code page, '\\' -> '/'
        let win32_codepage = cp::file_apis_cp();
        let cwd = std::env::current_dir().map(|p| p.into_os_string()).unwrap_or_default();
        let mut pwd: Vec<u8> = match cwd.to_str() {
            Some(s) if s.is_ascii() => s.as_bytes().to_vec(),
            _ => {
                #[cfg(windows)]
                {
                    use std::os::windows::ffi::OsStrExt;
                    let w: Vec<u16> = cwd.encode_wide().collect();
                    cp::wide_to_mb(win32_codepage, &w)
                }
                #[cfg(not(windows))]
                {
                    cwd.to_string_lossy().into_owned().into_bytes()
                }
            }
        };
        for b in pwd.iter_mut() {
            if *b == b'\\' {
                *b = b'/';
            }
        }
        let fs = self.sx_file_system_codepage();
        if fs != 0 && fs != win32_codepage {
            pwd = convert_cp(win32_codepage, fs, &pwd);
        }
        pwd.push(b'/');
        pwd.extend_from_slice(&full);
        pwd
    }

    // ------------------------------------------------------------------ options

    /// `_synctex_read_command_line_option`
    fn synctex_read_command_line_option(&mut self) {
        if self.rt.synctex.option_read {
            return;
        }
        if self.synctexoption == SYNCTEX_NO_OPTION {
            self.synctex_value_set(0);
        } else if self.synctexoption == 0 {
            self.rt.synctex.off = true;
            self.synctex_value_set(0);
        } else {
            if self.synctexoption < 0 {
                self.rt.synctex.no_gz = true;
                self.rt.synctex.options = self.synctexoption.wrapping_neg();
            } else {
                self.rt.synctex.no_gz = false;
                self.rt.synctex.options = self.synctexoption;
            }
            self.synctexoption |= 1;
            let v = self.synctexoption;
            self.synctex_value_set(v);
        }
        self.rt.synctex.option_read = true;
    }

    fn sx_no_gz_aux_name(&self) -> bool {
        self.rt.synctex.no_gz || (self.rt.synctex.options & 2) != 0
    }
    fn sx_with_forms(&self) -> bool {
        (self.rt.synctex.options & 4) != 0
    }
    fn sx_h_compress(&self) -> bool {
        (self.rt.synctex.options & 8) != 0
    }
    /// `SYNCTEX_SHOULD_COMPRESS_V`
    fn sx_should_compress_v(&self) -> bool {
        self.sx_h_compress() && self.rt.synctex.lastv == self.rt.synctex.curv
    }

    pub fn synctexinitcommand(&mut self) {
        self.synctex_read_command_line_option();
    }

    pub fn synctexabort(&mut self, _log_opened: bool) {
        if self.rt.synctex.file.is_some() {
            self.sx_close_file();
            if let Some(b) = self.rt.synctex.busy_name.take() {
                let _ = fsyscp_remove(&b);
            }
        }
        self.rt.synctex.root_name = None;
        self.rt.synctex.off = true;
    }

    /// `synctex_dot_open`: true when `SYNCTEX_FILE` is (now) open.
    fn synctex_dot_open(&mut self) -> bool {
        if self.rt.synctex.off || self.synctex_value() == 0 {
            return false;
        }
        if self.rt.synctex.file.is_some() {
            return true;
        }
        self.synctex_read_command_line_option();
        let tmp = self.sx_gettexstring(self.jobname);
        let len = tmp.len();
        if len > 0 {
            let mut the_busy_name: Vec<u8> = Vec::new();
            if let Some(od) = self.rt.output_directory.clone() {
                if !kpse::absolute_p(&bytes_to_string(&tmp), false) {
                    self.rt.synctex.output_p = true;
                    the_busy_name.extend_from_slice(od.as_bytes());
                    the_busy_name.push(b'/');
                }
            }
            if tmp[0] == b'"' && tmp[len - 1] == b'"' {
                self.rt.synctex.quoted = true;
                if len > 1 {
                    the_busy_name.extend_from_slice(&tmp[1..len - 1]);
                }
            } else {
                self.rt.synctex.quoted = false;
                the_busy_name.extend_from_slice(&tmp);
            }
            the_busy_name.extend_from_slice(SYNCTEX_SUFFIX);
            the_busy_name.extend_from_slice(SYNCTEX_SUFFIX_BUSY);
            let name = fs_name(&the_busy_name);
            self.rt.synctex.file = if self.rt.synctex.no_gz {
                match File::create(&name) {
                    Ok(f) => {
                        let w = std::io::BufWriter::with_capacity(4096, f);
                        Some(SyncFile::Plain(self.rt.files.add(Handle::Out { w, pos: 0 })))
                    }
                    Err(_) => None,
                }
            } else {
                match GzWriter::<File>::create(&name, "wb") {
                    Ok(g) => Some(SyncFile::Gz(Box::new(g))),
                    Err(_) => None,
                }
            };
            if self.rt.synctex.file.is_some() {
                if self.synctex_record_preamble() == SYNCTEX_NOERR {
                    if self.rt.synctex.magnification == 0 {
                        self.rt.synctex.magnification = 1000;
                    }
                    self.rt.synctex.unit = SYNCTEX_UNIT_FACTOR;
                    self.rt.synctex.busy_name = Some(the_busy_name);
                    if let Some(root) = self.rt.synctex.root_name.take() {
                        self.synctex_record_input(1, &root);
                        self.rt.synctex.root_name = None;
                    }
                    self.rt.synctex.count = 0;
                    return self.rt.synctex.file.is_some();
                } else {
                    let mut m = b"\nSyncTeX warning: no synchronization, problem with ".to_vec();
                    m.extend_from_slice(&the_busy_name);
                    m.push(b'\n');
                    self.sx_printf(&m);
                }
            }
        } else {
            self.sx_printf(b"\nSyncTeX information: no synchronization with keyboard input\n");
        }
        self.synctexabort(false);
        false
    }

    /// `synctex_prepare_content`
    fn synctex_prepare_content(&mut self) -> bool {
        if self.rt.synctex.content_ready {
            return self.rt.synctex.file.is_some();
        }
        if self.synctex_dot_open()
            && self.synctex_record_settings() == SYNCTEX_NOERR
            && self.synctex_record_content() == SYNCTEX_NOERR
        {
            self.rt.synctex.content_ready = true;
            return self.rt.synctex.file.is_some();
        }
        self.synctexabort(false);
        false
    }

    /// synctex.c synctexstartinput: input files get tags whenever SyncTeX is not
    /// switched off with -synctex=0, even while \synctex is 0 (the tags end up in nodes).
    pub fn synctexstartinput(&mut self) {
        self.synctex_read_command_line_option();
        if self.rt.synctex.off {
            return;
        }
        if !self.rt.synctex.tag_counter > 0 {
            self.rt.synctex.tag_counter += 1;
        } else {
            self.curinput.synctextagfield = 0;
            return;
        }
        self.curinput.synctextagfield = self.rt.synctex.tag_counter as i32;
        if self.rt.synctex.tag_counter == 1 {
            let tmpa = self.synctex_current_name();
            let mut name = self.sx_chgto_oem(tmpa);
            if name.is_empty() {
                name = b"texput".to_vec();
            }
            self.rt.synctex.root_name = Some(name);
            return;
        }
        if self.rt.synctex.file.is_some() || self.synctex_dot_open() {
            let tmpb = self.synctex_current_name();
            let tmp = self.sx_chgto_oem(tmpb);
            let tag = self.curinput.synctextagfield;
            self.synctex_record_input(tag, &tmp);
        }
    }

    pub fn synctexterminate(&mut self, log_opened: bool) {
        let mut the_real_syncname: Vec<u8>;
        if log_opened {
            the_real_syncname = self.sx_gettexstring(self.texmflogname);
            // remove the last path extension, which is in general log
            if let Some(i) = the_real_syncname.iter().rposition(|&c| c == b'.') {
                the_real_syncname.truncate(i);
            }
            the_real_syncname.extend_from_slice(SYNCTEX_SUFFIX);
            if !self.rt.synctex.no_gz {
                // Remove any uncompressed synctex file, from a previous build.
                let _ = fsyscp_remove(&the_real_syncname);
                if !self.sx_no_gz_aux_name() {
                    the_real_syncname.extend_from_slice(SYNCTEX_SUFFIX_GZ);
                }
            }
            // always remove the synctex output file before renaming it, windows requires it.
            if let Err(true) = fsyscp_remove(&the_real_syncname) {
                let mut m = b"SyncTeX: Can't remove ".to_vec();
                m.extend_from_slice(&the_real_syncname);
                m.extend_from_slice(b" (file is open or read only)\n");
                self.sx_eprintf(&m);
            }
            if self.rt.synctex.file.is_some() {
                if self.rt.synctex.not_void {
                    self.synctex_record_postamble();
                    // close the synctex file
                    self.sx_close_file();
                    // renaming the working synctex file
                    let busy = self.rt.synctex.busy_name.clone().unwrap_or_default();
                    if fsyscp_rename(&busy, &the_real_syncname) {
                        if log_opened && self.interaction > 0 {
                            let mut m: Vec<u8> = if self.rt.synctex.quoted {
                                b"\nSyncTeX written on \"".to_vec()
                            } else {
                                b"\nSyncTeX written on ".to_vec()
                            };
                            m.extend_from_slice(&the_real_syncname);
                            m.extend_from_slice(if self.rt.synctex.quoted { b"\"\n" } else { b".\n" });
                            self.sx_printf(&m);
                        }
                    } else {
                        let mut m = b"SyncTeX: Can't rename ".to_vec();
                        m.extend_from_slice(&busy);
                        m.extend_from_slice(b" to ");
                        m.extend_from_slice(&the_real_syncname);
                        m.push(b'\n');
                        self.sx_eprintf(&m);
                        let _ = fsyscp_remove(&busy);
                    }
                } else {
                    // close and remove the synctex file because there are no pages of output
                    self.sx_close_file();
                    if let Some(busy) = self.rt.synctex.busy_name.clone() {
                        let _ = fsyscp_remove(&busy);
                    }
                }
            }
            if self.rt.synctex.no_gz || self.sx_no_gz_aux_name() {
                // Remove any compressed synctex file, from a previous build.
                the_real_syncname.extend_from_slice(SYNCTEX_SUFFIX_GZ);
                let _ = fsyscp_remove(&the_real_syncname);
            }
        } else {
            let tmp = self.sx_gettexstring(self.jobname);
            let len = tmp.len();
            // There was a problem with the output. We just try to remove existing synctex
            // output files including the busy one.
            if len > 0 && tmp[0] == b'"' && tmp[len - 1] == b'"' {
                the_real_syncname = tmp[1..].to_vec();
                if the_real_syncname.last() == Some(&b'"') {
                    the_real_syncname.pop();
                }
            } else {
                the_real_syncname = tmp;
            }
            the_real_syncname.extend_from_slice(SYNCTEX_SUFFIX);
            let _ = fsyscp_remove(&the_real_syncname);
            the_real_syncname.extend_from_slice(SYNCTEX_SUFFIX_GZ);
            let _ = fsyscp_remove(&the_real_syncname);
            if self.rt.synctex.file.is_some() {
                self.sx_close_file();
                if let Some(busy) = self.rt.synctex.busy_name.clone() {
                    let _ = fsyscp_remove(&busy);
                }
            }
        }
        self.rt.synctex.busy_name = None;
        self.synctexabort(false);
    }

    /// `SYNCTEX_IGNORE` as defined above the box/math recorders.
    fn sx_ignore(&self) -> bool {
        self.rt.synctex.off
            || self.synctex_value() == 0
            || self.rt.synctex.file.is_none()
            || (self.rt.synctex.form_depth > 0 && !self.sx_with_forms())
    }

    /// `SYNCTEX_IGNORE(NODE,TYPE)` as defined above the glue/rule/kern recorders.
    fn sx_ignore_node(&self, p: i32, size: i32) -> bool {
        self.rt.synctex.off || self.synctex_value() == 0 || 0 >= self.sx_tag(p, size) || 0 >= self.sx_line(p, size)
    }

    /// `SYNCTEX_CONTEXT_DID_CHANGE(NODE,TYPE)`
    fn sx_context_did_change(&self, p: i32, size: i32) -> bool {
        0 == self.rt.synctex.node
            || self.sx_tag(p, size) != self.rt.synctex.tag
            || self.sx_line(p, size) != self.rt.synctex.line
    }

    fn sx_run_recorder(&mut self) {
        if let Some(Recorder::Kern) = self.rt.synctex.recorder {
            let n = self.rt.synctex.node;
            self.synctex_record_node_kern(n);
        }
    }

    fn sx_disabled_warning(&mut self) {
        if self.synctex_value() != 0 && !self.rt.synctex.warn {
            self.rt.synctex.warn = true;
            self.sx_printf(
                b"\nSyncTeX warning: Synchronization was disabled from\nthe command line with -synctex=0\nChanging the value of \\synctex has no effect.",
            );
        }
    }

    pub fn synctexsheet(&mut self, mag: i32) {
        if self.rt.synctex.off {
            self.sx_disabled_warning();
            return;
        }
        if self.totalpages == 0 {
            // Now it is time to properly set up the scale factor.
            if mag > 0 {
                self.rt.synctex.magnification = mag;
            }
        }
        if self.synctex_prepare_content() {
            let sheet = self.totalpages.wrapping_add(1);
            self.synctex_record_sheet(sheet);
        }
    }

    pub fn synctexteehs(&mut self) {
        if self.rt.synctex.off || self.rt.synctex.file.is_none() {
            return;
        }
        let sheet = self.totalpages;
        self.synctex_record_teehs(sheet);
    }

    fn synctex_record_teehs(&mut self, sheet: i32) -> i32 {
        if self.synctex_record_anchor() == SYNCTEX_NOERR {
            let len = self.sx_fprintf(format!("}}{}\n", sheet).as_bytes());
            if self.sx_len_noerr(len) {
                return SYNCTEX_NOERR;
            }
        }
        self.synctexabort(false);
        -1
    }

    /// `SYNCTEX_RECORD_LEN_AND_RETURN_NOERR` / `SYNCTEX_RECORD_LEN_AND_RETURN`: true when
    /// the record was written (and counted).
    fn sx_len_noerr(&mut self, len: i32) -> bool {
        if len > 0 {
            let s = &mut self.rt.synctex;
            s.total_length = s.total_length.wrapping_add(len);
            s.count = s.count.wrapping_add(1);
            true
        } else {
            false
        }
    }

    // ------------------------------------------------------------------ pdf forms

    pub fn synctexpdfxform(&mut self, p: i32) {
        if self.rt.synctex.off {
            self.sx_disabled_warning();
            return;
        }
        if self.synctex_prepare_content() {
            self.synctex_record_pdfxform(p);
        }
    }

    pub fn synctexmrofxfdp(&mut self) {
        if self.rt.synctex.file.is_some() {
            self.synctex_record_mrofxfdp();
        }
    }

    pub fn synctexpdfrefxform(&mut self, objnum: i32) {
        if self.rt.synctex.file.is_some() {
            self.synctex_record_node_pdfrefxform(objnum);
        }
    }

    /// Recording a "<..." line
    fn synctex_record_pdfxform(&mut self, _form: i32) -> i32 {
        if self.sx_ignore() {
            return SYNCTEX_NOERR;
        } else {
            self.rt.synctex.form_depth = self.rt.synctex.form_depth.wrapping_add(1);
            if self.sx_with_forms() {
                let len = self.sx_fprintf(format!("<{}\n", self.pdfcurform).as_bytes());
                if self.sx_len_noerr(len) {
                    return SYNCTEX_NOERR;
                }
            } else {
                return SYNCTEX_NOERR;
            }
        }
        self.synctexabort(false);
        -1
    }

    /// Recording a ">" line
    fn synctex_record_mrofxfdp(&mut self) -> i32 {
        if self.synctex_record_anchor() == SYNCTEX_NOERR {
            self.rt.synctex.form_depth = self.rt.synctex.form_depth.wrapping_sub(1);
            if self.sx_with_forms() {
                let len = self.sx_fprintf(b">\n");
                if self.sx_len_noerr(len) {
                    return SYNCTEX_NOERR;
                }
            } else {
                return SYNCTEX_NOERR;
            }
        }
        self.synctexabort(false);
        -1
    }

    /// Recording a "f..." line
    fn synctex_record_node_pdfrefxform(&mut self, objnum: i32) -> i32 {
        self.rt.synctex.curh = self.curh;
        self.rt.synctex.curv = self.curv;
        if self.sx_ignore() {
            return SYNCTEX_NOERR;
        } else if self.rt.synctex.form_depth > 0 && !self.sx_with_forms() {
            return SYNCTEX_NOERR;
        } else {
            let len;
            if self.sx_should_compress_v() {
                let s = format!("f{}:{},=\n", objnum, self.sx_unit(self.curh));
                len = self.sx_fprintf(s.as_bytes());
            } else {
                let s = format!("f{}:{},{}\n", objnum, self.sx_unit(self.curh), self.sx_unit(self.curv));
                len = self.sx_fprintf(s.as_bytes());
                self.rt.synctex.lastv = self.curv;
            }
            if self.sx_len_noerr(len) {
                return SYNCTEX_NOERR;
            }
        }
        self.synctexabort(false);
        -1
    }

    // ------------------------------------------------------------------ boxes

    /// Set the context to box node p (tag/line from a box node) at the current position.
    fn sx_set_box_context(&mut self, p: i32) {
        let tag = self.sx_tag(p, BOX_NODE_SIZE);
        let line = self.sx_line(p, BOX_NODE_SIZE);
        let s = &mut self.rt.synctex;
        s.node = p;
        s.tag = tag;
        s.line = line;
        s.curh = self.curh;
        s.curv = self.curv;
        s.recorder = None;
    }

    pub fn synctexvlist(&mut self, this_box: i32) {
        if self.sx_ignore() {
            return;
        }
        self.sx_set_box_context(this_box);
        self.synctex_record_node_vlist(this_box);
    }

    pub fn synctextsilv(&mut self, this_box: i32) {
        if self.sx_ignore() {
            return;
        }
        // Ignoring any pending info to be recorded
        self.sx_set_box_context(this_box);
        self.synctex_record_node_tsilv(this_box);
    }

    pub fn synctexvoidvlist(&mut self, p: i32, _this_box: i32) {
        if self.sx_ignore() {
            return;
        }
        self.sx_set_box_context(p);
        self.synctex_record_node_void_vlist(p);
    }

    pub fn synctexhlist(&mut self, this_box: i32) {
        if self.sx_ignore() {
            return;
        }
        self.sx_set_box_context(this_box);
        self.synctex_record_hlist(this_box);
    }

    pub fn synctextsilh(&mut self, this_box: i32) {
        if self.sx_ignore() {
            return;
        }
        // Ignoring any pending info to be recorded
        self.sx_set_box_context(this_box);
        self.synctex_record_node_tsilh(this_box);
    }

    pub fn synctexvoidhlist(&mut self, p: i32, _this_box: i32) {
        if self.sx_ignore() {
            return;
        }
        // the sync context has changed
        if self.rt.synctex.recorder.is_some() {
            // but was not yet recorded
            self.sx_run_recorder();
        }
        self.sx_set_box_context(p);
        self.synctex_record_node_void_hlist(p);
    }

    // ------------------------------------------------------------------ math, glue, rules, kerns

    pub fn synctexmath(&mut self, p: i32, _this_box: i32) {
        if self.sx_ignore() {
            return;
        }
        if self.rt.synctex.recorder.is_some() && self.sx_context_did_change(p, MATH_NODE_SIZE) {
            // the sync context did change
            self.sx_run_recorder();
        }
        let tag = self.sx_tag(p, MATH_NODE_SIZE);
        let line = self.sx_line(p, MATH_NODE_SIZE);
        let s = &mut self.rt.synctex;
        s.node = p;
        s.tag = tag;
        s.line = line;
        s.curh = self.curh;
        s.curv = self.curv;
        s.recorder = None; // no need to record once more
        self.synctex_record_node_math(p); // always record synchronously
    }

    pub fn synctexhorizontalruleorglue(&mut self, p: i32, _this_box: i32) {
        let ty = ix(self.mem, p).hh_b0() as i32;
        match ty {
            RULE_NODE => {
                if self.sx_ignore_node(p, RULE_NODE_SIZE) {
                    return;
                }
            }
            GLUE_NODE => {
                if self.sx_ignore_node(p, GLUE_NODE_SIZE) {
                    return;
                }
            }
            KERN_NODE => {
                if self.sx_ignore_node(p, KERN_NODE_SIZE) {
                    return;
                }
            }
            _ => self.sx_printf(format!("\nSynchronize ERROR: unknown node type {}\n", ty).as_bytes()),
        }
        {
            let s = &mut self.rt.synctex;
            s.node = p;
            s.curh = self.curh;
            s.curv = self.curv;
            s.recorder = None;
        }
        match ty {
            RULE_NODE => {
                self.rt.synctex.tag = self.sx_tag(p, RULE_NODE_SIZE);
                self.rt.synctex.line = self.sx_line(p, RULE_NODE_SIZE);
                self.synctex_record_node_rule(p);
            }
            GLUE_NODE => {
                self.rt.synctex.tag = self.sx_tag(p, GLUE_NODE_SIZE);
                self.rt.synctex.line = self.sx_line(p, GLUE_NODE_SIZE);
                self.synctex_record_node_glue(p);
            }
            KERN_NODE => {
                self.rt.synctex.tag = self.sx_tag(p, KERN_NODE_SIZE);
                self.rt.synctex.line = self.sx_line(p, KERN_NODE_SIZE);
                self.synctex_record_node_kern(p);
            }
            _ => self.sx_printf(format!("\nSynchronize ERROR: unknown node type {}\n", ty).as_bytes()),
        }
    }

    pub fn synctexkern(&mut self, p: i32, this_box: i32) {
        if self.sx_ignore_node(p, KERN_NODE_SIZE) {
            return;
        }
        let tag = self.sx_tag(p, KERN_NODE_SIZE);
        let line = self.sx_line(p, KERN_NODE_SIZE);
        if self.sx_context_did_change(p, KERN_NODE_SIZE) {
            // the sync context has changed
            if self.rt.synctex.recorder.is_some() {
                // but was not yet recorded
                self.sx_run_recorder();
            }
            if self.rt.synctex.node == this_box {
                // first node in the list
                let s = &mut self.rt.synctex;
                s.node = p;
                s.tag = tag;
                s.line = line;
                s.recorder = Some(Recorder::Kern);
            } else {
                let s = &mut self.rt.synctex;
                s.node = p;
                s.tag = tag;
                s.line = line;
                s.recorder = None;
                // always record when the context has just changed and when not the first node
                self.synctex_record_node_kern(p);
            }
        } else {
            // just update the geometry and type (for future improvements)
            let s = &mut self.rt.synctex;
            s.node = p;
            s.tag = tag;
            s.line = line;
            s.recorder = Some(Recorder::Kern);
        }
    }

    /// synctexchar: never sent by pdfTeX (kept for completeness).
    #[allow(dead_code)]
    pub fn synctexchar(&mut self, p: i32, _this_box: i32) {
        if self.rt.synctex.off
            || self.synctex_value() == 0
            || self.rt.synctex.file.is_none()
            || self.rt.synctex.count > 2000
        {
            return;
        }
        if self.rt.synctex.recorder.is_some() {
            self.sx_run_recorder();
        }
        let s = &mut self.rt.synctex;
        s.node = p;
        s.tag = 0;
        s.line = 0;
        s.recorder = None;
        self.synctex_record_node_char(p);
    }

    /// synctexnode: never sent by pdfTeX (kept for completeness).
    #[allow(dead_code)]
    pub fn synctexnode(&mut self, p: i32, _this_box: i32) {
        if self.rt.synctex.off || self.synctex_value() == 0 || self.rt.synctex.file.is_none() {
            return;
        }
        self.synctex_record_node_unknown(p);
    }

    /// Record information synchronously for the current location ("x" line).
    pub fn synctexcurrent(&mut self) {
        if self.rt.synctex.off || self.synctex_value() == 0 || self.rt.synctex.file.is_none() {
            return;
        }
        let (tag, line) = (self.rt.synctex.tag, self.rt.synctex.line);
        let len;
        if self.sx_should_compress_v() {
            let s = format!("x{},{}:{},=\n", tag, line, self.sx_unit(self.curh));
            len = self.sx_fprintf(s.as_bytes());
        } else {
            let s = format!("x{},{}:{},{}\n", tag, line, self.sx_unit(self.curh), self.sx_unit(self.curv));
            len = self.sx_fprintf(s.as_bytes());
            self.rt.synctex.lastv = self.curv;
        }
        if len > 0 {
            self.rt.synctex.total_length = self.rt.synctex.total_length.wrapping_add(len);
            return;
        }
        self.synctexabort(false);
    }

    // ------------------------------------------------------------------ records

    /// Recording the settings
    fn synctex_record_settings(&mut self) -> i32 {
        if self.rt.synctex.file.is_none() {
            return SYNCTEX_NOERR;
        }
        let pdf = self.pdfoutputvalue > 0;
        let off = if pdf { 0 } else { self.sx_unit(4736287) };
        let s = format!(
            "Output:{}\nMagnification:{}\nUnit:{}\nX Offset:{}\nY Offset:{}\n",
            if pdf { "pdf" } else { "dvi" },
            self.rt.synctex.magnification,
            self.rt.synctex.unit,
            off,
            off
        );
        let len = self.sx_fprintf(s.as_bytes());
        if len > 0 {
            self.rt.synctex.total_length = self.rt.synctex.total_length.wrapping_add(len);
            return SYNCTEX_NOERR;
        }
        self.synctexabort(false);
        -1
    }

    /// Recording a "SyncTeX..." line
    fn synctex_record_preamble(&mut self) -> i32 {
        let o = self.rt.synctex.options;
        let s = format!("SyncTeX Version:{}\n", if o > SYNCTEX_VERSION { o } else { SYNCTEX_VERSION });
        let len = self.sx_fprintf(s.as_bytes());
        if len > 0 {
            self.rt.synctex.total_length = len;
            return SYNCTEX_NOERR;
        }
        self.synctexabort(false);
        -1
    }

    /// Recording a "Input:..." line
    fn synctex_record_input(&mut self, tag: i32, name: &[u8]) -> i32 {
        let mut s = format!("Input:{}:", tag).into_bytes();
        s.extend_from_slice(name);
        s.push(b'\n');
        let len = self.sx_fprintf(&s);
        if len > 0 {
            self.rt.synctex.total_length = self.rt.synctex.total_length.wrapping_add(len);
            return SYNCTEX_NOERR;
        }
        self.synctexabort(false);
        -1
    }

    /// Recording a "!..." line
    fn synctex_record_anchor(&mut self) -> i32 {
        let s = format!("!{}\n", self.rt.synctex.total_length);
        let len = self.sx_fprintf(s.as_bytes());
        if len > 0 {
            self.rt.synctex.total_length = len;
            self.rt.synctex.count = self.rt.synctex.count.wrapping_add(1);
            return SYNCTEX_NOERR;
        }
        self.synctexabort(false);
        -1
    }

    /// Recording a "Content" line
    fn synctex_record_content(&mut self) -> i32 {
        let len = self.sx_fprintf(b"Content:\n");
        if len > 0 {
            self.rt.synctex.total_length = self.rt.synctex.total_length.wrapping_add(len);
            return SYNCTEX_NOERR;
        }
        self.synctexabort(false);
        -1
    }

    /// Recording a "{..." line
    fn synctex_record_sheet(&mut self, sheet: i32) -> i32 {
        if self.synctex_record_anchor() == SYNCTEX_NOERR {
            let len = self.sx_fprintf(format!("{{{}\n", sheet).as_bytes());
            if self.sx_len_noerr(len) {
                return SYNCTEX_NOERR;
            }
        }
        self.synctexabort(false);
        -1
    }

    /// The box records "v", "[", "h" and "(": tag,line:h,v:W,H,D
    fn sx_record_box(&mut self, c: char, p: i32) {
        let tag = self.sx_tag(p, BOX_NODE_SIZE);
        let line = self.sx_line(p, BOX_NODE_SIZE);
        let h = self.sx_unit(self.rt.synctex.curh);
        let w = self.sx_unit(self.sx_mem(p + WIDTH_OFFSET));
        let ht = self.sx_unit(self.sx_mem(p + HEIGHT_OFFSET));
        let dp = self.sx_unit(self.sx_mem(p + DEPTH_OFFSET));
        let len;
        if self.sx_should_compress_v() {
            let s = format!("{}{},{}:{},=:{},{},{}\n", c, tag, line, h, w, ht, dp);
            len = self.sx_fprintf(s.as_bytes());
        } else {
            let v = self.sx_unit(self.rt.synctex.curv);
            let s = format!("{}{},{}:{},{}:{},{},{}\n", c, tag, line, h, v, w, ht, dp);
            len = self.sx_fprintf(s.as_bytes());
            self.rt.synctex.lastv = self.rt.synctex.curv;
        }
        if self.sx_len_noerr(len) {
            return;
        }
        self.synctexabort(false);
    }

    /// A record without arguments ("]", ")").
    fn sx_record_plain(&mut self, s: &[u8]) {
        let len = self.sx_fprintf(s);
        if self.sx_len_noerr(len) {
            return;
        }
        self.synctexabort(false);
    }

    /// Recording a "v..." line
    fn synctex_record_node_void_vlist(&mut self, p: i32) {
        self.sx_record_box('v', p);
    }

    /// Recording a "[..." line
    fn synctex_record_node_vlist(&mut self, p: i32) {
        self.rt.synctex.not_void = true;
        self.sx_record_box('[', p);
    }

    /// Recording a "]..." line
    fn synctex_record_node_tsilv(&mut self, _p: i32) {
        self.sx_record_plain(b"]\n");
    }

    /// Recording a "h..." line
    fn synctex_record_node_void_hlist(&mut self, p: i32) {
        self.sx_record_box('h', p);
    }

    /// Recording a "(..." line
    fn synctex_record_hlist(&mut self, p: i32) {
        self.rt.synctex.not_void = true;
        self.sx_record_box('(', p);
    }

    /// Recording a ")..." line
    fn synctex_record_node_tsilh(&mut self, _p: i32) {
        self.sx_record_plain(b")\n");
    }

    /// Recording a "Count..." line
    fn synctex_record_count(&mut self) -> i32 {
        let s = format!("Count:{}\n", self.rt.synctex.count);
        let len = self.sx_fprintf(s.as_bytes());
        if len > 0 {
            self.rt.synctex.total_length = self.rt.synctex.total_length.wrapping_add(len);
            return SYNCTEX_NOERR;
        }
        self.synctexabort(false);
        -1
    }

    /// Recording a "Postamble" section
    fn synctex_record_postamble(&mut self) -> i32 {
        if self.synctex_record_anchor() == SYNCTEX_NOERR {
            let len = self.sx_fprintf(b"Postamble:\n");
            if len > 0 {
                self.rt.synctex.total_length = self.rt.synctex.total_length.wrapping_add(len);
                if self.synctex_record_count() != 0 || self.synctex_record_anchor() != 0 {
                } else {
                    let len = self.sx_fprintf(b"Post scriptum:\n");
                    if len > 0 {
                        self.rt.synctex.total_length = self.rt.synctex.total_length.wrapping_add(len);
                        return SYNCTEX_NOERR;
                    }
                }
            }
        }
        self.synctexabort(false);
        -1
    }

    /// Point records "g", "$": tag,line:h,v
    fn sx_record_point(&mut self, c: char, tag: i32, line: i32) {
        let h = self.sx_unit(self.rt.synctex.curh);
        let len;
        if self.sx_should_compress_v() {
            let s = format!("{}{},{}:{},=\n", c, tag, line, h);
            len = self.sx_fprintf(s.as_bytes());
        } else {
            let v = self.sx_unit(self.rt.synctex.curv);
            let s = format!("{}{},{}:{},{}\n", c, tag, line, h, v);
            len = self.sx_fprintf(s.as_bytes());
            self.rt.synctex.lastv = self.rt.synctex.curv;
        }
        if self.sx_len_noerr(len) {
            return;
        }
        self.synctexabort(false);
    }

    /// Recording a "g..." line
    fn synctex_record_node_glue(&mut self, p: i32) {
        let tag = self.sx_tag(p, GLUE_NODE_SIZE);
        let line = self.sx_line(p, GLUE_NODE_SIZE);
        self.sx_record_point('g', tag, line);
    }

    /// Recording a "k..." line
    fn synctex_record_node_kern(&mut self, p: i32) {
        let tag = self.sx_tag(p, KERN_NODE_SIZE);
        let line = self.sx_line(p, KERN_NODE_SIZE);
        let h = self.sx_unit(self.rt.synctex.curh);
        let w = self.sx_unit(self.sx_mem(p + WIDTH_OFFSET));
        let len;
        if self.sx_should_compress_v() {
            let s = format!("k{},{}:{},=:{}\n", tag, line, h, w);
            len = self.sx_fprintf(s.as_bytes());
        } else {
            let v = self.sx_unit(self.rt.synctex.curv);
            let s = format!("k{},{}:{},{}:{}\n", tag, line, h, v, w);
            len = self.sx_fprintf(s.as_bytes());
            self.rt.synctex.lastv = self.rt.synctex.curv;
        }
        if self.sx_len_noerr(len) {
            return;
        }
        self.synctexabort(false);
    }

    /// Recording a "r..." line
    fn synctex_record_node_rule(&mut self, p: i32) {
        let tag = self.sx_tag(p, RULE_NODE_SIZE);
        let line = self.sx_line(p, RULE_NODE_SIZE);
        let h = self.sx_unit(self.rt.synctex.curh);
        let (w, ht, dp) = (self.sx_unit(self.rulewd), self.sx_unit(self.ruleht), self.sx_unit(self.ruledp));
        let len;
        if self.sx_should_compress_v() {
            let s = format!("r{},{}:{},=:{},{},{}\n", tag, line, h, w, ht, dp);
            len = self.sx_fprintf(s.as_bytes());
        } else {
            let v = self.sx_unit(self.rt.synctex.curv);
            let s = format!("r{},{}:{},{}:{},{},{}\n", tag, line, h, v, w, ht, dp);
            len = self.sx_fprintf(s.as_bytes());
            self.rt.synctex.lastv = self.rt.synctex.curv;
        }
        if self.sx_len_noerr(len) {
            return;
        }
        self.synctexabort(false);
    }

    /// Recording a "$..." line
    fn synctex_record_node_math(&mut self, p: i32) {
        let tag = self.sx_tag(p, MATH_NODE_SIZE);
        let line = self.sx_line(p, MATH_NODE_SIZE);
        self.sx_record_point('$', tag, line);
    }

    /// Recording a "c..." line
    fn synctex_record_node_char(&mut self, _p: i32) {
        let h = self.sx_unit(self.rt.synctex.curh);
        let len;
        if self.sx_should_compress_v() {
            len = self.sx_fprintf(format!("c{},=\n", h).as_bytes());
        } else {
            let v = self.sx_unit(self.rt.synctex.curv);
            len = self.sx_fprintf(format!("c{},{}\n", h, v).as_bytes());
            self.rt.synctex.lastv = self.rt.synctex.curv;
        }
        if self.sx_len_noerr(len) {
            return;
        }
        self.synctexabort(false);
    }

    /// Recording a "?..." line, type, subtype and position
    fn synctex_record_node_unknown(&mut self, p: i32) {
        let h = self.sx_unit(self.rt.synctex.curh);
        let ty = ix(self.mem, p).hh_b0() as i32;
        let st = ix(self.mem, p).hh_b1() as i32;
        let len;
        if self.sx_should_compress_v() {
            len = self.sx_fprintf(format!("?{},=:{},{}\n", h, ty, st).as_bytes());
        } else {
            let v = self.sx_unit(self.rt.synctex.curv);
            len = self.sx_fprintf(format!("?{},{}:{},{}\n", h, v, ty, st).as_bytes());
            self.rt.synctex.lastv = self.rt.synctex.curv;
        }
        if self.sx_len_noerr(len) {
            return;
        }
        self.synctexabort(false);
    }
}
