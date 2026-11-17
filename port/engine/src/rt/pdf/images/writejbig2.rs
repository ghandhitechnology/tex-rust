//! Port of pdftexdir/writejbig2.c: JBIG2 pages (from sequential or random-access
//! organised files) as /JBIG2Decode image streams, with the page 0 (global) segments
//! written once per file as a /JBIG2Globals stream at the end of the run.

use std::collections::BTreeMap;

use super::{xfseek, CFile, ImageStruct, SEEK_CUR, SEEK_END, SEEK_SET};
use crate::gen::globals::Globals;
use crate::rt::bytes_to_string;

const M_SYMBOL_DICTIONARY: u32 = 0;
const M_INTERMEDIATE_TEXT_REGION: u32 = 4;
const M_IMMEDIATE_TEXT_REGION: u32 = 6;
const M_IMMEDIATE_LOSSLESS_TEXT_REGION: u32 = 7;
const M_PATTERN_DICTIONARY: u32 = 16;
const M_INTERMEDIATE_HALFTONE_REGION: u32 = 20;
const M_IMMEDIATE_HALFTONE_REGION: u32 = 22;
const M_IMMEDIATE_LOSSLESS_HALFTONE_REGION: u32 = 23;
const M_INTERMEDIATE_GENERIC_REGION: u32 = 36;
const M_IMMEDIATE_GENERIC_REGION: u32 = 38;
const M_IMMEDIATE_LOSSLESS_GENERIC_REGION: u32 = 39;
const M_INTERMEDIATE_GENERIC_REFINEMENT_REGION: u32 = 40;
const M_IMMEDIATE_GENERIC_REFINEMENT_REGION: u32 = 42;
const M_IMMEDIATE_LOSSLESS_GENERIC_REFINEMENT_REGION: u32 = 43;
const M_PAGE_INFORMATION: u32 = 48;
const M_END_OF_PAGE: u32 = 49;
const M_END_OF_STRIPE: u32 = 50;
const M_END_OF_FILE: u32 = 51;
const M_PROFILES: u32 = 52;
const M_TABLES: u32 = 53;
const M_EXTENSION: u32 = 62;

#[derive(Clone, Copy, PartialEq, Default)]
enum Phase {
    #[default]
    Initial,
    HaveInfo,
}

/// SEGINFO (unsigned long is 32 bits, off_t 64 bits on Windows)
#[derive(Clone, Default)]
struct SegInfo {
    segnum: u32,
    isrefered: bool,
    refers: bool,
    seghdrflags: u32,
    pageassocsizeflag: bool,
    reftosegcount: u32,
    countofrefered: u32,
    fieldlen: u32,
    segnumwidth: u32,
    segpage: i32,
    segdatalen: u32,
    hdrstart: i64,
    hdrend: i64,
    datastart: i64,
    dataend: i64,
    endofstripeflag: bool,
    endofpageflag: bool,
    pageinfoflag: bool,
    endoffileflag: bool,
}

/// PAGEINFO; `segments` is the LIST, `seg_tree` its AVL tree (segnum -> index)
#[derive(Default)]
struct PageInfo {
    segments: Vec<SegInfo>,
    seg_tree: Option<BTreeMap<u32, usize>>,
    pagenum: u32,
    width: u32,
    height: u32,
    xres: u32,
    yres: u32,
    pagesegmentflags: u32,
    stripinginfo: u32,
    stripedheight: u32,
}

/// FILEINFO
#[derive(Default)]
pub struct FileInfo {
    file: CFile,
    filename: Vec<u8>,
    filesize: i64,
    pages: Vec<PageInfo>,
    pages_tree: Option<BTreeMap<u32, usize>>,
    page0: Vec<PageInfo>,
    page0_tree: Option<BTreeMap<u32, usize>>,
    filehdrflags: u32,
    sequentialaccess: bool,
    numofpages: u32,
    streamstart: i64,
    pdfpage0objnum: u32,
    phase: Phase,
}

