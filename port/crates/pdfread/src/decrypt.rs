//! Port of xpdf `Decrypt.h` / `Decrypt.cc`: file key computation for the
//! standard security handler (revisions 2-6), the per-object
//! `DecryptStream` (RC4, AES-128-CBC, AES-256-CBC with xpdf's padding
//! handling) and the hash/cipher primitives (MD5, SHA-256/384/512, RC4,
//! AES).
//!
//! The primitives are standard algorithms (any correct implementation
//! gives identical results); the xpdf-specific behaviour — how the IV is
//! taken, how a trailing partial block or bad padding is treated, the
//! revision 6 hash loop, password padding — is ported line by line.

/// Mirrors `enum CryptAlgorithm` (Stream.h).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CryptAlgorithm {
    RC4,
    AES,
    AES256,
}

/// Encryption parameters stored in the XRef (`XRef::fileKey`,
/// `keyLength`, `encAlgorithm`).
#[derive(Clone, Debug)]
pub struct FileKey {
    pub key: [u8; 32],
    pub key_length: usize,
    pub algo: CryptAlgorithm,
}

/// `passwordPad[32]`.
pub const PASSWORD_PAD: [u8; 32] = [
    0x28, 0xbf, 0x4e, 0x5e, 0x4e, 0x75, 0x8a, 0x41, 0x64, 0x00, 0x4e, 0x56, 0xff, 0xfa, 0x01, 0x08,
    0x2e, 0x2e, 0x00, 0xb6, 0xd0, 0x68, 0x3e, 0x80, 0x2f, 0x0c, 0xa9, 0xfe, 0x64, 0x53, 0x69, 0x7a,
];

// ------------------------------------------------------------------------
// Decrypt::makeFileKey
// ------------------------------------------------------------------------

/// Parameters of `Decrypt::makeFileKey`.
pub struct MakeFileKeyParams<'a> {
    pub enc_version: i32,
    pub enc_revision: i32,
    pub key_length: usize,
    pub owner_key: &'a [u8],
    pub user_key: &'a [u8],
    pub owner_enc: Option<&'a [u8]>,
    pub user_enc: Option<&'a [u8]>,
    pub permissions: i32,
    pub file_id: &'a [u8],
    pub owner_password: Option<&'a [u8]>,
    pub user_password: Option<&'a [u8]>,
    pub encrypt_metadata: bool,
}

