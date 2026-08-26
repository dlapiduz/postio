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
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use ammonia::Builder;
use html5ever::driver::ParseOpts;
use html5ever::parse_document;
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{Handle, NodeData, RcDom};

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
    /// How many of the stripped remote references were **likely trackers**
    /// rather than ordinary pictures.
    ///
    /// Disjoint from [`Sanitized::remote_blocked`]: a reference is counted in
    /// one or the other, never both, so the parts panel can say "3 remote
    /// images and 1 likely tracker" from the two numbers directly.
    ///
    /// "Likely" is the honest word and the wording the panel uses. See
    /// [`is_likely_tracker`] for what the heuristic does and does not claim.
    pub trackers: u32,
}

impl Sanitized {
    /// Every remote reference that was stripped, whatever kind it was.
    ///
    /// What the banner asks: it decides whether it has anything to offer at
    /// all, and a message whose only remote reference was a beacon still has
    /// something to show if the user insists.
    pub fn held_back(&self) -> u32 {
        self.remote_blocked + self.trackers
    }
}

/// Sanitize one HTML body for the reading pane.
///
/// `cid:` references become [`CID_SCHEME`] URIs; `postio_gtk::reader::scheme` resolves
/// those against the message's local parts (or answers 404 for a dangling
/// reference — the corpus has one on purpose).
pub fn sanitize_body(html: &str, remote: RemoteImages) -> Sanitized {
    let blocked_count = Arc::new(AtomicU32::new(0));
    let counter = Arc::clone(&blocked_count);
    let tracker_count = Arc::new(AtomicU32::new(0));
    let trackers = Arc::clone(&tracker_count);

    // Which remote references look like beacons, decided before ammonia runs.
    //
    // `attribute_filter` is handed one attribute at a time and cannot see the
    // element's `width`, `height` or `style`, so the size an `<img>` declares
    // is simply not knowable from inside it. This walk answers that question
    // once, and the filter looks its `src` up.
    //
    // Skipped entirely when nothing is being blocked: the answer would not be
    // used, and a second parse of a large newsletter is not free.
    let beacons = if remote == RemoteImages::Blocked {
        likely_tracker_sources(html)
    } else {
        HashSet::new()
    };

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
            rewrite_attribute(
                element, attribute, value, remote, &counter, &trackers, &beacons,
            )
        });

    Sanitized {
        html: builder.clean(html).to_string(),
        remote_blocked: blocked_count.load(Ordering::Relaxed),
        trackers: tracker_count.load(Ordering::Relaxed),
    }
}

#[allow(clippy::too_many_arguments)]
fn rewrite_attribute<'u>(
    _element: &str,
    attribute: &str,
    value: &'u str,
    remote: RemoteImages,
    blocked_count: &AtomicU32,
    tracker_count: &AtomicU32,
    beacons: &HashSet<String>,
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
        // One or the other, never both: the panel adds them up.
        if beacons.contains(value.trim()) {
            tracker_count.fetch_add(1, Ordering::Relaxed);
        } else {
            blocked_count.fetch_add(1, Ordering::Relaxed);
        }
        return None;
    }
    Some(Cow::Borrowed(value))
}

/// The remote `src` values in `html` whose `<img>` declares beacon dimensions.
///
/// Returned as the set of URL strings rather than as a count, because the
/// counting has to happen where the blocking happens — ammonia decides what
/// actually gets stripped, and a reference this walk sees but the sanitizer
/// removes for some other reason must not be counted as held back.
///
/// A URL used twice in one message, once as a picture and once as a beacon,
/// is counted as a beacon both times. That is a real limitation and an
/// unreal message.
fn likely_tracker_sources(html: &str) -> HashSet<String> {
    let dom = parse_document(RcDom::default(), ParseOpts::default()).one(html);
    let mut found = HashSet::new();
    collect_beacons(&dom.document, &mut found);
    found
}

