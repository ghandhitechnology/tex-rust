//! Port of the parts of xpdf `GfxFont.h` / `GfxFont.cc` that pdfTeX uses
//! when it replaces an embedded Type 1 font of an included PDF
//! (`copyFont()` in pdftoepdf.cc): `GfxFont::makeFont()`,
//! `GfxFont::getFontType()`, `isCIDFont()` and the encoding built by the
//! `Gfx8BitFont` constructor (`getCharName()`).
//!
//! Widths, ToUnicode maps, font descriptors etc. are computed by xpdf as
//! well but never used by pdfTeX, so they are not ported.

use crate::error::{error, ErrorCategory};
use crate::fofi::{identify, Encoding, FoFiIdentifierType, FoFiType1, FoFiType1C};
use crate::font_encoding_tables::*;
use crate::object::{Dict, Object, Ref};
use crate::xref::XRef;

/// Mirrors `enum GfxFontType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum GfxFontType {
    //----- Gfx8BitFont
    Unknown,
    Type1,
    Type1C,
    Type1COT,
    Type3,
    TrueType,
    TrueTypeOT,
    //----- GfxCIDFont
    CIDType0,
    CIDType0C,
    CIDType0COT,
    CIDType2,
    CIDType2OT,
}

/// `base14FontMap[]`: alternate names of the Base 14 fonts (sorted).
static BASE14_FONT_MAP: [(&str, &str); 65] = [
    ("Arial", "Helvetica"),
    ("Arial,Bold", "Helvetica-Bold"),
    ("Arial,BoldItalic", "Helvetica-BoldOblique"),
    ("Arial,Italic", "Helvetica-Oblique"),
    ("Arial-Bold", "Helvetica-Bold"),
    ("Arial-BoldItalic", "Helvetica-BoldOblique"),
    ("Arial-BoldItalicMT", "Helvetica-BoldOblique"),
    ("Arial-BoldMT", "Helvetica-Bold"),
    ("Arial-Italic", "Helvetica-Oblique"),
    ("Arial-ItalicMT", "Helvetica-Oblique"),
    ("ArialMT", "Helvetica"),
    ("Courier", "Courier"),
    ("Courier,Bold", "Courier-Bold"),
    ("Courier,BoldItalic", "Courier-BoldOblique"),
    ("Courier,Italic", "Courier-Oblique"),
    ("Courier-Bold", "Courier-Bold"),
    ("Courier-BoldOblique", "Courier-BoldOblique"),
    ("Courier-Oblique", "Courier-Oblique"),
    ("CourierNew", "Courier"),
    ("CourierNew,Bold", "Courier-Bold"),
    ("CourierNew,BoldItalic", "Courier-BoldOblique"),
    ("CourierNew,Italic", "Courier-Oblique"),
    ("CourierNew-Bold", "Courier-Bold"),
    ("CourierNew-BoldItalic", "Courier-BoldOblique"),
    ("CourierNew-Italic", "Courier-Oblique"),
    ("CourierNewPS-BoldItalicMT", "Courier-BoldOblique"),
    ("CourierNewPS-BoldMT", "Courier-Bold"),
    ("CourierNewPS-ItalicMT", "Courier-Oblique"),
    ("CourierNewPSMT", "Courier"),
    ("Helvetica", "Helvetica"),
    ("Helvetica,Bold", "Helvetica-Bold"),
    ("Helvetica,BoldItalic", "Helvetica-BoldOblique"),
    ("Helvetica,Italic", "Helvetica-Oblique"),
    ("Helvetica-Bold", "Helvetica-Bold"),
    ("Helvetica-BoldItalic", "Helvetica-BoldOblique"),
    ("Helvetica-BoldOblique", "Helvetica-BoldOblique"),
    ("Helvetica-Italic", "Helvetica-Oblique"),
    ("Helvetica-Oblique", "Helvetica-Oblique"),
    ("Symbol", "Symbol"),
    ("Symbol,Bold", "Symbol"),
    ("Symbol,BoldItalic", "Symbol"),
    ("Symbol,Italic", "Symbol"),
    ("Times-Bold", "Times-Bold"),
    ("Times-BoldItalic", "Times-BoldItalic"),
    ("Times-Italic", "Times-Italic"),
    ("Times-Roman", "Times-Roman"),
    ("TimesNewRoman", "Times-Roman"),
    ("TimesNewRoman,Bold", "Times-Bold"),
    ("TimesNewRoman,BoldItalic", "Times-BoldItalic"),
    ("TimesNewRoman,Italic", "Times-Italic"),
    ("TimesNewRoman-Bold", "Times-Bold"),
    ("TimesNewRoman-BoldItalic", "Times-BoldItalic"),
    ("TimesNewRoman-Italic", "Times-Italic"),
    ("TimesNewRomanPS", "Times-Roman"),
    ("TimesNewRomanPS-Bold", "Times-Bold"),
    ("TimesNewRomanPS-BoldItalic", "Times-BoldItalic"),
    ("TimesNewRomanPS-BoldItalicMT", "Times-BoldItalic"),
    ("TimesNewRomanPS-BoldMT", "Times-Bold"),
    ("TimesNewRomanPS-Italic", "Times-Italic"),
    ("TimesNewRomanPS-ItalicMT", "Times-Italic"),
    ("TimesNewRomanPSMT", "Times-Roman"),
    ("TimesNewRomanPSMT,Bold", "Times-Bold"),
    ("TimesNewRomanPSMT,BoldItalic", "Times-BoldItalic"),
    ("TimesNewRomanPSMT,Italic", "Times-Italic"),
    ("ZapfDingbats", "ZapfDingbats"),
];

