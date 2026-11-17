//! File handles for the translated Pascal: web2c's FILE* world (lib/openclose.c,
//! lib/texmfmp.c input_line/open_in_or_pipe, lib/eofeoln.c, cpascal.h write macros).
//!
//! Input files are read whole into memory (TeX reads them sequentially anyway); output
//! files are buffered writers. stdout/stderr behave like the C runtime's text-mode
//! streams on Windows: "\n" is written as "\r\n".

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::process::{Child, ChildStdin, Command, Stdio};

use super::*;
use crate::gen::globals::Globals;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct FileRef(pub u32);

impl FileRef {
    pub const NULL: FileRef = FileRef(0);
    pub const STDIN: FileRef = FileRef(1);
    pub const STDOUT: FileRef = FileRef(2);
    pub const STDERR: FileRef = FileRef(3);
    pub fn is_null(self) -> bool {
        self.0 == 0
    }
}

pub enum Handle {
    /// An input file (or the complete output of an input pipe), read whole.
    In { data: Vec<u8>, pos: usize, pipe: bool, eof: bool },
    Out { w: std::io::BufWriter<File>, pos: u64 },
    PipeOut { child: Child, stdin: Option<ChildStdin> },
    /// A format file being written or read (gzip, see dump.rs).
    FmtOut(Box<dyn super::dump::FmtWriter>),
    FmtIn(Box<dyn super::dump::FmtReader>),
}

pub struct FileTable {
    pub slots: Vec<Option<Handle>>,
    /// kpathsea lives here so stderr can go through its emulation of the C runtime's
    /// stderr stream (shared by kpathsea's own messages and the engine's).
    pub kpse: Option<kpse::Kpse>,
    pub stdin_buf: Vec<u8>,
    pub stdin_pos: usize,
    pub stdin_eof: bool,
    pub stdout_buf: Vec<u8>,
}

impl Default for FileTable {
    fn default() -> Self {
        FileTable { slots: vec![None, None, None, None], kpse: None, stdin_buf: Vec::new(), stdin_pos: 0, stdin_eof: false, stdout_buf: Vec::with_capacity(8192) }
    }
}

impl FileTable {
    pub fn add(&mut self, h: Handle) -> FileRef {
        for (i, s) in self.slots.iter_mut().enumerate().skip(4) {
            if s.is_none() {
                *s = Some(h);
                return FileRef(i as u32);
            }
        }
        self.slots.push(Some(h));
        FileRef((self.slots.len() - 1) as u32)
    }

