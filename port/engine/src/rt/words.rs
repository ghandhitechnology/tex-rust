//! web2c's memory words (texk/web2c/texmfmem.h), little-endian, 64-bit halfwords.
//!
//! Byte layout of `memoryword` (8 bytes):
//!   0..4  LH (i32)        twohalves.u: B1 = bytes 0..2 (short), B0 = bytes 2..4 (short)
//!   4..8  RH (i32)        cint = bytes 4..8; qqqq: B3,B2,B1,B0 = bytes 4,5,6,7
//!   0..8  gr (f64)
//! `fmemoryword` (4 bytes): cint = bytes 0..4; qqqq: B3,B2,B1,B0 = bytes 0,1,2,3.
//! The layout is reproduced bit for bit so format files and aliasing between views
//! behave exactly as in the C build.

#[repr(C, align(8))]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct MemoryWord {
    pub lh: i32,
    pub rh: i32,
}

#[repr(C, align(8))]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct TwoHalves {
    pub lh: i32,
    pub rh: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct FourQuarters {
    pub b3: u8,
    pub b2: u8,
    pub b1: u8,
    pub b0: u8,
}

#[repr(C, align(4))]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct FMemoryWord {
    pub v: i32,
}

#[inline(always)]
fn hi16(x: i32) -> i16 {
    (x >> 16) as i16
}
#[inline(always)]
fn lo16(x: i32) -> i16 {
    x as i16
}
#[inline(always)]
fn set_hi16(x: i32, v: i16) -> i32 {
    (x & 0xFFFF) | ((v as u16 as i32) << 16)
}
#[inline(always)]
fn set_lo16(x: i32, v: i16) -> i32 {
    (x & !0xFFFF) | (v as u16 as i32)
}
#[inline(always)]
fn byte(x: i32, k: u32) -> u8 {
    (x >> (8 * k)) as u8
}
#[inline(always)]
fn set_byte(x: i32, k: u32, v: u8) -> i32 {
    (x & !(0xFF << (8 * k))) | ((v as i32) << (8 * k))
}

impl FourQuarters {
    #[inline(always)]
    pub fn b0(&self) -> u8 {
        self.b0
    }
    #[inline(always)]
    pub fn b1(&self) -> u8 {
        self.b1
    }
    #[inline(always)]
    pub fn b2(&self) -> u8 {
        self.b2
    }
    #[inline(always)]
    pub fn b3(&self) -> u8 {
        self.b3
    }
    #[inline(always)]
    pub fn set_b0(&mut self, v: u8) {
        self.b0 = v
    }
    #[inline(always)]
    pub fn set_b1(&mut self, v: u8) {
        self.b1 = v
    }
    #[inline(always)]
    pub fn set_b2(&mut self, v: u8) {
        self.b2 = v
    }
    #[inline(always)]
    pub fn set_b3(&mut self, v: u8) {
        self.b3 = v
    }
    #[inline(always)]
    fn to_i32(self) -> i32 {
        i32::from_le_bytes([self.b3, self.b2, self.b1, self.b0])
    }
    #[inline(always)]
    fn from_i32(x: i32) -> Self {
        let b = x.to_le_bytes();
        FourQuarters { b3: b[0], b2: b[1], b1: b[2], b0: b[3] }
    }
}

impl TwoHalves {
    #[inline(always)]
    pub fn rh(&self) -> i32 {
        self.rh
    }
    #[inline(always)]
    pub fn lh(&self) -> i32 {
        self.lh
    }
    #[inline(always)]
    pub fn b0(&self) -> i16 {
        hi16(self.lh)
    }
    #[inline(always)]
    pub fn b1(&self) -> i16 {
        lo16(self.lh)
    }
    #[inline(always)]
    pub fn set_rh(&mut self, v: i32) {
        self.rh = v
    }
    #[inline(always)]
    pub fn set_lh(&mut self, v: i32) {
        self.lh = v
    }
    #[inline(always)]
    pub fn set_b0(&mut self, v: i16) {
        self.lh = set_hi16(self.lh, v)
    }
    #[inline(always)]
    pub fn set_b1(&mut self, v: i16) {
        self.lh = set_lo16(self.lh, v)
    }
}