/// `builtinFonts[i].defaultBaseEnc` (BuiltinFontTables.cc): the 14 base
/// fonts use StandardEncoding except Symbol and ZapfDingbats.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BuiltinEnc {
    Standard,
    Symbol,
    ZapfDingbats,
}

static BUILTIN_FONTS: [(&str, BuiltinEnc); 14] = [
    ("Courier", BuiltinEnc::Standard),
    ("Courier-Bold", BuiltinEnc::Standard),
    ("Courier-BoldOblique", BuiltinEnc::Standard),
    ("Courier-Oblique", BuiltinEnc::Standard),
    ("Helvetica", BuiltinEnc::Standard),
    ("Helvetica-Bold", BuiltinEnc::Standard),
    ("Helvetica-BoldOblique", BuiltinEnc::Standard),
    ("Helvetica-Oblique", BuiltinEnc::Standard),
    ("Symbol", BuiltinEnc::Symbol),
    ("Times-Bold", BuiltinEnc::Standard),
    ("Times-BoldItalic", BuiltinEnc::Standard),
    ("Times-Italic", BuiltinEnc::Standard),
    ("Times-Roman", BuiltinEnc::Standard),
    ("ZapfDingbats", BuiltinEnc::ZapfDingbats),
];

fn builtin_table(e: BuiltinEnc) -> &'static [Option<&'static str>; 256] {
    match e {
        BuiltinEnc::Standard => &STANDARD_ENCODING,
        BuiltinEnc::Symbol => &SYMBOL_ENCODING,
        BuiltinEnc::ZapfDingbats => &ZAPF_DINGBATS_ENCODING,
    }
}

/// Where the base encoding comes from (`const char **baseEnc`).
enum BaseEnc<'a> {
    Table(&'static [Option<&'static str>; 256]),
    FontFile(&'a Encoding),
}

/// Mirrors `class GfxFont` with the `Gfx8BitFont` encoding.
#[derive(Clone, Debug)]
pub struct GfxFont {
    tag: Vec<u8>,
    id: Ref,
    name: Option<Vec<u8>>,
    type_: GfxFontType,
    emb_font_id: Ref,
    emb_font_name: Option<Vec<u8>>,
    base14_name: Option<&'static str>,
    has_encoding: bool,
    uses_mac_roman_enc: bool,
    /// `enc[256]` for 8-bit fonts
    enc: Vec<Option<Vec<u8>>>,
}

