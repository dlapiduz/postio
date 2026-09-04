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

use postio_body::quote;
use postio_body::reader_view;
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
pub fn wrap_document(content: &str, remote: RemoteImages, sheet: Sheet) -> String {
    let mut css = reader_css();
    let csp = content_security_policy(remote);
    let root_class = match sheet {
        Sheet::Theme => String::new(),
        Sheet::Senders => {
            css.push_str(&senders_sheet_css());
            format!(" class=\"{SENDERS_SHEET_CLASS}\"")
        }
    };
    format!(
        "<!DOCTYPE html>\n<html{root_class}><head>\n\
         <meta charset=\"utf-8\">\n\
         <meta http-equiv=\"Content-Security-Policy\" content=\"{csp}\">\n\
         <style>{css}</style>\n\
         </head><body>{content}</body></html>"
    )
}

/// The rules that turn the sender's box into their own page.
///
/// Scoped to `.postio-body` and not to `:root`, which is the whole design:
/// `body` keeps painting `var(--r-ground)` from the theme, so the chrome
/// around the sheet stays dark and the sheet has an edge to be inset from.
/// Custom properties inherit, so every rule inside the box — the link
/// colour, the quote ground, the hairlines — resolves against the light
/// palette without `reader.css` knowing this mode exists.
///
/// The values are read out of the generated palette rather than restated,
/// the same rule and the same reason as [`reader_ground`]: #296 says a
/// colour has one source, and a second copy is one that can drift.
fn senders_sheet_css() -> String {
    format!(
        "\n.{SENDERS_SHEET_CLASS} .postio-body {{{}\n  background: var(--r-ground);\n  \
         color: var(--r-ink);\n}}\n",
        light_tokens()
    )
}

/// The light half of the generated palette, as declarations.
///
/// A sibling of [`reader_ground`], split out of the same file the same way:
/// everything before the dark block is the light scheme, and the `:root`
/// body of it is the set of values a sender's page should be drawn with.
fn light_tokens() -> &'static str {
    const PALETTE: &str = include_str!("../../data/reader-tokens.css");
    const DARK_BLOCK: &str = "@media (prefers-color-scheme: dark)";

    let (light, _) = PALETTE
        .split_once(DARK_BLOCK)
        .expect("the generated palette always emits a dark block");
    light
        .split_once(":root {")
        .and_then(|(_, rest)| rest.split_once('}'))
        .map(|(declarations, _)| declarations.trim_end())
        .expect("the generated palette always opens with a light :root block")
}

/// The stylesheet [`wrap_document`] inlines: the generated token palette,
/// the structural rules, and the embedded font faces.
///
/// `include_str!` from this crate's own data directory — compiled in, so a
/// frontend cannot render with a stale or missing stylesheet.
fn reader_css() -> String {
    let mut css = embedded_font_faces().to_owned();
    css.push_str(include_str!("../../data/reader-tokens.css"));
    css.push_str(include_str!("../../data/reader.css"));
    css
}

/// The reader's ground colour for the given scheme, as the generated palette
/// spells it (`#rrggbb`).
///
/// A frontend needs this outside the document as well as inside it. The
/// document paints `--r-ground` on `body`, but only once it has *parsed* —
/// and a web view between one document and the next has no document to take
/// a colour from, so it paints whatever its widget background is. Left
/// unset, that is the engine default, which under the GTK4 GL path shows as
/// a black frame on every message change (#749).
///
/// Read out of the same generated CSS the document inlines, rather than
/// restated as a literal here: #296's rule is that a colour has one source,
/// and a second copy would be one that could silently drift from the design
/// system the first is regenerated from.
pub fn reader_ground(dark: bool) -> &'static str {
    const PALETTE: &str = include_str!("../../data/reader-tokens.css");
    const DARK_BLOCK: &str = "@media (prefers-color-scheme: dark)";

    let (light, dark_block) = PALETTE
        .split_once(DARK_BLOCK)
        .expect("the generated palette always emits a dark block");
    let scope = if dark { dark_block } else { light };
    scope
        .split_once("--r-ground:")
        .and_then(|(_, rest)| rest.split_once(';'))
        .map(|(value, _)| value.trim())
        .expect("the generated palette always defines --r-ground in both schemes")
}

