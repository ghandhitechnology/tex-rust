//! Port of xpdf `PDFDoc.h` / `PDFDoc.cc` (the non-rendering parts).

use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::catalog::Catalog;
use crate::error::{codes, error, ErrorCategory};
use crate::link::LinkDest;
use crate::object::Object;
use crate::security::make_security_handler;
use crate::xref::XRef;

/// `headerSearchSize`: read this many bytes at beginning of file to look
/// for '%PDF'.
const HEADER_SEARCH_SIZE: usize = 1024;

/// `supportedPDFVersionNum`
pub const SUPPORTED_PDF_VERSION_NUM: f64 = 2.0;
/// `supportedPDFVersionStr`
pub const SUPPORTED_PDF_VERSION_STR: &str = "2.0";

/// Mirrors `class PDFDoc`.
pub struct PdfDoc {
    file_name: Option<PathBuf>,
    data: Rc<[u8]>,
    start: u64,
    xref: Option<Rc<XRef>>,
    catalog: Option<Catalog>,
    ok: bool,
    err_code: i32,
    pdf_version: f64,
}

impl PdfDoc {
    /// `PDFDoc(GString *fileNameA, GString *ownerPassword = NULL, GString
    /// *userPassword = NULL)`: open a file.  As in xpdf, if the file
    /// can't be opened the lower-cased and then upper-cased name is
    /// tried.
    pub fn open(path: impl AsRef<Path>) -> PdfDoc {
        Self::open_with_passwords(path, None, None)
    }

    /// `PDFDoc(fileName, ownerPassword, userPassword)`.
    pub fn open_with_passwords(
        path: impl AsRef<Path>,
        owner_password: Option<&[u8]>,
        user_password: Option<&[u8]>,
    ) -> PdfDoc {
        let path = path.as_ref();
        let mut data = std::fs::read(path);
        if data.is_err() {
            let s = path.to_string_lossy();
            data = std::fs::read(s.to_lowercase());
            if data.is_err() {
                data = std::fs::read(s.to_uppercase());
            }
        }
        match data {
            Ok(d) => {
                let mut doc = PdfDoc::new_empty(Some(path.to_path_buf()), Rc::from(d));
                doc.ok = doc.setup(owner_password, user_password);
                doc
            }
            Err(_) => {
                error(ErrorCategory::IO, -1, &format!("Couldn't open file '{}'", path.display()));
                let mut doc = PdfDoc::new_empty(Some(path.to_path_buf()), Rc::from(Vec::new()));
                doc.err_code = codes::ERR_OPEN_FILE;
                doc
            }
        }
    }

    /// `PDFDoc(BaseStream *strA, ...)` with an in-memory file.
    pub fn from_bytes(data: impl Into<Vec<u8>>) -> PdfDoc {
        Self::from_bytes_with_passwords(data, None, None)
    }

    pub fn from_bytes_with_passwords(
        data: impl Into<Vec<u8>>,
        owner_password: Option<&[u8]>,
        user_password: Option<&[u8]>,
    ) -> PdfDoc {
        let mut doc = PdfDoc::new_empty(None, Rc::from(data.into()));
        doc.ok = doc.setup(owner_password, user_password);
        doc
    }

    fn new_empty(file_name: Option<PathBuf>, data: Rc<[u8]>) -> PdfDoc {
        PdfDoc {
            file_name,
            data,
            start: 0,
            xref: None,
            catalog: None,
            ok: false,
            err_code: codes::ERR_NONE,
            pdf_version: 0.0,
        }
    }

    /// `PDFDoc::setup(GString *ownerPassword, GString *userPassword)`.
    fn setup(&mut self, owner_password: Option<&[u8]>, user_password: Option<&[u8]>) -> bool {
        // check header
        self.check_header();

        // read the xref and catalog
        if !self.setup2(owner_password, user_password, false) {
            if self.err_code == codes::ERR_DAMAGED || self.err_code == codes::ERR_BAD_CATALOG {
                // try repairing the xref table
                error(
                    ErrorCategory::SyntaxWarning,
                    -1,
                    "PDF file is damaged - attempting to reconstruct xref table...",
                );
                if !self.setup2(owner_password, user_password, true) {
                    return false;
                }
            } else {
                return false;
            }
        }
        // (the outline and optional content are not read)
        true
    }

    /// `PDFDoc::setup2(...)`.
    fn setup2(&mut self, owner_password: Option<&[u8]>, user_password: Option<&[u8]>, repair_xref: bool) -> bool {
        // read xref table
        let xref = XRef::new(self.data.clone(), self.start, repair_xref);
        if !xref.is_ok() {
            error(ErrorCategory::SyntaxError, -1, "Couldn't read xref table");
            self.err_code = xref.get_error_code();
            self.xref = None;
            return false;
        }

        // check for encryption
        if !Self::check_encryption(&xref, owner_password, user_password) {
            self.err_code = codes::ERR_ENCRYPTED;
            self.xref = None;
            return false;
        }

        // read catalog
        let catalog = Catalog::new(xref.clone());
        if !catalog.is_ok() {
            error(ErrorCategory::SyntaxError, -1, "Couldn't read page catalog");
            self.err_code = codes::ERR_BAD_CATALOG;
            self.xref = None;
            self.catalog = None;
            return false;
        }
        self.xref = Some(xref);
        self.catalog = Some(catalog);
        true
    }