/// writejbig2.c file_tree (keyed by file name, strcmp order)
#[derive(Default)]
pub struct Jbig2State {
    file_tree: Option<BTreeMap<Vec<u8>, FileInfo>>,
}

#[derive(Clone, Copy)]
enum PageList {
    Pages,
    Page0,
}

fn checkseghdrflags(sip: &mut SegInfo) -> bool {
    sip.endofstripeflag = false;
    sip.endofpageflag = false;
    sip.pageinfoflag = false;
    sip.endoffileflag = false;
    match sip.seghdrflags & 0x3f {
        M_SYMBOL_DICTIONARY
        | M_INTERMEDIATE_TEXT_REGION
        | M_IMMEDIATE_TEXT_REGION
        | M_IMMEDIATE_LOSSLESS_TEXT_REGION
        | M_PATTERN_DICTIONARY
        | M_INTERMEDIATE_HALFTONE_REGION
        | M_IMMEDIATE_HALFTONE_REGION
        | M_IMMEDIATE_LOSSLESS_HALFTONE_REGION
        | M_INTERMEDIATE_GENERIC_REGION
        | M_IMMEDIATE_GENERIC_REGION
        | M_IMMEDIATE_LOSSLESS_GENERIC_REGION
        | M_INTERMEDIATE_GENERIC_REFINEMENT_REGION
        | M_IMMEDIATE_GENERIC_REFINEMENT_REGION
        | M_IMMEDIATE_LOSSLESS_GENERIC_REFINEMENT_REGION => {}
        M_PAGE_INFORMATION => sip.pageinfoflag = true,
        M_END_OF_PAGE => sip.endofpageflag = true,
        M_END_OF_STRIPE => sip.endofstripeflag = true,
        M_END_OF_FILE => sip.endoffileflag = true,
        M_PROFILES | M_TABLES | M_EXTENSION => {}
        _ => return false,
    }
    true
}

impl FileInfo {
    fn list(&mut self, l: PageList) -> &mut Vec<PageInfo> {
        match l {
            PageList::Pages => &mut self.pages,
            PageList::Page0 => &mut self.page0,
        }
    }
}

impl Globals {
    /// writejbig2.c ygetc
    fn ygetc(&mut self, f: &mut CFile) -> u32 {
        let c = f.getc();
        if c < 0 {
            self.pdftex_fail("getc() failed; premature end of JBIG2 image file");
        }
        c as u32
    }
    fn jb_read2bytes(&mut self, f: &mut CFile) -> u32 {
        let c = self.ygetc(f);
        (c << 8).wrapping_add(self.ygetc(f))
    }
    fn jb_read4bytes(&mut self, f: &mut CFile) -> u32 {
        let l = self.jb_read2bytes(f);
        (l << 16).wrapping_add(self.jb_read2bytes(f))
    }

    /// writejbig2.c readfilehdr
    fn readfilehdr(&mut self, fip: &mut FileInfo) {
        let name = bytes_to_string(&fip.filename);
        xfseek(&mut fip.file, 0, SEEK_SET, &name);
        const JBIG2_ID: [u32; 8] = [0x97, b'J' as u32, b'B' as u32, b'2' as u32, 0x0d, 0x0a, 0x1a, 0x0a];
        for id in JBIG2_ID {
            if self.ygetc(&mut fip.file) != id {
                self.pdftex_fail("readfilehdr(): reading JBIG2 image file failed: ID string missing");
            }
        }
        fip.filehdrflags = self.ygetc(&mut fip.file);
        fip.sequentialaccess = fip.filehdrflags & 0x01 != 0;
        if fip.sequentialaccess {
            xfseek(&mut fip.file, 0, SEEK_END, &name);
            fip.filesize = fip.file.tell();
            xfseek(&mut fip.file, 9, SEEK_SET, &name);
        }
        // if (!(fip->filehdrflags >> 1) & 0x01)
        if ((fip.filehdrflags >> 1) == 0) as u32 & 0x01 != 0 {
            fip.numofpages = self.jb_read4bytes(&mut fip.file);
        }
    }

