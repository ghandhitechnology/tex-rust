//! Port of xpdf `Error.h` / `Error.cc` (with TeX Live's `libxpdf:` label).
//!
//! xpdf reports problems through a global `error()` function.  pdfTeX
//! calls `globalParams->setErrQuiet(gFalse)` and installs no callback, so
//! every message goes to stderr as
//! `"\nlibxpdf: <Category> (<pos>): <msg>\n"` (or without ` (<pos>)` when
//! `pos < 0`).  This module reproduces that, and additionally lets the
//! embedding program install a callback (xpdf's `setErrorCallback`) or
//! silence the output.

use std::cell::RefCell;

/// Mirrors `enum ErrorCategory` (Error.h).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCategory {
    SyntaxWarning,
    SyntaxError,
    Config,
    CommandLine,
    IO,
    NotAllowed,
    Unimplemented,
    Internal,
}

impl ErrorCategory {
    /// Mirrors `errorCategoryNames[]`.
    pub fn name(self) -> &'static str {
        match self {
            ErrorCategory::SyntaxWarning => "Syntax Warning",
            ErrorCategory::SyntaxError => "Syntax Error",
            ErrorCategory::Config => "Config Error",
            ErrorCategory::CommandLine => "Command Line Error",
            ErrorCategory::IO => "I/O Error",
            ErrorCategory::NotAllowed => "Permission Error",
            ErrorCategory::Unimplemented => "Unimplemented Feature",
            ErrorCategory::Internal => "Internal Error",
        }
    }
}

/// Callback type: (category, position, sanitized message).
pub type ErrorCallback = Box<dyn FnMut(ErrorCategory, i64, &str)>;

thread_local! {
    static CALLBACK: RefCell<Option<ErrorCallback>> = const { RefCell::new(None) };
    static QUIET: RefCell<bool> = const { RefCell::new(false) };
}

/// Mirrors `setErrorCallback()`.  Pass `None` to restore stderr output.
pub fn set_error_callback(cbk: Option<ErrorCallback>) {
    CALLBACK.with(|c| *c.borrow_mut() = cbk);
}

/// Mirrors `GlobalParams::setErrQuiet()` (pdfTeX sets it to false).
pub fn set_err_quiet(quiet: bool) {
    QUIET.with(|q| *q.borrow_mut() = quiet);
}

/// Mirrors xpdf's `error(category, pos, msg, ...)`.  The message is
/// already formatted by the caller.
pub fn error(category: ErrorCategory, pos: i64, msg: &str) {
    let has_cbk = CALLBACK.with(|c| c.borrow().is_some());
    if !has_cbk && QUIET.with(|q| *q.borrow()) {
        return;
    }
    // remove non-printable characters (bytes outside 0x20..0x7e are
    // shown as <xx>)
    let mut sanitized = String::with_capacity(msg.len());
    for &c in msg.as_bytes() {
        if (0x20..=0x7e).contains(&c) {
            sanitized.push(c as char);
        } else {
            sanitized.push_str(&format!("<{:02x}>", c));
        }
    }
    let handled = CALLBACK.with(|c| {
        if let Ok(mut b) = c.try_borrow_mut() {
            if let Some(f) = b.as_mut() {
                f(category, pos, &sanitized);
                return true;
            }
        }
        false
    });
    if handled {
        return;
    }
    // pos is cast to int in the C code
    let ipos = pos as i32;
    if pos >= 0 {
        eprint!("\nlibxpdf: {} ({}): {}\n", category.name(), ipos, sanitized);
    } else {
        eprint!("\nlibxpdf: {}: {}\n", category.name(), sanitized);
    }
}

/// Error codes from `ErrorCodes.h`.
pub mod codes {
    pub const ERR_NONE: i32 = 0;
    pub const ERR_OPEN_FILE: i32 = 1;
    pub const ERR_BAD_CATALOG: i32 = 2;
    pub const ERR_DAMAGED: i32 = 3;
    pub const ERR_ENCRYPTED: i32 = 4;
    pub const ERR_HIGHLIGHT_FILE: i32 = 5;
    pub const ERR_BAD_PRINTER: i32 = 6;
    pub const ERR_PRINTING: i32 = 7;
    pub const ERR_PERMISSION: i32 = 8;
    pub const ERR_BAD_PAGE_NUM: i32 = 9;
    pub const ERR_FILE_IO: i32 = 10;
}
