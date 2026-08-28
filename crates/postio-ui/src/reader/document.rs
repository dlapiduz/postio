//! Assembling the document a message renders as — **the** document, one
//! implementation for every frontend (#567, ADR 0019 Q6).
//!
//! The highest risk in a second frontend is that the privacy invariants
//! silently fork: two readers, two content security policies, two link
//! policies, drifting invisibly until somebody's mail phones home. The
//! structural answer is here — `postio-gtk`'s WebKitGTK view and a macOS
//! `WKWebView` do not *agree* on the CSP, they **call the same function**.
//! What stays behind in each frontend is toolkit glue: how to hand this
//! string to a web view, nothing about what the string says.

use std::sync::OnceLock;

use base64::Engine as _;
use postio_body::quote;
use postio_body::sanitize::{self, RemoteImages};
use postio_model::message::MessageBody;

/// The security origin every rendered message loads under.
///
/// A fixed, non-`http(s)` scheme so a message's content is never same-origin
/// with any real site — nothing it contains gets that site's cookies, and
/// nothing on that site sees this page as one of its own frames. Nothing is
/// ever registered to handle this scheme, so a relative reference a sender
/// left in place resolves to a fetch that fails closed rather than one that
/// quietly reaches a host.
pub const DOCUMENT_BASE_URI: &str = "postio-reader:///";

/// Why the reading pane has no body to draw.
///
/// Issue #70, Cause A: all four of these used to render as a blank pane, so
/// a mailbox mid-backfill was indistinguishable from a broken application.
/// They are separate because the right response differs — three are worth
/// waiting for and one is finished, and only two are worth retrying.
///
/// This is the reader's half of the "partial" state every Postio surface
/// owes: headers synced, body not yet, which is the *ordinary* condition of
/// a mailbox that has just been added rather than a fault.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Absent {
    /// Headers are synced and the backfill has not reached the body yet.
    ///
    /// The overwhelmingly common one, and not an error: `request_body`
    /// queues a fetch and returns, so every message is in this state until
    /// the queue drains.
    Partial,
    /// Not downloaded, and nothing is downloading — the engine is offline.
    Offline,
    /// A body was recorded but its bytes are not in the blob store.
    ///
    /// Rare, and a real fault: the database and the blob directory disagree.
    Missing,
    /// Downloaded, and the message genuinely has no text or HTML part.
    Empty,
    /// `\Draft`, but written by another client: there is no local composer
    /// buffer behind the row, so there is nothing here to resume editing.
    ///
    /// Not a retryable wait like [`Absent::Partial`] or [`Absent::Offline`] --
    /// downloading its body would only ever produce something to read, never
    /// something to edit, so promising a retry would be a promise the reader
    /// cannot keep. See #175.
    ForeignDraft,
}

/// What the pane says for each [`Absent`], as the document's body.
///
/// A free function returning a string so the words and the key are testable
/// without a display — the assertions that matter here are "these four do
/// not say the same thing" and "the retryable ones name the retry key",
/// neither of which needs a web view to have painted anything.
pub fn absent_html(state: Absent) -> String {
    // `R` is the registry's alternate binding for `Refresh`, and already the
    // canvas' retry key for the list's empty and error plates. One retry key
    // for the whole application, not one per surface.
    const RETRY: &str = "Press <kbd>R</kbd> to check for new mail now.";
    let (heading, detail) = match state {
        Absent::Partial => (
            "Downloading this message",
            "Its headers are here; the body has not arrived yet. It will \
             appear as soon as it does."
                .to_owned(),
        ),
        Absent::Offline => (
            "Not downloaded yet",
            format!(
                "Postio is offline, so this message's body is not on this \
                 machine. It will arrive when the connection returns. {RETRY}"
            ),
        ),
        Absent::Missing => (
            "This message's body is missing",
            format!(
                "The message is in the local store but its body is not, so \
                 there is nothing here to show. {RETRY}"
            ),
        ),
        Absent::Empty => (
            "This message has no body",
            "Nothing arrived with it but the headers — that is the whole \
             message, not a fault."
                .to_owned(),
        ),
        Absent::ForeignDraft => (
            "Written on another device",
            "This draft was started in another mail client. Postio can \
             show it here but cannot edit it."
                .to_owned(),
        ),
    };
    format!(
        "<div class=\"postio-absent\">\
         <p class=\"postio-absent-heading\">{heading}</p>\
         <p class=\"postio-absent-detail\">{detail}</p>\
         </div>"
    )
}