    /// writejbig2.c readseghdr
    fn readseghdr(&mut self, fip: &mut FileInfo, sip: &mut SegInfo) -> bool {
        sip.hdrstart = fip.file.tell();
        if fip.sequentialaccess && sip.hdrstart == fip.filesize {
            return false;
        }
        sip.segnum = self.jb_read4bytes(&mut fip.file);
        sip.seghdrflags = self.ygetc(&mut fip.file);
        if !checkseghdrflags(sip) {
            self.pdftex_fail("checkseghdrflags(): unknown segment type in JBIG2 image file");
        }
        if fip.sequentialaccess && sip.endoffileflag {
            return true;
        }
        sip.pageassocsizeflag = (sip.seghdrflags >> 6) & 0x01 != 0;
        sip.reftosegcount = self.ygetc(&mut fip.file);
        sip.countofrefered = sip.reftosegcount >> 5;
        if sip.countofrefered < 5 {
            sip.fieldlen = 1;
        } else {
            sip.fieldlen = 5 + sip.countofrefered / 8;
            let name = bytes_to_string(&fip.filename);
            xfseek(&mut fip.file, sip.fieldlen as i64 - 1, SEEK_CUR, &name);
        }
        sip.segnumwidth = if sip.segnum <= 256 {
            1
        } else if sip.segnum <= 65536 {
            2
        } else {
            4
        };
        for _ in 0..sip.countofrefered {
            match sip.segnumwidth {
                1 => {
                    self.ygetc(&mut fip.file);
                }
                2 => {
                    self.jb_read2bytes(&mut fip.file);
                }
                _ => {
                    self.jb_read4bytes(&mut fip.file);
                }
            }
        }
        sip.segpage = if sip.pageassocsizeflag {
            self.jb_read4bytes(&mut fip.file) as i32
        } else {
            self.ygetc(&mut fip.file) as i32
        };
        sip.segdatalen = self.jb_read4bytes(&mut fip.file);
        sip.hdrend = fip.file.tell();
        true
    }

    /// writejbig2.c markpage0seg
    fn markpage0seg(&mut self, fip: &mut FileInfo, referedseg: u32) {
        let found = fip.page0[0].seg_tree.as_ref().expect("segments tree").get(&referedseg).copied();
        if let Some(si) = found {
            let (refers, cnt) = {
                let s = &fip.page0[0].segments[si];
                (s.refers, s.countofrefered)
            };
            if !refers && cnt > 0 {
                self.checkseghdr(fip, PageList::Page0, 0, si);
            }
            fip.page0[0].segments[si].isrefered = true;
        }
    }

    /// writejbig2.c checkseghdr (note: the seeks are relative to the current position)
    fn checkseghdr(&mut self, fip: &mut FileInfo, l: PageList, pi: usize, si: usize) {
        let name = bytes_to_string(&fip.filename);
        let s = fip.list(l)[pi].segments[si].clone();
        xfseek(&mut fip.file, s.fieldlen as i64 + 5, SEEK_CUR, &name);
        let mut referedseg: u32 = 0;
        for _ in 0..s.countofrefered {
            match s.segnumwidth {
                1 => referedseg = self.ygetc(&mut fip.file),
                2 => referedseg = self.jb_read2bytes(&mut fip.file),
                4 => referedseg = self.jb_read4bytes(&mut fip.file),
                _ => {}
            }
            if !fip.list(l)[pi].segments[si].refers {
                self.markpage0seg(fip, referedseg);
            }
        }
        if s.countofrefered > 0 {
            fip.list(l)[pi].segments[si].refers = true;
        }
        if s.pageassocsizeflag {
            xfseek(&mut fip.file, 8, SEEK_CUR, &name);
        } else {
            xfseek(&mut fip.file, 5, SEEK_CUR, &name);
        }
    }

