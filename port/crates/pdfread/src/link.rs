//! Port of `LinkDest` from xpdf `Link.h` / `Link.cc` (used by pdfTeX for
//! `\pdfximage named {...}` through `PDFDoc::findDest`).

use crate::error::{error, ErrorCategory};
use crate::object::{Array, Object, Ref};

/// Mirrors `enum LinkDestKind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkDestKind {
    XYZ,
    Fit,
    FitH,
    FitV,
    FitR,
    FitB,
    FitBH,
    FitBV,
}

/// Mirrors `class LinkDest`.
#[derive(Clone, Debug)]
pub struct LinkDest {
    kind: LinkDestKind,
    page_is_ref: bool,
    page_ref: Ref,
    page_num: i32,
    left: f64,
    bottom: f64,
    right: f64,
    top: f64,
    zoom: f64,
    change_left: bool,
    change_top: bool,
    change_zoom: bool,
    ok: bool,
}

impl LinkDest {
    /// `LinkDest::LinkDest(Array *a)`.
    pub fn new(a: &Array) -> LinkDest {
        let mut d = LinkDest {
            kind: LinkDestKind::Fit,
            page_is_ref: false,
            page_ref: Ref::default(),
            page_num: 0,
            left: 0.0,
            bottom: 0.0,
            right: 0.0,
            top: 0.0,
            zoom: 0.0,
            change_left: false,
            change_top: false,
            change_zoom: false,
            ok: false,
        };
        let warn = |m: &str| error(ErrorCategory::SyntaxWarning, -1, m);

        // get page
        if a.get_length() < 2 {
            warn("Annotation destination array is too short");
            return d;
        }
        match a.get_nf(0) {
            Object::Int(i) => {
                d.page_num = i.wrapping_add(1);
                d.page_is_ref = false;
            }
            Object::Ref(r) => {
                d.page_ref = r;
                d.page_is_ref = true;
            }
            _ => {
                warn("Bad annotation destination");
                return d;
            }
        }

        // get destination type
        let obj1 = a.get(1);
        let bad_pos = || error(ErrorCategory::SyntaxWarning, -1, "Bad annotation destination position");

        if obj1.is_name_of("XYZ") {
            // XYZ link
            d.kind = LinkDestKind::XYZ;
            if a.get_length() < 3 {
                d.change_left = false;
            } else {
                let o = a.get(2);
                if o.is_null() {
                    d.change_left = false;
                } else if o.is_num() {
                    d.change_left = true;
                    d.left = o.get_num();
                } else {
                    bad_pos();
                    return d;
                }
            }
            if a.get_length() < 4 {
                d.change_top = false;
            } else {
                let o = a.get(3);
                if o.is_null() {
                    d.change_top = false;
                } else if o.is_num() {
                    d.change_top = true;
                    d.top = o.get_num();
                } else {
                    bad_pos();
                    return d;
                }
            }
            if a.get_length() < 5 {
                d.change_zoom = false;
            } else {
                let o = a.get(4);
                if o.is_null() {
                    d.change_zoom = false;
                } else if o.is_num() {
                    d.change_zoom = true;
                    d.zoom = o.get_num();
                } else {
                    bad_pos();
                    return d;
                }
            }
        } else if obj1.is_name_of("Fit") {
            d.kind = LinkDestKind::Fit;
        } else if obj1.is_name_of("FitH") || obj1.is_name_of("FitBH") {
            if a.get_length() < 3 {
                warn("Annotation destination array is too short");
                return d;
            }
            d.kind = if obj1.is_name_of("FitH") { LinkDestKind::FitH } else { LinkDestKind::FitBH };
            let o = a.get(2);
            if o.is_num() {
                d.top = o.get_num();
                d.change_top = true;
            } else if o.is_null() {
                d.change_top = false;
            } else {
                bad_pos();
                d.kind = LinkDestKind::Fit;
            }
        } else if obj1.is_name_of("FitV") || obj1.is_name_of("FitBV") {
            if a.get_length() < 3 {
                warn("Annotation destination array is too short");
                return d;
            }
            d.kind = if obj1.is_name_of("FitV") { LinkDestKind::FitV } else { LinkDestKind::FitBV };
            let o = a.get(2);
            if o.is_num() {
                d.left = o.get_num();
                d.change_left = true;
            } else if o.is_null() {
                d.change_left = false;
            } else {
                bad_pos();
                d.kind = LinkDestKind::Fit;
            }
        } else if obj1.is_name_of("FitR") {
            if a.get_length() < 6 {
                warn("Annotation destination array is too short");
                return d;
            }
            d.kind = LinkDestKind::FitR;
            let o = a.get(2);
            if o.is_num() {
                d.left = o.get_num();
            } else {
                bad_pos();
                d.kind = LinkDestKind::Fit;
            }
            let o = a.get(3);
            if !o.is_num() {
                bad_pos();
                d.kind = LinkDestKind::Fit;
            }
            d.bottom = o.get_num();
            let o = a.get(4);
            if !o.is_num() {
                bad_pos();
                d.kind = LinkDestKind::Fit;
            }
            d.right = o.get_num();
            let o = a.get(5);
            if !o.is_num() {
                bad_pos();
                d.kind = LinkDestKind::Fit;
            }
            d.top = o.get_num();
        } else if obj1.is_name_of("FitB") {
            d.kind = LinkDestKind::FitB;
        } else {
            warn("Unknown annotation destination type");
            return d;
        }
        d.ok = true;
        d
    }

    pub fn is_ok(&self) -> bool {
        self.ok
    }
    pub fn get_kind(&self) -> LinkDestKind {
        self.kind
    }
    pub fn is_page_ref(&self) -> bool {
        self.page_is_ref
    }
    pub fn get_page_num(&self) -> i32 {
        self.page_num
    }
    /// `getPageRef()`.  In xpdf `pageRef` and `pageNum` share a union, so
    /// for a destination given by page *number* this returns
    /// `{num: pageNum, gen: <uninitialized>}`; here the generation is 0.
    /// (pdfTeX calls this without checking `isPageRef()`.)
    pub fn get_page_ref(&self) -> Ref {
        if self.page_is_ref {
            self.page_ref
        } else {
            Ref { num: self.page_num, gen: 0 }
        }
    }
    pub fn get_left(&self) -> f64 {
        self.left
    }
    pub fn get_bottom(&self) -> f64 {
        self.bottom
    }
    pub fn get_right(&self) -> f64 {
        self.right
    }
    pub fn get_top(&self) -> f64 {
        self.top
    }
    pub fn get_zoom(&self) -> f64 {
        self.zoom
    }
    pub fn get_change_left(&self) -> bool {
        self.change_left
    }
    pub fn get_change_top(&self) -> bool {
        self.change_top
    }
    pub fn get_change_zoom(&self) -> bool {
        self.change_zoom
    }
}
