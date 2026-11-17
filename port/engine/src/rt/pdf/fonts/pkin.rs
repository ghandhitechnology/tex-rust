//! Port of pdftexdir/pkin.c: reading characters from a PK file (readchar), derived from
//! dvips' loadfont.c/download.c/unpack.c. The PK file is writet3.c's t3_file.

use crate::gen::globals::Globals;

/// ptexlib.h chardesc
#[derive(Default, Clone)]
pub struct CharDesc {
    pub charcode: i32,
    pub cwidth: i32,
    pub cheight: i32,
    pub xoff: i32,
    pub yoff: i32,
    pub xescape: i32,
    pub rastersize: i32,
    pub raster: Vec<i32>,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum RealFunc {
    #[default]
    PkPackedNum,
    Rest,
}

/// pkin.c static state
#[derive(Default)]
pub struct PkState {
    inputbyte: i32,
    flagbyte: i32,
    bitweight: i32,
    dynf: i32,
    repeatcount: i32,
    realfunc: RealFunc,
    pk_remainder: i32,
}

const GPOWER: [i32; 17] = [0, 1, 3, 7, 15, 31, 63, 127, 255, 511, 1023, 2047, 4095, 8191, 16383, 32767, 65535];

impl Globals {
    /// pkin.c pkbyte
    fn pkbyte(&mut self) -> i32 {
        let i = self.rt.fonts.t3.file.getc();
        if i == -1 {
            self.pdftex_fail("unexpected eof in pk file");
        }
        i as i16 as i32
    }

    fn pkduo(&mut self) -> i32 {
        let mut i = self.pkbyte();
        if i > 127 {
            i -= 256;
        }
        i * 256 + self.pkbyte()
    }

    fn pktrio(&mut self) -> i32 {
        let mut i = self.pkbyte();
        if i > 127 {
            i -= 256;
        }
        i = i * 256 + self.pkbyte();
        i * 256 + self.pkbyte()
    }

    fn pkquad(&mut self) -> i32 {
        let mut i = self.pkbyte();
        if i > 127 {
            i -= 256;
        }
        i = i.wrapping_mul(256).wrapping_add(self.pkbyte());
        i = i.wrapping_mul(256).wrapping_add(self.pkbyte());
        i.wrapping_mul(256).wrapping_add(self.pkbyte())
    }

    /// pkin.c getnyb
    fn getnyb(&mut self) -> i32 {
        let temp;
        if self.rt.fonts.pk.bitweight == 0 {
            self.rt.fonts.pk.bitweight = 16;
            let b = self.pkbyte();
            self.rt.fonts.pk.inputbyte = b;
            temp = b >> 4;
        } else {
            self.rt.fonts.pk.bitweight = 0;
            temp = self.rt.fonts.pk.inputbyte & 15;
        }
        temp as i16 as i32
    }

    /// pkin.c getbit
    fn getbit(&mut self) -> bool {
        self.rt.fonts.pk.bitweight >>= 1;
        if self.rt.fonts.pk.bitweight == 0 {
            let b = self.pkbyte();
            self.rt.fonts.pk.inputbyte = b;
            self.rt.fonts.pk.bitweight = 128;
        }
        self.rt.fonts.pk.inputbyte & self.rt.fonts.pk.bitweight != 0
    }

    fn realfunc(&mut self) -> i32 {
        match self.rt.fonts.pk.realfunc {
            RealFunc::PkPackedNum => self.pkpackednum(),
            RealFunc::Rest => self.pk_rest(),
        }
    }

    /// pkin.c pkpackednum
    fn pkpackednum(&mut self) -> i32 {
        let dynf = self.rt.fonts.pk.dynf;
        let mut i = self.getnyb();
        if i == 0 {
            let mut j;
            loop {
                j = self.getnyb();
                i += 1;
                if j != 0 {
                    break;
                }
            }
            if i > 3 {
                return self.handlehuge(i, j);
            }
            while i > 0 {
                j = j * 16 + self.getnyb();
                i -= 1;
            }
            j - 15 + (13 - dynf) * 16 + dynf
        } else if i <= dynf {
            i
        } else if i < 14 {
            (i - dynf - 1) * 16 + self.getnyb() + dynf + 1
        } else {
            let rc = if i == 14 { self.pkpackednum() } else { 1 };
            self.rt.fonts.pk.repeatcount = rc;
            self.realfunc()
        }
    }