    fn fill_stdin(&mut self) -> bool {
        if self.stdin_eof {
            return false;
        }
        self.flush_stdout();
        let mut line = Vec::new();
        let stdin = std::io::stdin();
        let mut lock = stdin.lock();
        let mut b = [0u8; 1];
        loop {
            match lock.read(&mut b) {
                Ok(0) => {
                    if line.is_empty() {
                        self.stdin_eof = true;
                        return false;
                    }
                    break;
                }
                Ok(_) => {
                    line.push(b[0]);
                    if b[0] == b'\n' {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    self.stdin_eof = true;
                    return !line.is_empty();
                }
            }
        }
        self.stdin_buf.drain(..self.stdin_pos);
        self.stdin_pos = 0;
        self.stdin_buf.extend_from_slice(&line);
        true
    }

    /// C getc: next byte or -1.
    #[inline]
    pub fn getc(&mut self, f: FileRef) -> i32 {
        if f == FileRef::STDIN {
            if self.stdin_pos >= self.stdin_buf.len() && !self.fill_stdin() {
                return -1;
            }
            let c = self.stdin_buf[self.stdin_pos];
            self.stdin_pos += 1;
            return c as i32;
        }
        match self.slots.get_mut(f.0 as usize) {
            Some(Some(Handle::In { data, pos, eof, .. })) => {
                if *pos < data.len() {
                    let c = data[*pos];
                    *pos += 1;
                    c as i32
                } else {
                    *eof = true;
                    -1
                }
            }
            _ => -1,
        }
    }

    pub fn ungetc(&mut self, c: i32, f: FileRef) {
        if c < 0 {
            return;
        }
        if f == FileRef::STDIN {
            if self.stdin_pos > 0 {
                self.stdin_pos -= 1;
                self.stdin_buf[self.stdin_pos] = c as u8;
            }
            return;
        }
        if let Some(Some(Handle::In { data, pos, eof, .. })) = self.slots.get_mut(f.0 as usize) {
            *eof = false;
            if *pos > 0 {
                *pos -= 1;
                data[*pos] = c as u8;
            }
        }
    }

    pub fn tell(&mut self, f: FileRef) -> i64 {
        match self.slots.get_mut(f.0 as usize) {
            Some(Some(Handle::In { pos, .. })) => *pos as i64,
            Some(Some(Handle::Out { pos, .. })) => *pos as i64,
            _ => 0,
        }
    }

    pub fn rewind(&mut self, f: FileRef) {
        if let Some(Some(Handle::In { pos, eof, .. })) = self.slots.get_mut(f.0 as usize) {
            *pos = 0;
            *eof = false;
        }
    }

    pub fn is_pipe(&self, f: FileRef) -> bool {
        matches!(self.slots.get(f.0 as usize), Some(Some(Handle::In { pipe: true, .. })) | Some(Some(Handle::PipeOut { .. })))
    }

    /// Remaining bytes of an input file (fast paths).
    pub fn in_data(&mut self, f: FileRef) -> Option<(&mut Vec<u8>, &mut usize, &mut bool)> {
        match self.slots.get_mut(f.0 as usize) {
            Some(Some(Handle::In { data, pos, eof, .. })) => Some((data, pos, eof)),
            _ => None,
        }
    }

    pub fn flush_stdout(&mut self) {
        if !self.stdout_buf.is_empty() {
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(&self.stdout_buf);
            let _ = out.flush();
            self.stdout_buf.clear();
        }
    }

    pub fn write(&mut self, f: FileRef, bytes: &[u8]) {
        match f {
            FileRef::STDOUT => {
                for &b in bytes {
                    if b == b'\n' {
                        self.stdout_buf.push(b'\r');
                    }
                    self.stdout_buf.push(b);
                }
                if self.stdout_buf.len() > 1 << 16 {
                    self.flush_stdout();
                }
            }
            FileRef::STDERR => match self.kpse.as_mut() {
                Some(k) => k.stderr_write(bytes),
                None => {
                    let mut v = Vec::with_capacity(bytes.len() + 4);
                    for &b in bytes {
                        if b == b'\n' {
                            v.push(b'\r');
                        }
                        v.push(b);
                    }
                    let _ = std::io::stderr().write_all(&v);
                }
            },
            _ => match self.slots.get_mut(f.0 as usize) {
                Some(Some(Handle::Out { w, pos })) => {
                    if w.write_all(bytes).is_err() {
                        texmfmp::fatal_perror("fwrite");
                    }
                    *pos += bytes.len() as u64;
                }
                Some(Some(Handle::PipeOut { stdin: Some(s), .. })) => {
                    // popen(cmd, "w") is a text-mode stream on Windows: LF goes out as CR LF
                    let mut v = Vec::with_capacity(bytes.len() + 4);
                    for &b in bytes {
                        if b == b'\n' {
                            v.push(b'\r');
                        }
                        v.push(b);
                    }
                    let _ = s.write_all(&v);
                }
                Some(Some(Handle::FmtOut(w))) => w.write(bytes),
                _ => {}
            },
        }
    }

    pub fn flush(&mut self, f: FileRef) {
        match f {
            FileRef::STDOUT => self.flush_stdout(),
            FileRef::STDERR => {}
            _ => {
                if let Some(Some(Handle::Out { w, .. })) = self.slots.get_mut(f.0 as usize) {
                    let _ = w.flush();
                }
            }
        }
    }

    pub fn seek_out(&mut self, f: FileRef, off: u64) {
        if let Some(Some(Handle::Out { w, pos })) = self.slots.get_mut(f.0 as usize) {
            let _ = w.seek(SeekFrom::Start(off));
            *pos = off;
        }
    }

    pub fn close(&mut self, f: FileRef) {
        if (f.0 as usize) < 4 {
            return;
        }
        if let Some(slot) = self.slots.get_mut(f.0 as usize) {
            match slot.take() {
                Some(Handle::Out { mut w, .. }) => {
                    if w.flush().is_err() {
                        eprintln!("fclose: write error");
                    }
                }
                Some(Handle::PipeOut { mut child, stdin }) => {
                    drop(stdin);
                    let _ = child.wait();
                }
                Some(Handle::FmtOut(mut w)) => w.finish(),
                _ => {}
            }
        }
    }
}

/// Open a file for reading into memory.
pub fn read_file(path: &std::path::Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

/// How much of a format file open_input reads: the format cache needs only the start of
/// the file (rt/dump.rs wopenin reads the rest when the cache cannot serve it).
pub const FMT_HEAD: usize = 4096;

/// Up to `n` bytes from the start of a file.
fn read_prefix(path: &std::path::Path, n: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut f = File::open(path).ok()?;
    let mut v = vec![0u8; n];
    let mut got = 0;
    while got < n {
        match f.read(&mut v[got..]) {
            Ok(0) => break,
            Ok(k) => got += k,
            Err(_) => return None,
        }
    }
    v.truncate(got);
    Some(v)
}

fn read_input(path: &std::path::Path, filefmt: i32) -> Option<Vec<u8>> {
    if filefmt == kpse::FMT_FORMAT {
        read_prefix(path, FMT_HEAD)
    } else {
        read_file(path)
    }
}

/// openclose.c's recorder (`-recorder`, the .fls file). Shared with kpathsea's
/// record_input/record_output callbacks, which is why it sits behind Rc<RefCell>.
#[derive(Default)]
pub struct Recorder {
    pub enabled: bool,
    pub file: Option<std::io::BufWriter<File>>,
    pub name: Option<std::path::PathBuf>,
    pub output_directory: Option<Vec<u8>>,
    /// kpse_program_name, used for the initial file name
    pub progname: Vec<u8>,
}

fn join_dir(dir: &[u8], name: &[u8]) -> Vec<u8> {
    let mut v = dir.to_vec();
    v.push(b'/');
    v.extend_from_slice(name);
    v
}

impl Recorder {
    /// openclose.c recorder_start
    fn start(&mut self) {
        let mut name = self.progname.clone();
        name.extend_from_slice(format!("{}.fls", std::process::id()).as_bytes());
        if let Some(od) = &self.output_directory {
            name = join_dir(od, &name);
        }
        let path = std::path::PathBuf::from(bytes_to_string(&name));
        match File::create(&path) {
            Ok(f) => {
                // xgetcwd: the current directory with '/' separators
                let cwd = std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
                let mut w = std::io::BufWriter::new(f);
                let _ = w.write_all(b"PWD ");
                let _ = w.write_all(cwd.replace('\\', "/").as_bytes());
                let _ = w.write_all(b"\n");
                self.file = Some(w);
                self.name = Some(path);
            }
            Err(_) => texmfmp::fatal(&format!("fopen({}) failed", bytes_to_string(&name))),
        }
    }

    pub fn record(&mut self, prefix: &[u8], name: &[u8]) {
        if self.enabled {
            if self.file.is_none() {
                self.start();
            }
            if let Some(w) = self.file.as_mut() {
                let _ = w.write_all(prefix);
                let _ = w.write_all(b" ");
                let _ = w.write_all(name);
                let _ = w.write_all(b"\n");
                let _ = w.flush();
            }
        }
    }

    /// openclose.c recorder_change_filename (WIN32: close, remove target, rename, reopen).
    pub fn change_filename(&mut self, new_name: &[u8]) {
        if self.file.is_none() {
            return;
        }
        self.file = None;
        let new_name = match &self.output_directory {
            Some(od) => join_dir(od, new_name),
            None => new_name.to_vec(),
        };
        let path = std::path::PathBuf::from(bytes_to_string(&new_name));
        let _ = std::fs::remove_file(&path);
        if let Some(old) = &self.name {
            let _ = std::fs::rename(old, &path);
        }
        match std::fs::OpenOptions::new().append(true).create(true).open(&path) {
            Ok(f) => self.file = Some(std::io::BufWriter::new(f)),
            Err(_) => texmfmp::fatal(&format!("fopen({}) failed", bytes_to_string(&new_name))),
        }
        self.name = Some(path);
    }

    pub fn flush(&mut self) {
        if let Some(w) = self.file.as_mut() {
            let _ = w.flush();
        }
    }
}

impl Globals {
    // ---------------------------------------------------------------- nameoffile helpers
    /// `nameoffile+1` as bytes (up to the NUL).
    pub fn name_of_file(&self) -> Vec<u8> {
        if self.nameoffile.is_null() {
            return Vec::new();
        }
        self.nameoffile.off(1).cstr_bytes().to_vec()
    }

    pub fn name_of_file_str(&self) -> String {
        bytes_to_string(&self.name_of_file())
    }

    /// Replace nameoffile with `s` (1-based, NUL-terminated), as openclose.c does.
    pub fn set_name_of_file(&mut self, s: &[u8]) {
        self.nameoffile.free();
        self.namelength = s.len() as i32;
        let p = Ptr::<u8>::alloc_count(s.len() as i64 + 2);
        for (i, &b) in s.iter().enumerate() {
            *ix(p, i as i32 + 1) = b;
        }
        self.nameoffile = p;
    }

    /// The OS path for a TeX file name (kpathsea's file-system code page rules).
    pub fn os_path(&mut self, name: &[u8]) -> std::path::PathBuf {
        self.rt.kpse().os_path(name)
    }

    // ---------------------------------------------------------------- recorder
    pub fn recorder_record_input(&mut self, name: &[u8]) {
        self.rt.recorder.borrow_mut().record(b"INPUT", name);
    }

    pub fn recorder_record_output(&mut self, name: &[u8]) {
        self.rt.recorder.borrow_mut().record(b"OUTPUT", name);
    }

    pub fn recorderchangefilename(&mut self, new_name: Ptr<u8>) {
        let n = new_name.cstr_bytes().to_vec();
        self.rt.recorder.borrow_mut().change_filename(&n);
    }

    // ---------------------------------------------------------------- open/close (openclose.c)
    /// open_input: the file name is in nameoffile+1.
    pub fn open_input(&mut self, filefmt: i32) -> (FileRef, bool) {
        self.rt.fullnameoffile = None;
        let name = self.name_of_file();
        let mut found: Option<(Vec<u8>, Vec<u8>)> = None;
        if let Some(od) = self.rt.output_directory.clone() {
            if !kpse::absolute_p(&name, false) {
                let fname = join_dir(od.as_bytes(), &name);
                let p = self.os_path(&fname);
                if let Some(data) = read_input(&p, filefmt) {
                    self.rt.fullnameoffile = Some(fname.clone());
                    found = Some((fname, data));
                }
            }
        }
        if found.is_none() {
            if filefmt < 0 {
                let p = self.os_path(&name);
                if let Some(data) = read_file(&p) {
                    found = Some((name.clone(), data));
                }
            } else {
                let must_exist = (filefmt != kpse::TEX_FORMAT || self.texinputtype != 0) && filefmt != kpse::VF_FORMAT;
                if let Some(mut fname) = self.rt.kpse().find_file(&name, filefmt, must_exist) {
                    self.rt.fullnameoffile = Some(fname.clone());
                    if fname.len() >= 2 && fname[0] == b'.' && is_dir_sep(fname[1])
                        && !(name.len() >= 2 && name[0] == b'.' && is_dir_sep(name[1]))
                    {
                        fname.drain(..2);
                    }
                    let p = self.os_path(&fname);
                    match read_input(&p, filefmt) {
                        Some(data) => found = Some((fname, data)),
                        None => texmfmp::fatal_perror(&bytes_to_string(&fname)),
                    }
                }
            }
        }
        match found {
            Some((fname, data)) => {
                self.set_name_of_file(&fname);
                let f = self.rt.files.add(Handle::In { data, pos: 0, pipe: false, eof: false });
                self.recorder_record_input(&fname);
                if filefmt == kpse::TFM_FORMAT || filefmt == kpse::OFM_FORMAT {
                    self.tfmtemp = self.rt.files.getc(f);
                }
                (f, true)
            }
            None => (FileRef::NULL, false),
        }
    }

    /// open_output: the file name is in nameoffile+1.
    pub fn open_output(&mut self) -> (FileRef, bool) {
        let name = self.name_of_file();
        let absolute = kpse::absolute_p(&name, false);
        let mut fname = name.clone();
        if let Some(od) = self.rt.output_directory.clone() {
            if !absolute {
                fname = join_dir(od.as_bytes(), &name);
            }
        }
        let p = self.os_path(&fname);
        let mut file = File::create(&p).ok();
        if file.is_none() {
            if let Some(texmfoutput) = self.rt.kpse().var_value("TEXMFOUTPUT") {
                if !texmfoutput.is_empty() && !absolute {
                    fname = join_dir(texmfoutput.as_bytes(), &name);
                    let p = self.os_path(&fname);
                    file = File::create(&p).ok();
                }
            }
        }
        match file {
            Some(f) => {
                if fname != name {
                    self.set_name_of_file(&fname);
                }
                self.recorder_record_output(&fname);
                let h = self.rt.files.add(Handle::Out { w: std::io::BufWriter::with_capacity(1 << 16, f), pos: 0 });
                (h, true)
            }
            None => (FileRef::NULL, false),
        }
    }

    pub fn open_in_or_pipe(&mut self, filefmt: i32) -> (FileRef, bool) {
        let name = self.name_of_file();
        if self.shellenabledp != 0 && name.first() == Some(&b'|') {
            let cmd = bytes_to_string(&name[1..]);
            self.rt.fullnameoffile = Some(name.clone());
            self.recorder_record_input(&name[1..]);
            match self.runpopen_read(&cmd) {
                Some(data) => {
                    let f = self.rt.files.add(Handle::In { data, pos: 0, pipe: true, eof: false });
                    return (f, true);
                }
                None => return (FileRef::NULL, false),
            }
        }
        self.open_input(filefmt)
    }

    pub fn open_out_or_pipe(&mut self) -> (FileRef, bool) {
        let name = self.name_of_file();
        if self.shellenabledp != 0 && name.first() == Some(&b'|') {
            let mut fname = bytes_to_string(&name);
            if !fname.contains(' ') && !fname.contains('>') && fname.ends_with(".tex") {
                fname.truncate(fname.len() - 4);
            }
            let child = self.runpopen_write(&fname[1..]);
            self.recorder_record_output(&name[1..]);
            return match child {
                Some(mut c) => {
                    let stdin = c.stdin.take();
                    (self.rt.files.add(Handle::PipeOut { child: c, stdin }), true)
                }
                None => (FileRef::NULL, false),
            };
        }
        self.open_output()
    }

    pub fn bopenin(&mut self) -> (FileRef, bool) {
        self.open_input(kpse::TFM_FORMAT)
    }

    /// pdftex.h texbopenin / vfbopenin
    pub fn texbopenin(&mut self) -> (FileRef, bool) {
        self.open_input(kpse::TEX_FORMAT)
    }
    pub fn vfbopenin(&mut self) -> (FileRef, bool) {
        self.open_input(kpse::VF_FORMAT)
    }

    pub fn bopenout(&mut self) -> (FileRef, bool) {
        self.open_output()
    }

    // ---------------------------------------------------------------- closes
    pub fn aclose(&mut self, f: FileRef) {
        self.rt.files.close(f);
    }
    pub fn bclose(&mut self, f: FileRef) {
        self.rt.files.close(f);
    }

    // ---------------------------------------------------------------- reading
    pub fn getc(&mut self, f: FileRef) -> i32 {
        self.rt.files.getc(f)
    }

    /// web2c `eof(f)` (lib/eofeoln.c: test_eof): true at end of file.
    pub fn eof(&mut self, f: FileRef) -> bool {
        if f.is_null() {
            return true;
        }
        let c = self.rt.files.getc(f);
        if c < 0 {
            return true;
        }
        self.rt.files.ungetc(c, f);
        false
    }

    /// C feof: true only after a read hit the end of the file.
    pub fn feof(&mut self, f: FileRef) -> bool {
        match self.rt.files.slots.get(f.0 as usize) {
            Some(Some(Handle::In { eof, .. })) => *eof,
            _ => false,
        }
    }

    /// texmfmp.c input_line (non-XeTeX, WIN32 build).
    pub fn inputln(&mut self, f: FileRef, _bypass: bool) -> bool {
        // Windows: skip byte order marks at the start of a file (not stdin, not pipes).
        if f != FileRef::STDIN && !self.rt.files.is_pipe(f) && self.rt.files.tell(f) == 0 {
            let k1 = self.rt.files.getc(f);
            if k1 != 0xff && k1 != 0xfe && k1 != 0xef {
                self.rt.files.rewind(f);
            } else {
                let k2 = self.rt.files.getc(f);
                if k2 != 0xff && k2 != 0xfe && k2 != 0xbb {
                    self.rt.files.rewind(f);
                } else if (k1 == 0xff && k2 == 0xfe) || (k1 == 0xfe && k2 == 0xff) {
                    // UTF-16 BOM: skipped, nothing else done
                } else {
                    let k3 = self.rt.files.getc(f);
                    let k4 = self.rt.files.getc(f);
                    if k1 == 0xef && k2 == 0xbb && k3 == 0xbf && (0..=0x7e).contains(&k4) {
                        self.rt.files.ungetc(k4, f);
                    } else {
                        self.rt.files.rewind(f);
                    }
                }
            }
        }
        let first = self.first;
        let bufsize = self.bufsize;
        let buffer = self.buffer;
        let mut last = first;
        let mut i: i32;
        // fast path over in-memory data
        if let Some((data, pos, eofflag)) = self.rt.files.in_data(f) {
            let mut p = *pos;
            i = -1;
            while last < bufsize {
                if p >= data.len() {
                    i = -1;
                    *eofflag = true;
                    break;
                }
                let c = data[p];
                p += 1;
                if c == b'\n' || c == b'\r' {
                    i = c as i32;
                    break;
                }
                *ix(buffer, last) = c;
                last += 1;
                i = c as i32;
            }
            if last >= bufsize && p < data.len() && i != b'\n' as i32 && i != b'\r' as i32 {
                // buffer full: C reads no further char; the check below reports it
                i = 0;
            }
            *pos = p;
        } else {
            i = -1;
            loop {
                if last >= bufsize {
                    i = 0;
                    break;
                }
                i = self.rt.files.getc(f);
                if i < 0 || i == b'\n' as i32 || i == b'\r' as i32 {
                    break;
                }
                *ix(buffer, last) = i as u8;
                last += 1;
            }
        }
        self.last = last;
        if i < 0 && last == first {
            return false;
        }
        if i >= 0 && i != b'\n' as i32 && i != b'\r' as i32 {
            self.w_cstr_err(&format!("! Unable to read an entire line---bufsize={}.\n", bufsize as u32));
            self.w_cstr_err("Please increase buf_size in texmf.cnf.\n");
            self.x_uexit(1);
        }
        *ix(buffer, self.last) = b' ';
        if self.last >= self.maxbufstack {
            self.maxbufstack = self.last;
        }
        if i == b'\r' as i32 {
            let c = self.rt.files.getc(f);
            if c != b'\n' as i32 {
                self.rt.files.ungetc(c, f);
            }
        }
        while self.last > first && *ix(buffer, self.last - 1) == b' ' {
            self.last -= 1;
        }
        let xord = self.xord;
        for k in first..=self.last {
            let c = *ix(buffer, k);
            *ix(buffer, k) = *ix(xord, c as i32);
        }
        true
    }

    // ---------------------------------------------------------------- Pascal write/writeln (fixwrites)
    pub fn w_char(&mut self, f: FileRef, c: i32) {
        self.rt.files.write(f, &[c as u8]);
    }
    pub fn w_str(&mut self, f: FileRef, s: &[u8]) {
        self.rt.files.write(f, s);
    }
    pub fn w_cstr(&mut self, f: FileRef, s: Ptr<u8>) {
        let b = s.cstr_bytes().to_vec();
        self.rt.files.write(f, &b);
    }
    pub fn w_int(&mut self, f: FileRef, v: i64) {
        let s = v.to_string();
        self.rt.files.write(f, s.as_bytes());
    }
    pub fn w_int_width(&mut self, f: FileRef, v: i64, w: i32) {
        let s = format!("{:>width$}", v, width = w.max(0) as usize);
        self.rt.files.write(f, s.as_bytes());
    }
    pub fn w_ln(&mut self, f: FileRef) {
        self.rt.files.write(f, b"\n");
    }
    pub fn w_cstr_err(&mut self, s: &str) {
        self.rt.files.write(FileRef::STDERR, s.as_bytes());
    }
    pub fn fflush(&mut self, f: FileRef) {
        self.rt.files.flush(f);
    }
    pub fn putbyte(&mut self, x: i32, f: FileRef) {
        self.rt.files.write(f, &[(x & 255) as u8]);
    }
    pub fn fputs(&mut self, s: Ptr<u8>, f: FileRef) {
        let b = s.cstr_bytes().to_vec();
        self.rt.files.write(f, &b);
    }
    /// cpascal.h printcstring: print each char through TeX's print.
    pub fn printcstring(&mut self, s: Ptr<u8>) {
        let b = s.cstr_bytes().to_vec();
        for c in b {
            self.print(c as i32);
        }
    }

    /// texmfmp.h WRITE_OUT for the DVI buffer.
    pub fn writedvi(&mut self, a: i32, b: i32) {
        let n = (b - a + 1).max(0) as usize;
        let s = slice(self.dvibuf, a, n).to_vec();
        let f = self.dvifile;
        self.rt.files.write(f, &s);
    }

    // ---------------------------------------------------------------- pipes (texmfmp.c runpopen)
    /// system()/popen() on Windows run `cmd /c CMD`; the child sees the environment
    /// changes made with putenv (kpathsea snapshots the environment at start-up).
    fn shell_command(&mut self, cmd: &str) -> Command {
        let mut c = Command::new("cmd");
        use std::os::windows::process::CommandExt;
        c.raw_arg("/c").raw_arg(cmd);
        for (k, v) in self.rt.kpse().env_changes() {
            let k = bytes_to_string(&k);
            match v {
                Some(v) => {
                    c.env(k, bytes_to_string(&v));
                }
                None => {
                    c.env_remove(k);
                }
            }
        }
        c
    }

    /// texmfmp.c runpopen's permission check (WIN32: ' becomes " first).
    fn allowed_command(&mut self, cmd: &str) -> Option<String> {
        let cmd = cmd.replace('\'', "\"");
        if self.restrictedshell == 0 {
            return Some(cmd);
        }
        match texmfmp::shell_cmd_is_allowed(&cmd, self.rt.cmdlist.as_deref()) {
            (2, Some(safe)) => Some(safe),
            (-1, _) => {
                self.w_cstr_err(&format!("\nrunpopen quotation error in command line: {}\n", cmd));
                None
            }
            _ => {
                let name = cmd.trim_start_matches([' ', '\t']).split([' ', '\t']).next().unwrap_or("").to_string();
                self.w_cstr_err(&format!("\nrunpopen command not allowed: {}\n", name));
                None
            }
        }
    }

    /// popen(cmd, "r") on Windows: text mode, so CR LF becomes LF and ^Z ends the data.
    pub fn runpopen_read(&mut self, cmd: &str) -> Option<Vec<u8>> {
        let safe = self.allowed_command(cmd)?;
        self.rt.files.flush_stdout();
        let out = self.shell_command(&safe).stdout(Stdio::piped()).stdin(Stdio::inherit()).output().ok()?;
        let raw = out.stdout;
        let mut v = Vec::with_capacity(raw.len());
        let mut i = 0;
        while i < raw.len() {
            let c = raw[i];
            if c == 0x1a {
                break;
            }
            if c == b'\r' && raw.get(i + 1) == Some(&b'\n') {
                i += 1;
                continue;
            }
            v.push(c);
            i += 1;
        }
        Some(v)
    }

    pub fn runpopen_write(&mut self, cmd: &str) -> Option<Child> {
        let safe = self.allowed_command(cmd)?;
        self.rt.files.flush_stdout();
        self.shell_command(&safe).stdin(Stdio::piped()).spawn().ok()
    }

    /// texmfmp.c runsystem.
    pub fn runsystem(&mut self, cmd: Ptr<u8>) -> i32 {
        if self.shellenabledp <= 0 {
            return 0;
        }
        let cmd = bytes_to_string(cmd.cstr_bytes());
        let (allow, safecmd) = if self.restrictedshell == 0 {
            (1, Some(cmd.clone()))
        } else {
            texmfmp::shell_cmd_is_allowed(&cmd, self.rt.cmdlist.as_deref())
        };
        let mut status = 0;
        if allow == 1 || allow == 2 {
            let c = safecmd.unwrap_or_default();
            if allow == 2 && c.contains('|') {
                return 0;
            }
            self.rt.files.flush_stdout();
            status = self.shell_command(&c).status().map(|s| s.code().unwrap_or(1)).unwrap_or(-1);
        }
        if status != 0 {
            self.w_cstr_err(&format!("system returned with code {}\n", status));
        }
        allow
    }
}

#[inline]
pub fn is_dir_sep(c: u8) -> bool {
    c == b'/' || c == b'\\'
}

/// File names are bytes in TeX; on Windows TeX Live treats them as UTF-8.
pub fn bytes_to_string(b: &[u8]) -> String {
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => b.iter().map(|&c| c as char).collect(),
    }
}