impl GfxFont {
    /// `GfxFont::makeFont(XRef *xref, const char *tagA, Ref idA, Dict
    /// *fontDict)`.
    pub fn make_font(xref: &XRef, tag: &[u8], id: Ref, font_dict: &Dict) -> GfxFont {
        // get base font name
        let base_font = font_dict.lookup("BaseFont");
        let name = match &base_font {
            Object::Name(n) => Some(n.to_vec()),
            Object::String(s) => Some(s.to_vec()),
            _ => None,
        };

        // get embedded font ID and font type
        let (type_, emb_font_id) = get_font_type(xref, font_dict);

        let mut f = GfxFont {
            tag: tag.to_vec(),
            id,
            name,
            type_,
            emb_font_id,
            emb_font_name: None,
            base14_name: None,
            has_encoding: false,
            uses_mac_roman_enc: false,
            enc: Vec::new(),
        };
        if type_ < GfxFontType::CIDType0 {
            f.init_8bit(xref, font_dict);
        }
        f
    }

    /// The encoding part of `Gfx8BitFont::Gfx8BitFont(...)`.
    fn init_8bit(&mut self, xref: &XRef, font_dict: &Dict) {
        // do font name substitution for various aliases of the Base 14
        // font names
        if let Some(name) = &self.name {
            let name2: Vec<u8> = name.iter().copied().filter(|&c| c != b' ').collect();
            let mut a = 0usize;
            let mut b = BASE14_FONT_MAP.len(); // sizeof(base14FontMap) / sizeof(entry)
            // invariant: base14FontMap[a].altName <= name2 <
            // base14FontMap[b].altName
            while b - a > 1 {
                let m = (a + b) / 2;
                if crate::catalog::gstring_cmp(&name2, BASE14_FONT_MAP[m].0.as_bytes()) >= 0 {
                    a = m;
                } else {
                    b = m;
                }
            }
            if crate::catalog::gstring_cmp(&name2, BASE14_FONT_MAP[a].0.as_bytes()) == 0 {
                self.base14_name = Some(BASE14_FONT_MAP[a].1);
            }
        }

        // is it a built-in font?
        let builtin_font: Option<BuiltinEnc> =
            self.base14_name.and_then(|b14| BUILTIN_FONTS.iter().find(|(n, _)| *n == b14).map(|(_, e)| *e));

        //----- build the font encoding -----
        self.has_encoding = false;
        self.uses_mac_roman_enc = false;
        let mut base_enc_from_font_file = false;
        let obj1 = font_dict.lookup("Encoding");
        let name_enc = |n: &Object, has: &mut bool, mac: &mut bool| -> Option<BaseEnc> {
            if n.is_name_of("MacRomanEncoding") {
                *has = true;
                *mac = true;
                Some(BaseEnc::Table(&MAC_ROMAN_ENCODING))
            } else if n.is_name_of("MacExpertEncoding") {
                *has = true;
                Some(BaseEnc::Table(&MAC_EXPERT_ENCODING))
            } else if n.is_name_of("WinAnsiEncoding") {
                *has = true;
                Some(BaseEnc::Table(&WIN_ANSI_ENCODING))
            } else {
                None
            }
        };
        let mut base_enc: Option<BaseEnc> = if obj1.is_dict() {
            let obj2 = obj1.dict_lookup("BaseEncoding");
            name_enc(&obj2, &mut self.has_encoding, &mut self.uses_mac_roman_enc)
        } else {
            name_enc(&obj1, &mut self.has_encoding, &mut self.uses_mac_roman_enc)
        };

        // a non-embedded Symbol or ZapfDingbats font should never use one
        // of the non-symbol encodings
        if let Some(be) = builtin_font {
            if self.emb_font_id.num < 0
                && (be == BuiltinEnc::Symbol || be == BuiltinEnc::ZapfDingbats)
                && !obj1.is_dict()
            {
                base_enc = None;
            }
        }

        // check embedded font file for base encoding (only for Type 1
        // fonts - trying to get an encoding out of a TrueType font is a
        // losing proposition)
        let mut ff_t1: Option<FoFiType1> = None;
        let mut ff_t1c: Option<FoFiType1C> = None;
        if self.type_ == GfxFontType::Type1 && self.emb_font_id.num >= 0 {
            if let Some(buf) = self.read_emb_font_file(xref) {
                ff_t1 = Some(FoFiType1::make(&buf));
            }
        } else if self.type_ == GfxFontType::Type1C && self.emb_font_id.num >= 0 {
            if let Some(buf) = self.read_emb_font_file(xref) {
                ff_t1c = FoFiType1C::make(&buf);
            }
        }
        if let Some(ff) = &ff_t1 {
            if let Some(n) = ff.get_name() {
                self.emb_font_name = Some(n.to_vec());
            }
            if base_enc.is_none() {
                base_enc = ff.get_encoding().map(BaseEnc::FontFile);
                base_enc_from_font_file = true;
            }
        }
        if let Some(ff) = &ff_t1c {
            if let Some(n) = ff.get_name() {
                self.emb_font_name = Some(n.to_vec());
            }
            if base_enc.is_none() {
                base_enc = ff.get_encoding().map(BaseEnc::FontFile);
                base_enc_from_font_file = true;
            }
        }

        // get default base encoding
        if base_enc.is_none() {
            if let (Some(be), true) = (builtin_font, self.emb_font_id.num < 0) {
                base_enc = Some(BaseEnc::Table(builtin_table(be)));
                self.has_encoding = true;
            } else if self.type_ == GfxFontType::TrueType {
                base_enc = Some(BaseEnc::Table(&WIN_ANSI_ENCODING));
            } else {
                base_enc = Some(BaseEnc::Table(&STANDARD_ENCODING));
            }
        }

        // copy the base encoding
        let mut enc: Vec<Option<Vec<u8>>> = match base_enc.unwrap() {
            BaseEnc::Table(t) => t.iter().map(|o| o.map(|s| s.as_bytes().to_vec())).collect(),
            BaseEnc::FontFile(e) => e.clone(),
        };

        // some Type 1C font files have empty encodings, which can break
        // the T1C->T1 conversion (since the 'seac' operator depends on
        // having the accents in the encoding), so we fill in any gaps
        // from StandardEncoding
        if self.type_ == GfxFontType::Type1C && self.emb_font_id.num >= 0 && base_enc_from_font_file {
            for (i, e) in enc.iter_mut().enumerate() {
                if e.is_none() {
                    if let Some(s) = STANDARD_ENCODING[i] {
                        *e = Some(s.as_bytes().to_vec());
                    }
                }
            }
        }

        // merge differences into encoding
        if obj1.is_dict() {
            let obj2 = obj1.dict_lookup("Differences");
            if obj2.is_array() {
                self.has_encoding = true;
                let mut code: i32 = 0;
                for i in 0..obj2.array_get_length() {
                    let obj3 = obj2.array_get(i);
                    match &obj3 {
                        Object::Int(c) => code = *c,
                        Object::Name(n) => {
                            if (0..256).contains(&code) {
                                enc[code as usize] = Some(n.to_vec());
                            }
                            code = code.wrapping_add(1);
                        }
                        _ => error(
                            ErrorCategory::SyntaxError,
                            -1,
                            &format!(
                                "Wrong type in font encoding resource differences ({})",
                                obj3.get_type_name()
                            ),
                        ),
                    }
                }
            }
        }
        self.enc = enc;
    }