    /// `PDFDoc::checkHeader()`: check for a PDF header on this stream.
    /// Skip past some garbage if necessary.
    fn check_header(&mut self) {
        self.pdf_version = 0.0;
        let mut hdr_buf = [0u8; HEADER_SEARCH_SIZE + 1];
        let n = self.data.len().min(HEADER_SEARCH_SIZE);
        hdr_buf[..n].copy_from_slice(&self.data[..n]);
        let mut i = 0;
        while i < HEADER_SEARCH_SIZE - 5 {
            if &hdr_buf[i..i + 5] == b"%PDF-" {
                break;
            }
            i += 1;
        }
        if i >= HEADER_SEARCH_SIZE - 5 {
            error(ErrorCategory::SyntaxWarning, -1, "May not be a PDF file (continuing anyway)");
            return;
        }
        // str->moveStart(i)
        self.start = i as u64;
        // p = strtok(&hdrBuf[i+5], " \t\n\r")
        let rest = &hdr_buf[i + 5..];
        let is_delim = |c: u8| c == b' ' || c == b'\t' || c == b'\n' || c == b'\r';
        let mut s = 0;
        while s < rest.len() && rest[s] != 0 && is_delim(rest[s]) {
            s += 1;
        }
        if s >= rest.len() || rest[s] == 0 {
            error(ErrorCategory::SyntaxWarning, -1, "May not be a PDF file (continuing anyway)");
            return;
        }
        let mut e = s;
        while e < rest.len() && rest[e] != 0 && !is_delim(rest[e]) {
            e += 1;
        }
        let tok = &rest[s..e];
        self.pdf_version = atof(tok);
        if !hdr_buf[i + 5].is_ascii_digit() || self.pdf_version > SUPPORTED_PDF_VERSION_NUM + 0.0001 {
            error(
                ErrorCategory::SyntaxWarning,
                -1,
                &format!(
                    "PDF version {} -- xpdf supports version {} (continuing anyway)",
                    String::from_utf8_lossy(tok),
                    SUPPORTED_PDF_VERSION_STR
                ),
            );
        }
    }

    /// `PDFDoc::checkEncryption(GString *ownerPassword, GString
    /// *userPassword)`.
    fn check_encryption(xref: &XRef, owner_password: Option<&[u8]>, user_password: Option<&[u8]>) -> bool {
        let encrypt = xref.get_trailer_dict().dict_lookup("Encrypt");
        if !encrypt.is_dict() {
            // document is not encrypted
            return true;
        }
        match make_security_handler(xref, &encrypt) {
            Some(mut sec) => {
                if sec.is_unencrypted() {
                    // no encryption
                    true
                } else if sec.check_encryption(owner_password, user_password) {
                    // authorization succeeded
                    xref.set_encryption(
                        sec.get_permission_flags(),
                        sec.get_owner_password_ok(),
                        sec.get_file_key(),
                        sec.get_file_key_length() as usize,
                        sec.get_enc_version(),
                        sec.get_enc_algorithm(),
                    );
                    true
                } else {
                    // authorization failed
                    false
                }
            }
            // couldn't find the matching security handler
            None => false,
        }
    }