    /// pkin.c rest
    fn pk_rest(&mut self) -> i32 {
        let pk = &mut self.rt.fonts.pk;
        if pk.pk_remainder < 0 {
            pk.pk_remainder = -pk.pk_remainder;
            0
        } else if pk.pk_remainder > 0 {
            if pk.pk_remainder > 4000 {
                pk.pk_remainder = 4000 - pk.pk_remainder;
                4000
            } else {
                let i = pk.pk_remainder;
                pk.pk_remainder = 0;
                pk.realfunc = RealFunc::PkPackedNum;
                i
            }
        } else {
            self.pdftex_fail("shouldn't happen");
        }
    }

    /// pkin.c handlehuge
    fn handlehuge(&mut self, mut i: i32, k: i32) -> i32 {
        let mut j: i32 = k;
        while i != 0 {
            j = (j << 4).wrapping_add(self.getnyb());
            i -= 1;
        }
        let dynf = self.rt.fonts.pk.dynf;
        self.rt.fonts.pk.pk_remainder = j - 15 + (13 - dynf) * 16 + dynf;
        self.rt.fonts.pk.realfunc = RealFunc::Rest;
        self.pk_rest()
    }

    /// pkin.c unpack
    fn unpack(&mut self, cd: &mut CharDesc) {
        let wordwidth = ((cd.cwidth + 15) / 16) as i16 as i32;
        let mut i = 2 * cd.cheight * wordwidth;
        if i <= 0 {
            i = 2;
        }
        if i > cd.rastersize {
            cd.rastersize = i;
            cd.raster = vec![0; i as usize];
        }
        if cd.raster.len() < cd.rastersize as usize {
            cd.raster.resize(cd.rastersize as usize, 0);
        }
        let mut raster = 0usize;
        self.rt.fonts.pk.realfunc = RealFunc::PkPackedNum;
        let flagbyte = self.rt.fonts.pk.flagbyte;
        self.rt.fonts.pk.dynf = flagbyte / 16;
        let mut turnon = flagbyte & 8 != 0;
        let put = |r: &mut Vec<i32>, i: usize, v: i32| {
            if i >= r.len() {
                r.resize(i + 1, 0);
            }
            r[i] = v;
        };
        if self.rt.fonts.pk.dynf == 14 {
            self.rt.fonts.pk.bitweight = 0;
            for _ in 1..=cd.cheight {
                let mut word = 0;
                let mut wordweight = 32768;
                for _ in 1..=cd.cwidth {
                    if self.getbit() {
                        word += wordweight;
                    }
                    wordweight >>= 1;
                    if wordweight == 0 {
                        put(&mut cd.raster, raster, word);
                        raster += 1;
                        word = 0;
                        wordweight = 32768;
                    }
                }
                if wordweight != 32768 {
                    put(&mut cd.raster, raster, word);
                    raster += 1;
                }
            }
        } else {
            let mut rowsleft: i16 = cd.cheight as i16;
            let mut hbit: i16 = cd.cwidth as i16;
            self.rt.fonts.pk.repeatcount = 0;
            let mut wordweight: i32 = 16;
            let mut word: i32 = 0;
            self.rt.fonts.pk.bitweight = 0;
            while rowsleft > 0 {
                let mut count = self.realfunc();
                while count != 0 {
                    if count < wordweight && count < hbit as i32 {
                        if turnon {
                            word += GPOWER[wordweight as usize] - GPOWER[(wordweight - count) as usize];
                        }
                        hbit = (hbit as i32 - count) as i16;
                        wordweight -= count;
                        count = 0;
                    } else if count >= hbit as i32 && (hbit as i32) <= wordweight {
                        if turnon {
                            word += GPOWER[wordweight as usize] - GPOWER[(wordweight - hbit as i32) as usize];
                        }
                        put(&mut cd.raster, raster, word);
                        raster += 1;
                        let rc = self.rt.fonts.pk.repeatcount;
                        for _ in 1..=rc {
                            for _ in 1..=wordwidth {
                                let v = if raster >= wordwidth as usize { cd.raster.get(raster - wordwidth as usize).copied().unwrap_or(0) } else { 0 };
                                put(&mut cd.raster, raster, v);
                                raster += 1;
                            }
                        }
                        rowsleft = (rowsleft as i32 - (rc + 1)) as i16;
                        self.rt.fonts.pk.repeatcount = 0;
                        word = 0;
                        wordweight = 16;
                        count -= hbit as i32;
                        hbit = cd.cwidth as i16;
                    } else {
                        if turnon {
                            word += GPOWER[wordweight as usize];
                        }
                        put(&mut cd.raster, raster, word);
                        raster += 1;
                        word = 0;
                        count -= wordweight;
                        hbit = (hbit as i32 - wordweight) as i16;
                        wordweight = 16;
                    }
                }
                turnon = !turnon;
            }
            if rowsleft != 0 || hbit as i32 != cd.cwidth {
                self.pdftex_fail("error while unpacking; more bits than required");
            }
        }
    }