/// The scheme the reader's web view fetches Postio's own typefaces over
/// (ADR 0023).
///
/// A sibling of [`sanitize::CID_SCHEME`], and answered the same way: from
/// compiled-in bytes, in-process, never from a path and never from the
/// network. It exists because a web view's content process cannot see the
/// fonts the host application registered with its toolkit — so the faces
/// have to reach the engine somehow, and *serving* them is what keeps the
/// document proportional to the message.
pub const FONT_SCHEME: &str = "postio-font";

/// One vendored typeface, as the reader refers to it and as the handler
/// serves it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Face {
    /// The name a `postio-font:` URL carries, and the only thing a request
    /// may name — see [`font_bytes`].
    pub name: &'static str,
    /// The CSS family this face belongs to.
    pub family: &'static str,
    /// Its weight, as `font-weight`.
    pub weight: u16,
    /// `normal` or `italic`.
    pub style: &'static str,
    /// The face itself, compiled in.
    pub bytes: &'static [u8],
}

/// Every face the reader may draw with — the fixed table both the
/// `@font-face` rules and the scheme handler are generated from.
///
/// A compile-time table, deliberately: it is what makes "a font URL can only
/// ever resolve to one of eight files this project vendored" a property of
/// the type system rather than of a handler remembering to check. Nothing
/// here is a path, so there is no traversal to get wrong.
///
/// The bytes have one owner (#799): `postio-gtk`'s Pango integration
/// (`fonts::install_into`) reads them from here rather than keeping a second
/// copy in its own `GResource` bundle.
///
/// `static`, not `const`: a `const` is re-evaluated at every use site, so a
/// second crate reading `FACES` would get its own freshly promoted copy of
/// every byte array — the exact duplication this table exists to remove, just
/// moved from the `GResource` bundle into the linker's `.rodata` instead. A
/// `static` has one address for the life of the binary, so `postio-gtk`
/// referencing it costs a pointer, not 909 KB.
///
/// Provenance — https://github.com/google/fonts, `main`, fetched 2026-08-22:
///   fonts/barlow/            ofl/barlow/            © 2017 The Barlow Project Authors
///   fonts/barlow-condensed/  ofl/barlowcondensed/   © 2017 The Barlow Project Authors
///   fonts/ibm-plex-mono/     ofl/ibmplexmono/       © 2017 IBM Corp. ("Plex")
pub static FACES: &[Face] = &[
    Face {
        name: "Barlow-Regular.ttf",
        family: "Barlow",
        weight: 400,
        style: "normal",
        bytes: include_bytes!("../../data/fonts/barlow/Barlow-Regular.ttf"),
    },
    Face {
        name: "Barlow-Medium.ttf",
        family: "Barlow",
        weight: 500,
        style: "normal",
        bytes: include_bytes!("../../data/fonts/barlow/Barlow-Medium.ttf"),
    },
    Face {
        name: "Barlow-Bold.ttf",
        family: "Barlow",
        weight: 700,
        style: "normal",
        bytes: include_bytes!("../../data/fonts/barlow/Barlow-Bold.ttf"),
    },
    Face {
        name: "Barlow-Italic.ttf",
        family: "Barlow",
        weight: 400,
        style: "italic",
        bytes: include_bytes!("../../data/fonts/barlow/Barlow-Italic.ttf"),
    },
    Face {
        name: "BarlowCondensed-Regular.ttf",
        family: "Barlow Condensed",
        weight: 400,
        style: "normal",
        bytes: include_bytes!("../../data/fonts/barlow-condensed/BarlowCondensed-Regular.ttf"),
    },
    Face {
        name: "BarlowCondensed-SemiBold.ttf",
        family: "Barlow Condensed",
        weight: 600,
        style: "normal",
        bytes: include_bytes!("../../data/fonts/barlow-condensed/BarlowCondensed-SemiBold.ttf"),
    },
    Face {
        name: "IBMPlexMono-Regular.ttf",
        family: "IBM Plex Mono",
        weight: 400,
        style: "normal",
        bytes: include_bytes!("../../data/fonts/ibm-plex-mono/IBMPlexMono-Regular.ttf"),
    },
    Face {
        name: "IBMPlexMono-Medium.ttf",
        family: "IBM Plex Mono",
        weight: 500,
        style: "normal",
        bytes: include_bytes!("../../data/fonts/ibm-plex-mono/IBMPlexMono-Medium.ttf"),
    },
];

