//! Port of xpdf `Page.h` / `Page.cc` (attributes and the parts of `Page`
//! that do not render).

use std::rc::Rc;

use crate::error::{error, ErrorCategory};
use crate::object::{Dict, Object};
use crate::stream::Stream;
use crate::xref::XRef;

/// Mirrors `class PDFRectangle`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PDFRectangle {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

impl PDFRectangle {
    pub fn new(x1: f64, y1: f64, x2: f64, y2: f64) -> PDFRectangle {
        PDFRectangle { x1, y1, x2, y2 }
    }
    /// `isValid()`
    pub fn is_valid(&self) -> bool {
        self.x1 != 0.0 || self.y1 != 0.0 || self.x2 != 0.0 || self.y2 != 0.0
    }
    /// `clipTo(PDFRectangle *rect)`
    pub fn clip_to(&mut self, rect: &PDFRectangle) {
        if self.x1 < rect.x1 {
            self.x1 = rect.x1;
        } else if self.x1 > rect.x2 {
            self.x1 = rect.x2;
        }
        if self.x2 < rect.x1 {
            self.x2 = rect.x1;
        } else if self.x2 > rect.x2 {
            self.x2 = rect.x2;
        }
        if self.y1 < rect.y1 {
            self.y1 = rect.y1;
        } else if self.y1 > rect.y2 {
            self.y1 = rect.y2;
        }
        if self.y2 < rect.y1 {
            self.y2 = rect.y1;
        } else if self.y2 > rect.y2 {
            self.y2 = rect.y2;
        }
    }
}

/// Mirrors `class PageAttrs`.
#[derive(Clone)]
pub struct PageAttrs {
    media_box: PDFRectangle,
    crop_box: PDFRectangle,
    have_crop_box: bool,
    bleed_box: PDFRectangle,
    trim_box: PDFRectangle,
    art_box: PDFRectangle,
    rotate: i32,
    last_modified: Object,
    box_color_info: Object,
    group: Object,
    metadata: Object,
    piece_info: Object,
    separation_info: Object,
    user_unit: f64,
    resources: Object,
}

