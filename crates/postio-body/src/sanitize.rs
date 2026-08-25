//! Sanitizing a message body before it ever reaches the `WebView`.
//!
//! This is defense in depth, not the primary control — the primary control is
//! that the reader never gives the `WebView` a live path to a remote host in
//! the first place (`postio-cid:` for inline parts, no scheme at all for
//! anything else; see `postio_gtk::reader::view`). A bug here should degrade
//! markup, never to a live tracking pixel.
//!
//! [`ammonia`] does most of the work from its own defaults: `<script>` and
//! `<style>` are removed tag-and-contents, every `on*` handler is dropped, and
//! the `style` attribute is not in its generic allow-list — a sender's CSS
//! never competes with Postio's injected stylesheet
//! (`postio_gtk::reader::view::DOCUMENT_TEMPLATE`).
//! What this module adds on top:
//!
//! * `<iframe>`, `<object>`, `<embed>`, `<svg>`, `<math>` and `<noscript>` are
//!   removed tag-and-contents too. `<noscript>` is the interesting one: with
//!   scripting off, the HTML spec has a browser parse its content as *markup*
//!   rather than inert text, which turns a sender's "if you can't run our
//!   JavaScript, at least load this" fallback into exactly the tracking pixel
//!   disabling JavaScript was supposed to stop.
//! * Every `src` is rewritten: a `cid:` reference becomes the app's own
//!   [`CID_SCHEME`], and — unless the caller passes [`RemoteImages::Allowed`]
//!   — anything pointing at a remote host is dropped outright rather than
//!   left for the network layer to refuse.
//! * `href` is never touched. Links stay live text; `view.rs` intercepts the
//!   click and hands it to the system browser instead of ever navigating the
//!   pane.

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use ammonia::Builder;

/// The scheme the reading pane resolves inline (`cid:`) images through.
///
/// Kept out of `ammonia`'s default URL schemes, so it has to be added
/// explicitly — see [`sanitize_body`].
pub const CID_SCHEME: &str = "postio-cid";

/// Whether remote (`http`/`https`) image references may stay in the markup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RemoteImages {
    /// Strip every remote `src`. The default, and the whole point of
    /// `postio-xxz`: nothing loads before the reader decides to allow it.
    #[default]
    Blocked,
    /// Leave remote `src` values in place — the sender is allow-listed, or
    /// the user asked to see this message's images once.
    Allowed,
}

/// The result of sanitizing one HTML body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sanitized {
    /// The cleaned markup.
    pub html: String,
    /// How many remote (`http`/`https`) references were stripped.
    ///
    /// `postio_gtk::reader::banner::RemoteImageBanner` uses whether this is
    /// nonzero to decide whether a message actually has anything for it to
    /// say — a newsletter with no images should not get a "remote images
    /// blocked" banner it can never have anything to show for. The parts
    /// panel's held-back count (`postio_gtk::parts::PartsPanel::set_held_back`)
    /// uses the number itself.
    pub remote_blocked: u32,
}

/// Sanitize one HTML body for the reading pane.
///
/// `cid:` references become [`CID_SCHEME`] URIs; `postio_gtk::reader::scheme` resolves
/// those against the message's local parts (or answers 404 for a dangling
/// reference — the corpus has one on purpose).
pub fn sanitize_body(html: &str, remote: RemoteImages) -> Sanitized {
    let blocked_count = Arc::new(AtomicU32::new(0));
    let counter = Arc::clone(&blocked_count);

    let mut builder = Builder::default();
    builder
        .rm_tags([
            "link", "base", "meta", "form", "input", "button", "textarea", "select",
        ])
        .add_clean_content_tags(["iframe", "object", "embed", "svg", "math", "noscript"])
        // `cid` has to be allowed on the *input* side too: ammonia checks
        // `url_schemes` against a URL attribute's original value before
        // `attribute_filter` ever runs, so a `cid:` reference would be
        // stripped before this module gets a chance to rewrite it to
        // `CID_SCHEME`. Nothing re-validates the filter's output against
        // `url_schemes` afterward, so `CID_SCHEME` itself does not need to be
        // listed here — it is added anyway, for the reader it is documenting
        // intent to.
        .add_url_schemes(["cid", CID_SCHEME])
        .attribute_filter(move |element, attribute, value| {
            rewrite_attribute(element, attribute, value, remote, &counter)
        });

    Sanitized {
        html: builder.clean(html).to_string(),
        remote_blocked: blocked_count.load(Ordering::Relaxed),
    }
}

fn rewrite_attribute<'u>(
    _element: &str,
    attribute: &str,
    value: &'u str,
    remote: RemoteImages,
    blocked_count: &AtomicU32,
) -> Option<Cow<'u, str>> {
    if attribute != "src" {
        return Some(Cow::Borrowed(value));
    }
    if let Some(id) = value.strip_prefix("cid:") {
        return Some(Cow::Owned(format!(
            "{CID_SCHEME}:{}",
            percent_encode(id.trim().trim_start_matches('<').trim_end_matches('>'))
        )));
    }
    if is_remote(value) && remote == RemoteImages::Blocked {
        blocked_count.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    Some(Cow::Borrowed(value))
}

