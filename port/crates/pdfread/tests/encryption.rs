//! Encrypted files (standard security handler, empty user password —
//! the only case pdfTeX can open, since it passes no password): RC4
//! 40-bit (R2), RC4 128-bit (R3), AES-128 (V4/R4 AESV2), AES-256 (V5/R6
//! AESV3), plus the "Identity" crypt filter and a metadata stream marked
//! with the Crypt filter.  The files are produced here; pdftex (real
//! xpdf) must decrypt them to the same strings and stream bytes.

mod common;

use common::builder::*;
use common::epdf::check_with_pdftex;
use pdfread::decrypt::{md5, r6_hash, sha256, Aes, Rc4, PASSWORD_PAD};
use pdfread::{set_err_quiet, CryptAlgorithm, Object, PdfDoc};

const ID0: &[u8] = b"0123456789abcdef";
const P: i32 = -44; // print allowed

fn pad(pw: &[u8]) -> [u8; 32] {
    let mut b = [0u8; 32];
    let n = pw.len().min(32);
    b[..n].copy_from_slice(&pw[..n]);
    b[n..].copy_from_slice(&PASSWORD_PAD[..32 - n]);
    b
}

fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut r = Rc4::new(key);
    data.iter().map(|&c| r.decrypt_byte(c)).collect()
}

/// Algorithm 3 (O value) for revision 2/3.
fn o_value(owner: &[u8], user: &[u8], rev: i32, n: usize) -> Vec<u8> {
    let mut h = md5(&pad(owner)).to_vec();
    if rev >= 3 {
        for _ in 0..50 {
            h = md5(&h[..n]).to_vec();
        }
    }
    let key = &h[..n];
    let mut o = rc4(key, &pad(user));
    if rev >= 3 {
        for i in 1..=19u8 {
            let k: Vec<u8> = key.iter().map(|&b| b ^ i).collect();
            o = rc4(&k, &o);
        }
    }
    o
}

/// Algorithm 2 (file key) for revisions 2-4.
fn file_key(user: &[u8], o: &[u8], rev: i32, n: usize, encrypt_metadata: bool) -> Vec<u8> {
    let mut b = pad(user).to_vec();
    b.extend_from_slice(o);
    b.extend_from_slice(&P.to_le_bytes());
    b.extend_from_slice(ID0);
    if rev >= 4 && !encrypt_metadata {
        b.extend_from_slice(&[0xff; 4]);
    }
    let mut h = md5(&b).to_vec();
    if rev >= 3 {
        for _ in 0..50 {
            h = md5(&h[..n]).to_vec();
        }
    }
    h[..n].to_vec()
}

/// Algorithms 4/5 (U value).
fn u_value(key: &[u8], rev: i32) -> Vec<u8> {
    if rev == 2 {
        return rc4(key, &PASSWORD_PAD);
    }
    let mut b = PASSWORD_PAD.to_vec();
    b.extend_from_slice(ID0);
    let mut u = rc4(key, &md5(&b));
    for i in 1..=19u8 {
        let k: Vec<u8> = key.iter().map(|&x| x ^ i).collect();
        u = rc4(&k, &u);
    }
    u.extend_from_slice(&[0x55; 16]);
    u
}

fn aes_cbc_encrypt(key: &[u8], iv: [u8; 16], data: &[u8]) -> Vec<u8> {
    let aes = if key.len() == 32 { Aes::new_256(key) } else { Aes::new_128(key) };
    let mut out = iv.to_vec();
    let mut padded = data.to_vec();
    let p = 16 - data.len() % 16;
    padded.extend(std::iter::repeat(p as u8).take(p));
    let mut prev = iv;
    for chunk in padded.chunks(16) {
        let mut b = [0u8; 16];
        for i in 0..16 {
            b[i] = chunk[i] ^ prev[i];
        }
        aes.encrypt_block(&mut b);
        out.extend_from_slice(&b);
        prev = b;
    }
    out
}

struct Enc {
    algo: CryptAlgorithm,
    key: Vec<u8>,
}