impl PageAttrs {
    /// `PageAttrs(PageAttrs *attrs, Dict *dict, XRef *xref)`: construct a
    /// new PageAttrs object by merging a dictionary (of type Pages or
    /// Page) into another PageAttrs object.  If `attrs` is `None`, use
    /// defaults.
    pub fn new(attrs: Option<&PageAttrs>, dict: &Dict, xref: &XRef) -> PageAttrs {
        // get old/default values
        let mut a = match attrs {
            Some(p) => PageAttrs {
                media_box: p.media_box,
                crop_box: p.crop_box,
                have_crop_box: p.have_crop_box,
                bleed_box: PDFRectangle::default(),
                trim_box: PDFRectangle::default(),
                art_box: PDFRectangle::default(),
                rotate: p.rotate,
                last_modified: Object::Null,
                box_color_info: Object::Null,
                group: Object::Null,
                metadata: Object::Null,
                piece_info: Object::Null,
                separation_info: Object::Null,
                user_unit: 1.0,
                resources: Object::Null,
            },
            None => PageAttrs {
                // set default MediaBox to 8.5" x 11" -- this shouldn't be
                // necessary but some (non-compliant) PDF files don't
                // specify a MediaBox
                media_box: PDFRectangle::new(0.0, 0.0, 612.0, 792.0),
                crop_box: PDFRectangle::default(),
                have_crop_box: false,
                bleed_box: PDFRectangle::default(),
                trim_box: PDFRectangle::default(),
                art_box: PDFRectangle::default(),
                rotate: 0,
                last_modified: Object::Null,
                box_color_info: Object::Null,
                group: Object::Null,
                metadata: Object::Null,
                piece_info: Object::Null,
                separation_info: Object::Null,
                user_unit: 1.0,
                resources: Object::Null,
            },
        };

        // media box
        Self::read_box(dict, "MediaBox", &mut a.media_box);

        // crop box
        if Self::read_box(dict, "CropBox", &mut a.crop_box) {
            a.have_crop_box = true;
        }
        if !a.have_crop_box {
            a.crop_box = a.media_box;
        }

        // other boxes
        a.bleed_box = a.crop_box;
        Self::read_box(dict, "BleedBox", &mut a.bleed_box);
        a.trim_box = a.crop_box;
        Self::read_box(dict, "TrimBox", &mut a.trim_box);
        a.art_box = a.crop_box;
        Self::read_box(dict, "ArtBox", &mut a.art_box);

        // rotate
        if let Object::Int(r) = dict.lookup("Rotate") {
            a.rotate = r;
        }
        while a.rotate < 0 {
            a.rotate += 360;
        }
        while a.rotate >= 360 {
            a.rotate -= 360;
        }

        // misc attributes
        a.last_modified = dict.lookup("LastModified");
        a.box_color_info = dict.lookup("BoxColorInfo");
        a.group = dict.lookup("Group");
        a.metadata = dict.lookup("Metadata");
        a.piece_info = dict.lookup("PieceInfo");
        a.separation_info = dict.lookup("SeparationInfo");
        let uu = dict.lookup("UserUnit");
        if uu.is_num() {
            a.user_unit = uu.get_num();
            if a.user_unit < 1.0 {
                a.user_unit = 1.0;
            }
        } else {
            a.user_unit = 1.0;
        }

        // resource dictionary
        let child_res_dict_obj = dict.lookup("Resources");
        let parent_res = attrs.and_then(|p| p.resources.get_dict().cloned());
        if let (Some(parent_res_dict), Object::Dict(child_res_dict)) = (&parent_res, &child_res_dict_obj) {
            // merge this node's resources into the parent's resources
            // (some PDF files violate the PDF spec and expect this
            // merging)
            //
            // xpdf builds the merged dictionary in place, adding to
            // sub-dictionaries obtained through lookup() (which share the
            // Dict).  Only freshly created sub-dictionaries are ever
            // modified, so building them here before sharing is
            // equivalent.
            let mut res_dict = Dict::new(Some(xref));
            for i in 0..parent_res_dict.get_length() {
                let res_type = parent_res_dict.get_key(i);
                let subdict_obj1 = parent_res_dict.get_val(i);
                if let Object::Dict(subdict1) = &subdict_obj1 {
                    let mut subdict2 = Dict::new(Some(xref));
                    for j in 0..subdict1.get_length() {
                        subdict2.add(&subdict1.get_key(j), subdict1.get_val_nf(j));
                    }
                    res_dict.add(&res_type, Object::Dict(Rc::new(subdict2)));
                }
            }
            for i in 0..child_res_dict.get_length() {
                let res_type = child_res_dict.get_key(i);
                let subdict_obj1 = child_res_dict.get_val(i);
                if let Object::Dict(subdict1) = &subdict_obj1 {
                    // resDict->lookup(resType, &subdictObj2)->isDict()
                    let existing = res_dict.lookup(&*res_type);
                    if let Object::Dict(subdict2) = existing {
                        // subdict2 is one of the dictionaries created just
                        // above (unshared apart from res_dict itself)
                        let mut merged: Dict = (*subdict2).clone();
                        for j in 0..subdict1.get_length() {
                            merged.add(&subdict1.get_key(j), subdict1.get_val_nf(j));
                        }
                        res_dict.add(&res_type, Object::Dict(Rc::new(merged)));
                    } else {
                        res_dict.add(&res_type, subdict_obj1.clone());
                    }
                }
            }
            a.resources = Object::Dict(Rc::new(res_dict));
        } else if parent_res.is_some() {
            a.resources = attrs.unwrap().resources.clone();
        } else if child_res_dict_obj.is_dict() {
            a.resources = child_res_dict_obj;
        } else {
            a.resources = Object::Null;
        }
        a
    }

    /// `PageAttrs()`: dummy attributes for a page that couldn't be loaded
    /// (50 x 50 media box).
    pub fn new_default() -> PageAttrs {
        let mb = PDFRectangle::new(0.0, 0.0, 50.0, 50.0);
        PageAttrs {
            media_box: mb,
            crop_box: mb,
            have_crop_box: false,
            bleed_box: mb,
            trim_box: mb,
            art_box: mb,
            rotate: 0,
            last_modified: Object::Null,
            box_color_info: Object::Null,
            group: Object::Null,
            metadata: Object::Null,
            piece_info: Object::Null,
            separation_info: Object::Null,
            user_unit: 1.0,
            resources: Object::Null,
        }
    }