    /// pkin.c readchar: check the preamble if asked, read the next character definition
    /// into cd; 0 if there is none.
    pub fn readchar(&mut self, check_preamble: bool, cd: &mut CharDesc) -> i32 {
        if check_preamble {
            if self.pkbyte() != 247 {
                self.pdftex_fail("bad pk file, expected pre");
            }
            if self.pkbyte() != 89 {
                self.pdftex_fail("bad version of pk file");
            }
            let mut i = self.pkbyte();
            while i > 0 {
                self.pkbyte();
                i -= 1;
            }
            self.pkquad();
            self.pkquad();
            self.pkquad();
            self.pkquad();
        }
        loop {
            let flagbyte = self.pkbyte();
            self.rt.fonts.pk.flagbyte = flagbyte;
            if flagbyte == 245 {
                break;
            }
            if flagbyte < 240 {
                let length: i32;
                match flagbyte & 7 {
                    0..=3 => {
                        length = (flagbyte & 7) * 256 + self.pkbyte() - 3;
                        cd.charcode = self.pkbyte();
                        self.pktrio();
                        cd.xescape = self.pkbyte();
                        cd.cwidth = self.pkbyte();
                        cd.cheight = self.pkbyte();
                        cd.xoff = self.pkbyte();
                        cd.yoff = self.pkbyte();
                        if cd.xoff > 127 {
                            cd.xoff -= 256;
                        }
                        if cd.yoff > 127 {
                            cd.yoff -= 256;
                        }
                    }
                    4..=6 => {
                        let mut l = (flagbyte & 3) * 65536 + self.pkbyte() * 256;
                        l = l + self.pkbyte() - 4;
                        length = l;
                        cd.charcode = self.pkbyte();
                        self.pktrio();
                        cd.xescape = self.pkduo();
                        cd.cwidth = self.pkduo();
                        cd.cheight = self.pkduo();
                        cd.xoff = self.pkduo();
                        cd.yoff = self.pkduo();
                    }
                    _ => {
                        length = self.pkquad().wrapping_sub(9);
                        cd.charcode = self.pkquad();
                        self.pkquad();
                        cd.xescape = self.pkquad();
                        self.pkquad();
                        cd.cwidth = self.pkquad();
                        cd.cheight = self.pkquad();
                        cd.xoff = self.pkquad();
                        cd.yoff = self.pkquad();
                    }
                }
                if length <= 0 {
                    self.pdftex_fail(&format!("packet length ({}) too small", length));
                }
                self.unpack(cd);
                return 1;
            } else {
                let mut k: i32 = 0;
                match flagbyte {
                    240..=243 => {
                        if flagbyte == 243 {
                            k = self.pkbyte();
                            if k > 127 {
                                k -= 256;
                            }
                        }
                        if flagbyte >= 242 {
                            k = k * 256 + self.pkbyte();
                        }
                        if flagbyte >= 241 {
                            k = k * 256 + self.pkbyte();
                        }
                        k = k * 256 + self.pkbyte();
                        while k > 0 {
                            k -= 1;
                            self.pkbyte();
                        }
                    }
                    244 => {
                        self.pkquad();
                    }
                    246 => {}
                    _ => self.pdftex_fail(&format!("unexpected command ({})", flagbyte)),
                }
            }
        }
        0
    }
}
