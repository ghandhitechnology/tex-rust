//! debug.h / debug.c and the WARNING / FATAL macros of lib.h.
//!
//! Everything kpathsea writes to `stderr` goes through [`ErrSink`], which
//! emulates the MSVC CRT text-mode stream (LF -> CR LF) and supports the
//! `KPATHSEA_DEBUG_OUTPUT` / `KPATHSEA_DEBUG_APPEND` redirection done in
//! progname.c.

use std::io::Write;

use crate::Kpse;

pub const KPSE_DEBUG_STAT: u32 = 0;
pub const KPSE_DEBUG_HASH: u32 = 1;
pub const KPSE_DEBUG_FOPEN: u32 = 2;
pub const KPSE_DEBUG_PATHS: u32 = 3;
pub const KPSE_DEBUG_EXPAND: u32 = 4;
pub const KPSE_DEBUG_SEARCH: u32 = 5;
pub const KPSE_DEBUG_VARS: u32 = 6;

/// Size of the CRT's stdio buffer (`_INTERNAL_BUFSIZ`).
const CRT_BUFSIZ: usize = 4096;

/// kpathsea's `stderr`.  Like the MSVC CRT stream it emulates, it is
/// buffered when it is not a console (so, e.g., "kpathsea: Running
/// mktextfm" reaches a redirected stderr only at the next flush, after the
/// child's own messages); `DEBUGF` and `WARNING` flush it, as in C.
pub(crate) enum ErrSink {
    Stderr { buf: Vec<u8>, console: bool },
    File(std::fs::File),
}

impl ErrSink {
    pub(crate) fn stderr() -> ErrSink {
        use std::io::IsTerminal;
        ErrSink::Stderr { buf: Vec::new(), console: std::io::stderr().is_terminal() }
    }

    pub(crate) fn write(&mut self, bytes: &[u8]) {
        // Text-mode translation as done by the CRT on Windows.
        let mut out: Vec<u8>;
        let data: &[u8] = if cfg!(windows) && bytes.contains(&b'\n') {
            out = Vec::with_capacity(bytes.len() + 8);
            for &b in bytes {
                if b == b'\n' {
                    out.push(b'\r');
                }
                out.push(b);
            }
            &out
        } else {
            bytes
        };
        match self {
            ErrSink::Stderr { buf, console } => {
                if *console {
                    let e = std::io::stderr();
                    let mut l = e.lock();
                    let _ = l.write_all(data);
                    let _ = l.flush();
                } else {
                    if buf.len() + data.len() > CRT_BUFSIZ {
                        flush_buf(buf);
                    }
                    if data.len() >= CRT_BUFSIZ {
                        let e = std::io::stderr();
                        let mut l = e.lock();
                        let _ = l.write_all(data);
                        let _ = l.flush();
                    } else {
                        buf.extend_from_slice(data);
                    }
                }
            }
            ErrSink::File(f) => {
                let _ = f.write_all(data);
            }
        }
    }

    /// Drop buffered output (what a crash does to a stdio buffer).
    pub(crate) fn discard(&mut self) {
        if let ErrSink::Stderr { buf, .. } = self {
            buf.clear();
        }
    }

    /// `fflush (stderr)`.
    pub(crate) fn flush(&mut self) {
        if let ErrSink::Stderr { buf, .. } = self {
            flush_buf(buf);
        }
    }
}

fn flush_buf(buf: &mut Vec<u8>) {
    if !buf.is_empty() {
        let e = std::io::stderr();
        let mut l = e.lock();
        let _ = l.write_all(buf);
        let _ = l.flush();
        buf.clear();
    }
}

impl Drop for ErrSink {
    fn drop(&mut self) {
        self.flush();
    }
}

/// `printf("%.1f")` as done by the UCRT (correctly rounded, ties away
/// from zero on the exact binary value).
pub(crate) fn fmt_f1(x: f64) -> String {
    let exact = format!("{:.80}", x.abs());
    let (int_part, frac) = exact.split_once('.').unwrap_or((&exact, ""));
    let frac = frac.as_bytes();
    let d1 = frac.first().map(|c| c - b'0').unwrap_or(0);
    let rest = &frac[1.min(frac.len())..];
    let round_up = match rest.first() {
        Some(&c) if c > b'5' => true,
        Some(&b'5') => true, // tie or above: away from zero
        _ => false,
    };
    let mut ip: u128 = int_part.parse().unwrap_or(0);
    let mut d = d1 as u32;
    if round_up {
        d += 1;
        if d == 10 {
            d = 0;
            ip += 1;
        }
    }
    let sign = if x.is_sign_negative() && (ip != 0 || d != 0) { "-" } else { "" };
    format!("{}{}.{}", sign, ip, d)
}