/// `Decrypt::makeFileKey`.  Returns `(file_key, owner_password_ok)` on
/// success.
pub fn make_file_key(p: &MakeFileKeyParams) -> Option<([u8; 32], bool)> {
    if p.enc_revision == 5 || p.enc_revision == 6 {
        let ok_len = |s: &[u8], n: usize| s.len() >= n;
        // check the owner password
        if let Some(opw) = p.owner_password {
            let len = opw.len().min(127);
            if ok_len(p.owner_key, 48) && ok_len(p.user_key, 48) {
                let mut test = Vec::with_capacity(len + 56);
                test.extend_from_slice(&opw[..len]);
                test.extend_from_slice(&p.owner_key[32..40]);
                test.extend_from_slice(&p.user_key[..48]);
                let mut h = sha256(&test).to_vec();
                if p.enc_revision == 6 {
                    h = r6_hash(&h, &opw[..len], Some(&p.user_key[..48]));
                }
                if h[..32] == p.owner_key[..32] {
                    // compute the file key from the owner password
                    let mut test = Vec::with_capacity(len + 56);
                    test.extend_from_slice(&opw[..len]);
                    test.extend_from_slice(&p.owner_key[40..48]);
                    test.extend_from_slice(&p.user_key[..48]);
                    let mut h = sha256(&test).to_vec();
                    if p.enc_revision == 6 {
                        h = r6_hash(&h, &opw[..len], Some(&p.user_key[..48]));
                    }
                    let enc = p.owner_enc.unwrap_or(&[]);
                    let file_key = aes256_cbc_zero_iv_decrypt_32(&h[..32], enc);
                    return Some((file_key, true));
                }
            }
        }
        // check the user password
        let upw: &[u8] = match p.user_password {
            Some(u) => &u[..u.len().min(127)],
            None => b"",
        };
        if !ok_len(p.user_key, 48) {
            return None;
        }
        let mut test = Vec::with_capacity(upw.len() + 8);
        test.extend_from_slice(upw);
        test.extend_from_slice(&p.user_key[32..40]);
        let mut h = sha256(&test).to_vec();
        if p.enc_revision == 6 {
            h = r6_hash(&h, upw, None);
        }
        if h[..32] == p.user_key[..32] {
            // compute the file key from the user password
            let mut test = Vec::with_capacity(upw.len() + 8);
            test.extend_from_slice(upw);
            test.extend_from_slice(&p.user_key[40..48]);
            let mut h = sha256(&test).to_vec();
            if p.enc_revision == 6 {
                h = r6_hash(&h, upw, None);
            }
            let enc = p.user_enc.unwrap_or(&[]);
            let file_key = aes256_cbc_zero_iv_decrypt_32(&h[..32], enc);
            return Some((file_key, false));
        }
        None
    } else {
        let kl = p.key_length;
        // try using the supplied owner password to generate the user
        // password
        if let Some(opw) = p.owner_password {
            let mut test = [0u8; 32];
            if opw.len() < 32 {
                test[..opw.len()].copy_from_slice(opw);
                test[opw.len()..].copy_from_slice(&PASSWORD_PAD[..32 - opw.len()]);
            } else {
                test.copy_from_slice(&opw[..32]);
            }
            let mut t = md5(&test).to_vec();
            if p.enc_revision == 3 {
                for _ in 0..50 {
                    t = md5(&t[..kl]).to_vec();
                }
            }
            let mut test2 = [0u8; 32];
            if p.enc_revision == 2 {
                let mut st = Rc4::new(&t[..kl]);
                for i in 0..32 {
                    test2[i] = st.decrypt_byte(byte_at(p.owner_key, i));
                }
            } else {
                for i in 0..32 {
                    test2[i] = byte_at(p.owner_key, i);
                }
                for i in (0..=19u8).rev() {
                    let tmp_key: Vec<u8> = t[..kl].iter().map(|&b| b ^ i).collect();
                    let mut st = Rc4::new(&tmp_key);
                    for b in test2.iter_mut() {
                        *b = st.decrypt_byte(*b);
                    }
                }
            }
            if let Some(k) = make_file_key2(p, Some(&test2)) {
                return Some((k, true));
            }
        }
        // try using the supplied user password
        make_file_key2(p, p.user_password).map(|k| (k, false))
    }
}

#[inline]
fn byte_at(s: &[u8], i: usize) -> u8 {
    // GString::getChar on a string that the caller guarantees is at least
    // 32 bytes long (revision <= 4 keys are zero-padded to 32)
    s.get(i).copied().unwrap_or(0)
}

/// Decrypt the 32-byte OE/UE value with AES-256, IV = 0 (the two
/// `aes256DecryptBlock(..., gFalse)` calls in makeFileKey).
fn aes256_cbc_zero_iv_decrypt_32(key: &[u8], enc: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let aes = Aes::new_256(key);
    let mut cbc = [0u8; 16];
    for blk in 0..2 {
        let mut inb = [0u8; 16];
        for i in 0..16 {
            inb[i] = enc.get(blk * 16 + i).copied().unwrap_or(0);
        }
        let mut st = inb;
        aes.decrypt_block(&mut st);
        for i in 0..16 {
            out[blk * 16 + i] = st[i] ^ cbc[i];
        }
        cbc = inb;
    }
    out
}

