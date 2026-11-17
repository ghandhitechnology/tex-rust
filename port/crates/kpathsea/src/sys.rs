//! Operating-system layer.
//!
//! The C kpathsea calls a handful of Win32 functions directly (some of them
//! in their ANSI form, which interprets bytes in the system code page).  To
//! get *identical* behaviour we call the very same functions with the very
//! same bytes.  These are plain `kernel32` imports (the Rust standard library
//! links kernel32 anyway), so no C library is involved.
//!
//! On non-Windows hosts a portable fallback built on `std::fs` is provided so
//! that the crate still compiles and mostly works, but parity is only
//! claimed for Windows.

#![allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms)]

#[allow(dead_code)]
pub const CP_ACP: u32 = 0;
pub const CP_UTF8: u32 = 65001;

/// Result of `GetFileAttributes`: `Ok(attributes)` or `Err(GetLastError())`.
pub type AttrResult = Result<u32, u32>;

pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
pub const ERROR_ACCESS_DENIED: u32 = 5;
pub const ERROR_BUFFER_OVERFLOW: u32 = 111;

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ffi::c_void;

    type HANDLE = *mut c_void;
    const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
    const INVALID_FILE_ATTRIBUTES: u32 = 0xFFFF_FFFF;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct FILETIME {
        lo: u32,
        hi: u32,
    }

    #[repr(C)]
    struct WIN32_FIND_DATAW {
        dwFileAttributes: u32,
        ftCreationTime: FILETIME,
        ftLastAccessTime: FILETIME,
        ftLastWriteTime: FILETIME,
        nFileSizeHigh: u32,
        nFileSizeLow: u32,
        dwReserved0: u32,
        dwReserved1: u32,
        cFileName: [u16; 260],
        cAlternateFileName: [u16; 14],
    }

    #[repr(C)]
    struct WIN32_FIND_DATAA {
        dwFileAttributes: u32,
        ftCreationTime: FILETIME,
        ftLastAccessTime: FILETIME,
        ftLastWriteTime: FILETIME,
        nFileSizeHigh: u32,
        nFileSizeLow: u32,
        dwReserved0: u32,
        dwReserved1: u32,
        cFileName: [u8; 260],
        cAlternateFileName: [u8; 14],
    }

    #[repr(C)]
    struct BY_HANDLE_FILE_INFORMATION {
        dwFileAttributes: u32,
        ftCreationTime: FILETIME,
        ftLastAccessTime: FILETIME,
        ftLastWriteTime: FILETIME,
        dwVolumeSerialNumber: u32,
        nFileSizeHigh: u32,
        nFileSizeLow: u32,
        nNumberOfLinks: u32,
        nFileIndexHigh: u32,
        nFileIndexLow: u32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileAttributesW(name: *const u16) -> u32;
        fn GetFileAttributesA(name: *const u8) -> u32;
        fn GetLastError() -> u32;
        fn FindFirstFileW(name: *const u16, data: *mut WIN32_FIND_DATAW) -> HANDLE;
        fn FindNextFileW(h: HANDLE, data: *mut WIN32_FIND_DATAW) -> i32;
        fn FindFirstFileA(name: *const u8, data: *mut WIN32_FIND_DATAA) -> HANDLE;
        fn FindClose(h: HANDLE) -> i32;
        fn MultiByteToWideChar(
            cp: u32,
            flags: u32,
            s: *const u8,
            n: i32,
            w: *mut u16,
            wn: i32,
        ) -> i32;
        fn WideCharToMultiByte(
            cp: u32,
            flags: u32,
            w: *const u16,
            wn: i32,
            s: *mut u8,
            n: i32,
            def: *const u8,
            used: *mut i32,
        ) -> i32;
        fn GetACP() -> u32;
        fn GetOEMCP() -> u32;
        fn AreFileApisANSI() -> i32;
        fn GetFileInformationByHandle(h: HANDLE, info: *mut BY_HANDLE_FILE_INFORMATION) -> i32;
        fn LoadLibraryA(name: *const u8) -> HANDLE;
        fn GetProcAddress(h: HANDLE, name: *const u8) -> *const c_void;
    }

    fn cstr(bytes: &[u8]) -> Vec<u8> {
        // C strings end at the first NUL.
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        let mut v = Vec::with_capacity(end + 1);
        v.extend_from_slice(&bytes[..end]);
        v.push(0);
        v
    }

    /// `MultiByteToWideChar(cp, 0, s, -1, ...)`; result is NUL-terminated.
    pub fn mb_to_wide(cp: u32, s: &[u8]) -> Vec<u16> {
        let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
        let s = &s[..end];
        if s.is_ascii() {
            let mut w: Vec<u16> = s.iter().map(|&b| b as u16).collect();
            w.push(0);
            return w;
        }
        let c = cstr(s);
        unsafe {
            let n = MultiByteToWideChar(cp, 0, c.as_ptr(), -1, std::ptr::null_mut(), 0);
            if n <= 0 {
                return vec![0];
            }
            let mut w = vec![0u16; n as usize];
            MultiByteToWideChar(cp, 0, c.as_ptr(), -1, w.as_mut_ptr(), n);
            w
        }
    }

    /// `WideCharToMultiByte(cp, 0, w, len, ...)`, no terminator in result.
    pub fn wide_to_mb(cp: u32, w: &[u16]) -> Vec<u8> {
        let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
        let w = &w[..end];
        if w.iter().all(|&c| c < 0x80) {
            return w.iter().map(|&c| c as u8).collect();
        }
        if w.is_empty() {
            return Vec::new();
        }
        unsafe {
            let n = WideCharToMultiByte(
                cp,
                0,
                w.as_ptr(),
                w.len() as i32,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
                std::ptr::null_mut(),
            );
            if n <= 0 {
                return Vec::new();
            }
            let mut s = vec![0u8; n as usize];
            WideCharToMultiByte(
                cp,
                0,
                w.as_ptr(),
                w.len() as i32,
                s.as_mut_ptr(),
                n,
                std::ptr::null(),
                std::ptr::null_mut(),
            );
            s
        }
    }

    pub fn file_apis_codepage() -> u32 {
        unsafe {
            if AreFileApisANSI() != 0 {
                GetACP()
            } else {
                GetOEMCP()
            }
        }
    }

    pub fn acp() -> u32 {
        unsafe { GetACP() }
    }

    pub fn get_file_attributes_w(cp: u32, name: &[u8]) -> AttrResult {
        let w = mb_to_wide(cp, name);
        unsafe {
            let a = GetFileAttributesW(w.as_ptr());
            if a == INVALID_FILE_ATTRIBUTES {
                Err(GetLastError())
            } else {
                Ok(a)
            }
        }
    }

    pub fn get_file_attributes_a(name: &[u8]) -> AttrResult {
        let c = cstr(name);
        unsafe {
            let a = GetFileAttributesA(c.as_ptr());
            if a == INVALID_FILE_ATTRIBUTES {
                Err(GetLastError())
            } else {
                Ok(a)
            }
        }
    }

    /// `FindFirstFileA(name)`; returns `cFileName` of the first match.
    pub fn find_first_a(name: &[u8]) -> Option<Vec<u8>> {
        let c = cstr(name);
        unsafe {
            let mut d: WIN32_FIND_DATAA = std::mem::zeroed();
            let h = FindFirstFileA(c.as_ptr(), &mut d);
            if h == INVALID_HANDLE_VALUE {
                return None;
            }
            FindClose(h);
            let end = d.cFileName.iter().position(|&b| b == 0).unwrap_or(260);
            Some(d.cFileName[..end].to_vec())
        }
    }

    /// Enumerate `FindFirstFileW(pattern)` / `FindNextFileW`, converting the
    /// names with code page `cp` (as `get_mbstring_from_wstring` does).
    /// Returns `None` if `FindFirstFileW` fails.
    pub fn list_dir_w(cp: u32, pattern: &[u8], mut f: impl FnMut(Vec<u8>, u32)) -> bool {
        let w = mb_to_wide(cp, pattern);
        unsafe {
            let mut d: WIN32_FIND_DATAW = std::mem::zeroed();
            let h = FindFirstFileW(w.as_ptr(), &mut d);
            if h == INVALID_HANDLE_VALUE {
                return false;
            }
            loop {
                let end = d.cFileName.iter().position(|&c| c == 0).unwrap_or(260);
                let name = &d.cFileName[..end];
                // Names starting with '.' are skipped by the caller anyway;
                // avoid converting them.
                let bytes = if name.first() == Some(&(b'.' as u16)) {
                    vec![b'.']
                } else {
                    wide_to_mb(cp, name)
                };
                f(bytes, d.dwFileAttributes);
                if FindNextFileW(h, &mut d) == 0 {
                    break;
                }
            }
            FindClose(h);
            true
        }
    }

    /// `same_file_p` from file-p.c: compare volume serial and file index.
    pub fn same_file(acp: u32, a: &[u8], b: &[u8]) -> bool {
        use std::os::windows::ffi::OsStringExt;
        use std::os::windows::io::AsRawHandle;
        let open = |n: &[u8]| {
            let mut w = mb_to_wide(acp, n);
            w.pop();
            std::fs::File::open(std::ffi::OsString::from_wide(&w)).ok()
        };
        let (fa, fb) = match (open(a), open(b)) {
            (Some(x), Some(y)) => (x, y),
            _ => return false,
        };
        unsafe {
            let mut ia: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
            let mut ib: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
            if GetFileInformationByHandle(fa.as_raw_handle() as HANDLE, &mut ia) == 0 {
                return false;
            }
            if GetFileInformationByHandle(fb.as_raw_handle() as HANDLE, &mut ib) == 0 {
                return false;
            }
            ia.dwVolumeSerialNumber == ib.dwVolumeSerialNumber
                && ia.nFileIndexHigh == ib.nFileIndexHigh
                && ia.nFileIndexLow == ib.nFileIndexLow
        }
    }

    /// `CharLowerA` applied to each byte 0x80..=0xFF (the `TRANSFORM` macro of
    /// config.h).  user32 is loaded dynamically, and only when a non-ASCII
    /// byte is actually seen, to keep process start-up cheap.
    pub fn charlower_high_table() -> [u8; 128] {
        let mut t = [0u8; 128];
        for (i, v) in t.iter_mut().enumerate() {
            *v = (0x80 + i) as u8;
        }
        unsafe {
            let h = LoadLibraryA(b"user32.dll\0".as_ptr());
            if h.is_null() {
                return t;
            }
            let p = GetProcAddress(h, b"CharLowerA\0".as_ptr());
            if p.is_null() {
                return t;
            }
            let f: extern "system" fn(usize) -> usize = std::mem::transmute(p);
            for (i, v) in t.iter_mut().enumerate() {
                let r = f(0x80 + i);
                *v = (r & 0xff) as u8;
            }
        }
        t
    }

    /// Environment snapshot, values converted to the ANSI code page like the
    /// MSVC CRT's narrow environment.
    pub fn env_snapshot(acp: u32) -> Vec<(Vec<u8>, Vec<u8>)> {
        use std::os::windows::ffi::OsStrExt;
        std::env::vars_os()
            .map(|(k, v)| {
                let kw: Vec<u16> = k.encode_wide().collect();
                let vw: Vec<u16> = v.encode_wide().collect();
                (wide_to_mb(acp, &kw), wide_to_mb(acp, &vw))
            })
            .collect()
    }

    /// Convert an OS string to bytes in code page `cp`.
    pub fn os_to_bytes(cp: u32, s: &std::ffi::OsStr) -> Vec<u8> {
        use std::os::windows::ffi::OsStrExt;
        let w: Vec<u16> = s.encode_wide().collect();
        wide_to_mb(cp, &w)
    }

    /// Convert bytes in code page `cp` to an OS string.
    pub fn bytes_to_os(cp: u32, s: &[u8]) -> std::ffi::OsString {
        use std::os::windows::ffi::OsStringExt;
        let mut w = mb_to_wide(cp, s);
        w.pop();
        std::ffi::OsString::from_wide(&w)
    }

    /// `GetUserNameW`, via advapi32 loaded on demand.
    pub fn user_name(acp: u32) -> Option<Vec<u8>> {
        unsafe {
            let h = LoadLibraryA(b"advapi32.dll\0".as_ptr());
            if h.is_null() {
                return None;
            }
            let p = GetProcAddress(h, b"GetUserNameW\0".as_ptr());
            if p.is_null() {
                return None;
            }
            let f: extern "system" fn(*mut u16, *mut u32) -> i32 = std::mem::transmute(p);
            let mut buf = [0u16; 256];
            let mut n = 256u32;
            if f(buf.as_mut_ptr(), &mut n) == 0 {
                return None;
            }
            Some(wide_to_mb(acp, &buf))
        }
    }
}

