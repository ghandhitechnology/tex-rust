//! Port of xpdf `SecurityHandler.h` / `SecurityHandler.cc` (only the
//! `StandardSecurityHandler` exists in xpdf; other `/Filter` values make
//! the document fail to open).

use crate::decrypt::{make_file_key, CryptAlgorithm, MakeFileKeyParams};
use crate::error::{error, ErrorCategory};
use crate::object::Object;
use crate::xref::XRef;

/// Mirrors `class StandardSecurityHandler`.
pub struct StandardSecurityHandler {
    ok: bool,
    file_id: Vec<u8>,
    owner_key: Vec<u8>,
    user_key: Vec<u8>,
    owner_enc: Option<Vec<u8>>,
    user_enc: Option<Vec<u8>>,
    file_key_length: i32,
    enc_version: i32,
    enc_revision: i32,
    enc_algorithm: CryptAlgorithm,
    encrypt_metadata: bool,
    perm_flags: i32,
    file_key: [u8; 32],
    owner_password_ok: bool,
}

/// `SecurityHandler::make(PDFDoc *docA, Object *encryptDictA)`.
pub fn make_security_handler(xref: &XRef, encrypt_dict: &Object) -> Option<StandardSecurityHandler> {
    let filter = encrypt_dict.dict_lookup("Filter");
    if filter.is_name_of("Standard") {
        Some(StandardSecurityHandler::new(xref, encrypt_dict))
    } else if filter.is_name() {
        error(
            ErrorCategory::SyntaxError,
            -1,
            &format!("Couldn't find the '{}' security handler", filter.get_name_str()),
        );
        None
    } else {
        error(ErrorCategory::SyntaxError, -1, "Missing or invalid 'Filter' entry in encryption dictionary");
        None
    }
}

impl StandardSecurityHandler {
    /// `StandardSecurityHandler::StandardSecurityHandler(PDFDoc*, Object*)`.
    pub fn new(xref: &XRef, encrypt_dict: &Object) -> StandardSecurityHandler {
        let mut h = StandardSecurityHandler {
            ok: false,
            file_id: Vec::new(),
            owner_key: Vec::new(),
            user_key: Vec::new(),
            owner_enc: None,
            user_enc: None,
            file_key_length: 0,
            enc_version: -1,
            enc_revision: -1,
            enc_algorithm: CryptAlgorithm::RC4,
            encrypt_metadata: true,
            perm_flags: 0,
            file_key: [0; 32],
            owner_password_ok: false,
        };

        //--- get the main parameters
        let version_obj = encrypt_dict.dict_lookup("V");
        let revision_obj = encrypt_dict.dict_lookup("R");
        let length_obj = encrypt_dict.dict_lookup("Length");
        let owner_key_obj = encrypt_dict.dict_lookup("O");
        let user_key_obj = encrypt_dict.dict_lookup("U");
        let owner_enc_obj = encrypt_dict.dict_lookup("OE");
        let user_enc_obj = encrypt_dict.dict_lookup("UE");
        let perm_obj = encrypt_dict.dict_lookup("P");
        let file_id_obj = xref.get_trailer_dict().dict_lookup("ID");
        if !version_obj.is_int()
            || !revision_obj.is_int()
            || !perm_obj.is_int()
            || !owner_key_obj.is_string()
            || !user_key_obj.is_string()
        {
            error(ErrorCategory::SyntaxError, -1, "Invalid encryption parameters");
            return h;
        }
        h.enc_version = version_obj.get_int();
        h.enc_revision = revision_obj.get_int();
        h.enc_algorithm = CryptAlgorithm::RC4;
        // revision 2 forces a 40-bit key - some buggy PDF generators set
        // the Length value incorrectly
        if h.enc_revision == 2 || !length_obj.is_int() {
            h.file_key_length = 5;
        } else {
            h.file_key_length = length_obj.get_int() / 8;
        }
        h.encrypt_metadata = true;

        //--- check for a crypt filter (which can modify the parameters)
        if (h.enc_version == 4 || h.enc_version == 5)
            && (h.enc_revision == 4 || h.enc_revision == 5 || h.enc_revision == 6)
        {
            let crypt_filters_obj = encrypt_dict.dict_lookup("CF");
            let stream_filter_obj = encrypt_dict.dict_lookup("StmF");
            let string_filter_obj = encrypt_dict.dict_lookup("StrF");
            if crypt_filters_obj.is_dict()
                && stream_filter_obj.is_name()
                && string_filter_obj.is_name()
                && stream_filter_obj.get_name() == string_filter_obj.get_name()
            {
                if stream_filter_obj.is_name_of("Identity") {
                    // no encryption on streams or strings
                    return h;
                }
                let crypt_filter_obj = crypt_filters_obj.dict_lookup(stream_filter_obj.get_name());
                if crypt_filter_obj.is_dict() {
                    let cfm_obj = crypt_filter_obj.dict_lookup("CFM");
                    if cfm_obj.is_name_of("V2") {
                        if let Object::Int(l) = crypt_filter_obj.dict_lookup("Length") {
                            h.file_key_length = l;
                        }
                        h.enc_version = 2;
                        h.enc_revision = 3;
                    } else if cfm_obj.is_name_of("AESV2") {
                        if let Object::Int(l) = crypt_filter_obj.dict_lookup("Length") {
                            h.file_key_length = l;
                        }
                        h.enc_version = 2;
                        h.enc_revision = 3;
                        h.enc_algorithm = CryptAlgorithm::AES;
                    } else if cfm_obj.is_name_of("AESV3") {
                        if let Object::Int(l) = crypt_filter_obj.dict_lookup("Length") {
                            h.file_key_length = l;
                        }
                        h.enc_version = 5;
                        if h.enc_revision != 5 && h.enc_revision != 6 {
                            h.enc_revision = 6;
                        }
                        h.enc_algorithm = CryptAlgorithm::AES256;
                        // The PDF 2.0 spec says Length and CF.Length are both
                        // deprecated.  Acrobat X honors Length and ignores
                        // CF.Length.  I think it's safest to ignore both.
                        h.file_key_length = 32;
                    }
                }
            }
            if let Object::Bool(b) = encrypt_dict.dict_lookup("EncryptMetadata") {
                h.encrypt_metadata = b;
            }
        }

        //--- version-specific parameters
        if h.enc_revision <= 4 {
            if owner_key_obj.get_string().len() != 32 || user_key_obj.get_string().len() != 32 {
                error(ErrorCategory::SyntaxError, -1, "Invalid encryption key length");
                // this is non-fatal -- see below
            }
        } else if h.enc_revision <= 6 {
            // the spec says 48 bytes, but Acrobat pads them out longer
            if owner_key_obj.get_string().len() < 48
                || user_key_obj.get_string().len() < 48
                || !owner_enc_obj.is_string()
                || owner_enc_obj.get_string().len() != 32
                || !user_enc_obj.is_string()
                || user_enc_obj.get_string().len() != 32
            {
                error(ErrorCategory::SyntaxError, -1, "Invalid encryption key length");
                return h;
            }
        }
        h.perm_flags = perm_obj.get_int();
        h.owner_key = owner_key_obj.get_string().to_vec();
        h.user_key = user_key_obj.get_string().to_vec();
        if h.enc_revision <= 4 {
            // Adobe apparently zero-pads the U value (and maybe the O
            // value?) if it's short
            while h.owner_key.len() < 32 {
                h.owner_key.push(0);
            }
            while h.user_key.len() < 32 {
                h.user_key.push(0);
            }
        }
        if h.enc_version >= 1 && h.enc_version <= 2 && h.enc_revision >= 2 && h.enc_revision <= 3 {
            if file_id_obj.is_array() {
                let id0 = file_id_obj.array_get(0);
                h.file_id = if id0.is_string() { id0.get_string().to_vec() } else { Vec::new() };
            } else {
                h.file_id = Vec::new();
            }
            if h.file_key_length > 16 || h.file_key_length <= 0 {
                h.file_key_length = 16;
            }
            h.ok = true;
        } else if h.enc_version == 5 && (h.enc_revision == 5 || h.enc_revision == 6) {
            h.file_id = Vec::new(); // unused for V=R=5
            h.owner_enc = Some(owner_enc_obj.get_string().to_vec());
            h.user_enc = Some(user_enc_obj.get_string().to_vec());
            if h.file_key_length > 32 || h.file_key_length <= 0 {
                h.file_key_length = 32;
            }
            h.ok = true;
        } else {
            error(
                ErrorCategory::Unimplemented,
                -1,
                &format!(
                    "Unsupported version/revision ({}/{}) of Standard security handler",
                    h.enc_version, h.enc_revision
                ),
            );
        }
        h
    }