/// How many invisible scroll markers [`scroll_markers`] lays down.
///
/// Bounds how far `page_down` can walk a message, not how long a message can
/// be: past the last marker further presses are a no-op, same as reaching
/// the end of any scrollable view. 60 markers at [`SCROLL_MARKER_STEP_VH`]
/// apart is generous enough that reaching it means an extraordinarily long
/// message, not an ordinary one.
pub const SCROLL_MARKERS: u32 = 60;

/// How far apart the markers sit, in viewport heights.
///
/// Short of 100 so consecutive pages keep a sliver of what was on screen —
/// paging with nothing carried over from the last screen is disorienting,
/// the same reason a paged reader or terminal pager rarely pages by exactly
/// one screen either.
pub const SCROLL_MARKER_STEP_VH: u32 = 90;

/// Invisible anchors spaced down the document, `#pos-0`, `#pos-1`, … — the
/// frontends' `page_down`/`page_up` jump between them.
///
/// This exists because a hardened web view leaves no other way to move the
/// scroll position: JavaScript is off on purpose, and neither WebKitGTK nor
/// WKWebView exposes a scroll-by-amount call to the host with it off. A
/// same-document fragment navigation is the one scroll primitive still open
/// with both closed — it is what every browser already implements `#anchor`
/// links with, and a frontend's link policy only intercepts a *user's*
/// click, so an app-issued navigation passes straight through.
///
/// Placed at `top: Nvh`, not a pixel offset: `vh` is resolved against
/// whatever the real viewport height is when the engine lays the document
/// out, so the markers land a consistent fraction of a screen apart on any
/// window size without this code ever learning what that size is. Confirmed
/// these contribute nothing to the document's scrollable height even when
/// they land far past the real content -- `scrollHeight` measured identical
/// with and without 50 such markers spanning 4500vh over one short
/// paragraph -- so a short message cannot grow a phantom scrollbar from
/// markers it never reaches.
pub fn scroll_markers() -> String {
    let mut markers = String::with_capacity(SCROLL_MARKERS as usize * 48);
    for position in 0..SCROLL_MARKERS {
        let top = position * SCROLL_MARKER_STEP_VH;
        markers.push_str(&format!(
            r#"<a id="pos-{position}" style="position:absolute;top:{top}vh;left:0;width:0;height:0;"></a>"#
        ));
    }
    markers
}

/// Wrap sanitized body markup in the document a web view is handed: Postio's
/// own stylesheet, and a `Content-Security-Policy` that closes off anything
/// the sanitizer missed.
///
/// The stylesheet is literal CSS, not the GTK `--postio-*` variables `tokens
/// .css` defines — a web view's CSS engine has no notion of the GTK style
/// context those live on. `reader-tokens.css` is generated from the same
/// design tokens as a literal `--r-*` palette (#296); `reader.css` is
/// structure only, referencing those variables.
pub fn wrap_document(content: &str, remote: RemoteImages) -> String {
    let css = reader_css();
    let csp = content_security_policy(remote);
    format!(
        "<!DOCTYPE html>\n<html><head>\n\
         <meta charset=\"utf-8\">\n\
         <meta http-equiv=\"Content-Security-Policy\" content=\"{csp}\">\n\
         <style>{css}</style>\n\
         </head><body>{content}</body></html>"
    )
}