/// `Decrypt::r6Hash` (ISO 32000-2 algorithm 2.B): `key_in` is the initial
/// SHA-256 hash, `user_key` the 48-byte U value for owner checks.
pub fn r6_hash(key_in: &[u8], pwd: &[u8], user_key: Option<&[u8]>) -> Vec<u8> {
    let mut key: Vec<u8> = key_in.to_vec(); // keyLen = 32 initially
    let mut i: i32 = 0;
    loop {
        let mut unit: Vec<u8> = Vec::with_capacity(pwd.len() + key.len() + 48);
        unit.extend_from_slice(pwd);
        unit.extend_from_slice(&key);
        if let Some(uk) = user_key {
            unit.extend_from_slice(&uk[..48]);
        }
        let mut key1: Vec<u8> = Vec::with_capacity(unit.len() * 64);
        for _ in 0..64 {
            key1.extend_from_slice(&unit);
        }
        let n = key1.len();
        // AES-128-CBC encrypt with key[0..16], IV = key[16..32]
        let aes = Aes::new_128(&key[..16]);
        let mut cbc = [0u8; 16];
        cbc.copy_from_slice(&key[16..32]);
        let mut j = 0;
        while j < n {
            let mut st = [0u8; 16];
            for k in 0..16 {
                st[k] = key1.get(j + k).copied().unwrap_or(0) ^ cbc[k];
            }
            aes.encrypt_block(&mut st);
            for k in 0..16 {
                if j + k < n {
                    key1[j + k] = st[k];
                }
            }
            cbc = st;
            j += 16;
        }
        let mut k: i32 = 0;
        for j in 0..16 {
            k += (key1[j] % 3) as i32;
        }
        k %= 3;
        key = match k {
            0 => sha256(&key1).to_vec(),
            1 => sha384(&key1).to_vec(),
            _ => sha512(&key1).to_vec(),
        };
        // from the spec, it appears that i should be incremented after
        // the test, but that doesn't match what Adobe does
        i += 1;
        if i >= 64 && (key1[n - 1] as i32) <= i - 32 {
            break;
        }
    }
    key
}

/// `Decrypt::makeFileKey2`.
fn make_file_key2(p: &MakeFileKeyParams, user_password: Option<&[u8]>) -> Option<[u8; 32]> {
    let kl = p.key_length;
    let mut buf: Vec<u8> = Vec::with_capacity(72 + p.file_id.len());
    match user_password {
        Some(u) => {
            if u.len() < 32 {
                buf.extend_from_slice(u);
                buf.extend_from_slice(&PASSWORD_PAD[..32 - u.len()]);
            } else {
                buf.extend_from_slice(&u[..32]);
            }
        }
        None => buf.extend_from_slice(&PASSWORD_PAD),
    }
    for i in 0..32 {
        buf.push(byte_at(p.owner_key, i));
    }
    let perm = p.permissions;
    buf.push((perm & 0xff) as u8);
    buf.push(((perm >> 8) & 0xff) as u8);
    buf.push(((perm >> 16) & 0xff) as u8);
    buf.push(((perm >> 24) & 0xff) as u8);
    buf.extend_from_slice(p.file_id);
    if !p.encrypt_metadata {
        buf.extend_from_slice(&[0xff, 0xff, 0xff, 0xff]);
    }
    let mut file_key = [0u8; 32];
    let d = md5(&buf);
    file_key[..16].copy_from_slice(&d);
    if p.enc_revision == 3 {
        for _ in 0..50 {
            let d = md5(&file_key[..kl]);
            file_key[..16].copy_from_slice(&d);
        }
    }

    // test user password
    let ok = if p.enc_revision == 2 {
        let mut st = Rc4::new(&file_key[..kl]);
        let mut test = [0u8; 32];
        for i in 0..32 {
            test[i] = st.decrypt_byte(byte_at(p.user_key, i));
        }
        test == PASSWORD_PAD
    } else if p.enc_revision == 3 {
        let mut test = [0u8; 32];
        for i in 0..32 {
            test[i] = byte_at(p.user_key, i);
        }
        for i in (0..=19u8).rev() {
            let tmp_key: Vec<u8> = file_key[..kl].iter().map(|&b| b ^ i).collect();
            let mut st = Rc4::new(&tmp_key);
            for b in test.iter_mut() {
                *b = st.decrypt_byte(*b);
            }
        }
        let mut b2: Vec<u8> = Vec::with_capacity(32 + p.file_id.len());
        b2.extend_from_slice(&PASSWORD_PAD);
        b2.extend_from_slice(p.file_id);
        let d = md5(&b2);
        test[..16] == d[..]
    } else {
        false
    };
    if ok {
        Some(file_key)
    } else {
        None
    }
}

// ------------------------------------------------------------------------
// DecryptStream
// ------------------------------------------------------------------------

