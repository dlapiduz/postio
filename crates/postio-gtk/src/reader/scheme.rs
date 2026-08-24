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

/// Resolves a `Content-ID` to its bytes and MIME type.
///
/// A `Content-ID` is passed exactly as `postio_body::sanitize::percent_decode` recovered
/// it: without the `cid:` prefix and without the angle brackets some senders
/// wrap it in (`sanitize_body` already strips those before encoding).
///
/// Synchronous and local by design — see the module docs — so an
/// implementation is a blob-store read or a lookup into whatever the caller
/// already has in memory for the open message, never a call that blocks on
/// I/O the reader would have to await.
pub trait BlobSource {
    /// The part's bytes and MIME type, or `None` if no part carries this id.
    fn resolve(&self, content_id: &str) -> Option<(Vec<u8>, String)>;
}

impl<F: Fn(&str) -> Option<(Vec<u8>, String)>> BlobSource for F {
    fn resolve(&self, content_id: &str) -> Option<(Vec<u8>, String)> {
        self(content_id)
    }
}

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
