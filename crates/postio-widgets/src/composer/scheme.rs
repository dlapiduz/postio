//! The `postio-cid:` scheme: answers the composer `WebView`'s inline-image requests from local
//! blobs, never the network;
//! a request resolves locally or fails "not found".

use std::rc::Rc;

use gtk::glib;
use webkit6::{URISchemeRequest, WebContext};

use postio_body::sanitize::{CID_SCHEME, percent_decode};

// Which `Content-ID` may resolve to what is a security property both frontends share, so
// `BlobSource` lives in `postio_ui` (#608).
pub use postio_ui::reader::parts::BlobSource;
// The font table is shared for the same reason (ADR 0023).
use postio_ui::reader::document::{FONT_MIME, FONT_SCHEME, font_bytes};

/// `WebKitWebContext` cannot unregister a scheme, so call once per context with a `source` valid
/// for the context's life.
pub fn register(context: &WebContext, source: Rc<dyn BlobSource>) {
    context.register_uri_scheme(CID_SCHEME, move |request| {
        respond(request, source.as_ref());
    });
    register_fonts(context);
}

/// Serves the static face table: the engine fetches only the faces a page draws with instead of
/// re-reading ~1.21 MB of
/// base64 on every render (#768).
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
            let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_static(bytes));
            request.finish(&stream, length, Some(FONT_MIME));
        }
        None => {
            // Not found, like a `cid:` with no part: a scheme that fell through could be aimed.
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
    // A face is named, never pathed (ADR 0023); `font_bytes` is the authority on what resolves.
    percent_decode(rest.trim_start_matches('/'))
}

fn respond(request: &URISchemeRequest, source: &dyn BlobSource) {
    let (scope, content_id) = request
        .uri()
        .map(|uri| reference_from_uri(&uri))
        .unwrap_or_default();

    match source.resolve_in(scope.as_deref(), &content_id) {
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

/// `postio-cid:<id>` for a single-message document (message `None`), `postio-cid:<scope>/<id>` for
/// a conversation document.
/// Split on the first `/` only: `percent_encode` escapes `/`, so an id's slash arrives as `%2F`,
/// and splitting elsewhere
/// would let a sender grow their own scope.
fn reference_from_uri(uri: &str) -> (Option<String>, String) {
    let rest = uri
        .strip_prefix(CID_SCHEME)
        .and_then(|rest| rest.strip_prefix(':'))
        .unwrap_or(uri);
    match rest.split_once('/') {
        Some((scope, id)) => (Some(percent_decode(scope)), percent_decode(id)),
        None => (None, percent_decode(rest)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scoped_uri_names_its_message_and_its_part() {
        assert_eq!(
            reference_from_uri("postio-cid:42/logo"),
            (Some("42".to_string()), "logo".to_string())
        );
    }

    #[test]
    fn an_unscoped_uri_is_what_it_always_was() {
        assert_eq!(
            reference_from_uri("postio-cid:reader-left.44b1%40example.com"),
            (None, "reader-left.44b1@example.com".to_string())
        );
    }

    #[test]
    fn a_content_id_cannot_smuggle_a_separator() {
        assert_eq!(
            reference_from_uri("postio-cid:42/a%2Fb"),
            (Some("42".to_string()), "a/b".to_string())
        );
        assert_eq!(
            reference_from_uri("postio-cid:42/7/x"),
            (Some("42".to_string()), "7/x".to_string())
        );
    }

    #[test]
    fn the_uri_prefix_is_stripped_and_decoded() {
        assert_eq!(
            reference_from_uri("postio-cid:reader-left.44b1%40example.com").1,
            "reader-left.44b1@example.com"
        );
    }

    #[test]
    fn a_uri_missing_the_scheme_prefix_is_decoded_as_is() {
        assert_eq!(reference_from_uri("not-our-scheme").1, "not-our-scheme");
    }

    #[test]
    fn a_font_uri_resolves_to_the_face_it_names() {
        assert_eq!(
            face_name_from_uri("postio-font:Barlow-Regular.ttf"),
            "Barlow-Regular.ttf"
        );
        assert!(font_bytes(&face_name_from_uri("postio-font:Barlow-Regular.ttf")).is_some());
        assert_eq!(
            face_name_from_uri("postio-font://IBMPlexMono-Medium.ttf"),
            "IBMPlexMono-Medium.ttf"
        );
    }

    #[test]
    fn a_font_uri_naming_anything_else_resolves_to_nothing() {
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