/// Object key computed in the `DecryptStream` constructor.
fn make_obj_key(fk: &FileKey, obj_num: i32, obj_gen: i32) -> (Vec<u8>, usize) {
    let kl = fk.key_length;
    let mut obj_key = [0u8; 32];
    obj_key[..kl].copy_from_slice(&fk.key[..kl]);
    match fk.algo {
        CryptAlgorithm::RC4 | CryptAlgorithm::AES => {
            let mut v: Vec<u8> = fk.key[..kl].to_vec();
            v.push((obj_num & 0xff) as u8);
            v.push(((obj_num >> 8) & 0xff) as u8);
            v.push(((obj_num >> 16) & 0xff) as u8);
            v.push((obj_gen & 0xff) as u8);
            v.push(((obj_gen >> 8) & 0xff) as u8);
            if fk.algo == CryptAlgorithm::AES {
                v.extend_from_slice(b"sAlT");
            }
            let d = md5(&v);
            obj_key[..16].copy_from_slice(&d);
            let okl = (kl + 5).min(16);
            (obj_key.to_vec(), okl)
        }
        CryptAlgorithm::AES256 => (obj_key.to_vec(), kl),
    }
}

/// The byte sequence a `DecryptStream` wrapped around `data` yields
/// after `reset()` (repeated `getChar()` until EOF).
pub fn decrypt_bytes(data: &[u8], fk: &FileKey, obj_num: i32, obj_gen: i32) -> Vec<u8> {
    let (obj_key, obj_key_len) = make_obj_key(fk, obj_num, obj_gen);
    match fk.algo {
        CryptAlgorithm::RC4 => {
            let mut st = Rc4::new(&obj_key[..obj_key_len.max(1)]);
            data.iter().map(|&c| st.decrypt_byte(c)).collect()
        }
        CryptAlgorithm::AES | CryptAlgorithm::AES256 => {
            // aesKeyExpansion always reads 16 (resp. 32) key bytes
            let aes = if fk.algo == CryptAlgorithm::AES {
                Aes::new_128(&obj_key[..16])
            } else {
                Aes::new_256(&obj_key[..32])
            };
            let mut out = Vec::with_capacity(data.len());
            // reset(): str->getBlock(cbc, 16)
            let mut cbc = [0u8; 16];
            let n0 = data.len().min(16);
            cbc[..n0].copy_from_slice(&data[..n0]);
            let mut pos = n0;
            loop {
                // getChar(): bufIdx == 16 -> read a block
                if data.len() - pos < 16 {
                    break; // getBlock(in, 16) != 16 -> EOF
                }
                let mut inb = [0u8; 16];
                inb.copy_from_slice(&data[pos..pos + 16]);
                pos += 16;
                let last = pos >= data.len(); // str->lookChar() == EOF
                let mut st = inb;
                aes.decrypt_block(&mut st);
                let mut buf = [0u8; 16];
                for i in 0..16 {
                    buf[i] = st[i] ^ cbc[i];
                }
                cbc = inb;
                let mut buf_idx = 0usize;
                if last {
                    let mut n = buf[15] as usize;
                    if !(1..=16).contains(&n) {
                        n = 16;
                    }
                    let mut i = 15isize;
                    while i >= n as isize {
                        buf[i as usize] = buf[i as usize - n];
                        i -= 1;
                    }
                    buf_idx = n;
                }
                out.extend_from_slice(&buf[buf_idx..]);
                if last {
                    break;
                }
            }
            out
        }
    }
}

// ------------------------------------------------------------------------
// RC4
// ------------------------------------------------------------------------

/// `rc4InitKey` / `rc4DecryptByte`.
pub struct Rc4 {
    state: [u8; 256],
    x: u8,
    y: u8,
}

impl Rc4 {
    pub fn new(key: &[u8]) -> Rc4 {
        let mut state = [0u8; 256];
        for (i, s) in state.iter_mut().enumerate() {
            *s = i as u8;
        }
        let key_len = key.len().max(1);
        let mut index1: usize = 0;
        let mut index2: u8 = 0;
        for i in 0..256 {
            let k = key.get(index1).copied().unwrap_or(0);
            index2 = k.wrapping_add(state[i]).wrapping_add(index2);
            state.swap(i, index2 as usize);
            index1 = (index1 + 1) % key_len;
        }
        Rc4 { state, x: 0, y: 0 }
    }
    pub fn decrypt_byte(&mut self, c: u8) -> u8 {
        self.x = self.x.wrapping_add(1);
        let x1 = self.x;
        self.y = self.state[x1 as usize].wrapping_add(self.y);
        let y1 = self.y;
        let tx = self.state[x1 as usize];
        let ty = self.state[y1 as usize];
        self.state[x1 as usize] = ty;
        self.state[y1 as usize] = tx;
        c ^ self.state[(tx.wrapping_add(ty)) as usize]
    }
}

