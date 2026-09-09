//! Resolving a `Content-ID` to the bytes it names.
//!
//! Toolkit-free and frontend-free on purpose. The GTK reader resolves `cid:`
//! through a registered URI scheme and a `WKWebView` will resolve it through
//! something else entirely, but *what a `Content-ID` may resolve to* is a
//! security property rather than a rendering detail, so it is stated once
//! here and both frontends are handed the same answer (ADR 0019 Q6, #608).

/// Resolves a `Content-ID` to its bytes and MIME type.
///
/// A `Content-ID` is passed exactly as `postio_body::sanitize::percent_decode`
/// recovered it: without the `cid:` prefix and without the angle brackets some
/// senders wrap it in (`sanitize_body` already strips those before encoding).
///
/// Synchronous and local by design — an implementation is a blob-store read or
/// a lookup into whatever the caller already has in memory for the open
/// message, never a call that blocks on I/O the reader would have to await.
/// That is not only about latency: a `cid:` that could reach the network would
/// be the tracking pixel the reader spends so much effort blocking, arriving
/// through the back door.
pub trait BlobSource {
    /// The part's bytes and MIME type, or `None` if no part carries this id.
    fn resolve(&self, content_id: &str) -> Option<(Vec<u8>, String)>;

    /// The same, for a reference that names *which message* it belongs to.
    ///
    /// A single-message document does not need to: the reader knows which
    /// message is open, so `scope` is `None` and this is [`resolve`]. ADR
    /// 0032's conversation document holds a whole thread, where "whichever is
    /// open" names nothing — two messages may each carry a part called `logo`,
    /// and a sender may reference a `Content-ID` they know belongs to somebody
    /// else's message in the same thread.
    ///
    /// The default ignores the scope, which is right for a source that only
    /// ever holds one message's parts and wrong for one that holds a thread's.
    /// A thread source must override it, and a scope it does not recognise
    /// must resolve to nothing rather than to whatever it has.
    ///
    /// [`resolve`]: Self::resolve
    fn resolve_in(&self, scope: Option<&str>, content_id: &str) -> Option<(Vec<u8>, String)> {
        let _ = scope;
        self.resolve(content_id)
    }
}

impl<F: Fn(&str) -> Option<(Vec<u8>, String)>> BlobSource for F {
    fn resolve(&self, content_id: &str) -> Option<(Vec<u8>, String)> {
        self(content_id)
    }
}