    /// writejbig2.c writeseghdr
    fn writeseghdr(&mut self, fip: &mut FileInfo, l: PageList, pi: usize, si: usize) {
        let s = fip.list(l)[pi].segments[si].clone();
        for _ in 0..5 + s.fieldlen {
            let c = self.ygetc(&mut fip.file);
            self.pdfout(c as u8);
        }
        let mut referedseg: u32 = 0;
        for _ in 0..s.countofrefered {
            match s.segnumwidth {
                1 => {
                    referedseg = self.ygetc(&mut fip.file);
                    self.pdfout(referedseg as u8);
                }
                2 => {
                    referedseg = self.jb_read2bytes(&mut fip.file);
                    self.pdfout((referedseg >> 8) as u8);
                    self.pdfout(referedseg as u8);
                }
                4 => {
                    referedseg = self.jb_read4bytes(&mut fip.file);
                    self.pdfout((referedseg >> 24) as u8);
                    self.pdfout((referedseg >> 16) as u8);
                    self.pdfout((referedseg >> 8) as u8);
                    self.pdfout(referedseg as u8);
                }
                _ => {}
            }
            if !fip.page0.is_empty() && !fip.list(l)[pi].segments[si].refers {
                self.markpage0seg(fip, referedseg);
            }
        }
        if s.countofrefered > 0 {
            fip.list(l)[pi].segments[si].refers = true;
        }
        if s.pageassocsizeflag {
            for _ in 0..3 {
                self.ygetc(&mut fip.file);
                self.pdfout(0);
            }
        }
        self.ygetc(&mut fip.file);
        self.pdfout(if s.segpage > 0 { 1 } else { 0 });
        for _ in 0..4 {
            let c = self.ygetc(&mut fip.file);
            self.pdfout(c as u8);
        }
    }

    /// writejbig2.c findstreamstart
    fn findstreamstart(&mut self, fip: &mut FileInfo) -> i64 {
        let mut tmp = SegInfo::default();
        loop {
            self.readseghdr(fip, &mut tmp);
            if tmp.endoffileflag {
                break;
            }
        }
        fip.streamstart = tmp.hdrend;
        self.readfilehdr(fip);
        fip.streamstart
    }

    /// writejbig2.c rd_jbig2_info
    fn rd_jbig2_info(&mut self, fip: &mut FileInfo) {
        let mut streampos: i64 = 0;
        let mut currentpage: u32 = 0;
        let fname = fip.filename.clone();
        fip.file = self.img_xfopen(&fname);
        self.readfilehdr(fip);
        if !fip.sequentialaccess {
            streampos = self.findstreamstart(fip);
        }
        let name = bytes_to_string(&fip.filename);
        loop {
            let mut sip = SegInfo::default();
            if !self.readseghdr(fip, &mut sip) || sip.endoffileflag {
                break;
            }
            let l = if sip.segpage > 0 {
                if sip.segpage as u32 > currentpage {
                    fip.pages.push(PageInfo::default());
                    currentpage = sip.segpage as u32;
                }
                PageList::Pages
            } else {
                if fip.page0.is_empty() {
                    fip.page0.push(PageInfo::default());
                }
                PageList::Page0
            };
            if fip.list(l).is_empty() {
                // fip->pages.last->d with an empty list: a segment of page n > 0 always
                // creates the first page above, so this cannot happen
                break;
            }
            let pi = fip.list(l).len() - 1;
            if !fip.sequentialaccess {
                sip.datastart = streampos;
            } else {
                sip.datastart = sip.hdrend;
            }
            sip.dataend = sip.datastart + sip.segdatalen as i64;
            if !fip.sequentialaccess && (sip.pageinfoflag || sip.endofstripeflag) {
                xfseek(&mut fip.file, sip.datastart, SEEK_SET, &name);
            }
            let mut seekdist = sip.segdatalen as i64;
            if sip.pageinfoflag {
                let pagenum = sip.segpage as u32;
                let w = self.jb_read4bytes(&mut fip.file);
                let h = self.jb_read4bytes(&mut fip.file);
                let xr = self.jb_read4bytes(&mut fip.file);
                let yr = self.jb_read4bytes(&mut fip.file);
                let fl = self.ygetc(&mut fip.file);
                let si = self.jb_read2bytes(&mut fip.file);
                let pip = &mut fip.list(l)[pi];
                pip.pagenum = pagenum;
                pip.width = w;
                pip.height = h;
                pip.xres = xr;
                pip.yres = yr;
                pip.pagesegmentflags = fl;
                pip.stripinginfo = si;
                seekdist -= 19;
            }
            if sip.endofstripeflag {
                let sh = self.jb_read4bytes(&mut fip.file);
                fip.list(l)[pi].stripedheight = sh;
                seekdist -= 4;
            }
            if !fip.sequentialaccess && (sip.pageinfoflag || sip.endofstripeflag) {
                xfseek(&mut fip.file, sip.hdrend, SEEK_SET, &name);
            }
            if !fip.sequentialaccess {
                streampos += sip.segdatalen as i64;
            }
            if fip.sequentialaccess {
                xfseek(&mut fip.file, seekdist, SEEK_CUR, &name);
            }
            {
                let pip = &mut fip.list(l)[pi];
                if sip.endofpageflag && currentpage != 0 && (pip.stripinginfo >> 15) != 0 {
                    pip.height = pip.stripedheight;
                }
            }
            if !sip.endofpageflag {
                fip.list(l)[pi].segments.push(sip);
            }
        }
        fip.phase = Phase::HaveInfo;
        fip.file = CFile::default();
    }