/// Each vendored family's licence text, as `(family, OFL text)` — the same
/// families [`FACES`] embeds, read from beside them so the licence a family
/// ships under can never drift from the bytes it names.
///
/// `postio-gtk`'s About dialog (`fonts::licenses`) attributes the fonts from
/// here rather than from a copy that could go stale. `static` for the same
/// reason as [`FACES`].
pub static LICENSES: &[(&str, &str)] = &[
    ("Barlow", include_str!("../../data/fonts/barlow/OFL.txt")),
    (
        "Barlow Condensed",
        include_str!("../../data/fonts/barlow-condensed/OFL.txt"),
    ),
    (
        "IBM Plex Mono",
        include_str!("../../data/fonts/ibm-plex-mono/OFL.txt"),
    ),
];

/// The face `name` refers to, or `None`.
///
/// What a frontend's `postio-font:` handler answers requests from. A name
/// that is not in [`FACES`] resolves to nothing — the same "not found" a
/// `cid:` with no matching part gets, and for the same reason: a scheme that
/// falls through to somewhere is a scheme that can be aimed.
pub fn font_bytes(name: &str) -> Option<&'static [u8]> {
    FACES
        .iter()
        .find(|face| face.name == name)
        .map(|face| face.bytes)
}

/// The MIME type every served face carries.
pub const FONT_MIME: &str = "font/ttf";

/// `@font-face` rules pointing at [`FONT_SCHEME`] URLs.
///
/// A web view's rendering happens in its own web process, which never sees
/// the fonts the host application registered — referencing the family name
/// by itself would fall back to whatever generic sans the sandbox happens
/// to have. Naming the faces here is what makes "rendered text inherits
/// Postio typography" true regardless of what the web process can see.
///
/// These used to carry the bytes, as ~1.21 MB of base64 inlined into every
/// document — an empty pane and every absent plate included (#768, ADR
/// 0023). Now the document names the faces and the engine fetches only the
/// ones the page actually draws with: two for a state plate, one for a
/// plain-text body, rather than eight for everything.
///
/// `font-display: block` because the fetch is asynchronous where a `data:`
/// URI was immediate. Without it there is a window where body text paints in
/// a fallback face and then reflows — a flash of wrong font in the one pane
/// whose whole job is rendering text stably. `block` paints nothing briefly
/// and then swaps, which for bytes compiled into the process and served from
/// memory is imperceptible; `optional` would let one hiccup leave the reader
/// in system sans for good, which is the outcome this whole mechanism exists
/// to prevent.
///
/// Computed once: the rules are static for the process' life.
fn embedded_font_faces() -> &'static str {
    static RULES: OnceLock<String> = OnceLock::new();
    RULES.get_or_init(build_font_faces)
}