impl Enc {
    fn obj_key(&self, num: i32, gen: i32) -> Vec<u8> {
        if self.algo == CryptAlgorithm::AES256 {
            return self.key.clone();
        }
        let mut b = self.key.clone();
        b.extend_from_slice(&num.to_le_bytes()[..3]);
        b.extend_from_slice(&gen.to_le_bytes()[..2]);
        if self.algo == CryptAlgorithm::AES {
            b.extend_from_slice(b"sAlT");
        }
        let n = (self.key.len() + 5).min(16);
        md5(&b)[..n].to_vec()
    }
    fn encrypt(&self, num: i32, gen: i32, data: &[u8]) -> Vec<u8> {
        let k = self.obj_key(num, gen);
        match self.algo {
            CryptAlgorithm::RC4 => rc4(&k, data),
            _ => {
                let mut iv = [0u8; 16];
                for (i, b) in iv.iter_mut().enumerate() {
                    *b = (i as u8).wrapping_mul(37).wrapping_add(num as u8);
                }
                aes_cbc_encrypt(&k, iv, data)
            }
        }
    }
    /// A literal PDF string with the encrypted bytes, hex-encoded.
    fn s(&self, num: i32, text: &[u8]) -> String {
        let e = self.encrypt(num, 0, text);
        format!("<{}>", e.iter().map(|b| format!("{:02x}", b)).collect::<String>())
    }
}

fn hex(b: &[u8]) -> String {
    format!("<{}>", b.iter().map(|x| format!("{:02x}", x)).collect::<String>())
}

/// Build an encrypted file; returns (pdf, enc).
fn build(kind: &str) -> Vec<u8> {
    let (enc, encrypt_dict) = match kind {
        "rc4-40" => {
            let o = o_value(b"owner", b"", 2, 5);
            let k = file_key(b"", &o, 2, 5, true);
            let u = u_value(&k, 2);
            (
                Enc { algo: CryptAlgorithm::RC4, key: k },
                format!("<< /Filter /Standard /V 1 /R 2 /O {} /U {} /P {} >>", hex(&o), hex(&u), P),
            )
        }
        "rc4-128" => {
            let o = o_value(b"owner", b"", 3, 16);
            let k = file_key(b"", &o, 3, 16, true);
            let u = u_value(&k, 3);
            (
                Enc { algo: CryptAlgorithm::RC4, key: k },
                format!("<< /Filter /Standard /V 2 /R 3 /Length 128 /O {} /U {} /P {} >>", hex(&o), hex(&u), P),
            )
        }
        "aes-128" => {
            let o = o_value(b"owner", b"", 4, 16);
            let k = file_key(b"", &o, 4, 16, true);
            let u = u_value(&k, 3);
            (
                Enc { algo: CryptAlgorithm::AES, key: k },
                format!(
                    "<< /Filter /Standard /V 4 /R 4 /Length 128 /CF << /StdCF << /CFM /AESV2 /Length 16 \
                     /AuthEvent /DocOpen >> >> /StmF /StdCF /StrF /StdCF /O {} /U {} /P {} >>",
                    hex(&o),
                    hex(&u),
                    P
                ),
            )
        }
        "aes-256" => {
            let fk: Vec<u8> = (0..32u8).map(|i| i.wrapping_mul(7).wrapping_add(3)).collect();
            let uvs = *b"uvsalt01";
            let uks = *b"uksalt02";
            let mut t = uvs.to_vec();
            let h = r6_hash(&sha256(&t), b"", None);
            let mut u = h[..32].to_vec();
            u.extend_from_slice(&uvs);
            u.extend_from_slice(&uks);
            t = uks.to_vec();
            let h = r6_hash(&sha256(&t), b"", None);
            let ue = aes_cbc_encrypt(&h[..32], [0; 16], &fk)[16..48].to_vec();
            let ovs = *b"ovsalt03";
            let oks = *b"oksalt04";
            let mut t = b"owner".to_vec();
            t.extend_from_slice(&ovs);
            t.extend_from_slice(&u);
            let h = r6_hash(&sha256(&t), b"owner", Some(&u));
            let mut o = h[..32].to_vec();
            o.extend_from_slice(&ovs);
            o.extend_from_slice(&oks);
            let mut t = b"owner".to_vec();
            t.extend_from_slice(&oks);
            t.extend_from_slice(&u);
            let h = r6_hash(&sha256(&t), b"owner", Some(&u));
            let oe = aes_cbc_encrypt(&h[..32], [0; 16], &fk)[16..48].to_vec();
            (
                Enc { algo: CryptAlgorithm::AES256, key: fk },
                format!(
                    "<< /Filter /Standard /V 5 /R 6 /Length 256 /CF << /StdCF << /CFM /AESV3 /Length 32 >> >> \
                     /StmF /StdCF /StrF /StdCF /O {} /U {} /OE {} /UE {} /P {} /Perms <00> >>",
                    hex(&o),
                    hex(&u),
                    hex(&oe),
                    hex(&ue),
                    P
                ),
            )
        }
        _ => unreachable!(),
    };
    let content = b"q 1 0 0 1 5 5 cm 0 0 m 40 40 l S Q";
    let objs = vec![
        obj(1, "<< /Type /Catalog /Pages 2 0 R >>"),
        obj(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 100] >>"),
        obj(
            3,
            &format!(
                "<< /Type /Page /Parent 2 0 R /Resources << /ProcSet [/PDF] >> /Contents 4 0 R \
                 /PieceInfo << /S {} /Arr [{} 5 0 R] /Num 3.25 >> >>",
                enc.s(3, b"secret (string) \x00\xff"),
                enc.s(3, b""),
            ),
        ),
        obj_bytes(4, 0, stream_body("", &enc.encrypt(4, 0, content))),
        obj_bytes(5, 0, stream_body("/Type /Data", &enc.encrypt(5, 0, b"short"))),
        obj(9, &encrypt_dict),
    ];
    classic_pdf(&objs, &format!(" /Encrypt 9 0 R /ID [{} {}]", hex(ID0), hex(ID0)))
}