    fn pages_maketree(list: &[PageInfo]) -> BTreeMap<u32, usize> {
        let mut t = BTreeMap::new();
        for (i, p) in list.iter().enumerate() {
            t.entry(p.pagenum).or_insert(i);
        }
        t
    }

    /// writejbig2.c getstreamlen
    fn getstreamlen(segs: &[SegInfo], refer: bool) -> i64 {
        segs.iter()
            .filter(|s| refer || s.isrefered)
            .map(|s| (s.hdrend - s.hdrstart) + (s.dataend - s.datastart))
            .sum()
    }

    /// writejbig2.c wr_jbig2
    fn wr_jbig2(&mut self, fip: &mut FileInfo, page: u32) {
        let (l, pi) = if page > 0 {
            let pi = *fip.pages_tree.as_ref().unwrap().get(&page).expect("pip != NULL");
            let (w, h, len) = {
                let p = &fip.pages[pi];
                (p.width, p.height, Self::getstreamlen(&p.segments, true))
            };
            self.pdf_puts(b"/Type /XObject\n");
            self.pdf_puts(b"/Subtype /Image\n");
            self.pdf_printf(&format!("/Width {}\n", w as i32));
            self.pdf_printf(&format!("/Height {}\n", h as i32));
            self.pdf_puts(b"/ColorSpace /DeviceGray\n");
            self.pdf_puts(b"/BitsPerComponent 1\n");
            self.pdf_printf(&format!("/Length {}\n", len));
            self.pdf_puts(b"/Filter [/JBIG2Decode]\n");
            if !fip.page0.is_empty() {
                if fip.pdfpage0objnum == 0 {
                    self.pdfcreateobj(0, 0);
                    fip.pdfpage0objnum = self.objptr as u32;
                }
                self.pdf_printf(&format!("/DecodeParms [<< /JBIG2Globals {} 0 R >>]\n", fip.pdfpage0objnum));
            }
            (PageList::Pages, pi)
        } else {
            let pi = *fip.page0_tree.as_ref().unwrap().get(&page).expect("pip != NULL");
            self.pdfbegindict(fip.pdfpage0objnum as i32, 0);
            let len = Self::getstreamlen(&fip.page0[pi].segments, false);
            self.pdf_printf(&format!("/Length {}\n", len));
            (PageList::Page0, pi)
        };
        self.pdf_puts(b">>\n");
        self.pdf_puts(b"stream\n");
        let fname = fip.filename.clone();
        fip.file = self.img_xfopen(&fname);
        let name = bytes_to_string(&fip.filename);
        let nseg = fip.list(l)[pi].segments.len();
        for si in 0..nseg {
            let s = fip.list(l)[pi].segments[si].clone();
            if s.isrefered || page > 0 {
                xfseek(&mut fip.file, s.hdrstart, SEEK_SET, &name);
                self.writeseghdr(fip, l, pi, si);
                let s = fip.list(l)[pi].segments[si].clone();
                xfseek(&mut fip.file, s.datastart, SEEK_SET, &name);
                let mut i = s.datastart;
                while i < s.dataend {
                    let c = self.ygetc(&mut fip.file);
                    self.pdfout(c as u8);
                    i += 1;
                }
            }
        }
        self.pdfendstream();
        fip.file = CFile::default();
    }