    /// `isOk()`
    pub fn is_ok(&self) -> bool {
        self.ok
    }
    /// `getErrorCode()`
    pub fn get_error_code(&self) -> i32 {
        self.err_code
    }
    /// `getFileName()`
    pub fn get_file_name(&self) -> Option<&Path> {
        self.file_name.as_deref()
    }
    /// `getXRef()` (None if the document failed to open)
    pub fn get_xref(&self) -> Option<&Rc<XRef>> {
        self.xref.as_ref()
    }
    /// `getCatalog()` (None if the document failed to open)
    pub fn get_catalog(&self) -> Option<&Catalog> {
        self.catalog.as_ref()
    }
    /// `getNumPages()`
    pub fn get_num_pages(&self) -> i32 {
        self.catalog.as_ref().map(|c| c.get_num_pages()).unwrap_or(0)
    }
    /// `getPDFVersion()`: the number after `%PDF-`, parsed with `atof()`.
    pub fn get_pdf_version(&self) -> f64 {
        self.pdf_version
    }
    /// `findDest(GString *name)`
    pub fn find_dest(&self, name: &[u8]) -> Option<LinkDest> {
        self.catalog.as_ref()?.find_dest(name)
    }
    /// `okToPrint(GBool ignoreOwnerPW = gFalse)`
    pub fn ok_to_print(&self, ignore_owner_pw: bool) -> bool {
        self.xref.as_ref().map(|x| x.ok_to_print(ignore_owner_pw)).unwrap_or(false)
    }
    pub fn ok_to_change(&self, ignore_owner_pw: bool) -> bool {
        self.xref.as_ref().map(|x| x.ok_to_change(ignore_owner_pw)).unwrap_or(false)
    }
    pub fn ok_to_copy(&self, ignore_owner_pw: bool) -> bool {
        self.xref.as_ref().map(|x| x.ok_to_copy(ignore_owner_pw)).unwrap_or(false)
    }
    pub fn ok_to_add_notes(&self, ignore_owner_pw: bool) -> bool {
        self.xref.as_ref().map(|x| x.ok_to_add_notes(ignore_owner_pw)).unwrap_or(false)
    }
    /// `isEncrypted()`
    pub fn is_encrypted(&self) -> bool {
        self.xref.as_ref().map(|x| x.is_encrypted()).unwrap_or(false)
    }
    /// `getDocInfo(Object *obj)`
    pub fn get_doc_info(&self) -> Object {
        self.xref.as_ref().map(|x| x.get_doc_info()).unwrap_or(Object::Null)
    }
    /// `getDocInfoNF(Object *obj)`
    pub fn get_doc_info_nf(&self) -> Object {
        self.xref.as_ref().map(|x| x.get_doc_info_nf()).unwrap_or(Object::Null)
    }
    /// The whole file.
    pub fn get_data(&self) -> &Rc<[u8]> {
        &self.data
    }
}

/// C `atof()` (= `strtod(s, NULL)`) on a NUL-free token.
pub(crate) fn atof(s: &[u8]) -> f64 {
    let mut i = 0;
    while i < s.len() && (s[i] == b' ' || (0x09..=0x0d).contains(&s[i])) {
        i += 1;
    }
    let st = i;
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    let lower: Vec<u8> = s[i..].iter().map(|c| c.to_ascii_lowercase()).collect();
    if lower.starts_with(b"inf") {
        return if neg { f64::NEG_INFINITY } else { f64::INFINITY };
    }
    if lower.starts_with(b"nan") {
        return f64::NAN;
    }
    if lower.starts_with(b"0x") && lower.len() > 2 && (lower[2].is_ascii_hexdigit() || lower[2] == b'.') {
        // hexadecimal floating point
        let mut j = 2;
        let mut mant: f64 = 0.0;
        let mut any = false;
        while j < lower.len() && lower[j].is_ascii_hexdigit() {
            mant = mant * 16.0 + (lower[j] as char).to_digit(16).unwrap() as f64;
            j += 1;
            any = true;
        }
        let mut scale = 0i32;
        if j < lower.len() && lower[j] == b'.' {
            j += 1;
            while j < lower.len() && lower[j].is_ascii_hexdigit() {
                mant = mant * 16.0 + (lower[j] as char).to_digit(16).unwrap() as f64;
                scale -= 4;
                j += 1;
                any = true;
            }
        }
        if !any {
            return 0.0;
        }
        if j < lower.len() && lower[j] == b'p' {
            let mut k = j + 1;
            let mut eneg = false;
            if k < lower.len() && (lower[k] == b'+' || lower[k] == b'-') {
                eneg = lower[k] == b'-';
                k += 1;
            }
            if k < lower.len() && lower[k].is_ascii_digit() {
                let mut e: i32 = 0;
                while k < lower.len() && lower[k].is_ascii_digit() {
                    e = e.saturating_mul(10).saturating_add((lower[k] - b'0') as i32);
                    k += 1;
                }
                scale = scale.saturating_add(if eneg { -e } else { e });
            }
        }
        let v = mant * 2f64.powi(scale);
        return if neg { -v } else { v };
    }
    // decimal: digits [. digits] [e [sign] digits]
    let mut j = i;
    let mut digits = 0;
    while j < s.len() && s[j].is_ascii_digit() {
        j += 1;
        digits += 1;
    }
    if j < s.len() && s[j] == b'.' {
        j += 1;
        while j < s.len() && s[j].is_ascii_digit() {
            j += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return 0.0;
    }
    if j < s.len() && (s[j] == b'e' || s[j] == b'E') {
        let mut k = j + 1;
        if k < s.len() && (s[k] == b'+' || s[k] == b'-') {
            k += 1;
        }
        if k < s.len() && s[k].is_ascii_digit() {
            while k < s.len() && s[k].is_ascii_digit() {
                k += 1;
            }
            j = k;
        }
    }
    let txt = std::str::from_utf8(&s[st..j]).unwrap_or("0");
    // Rust's parser rejects a trailing '.' and a leading '+' only in some
    // forms; normalise those
    let mut t = txt.trim_start_matches('+').to_string();
    if t.ends_with('.') {
        t.push('0');
    }
    if t.starts_with('.') || t.starts_with("-.") {
        t = t.replacen('.', "0.", 1);
    }
    t.parse::<f64>().unwrap_or(0.0)
}