// ------------------------------------------------------------------------
// AES (FIPS-197)
// ------------------------------------------------------------------------

struct AesTables {
    sbox: [u8; 256],
    inv_sbox: [u8; 256],
}

fn gf_mul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0u8;
    while b != 0 {
        if b & 1 != 0 {
            p ^= a;
        }
        let hi = a & 0x80;
        a <<= 1;
        if hi != 0 {
            a ^= 0x1b;
        }
        b >>= 1;
    }
    p
}

fn aes_tables() -> &'static AesTables {
    use std::sync::OnceLock;
    static T: OnceLock<AesTables> = OnceLock::new();
    T.get_or_init(|| {
        let mut sbox = [0u8; 256];
        let mut inv_sbox = [0u8; 256];
        for i in 0..256usize {
            // multiplicative inverse in GF(2^8)
            let mut inv = 0u8;
            if i != 0 {
                for j in 1..256usize {
                    if gf_mul(i as u8, j as u8) == 1 {
                        inv = j as u8;
                        break;
                    }
                }
            }
            let b = inv;
            let s = b ^ b.rotate_left(1) ^ b.rotate_left(2) ^ b.rotate_left(3) ^ b.rotate_left(4) ^ 0x63;
            sbox[i] = s;
            inv_sbox[s as usize] = i as u8;
        }
        AesTables { sbox, inv_sbox }
    })
}

/// AES block cipher with an expanded key (column-major state like xpdf).
pub struct Aes {
    w: Vec<u32>,
    nr: usize,
}

impl Aes {
    pub fn new_128(key: &[u8]) -> Aes {
        Aes::new(key, 4)
    }
    pub fn new_256(key: &[u8]) -> Aes {
        Aes::new(key, 8)
    }
    fn new(key: &[u8], nk: usize) -> Aes {
        let t = aes_tables();
        let nr = nk + 6;
        let mut w = vec![0u32; 4 * (nr + 1)];
        let kb = |i: usize| key.get(i).copied().unwrap_or(0) as u32;
        for i in 0..nk {
            w[i] = (kb(4 * i) << 24) | (kb(4 * i + 1) << 16) | (kb(4 * i + 2) << 8) | kb(4 * i + 3);
        }
        let sub_word = |x: u32| -> u32 {
            ((t.sbox[(x >> 24) as usize] as u32) << 24)
                | ((t.sbox[((x >> 16) & 0xff) as usize] as u32) << 16)
                | ((t.sbox[((x >> 8) & 0xff) as usize] as u32) << 8)
                | (t.sbox[(x & 0xff) as usize] as u32)
        };
        let mut rcon: u32 = 0x01000000;
        for i in nk..4 * (nr + 1) {
            let mut temp = w[i - 1];
            if i % nk == 0 {
                temp = sub_word(temp.rotate_left(8)) ^ rcon;
                let r = (rcon >> 24) as u8;
                rcon = (gf_mul(r, 2) as u32) << 24;
            } else if nk > 6 && i % nk == 4 {
                temp = sub_word(temp);
            }
            w[i] = w[i - nk] ^ temp;
        }
        Aes { w, nr }
    }

    fn add_round_key(&self, s: &mut [u8; 16], round: usize) {
        for c in 0..4 {
            let w = self.w[round * 4 + c];
            s[4 * c] ^= (w >> 24) as u8;
            s[4 * c + 1] ^= (w >> 16) as u8;
            s[4 * c + 2] ^= (w >> 8) as u8;
            s[4 * c + 3] ^= w as u8;
        }
    }

    /// Encrypt one block in place (state bytes in input order).
    pub fn encrypt_block(&self, s: &mut [u8; 16]) {
        let t = aes_tables();
        self.add_round_key(s, 0);
        for round in 1..=self.nr {
            for b in s.iter_mut() {
                *b = t.sbox[*b as usize];
            }
            shift_rows(s);
            if round != self.nr {
                mix_columns(s);
            }
            self.add_round_key(s, round);
        }
    }

