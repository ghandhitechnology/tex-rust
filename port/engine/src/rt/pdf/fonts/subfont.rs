//! Port of pdftexdir/subfont.c: subfont definition (.sfd) files and `name@sfd@` map
//! entries.

use std::collections::BTreeMap;

use super::*;
use crate::gen::globals::Globals;
use crate::rt::kpse;

const SFD_BUF_SIZE: usize = SMALL_BUF_SIZE;

/// ptexlib.h subfont_entry
pub struct SubfontEntry {
    pub infix: Vec<u8>,
    pub charcodes: [i64; 256],
}

/// ptexlib.h sfd_entry: the subfont list in linked-list order (newest first).
pub struct SfdEntry {
    #[allow(dead_code)]
    pub name: Vec<u8>,
    pub subfonts: Vec<usize>,
}

#[derive(Default)]
pub struct SfdState {
    pub subfonts: Vec<SubfontEntry>,
    pub sfd: Vec<SfdEntry>,
    pub sfd_tree: BTreeMap<Vec<u8>, usize>,
    sfd_file: Option<CFile>,
    sfd_line: Vec<u8>,
}

impl Globals {
    /// subfont.c sfd_getline
    fn sfd_getline(&mut self, expect_eof: bool) {
        loop {
            if self.rt.fonts.sfd.sfd_file.as_ref().unwrap().feof() {
                if expect_eof {
                    return;
                }
                self.pdftex_fail("unexpected end of file");
            }
            let mut p: Vec<u8> = Vec::new();
            loop {
                let mut c = self.rt.fonts.sfd.sfd_file.as_mut().unwrap().getc();
                if !append_char_to_buf(&mut c, &mut p, SFD_BUF_SIZE) {
                    self.check_buf_fail("subfont.c", 95);
                }
                if c == 10 {
                    break;
                }
            }
            if !append_eol(&mut p, SFD_BUF_SIZE) {
                self.check_buf_fail("subfont.c", 97);
            }
            let short = p.len() < 2 || p[0] == b'#';
            p.push(0);
            self.rt.fonts.sfd.sfd_line = p;
            if !short {
                return;
            }
        }
    }

    /// subfont.c read_sfd
    fn read_sfd(&mut self, sfd_name: &[u8]) -> Option<usize> {
        self.rt.fonts.map.uncacheable = true;
        if let Some(&s) = self.rt.fonts.sfd.sfd_tree.get(sfd_name) {
            return Some(s);
        }
        self.set_cur_file_name(sfd_name);
        let f = match self.font_open_input(kpse::SFD_FORMAT) {
            None => {
                self.pdftex_warn("cannot open SFD file for reading");
                self.set_cur_file_name_none();
                return None;
            }
            Some(f) => f,
        };
        self.rt.fonts.sfd.sfd_file = Some(f);
        self.tex_printf("{");
        let cur = self.rt.pdfu.cur_file_name.clone().unwrap_or_default();
        self.tex_printf_b(cur.as_bytes());
        let mut list: Vec<usize> = Vec::new();
        self.rt.fonts.sfd.sfd_line = vec![0];
        while !self.rt.fonts.sfd.sfd_file.as_ref().unwrap().feof() {
            self.sfd_getline(true);
            if self.rt.fonts.sfd.sfd_line[0] == 10 {
                break;
            }
            let line = self.rt.fonts.sfd.sfd_line.clone();
            let (infix, e) = scan_word(&line, 0, usize::MAX).unwrap_or((Vec::new(), 0));
            let mut sf = SubfontEntry { infix, charcodes: [-1; 256] };
            let mut s = line;
            let mut p = skip_ws(&s, e);
            let mut k: i64 = 0;
            loop {
                if at(&s, p) == b'\\' {
                    self.sfd_getline(false);
                    s = self.rt.fonts.sfd.sfd_line.clone();
                    p = 0;
                    continue;
                } else if at(&s, p) == 0 {
                    break;
                }
                let (mut i, e) = match scan_int(&s, p, 0) {
                    Some((v, e)) => (v as i32 as i64, e),
                    None => {
                        let m = format!("invalid token:\n{}", bs(c_str(&s, p)));
                        self.pdftex_fail(&m);
                    }
                };
                p = skip_ws(&s, e);
                if at(&s, p) == b':' {
                    k = i;
                    p += 1;
                } else if at(&s, p) == b'_' {
                    let (j, e) = match scan_int(&s, p + 1, 0) {
                        Some((v, e)) => (v as i32 as i64, e),
                        None => {
                            let m = format!("invalid token:\n{}", bs(c_str(&s, p)));
                            self.pdftex_fail(&m);
                        }
                    };
                    if i > j || k + (j - i) > 255 {
                        let m = format!("invalid range:\n{}", bs(c_str(&s, p)));
                        self.pdftex_fail(&m);
                    }
                    while i <= j {
                        if (0..256).contains(&k) {
                            sf.charcodes[k as usize] = i;
                        }
                        k += 1;
                        i += 1;
                    }
                    p = skip_ws(&s, e);
                } else {
                    if (0..256).contains(&k) {
                        sf.charcodes[k as usize] = i;
                    }
                    k += 1;
                }
            }
            self.rt.fonts.sfd.subfonts.push(sf);
            list.insert(0, self.rt.fonts.sfd.subfonts.len() - 1);
        }
        self.rt.fonts.sfd.sfd_file = None;
        self.tex_printf("}");
        let st = &mut self.rt.fonts.sfd;
        st.sfd.push(SfdEntry { name: sfd_name.to_vec(), subfonts: list });
        let idx = st.sfd.len() - 1;
        st.sfd_tree.insert(sfd_name.to_vec(), idx);
        Some(idx)
    }

    /// subfont.c handle_subfont_fm: None if fm was a subfont entry (and has been handled),
    /// otherwise the entry is given back.
    pub fn handle_subfont_fm(&mut self, mut fm: FmEntry, mode: i32) -> Option<FmEntry> {
        let p = fm.tfm().to_vec();
        let Some(q) = p.iter().position(|&c| c == b'@') else { return Some(fm) };
        let Some(r) = p[q + 1..].iter().position(|&c| c == b'@').map(|x| x + q + 1) else { return Some(fm) };
        if q == 0 || r <= q + 1 || r != p.len() - 1 {
            return Some(fm);
        }
        let mut buf = p[q + 1..r].to_vec();
        if buf.len() + 4 > SMALL_BUF_SIZE {
            self.check_buf_fail("subfont.c", 196);
        }
        buf.extend_from_slice(b".sfd");
        let Some(sfd) = self.read_sfd(&buf) else { return Some(fm) };
        fm.type_ |= F_SUBFONT;
        fm.ps_name = None;
        if fm.pid == -1 {
            fm.pid = 3;
            fm.eid = 1;
        }
        let list = self.rt.fonts.sfd.sfd[sfd].subfonts.clone();
        for sf in list {
            let mut name = p[..q].to_vec();
            name.extend_from_slice(&self.rt.fonts.sfd.subfonts[sf].infix);
            let mut fm2 = super::mapfile::new_fm_entry();
            fm2.tfm_name = Some(name);
            fm2.ff_name = fm.ff_name.clone();
            fm2.type_ = fm.type_;
            fm2.pid = fm.pid;
            fm2.eid = fm.eid;
            fm2.subfont = Some(sf);
            let idx = self.fm_alloc(fm2);
            self.avl_do_entry(idx, mode);
        }
        None
    }

    /// subfont.c sfd_free
    pub fn sfd_free(&mut self) {
        self.rt.fonts.sfd.sfd_tree.clear();
    }
}
