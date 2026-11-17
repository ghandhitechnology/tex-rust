//! `texrs-pdfread`: a pure-Rust port of the subset of xpdf 4.06 (as
//! shipped in TeX Live 2026, `libs/xpdf`) that pdfTeX's `pdftoepdf.cc`
//! uses to include PDF files (`\pdfximage`).
//!
//! The goal is that every value pdfTeX re-serializes from an included PDF
//! is parsed exactly as xpdf parses it: the same int/real decisions, the
//! same string bytes, dictionary entry order, duplicate-key handling,
//! damaged-file recovery, object-stream and xref behaviour, and the same
//! decoded stream bytes.  See README.md for the C++ -> Rust mapping and
//! the behavioural notes.
//!
//! ```no_run
//! use pdfread::{PdfDoc, Object};
//! let doc = PdfDoc::open("figure.pdf");
//! assert!(doc.is_ok() && doc.ok_to_print(false));
//! let cat = doc.get_catalog().unwrap();
//! let page = cat.get_page(1).unwrap();
//! let r = cat.get_page_ref(1).unwrap();
//! let page_obj = doc.get_xref().unwrap().fetch(r.num, r.gen);
//! let mb = page.get_media_box();
//! for (key, val) in page.get_resource_dict().unwrap().iter() { let _ = (key, val); }
//! let _ = (page_obj, mb, Object::Null);
//! ```

pub mod catalog;
pub mod decrypt;
pub mod doc;
pub mod error;
pub mod filters;
pub mod fofi;
mod fofi_tables;
pub mod font_encoding_tables;
pub mod gfxfont;
pub mod lexer;
pub mod link;
pub mod object;
pub mod page;
pub mod parser;
pub mod security;
pub mod stream;
pub mod xref;

pub use catalog::Catalog;
pub use decrypt::CryptAlgorithm;
pub use doc::PdfDoc;
pub use gfxfont::{GfxFont, GfxFontType};
pub use error::{set_err_quiet, set_error_callback, ErrorCategory};
pub use lexer::Lexer;
pub use link::{LinkDest, LinkDestKind};
pub use object::{Array, Dict, ObjType, Object, Ref};
pub use page::{PDFRectangle, Page, PageAttrs};
pub use parser::Parser;
pub use stream::{Stream, StreamKind, StreamReader, UnsupportedFilter};
pub use xref::{XRef, XRefEntry, XRefEntryType};