fn collect_beacons(node: &Handle, found: &mut HashSet<String>) {
    if let NodeData::Element { name, attrs, .. } = &node.data
        && name.local.as_ref().eq_ignore_ascii_case("img")
    {
        let attrs = attrs.borrow();
        let get = |wanted: &str| {
            attrs
                .iter()
                .find(|attr| attr.name.local.as_ref().eq_ignore_ascii_case(wanted))
                .map(|attr| attr.value.to_string())
        };
        if let Some(src) = get("src")
            && is_remote(&src)
            && is_likely_tracker(get("width"), get("height"), get("style"))
        {
            found.insert(src.trim().to_string());
        }
    }
    for child in node.children.borrow().iter() {
        collect_beacons(child, found);
    }
}

/// Whether an `<img>`'s own declarations mark it as a beacon rather than a
/// picture.
///
/// The maintainer settled this on 2026-08-25 (#174): **declared dimensions of
/// 2px or less in either axis, or a declaration that it is not to be shown at
/// all.** Nothing else. In particular nothing reads the host or the path — a
/// list of known tracking vendors is the provider hard-coding CLAUDE.md
/// forbids, and it would rot from the day it was written while mislabelling
/// every real picture served from a path with `pixel` in it.
///
/// # What it does not claim
///
/// It under-counts on purpose. A beacon that declares no size at all is
/// indistinguishable from an ordinary image that declares no size, and
/// ordinary images that declare no size are the common case — so silence is
/// read as a picture. This is a wording signal for the parts panel and
/// nothing more: **both kinds are blocked identically**, and a beacon this
/// misses is still never fetched. Being wrong here costs a noun, not a
/// request.
fn is_likely_tracker(width: Option<String>, height: Option<String>, style: Option<String>) -> bool {
    /// The largest an axis can be and still be a beacon rather than a picture.
    const BEACON_PX: f32 = 2.0;

    let tiny = |value: &str| length_px(value).is_some_and(|px| px <= BEACON_PX);

    if width.as_deref().is_some_and(&tiny) || height.as_deref().is_some_and(&tiny) {
        return true;
    }
    let Some(style) = style else { return false };
    let style = style.to_ascii_lowercase();
    for declaration in style.split(';') {
        let Some((property, value)) = declaration.split_once(':') else {
            continue;
        };
        let (property, value) = (property.trim(), value.trim());
        match property {
            "width" | "height" | "max-width" | "max-height" if tiny(value) => return true,
            // Said outright: this is not here to be looked at.
            "display" if value == "none" => return true,
            "visibility" if value == "hidden" => return true,
            _ => {}
        }
    }
    false
}

