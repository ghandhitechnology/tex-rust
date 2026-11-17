//! The process environment as seen by the C runtime (`getenv`/`putenv`),
//! plus kpathsea's `xputenv` (xputenv.c).
//!
//! The MSVC CRT keeps its own narrow copy of the environment, built at
//! start-up and converted to the ANSI code page; lookups are
//! case-insensitive; `putenv("VAR=")` removes VAR.  We keep a snapshot of
//! the environment taken when the `Kpse` instance is created, plus an
//! overlay with every variable kpathsea (or the client, through
//! [`crate::Kpse::xputenv`]) has set.  The overlay is passed on to child
//! processes (mktex scripts), as the real `putenv` would.

use std::collections::HashMap;

use crate::sys;

#[derive(Default)]
pub(crate) struct Env {
    /// Upper-cased name -> value.
    base: Option<HashMap<Vec<u8>, Vec<u8>>>,
    /// Upper-cased name -> (name as given, value or None if removed).
    overlay: HashMap<Vec<u8>, (Vec<u8>, Option<Vec<u8>>)>,
    acp: u32,
}

impl Env {
    pub(crate) fn new(acp: u32) -> Self {
        Env { base: None, overlay: HashMap::new(), acp }
    }

    fn base(&mut self) -> &HashMap<Vec<u8>, Vec<u8>> {
        if self.base.is_none() {
            let mut m = HashMap::new();
            for (k, v) in sys::env_snapshot(self.acp) {
                if k.is_empty() || k[0] == b'=' {
                    continue;
                }
                m.entry(k.to_ascii_uppercase()).or_insert(v);
            }
            self.base = Some(m);
        }
        self.base.as_ref().unwrap()
    }

    /// `getenv`.
    pub(crate) fn get(&mut self, name: &[u8]) -> Option<Vec<u8>> {
        if name.is_empty() || name.contains(&b'=') {
            return None;
        }
        let key = name.to_ascii_uppercase();
        if let Some((_, v)) = self.overlay.get(&key) {
            return v.clone();
        }
        self.base().get(&key).cloned()
    }

    /// Whether `getenv` would return a non-NULL value.
    pub(crate) fn is_set(&mut self, name: &[u8]) -> bool {
        self.get(name).is_some()
    }

    /// `putenv("VAR=VALUE")` as done by the MSVC CRT: an empty value
    /// removes the variable.
    pub(crate) fn put(&mut self, name: &[u8], value: &[u8]) {
        let key = name.to_ascii_uppercase();
        let v = if value.is_empty() { None } else { Some(value.to_vec()) };
        self.overlay.insert(key, (name.to_vec(), v));
    }

    /// All variables changed through `put`, for passing to children.
    pub(crate) fn changes(&self) -> impl Iterator<Item = (&[u8], Option<&[u8]>)> {
        self.overlay.values().map(|(k, v)| (k.as_slice(), v.as_deref()))
    }
}