    /// Decrypt one block in place.
    pub fn decrypt_block(&self, s: &mut [u8; 16]) {
        let t = aes_tables();
        self.add_round_key(s, self.nr);
        for round in (0..self.nr).rev() {
            inv_shift_rows(s);
            for b in s.iter_mut() {
                *b = t.inv_sbox[*b as usize];
            }
            self.add_round_key(s, round);
            if round != 0 {
                inv_mix_columns(s);
            }
        }
    }
}

// state layout: byte index 4*c + r (column c, row r), as in FIPS-197
fn shift_rows(s: &mut [u8; 16]) {
    let t = *s;
    for c in 0..4 {
        for r in 0..4 {
            s[4 * c + r] = t[4 * ((c + r) % 4) + r];
        }
    }
}
fn inv_shift_rows(s: &mut [u8; 16]) {
    let t = *s;
    for c in 0..4 {
        for r in 0..4 {
            s[4 * ((c + r) % 4) + r] = t[4 * c + r];
        }
    }
}
fn mix_columns(s: &mut [u8; 16]) {
    for c in 0..4 {
        let a = [s[4 * c], s[4 * c + 1], s[4 * c + 2], s[4 * c + 3]];
        s[4 * c] = gf_mul(a[0], 2) ^ gf_mul(a[1], 3) ^ a[2] ^ a[3];
        s[4 * c + 1] = a[0] ^ gf_mul(a[1], 2) ^ gf_mul(a[2], 3) ^ a[3];
        s[4 * c + 2] = a[0] ^ a[1] ^ gf_mul(a[2], 2) ^ gf_mul(a[3], 3);
        s[4 * c + 3] = gf_mul(a[0], 3) ^ a[1] ^ a[2] ^ gf_mul(a[3], 2);
    }
}
fn inv_mix_columns(s: &mut [u8; 16]) {
    for c in 0..4 {
        let a = [s[4 * c], s[4 * c + 1], s[4 * c + 2], s[4 * c + 3]];
        s[4 * c] = gf_mul(a[0], 14) ^ gf_mul(a[1], 11) ^ gf_mul(a[2], 13) ^ gf_mul(a[3], 9);
        s[4 * c + 1] = gf_mul(a[0], 9) ^ gf_mul(a[1], 14) ^ gf_mul(a[2], 11) ^ gf_mul(a[3], 13);
        s[4 * c + 2] = gf_mul(a[0], 13) ^ gf_mul(a[1], 9) ^ gf_mul(a[2], 14) ^ gf_mul(a[3], 11);
        s[4 * c + 3] = gf_mul(a[0], 11) ^ gf_mul(a[1], 13) ^ gf_mul(a[2], 9) ^ gf_mul(a[3], 14);
    }
}

// ------------------------------------------------------------------------
// MD5 (RFC 1321)
// ------------------------------------------------------------------------