fn build_font_faces() -> String {
    let mut out = String::new();
    for face in FACES {
        let Face {
            name,
            family,
            weight,
            style,
            ..
        } = face;
        out.push_str(&format!(
            "@font-face{{font-family:'{family}';font-weight:{weight};\
             font-style:{style};font-display:block;\
             src:url({FONT_SCHEME}:{name}) format('truetype');}}\n"
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
    /// What the notice says: `14 remote images and 1 tracker blocked`.
    ///
    /// Both numbers, because they are different claims: a picture the sender
    /// wanted you to see and a beacon that wanted to see you are not the same
    /// thing to a person deciding whether to load them. Trackers alone still
    /// say "tracker" rather than folding into a picture count — the whole
    /// reason #174 taught the parts panel to tell them apart.
    ///
    /// Empty when nothing was held back: a notice with nothing to report
    /// should not be on screen at all.
    pub fn summary(self) -> String {
        let images = self.remote_images;
        let trackers = self.trackers;
        let picture = |n: u32| {
            if n == 1 {
                "1 remote image".to_owned()
            } else {
                format!("{n} remote images")
            }
        };
        let beacon = |n: u32| {
            if n == 1 {
                "1 tracker".to_owned()
            } else {
                format!("{n} trackers")
            }
        };
        match (images, trackers) {
            (0, 0) => String::new(),
            (n, 0) => format!("{} blocked", picture(n)),
            (0, n) => format!("{} blocked", beacon(n)),
            (i, t) => format!("{} and {} blocked", picture(i), beacon(t)),
        }
    }

    /// Everything held back, whatever kind it was.
    ///
    /// What the banner asks: whether it has anything at all to offer.
    pub fn total(self) -> u32 {
        self.remote_images + self.trackers
    }
}

/// Which of the two ways a message can be drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rendering {
    /// The sender's own markup, sanitized and quote-folded. What ordinary
    /// correspondence gets, and what `View original` goes back to.
    #[default]
    Original,
    /// Reduced to what carries meaning — [`postio_body::reader_view`]. The
    /// default for bulk mail.
    Reader,
}

/// Which paper a message is drawn on.
///
/// Turn 7, screen 20 of the canvas: "When you do open the original, it
/// renders on its own paper-white sheet inset from the dark chrome — sender
/// CSS never fights the app theme, and link colors stay the sender's."
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sheet {
    /// Postio's own ground, following the reader's colour scheme.
    ///
    /// What correspondence gets. A reply drawn on a white sheet inside a
    /// dark window would be worse than what the theme already does, so this
    /// stays the default and the sender's sheet is the exception.
    #[default]
    Theme,
    /// The sender's own paper: the light palette whatever the scheme, so
    /// their white-background logo, their mid-grey body text and their links
    /// land on the page they were designed for.
    ///
    /// Only the sender's *box* changes. The chrome around it keeps the
    /// theme's ground, which is what makes the sheet read as a sheet.
    Senders,
}

/// The class [`Sheet::Senders`] puts on the document root.
///
/// Namespaced like every other class Postio injects, because the sanitizer
/// leaves a sender's own `class` attributes alone.
pub const SENDERS_SHEET_CLASS: &str = "postio-senders-sheet";

/// Which paper this rendering belongs on.
///
/// Exactly one situation earns the sender's sheet: the person left reader
/// view to see the original of something reader view had offered to reduce.
/// That is a deliberate "show me what they actually sent", and it is the
/// only time Postio's palette is the wrong one to draw it in.
///
/// `suits_reader_view` is passed rather than recomputed from the body:
/// [`suits_reader_view`] parses the markup, both frontends already call it
/// to choose the rendering, and asking twice per render would put an
/// html5ever pass on the path every ordinary message takes.
pub fn sheet_for(rendering: Rendering, suits_reader_view: bool) -> Sheet {
    match (rendering, suits_reader_view) {
        (Rendering::Original, true) => Sheet::Senders,
        _ => Sheet::Theme,
    }
}

/// A body, drawn, and everything a surface needs to say about how.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rendered {
    /// The markup, ready for [`contain_body`].
    pub html: String,
    /// What the remote-image policy held back.
    pub held_back: HeldBack,
    /// Which way it was drawn — what the notice reports, and what decides
    /// whether `View original` has anything to offer.
    pub rendering: Rendering,
    /// Links that survived reduction. Zero in [`Rendering::Original`], where
    /// nothing is dropped and the count would be a claim about nothing.
    pub links_kept: usize,
    /// Links reduction collapsed. The canvas draws the pair as
    /// `1 link kept of 23`.
    pub links_dropped: usize,
}

impl Rendered {
    /// Every link the message had. Zero unless it was reduced.
    pub fn links_total(&self) -> usize {
        self.links_kept + self.links_dropped
    }
}

/// Whether this body is one reader view should open on.
///
/// **Bulk mail only.** A person's correspondence opening reduced is the one
/// failure that would make the feature hated, so the question is not "could
/// this be reduced" — anything can — but "was this laid out by a template".
/// [`postio_body::reader_view::reads_as_bulk`] answers it from the
/// arrangement.
///
/// A message with no HTML part at all is never a candidate: there is no
/// markup to reduce, and plain text is already what reader view is trying to
/// get back to.
pub fn suits_reader_view(body: &MessageBody) -> bool {
    body.html
        .as_deref()
        .filter(|html| !html.trim().is_empty())
        .is_some_and(reader_view::reads_as_bulk)
}

/// The body markup: sanitized and quote-folded, but not yet wrapped in the
/// document template [`wrap_document`] adds, plus what was held back to
/// produce it — see [`sanitize::Sanitized`].
///
/// # Reader view prefers the plain part
///
/// A `multipart/alternative` carries the sender's own plain-text version,
/// which is the thing reduction is trying to reconstruct — so when there is
/// one, reader view uses it rather than reducing the HTML alongside it.
/// Reduction is the fallback for HTML-only bulk mail, not the first
/// resort.
///
/// Note what this does *not* do: it never reaches for the plain part in
/// [`Rendering::Original`]. The existing preference order there — HTML
/// first, text as a fallback — is what makes ordinary mail look like the
/// sender wrote it.
pub fn body_html(body: &MessageBody, remote: RemoteImages, rendering: Rendering) -> Rendered {
    let html = body.html.as_deref().filter(|html| !html.trim().is_empty());
    let text = body.text.as_deref().filter(|text| !text.trim().is_empty());

    if rendering == Rendering::Reader {
        if let Some(text) = text {
            return Rendered {
                html: quote::text_to_html(text),
                held_back: HeldBack::default(),
                rendering: Rendering::Reader,
                ..Rendered::default()
            };
        }
        if let Some(html) = html {
            // Sanitized first, always. Reduction is a readability pass over
            // markup that has already been made safe, and running it on raw
            // sender HTML would be relying on it for a promise it does not
            // make.
            let sanitized = sanitize::sanitize_body(html, remote);
            let reduced = reader_view::reduce(&sanitized.html);
            return Rendered {
                html: reduced.html,
                held_back: HeldBack {
                    remote_images: sanitized.remote_blocked,
                    trackers: sanitized.trackers,
                },
                rendering: Rendering::Reader,
                links_kept: reduced.links_kept,
                links_dropped: reduced.links_dropped,
            };
        }
    }

    if let Some(html) = html {
        let sanitized = sanitize::sanitize_body(html, remote);
        return Rendered {
            html: quote::fold_html_quotes(&sanitized.html),
            held_back: HeldBack {
                remote_images: sanitized.remote_blocked,
                trackers: sanitized.trackers,
            },
            rendering: Rendering::Original,
            ..Rendered::default()
        };
    }
    if let Some(text) = text {
        return Rendered {
            html: quote::text_to_html(text),
            ..Rendered::default()
        };
    }
    Rendered::default()
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
pub fn document_for(content: &str, remote: RemoteImages, sheet: Sheet) -> String {
    wrap_document(
        &format!("{}{}", contain_body(content), scroll_markers()),
        remote,
        sheet,
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
         img-src {img_src}; font-src {FONT_SCHEME}:; base-uri 'none'; form-action 'none'; \
         frame-src 'none'; connect-src 'none'"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_model::message::MessageBody;

    /// The sender's sheet is for one gesture only: leaving reader view.
    ///
    /// Correspondence is `Original` too, and is the case that must *not*
    /// change — a reply on a white page inside a dark window is the failure
    /// this rule exists to avoid.
    #[test]
    fn only_the_original_of_something_reader_view_offered_gets_the_senders_sheet() {
        assert_eq!(sheet_for(Rendering::Original, true), Sheet::Senders);
        assert_eq!(sheet_for(Rendering::Original, false), Sheet::Theme);
        assert_eq!(sheet_for(Rendering::Reader, true), Sheet::Theme);
        assert_eq!(sheet_for(Rendering::Reader, false), Sheet::Theme);
    }

    #[test]
    fn an_ordinary_document_carries_no_sheet_of_its_own() {
        let document = document_for("<p>hi</p>", RemoteImages::Blocked, Sheet::Theme);
        assert!(
            !document.contains(SENDERS_SHEET_CLASS),
            "the theme document must not carry the sender's sheet: {document}"
        );
    }

    /// The light values land **inside** the sender's box and nowhere else.
    ///
    /// Scoped to `:root` they would repaint `body` too, and the sheet would
    /// have no dark chrome to be inset from — which is the whole picture the
    /// canvas asks for, not a detail of it.
    #[test]
    fn the_senders_sheet_lights_the_body_box_and_leaves_the_chrome_alone() {
        let document = document_for("<p>hi</p>", RemoteImages::Blocked, Sheet::Senders);
        assert!(
            document.contains(&format!(r#"<html class="{SENDERS_SHEET_CLASS}">"#)),
            "the root says which sheet it is: {document}"
        );
        assert!(
            document.contains(&format!(".{SENDERS_SHEET_CLASS} .postio-body {{")),
            "the override is scoped to the sender's box: {document}"
        );
        assert!(
            !document.contains(&format!(".{SENDERS_SHEET_CLASS} :root")),
            "nothing may repaint the root, or the chrome stops being chrome"
        );
    }

    /// The sheet's colours are the palette's light ones, read from the same
    /// place `reader_ground` reads them — not a second copy that can drift
    /// away from the design system (#296).
    #[test]
    fn the_senders_sheet_is_the_generated_light_palette_and_not_a_second_copy() {
        let document = document_for("<p>hi</p>", RemoteImages::Blocked, Sheet::Senders);
        let light = reader_ground(false);
        let dark = reader_ground(true);
        assert_ne!(light, dark, "the palette must actually differ by scheme");

        let sheet = document
            .split_once(&format!(".{SENDERS_SHEET_CLASS} .postio-body {{"))
            .expect("the sheet rule is in the document")
            .1
            .split_once('}')
            .expect("the sheet rule closes")
            .0;
        assert!(
            sheet.contains(&format!("--r-ground: {light}")),
            "the sheet paints the light ground: {sheet}"
        );
        assert!(
            !sheet.contains(dark),
            "no dark value belongs on the sender's page: {sheet}"
        );
        // The link colour is what the acceptance names, and it rides the
        // same variable `reader.css` already resolves `a { color: ... }`
        // against -- so proving the accent is overridden proves the links.
        assert!(
            sheet.contains("--r-accent:"),
            "the accent is what `a` resolves against: {sheet}"
        );
    }

    /// A body drawn the ordinary way — what every one of these asserted on
    /// before reader view existed.
    fn drawn(body: &MessageBody) -> Rendered {
        body_html(body, RemoteImages::Blocked, Rendering::Original)
    }

    #[test]
    fn an_empty_body_produces_empty_content() {
        let rendered = drawn(&MessageBody::default());
        assert_eq!(rendered.html, "");
        assert_eq!(rendered.held_back, HeldBack::default());
    }

    #[test]
    fn html_is_preferred_over_text_when_both_are_present() {
        let body = MessageBody {
            text: Some("plain fallback".to_owned()),
            html: Some("<p>rich</p>".to_owned()),
        };
        assert_eq!(drawn(&body).html, "<p>rich</p>");
    }

    #[test]
    fn text_only_bodies_still_render() {
        let body = MessageBody {
            text: Some("hello".to_owned()),
            html: None,
        };
        assert!(drawn(&body).html.contains("hello"));
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
            drawn(&body).held_back,
            HeldBack {
                remote_images: 1,
                trackers: 0
            }
        );
    }

    // -- reader view (#1009) ----------------------------------------------

    /// A campaign: nested layout tables, and more links than a person writes.
    fn campaign() -> String {
        let mut html = String::from("<table><tr><td><table><tr><td>");
        html.push_str(r#"<p><a href="https://example.com/track">Track delivery</a></p>"#);
        for index in 0..12 {
            html.push_str(&format!(
                r#"<p><a href="https://example.com/{index}">more</a></p>"#
            ));
        }
        html.push_str("</td></tr></table></td></tr></table>");
        html
    }

    #[test]
    fn reader_view_prefers_the_senders_own_plain_part() {
        // The `multipart/alternative` case, and the whole reason reduction is
        // a fallback rather than the first resort: the sender already wrote
        // the version reduction is trying to reconstruct.
        let body = MessageBody {
            text: Some("Your package is out for delivery.".to_owned()),
            html: Some(campaign()),
        };
        let rendered = body_html(&body, RemoteImages::Blocked, Rendering::Reader);
        assert!(
            rendered.html.contains("Your package is out for delivery."),
            "{}",
            rendered.html
        );
        assert!(
            !rendered.html.contains("<table"),
            "the HTML part was drawn instead of the plain one: {}",
            rendered.html
        );
        assert_eq!(
            rendered.links_total(),
            0,
            "there is nothing to count in a plain part"
        );
    }

    #[test]
    fn reader_view_reduces_html_only_bulk_mail() {
        let body = MessageBody {
            text: None,
            html: Some(campaign()),
        };
        let rendered = body_html(&body, RemoteImages::Blocked, Rendering::Reader);
        assert_eq!(rendered.rendering, Rendering::Reader);
        assert!(!rendered.html.contains("<table"), "{}", rendered.html);
        assert_eq!(rendered.links_kept, 1);
        assert_eq!(rendered.links_total(), 13, "1 link kept of 13");
    }

    #[test]
    fn the_original_rendering_never_reaches_for_the_plain_part() {
        // The preference order that makes ordinary mail look like the sender
        // wrote it. Reader view inverts it; `View original` must not.
        let body = MessageBody {
            text: Some("plain fallback".to_owned()),
            html: Some(campaign()),
        };
        let rendered = drawn(&body);
        assert_eq!(rendered.rendering, Rendering::Original);
        assert!(
            !rendered.html.contains("plain fallback"),
            "the original is the sender's markup, whatever else came with it"
        );
    }

    #[test]
    fn only_bulk_mail_is_offered_reader_view() {
        // The failure that would make the feature hated: correspondence
        // opening reduced.
        let reply = MessageBody {
            text: None,
            html: Some("<p>Hi Ada,</p><p>Friday works.</p>".to_owned()),
        };
        assert!(!suits_reader_view(&reply));

        let bulk = MessageBody {
            text: None,
            html: Some(campaign()),
        };
        assert!(suits_reader_view(&bulk));

        // Nothing to reduce is not a candidate either: plain text is already
        // where reader view is trying to get to.
        let plain = MessageBody {
            text: Some("hello".to_owned()),
            html: None,
        };
        assert!(!suits_reader_view(&plain));
    }

    #[test]
    fn a_blocked_image_is_still_reported_when_the_markup_is_reduced() {
        // Reduction runs after sanitizing, so the counts have to survive it —
        // a notice that went quiet in reader view would be the privacy
        // invariant silently weakening in one of two modes.
        let mut html = campaign();
        html.push_str(r#"<img src="https://tracker.example.org/o.gif">"#);
        let body = MessageBody {
            text: None,
            html: Some(html),
        };
        let rendered = body_html(&body, RemoteImages::Blocked, Rendering::Reader);
        assert_eq!(rendered.held_back.total(), 1);
    }

    /// #323: a sender's content sits inside a bounded container, distinct
    /// from Postio's own words — this is the seam `render_open` uses, so
    /// proving it here proves the container actually reaches what a real
    /// render produces, not just that the CSS rule exists unused.
    #[test]
    fn a_rendered_body_sits_inside_its_own_container() {
        let document = wrap_document(
            &contain_body("<p>hi</p>"),
            RemoteImages::Blocked,
            Sheet::Theme,
        );
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
             img-src postio-cid: data:; font-src postio-font:; base-uri 'none'; \
             form-action 'none'; frame-src 'none'; connect-src 'none'"
        );
        assert_eq!(
            content_security_policy(RemoteImages::Allowed),
            "default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; \
             img-src postio-cid: data: http: https:; font-src postio-font:; base-uri 'none'; \
             form-action 'none'; frame-src 'none'; connect-src 'none'"
        );
    }

    #[test]
    fn the_wrapper_carries_the_policy_the_styles_and_the_content() {
        let document = wrap_document("<p>hello</p>", RemoteImages::Blocked, Sheet::Theme);
        assert!(document.starts_with("<!DOCTYPE html>"));
        assert!(document.contains("Content-Security-Policy"));
        assert!(document.contains("img-src postio-cid: data:; font-src"));
        assert!(document.contains("<p>hello</p>"));
        assert!(
            document.contains("--r-"),
            "the generated reader palette is inlined"
        );
    }

    /// The colour a frontend paints behind the document has to be the one the
    /// document paints on itself, or the seam shows on every message change
    /// (#749). Both are read from the generated palette, so this asserts they
    /// are the same string rather than restating either.
    #[test]
    fn the_ground_a_frontend_paints_is_the_one_the_document_paints() {
        for dark in [false, true] {
            let ground = reader_ground(dark);
            assert!(
                ground.starts_with('#'),
                "the ground should be a hex literal the frontend can parse: {ground}"
            );
            let palette = include_str!("../../data/reader-tokens.css");
            assert!(
                palette.contains(&format!("--r-ground: {ground};")),
                "reader_ground({dark}) returned {ground}, which the generated \
                 palette does not define"
            );
        }
        assert_ne!(
            reader_ground(false),
            reader_ground(true),
            "the two schemes should not share a ground"
        );
    }

    #[test]
    fn every_vendored_face_is_referenced_and_resolvable() {
        // ADR 0023 replaced "embedded as a data URI" with this, and it is
        // the same guarantee: a face that silently goes missing takes the
        // reader to system sans, whether it went missing from a base64 blob
        // or from the table the handler serves.
        let faces = embedded_font_faces();
        for family in ["Barlow", "Barlow Condensed", "IBM Plex Mono"] {
            assert!(
                faces.contains(&format!("font-family:'{family}'")),
                "{family}"
            );
        }
        assert_eq!(
            faces.matches(&format!("src:url({FONT_SCHEME}:")).count(),
            8,
            "eight faces, each referenced"
        );

        // Referenced *and* resolvable: a rule naming a face the handler
        // cannot serve is a font that never arrives, which is exactly the
        // failure a count of rules alone would miss.
        assert_eq!(FACES.len(), 8, "eight vendored faces");
        for face in FACES {
            assert!(
                faces.contains(&format!("src:url({FONT_SCHEME}:{})", face.name)),
                "{} is served but never referenced",
                face.name
            );
            let bytes = font_bytes(face.name).expect("a referenced face resolves");
            assert!(
                bytes.starts_with(&[0x00, 0x01, 0x00, 0x00]) || bytes.starts_with(b"true"),
                "{} does not begin with a TrueType signature",
                face.name
            );
        }

        // And nothing else resolves. The table is the whole of what a font
        // URL may reach — no paths, so no traversal to get wrong.
        for name in [
            "",
            "Barlow-Regular",
            "../fonts/barlow/Barlow-Regular.ttf",
            "/etc/passwd",
        ] {
            assert!(font_bytes(name).is_none(), "{name} resolved to something");
        }
    }

    /// #768: the document is what a message change costs. It used to carry
    /// ~1.21 MB of base64 whatever the message was — an empty pane and every
    /// absent plate included.
    #[test]
    fn the_document_is_proportional_to_the_message_not_to_the_font_catalogue() {
        let document = document_for("<p>hi</p>", RemoteImages::Blocked, Sheet::Theme);
        assert!(
            !document.contains("data:font/"),
            "the faces are still travelling with the document"
        );
        assert!(
            document.len() < 64 * 1024,
            "a one-line message renders a {} byte document",
            document.len()
        );
        // Still Postio's type: the rules are there, naming faces the handler
        // serves. What left is the payload, not the typography.
        assert!(document.contains(&format!("src:url({FONT_SCHEME}:")));
        assert!(document.contains("--r-"), "the palette is still inlined");
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

    #[test]
    fn the_notice_counts_pictures_and_beacons_separately() {
        // A picture the sender wanted you to see and a beacon that wanted to
        // see you are different claims (#174).
        assert_eq!(
            HeldBack {
                remote_images: 14,
                trackers: 1
            }
            .summary(),
            "14 remote images and 1 tracker blocked",
            "the canvas's own wording"
        );
        assert_eq!(
            HeldBack {
                remote_images: 3,
                trackers: 0
            }
            .summary(),
            "3 remote images blocked"
        );
        assert_eq!(
            HeldBack {
                remote_images: 0,
                trackers: 2
            }
            .summary(),
            "2 trackers blocked",
            "a beacon is still named a beacon when it is the only thing there"
        );
    }

    #[test]
    fn one_of_each_is_singular() {
        assert_eq!(
            HeldBack {
                remote_images: 1,
                trackers: 1
            }
            .summary(),
            "1 remote image and 1 tracker blocked"
        );
    }

    #[test]
    fn nothing_held_back_says_nothing() {
        // A notice with nothing to report should not be on screen at all.
        assert_eq!(HeldBack::default().summary(), "");
    }
}
