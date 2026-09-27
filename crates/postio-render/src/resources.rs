//! The closed set of resources one render may use (spec FR-003,
//! data-model.md "Resource table"): the message's own parts and the images
//! it embeds. Built by the caller before the render; the engine is handed
//! this as its only `NetProvider`, so a lookup outside it resolves to
//! nothing -- counted, never fetched.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};

use crate::Scope;

/// The scheme the composed document's `@font-face` rules use for the
/// bundled faces (`postio_ui::reader::document::FONT_SCHEME`).
pub const FONT_SCHEME: &str = "postio-font";

/// The resource table.
#[derive(Debug, Default)]
pub struct Resources {
    /// `(scope, content-id)` → the part's bytes. An unscoped document (one
    /// message, composed without a scope) keys its parts by `None`.
    parts: Mutex<HashMap<(Option<Scope>, String), Bytes>>,
    /// `postio-font:<name>` → a bundled face (ADR 0023). The URL scheme is
    /// fixed; the faces are the caller's, because the composed document's
    /// `@font-face` rules name them.
    fonts: Mutex<HashMap<String, Bytes>>,
    resolved: AtomicU32,
    unresolved: AtomicU32,
}

impl Resources {
    /// An empty table.
    pub fn new() -> Resources {
        Resources::default()
    }

    /// Add one of a message's own parts, by its `Content-ID` (with or
    /// without the angle brackets).
    pub fn insert_part(&self, scope: Option<&str>, content_id: &str, bytes: Vec<u8>) {
        let id = content_id
            .trim()
            .trim_start_matches('<')
            .trim_end_matches('>');
        self.parts
            .lock()
            .expect("the resource table is never poisoned")
            .insert(
                (scope.map(str::to_owned), id.to_owned()),
                Bytes::from(bytes),
            );
    }

    /// Add a bundled face, answered for `postio-font:<name>`.
    pub fn insert_font(&self, name: &str, bytes: &'static [u8]) {
        self.fonts
            .lock()
            .expect("the resource table is never poisoned")
            .insert(name.to_owned(), Bytes::from_static(bytes));
    }

    /// Lookups answered, and not, since the table was built.
    pub(crate) fn counts(&self) -> (u32, u32) {
        (
            self.resolved.load(Ordering::Relaxed),
            self.unresolved.load(Ordering::Relaxed),
        )
    }

    fn lookup(&self, url: &str) -> Option<Bytes> {
        if let Some(rest) = url
            .strip_prefix(postio_body::CID_SCHEME)
            .and_then(|rest| rest.strip_prefix(':'))
        {
            // `postio-cid:<scope>/<id>`, or `postio-cid:<id>` unscoped. The
            // encoded id never holds a `/` (postio-body's `part_uri`), and a
            // part of one scope is never returned to another (FR-004).
            let (scope, id) = match rest.split_once('/') {
                Some((scope, id)) => (Some(scope.to_owned()), id),
                None => (None, rest),
            };
            let id = postio_body::sanitize::percent_decode(id);
            return self
                .parts
                .lock()
                .expect("the resource table is never poisoned")
                .get(&(scope, id))
                .cloned();
        }
        if let Some(name) = url.strip_prefix(FONT_SCHEME) {
            return self
                .fonts
                .lock()
                .expect("the resource table is never poisoned")
                .get(name.trim_start_matches([':', '/']))
                .cloned();
        }
        // An image the message embeds in itself: decoded here, never
        // fetched. Any other `data:` was refused by the sanitizer.
        let payload = url.strip_prefix("data:image/")?.split_once(";base64,")?.1;
        let payload: String = payload.chars().filter(|c| !c.is_whitespace()).collect();
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, payload)
            .ok()
            .map(Bytes::from)
    }
}

impl NetProvider for Resources {
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url.as_str().to_owned();
        match self.lookup(&url) {
            Some(bytes) => {
                self.resolved.fetch_add(1, Ordering::Relaxed);
                handler.bytes(url, bytes);
            }
            None => {
                self.unresolved.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}