    /// `clipBoxes()`: clip all other boxes to the media box.
    pub fn clip_boxes(&mut self) {
        let mb = self.media_box;
        self.crop_box.clip_to(&mb);
        self.bleed_box.clip_to(&mb);
        self.trim_box.clip_to(&mb);
        self.art_box.clip_to(&mb);
    }

    /// `readBox(Dict *dict, const char *key, PDFRectangle *box)`.
    fn read_box(dict: &Dict, key: &str, bx: &mut PDFRectangle) -> bool {
        let obj1 = dict.lookup(key);
        if !(obj1.is_array() && obj1.array_get_length() == 4) {
            return false;
        }
        let mut ok = true;
        let mut v = [0.0f64; 4];
        for (i, vi) in v.iter_mut().enumerate() {
            let obj2 = obj1.array_get(i as i32);
            if obj2.is_num() {
                *vi = obj2.get_num();
            } else {
                ok = false;
            }
        }
        if ok {
            // limit the box coords so they can be converted to 32-bit
            // ints later
            for vi in v.iter_mut() {
                if *vi < -1e9 {
                    *vi = -1e9;
                } else if *vi > 1e9 {
                    *vi = 1e9;
                }
            }
            let mut tmp = PDFRectangle::new(v[0], v[1], v[2], v[3]);
            if tmp.x1 > tmp.x2 {
                std::mem::swap(&mut tmp.x1, &mut tmp.x2);
            }
            if tmp.y1 > tmp.y2 {
                std::mem::swap(&mut tmp.y1, &mut tmp.y2);
            }
            *bx = tmp;
        }
        ok
    }

    pub fn get_media_box(&self) -> &PDFRectangle {
        &self.media_box
    }
    pub fn get_crop_box(&self) -> &PDFRectangle {
        &self.crop_box
    }
    pub fn is_cropped(&self) -> bool {
        self.have_crop_box
    }
    pub fn get_bleed_box(&self) -> &PDFRectangle {
        &self.bleed_box
    }
    pub fn get_trim_box(&self) -> &PDFRectangle {
        &self.trim_box
    }
    pub fn get_art_box(&self) -> &PDFRectangle {
        &self.art_box
    }
    pub fn get_rotate(&self) -> i32 {
        self.rotate
    }
    /// `getLastModified()`: string or None.
    pub fn get_last_modified(&self) -> Option<&[u8]> {
        if self.last_modified.is_string() {
            Some(self.last_modified.get_string())
        } else {
            None
        }
    }
    pub fn get_box_color_info(&self) -> Option<&Rc<Dict>> {
        self.box_color_info.get_dict()
    }
    pub fn get_group(&self) -> Option<&Rc<Dict>> {
        self.group.get_dict()
    }
    pub fn get_metadata(&self) -> Option<&Rc<Stream>> {
        self.metadata.get_stream()
    }
    pub fn get_piece_info(&self) -> Option<&Rc<Dict>> {
        self.piece_info.get_dict()
    }
    pub fn get_separation_info(&self) -> Option<&Rc<Dict>> {
        self.separation_info.get_dict()
    }
    pub fn get_user_unit(&self) -> f64 {
        self.user_unit
    }
    pub fn get_resource_dict(&self) -> Option<&Rc<Dict>> {
        self.resources.get_dict()
    }
    /// The resources object itself (`Dict` or `Null`).
    pub fn get_resources(&self) -> &Object {
        &self.resources
    }
}

/// Mirrors `class Page`.
pub struct Page {
    xref: Rc<XRef>,
    num: i32,
    attrs: PageAttrs,
    annots: Object,
    contents: Object,
    thumbnail: Object,
    ok: bool,
}

impl Page {
    /// `Page(PDFDoc *docA, int numA, Dict *pageDict, PageAttrs *attrsA)`.
    pub fn new(xref: Rc<XRef>, num: i32, page_dict: &Dict, mut attrs: PageAttrs) -> Page {
        attrs.clip_boxes();
        let mut p = Page {
            xref,
            num,
            attrs,
            annots: Object::Null,
            contents: Object::Null,
            thumbnail: Object::Null,
            ok: true,
        };

        // annotations
        p.annots = page_dict.lookup_nf("Annots");
        if !(p.annots.is_ref() || p.annots.is_array() || p.annots.is_null()) {
            error(
                ErrorCategory::SyntaxError,
                -1,
                &format!("Page annotations object (page {}) is wrong type ({})", num, p.annots.get_type_name()),
            );
            p.annots = Object::Null;
            p.contents = Object::Null;
            p.thumbnail = Object::Null;
            p.ok = false;
            return p;
        }

        // contents
        p.contents = page_dict.lookup_nf("Contents");
        if !(p.contents.is_ref() || p.contents.is_array() || p.contents.is_null()) {
            error(
                ErrorCategory::SyntaxError,
                -1,
                &format!("Page contents object (page {}) is wrong type ({})", num, p.contents.get_type_name()),
            );
            p.contents = Object::Null;
            p.thumbnail = Object::Null;
            p.ok = false;
            return p;
        }

        // thumbnail
        p.thumbnail = page_dict.lookup_nf("Thumb");
        if !p.thumbnail.is_ref() {
            p.thumbnail = Object::Null;
        }
        p
    }

