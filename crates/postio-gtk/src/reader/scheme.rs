//! The `postio-cid:` custom scheme: how an inline image reaches the reader.
//!
//! `sanitize.rs` rewrites every `cid:` reference in a message body to this
//! scheme before the markup ever reaches the `WebView`; this module is what
//! answers those requests. It never touches the network — inline parts are
//! local blobs by the time a message has a renderable body at all — so a
//! request either resolves from local bytes or fails with "not found", never
//! by falling through to somewhere on the internet with the same name.
//!
//! The corpus fixture `inline-image-cid` carries a `cid:` reference with no
//! matching part on purpose. [`respond`] answering that with
//! [`gio::IOErrorEnum::NotFound`] — a broken-image icon in the pane, not a
//! panic or a silent stall — is what that fixture is for.

use std::rc::Rc;

use webkit6::{URISchemeRequest, WebContext};

use postio_body::sanitize::{CID_SCHEME, percent_decode};

// `BlobSource` now lives in `postio_ui::reader::parts`: what a `Content-ID`
// may resolve to is a security property both frontends have to share, not a
// GTK detail (#608). Re-exported so every path here still resolves.
pub use postio_ui::reader::parts::BlobSource;
// The font table is shared for the same reason, one ADR later: what a font
// URL may resolve to is a security property, not a GTK detail (ADR 0023).
use postio_ui::reader::document::{FONT_MIME, FONT_SCHEME, font_bytes};

/// Register [`CID_SCHEME`] on `context`, resolving every request through
/// `source`.
///
/// `WebKitWebContext` offers no way to unregister a scheme, so this is meant
/// to be called once per context with a `source` that stays valid for the
/// context's whole life — [`super::view::Reader`] hands it a handle onto
/// whichever message is currently open, not the message itself.
pub fn register(context: &WebContext, source: Rc<dyn BlobSource>) {
    context.register_uri_scheme(CID_SCHEME, move |request| {
        respond(request, source.as_ref());
    });
    register_fonts(context);
}

/// Register [`FONT_SCHEME`] on `context`, serving Postio's own typefaces
/// (ADR 0023).
///
/// Simpler than the `postio-cid:` case rather than harder: the face table is
/// static for the process' life, so there is no per-message handle to keep
/// valid and nothing that would need unregistering — just as well, since
/// `WebKitWebContext` offers no way to.
///
/// The document used to carry these as ~1.21 MB of base64 re-fed to the
/// engine on every render (#768). Served, the engine fetches only the faces
/// the page actually draws with, and may cache them across documents.
fn register_fonts(context: &WebContext) {
    context.register_uri_scheme(FONT_SCHEME, respond_with_font);
}

fn respond_with_font(request: &URISchemeRequest) {
    let name = request
        .uri()
        .map(|uri| face_name_from_uri(&uri))
        .unwrap_or_default();

    match font_bytes(&name) {
        Some(bytes) => {
            let length = bytes.len() as i64;
            // `&'static [u8]` compiled into the binary, so the stream borrows
            // rather than copies: no read, no file, nothing to fail partway.
            let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_static(bytes));
            request.finish(&stream, length, Some(FONT_MIME));
        }
        None => {
            // A name that is not one of the eight vendored faces. Not found,
            // exactly as a `cid:` with no matching part is — a scheme that
            // fell through to somewhere would be a scheme that can be aimed.
            let mut error = glib::Error::new(gio::IOErrorEnum::NotFound, "no such font face");
            request.finish_error(&mut error);
        }
    }
}

fn face_name_from_uri(uri: &str) -> String {
    let rest = uri
        .strip_prefix(FONT_SCHEME)
        .and_then(|rest| rest.strip_prefix(':'))
        .unwrap_or(uri);
    // A face is named, never pathed (ADR 0023). Leading slashes are trimmed
    // rather than rejected here because [`font_bytes`] is the authority on
    // what resolves, and it answers only for names in the table.
    percent_decode(rest.trim_start_matches('/'))
}

fn respond(request: &URISchemeRequest, source: &dyn BlobSource) {
    let content_id = request
        .uri()
        .map(|uri| content_id_from_uri(&uri))
        .unwrap_or_default();

    match source.resolve(&content_id) {
        Some((bytes, mime_type)) => {
            let length = bytes.len() as i64;
            let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(bytes));
            request.finish(&stream, length, Some(&mime_type));
        }
        None => {
            let mut error = glib::Error::new(gio::IOErrorEnum::NotFound, "no such inline part");
            request.finish_error(&mut error);
        }
    }
}

fn content_id_from_uri(uri: &str) -> String {
    let rest = uri
        .strip_prefix(CID_SCHEME)
        .and_then(|rest| rest.strip_prefix(':'))
        .unwrap_or(uri);
    percent_decode(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_uri_prefix_is_stripped_and_decoded() {
        assert_eq!(
            content_id_from_uri("postio-cid:reader-left.44b1%40example.com"),
            "reader-left.44b1@example.com"
        );
    }

    #[test]
    fn a_uri_missing_the_scheme_prefix_is_decoded_as_is() {
        // Defensive: WebKit always hands us our own scheme's URIs, but a
        // malformed one should not panic.
        assert_eq!(content_id_from_uri("not-our-scheme"), "not-our-scheme");
    }

    #[test]
    fn a_font_uri_resolves_to_the_face_it_names() {
        assert_eq!(
            face_name_from_uri("postio-font:Barlow-Regular.ttf"),
            "Barlow-Regular.ttf"
        );
        assert!(font_bytes(&face_name_from_uri("postio-font:Barlow-Regular.ttf")).is_some());
        // WebKit may hand the authority form back; a face is named, not
        // pathed, so the slashes are not part of the name.
        assert_eq!(
            face_name_from_uri("postio-font://IBMPlexMono-Medium.ttf"),
            "IBMPlexMono-Medium.ttf"
        );
    }

    #[test]
    fn a_font_uri_naming_anything_else_resolves_to_nothing() {
        // The table is the whole of what a font URL may reach (ADR 0023), so
        // the handler has no path to traverse and nothing to fall through to.
        for uri in [
            "postio-font:../../etc/passwd",
            "postio-font:/etc/passwd",
            "postio-font:Barlow-Regular.ttf.exe",
            "postio-font:",
        ] {
            assert!(
                font_bytes(&face_name_from_uri(uri)).is_none(),
                "{uri} resolved to something"
            );
        }
    }

    #[test]
    fn a_closure_implements_blob_source() {
        let source: &dyn BlobSource =
            &|id: &str| (id == "known").then(|| (vec![1, 2, 3], "image/png".to_string()));
        assert_eq!(
            source.resolve("known"),
            Some((vec![1, 2, 3], "image/png".to_string()))
        );
        assert_eq!(source.resolve("missing"), None);
    }
}