/// The stylesheet [`wrap_document`] inlines: the generated token palette,
/// the structural rules, and the embedded font faces.
///
/// `include_str!` from the design data both frontends share — compiled in,
/// so a frontend cannot render with a stale or missing stylesheet.
fn reader_css() -> String {
    let mut css = embedded_font_faces().to_owned();
    css.push_str(include_str!("../../../postio-gtk/data/reader-tokens.css"));
    css.push_str(include_str!("../../../postio-gtk/data/reader.css"));
    css
}

/// `@font-face` rules embedding the vendored faces as `data:` URIs.
///
/// A web view's rendering happens in its own web process, which never sees
/// the fonts the host application registered — referencing the family name
/// by itself would fall back to whatever generic sans the sandbox happens
/// to have. Embedding the bytes is what makes "rendered text inherits
/// Postio typography" true regardless of what the web process can see.
///
/// Computed once: the font bytes are static for the process' life, and
/// base64-encoding four faces on every render would be wasted work.
fn embedded_font_faces() -> &'static str {
    static FACES: OnceLock<String> = OnceLock::new();
    FACES.get_or_init(build_font_faces)
}

fn build_font_faces() -> String {
    const FACES: &[(&[u8], &str, u16, &str)] = &[
        (
            include_bytes!("../../../postio-gtk/data/fonts/barlow/Barlow-Regular.ttf"),
            "Barlow",
            400,
            "normal",
        ),
        (
            include_bytes!("../../../postio-gtk/data/fonts/barlow/Barlow-Medium.ttf"),
            "Barlow",
            500,
            "normal",
        ),
        (
            include_bytes!("../../../postio-gtk/data/fonts/barlow/Barlow-Bold.ttf"),
            "Barlow",
            700,
            "normal",
        ),
        (
            include_bytes!("../../../postio-gtk/data/fonts/barlow/Barlow-Italic.ttf"),
            "Barlow",
            400,
            "italic",
        ),
        (
            include_bytes!(
                "../../../postio-gtk/data/fonts/barlow-condensed/BarlowCondensed-Regular.ttf"
            ),
            "Barlow Condensed",
            400,
            "normal",
        ),
        (
            include_bytes!(
                "../../../postio-gtk/data/fonts/barlow-condensed/BarlowCondensed-SemiBold.ttf"
            ),
            "Barlow Condensed",
            600,
            "normal",
        ),
        (
            include_bytes!("../../../postio-gtk/data/fonts/ibm-plex-mono/IBMPlexMono-Regular.ttf"),
            "IBM Plex Mono",
            400,
            "normal",
        ),
        (
            include_bytes!("../../../postio-gtk/data/fonts/ibm-plex-mono/IBMPlexMono-Medium.ttf"),
            "IBM Plex Mono",
            500,
            "normal",
        ),
    ];

    let mut out = String::new();
    for (bytes, family, weight, style) in FACES {
        let base64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        out.push_str(&format!(
            "@font-face{{font-family:'{family}';font-weight:{weight};\
             font-style:{style};src:url(data:font/ttf;base64,{base64}) format('truetype');}}\n"
        ));
    }
    out
}

/// What a render is holding back, split by kind.
///
/// Two numbers rather than one because the parts panel says them separately
/// ("3 remote images and 1 likely tracker"), and a single total could not.
/// The split comes from [`postio_body::sanitize::Sanitized`]; see its
/// `trackers` field for what the heuristic claims, which is less than the
/// name suggests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeldBack {
    /// Ordinary remote pictures that were stripped.
    pub remote_images: u32,
    /// Stripped references whose declared size makes them likely beacons.
    pub trackers: u32,
}

impl HeldBack {
    /// Everything held back, whatever kind it was.
    ///
    /// What the banner asks: whether it has anything at all to offer.
    pub fn total(self) -> u32 {
        self.remote_images + self.trackers
    }
}