/// `md5(Guchar *msg, int msgLen, Guchar *digest)`.
pub fn md5(msg: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
        14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
        21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];
    let mut a0: u32 = 0x67452301;
    let mut b0: u32 = 0xefcdab89;
    let mut c0: u32 = 0x98badcfe;
    let mut d0: u32 = 0x10325476;
    let mut m = msg.to_vec();
    let bit_len = (msg.len() as u64).wrapping_mul(8);
    m.push(0x80);
    while m.len() % 64 != 56 {
        m.push(0);
    }
    m.extend_from_slice(&bit_len.to_le_bytes());
    for chunk in m.chunks(64) {
        let mut x = [0u32; 16];
        for i in 0..16 {
            x[i] = u32::from_le_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f2 = f.wrapping_add(a).wrapping_add(K[i]).wrapping_add(x[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f2.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..16].copy_from_slice(&d0.to_le_bytes());
    out
}

// ------------------------------------------------------------------------
// SHA-256 / SHA-384 / SHA-512 (FIPS 180-4)
// ------------------------------------------------------------------------

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// `sha256(Guchar *msg, int msgLen, Guchar *hash)`.
pub fn sha256(msg: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut m = msg.to_vec();
    let bit_len = (msg.len() as u64).wrapping_mul(8);
    m.push(0x80);
    while m.len() % 64 != 56 {
        m.push(0);
    }
    m.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in m.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(SHA256_K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v[7] = v[6];
            v[6] = v[5];
            v[5] = v[4];
            v[4] = v[3].wrapping_add(t1);
            v[3] = v[2];
            v[2] = v[1];
            v[1] = v[0];
            v[0] = t1.wrapping_add(t2);
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    let mut out = [0u8; 32];
    for i in 0..8 {
        out[4 * i..4 * i + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

const SHA512_K: [u64; 80] = [
    0x428a2f98d728ae22, 0x7137449123ef65cd, 0xb5c0fbcfec4d3b2f, 0xe9b5dba58189dbbc,
    0x3956c25bf348b538, 0x59f111f1b605d019, 0x923f82a4af194f9b, 0xab1c5ed5da6d8118,
    0xd807aa98a3030242, 0x12835b0145706fbe, 0x243185be4ee4b28c, 0x550c7dc3d5ffb4e2,
    0x72be5d74f27b896f, 0x80deb1fe3b1696b1, 0x9bdc06a725c71235, 0xc19bf174cf692694,
    0xe49b69c19ef14ad2, 0xefbe4786384f25e3, 0x0fc19dc68b8cd5b5, 0x240ca1cc77ac9c65,
    0x2de92c6f592b0275, 0x4a7484aa6ea6e483, 0x5cb0a9dcbd41fbd4, 0x76f988da831153b5,
    0x983e5152ee66dfab, 0xa831c66d2db43210, 0xb00327c898fb213f, 0xbf597fc7beef0ee4,
    0xc6e00bf33da88fc2, 0xd5a79147930aa725, 0x06ca6351e003826f, 0x142929670a0e6e70,
    0x27b70a8546d22ffc, 0x2e1b21385c26c926, 0x4d2c6dfc5ac42aed, 0x53380d139d95b3df,
    0x650a73548baf63de, 0x766a0abb3c77b2a8, 0x81c2c92e47edaee6, 0x92722c851482353b,
    0xa2bfe8a14cf10364, 0xa81a664bbc423001, 0xc24b8b70d0f89791, 0xc76c51a30654be30,
    0xd192e819d6ef5218, 0xd69906245565a910, 0xf40e35855771202a, 0x106aa07032bbd1b8,
    0x19a4c116b8d2d0c8, 0x1e376c085141ab53, 0x2748774cdf8eeb99, 0x34b0bcb5e19b48a8,
    0x391c0cb3c5c95a63, 0x4ed8aa4ae3418acb, 0x5b9cca4f7763e373, 0x682e6ff3d6b2b8a3,
    0x748f82ee5defb2fc, 0x78a5636f43172f60, 0x84c87814a1f0ab72, 0x8cc702081a6439ec,
    0x90befffa23631e28, 0xa4506cebde82bde9, 0xbef9a3f7b2c67915, 0xc67178f2e372532b,
    0xca273eceea26619c, 0xd186b8c721c0c207, 0xeada7dd6cde0eb1e, 0xf57d4f7fee6ed178,
    0x06f067aa72176fba, 0x0a637dc5a2c898a6, 0x113f9804bef90dae, 0x1b710b35131c471b,
    0x28db77f523047d84, 0x32caab7b40c72493, 0x3c9ebe0a15c9bebc, 0x431d67c49c100d4c,
    0x4cc5d4becb3e42b6, 0x597f299cfc657e2a, 0x5fcb6fab3ad6faec, 0x6c44198c4a475817,
];

fn sha512_core(msg: &[u8], init: [u64; 8]) -> [u64; 8] {
    let mut h = init;
    let mut m = msg.to_vec();
    let bit_len = (msg.len() as u128).wrapping_mul(8);
    m.push(0x80);
    while m.len() % 128 != 112 {
        m.push(0);
    }
    m.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in m.chunks(128) {
        let mut w = [0u64; 80];
        for i in 0..16 {
            let mut b = [0u8; 8];
            b.copy_from_slice(&chunk[8 * i..8 * i + 8]);
            w[i] = u64::from_be_bytes(b);
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..80 {
            let s1 = v[4].rotate_right(14) ^ v[4].rotate_right(18) ^ v[4].rotate_right(41);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(SHA512_K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(28) ^ v[0].rotate_right(34) ^ v[0].rotate_right(39);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v[7] = v[6];
            v[6] = v[5];
            v[5] = v[4];
            v[4] = v[3].wrapping_add(t1);
            v[3] = v[2];
            v[2] = v[1];
            v[1] = v[0];
            v[0] = t1.wrapping_add(t2);
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    h
}

/// `sha512(Guchar *msg, int msgLen, Guchar *hash)`.
pub fn sha512(msg: &[u8]) -> [u8; 64] {
    let h = sha512_core(
        msg,
        [
            0x6a09e667f3bcc908, 0xbb67ae8584caa73b, 0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1,
            0x510e527fade682d1, 0x9b05688c2b3e6c1f, 0x1f83d9abfb41bd6b, 0x5be0cd19137e2179,
        ],
    );
    let mut out = [0u8; 64];
    for i in 0..8 {
        out[8 * i..8 * i + 8].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

/// `sha384(Guchar *msg, int msgLen, Guchar *hash)`.
pub fn sha384(msg: &[u8]) -> [u8; 48] {
    let h = sha512_core(
        msg,
        [
            0xcbbb9d5dc1059ed8, 0x629a292a367cd507, 0x9159015a3070dd17, 0x152fecd8f70e5939,
            0x67332667ffc00b31, 0x8eb44a8768581511, 0xdb0c2e0d64f98fa7, 0x47b5481dbefa4fa4,
        ],
    );
    let mut out = [0u8; 48];
    for i in 0..6 {
        out[8 * i..8 * i + 8].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{:02x}", x)).collect()
    }

    #[test]
    fn md5_vectors() {
        assert_eq!(hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            hex(&md5(b"12345678901234567890123456789012345678901234567890123456789012345678901234567890")),
            "57edf4a22be3c955ac49da2e2107b67a"
        );
    }

    #[test]
    fn sha_vectors() {
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            hex(&sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            hex(&sha384(b"abc")),
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"
        );
        assert_eq!(
            hex(&sha512(b"abc")),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
    }

    #[test]
    fn aes_fips197() {
        // FIPS-197 C.1 and C.3
        let pt: [u8; 16] = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
        ];
        let k128: Vec<u8> = (0u8..16).collect();
        let a = Aes::new_128(&k128);
        let mut b = pt;
        a.encrypt_block(&mut b);
        assert_eq!(hex(&b), "69c4e0d86a7b0430d8cdb78070b4c55a");
        a.decrypt_block(&mut b);
        assert_eq!(b, pt);
        let k256: Vec<u8> = (0u8..32).collect();
        let a = Aes::new_256(&k256);
        let mut b = pt;
        a.encrypt_block(&mut b);
        assert_eq!(hex(&b), "8ea2b7ca516745bfeafc49904b496089");
        a.decrypt_block(&mut b);
        assert_eq!(b, pt);
    }

    #[test]
    fn rc4_vector() {
        // "Key" / "Plaintext" -> BBF316E8D940AF0AD3
        let mut r = Rc4::new(b"Key");
        let out: Vec<u8> = b"Plaintext".iter().map(|&c| r.decrypt_byte(c)).collect();
        assert_eq!(hex(&out), "bbf316e8d940af0ad3");
    }

    #[test]
    fn aes_cbc_padding_like_xpdf() {
        // encrypt "hello" with PKCS#7 padding, IV prefix, then decrypt via
        // the DecryptStream emulation
        let fk = FileKey { key: [7u8; 32], key_length: 16, algo: CryptAlgorithm::AES };
        let (ok, _) = make_obj_key(&fk, 12, 0);
        let aes = Aes::new_128(&ok[..16]);
        let iv = [3u8; 16];
        let mut blk = [11u8; 16];
        blk[..5].copy_from_slice(b"hello");
        for i in 0..16 {
            blk[i] ^= iv[i];
        }
        aes.encrypt_block(&mut blk);
        let mut data = iv.to_vec();
        data.extend_from_slice(&blk);
        assert_eq!(decrypt_bytes(&data, &fk, 12, 0), b"hello");
        // trailing partial block: the last full block is not "last", so
        // no padding is removed, and the partial block is dropped
        let mut d2 = data.clone();
        d2.extend_from_slice(&[1, 2, 3]);
        let out = decrypt_bytes(&d2, &fk, 12, 0);
        assert_eq!(out.len(), 16);
        assert_eq!(&out[..5], b"hello");
        // too short -> empty
        assert!(decrypt_bytes(&data[..20], &fk, 12, 0).is_empty());
    }
}