impl MemoryWord {
    #[inline(always)]
    pub fn hh(&self) -> TwoHalves {
        TwoHalves { lh: self.lh, rh: self.rh }
    }
    #[inline(always)]
    pub fn set_hh(&mut self, v: TwoHalves) {
        self.lh = v.lh;
        self.rh = v.rh;
    }
    #[inline(always)]
    pub fn hh_rh(&self) -> i32 {
        self.rh
    }
    #[inline(always)]
    pub fn hh_lh(&self) -> i32 {
        self.lh
    }
    #[inline(always)]
    pub fn hh_b0(&self) -> i16 {
        hi16(self.lh)
    }
    #[inline(always)]
    pub fn hh_b1(&self) -> i16 {
        lo16(self.lh)
    }
    #[inline(always)]
    pub fn set_hh_rh(&mut self, v: i32) {
        self.rh = v
    }
    #[inline(always)]
    pub fn set_hh_lh(&mut self, v: i32) {
        self.lh = v
    }
    #[inline(always)]
    pub fn set_hh_b0(&mut self, v: i16) {
        self.lh = set_hi16(self.lh, v)
    }
    #[inline(always)]
    pub fn set_hh_b1(&mut self, v: i16) {
        self.lh = set_lo16(self.lh, v)
    }
    #[inline(always)]
    pub fn int(&self) -> i32 {
        self.rh
    }
    #[inline(always)]
    pub fn set_int(&mut self, v: i32) {
        self.rh = v
    }
    #[inline(always)]
    pub fn gr(&self) -> f64 {
        f64::from_bits(((self.rh as u32 as u64) << 32) | (self.lh as u32 as u64))
    }
    #[inline(always)]
    pub fn set_gr(&mut self, v: f64) {
        let b = v.to_bits();
        self.lh = b as u32 as i32;
        self.rh = (b >> 32) as u32 as i32;
    }
    #[inline(always)]
    pub fn qqqq(&self) -> FourQuarters {
        FourQuarters::from_i32(self.rh)
    }
    #[inline(always)]
    pub fn set_qqqq(&mut self, v: FourQuarters) {
        self.rh = v.to_i32()
    }
    #[inline(always)]
    pub fn qqqq_b0(&self) -> u8 {
        byte(self.rh, 3)
    }
    #[inline(always)]
    pub fn qqqq_b1(&self) -> u8 {
        byte(self.rh, 2)
    }
    #[inline(always)]
    pub fn qqqq_b2(&self) -> u8 {
        byte(self.rh, 1)
    }
    #[inline(always)]
    pub fn qqqq_b3(&self) -> u8 {
        byte(self.rh, 0)
    }
    #[inline(always)]
    pub fn set_qqqq_b0(&mut self, v: u8) {
        self.rh = set_byte(self.rh, 3, v)
    }
    #[inline(always)]
    pub fn set_qqqq_b1(&mut self, v: u8) {
        self.rh = set_byte(self.rh, 2, v)
    }
    #[inline(always)]
    pub fn set_qqqq_b2(&mut self, v: u8) {
        self.rh = set_byte(self.rh, 1, v)
    }
    #[inline(always)]
    pub fn set_qqqq_b3(&mut self, v: u8) {
        self.rh = set_byte(self.rh, 0, v)
    }
}

impl FMemoryWord {
    #[inline(always)]
    pub fn int(&self) -> i32 {
        self.v
    }
    #[inline(always)]
    pub fn set_int(&mut self, v: i32) {
        self.v = v
    }
    #[inline(always)]
    pub fn qqqq(&self) -> FourQuarters {
        FourQuarters::from_i32(self.v)
    }
    #[inline(always)]
    pub fn set_qqqq(&mut self, v: FourQuarters) {
        self.v = v.to_i32()
    }
    #[inline(always)]
    pub fn qqqq_b0(&self) -> u8 {
        byte(self.v, 3)
    }
    #[inline(always)]
    pub fn qqqq_b1(&self) -> u8 {
        byte(self.v, 2)
    }
    #[inline(always)]
    pub fn qqqq_b2(&self) -> u8 {
        byte(self.v, 1)
    }
    #[inline(always)]
    pub fn qqqq_b3(&self) -> u8 {
        byte(self.v, 0)
    }
    #[inline(always)]
    pub fn set_qqqq_b0(&mut self, v: u8) {
        self.v = set_byte(self.v, 3, v)
    }
    #[inline(always)]
    pub fn set_qqqq_b1(&mut self, v: u8) {
        self.v = set_byte(self.v, 2, v)
    }
    #[inline(always)]
    pub fn set_qqqq_b2(&mut self, v: u8) {
        self.v = set_byte(self.v, 1, v)
    }
    #[inline(always)]
    pub fn set_qqqq_b3(&mut self, v: u8) {
        self.v = set_byte(self.v, 0, v)
    }
}