    /// `GfxFont::readEmbFontFile(XRef *xref, int *len)`: the decoded font
    /// file, or `None` (and the embedded font ID is cleared) if the
    /// object is not a stream.
    fn read_emb_font_file(&mut self, xref: &XRef) -> Option<Vec<u8>> {
        let obj2 = xref.fetch(self.emb_font_id.num, self.emb_font_id.gen);
        match obj2.get_stream() {
            Some(s) => Some(s.get_decoded_data().unwrap_or_default()),
            None => {
                error(ErrorCategory::SyntaxError, -1, "Embedded font file is not a stream");
                self.emb_font_id.num = -1;
                None
            }
        }
    }

    /// `isCIDFont()`
    pub fn is_cid_font(&self) -> bool {
        self.type_ >= GfxFontType::CIDType0
    }
    /// `Gfx8BitFont::getCharName(int code)`: glyph name for a code, or
    /// `None` (pdfTeX then uses ".notdef").
    pub fn get_char_name(&self, code: i32) -> Option<&[u8]> {
        if !(0..256).contains(&code) || self.enc.is_empty() {
            return None;
        }
        self.enc[code as usize].as_deref()
    }
    /// `Gfx8BitFont::getEncoding()`
    pub fn get_encoding(&self) -> &[Option<Vec<u8>>] {
        &self.enc
    }
    pub fn get_tag(&self) -> &[u8] {
        &self.tag
    }
    pub fn get_id(&self) -> Ref {
        self.id
    }
    pub fn get_name(&self) -> Option<&[u8]> {
        self.name.as_deref()
    }
    pub fn get_type(&self) -> GfxFontType {
        self.type_
    }
    /// `getEmbeddedFontID(Ref *embID)`: `None` if there is no embedded
    /// font.
    pub fn get_embedded_font_id(&self) -> Option<Ref> {
        if self.emb_font_id.num >= 0 {
            Some(self.emb_font_id)
        } else {
            None
        }
    }
    pub fn get_embedded_font_name(&self) -> Option<&[u8]> {
        self.emb_font_name.as_deref()
    }
    /// Name of the Base 14 font this font's name is an alias of.
    pub fn get_base14_name(&self) -> Option<&'static str> {
        self.base14_name
    }
    pub fn get_has_encoding(&self) -> bool {
        self.has_encoding
    }
    pub fn get_uses_mac_roman_enc(&self) -> bool {
        self.uses_mac_roman_enc
    }
}

