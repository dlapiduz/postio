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

use gtk::glib;
use gtk::prelude::*;
use webkit6::{URISchemeRequest, WebContext, WebView};

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

/// A reader view, weakly, and the source its `postio-cid` requests answer
/// from.
type Attached = (glib::WeakRef<WebView>, Rc<dyn BlobSource>);

thread_local! {
    /// Each reader view's own blob source, for a context the views share
    /// (#1603). Weak, so a reader that goes takes its entry's usefulness with
    /// it; pruned whenever a view is added.
    static SOURCES: std::cell::RefCell<Vec<Attached>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Register the schemes on a context that several reader views share.
///
/// The fonts are the same for every view. `postio-cid` is not: each reader
/// shows its own message and resolves its inline images through its own
/// source, so the handler asks which view is loading and answers from the
/// source [`attach`] recorded for it. A request from a view with no source
/// -- or one whose view is already gone -- resolves nothing, exactly as a
/// `cid:` with no matching part does.
pub fn register_shared(context: &WebContext) {
    context.register_uri_scheme(CID_SCHEME, |request| {
        let source = request.web_view().and_then(|view| {
            SOURCES.with(|sources| {
                sources
                    .borrow()
                    .iter()
                    .find(|(held, _)| held.upgrade().as_ref() == Some(&view))
                    .map(|(_, source)| Rc::clone(source))
            })
        });
        match source {
            Some(source) => respond(request, source.as_ref()),
            None => respond(request, &|_: &str| None),
        }
    });
    register_fonts(context);
}

/// Say which source answers `view`'s `postio-cid` requests on a shared
/// context. See [`register_shared`].
pub fn attach(view: &WebView, source: Rc<dyn BlobSource>) {
    SOURCES.with(|sources| {
        let mut sources = sources.borrow_mut();
        sources.retain(|(held, _)| held.upgrade().is_some());
        sources.push((view.downgrade(), source));
    });
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

/// How many faces the font scheme has served in this process.
static FONTS_SERVED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// How many vendored faces the engine has fetched over `postio-font` in this
/// process. Process-wide on purpose: readers share one web process (#1603),
/// so a face fetched for one reader is cached for the next, and whether a
/// given view asked says less than whether the engine ever did.
#[doc(hidden)]
pub fn fonts_served() -> usize {
    FONTS_SERVED.load(std::sync::atomic::Ordering::Relaxed)
}

fn respond_with_font(request: &URISchemeRequest) {
    let name = request
        .uri()
        .map(|uri| face_name_from_uri(&uri))
        .unwrap_or_default();

    match font_bytes(&name) {
        Some(bytes) => {
            FONTS_SERVED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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

/// The message a request names, and the `Content-ID` within it.
///
/// A single-message document produces `postio-cid:<id>` and answers `None`
/// for the message: the reader knows which one is open. ADR 0032's
/// conversation document produces `postio-cid:<scope>/<id>`, because one
/// document holds a whole thread and "whichever is open" no longer names one.
///
/// The split is on the **first** literal `/` and only that one. It is
/// unambiguous because `postio_body::sanitize::percent_encode` escapes `/`,
/// so an encoded `Content-ID` never contains a literal one — a slash in the
/// id itself arrives as `%2F` and comes back through the decode. Splitting on
/// the last, or on every, slash would let a sender grow their own scope by
/// adding separators.
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
        // ADR 0032's conversation document: every reference carries the
        // message it belongs to, because one document holds a whole thread.
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
        // `percent_encode` escapes `/`, so a literal one is always Postio's
        // separator and never part of an id. An id that arrived with a slash
        // in it comes back through the escape, not through the split.
        assert_eq!(
            reference_from_uri("postio-cid:42/a%2Fb"),
            (Some("42".to_string()), "a/b".to_string())
        );
        // And the split is on the *first* slash only, so a scope cannot be
        // grown by adding more.
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
        // Defensive: WebKit always hands us our own scheme's URIs, but a
        // malformed one should not panic.
        assert_eq!(reference_from_uri("not-our-scheme").1, "not-our-scheme");
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