/// Whether `value` names a remote host rather than something local
/// (`postio-cid:`, `data:`, or a bare fragment/relative path a sender's
/// markup left dangling).
///
/// The scheme is compared case-insensitively, because that is what it means:
/// RFC 3986 §3.1 makes schemes case-insensitive and WebKit resolves them that
/// way. #147's `sanitize_html` fuzz target found this the hard way — a pixel
/// spelled `HTTPS://` was left in the document and reported as nothing held
/// back, so the reader fetched it and the badge said zero. Blocking a sender
/// defeats by holding shift is not blocking.
///
/// ASCII case only, deliberately: a scheme is ASCII by grammar, and
/// `to_lowercase` on attacker-controlled text would allocate for every
/// attribute in every message to fold characters no scheme can contain.
fn is_remote(value: &str) -> bool {
    let value = value.trim();
    ["http://", "https://", "ftp://"]
        .iter()
        // `get`, not a slice: an attribute value is attacker-controlled text
        // and may begin with a multi-byte character, so `value[..8]` panics
        // when byte 8 lands inside one. The fuzz target found that in this
        // very function, 71 executions after it was written.
        .any(|scheme| {
            value
                .get(..scheme.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(scheme))
        })
        // Protocol-relative: no scheme to fold, and whatever the document was
        // loaded over is what it would use.
        || value.starts_with("//")
}