/// The body markup: sanitized and quote-folded, but not yet wrapped in the
/// document template [`wrap_document`] adds, plus what was held back to
/// produce it — see [`sanitize::Sanitized`].
pub fn body_html(body: &MessageBody, remote: RemoteImages) -> (String, HeldBack) {
    if let Some(html) = body.html.as_deref().filter(|html| !html.trim().is_empty()) {
        let sanitized = sanitize::sanitize_body(html, remote);
        return (
            quote::fold_html_quotes(&sanitized.html),
            HeldBack {
                remote_images: sanitized.remote_blocked,
                trackers: sanitized.trackers,
            },
        );
    }
    if let Some(text) = body.text.as_deref().filter(|text| !text.trim().is_empty()) {
        return (quote::text_to_html(text), HeldBack::default());
    }
    (String::new(), HeldBack::default())
}

/// Give a sender's content a bounded surface of its own (#323): a visible
/// edge between what Postio wrote and what arrived in the message, so a
/// sender styling their markup to imitate application chrome has a harder
/// time — the boundary is a security affordance as much as a visual one.
///
/// Only ever wraps a real body. A frontend's own absent/empty states bypass
/// this and call [`wrap_document`] directly, so Postio's own words — the
/// "downloading" placeholder among them — stay outside the container, same
/// as chrome that is native toolkit widgets stacked around the rendering
/// surface rather than markup inside its document.
pub fn contain_body(content: &str) -> String {
    format!(r#"<div class="postio-body">{content}</div>"#)
}

/// The whole document for sender content that has already been sanitized.
///
/// The three steps a reader must not get wrong, in one place: bound the
/// sender's markup in `.postio-body`, append the scroll markers, and wrap the
/// result in the hardened template.
///
/// It exists because those three were a `format!` at the call site, and a
/// second frontend composing its own would be free to forget one. Forgetting
/// [`contain_body`] in particular loses a **security affordance** rather than
/// a style: #323 gave the sender's content a visible edge so that markup
/// imitating application chrome has a harder time, and a reader missing it
/// would look completely fine.
pub fn document_for(content: &str, remote: RemoteImages) -> String {
    wrap_document(
        &format!("{}{}", contain_body(content), scroll_markers()),
        remote,
    )
}

/// What the sanitizer already enforces at the DOM level, restated as policy
/// the rendering engine itself refuses to violate — so a sanitizer bug
/// degrades to broken markup, not a live request.
pub fn content_security_policy(remote: RemoteImages) -> String {
    let cid = sanitize::CID_SCHEME;
    let img_src = match remote {
        RemoteImages::Blocked => format!("{cid}: data:"),
        RemoteImages::Allowed => format!("{cid}: data: http: https:"),
    };
    format!(
        "default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; \
         img-src {img_src}; font-src data:; base-uri 'none'; form-action 'none'; \
         frame-src 'none'; connect-src 'none'"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_model::message::MessageBody;

    #[test]
    fn an_empty_body_produces_empty_content() {
        let (content, held_back) = body_html(&MessageBody::default(), RemoteImages::Blocked);
        assert_eq!(content, "");
        assert_eq!(held_back, HeldBack::default());
    }

    #[test]
    fn html_is_preferred_over_text_when_both_are_present() {
        let body = MessageBody {
            text: Some("plain fallback".to_owned()),
            html: Some("<p>rich</p>".to_owned()),
        };
        assert_eq!(body_html(&body, RemoteImages::Blocked).0, "<p>rich</p>");
    }

    #[test]
    fn text_only_bodies_still_render() {
        let body = MessageBody {
            text: Some("hello".to_owned()),
            html: None,
        };
        assert!(body_html(&body, RemoteImages::Blocked).0.contains("hello"));
    }

    #[test]
    fn a_remote_image_in_the_body_is_reported_as_blocked() {
        let body = MessageBody {
            text: None,
            html: Some(r#"<img src="https://tracker.example.org/o.gif">"#.to_owned()),
        };
        // No declared size, so the size heuristic reads it as an ordinary
        // picture whatever the host is called -- nothing here is domain-based
        // (#174). Blocked identically either way.
        assert_eq!(
            body_html(&body, RemoteImages::Blocked).1,
            HeldBack {
                remote_images: 1,
                trackers: 0
            }
        );
    }

    /// #323: a sender's content sits inside a bounded container, distinct
    /// from Postio's own words — this is the seam `render_open` uses, so
    /// proving it here proves the container actually reaches what a real
    /// render produces, not just that the CSS rule exists unused.
    #[test]
    fn a_rendered_body_sits_inside_its_own_container() {
        let document = wrap_document(&contain_body("<p>hi</p>"), RemoteImages::Blocked);
        assert!(
            document.contains(r#"<div class="postio-body"><p>hi</p></div>"#),
            "{document}"
        );
    }

    #[test]
    fn the_csp_is_byte_for_byte_what_the_adr_promises() {
        // ADR 0019 Q6: this exact string is the invariant both frontends
        // share. A frontend does not agree with it — it calls this.
        assert_eq!(
            content_security_policy(RemoteImages::Blocked),
            "default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; \
             img-src postio-cid: data:; font-src data:; base-uri 'none'; \
             form-action 'none'; frame-src 'none'; connect-src 'none'"
        );
        assert_eq!(
            content_security_policy(RemoteImages::Allowed),
            "default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; \
             img-src postio-cid: data: http: https:; font-src data:; base-uri 'none'; \
             form-action 'none'; frame-src 'none'; connect-src 'none'"
        );
    }

    #[test]
    fn the_wrapper_carries_the_policy_the_styles_and_the_content() {
        let document = wrap_document("<p>hello</p>", RemoteImages::Blocked);
        assert!(document.starts_with("<!DOCTYPE html>"));
        assert!(document.contains("Content-Security-Policy"));
        assert!(document.contains("img-src postio-cid: data:; font-src"));
        assert!(document.contains("<p>hello</p>"));
        assert!(
            document.contains("--r-"),
            "the generated reader palette is inlined"
        );
    }

    #[test]
    fn every_vendored_face_is_embedded_as_a_data_uri() {
        let faces = embedded_font_faces();
        for family in ["Barlow", "Barlow Condensed", "IBM Plex Mono"] {
            assert!(
                faces.contains(&format!("font-family:'{family}'")),
                "{family}"
            );
        }
        assert_eq!(
            faces.matches("data:font/ttf;base64,").count(),
            8,
            "eight faces, each embedded"
        );
    }

    #[test]
    fn the_absent_states_do_not_say_the_same_thing() {
        let states = [
            Absent::Partial,
            Absent::Offline,
            Absent::Missing,
            Absent::Empty,
            Absent::ForeignDraft,
        ];
        let texts: Vec<String> = states.iter().map(|state| absent_html(*state)).collect();
        for (index, text) in texts.iter().enumerate() {
            for other in texts.iter().skip(index + 1) {
                assert_ne!(text, other);
            }
        }
        // The retryable ones name the retry key; the others must not
        // promise a retry they cannot deliver.
        assert!(absent_html(Absent::Offline).contains("<kbd>R</kbd>"));
        assert!(absent_html(Absent::Missing).contains("<kbd>R</kbd>"));
        assert!(!absent_html(Absent::ForeignDraft).contains("<kbd>R</kbd>"));
    }

    #[test]
    fn the_markers_are_evenly_spaced_anchors() {
        let markers = scroll_markers();
        assert_eq!(markers.matches("<a id=\"pos-").count(), 60);
        assert!(markers.contains("top:0vh"));
        assert!(markers.contains("top:90vh"));
        assert!(markers.contains(&format!("top:{}vh", 59 * 90)));
    }
}