    /// `Page(PDFDoc *docA, int numA)`: create an empty page.
    pub fn new_empty(xref: Rc<XRef>, num: i32) -> Page {
        Page {
            xref,
            num,
            attrs: PageAttrs::new_default(),
            annots: Object::Null,
            contents: Object::Null,
            thumbnail: Object::Null,
            ok: true,
        }
    }

    pub fn is_ok(&self) -> bool {
        self.ok
    }
    pub fn get_num(&self) -> i32 {
        self.num
    }
    pub fn get_attrs(&self) -> &PageAttrs {
        &self.attrs
    }
    pub fn get_media_box(&self) -> &PDFRectangle {
        self.attrs.get_media_box()
    }
    pub fn get_crop_box(&self) -> &PDFRectangle {
        self.attrs.get_crop_box()
    }
    pub fn is_cropped(&self) -> bool {
        self.attrs.is_cropped()
    }
    pub fn get_media_width(&self) -> f64 {
        self.attrs.media_box.x2 - self.attrs.media_box.x1
    }
    pub fn get_media_height(&self) -> f64 {
        self.attrs.media_box.y2 - self.attrs.media_box.y1
    }
    pub fn get_crop_width(&self) -> f64 {
        self.attrs.crop_box.x2 - self.attrs.crop_box.x1
    }
    pub fn get_crop_height(&self) -> f64 {
        self.attrs.crop_box.y2 - self.attrs.crop_box.y1
    }
    pub fn get_bleed_box(&self) -> &PDFRectangle {
        self.attrs.get_bleed_box()
    }
    pub fn get_trim_box(&self) -> &PDFRectangle {
        self.attrs.get_trim_box()
    }
    pub fn get_art_box(&self) -> &PDFRectangle {
        self.attrs.get_art_box()
    }
    pub fn get_rotate(&self) -> i32 {
        self.attrs.get_rotate()
    }
    pub fn get_last_modified(&self) -> Option<&[u8]> {
        self.attrs.get_last_modified()
    }
    pub fn get_box_color_info(&self) -> Option<&Rc<Dict>> {
        self.attrs.get_box_color_info()
    }
    pub fn get_group(&self) -> Option<&Rc<Dict>> {
        self.attrs.get_group()
    }
    pub fn get_metadata(&self) -> Option<&Rc<Stream>> {
        self.attrs.get_metadata()
    }
    pub fn get_piece_info(&self) -> Option<&Rc<Dict>> {
        self.attrs.get_piece_info()
    }
    pub fn get_separation_info(&self) -> Option<&Rc<Dict>> {
        self.attrs.get_separation_info()
    }
    pub fn get_user_unit(&self) -> f64 {
        self.attrs.get_user_unit()
    }
    /// `getResourceDict()`
    pub fn get_resource_dict(&self) -> Option<&Rc<Dict>> {
        self.attrs.get_resource_dict()
    }
    /// `getAnnots(Object *obj)`: `annots.fetch(xref, obj)`.
    pub fn get_annots(&self) -> Object {
        self.annots.fetch(Some(&self.xref))
    }
    /// `getContents(Object *obj)`: `contents.fetch(xref, obj)`.
    pub fn get_contents(&self) -> Object {
        self.contents.fetch(Some(&self.xref))
    }
    /// The unresolved /Contents value.
    pub fn get_contents_nf(&self) -> &Object {
        &self.contents
    }
    /// `getThumb(Object *obj)`.
    pub fn get_thumb(&self) -> Object {
        self.thumbnail.fetch(Some(&self.xref))
    }
}