/// Percent-encode a `Content-ID` for use as a URI's opaque part.
///
/// RFC 3986's unreserved set passes through unescaped; everything else —
/// `@`, `%`, whitespace, non-ASCII — is escaped. Content-IDs are usually
/// plain ASCII already; this is just so a stray odd one cannot produce a
/// URI `postio_gtk::reader::scheme` parses differently than it means.
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The inverse of `percent_encode`, for `postio_gtk::reader::scheme` to recover the
/// `Content-ID` a request named.
pub fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(byte) = u8::from_str_radix(&value[i + 1..i + 3], 16)
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_tag_and_its_content_are_removed() {
        let out = sanitize_body(
            "<p>hi</p><script>alert(document.cookie)</script>",
            RemoteImages::Blocked,
        );
        assert_eq!(out.html, "<p>hi</p>");
    }

    #[test]
    fn an_event_handler_attribute_is_stripped() {
        let out = sanitize_body(
            r#"<img src="cid:x" onerror="steal()">"#,
            RemoteImages::Blocked,
        );
        assert!(!out.html.contains("onerror"), "{}", out.html);
    }

    #[test]
    fn a_style_tag_and_its_css_are_removed() {
        let out = sanitize_body(
            "<style>.hero{background:url('https://tracker.example.org/bg.jpg')}</style><p>ok</p>",
            RemoteImages::Blocked,
        );
        assert_eq!(out.html, "<p>ok</p>");
        assert!(!out.html.contains("tracker.example.org"));
        // A stripped <style> is not a stripped *image*: the banner has
        // nothing to report about a message that never referenced one.
        assert_eq!(out.remote_blocked, 0);
    }

    #[test]
    fn an_inline_style_attribute_is_stripped_so_postio_css_always_wins() {
        let out = sanitize_body(r#"<p style="color:red">hi</p>"#, RemoteImages::Blocked);
        assert!(!out.html.contains("style"), "{}", out.html);
    }

    #[test]
    fn noscript_content_is_removed_not_unwrapped() {
        // With scripting off, unwrapped <noscript> content is exactly the
        // beacon disabling JavaScript was meant to stop.
        let out = sanitize_body(
            r#"<noscript><img src="https://tracker.example.org/o.gif"></noscript><p>body</p>"#,
            RemoteImages::Allowed,
        );
        assert_eq!(out.html, "<p>body</p>");
    }

    /// #147, found by the `sanitize_html` fuzz target. URL schemes are
    /// case-insensitive (RFC 3986 §3.1) and WebKit treats them that way, so a
    /// tracking pixel spelled `HTTPS://` was fetched while Postio reported
    /// nothing held back. Blocking that a sender defeats by pressing shift is
    /// not blocking, and this is the promise in PRODUCT.md that nothing leaves
    /// the machine unasked.
    #[test]
    fn a_remote_image_cannot_dodge_blocking_by_changing_case() {
        for spelling in [
            "HTTPS://", "hTtps://", "Http://", "HTTP://", "FTP://", "Ftp://",
        ] {
            let html = format!(r#"<img src="{spelling}tracker.example.org/o.gif">"#);
            let out = sanitize_body(&html, RemoteImages::Blocked);
            assert!(
                !out.html.contains("tracker.example.org"),
                "{spelling} survived: {}",
                out.html
            );
            assert_eq!(out.remote_blocked, 1, "{spelling} was not counted");
        }
    }

    /// The regression the `sanitize_html` target caught 71 executions after
    /// the case-insensitivity fix above was written — in the fix itself, not
    /// in anything older. Comparing a prefix by slicing `value[..8]` panics
    /// when byte 8 lands inside a multi-byte character, and an attribute value
    /// is attacker-controlled text that can start with any character at all.
    #[test]
    fn a_multibyte_attribute_value_does_not_split_a_character() {
        for value in ["日本語です", "é", "\u{fffd}\u{fffd}\u{fffd}", "🐟🐟🐟"] {
            let html = format!(r#"<img src="{value}">"#);
            // The assertion is that this returns at all.
            let out = sanitize_body(&html, RemoteImages::Blocked);
            assert_eq!(out.remote_blocked, 0, "{value} is not remote");
        }
    }

    /// The other half: case-folding must not start blocking things that are
    /// local. `postio-cid:` and `data:` are resolved on this machine.
    #[test]
    fn case_folding_does_not_make_a_local_reference_remote() {
        for local in [
            "postio-cid:x@example.com",
            "DATA:image/png;base64,AA==",
            "#anchor",
        ] {
            let html = format!(r#"<img src="{local}">"#);
            let out = sanitize_body(&html, RemoteImages::Blocked);
            assert_eq!(out.remote_blocked, 0, "{local} was treated as remote");
        }
    }

    #[test]
    fn a_remote_image_is_dropped_by_default_and_reported_as_blocked() {
        let out = sanitize_body(
            r#"<img src="https://tracker.example.org/o.gif" alt="">"#,
            RemoteImages::Blocked,
        );
        assert!(!out.html.contains("tracker.example.org"), "{}", out.html);
        assert!(
            !out.html.contains("src="),
            "the src attribute itself must be gone: {}",
            out.html
        );
        assert_eq!(out.remote_blocked, 1);
    }

    #[test]
    fn each_remote_image_is_counted_not_just_flagged() {
        // The parts panel's held-back count (postio-m2ex) needs a real
        // number, not the bool this used to be — three blocked images must
        // read as 3, not as "some".
        let out = sanitize_body(
            r#"<img src="https://a.example.org/1.gif">
               <img src="https://a.example.org/2.gif">
               <img src="https://a.example.org/3.gif">"#,
            RemoteImages::Blocked,
        );
        assert_eq!(out.remote_blocked, 3);
    }

    #[test]
    fn a_remote_image_survives_when_explicitly_allowed() {
        let out = sanitize_body(
            r#"<img src="https://cdn.example.org/lamp.png" alt="">"#,
            RemoteImages::Allowed,
        );
        assert!(
            out.html.contains("https://cdn.example.org/lamp.png"),
            "{}",
            out.html
        );
        assert_eq!(
            out.remote_blocked, 0,
            "nothing was blocked when images are allowed"
        );
    }

    #[test]
    fn a_message_with_no_remote_reference_reports_nothing_blocked() {
        let out = sanitize_body("<p>plain text only</p>", RemoteImages::Blocked);
        assert_eq!(out.remote_blocked, 0);
    }

    #[test]
    fn a_cid_reference_is_rewritten_to_the_local_scheme() {
        let out = sanitize_body(
            r#"<img src="cid:reader-left.44b1@example.com" alt="">"#,
            RemoteImages::Blocked,
        );
        assert!(
            out.html
                .contains("src=\"postio-cid:reader-left.44b1%40example.com\""),
            "{}",
            out.html
        );
        assert_eq!(out.remote_blocked, 0, "a cid: reference is not remote");
    }

    #[test]
    fn a_link_href_is_never_touched_even_when_images_are_blocked() {
        let out = sanitize_body(
            r#"<a href="https://news.example.org/issues/214">read</a>"#,
            RemoteImages::Blocked,
        );
        assert!(
            out.html
                .contains(r#"href="https://news.example.org/issues/214""#),
            "{}",
            out.html
        );
        assert_eq!(out.remote_blocked, 0, "a link is not a fetch");
    }

    #[test]
    fn a_javascript_href_is_stripped() {
        let out = sanitize_body(
            r#"<a href="javascript:alert(1)">click</a>"#,
            RemoteImages::Blocked,
        );
        assert!(!out.html.contains("javascript:"), "{}", out.html);
    }

    #[test]
    fn percent_encoding_round_trips() {
        for id in ["reader-left.44b1@example.com", "a b%c", "plain"] {
            assert_eq!(percent_decode(&percent_encode(id)), id);
        }
    }

    #[test]
    fn an_iframe_and_its_content_are_removed() {
        let out = sanitize_body(
            r#"<iframe src="https://tracker.example.org/beacon"></iframe><p>ok</p>"#,
            RemoteImages::Allowed,
        );
        assert_eq!(out.html, "<p>ok</p>");
    }
}