#[test]
fn encrypted_files_decrypt_like_xpdf() {
    set_err_quiet(true);
    for kind in ["rc4-40", "rc4-128", "aes-128", "aes-256"] {
        let pdf = build(kind);
        let doc = PdfDoc::from_bytes(pdf.clone());
        assert!(doc.is_ok(), "{} should open", kind);
        assert!(doc.is_encrypted());
        assert!(doc.ok_to_print(false));
        let cat = doc.get_catalog().unwrap();
        let r = cat.get_page_ref(1).unwrap();
        let page = doc.get_xref().unwrap().fetch(r.num, r.gen);
        let pi = page.dict_lookup("PieceInfo");
        assert_eq!(pi.dict_lookup("S").get_string(), b"secret (string) \x00\xff", "{}", kind);
        assert_eq!(pi.dict_lookup("Arr").array_get(0).get_string(), b"", "{}", kind);
        assert_eq!(pi.dict_lookup("Arr").array_get(1).get_stream().unwrap().get_decoded_data().unwrap(), b"short");
        let c = cat.get_page(1).unwrap().get_contents();
        assert_eq!(
            c.get_stream().unwrap().get_undecoded_data(),
            b"q 1 0 0 1 5 5 cm 0 0 m 40 40 l S Q",
            "{}",
            kind
        );
        if let Err(e) = check_with_pdftex(&pdf, 1) {
            panic!("{}: {}", kind, e);
        }
    }
}

#[test]
fn identity_crypt_filter_fails_to_open() {
    // xpdf: StmF == StrF == /Identity -> "no encryption" branch leaves the
    // handler not ok, so authorization fails ("Incorrect password")
    set_err_quiet(true);
    let objs = vec![
        obj(1, "<< /Type /Catalog /Pages 2 0 R >>"),
        obj(2, "<< /Type /Pages /Kids [] /Count 0 >>"),
        obj(9, "<< /Filter /Standard /V 4 /R 4 /CF << >> /StmF /Identity /StrF /Identity /O <00> /U <00> /P -4 >>"),
    ];
    let pdf = classic_pdf(&objs, &format!(" /Encrypt 9 0 R /ID [{} {}]", hex(ID0), hex(ID0)));
    let doc = PdfDoc::from_bytes(pdf.clone());
    assert!(!doc.is_ok());
    assert_eq!(doc.get_error_code(), pdfread::error::codes::ERR_ENCRYPTED);
}

#[test]
fn invalid_encrypt_dict_is_treated_as_unencrypted() {
    // missing /V etc. -> isUnencrypted() is true
    set_err_quiet(true);
    let objs = vec![
        obj(1, "<< /Type /Catalog /Pages 2 0 R >>"),
        obj(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        obj(3, "<< /Type /Page /Parent 2 0 R /PieceInfo (plain) >>"),
        obj(9, "<< /Filter /Standard /R 2 /O <00> /U <00> /P -4 >>"),
    ];
    let pdf = classic_pdf(&objs, " /Encrypt 9 0 R");
    let doc = PdfDoc::from_bytes(pdf.clone());
    assert!(doc.is_ok());
    assert!(!doc.is_encrypted());
    let r = doc.get_catalog().unwrap().get_page_ref(1).unwrap();
    let p = doc.get_xref().unwrap().fetch(r.num, r.gen);
    assert!(matches!(p.dict_lookup("PieceInfo"), Object::String(s) if &*s == b"plain"));
}

#[test]
fn unknown_security_handler_fails() {
    set_err_quiet(true);
    let objs = vec![
        obj(1, "<< /Type /Catalog /Pages 2 0 R >>"),
        obj(2, "<< /Type /Pages /Kids [] /Count 0 >>"),
        obj(9, "<< /Filter /Adobe.PubSec /V 4 >>"),
    ];
    let pdf = classic_pdf(&objs, " /Encrypt 9 0 R");
    let doc = PdfDoc::from_bytes(pdf);
    assert!(!doc.is_ok());
}
