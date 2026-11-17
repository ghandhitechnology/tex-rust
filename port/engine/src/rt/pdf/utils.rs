//! Port of pdftexdir/utils.c: messages, job id, PDF string escaping, /ID, color stacks,
//! \pdfsetmatrix, \pdfsave/\pdfrestore bookkeeping, VF font arrays.

use crate::gen::globals::Globals;
use crate::rt::*;

const PRINTF_BUF_SIZE: usize = 1024;
const MAX_PSTRING_LEN: usize = 1024;
const STACK_INCREMENT: usize = 8;
const MAX_COLORSTACKS: usize = 32768;
const COLOR_DEFAULT: &str = "0 g 0 G";
const DIRECT_ALWAYS: i32 = 2;

#[derive(Default)]
pub struct ColStack {
    page_stack: Vec<Option<String>>,
    form_stack: Vec<Option<String>>,
    page_current: Option<String>,
    form_current: Option<String>,
    form_init: Option<String>,
    literal_mode: i32,
    page_start: bool,
}

#[derive(Default, Clone, Copy)]
struct MatrixEntry {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

#[derive(Default, Clone, Copy)]
struct PosEntry {
    pos_h: i32,
    pos_v: i32,
    matrix_stack: usize,
}

/// utils.c's static state.
#[derive(Default)]
pub struct UtilsState {
    pub cur_file_name: Option<String>,
    pub last_tex_string: i32,
    jobname_cstr: Option<String>,
    job_id_string: Option<String>,
    pdftexbanner_init: bool,
    /// fb_array (bytes beyond fb_ptr are kept, as in C) and fb_ptr
    pub fb: Vec<u8>,
    pub fb_ptr: usize,
    colstacks: Vec<ColStack>,
    colstacks_init: bool,
    page_mode: bool,
    matrix_stack: Vec<MatrixEntry>,
    matrix_stack_used: usize,
    pos_stack: Vec<PosEntry>,
    ret_llx: i32,
    ret_lly: i32,
    ret_urx: i32,
    ret_ury: i32,
    last_llx: i32,
    last_lly: i32,
    last_urx: i32,
    last_ury: i32,
    // \pdfmatch
    /// utils.c statics of \pdfmatch/\pdflastmatch (sub_match_count, pmatch,
    /// match_string, last_match_succeeded)
    matchstate: texrs_posix_regex::pdftex::MatchState,
    // allocvffnts
    vf_fnts_ptr: i32,
    vf_fnts_limit: i32,
    vf_fnts_init: bool,
}

/// vsnprintf into a PRINTF_BUF_SIZE buffer: at most 1023 bytes survive.
fn clip(s: &str) -> &[u8] {
    let b = s.as_bytes();
    &b[..b.len().min(PRINTF_BUF_SIZE - 1)]
}

/// zlib crc32 (IEEE 802.3, reflected), used by getresnameprefix.
pub fn crc32(mut crc: u32, data: &[u8]) -> u32 {
    crc = !crc;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

impl Globals {
    // ------------------------------------------------------------------ fb (font buffer)
    /// utils.c fb_offset: fb_ptr - fb_array
    pub fn fb_offset(&self) -> i32 {
        self.rt.pdfu.fb_ptr as i32
    }
    /// utils.c fb_seek: fb_ptr = fb_array + offset (contents are kept)
    pub fn fb_seek(&mut self, offset: i32) {
        self.rt.pdfu.fb_ptr = offset as usize;
    }
    /// utils.c fb_putchar
    pub fn fb_putchar(&mut self, b: u8) {
        let u = &mut self.rt.pdfu;
        if u.fb.len() <= u.fb_ptr {
            u.fb.resize(u.fb_ptr + 1, 0);
        }
        u.fb[u.fb_ptr] = b;
        u.fb_ptr += 1;
    }
    /// utils.c fb_flush: copy fb_array[0..fb_ptr) to the PDF buffer
    pub fn fb_flush(&mut self) {
        let data = std::mem::take(&mut self.rt.pdfu.fb);
        let end = self.rt.pdfu.fb_ptr.min(data.len());
        let mut p = 0;
        while p < end {
            let mut n = (self.pdfbufsize - self.pdfptr) as usize;
            if end - p < n {
                n = end - p;
            }
            slice(self.pdfbuf, self.pdfptr, n).copy_from_slice(&data[p..p + n]);
            self.pdfptr += n as i32;
            if self.pdfptr == self.pdfbufsize {
                self.pdfflush();
            }
            p += n;
        }
        self.rt.pdfu.fb = data;
        self.rt.pdfu.fb_ptr = 0;
    }

    // ------------------------------------------------------------------ pdf output
    /// ptexmac.h pdfroom(n)
    pub fn pdfroom(&mut self, n: i32) {
        if (n.wrapping_add(self.pdfptr)) as u32 > self.pdfbufsize as u32 {
            if self.pdfosmode {
                self.pdfosgetosbuf(n);
            } else if n as u32 > self.pdfbufsize as u32 {
                self.pdftex_fail("PDF output buffer overflowed");
            } else {
                self.pdfflush();
            }
        }
    }

    /// ptexmac.h pdfout(c)
    #[inline]
    pub fn pdfout(&mut self, c: u8) {
        self.pdfroom(1);
        *ix(self.pdfbuf, self.pdfptr) = c;
        self.pdfptr += 1;
    }

    pub fn pdf_puts(&mut self, s: &[u8]) {
        if s.is_empty() {
            return;
        }
        self.pdfroom(s.len() as i32 + 1);
        slice(self.pdfbuf, self.pdfptr, s.len()).copy_from_slice(s);
        self.pdfptr += s.len() as i32;
        self.pdflastbyte = s[s.len() - 1];
    }
    pub fn pdf_newline(&mut self) {
        if self.pdflastbyte != b'\n' {
            self.pdf_puts(b"\n");
        }
    }
    pub fn pdf_printf(&mut self, s: &str) {
        let b = clip(s).to_vec();
        self.pdf_puts(&b);
    }

    // ------------------------------------------------------------------ strings and messages
    /// utils.c maketexstring (check_buf + copy + makestring).
    pub fn maketexstring_u(&mut self, s: &[u8]) -> i32 {
        if s.is_empty() {
            return self.getnullstr();
        }
        if (self.poolptr as i64 + s.len() as i64) > self.poolsize as i64 {
            self.pdftex_fail(&format!("buffer overflow at file {}, line {}", "utils.c", 183));
        }
        for &c in s {
            *ix(self.strpool, self.poolptr) = c;
            self.poolptr += 1;
        }
        let g = self.makestring();
        self.rt.pdfu.last_tex_string = g;
        g
    }

    pub fn tex_printf(&mut self, s: &str) {
        let b = clip(s).to_vec();
        let st = self.maketexstring_u(&b);
        self.print(st);
        let l = self.rt.pdfu.last_tex_string;
        self.flushstr(l);
        self.rt.files.flush(FileRef::STDOUT);
    }

    fn safe_print(&mut self, s: &[u8]) {
        for &c in s {
            self.print(c as i32);
        }
    }

    pub fn removepdffile(&mut self) {
        if self.rt.kpathsea_debug == 0 && self.outputfilename != 0 && self.fixedpdfdraftmode == 0 {
            self.parzip_abandon();
            let f = self.pdffile;
            self.rt.files.close(f);
            let name = bytes_to_string(&self.tex_string(self.outputfilename));
            let _ = std::fs::remove_file(name);
        }
    }

    /// utils.c pdftex_fail (format must match pdf_error in pdftex.web).
    pub fn pdftex_fail(&mut self, msg: &str) -> ! {
        self.println();
        self.safe_print(b"!pdfTeX error: ");
        let inv = self.rt.invocation_name();
        self.safe_print(inv.as_bytes());
        if let Some(f) = self.rt.pdfu.cur_file_name.clone() {
            self.safe_print(b" (file ");
            self.safe_print(f.as_bytes());
            self.safe_print(b")");
        }
        self.safe_print(b": ");
        let m = clip(msg).to_vec();
        self.safe_print(&m);
        self.println();
        self.removepdffile();
        self.safe_print(b" ==> Fatal error occurred, no output PDF file produced!");
        self.println();
        if self.rt.kpathsea_debug != 0 {
            self.safe_print(b"kpathsea_debug enabled, calling abort()...");
            self.println();
            self.rt.files.flush_stdout();
            std::process::abort();
        }
        self.x_uexit(1);
    }

    /// utils.c pdftex_warn (format must match pdf_warn in pdftex.web).
    pub fn pdftex_warn(&mut self, msg: &str) {
        if let Some(c) = self.rt.fonts.map.capture.as_mut() {
            c.push(msg.to_string());
        }
        self.println();
        self.println();
        let inv = self.rt.invocation_name();
        self.tex_printf(&format!("pdfTeX warning: {}", inv));
        if let Some(f) = self.rt.pdfu.cur_file_name.clone() {
            self.tex_printf(&format!(" (file {})", f));
        }
        self.tex_printf(": ");
        let m = clip(msg).to_vec();
        let s = self.maketexstring_u(&m);
        self.print(s);
        let l = self.rt.pdfu.last_tex_string;
        self.flushstr(l);
        self.println();
    }

    pub fn garbagewarning(&mut self) {
        self.pdftex_warn("dangling objects discarded, no output file produced.");
        self.removepdffile();
    }

    pub fn setjobid(&mut self, year: i32, month: i32, day: i32, time: i32) {
        if self.rt.pdfu.job_id_string.is_some() {
            return;
        }
        let name = bytes_to_string(&self.tex_string(self.jobname));
        let fmt = bytes_to_string(&self.tex_string(self.formatident));
        let s = format!(
            "{:04}/{:02}/{:02} {:02}:{:02} {} {} {}{} {}",
            year,
            month,
            day,
            time / 60,
            time % 60,
            name,
            fmt,
            texmfmp::BANNER,
            texmfmp::VERSION_STRING,
            texmfmp::KPSE_VERSION
        );
        self.rt.pdfu.job_id_string = Some(s);
    }

    pub fn makepdftexbanner(&mut self) {
        if self.rt.pdfu.pdftexbanner_init {
            return;
        }
        let s = format!("{}{} {}", texmfmp::BANNER, texmfmp::VERSION_STRING, texmfmp::KPSE_VERSION);
        self.pdftexbanner = self.maketexstring_u(s.as_bytes());
        self.rt.pdfu.pdftexbanner_init = true;
    }

    pub fn getresnameprefix(&mut self) -> i32 {
        const NAME_STR: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
        let id = self.rt.pdfu.job_id_string.clone().unwrap_or_default();
        let mut crc = crc32(0, id.as_bytes()) as u64;
        let base = NAME_STR.len() as u64;
        let mut prefix = [0u8; 6];
        for p in prefix.iter_mut() {
            *p = NAME_STR[(crc % base) as usize];
            crc /= base;
        }
        self.maketexstring_u(&prefix)
    }

    pub fn writestreamlength(&mut self, length: i64, offset: i64) {
        if self.rt.pdfu.jobname_cstr.is_none() {
            self.rt.pdfu.jobname_cstr = Some(bytes_to_string(&self.tex_string(self.jobname)));
        }
        if self.fixedpdfdraftmode == 0 {
            if self.parzip_stream_length(length, offset) {
                return;
            }
            let f = self.pdffile;
            self.rt.files.flush(f);
            self.rt.files.seek_out(f, offset as u64);
            let s = format!("{}", length);
            self.rt.files.write(f, s.as_bytes());
            let back = self.pdfoffset_c();
            self.rt.files.seek_out(f, back as u64);
        }
    }

    /// pdfoffset() macro: pdf_gone + pdf_ptr
    pub fn pdfoffset_c(&self) -> i64 {
        self.pdfgone + self.pdfptr as i64
    }

    pub fn extxnoverd(&mut self, x: i32, n: i32, d: i32) -> i32 {
        let mut r = (x as f64 * n as f64) / d as f64;
        if r > f64::EPSILON {
            r += 0.5;
        } else {
            r -= 0.5;
        }
        if r >= i32::MAX as f64 || r <= -(i32::MAX as f64) {
            self.pdftex_warn("arithmetic: number too big");
        }
        r as i32
    }

    pub fn libpdffinish(&mut self) {
        self.rt.pdfu.fb = Vec::new();
        self.rt.pdfu.fb_ptr = 0;
        self.rt.pdfu.job_id_string = None;
        self.pdflib_free_all();
    }

    // ------------------------------------------------------------------ escaping
    pub fn convert_string_to_pdf_string(&mut self, input: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for &c in input {
            if out.len() + 5 > MAX_PSTRING_LEN {
                self.pdftex_fail(&format!("buffer overflow at file {}, line {}", "utils.c", 438));
            }
            if c < b'!' || c > b'~' {
                out.extend_from_slice(format!("\\{:03o}", c).as_bytes());
            } else if c == b'(' || c == b')' {
                out.push(b'\\');
                out.push(c);
            } else if c == b'\\' {
                out.extend_from_slice(b"\\\\");
            } else {
                out.push(c);
            }
        }
        out
    }

    pub fn escapestring(&mut self, mut inp: i32) {
        let out = self.poolptr;
        while inp < out {
            if self.poolptr + 4 >= self.poolsize {
                self.poolptr = self.poolsize;
                return;
            }
            let ch = *ix(self.strpool, inp);
            inp += 1;
            if ch < b'!' || ch > b'~' {
                for &c in format!("\\{:03o}", ch).as_bytes() {
                    *ix(self.strpool, self.poolptr) = c;
                    self.poolptr += 1;
                }
                continue;
            }
            if ch == b'(' || ch == b')' || ch == b'\\' {
                *ix(self.strpool, self.poolptr) = b'\\';
                self.poolptr += 1;
            }
            *ix(self.strpool, self.poolptr) = ch;
            self.poolptr += 1;
        }
    }

    pub fn escapename(&mut self, mut inp: i32) {
        let out = self.poolptr;
        while inp < out {
            if self.poolptr + 3 >= self.poolsize {
                self.poolptr = self.poolsize;
                return;
            }
            let ch = *ix(self.strpool, inp);
            inp += 1;
            let escape = (1..=32).contains(&ch) || ch >= 127 || matches!(ch, 35 | 37 | 40 | 41 | 47 | 60 | 62 | 91 | 93 | 123 | 125);
            if ch == 0 {
                continue;
            }
            if escape {
                for &c in format!("#{:02X}", ch).as_bytes() {
                    *ix(self.strpool, self.poolptr) = c;
                    self.poolptr += 1;
                }
            } else {
                *ix(self.strpool, self.poolptr) = ch;
                self.poolptr += 1;
            }
        }
    }

    pub fn escapehex(&mut self, mut inp: i32) {
        let out = self.poolptr;
        while inp < out {
            if self.poolptr + 2 >= self.poolsize {
                self.poolptr = self.poolsize;
                return;
            }
            let ch = *ix(self.strpool, inp);
            inp += 1;
            for &c in format!("{:02X}", ch).as_bytes() {
                *ix(self.strpool, self.poolptr) = c;
                self.poolptr += 1;
            }
        }
    }

    pub fn unescapehex(&mut self, mut inp: i32) {
        let out = self.poolptr;
        let mut a: u8 = 0;
        let mut first = true;
        while inp < out {
            if self.poolptr + 1 >= self.poolsize {
                self.poolptr = self.poolsize;
                return;
            }
            let mut ch = *ix(self.strpool, inp);
            inp += 1;
            if ch.is_ascii_digit() {
                ch -= b'0';
            } else if (b'A'..=b'F').contains(&ch) {
                ch -= b'A' - 10;
            } else if (b'a'..=b'f').contains(&ch) {
                ch -= b'a' - 10;
            } else {
                continue;
            }
            if first {
                a = ch << 4;
                first = false;
                continue;
            }
            *ix(self.strpool, self.poolptr) = a.wrapping_add(ch);
            self.poolptr += 1;
            first = true;
        }
        if !first {
            *ix(self.strpool, self.poolptr) = a;
            self.poolptr += 1;
        }
    }

    fn hex16(d: &[u8; 16]) -> String {
        d.iter().map(|b| format!("{:02X}", b)).collect()
    }

    pub fn printID(&mut self, filename: i32) {
        let mut m = md5::Md5::new();
        self.initstarttime();
        m.update(self.rt.start_time_str.as_bytes());
        let name = self.tex_string(filename);
        m.update(&name);
        let id = Self::hex16(&m.finish());
        self.pdf_printf(&format!("/ID [<{}> <{}>]", id, id));
    }

    pub fn printIDalt(&mut self, toks: i32) {
        let s = self.tokenstostring(toks);
        let bytes = self.tex_string(s);
        let l = self.lasttokensstring;
        self.flushstr(l);
        if bytes.is_empty() {
            return;
        }
        let id = Self::hex16(&md5::md5(&bytes));
        self.pdf_printf(&format!("/ID [<{}> <{}>]", id, id));
    }

    pub fn printcreationdate(&mut self) {
        self.initstarttime();
        let s = format!("/CreationDate ({})\n", self.rt.start_time_str);
        self.pdf_printf(&s);
    }

    pub fn printmoddate(&mut self) {
        self.initstarttime();
        let s = format!("/ModDate ({})\n", self.rt.start_time_str);
        self.pdf_printf(&s);
    }

    // ------------------------------------------------------------------ \pdfmatch
    pub fn matchstrings(&mut self, s: i32, t: i32, subcount: i32, icase: bool) {
        use texrs_posix_regex::pdftex::MatchOutcome;
        if self.poolptr + 10 >= self.poolsize {
            self.poolptr = self.poolsize;
            return;
        }
        let pattern = self.tex_string(s);
        let subject = self.tex_string(t);
        let outcome = match self.rt.pdfu.matchstate.match_strings(&pattern, &subject, subcount, icase) {
            Ok(o) => o,
            Err(abort) => self.regex_abort(abort),
        };
        if let MatchOutcome::CompileError(_) = outcome {
            self.pdftex_warn(&outcome.warning().unwrap());
        }
        for &c in outcome.as_bytes() {
            *ix(self.strpool, self.poolptr) = c;
            self.poolptr += 1;
        }
    }

    /// Where TeX Live's pdfTeX does not come back from regexec: a failed assert()
    /// (MinGW's message on stderr, then abort(), exit code 3) or a NULL dereference
    /// (access violation). Patterns the C engine loops on forever loop here too.
    fn regex_abort(&mut self, abort: texrs_posix_regex::ExecAbort) -> ! {
        use texrs_posix_regex::ExecAbort;
        match abort {
            ExecAbort::Assertion { .. } => {
                let k = self.rt.kpse();
                k.stderr_write(format!("{abort}\n").as_bytes());
                k.flush_stderr();
                std::process::exit(3)
            }
            ExecAbort::NullState { .. } => std::process::exit(0xC000_0005u32 as i32),
            ExecAbort::StepLimit => unreachable!("regexec runs without a step limit"),
        }
    }

    pub fn getmatch(&mut self, i: i32) {
        let out = self.rt.pdfu.matchstate.get_match(i);
        // utils.c reserves 20 + match length (found) or 4 ("-1->") bytes
        let size = if out == b"-1->" {
            4
        } else {
            let arrow = out.windows(2).position(|w| w == b"->").unwrap();
            20 + (out.len() - arrow - 2) as i32
        };
        if self.poolptr + size >= self.poolsize {
            self.poolptr = self.poolsize;
            return;
        }
        for c in out {
            *ix(self.strpool, self.poolptr) = c;
            self.poolptr += 1;
        }
    }

    // ------------------------------------------------------------------ color stacks
    fn init_colorstacks(&mut self) {
        if !self.rt.pdfu.colstacks_init {
            self.rt.pdfu.colstacks_init = true;
            self.rt.pdfu.colstacks.push(ColStack {
                page_current: Some(COLOR_DEFAULT.into()),
                form_current: Some(COLOR_DEFAULT.into()),
                form_init: Some(COLOR_DEFAULT.into()),
                literal_mode: DIRECT_ALWAYS,
                page_start: true,
                ..Default::default()
            });
        }
    }

    pub fn colorstackused(&mut self) -> i32 {
        self.init_colorstacks();
        self.rt.pdfu.colstacks.len() as i32
    }

    fn cstr_of(&self, s: i32) -> Option<String> {
        let b = self.tex_string(s);
        // makecstring stops at an embedded NUL
        let b: Vec<u8> = b.into_iter().take_while(|&c| c != 0).collect();
        if b.is_empty() {
            None
        } else {
            Some(bytes_to_string(&b))
        }
    }

    pub fn newcolorstack(&mut self, s: i32, literal_mode: i32, page_start: bool) -> i32 {
        self.init_colorstacks();
        if self.rt.pdfu.colstacks.len() == MAX_COLORSTACKS {
            return -1;
        }
        let cur = self.cstr_of(s);
        let n = self.rt.pdfu.colstacks.len();
        self.rt.pdfu.colstacks.push(ColStack {
            page_current: cur.clone(),
            form_current: cur.clone(),
            form_init: cur,
            literal_mode,
            page_start,
            ..Default::default()
        });
        n as i32
    }

    fn put_cstring_on_strpool(&mut self, start: i32, s: Option<String>) {
        let Some(s) = s else { return };
        if s.is_empty() {
            return;
        }
        let b = s.as_bytes();
        self.poolptr = start + b.len() as i32;
        if self.poolptr >= self.poolsize {
            self.poolptr = self.poolsize;
            return;
        }
        slice(self.strpool, start, b.len()).copy_from_slice(b);
    }

    pub fn colorstackset(&mut self, n: i32, s: i32) -> i32 {
        let v = Some(bytes_to_string(&self.tex_string(s).into_iter().take_while(|&c| c != 0).collect::<Vec<_>>()));
        let page = self.rt.pdfu.page_mode;
        let cs = &mut self.rt.pdfu.colstacks[n as usize];
        if page {
            cs.page_current = v;
        } else {
            cs.form_current = v;
        }
        cs.literal_mode
    }

    pub fn colorstackcurrent(&mut self, n: i32) -> i32 {
        let page = self.rt.pdfu.page_mode;
        let cs = &self.rt.pdfu.colstacks[n as usize];
        let cur = if page { cs.page_current.clone() } else { cs.form_current.clone() };
        let lm = cs.literal_mode;
        let p = self.poolptr;
        self.put_cstring_on_strpool(p, cur);
        lm
    }

    pub fn colorstackpush(&mut self, n: i32, s: i32) -> i32 {
        let v = self.cstr_of(s);
        let page = self.rt.pdfu.page_mode;
        let cs = &mut self.rt.pdfu.colstacks[n as usize];
        if page {
            let cur = cs.page_current.take();
            cs.page_stack.push(cur);
            cs.page_current = v;
        } else {
            let cur = cs.form_current.take();
            cs.form_stack.push(cur);
            cs.form_current = v;
        }
        let _ = STACK_INCREMENT;
        cs.literal_mode
    }

    pub fn colorstackpop(&mut self, n: i32) -> i32 {
        let page = self.rt.pdfu.page_mode;
        let lm = self.rt.pdfu.colstacks[n as usize].literal_mode;
        let empty = {
            let cs = &self.rt.pdfu.colstacks[n as usize];
            if page { cs.page_stack.is_empty() } else { cs.form_stack.is_empty() }
        };
        if empty {
            self.pdftex_warn(&format!("pop empty color {} stack {}", if page { "page" } else { "form" }, n as u32));
            return lm;
        }
        let cur = {
            let cs = &mut self.rt.pdfu.colstacks[n as usize];
            if page {
                cs.page_current = cs.page_stack.pop().unwrap();
                cs.page_current.clone()
            } else {
                cs.form_current = cs.form_stack.pop().unwrap();
                cs.form_current.clone()
            }
        };
        let p = self.poolptr;
        self.put_cstring_on_strpool(p, cur);
        lm
    }

    fn colorstackpagestart(&mut self) {
        if self.rt.pdfu.page_mode {
            return;
        }
        for cs in self.rt.pdfu.colstacks.iter_mut() {
            cs.form_stack.clear();
            cs.form_current = cs.form_init.clone();
        }
    }

    pub fn colorstackskippagestart(&mut self, n: i32) -> i32 {
        let cs = &self.rt.pdfu.colstacks[n as usize];
        if !cs.page_start {
            return 1;
        }
        match &cs.page_current {
            None => 0,
            Some(c) if c == COLOR_DEFAULT => 2,
            Some(_) => 0,
        }
    }

    // ------------------------------------------------------------------ matrix / save-restore
    pub fn matrixused(&mut self) -> bool {
        self.rt.pdfu.matrix_stack_used > 0
    }

    pub fn checkpdfsave(&mut self, cur_h: i32, cur_v: i32) {
        let ms = self.rt.pdfu.matrix_stack_used;
        let page = self.rt.pdfu.page_mode;
        let prev = PosEntry { pos_h: cur_h, pos_v: cur_v, matrix_stack: 0 };
        let mut e = prev;
        if page {
            e.matrix_stack = ms;
        }
        self.rt.pdfu.pos_stack.push(e);
    }

    pub fn checkpdfrestore(&mut self, cur_h: i32, cur_v: i32) {
        if self.rt.pdfu.pos_stack.is_empty() {
            self.pdftex_warn("\\pdfrestore: missing \\pdfsave");
            return;
        }
        let e = self.rt.pdfu.pos_stack.pop().unwrap();
        let diff_h = cur_h.wrapping_sub(e.pos_h);
        let diff_v = cur_v.wrapping_sub(e.pos_v);
        if diff_h != 0 || diff_v != 0 {
            self.pdftex_warn(&format!("Misplaced \\pdfrestore by ({}sp, {}sp)", diff_h as u32, diff_v as u32));
        }
        if self.rt.pdfu.page_mode {
            self.rt.pdfu.matrix_stack_used = e.matrix_stack;
        }
    }

    pub fn pdfshipoutbegin(&mut self, shipping_page: bool) {
        self.rt.pdfu.pos_stack.clear();
        self.rt.pdfu.page_mode = shipping_page;
        if shipping_page {
            self.colorstackpagestart();
        }
    }

    pub fn pdfshipoutend(&mut self, shipping_page: bool) {
        let n = self.rt.pdfu.pos_stack.len();
        if n > 0 {
            self.pdftex_fail(&format!(
                "{} unmatched \\pdfsave after {} shipout",
                n as u32,
                if shipping_page { "page" } else { "form" }
            ));
        }
    }

    pub fn pdfsetmatrix(&mut self, inp: i32, cur_h: i32, cur_v: i32) -> i32 {
        if self.rt.pdfu.page_mode {
            // sscanf(&strpool[in], " %lf %lf %lf %lf %c", ...) != 4 -> failure
            let mut buf: Vec<u8> = Vec::new();
            let mut k = inp;
            while k < self.poolptr {
                buf.push(*ix(self.strpool, k));
                k += 1;
            }
            // The C code reads the pool as a C string: it stops at a NUL or at the
            // first byte past pool_ptr that happens to be NUL; TeX leaves pool bytes
            // after pool_ptr untouched, so read on until a NUL.
            let mut k2 = self.poolptr;
            while k2 < self.poolsize && *ix(self.strpool, k2) != 0 {
                buf.push(*ix(self.strpool, k2));
                k2 += 1;
            }
            let buf: Vec<u8> = buf.into_iter().take_while(|&c| c != 0).collect();
            let mut pos = 0;
            let mut v = [0f64; 4];
            for slot in v.iter_mut() {
                match scan_double(&buf, &mut pos) {
                    Some(x) => *slot = x,
                    None => return 0,
                }
            }
            // " %c": if another non-space char follows, sscanf returns 5 -> failure
            while pos < buf.len() && is_c_space(buf[pos]) {
                pos += 1;
            }
            if pos < buf.len() {
                return 0;
            }
            let x = MatrixEntry {
                a: v[0],
                b: v[1],
                c: v[2],
                d: v[3],
                e: cur_h as f64 * (1.0 - v[0]) - cur_v as f64 * v[2],
                f: cur_v as f64 * (1.0 - v[3]) - cur_h as f64 * v[1],
            };
            let st = &mut self.rt.pdfu;
            let used = st.matrix_stack_used;
            let z = if used > 0 {
                let y = st.matrix_stack[used - 1];
                MatrixEntry {
                    a: x.a * y.a + x.b * y.c,
                    b: x.a * y.b + x.b * y.d,
                    c: x.c * y.a + x.d * y.c,
                    d: x.c * y.b + x.d * y.d,
                    e: x.e * y.a + x.f * y.c + y.e,
                    f: x.e * y.b + x.f * y.d + y.f,
                }
            } else {
                x
            };
            if st.matrix_stack.len() <= used {
                st.matrix_stack.resize(used + 1, MatrixEntry::default());
            }
            st.matrix_stack[used] = z;
            st.matrix_stack_used += 1;
        }
        1
    }

    pub fn getllx(&mut self) -> i32 {
        self.rt.pdfu.ret_llx
    }
    pub fn getlly(&mut self) -> i32 {
        self.rt.pdfu.ret_lly
    }
    pub fn geturx(&mut self) -> i32 {
        self.rt.pdfu.ret_urx
    }
    pub fn getury(&mut self) -> i32 {
        self.rt.pdfu.ret_ury
    }

    fn do_matrixtransform(&self, x: i32, y: i32) -> (i32, i32) {
        let st = &self.rt.pdfu;
        let m = st.matrix_stack[st.matrix_stack_used - 1];
        let (xo, yo) = (x as f64, y as f64);
        let xn = xo * m.a + yo * m.c + m.e;
        let yn = xo * m.b + yo * m.d + m.f;
        let r = |v: f64| (if v > 0.0 { v + 0.5 } else { v - 0.5 }) as i32;
        (r(xn), r(yn))
    }

    pub fn matrixtransformrect(&mut self, llx: i32, lly: i32, urx: i32, ury: i32) {
        if self.rt.pdfu.page_mode && self.rt.pdfu.matrix_stack_used > 0 {
            let st = &mut self.rt.pdfu;
            st.last_llx = llx;
            st.last_lly = lly;
            st.last_urx = urx;
            st.last_ury = ury;
            let (x1, y1) = self.do_matrixtransform(llx, lly);
            let (x2, y2) = self.do_matrixtransform(llx, ury);
            let (x3, y3) = self.do_matrixtransform(urx, lly);
            let (x4, y4) = self.do_matrixtransform(urx, ury);
            let st = &mut self.rt.pdfu;
            st.ret_llx = x1.min(x2).min(x3.min(x4));
            st.ret_lly = y1.min(y2).min(y3.min(y4));
            st.ret_urx = x1.max(x2).max(x3.max(x4));
            st.ret_ury = y1.max(y2).max(y3.max(y4));
        } else {
            let st = &mut self.rt.pdfu;
            st.ret_llx = llx;
            st.ret_lly = lly;
            st.ret_urx = urx;
            st.ret_ury = ury;
        }
    }

    pub fn matrixtransformpoint(&mut self, x: i32, y: i32) {
        if self.rt.pdfu.page_mode && self.rt.pdfu.matrix_stack_used > 0 {
            let (a, b) = self.do_matrixtransform(x, y);
            self.rt.pdfu.ret_llx = a;
            self.rt.pdfu.ret_lly = b;
        } else {
            self.rt.pdfu.ret_llx = x;
            self.rt.pdfu.ret_lly = y;
        }
    }

    pub fn matrixrecalculate(&mut self, urx: i32) {
        let (a, b, d) = (self.rt.pdfu.last_llx, self.rt.pdfu.last_lly, self.rt.pdfu.last_ury);
        self.matrixtransformrect(a, b, urx, d);
    }

    /// utils.c allocvffnts: grow vfefnts/vfifnts (alloc_array with limit font_max).
    pub fn allocvffnts(&mut self) {
        let st = &mut self.rt.pdfu;
        if !st.vf_fnts_init {
            st.vf_fnts_init = true;
            st.vf_fnts_limit = self.fontmax;
            st.vf_fnts_ptr = 0;
        }
        if (st.vf_fnts_ptr + 1) as u32 > st.vf_fnts_limit as u32 {
            st.vf_fnts_limit *= 2;
            if (st.vf_fnts_ptr + 1) as u32 > st.vf_fnts_limit as u32 {
                st.vf_fnts_limit = st.vf_fnts_ptr + 1;
            }
            // xretalloc to limit entries (not limit+1)
            let lim = st.vf_fnts_limit;
            self.vfefnts = self.vfefnts.xrealloc_array(lim - 1);
            self.vfifnts = self.vfifnts.xrealloc_array(lim - 1);
        }
        self.rt.pdfu.vf_fnts_ptr += 1;
    }
}

pub fn is_c_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// C strtod-like scan used for sscanf("%lf"): skips white space, reads the longest valid
/// decimal floating-point prefix (also inf/nan and hex floats), advances pos.
pub fn scan_double(buf: &[u8], pos: &mut usize) -> Option<f64> {
    let mut i = *pos;
    while i < buf.len() && is_c_space(buf[i]) {
        i += 1;
    }
    let start = i;
    if i < buf.len() && (buf[i] == b'+' || buf[i] == b'-') {
        i += 1;
    }
    let rest = &buf[i..];
    let lower: Vec<u8> = rest.iter().take(8).map(|c| c.to_ascii_lowercase()).collect();
    if lower.starts_with(b"infinity") || lower.starts_with(b"inf") || lower.starts_with(b"nan") {
        let n = if lower.starts_with(b"infinity") { 8 } else { 3 };
        let txt = std::str::from_utf8(&buf[start..i + n]).ok()?;
        *pos = i + n;
        return txt.parse::<f64>().ok().or(Some(if lower.starts_with(b"nan") { f64::NAN } else { f64::INFINITY }));
    }
    let digits_start = i;
    let mut saw_digit = false;
    while i < buf.len() && buf[i].is_ascii_digit() {
        i += 1;
        saw_digit = true;
    }
    if i < buf.len() && buf[i] == b'.' {
        i += 1;
        while i < buf.len() && buf[i].is_ascii_digit() {
            i += 1;
            saw_digit = true;
        }
    }
    if !saw_digit {
        let _ = digits_start;
        return None;
    }
    if i < buf.len() && (buf[i] == b'e' || buf[i] == b'E') {
        let mut j = i + 1;
        if j < buf.len() && (buf[j] == b'+' || buf[j] == b'-') {
            j += 1;
        }
        if j < buf.len() && buf[j].is_ascii_digit() {
            while j < buf.len() && buf[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    let mut txt = String::from_utf8_lossy(&buf[start..i]).into_owned();
    if txt.ends_with('.') {
        txt.push('0');
    }
    if txt.starts_with('.') || txt.starts_with("-.") || txt.starts_with("+.") {
        txt = txt.replacen('.', "0.", 1);
    }
    *pos = i;
    txt.parse::<f64>().ok()
}