/// A declared length in CSS pixels, for the handful of spellings a mail body
/// actually uses.
///
/// `None` for anything else — a percentage, `auto`, `em`, or nonsense. A
/// length this cannot read is not evidence of a beacon, so it is not treated
/// as one.
fn length_px(value: &str) -> Option<f32> {
    let value = value.trim();
    let number = value
        .strip_suffix("px")
        .unwrap_or(value)
        .trim()
        .parse::<f32>()
        .ok()?;
    number.is_finite().then_some(number)
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
pub(crate) fn percent_encode(value: &str) -> String {
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

    // -- likely trackers ---------------------------------------------------
    //
    // The maintainer settled the heuristic on 2026-08-25 (#174): a remote
    // image reference is a likely tracker when its *declared* dimensions are
    // <= 2px in either axis, or when it is declared hidden outright. Nothing
    // domain-based and nothing path-based -- a list of known vendors is
    // exactly the provider hard-coding CLAUDE.md forbids, and it rots.
    //
    // This only ever changes the parts panel's *wording*. Both kinds are
    // blocked identically, so a beacon this misses is still not fetched.

    #[test]
    fn a_one_by_one_remote_pixel_is_a_likely_tracker() {
        let out = sanitize_body(
            r#"<img src="https://example.com/o.gif" width="1" height="1">"#,
            RemoteImages::Blocked,
        );
        assert_eq!(out.trackers, 1, "a 1x1 remote image is the classic beacon");
        assert_eq!(
            out.remote_blocked, 0,
            "counted as a tracker, so not also as an ordinary image"
        );
        assert_eq!(out.held_back(), 1);
    }

    #[test]
    fn an_ordinary_remote_picture_is_not_a_tracker() {
        let out = sanitize_body(
            r#"<img src="https://example.com/hero.png" width="600" height="300">"#,
            RemoteImages::Blocked,
        );
        assert_eq!(out.trackers, 0);
        assert_eq!(out.remote_blocked, 1);
    }

    #[test]
    fn a_remote_image_with_no_dimensions_at_all_is_not_a_tracker() {
        // Most senders declare nothing. Guessing "tracker" from silence would
        // label the ordinary case, which is worse than under-counting.
        let out = sanitize_body(
            r#"<img src="https://example.com/photo.jpg">"#,
            RemoteImages::Blocked,
        );
        assert_eq!(out.trackers, 0);
        assert_eq!(out.remote_blocked, 1);
    }

    #[test]
    fn a_pixel_sized_in_css_is_a_likely_tracker() {
        let out = sanitize_body(
            r#"<img src="https://example.com/p.gif" style="width:1px;height:1px">"#,
            RemoteImages::Blocked,
        );
        assert_eq!(
            out.trackers, 1,
            "the beacon shape is the size, not the attribute it was spelled in"
        );
    }

    #[test]
    fn an_image_declared_hidden_is_a_likely_tracker() {
        // No dimensions, but it says outright that it is not to be seen.
        let out = sanitize_body(
            r#"<img src="https://example.com/b.gif" style="display:none">"#,
            RemoteImages::Blocked,
        );
        assert_eq!(out.trackers, 1);
    }

    #[test]
    fn a_url_that_merely_looks_like_a_beacon_is_not_one() {
        // Path- and domain-based guessing is out of bounds: it mislabels a
        // real picture served from a path with "track" in it, and a vendor
        // list would need updating forever.
        let out = sanitize_body(
            r#"<img src="https://example.com/track/pixel/beacon.gif?id=9" width="480" height="240">"#,
            RemoteImages::Blocked,
        );
        assert_eq!(
            out.trackers, 0,
            "480x240 is a picture, whatever its URL is spelled like"
        );
        assert_eq!(out.remote_blocked, 1);
    }

    #[test]
    fn pictures_and_beacons_are_counted_separately_in_one_message() {
        let out = sanitize_body(
            r#"<img src="https://example.com/a.png" width="600" height="200">
               <img src="https://example.com/b.png">
               <img src="https://example.com/o.gif" width="1" height="1">"#,
            RemoteImages::Blocked,
        );
        assert_eq!(out.remote_blocked, 2);
        assert_eq!(out.trackers, 1);
        assert_eq!(out.held_back(), 3);
    }

    #[test]
    fn an_allowed_message_holds_nothing_back_of_either_kind() {
        let out = sanitize_body(
            r#"<img src="https://example.com/o.gif" width="1" height="1">"#,
            RemoteImages::Allowed,
        );
        assert_eq!(out.remote_blocked, 0);
        assert_eq!(out.trackers, 0);
        assert!(out.html.contains("https://example.com/o.gif"));
    }

    #[test]
    fn an_inline_pixel_is_not_a_tracker_because_nothing_is_fetched_for_it() {
        // `cid:` resolves against the message's own parts. There is no host
        // to report to, so its size says nothing.
        let out = sanitize_body(
            r#"<img src="cid:x" width="1" height="1">"#,
            RemoteImages::Blocked,
        );
        assert_eq!(out.trackers, 0);
        assert_eq!(out.remote_blocked, 0);
    }

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