#[cfg(not(windows))]
mod imp {
    //! Portable approximation used on non-Windows hosts.
    use super::*;
    use std::ffi::{OsStr, OsString};
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    fn path(name: &[u8]) -> &std::path::Path {
        let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
        std::path::Path::new(OsStr::from_bytes(&name[..end]))
    }
    pub fn mb_to_wide(_cp: u32, s: &[u8]) -> Vec<u16> {
        let mut w: Vec<u16> = String::from_utf8_lossy(s).encode_utf16().collect();
        w.push(0);
        w
    }
    pub fn wide_to_mb(_cp: u32, w: &[u16]) -> Vec<u8> {
        String::from_utf16_lossy(w).into_bytes()
    }
    pub fn file_apis_codepage() -> u32 {
        CP_UTF8
    }
    pub fn acp() -> u32 {
        CP_UTF8
    }
    fn attrs(name: &[u8]) -> AttrResult {
        let p = String::from_utf8_lossy(name).replace('\\', "/");
        match std::fs::metadata(&p) {
            Ok(m) => Ok(if m.is_dir() { FILE_ATTRIBUTE_DIRECTORY } else { 0x80 }),
            Err(e) => Err(match e.kind() {
                std::io::ErrorKind::PermissionDenied => ERROR_ACCESS_DENIED,
                _ => 2,
            }),
        }
    }
    pub fn get_file_attributes_w(_cp: u32, name: &[u8]) -> AttrResult {
        let mut n = name;
        if n.starts_with(b"\\\\?\\") {
            n = &n[4..];
        }
        attrs(n)
    }
    pub fn get_file_attributes_a(name: &[u8]) -> AttrResult {
        attrs(name)
    }
    pub fn find_first_a(name: &[u8]) -> Option<Vec<u8>> {
        let p = String::from_utf8_lossy(name).replace('\\', "/");
        let p = std::path::Path::new(&p);
        if p.exists() {
            p.file_name().map(|f| f.as_bytes().to_vec())
        } else {
            None
        }
    }
    pub fn list_dir_w(_cp: u32, pattern: &[u8], mut f: impl FnMut(Vec<u8>, u32)) -> bool {
        let mut d = pattern.to_vec();
        if d.ends_with(b"/*.*") {
            d.truncate(d.len() - 4);
        }
        let rd = match std::fs::read_dir(path(&d)) {
            Ok(r) => r,
            Err(_) => return false,
        };
        let mut names: Vec<(Vec<u8>, u32)> = Vec::new();
        names.push((b".".to_vec(), FILE_ATTRIBUTE_DIRECTORY));
        names.push((b"..".to_vec(), FILE_ATTRIBUTE_DIRECTORY));
        for e in rd.flatten() {
            let isdir = e.metadata().map(|m| m.is_dir()).unwrap_or(false);
            names.push((
                e.file_name().into_vec(),
                if isdir { FILE_ATTRIBUTE_DIRECTORY } else { 0x80 },
            ));
        }
        for (n, a) in names {
            f(n, a);
        }
        true
    }
    pub fn same_file(_acp: u32, a: &[u8], b: &[u8]) -> bool {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::metadata(path(a)), std::fs::metadata(path(b))) {
            (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
            _ => false,
        }
    }
    pub fn charlower_high_table() -> [u8; 128] {
        let mut t = [0u8; 128];
        for (i, v) in t.iter_mut().enumerate() {
            *v = (0x80 + i) as u8;
        }
        t
    }
    pub fn env_snapshot(_acp: u32) -> Vec<(Vec<u8>, Vec<u8>)> {
        std::env::vars_os()
            .map(|(k, v)| (k.into_vec(), v.into_vec()))
            .collect()
    }
    pub fn os_to_bytes(_cp: u32, s: &OsStr) -> Vec<u8> {
        s.as_bytes().to_vec()
    }
    pub fn bytes_to_os(_cp: u32, s: &[u8]) -> OsString {
        OsString::from_vec(s.to_vec())
    }
    pub fn user_name(_acp: u32) -> Option<Vec<u8>> {
        std::env::var_os("USER").map(|s| s.into_vec())
    }
}

pub use imp::*;