impl Kpse {
    /// `KPATHSEA_DEBUG_P (bit)`.
    #[inline]
    pub(crate) fn debug_p(&self, bit: u32) -> bool {
        self.debug & (1 << bit) != 0
    }

    /// Raw write to kpathsea's stderr.
    pub(crate) fn err_write(&mut self, bytes: &[u8]) {
        self.err.write(bytes);
    }

    /// Write to the emulated C `stderr` stream (buffered like the CRT's
    /// when redirected).  An engine that wants its own stderr output to
    /// interleave exactly as in C should write through this.
    pub fn stderr_write(&mut self, bytes: &[u8]) {
        self.err.write(bytes);
    }

    /// `fflush (stderr)`.  Call before exiting the process (the C runtime
    /// flushes at exit; dropping the `Kpse` also flushes).
    pub fn flush_stderr(&mut self) {
        self.err.flush();
    }

    pub(crate) fn discard_stderr(&mut self) {
        self.err.discard();
    }

    /// `DEBUGF*`: "kdebug:" prefix, then the message, then `fflush`.
    pub(crate) fn debugf(&mut self, msg: &[u8]) {
        let mut v = Vec::with_capacity(msg.len() + 7);
        v.extend_from_slice(b"kdebug:");
        v.extend_from_slice(msg);
        self.err.write(&v);
        self.err.flush();
    }

    /// `WARNING*`: "warning: " + message + ".\n", then `fflush`.
    pub(crate) fn warning(&mut self, msg: &[u8]) {
        let mut v = Vec::with_capacity(msg.len() + 12);
        v.extend_from_slice(b"warning: ");
        v.extend_from_slice(msg);
        v.extend_from_slice(b".\n");
        self.err.write(&v);
        self.err.flush();
    }

    /// `hash_lookup` tracing, which in C uses the global `kpse_def`
    /// instance and therefore only appears in programs using the
    /// compatibility API (like pdfTeX), never in kpsewhich.
    pub(crate) fn trace_hash_lookup(&mut self, key: &[u8], values: &[&[u8]], as_int: bool) {
        if !self.compat_def || !self.debug_p(KPSE_DEBUG_HASH) {
            return;
        }
        let mut v = bcat!(b"hash_lookup(", key, b") =>");
        if values.is_empty() {
            v.extend_from_slice(b" (nil)\n");
        } else {
            for val in values {
                v.push(b' ');
                if as_int {
                    // C prints the pointer stored in the table; we print the
                    // stored link count instead (pointer values are not
                    // reproducible anyway).
                    v.extend_from_slice(val);
                } else {
                    v.extend_from_slice(val);
                }
            }
            v.push(b'\n');
        }
        self.debugf(&v);
    }

    /// The fopen trace of debug.c (`fopen(%s, %s) => 0x%I64x`); only for
    /// the compatibility instance.  The handle value is not reproducible;
    /// we print a fixed placeholder.
    pub(crate) fn trace_fopen(&mut self, name: &[u8], mode: &[u8], ok: bool) {
        if self.compat_def && self.debug_p(KPSE_DEBUG_FOPEN) {
            let m = bcat!(b"fopen(", name, b", ", mode, b") => ", if ok { "0x1" } else { "0x0" }, b"\n");
            self.debugf(&m);
        }
    }

    pub(crate) fn trace_fclose(&mut self) {
        if self.compat_def && self.debug_p(KPSE_DEBUG_FOPEN) {
            self.debugf(b"fclose(0x1) => 0\n");
        }
    }

    /// `LIB_FATAL*`: "<invocation_name>: fatal: " + message + ".\n", exit 1.
    pub(crate) fn fatal(&mut self, msg: &[u8]) -> ! {
        let m = bcat!(&self.invocation_name, b": fatal: ", msg, b".\n");
        self.err.write(&m);
        self.err.flush();
        std::process::exit(1);
    }

    /// `perror (name)` (writes straight to the file descriptor).
    pub(crate) fn perror(&mut self, name: &[u8], err: &std::io::Error) {
        let msg = match err.kind() {
            std::io::ErrorKind::NotFound => "No such file or directory".to_string(),
            std::io::ErrorKind::PermissionDenied => "Permission denied".to_string(),
            _ => err.to_string(),
        };
        self.err.flush();
        let m = bcat!(name, b": ", msg, b"\n");
        self.err.write(&m);
        self.err.flush();
    }
}