    fn jbig2_selected_page(&mut self, img: i32) -> i32 {
        match &self.img(img).st {
            ImageStruct::Jbig2(j) => j.selected_page,
            _ => 0,
        }
    }

    /// writejbig2.c read_jbig2_info
    pub fn read_jbig2_info(&mut self, img: i32) {
        let sel = self.jbig2_selected_page(img);
        if sel < 1 {
            self.pdftex_fail(&format!(
                "read_jbig2_info(): page {} not in JBIG2 image file; page must be > 0",
                sel
            ));
        }
        let name = self.img(img).image_name.clone().unwrap_or_default();
        let key = name.clone();
        let tree = self.rt.images.jbig2.file_tree.get_or_insert_with(BTreeMap::new);
        let mut fip = tree.remove(&key).unwrap_or_else(|| FileInfo { filename: name.clone(), ..Default::default() });
        if fip.phase == Phase::Initial {
            self.rd_jbig2_info(&mut fip);
            fip.pages_tree = Some(Self::pages_maketree(&fip.pages));
            if !fip.page0.is_empty() {
                fip.page0_tree = Some(Self::pages_maketree(&fip.page0));
                let mut t = BTreeMap::new();
                for (i, s) in fip.page0[0].segments.iter().enumerate() {
                    t.entry(s.segnum).or_insert(i);
                }
                fip.page0[0].seg_tree = Some(t);
            }
        }
        let pi = fip.pages_tree.as_ref().unwrap().get(&(sel as u32)).copied();
        let Some(pi) = pi else {
            self.rt.images.jbig2.file_tree.as_mut().unwrap().insert(key, fip);
            self.pdftex_fail(&format!("read_jbig2_info(): page {} not found in JBIG2 image file", sel));
        };
        let (np, w, h, xr, yr) = {
            let p = &fip.pages[pi];
            (fip.numofpages, p.width, p.height, p.xres, p.yres)
        };
        self.rt.images.jbig2.file_tree.as_mut().unwrap().insert(key, fip);
        let e = self.img(img);
        e.num_pages = np as i32;
        e.width = w as i32;
        e.height = h as i32;
        e.x_res = (xr as f64 * 0.0254 + 0.5) as i32;
        e.y_res = (yr as f64 * 0.0254 + 0.5) as i32;
    }

    /// writejbig2.c write_jbig2
    pub fn write_jbig2(&mut self, img: i32) {
        let key = self.img(img).image_name.clone().unwrap_or_default();
        let sel = self.jbig2_selected_page(img);
        let mut fip = self.rt.images.jbig2.file_tree.as_mut().and_then(|t| t.remove(&key)).expect("fip != NULL");
        let pi = *fip.pages_tree.as_ref().unwrap().get(&(sel as u32)).expect("pip != NULL");
        let pagenum = fip.pages[pi].pagenum;
        self.wr_jbig2(&mut fip, pagenum);
        self.rt.images.jbig2.file_tree.as_mut().unwrap().insert(key, fip);
    }

    /// writejbig2.c flushjbig2page0objects
    pub fn flushjbig2page0objects(&mut self) {
        let Some(tree) = self.rt.images.jbig2.file_tree.take() else { return };
        let mut tree = tree;
        let keys: Vec<Vec<u8>> = tree.keys().cloned().collect();
        for k in keys {
            let mut fip = tree.remove(&k).unwrap();
            if !fip.page0.is_empty() {
                self.wr_jbig2(&mut fip, 0);
            }
            tree.insert(k, fip);
        }
        self.rt.images.jbig2.file_tree = Some(tree);
    }
}