/// `GfxFont::getFontType(XRef *xref, Dict *fontDict, Ref *embID)`.
pub fn get_font_type(xref: &XRef, font_dict: &Dict) -> (GfxFontType, Ref) {
    use GfxFontType::*;
    let mut emb_id = Ref { num: -1, gen: -1 };
    let mut err = false;

    let subtype = font_dict.lookup("Subtype");
    let mut expected_type = Unknown;
    let mut is_type0 = false;
    if subtype.is_name_of("Type1") || subtype.is_name_of("MMType1") {
        expected_type = Type1;
    } else if subtype.is_name_of("Type1C") {
        expected_type = Type1C;
    } else if subtype.is_name_of("Type3") {
        expected_type = Type3;
    } else if subtype.is_name_of("TrueType") {
        expected_type = TrueType;
    } else if subtype.is_name_of("Type0") {
        is_type0 = true;
    } else {
        error(
            ErrorCategory::SyntaxWarning,
            -1,
            &format!(
                "Unknown font type: '{}'",
                if subtype.is_name() { subtype.get_name_str() } else { "???".into() }
            ),
        );
    }

    let mut font_dict2: Dict = font_dict.clone();
    let obj1 = font_dict.lookup("DescendantFonts");
    if obj1.is_array() {
        if obj1.array_get_length() == 0 {
            error(ErrorCategory::SyntaxWarning, -1, "Empty DescendantFonts array in font");
        } else {
            let obj2 = obj1.array_get(0);
            if let Object::Dict(d) = &obj2 {
                if !is_type0 {
                    error(ErrorCategory::SyntaxWarning, -1, "Non-CID font with DescendantFonts array");
                }
                font_dict2 = (**d).clone();
                let st = font_dict2.lookup("Subtype");
                if st.is_name_of("CIDFontType0") {
                    if is_type0 {
                        expected_type = CIDType0;
                    }
                } else if st.is_name_of("CIDFontType2") && is_type0 {
                    expected_type = CIDType2;
                }
            }
        }
    }

    // NB: the PDF spec doesn't say anything about precedence, but Adobe
    // uses FontFile3 over FontFile2 if both are present.
    let font_desc = font_dict2.lookup("FontDescriptor");
    if font_desc.is_dict() {
        let obj3 = font_desc.dict_lookup_nf("FontFile3");
        if let Object::Ref(r) = obj3 {
            emb_id = r;
            let obj4 = obj3.fetch(Some(xref));
            if let Some(s) = obj4.get_stream() {
                let st = s.get_dict().lookup("Subtype");
                if st.is_name_of("Type1") {
                    if expected_type != Type1 {
                        err = true;
                        expected_type = if is_type0 { CIDType0 } else { Type1 };
                    }
                } else if st.is_name_of("Type1C") {
                    if expected_type == Type1 {
                        expected_type = Type1C;
                    } else if expected_type != Type1C {
                        err = true;
                        expected_type = if is_type0 { CIDType0C } else { Type1C };
                    }
                } else if st.is_name_of("TrueType") {
                    if expected_type != TrueType {
                        err = true;
                        expected_type = if is_type0 { CIDType2 } else { TrueType };
                    }
                } else if st.is_name_of("CIDFontType0C") {
                    if expected_type == CIDType0 {
                        expected_type = CIDType0C;
                    } else {
                        err = true;
                        expected_type = if is_type0 { CIDType0C } else { Type1C };
                    }
                } else if st.is_name_of("OpenType") {
                    if expected_type == TrueType {
                        expected_type = TrueTypeOT;
                    } else if expected_type == Type1 {
                        expected_type = Type1COT;
                    } else if expected_type == CIDType0 {
                        expected_type = CIDType0COT;
                    } else if expected_type == CIDType2 {
                        expected_type = CIDType2OT;
                    } else {
                        err = true;
                    }
                } else {
                    error(
                        ErrorCategory::SyntaxError,
                        -1,
                        &format!("Unknown font type '{}'", if st.is_name() { st.get_name_str() } else { "???".into() }),
                    );
                }
            }
        }
        if emb_id.num == -1 {
            if let Object::Ref(r) = font_desc.dict_lookup_nf("FontFile2") {
                emb_id = r;
                if is_type0 {
                    expected_type = CIDType2;
                } else if expected_type != TrueType {
                    err = true;
                }
            }
        }
        if emb_id.num == -1 {
            if let Object::Ref(r) = font_desc.dict_lookup_nf("FontFile") {
                emb_id = r;
                if expected_type != Type1 {
                    err = true;
                }
            }
        }
    }

    let mut t = Unknown;
    if emb_id.num >= 0 {
        let obj4 = xref.fetch(emb_id.num, emb_id.gen);
        if let Some(s) = obj4.get_stream() {
            let data = s.get_decoded_data().unwrap_or_default();
            t = match identify(&data) {
                FoFiIdentifierType::Type1PFA | FoFiIdentifierType::Type1PFB => Type1,
                FoFiIdentifierType::CFF8Bit => {
                    if is_type0 {
                        CIDType0C
                    } else {
                        Type1C
                    }
                }
                FoFiIdentifierType::CFFCID => CIDType0C,
                FoFiIdentifierType::TrueType | FoFiIdentifierType::TrueTypeCollection => {
                    if is_type0 {
                        CIDType2
                    } else {
                        TrueType
                    }
                }
                FoFiIdentifierType::OpenTypeCFF8Bit => {
                    if is_type0 {
                        CIDType0COT
                    } else {
                        Type1COT
                    }
                }
                FoFiIdentifierType::OpenTypeCFFCID => CIDType0COT,
                _ => {
                    error(ErrorCategory::SyntaxError, -1, "Embedded font file may be invalid");
                    Unknown
                }
            };
        } else {
            error(ErrorCategory::SyntaxError, -1, "Embedded font object is wrong type");
        }
    }

    // If the embedded font doesn't exist, or it's not a stream object, or
    // we couldn't identify its type, then remove the reference so nothing
    // tries to use it later.
    if t == Unknown {
        emb_id = Ref { num: -1, gen: -1 };
        t = expected_type;
    }
    if t != expected_type {
        err = true;
    }
    if err {
        error(ErrorCategory::SyntaxWarning, -1, "Mismatch between font type and embedded font file");
    }
    (t, emb_id)
}
