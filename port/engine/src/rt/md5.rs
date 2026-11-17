//! MD5 (RFC 1321), replacing web2c's libmd5 (md5.c by L. Peter Deutsch).

pub struct Md5 {
    state: [u32; 4],
    count: u64,
    buf: [u8; 64],
    buflen: usize,
}

const S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14,
    20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6,
    10, 15, 21,
];

fn k(i: usize) -> u32 {
    ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32
}

impl Default for Md5 {
    fn default() -> Self {
        Self::new()
    }
}

impl Md5 {
    pub fn new() -> Md5 {
        Md5 { state: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476], count: 0, buf: [0; 64], buflen: 0 }
    }

    fn block(&mut self, block: &[u8]) {
        let mut m = [0u32; 16];
        for i in 0..16 {
            m[i] = u32::from_le_bytes([block[4 * i], block[4 * i + 1], block[4 * i + 2], block[4 * i + 3]]);
        }
        let [mut a, mut b, mut c, mut d] = self.state;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let tmp = d;
            d = c;
            c = b;
            b = b.wrapping_add(a.wrapping_add(f).wrapping_add(k(i)).wrapping_add(m[g]).rotate_left(S[i]));
            a = tmp;
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.count += data.len() as u64;
        if self.buflen > 0 {
            let take = (64 - self.buflen).min(data.len());
            self.buf[self.buflen..self.buflen + take].copy_from_slice(&data[..take]);
            self.buflen += take;
            data = &data[take..];
            if self.buflen == 64 {
                let b = self.buf;
                self.block(&b);
                self.buflen = 0;
            } else {
                return; // input exhausted, partial block stays buffered
            }
        }
        while data.len() >= 64 {
            self.block(&data[..64]);
            data = &data[64..];
        }
        self.buf[..data.len()].copy_from_slice(data);
        self.buflen = data.len();
    }

    pub fn finish(mut self) -> [u8; 16] {
        let bits = self.count.wrapping_mul(8);
        self.update(&[0x80]);
        while self.buflen != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_le_bytes());
        let mut out = [0u8; 16];
        for i in 0..4 {
            out[4 * i..4 * i + 4].copy_from_slice(&self.state[i].to_le_bytes());
        }
        out
    }
}

pub fn md5(data: &[u8]) -> [u8; 16] {
    let mut m = Md5::new();
    m.update(data);
    m.finish()
}

#[cfg(test)]
mod tests {
    #[test]
    fn vectors() {
        let hex = |d: [u8; 16]| d.iter().map(|b| format!("{:02x}", b)).collect::<String>();
        assert_eq!(hex(super::md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(super::md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hex(super::md5(&[b'a'; 1000])), "cabe45dcc9ae5b66ba86600cca6b8ba8");
    }
}
