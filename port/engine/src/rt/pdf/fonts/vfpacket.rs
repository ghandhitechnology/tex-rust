//! Port of pdftexdir/vfpacket.c: storage of virtual font character packets.

use super::*;
use crate::gen::globals::Globals;

/// vfpacket.c vf_entry
#[derive(Default)]
pub struct VfEntry {
    pub data: Vec<Option<Vec<u8>>>,
    pub len: Vec<i32>,
}

/// vfpacket.c static state: vf_array, packet_array (stack), packet_data_ptr.
#[derive(Default)]
pub struct VfState {
    pub vf: Vec<VfEntry>,
    /// packet_data_ptr: (vf index, char index, offset)
    data_ptr: (usize, usize, usize),
    stack: Vec<((usize, usize, usize), i32)>,
}

impl Globals {
    /// vfpacket.c newvfpacket
    pub fn newvfpacket(&mut self, f: i32) -> i32 {
        let n = (*ix(self.fontec, f) as i32 - *ix(self.fontbc, f) as i32 + 1).max(0) as usize;
        let v = &mut self.rt.fonts.vf.vf;
        v.push(VfEntry { data: vec![None; n], len: vec![0; n] });
        (v.len() - 1) as i32
    }

    /// vfpacket.c storepacket
    pub fn storepacket(&mut self, f: i32, c: i32, s: i32) {
        let data = self.tex_string(s);
        let l = data.len() as i32;
        let b = *ix(self.vfpacketbase, f) as usize;
        let k = (c - *ix(self.fontbc, f) as i32) as usize;
        let e = &mut self.rt.fonts.vf.vf[b];
        e.len[k] = l;
        e.data[k] = Some(data);
    }

    /// vfpacket.c startpacket
    pub fn startpacket(&mut self, f: i32, c: i32) {
        let b = *ix(self.vfpacketbase, f) as usize;
        let k = (c - *ix(self.fontbc, f) as i32) as usize;
        self.rt.fonts.vf.data_ptr = (b, k, 0);
        self.vfpacketlength = self.rt.fonts.vf.vf[b].len[k];
    }

    /// vfpacket.c packetbyte
    pub fn packetbyte(&mut self) -> u8 {
        self.vfpacketlength -= 1;
        let (b, k, o) = self.rt.fonts.vf.data_ptr;
        self.rt.fonts.vf.data_ptr.2 += 1;
        match &self.rt.fonts.vf.vf[b].data[k] {
            Some(d) if o < d.len() => d[o],
            _ => 0,
        }
    }

    /// vfpacket.c pushpacketstate
    pub fn pushpacketstate(&mut self) {
        let st = (self.rt.fonts.vf.data_ptr, self.vfpacketlength);
        self.rt.fonts.vf.stack.push(st);
    }

    /// vfpacket.c poppacketstate
    pub fn poppacketstate(&mut self) {
        match self.rt.fonts.vf.stack.pop() {
            None => self.pdftex_fail("packet stack empty, impossible to pop"),
            Some((p, l)) => {
                self.rt.fonts.vf.data_ptr = p;
                self.vfpacketlength = l;
            }
        }
    }

    /// vfpacket.c vf_free
    pub fn vf_free(&mut self) {
        self.rt.fonts.vf = VfState::default();
    }
}