    /// `isUnencrypted()`: only true when the parameters were invalid
    /// before V/R were read (xpdf then treats the file as unencrypted).
    pub fn is_unencrypted(&self) -> bool {
        self.enc_version == -1 && self.enc_revision == -1
    }

    /// `SecurityHandler::checkEncryption(ownerPassword, userPassword)`.
    /// pdfTeX passes no passwords and has no `PDFCore` to ask for one.
    pub fn check_encryption(&mut self, owner_password: Option<&[u8]>, user_password: Option<&[u8]>) -> bool {
        let ok = self.authorize(owner_password, user_password);
        if !ok {
            error(ErrorCategory::CommandLine, -1, "Incorrect password");
        }
        ok
    }

    /// `StandardSecurityHandler::authorize(void *authData)`.
    fn authorize(&mut self, owner_password: Option<&[u8]>, user_password: Option<&[u8]>) -> bool {
        if !self.ok {
            return false;
        }
        let p = MakeFileKeyParams {
            enc_version: self.enc_version,
            enc_revision: self.enc_revision,
            key_length: self.file_key_length as usize,
            owner_key: &self.owner_key,
            user_key: &self.user_key,
            owner_enc: self.owner_enc.as_deref(),
            user_enc: self.user_enc.as_deref(),
            permissions: self.perm_flags,
            file_id: &self.file_id,
            owner_password,
            user_password,
            encrypt_metadata: self.encrypt_metadata,
        };
        match make_file_key(&p) {
            Some((k, owner_ok)) => {
                self.file_key = k;
                self.owner_password_ok = owner_ok;
                true
            }
            None => false,
        }
    }

    pub fn get_permission_flags(&self) -> i32 {
        self.perm_flags
    }
    pub fn get_owner_password_ok(&self) -> bool {
        self.owner_password_ok
    }
    pub fn get_file_key(&self) -> &[u8; 32] {
        &self.file_key
    }
    pub fn get_file_key_length(&self) -> i32 {
        self.file_key_length
    }
    pub fn get_enc_version(&self) -> i32 {
        self.enc_version
    }
    pub fn get_enc_revision(&self) -> i32 {
        self.enc_revision
    }
    pub fn get_enc_algorithm(&self) -> CryptAlgorithm {
        self.enc_algorithm
    }
}
