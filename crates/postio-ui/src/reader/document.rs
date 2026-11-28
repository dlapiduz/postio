//! Assembling the document a message renders as: one implementation for every frontend (#567, ADR
//! 0019 Q6), so the CSP and link policy cannot fork. Frontends keep only the glue that hands the
//! string to a web view.

use std::sync::OnceLock;

use postio_body::quote;
use postio_body::reader_view;
use postio_body::sanitize;
pub use postio_body::sanitize::RemoteImages;
use postio_body::treatment::{self, TREATMENT_ATTRIBUTE, Treatment, Trigger};
use postio_model::message::MessageBody;

/// Why the reading pane has no body to draw (#70). Separate because the response differs: three are
/// worth waiting for, one is finished, only two are worth retrying.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Absent {
    /// The ordinary condition of a mailbox just added, not an error: `request_body` queues a fetch
    /// and returns.
    Partial,
    Offline,
    Missing,
    Empty,
    /// Written by another client: nothing to resume editing, and a body download would only give
    /// something to read, so no retry is promised (#175).
    ForeignDraft,
    /// Finished rather than a wait: a "downloading" plate would be a spinner with nothing behind
    /// it.
    Failed,
}

/// `None` is an account nothing has been said about: reachable, so a pane never claims offline on
/// no evidence (#1988). Only `Online` reaches the server (#1986).
pub fn reaches_server(link: Option<postio_core::ConnectionState>) -> bool {
    matches!(link, None | Some(postio_core::ConnectionState::Online))
}

/// A server's refusal outranks the link; the link outranks "downloading", which would promise a
/// fetch that cannot start (#1986).
pub fn unfetched(failed: bool, unreachable: bool) -> Absent {
    if failed {
        Absent::Failed
    } else if unreachable {
        Absent::Offline
    } else {
        Absent::Partial
    }
}

pub fn absent_html(state: Absent) -> String {
    // Refresh's own key from the registry: one retry key for the whole application.
    let retry = format!(
        "Press <kbd>{}</kbd> to check for new mail now.",
        crate::hints::short(
            postio_core::registry::get(postio_core::CommandId::Refresh).default_binding
        )
    );
    let retry = retry.as_str();
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
                 machine. It will arrive when the connection returns. {retry}"
            ),
        ),
        Absent::Missing => (
            "This message's body is missing",
            format!(
                "The message is in the local store but its body is not, so \
                 there is nothing here to show. {retry}"
            ),
        ),
        Absent::Empty => (
            "This message has no body",
            "Nothing arrived with it but the headers — that is the whole \
             message, not a fault."
                .to_owned(),
        ),
        Absent::Failed => (
            "This message could not be downloaded",
            "The server did not give up its body when Postio last asked. Its \
             headers are shown above. Postio asks again each time the \
             message is opened."
                .to_owned(),
        ),
        Absent::ForeignDraft => (
            "Written on another device",
            "This draft was started in another mail client. Postio can \
             show it here but cannot edit it."
                .to_owned(),
        ),
    };
    // `role="status"`, polite: the content is replaced rather than focused, so nothing else
    // announces this state change.
    format!(
        "<div class=\"postio-absent\" role=\"status\" aria-live=\"polite\">\
         <p class=\"postio-absent-heading\">{heading}</p>\
         <p class=\"postio-absent-detail\">{detail}</p>\
         </div>"
    )
}

/// One line, deliberately short of the reason: what a reader needs is that the words below may not
/// be the words sent (#901).
pub const ENCRYPTED: &str = "This message is encrypted. Postio can't decrypt mail yet.";

pub fn is_encrypted(content_type: &str) -> bool {
    matches!(
        content_type.trim().to_ascii_lowercase().as_str(),
        "multipart/encrypted" | "application/pkcs7-mime" | "application/x-pkcs7-mime"
    )
}

pub const DECODE_CAVEAT: &str = "Parts of this message could not be decoded";

/// A notice with nothing to report must not be on screen: it teaches people to dismiss the one that
/// matters.
pub fn decode_caveat(encoding_problems: bool) -> Option<&'static str> {
    encoding_problems.then_some(DECODE_CAVEAT)
}

/// Bounds how far `page_down` can walk a message; past the last marker further presses are a no-op.
pub const SCROLL_MARKERS: u32 = 60;

/// Short of 100 so consecutive pages keep a sliver of what was on screen.
pub const SCROLL_MARKER_STEP_VH: u32 = 90;

/// The markers are the only scroll primitive a hardened web view leaves (JavaScript is off).
/// Clamped at both ends rather than wrapping: a `space` that jumped to the top would lose the
/// reader's place.
pub fn page_after(current: u32, forward: bool) -> u32 {
    if forward {
        (current + 1).min(SCROLL_MARKERS - 1)
    } else {
        current.saturating_sub(1)
    }
}

/// One spelling, so the anchors laid down and the fragments asked for cannot drift apart.
pub fn page_fragment(page: u32) -> String {
    format!("pos-{page}")
}

/// Invisible anchors `#pos-0`, `#pos-1`, ... for `page_down`/`page_up`: with JavaScript off, a
/// same-document fragment navigation is the one scroll primitive left. Placed in `vh` so they land
/// a fixed fraction of a screen apart on any window; they add nothing to the scrollable height.
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

/// The stylesheet is literal CSS, not the GTK `--postio-*` variables: a web view has no GTK style
/// context.
/// `reader-tokens.css` is generated from the design tokens (#296); `reader.css` is structure only.
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
    let document = format!(
        "<!DOCTYPE html>\n<html{root_class}><head>\n\
         <meta charset=\"utf-8\">\n\
         <meta http-equiv=\"Content-Security-Policy\" content=\"{csp}\">\n\
         <style>{css}</style>\n\
         </head><body>{content}</body></html>"
    );
    // Counted here, where every document ends, so a thread's document is counted like any other.
    crate::reader::cost::note_document(document.len());
    document
}

/// Scoped to `.postio-body`, not `:root`: `body` keeps painting `var(--r-ground)` so the chrome
/// stays dark.
/// Custom properties inherit, so rules inside resolve against the light palette. Read from the
/// generated palette (#296).
pub(crate) fn senders_sheet_css() -> String {
    format!(
        "\n.{SENDERS_SHEET_CLASS} .postio-body {{{}\n  background: var(--r-ground);\n  \
         color: var(--r-ink);\n}}\n",
        light_tokens()
    )
}

pub(crate) const PALETTE: &str = include_str!("../../data/reader-tokens.css");

pub(crate) const DARK_BLOCK: &str = "@media (prefers-color-scheme: dark)";

pub(crate) fn light_tokens() -> &'static str {
    let (light, _) = PALETTE
        .split_once(DARK_BLOCK)
        .expect("the generated palette always emits a dark block");
    light
        .split_once(":root {")
        .and_then(|(_, rest)| rest.split_once('}'))
        .map(|(declarations, _)| declarations.trim_end())
        .expect("the generated palette always opens with a light :root block")
}

/// `include_str!` so a frontend cannot render with a stale or missing stylesheet.
fn reader_css() -> String {
    let mut css = embedded_font_faces().to_owned();
    css.push_str(include_str!("../../data/reader-tokens.css"));
    css.push_str(include_str!("../../data/reader.css"));
    css.push_str(include_str!("../../data/treatment.css"));
    css.push_str(&paper_palette_css());
    css
}

/// The light scheme whatever the reader's, so anything of Postio's drawn on the sheet reads on
/// white.
fn paper_palette_css() -> String {
    format!(
        "\n.{}[{TREATMENT_ATTRIBUTE}=\"{}\"] {{{}\n}}\n",
        sanitize::BODY_CLASS,
        Treatment::Paper.attribute_value(),
        light_tokens()
    )
}

/// Needed outside the document: between documents a web view has no colour to take, and the engine
/// default shows as a black frame on every message change under the GTK4 GL path (#749). Read from
/// the generated CSS (#296).
pub fn reader_ground(dark: bool) -> &'static str {
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

/// Answered like `sanitize::CID_SCHEME`: from compiled-in bytes, in-process, never a path or the
/// network.
/// The web view's content process cannot see fonts the host registered (ADR 0023).
pub const FONT_SCHEME: &str = "postio-font";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Face {
    pub name: &'static str,
    pub family: &'static str,
    pub weight: u16,
    pub style: &'static str,
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
/// The bytes have one owner (#799): the classic app's Pango integration
/// (`fonts::install_into`) read them from here rather than keeping a second
/// copy in its own `GResource` bundle.
///
/// `static`, not `const`: a `const` is re-evaluated at every use site, so a
/// second crate reading `FACES` would get its own freshly promoted copy of
/// every byte array — the exact duplication this table exists to remove, just
/// moved from the `GResource` bundle into the linker's `.rodata` instead. A
/// `static` has one address for the life of the binary, so the classic app
/// referencing it costs a pointer, not 909 KB.
///
/// Provenance — <https://github.com/google/fonts>, `main`, fetched 2026-08-22:
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

/// A name not in `FACES` resolves to nothing, like a `cid:` with no matching part: a scheme that
/// falls through somewhere can be aimed.
pub fn font_bytes(name: &str) -> Option<&'static [u8]> {
    FACES
        .iter()
        .find(|face| face.name == name)
        .map(|face| face.bytes)
}

pub const FONT_MIME: &str = "font/ttf";

/// `font-display: block` because the fetch is asynchronous: otherwise text paints in a fallback
/// face and reflows, and `optional` could leave the reader in system sans for good. The document
/// names the faces so the engine fetches only those drawn with (#768, ADR 0023). Computed once.
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeldBack {
    pub remote_images: u32,
    pub trackers: u32,
}

impl HeldBack {
    /// Empty when nothing was held back. Both numbers, since a picture the sender wanted seen and a
    /// beacon are different claims (#174).
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

    pub fn total(self) -> u32 {
        self.remote_images + self.trackers
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rendering {
    #[default]
    Original,
    Reader,
}

/// The original renders on its own paper-white sheet inset from the dark chrome, so sender CSS
/// never fights the app theme.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sheet {
    /// The default: a reply on a white sheet in a dark window would be worse than the theme.
    #[default]
    Theme,
    /// The light palette whatever the scheme; only the sender's box changes, so the chrome around
    /// it keeps the theme's ground.
    Senders,
}

pub const SENDERS_SHEET_CLASS: &str = "postio-senders-sheet";

/// Only leaving reader view to see the original of something it offered to reduce earns the
/// sender's sheet.
/// `suits_reader_view` is passed in because computing it parses the markup.
pub fn sheet_for(rendering: Rendering, suits_reader_view: bool) -> Sheet {
    match (rendering, suits_reader_view) {
        (Rendering::Original, true) => Sheet::Senders,
        _ => Sheet::Theme,
    }
}

/// Reader view is a command away, never the default: it flattened newsletters.
pub fn opening_rendering() -> Rendering {
    Rendering::Original
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rendered {
    pub html: String,
    /// Carried beside the markup to the document because the sender's `<style>` does not survive
    /// sanitizing; Postio emits CSS it parsed itself.
    pub styles: String,
    pub held_back: HeldBack,
    pub rendering: Rendering,
    /// Zero in `Original`, where nothing is dropped.
    pub links_kept: usize,
    pub links_dropped: usize,
    pub over_cap: Option<postio_body::Cap>,
    pub treated: Option<Treated>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Treated {
    pub shown: Treatment,
    pub classified: Treatment,
    pub trigger: Option<Trigger>,
    pub html: bool,
}

impl Treated {
    pub const TEXT: Treated = Treated {
        shown: Treatment::AppColours,
        classified: Treatment::AppColours,
        trigger: None,
        html: false,
    };
}

pub const FLOW_CSS: &str = "\nbody { background: var(--flow-ground); padding: 0; }\n";

/// Correspondence has no page of its own: the body loses its frame and padding so its lines share
/// both edges with the blocks above.
pub const FLOW_FLAT_CSS: &str =
    ".postio-body { padding: 0; border: 0; border-radius: 0; min-height: 0; }\n";

pub const ALWAYS_FOR_SENDER: &str = "Always for this sender";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderModeWords {
    pub title: &'static str,
    pub detail: &'static str,
    pub action: &'static str,
    pub offer_always: bool,
}

pub fn render_mode_words(
    treated: Treated,
    remembered: Option<Treatment>,
) -> Option<RenderModeWords> {
    if !treated.html {
        return None;
    }
    let (title, detail, action) = match (treated.shown, treated.classified) {
        (Treatment::AppColours, _) => (
            "App colours",
            "sender colours and fonts removed",
            "Show original",
        ),
        (Treatment::Paper, Treatment::Paper) => (
            "Original layout, on paper",
            if treated
                .trigger
                .is_some_and(|trigger| !trigger.is_background())
            {
                "this message lays out its own page"
            } else {
                "this message sets its own background"
            },
            "Use app colours",
        ),
        (Treatment::Paper, Treatment::AppColours) => (
            "Original colours, on paper",
            "as the sender styled it",
            "Use app colours",
        ),
    };
    let kept = remembered == Some(treated.shown);
    Some(RenderModeWords {
        title,
        detail: if kept {
            "always for this sender"
        } else {
            detail
        },
        action,
        offer_always: !kept && (treated.shown != treated.classified || remembered.is_some()),
    })
}

impl Rendered {
    pub fn links_total(&self) -> usize {
        self.links_kept + self.links_dropped
    }
}

/// Bulk mail only: correspondence opening reduced would be the failure that makes the feature
/// hated, so the question is "was this laid out by a template". No HTML part is never a candidate.
pub fn suits_reader_view(body: &MessageBody) -> bool {
    body.html
        .as_deref()
        .filter(|html| !html.trim().is_empty())
        .is_some_and(|html| {
            crate::reader::cost::bump(&crate::reader::cost::BULK_JUDGED, 1);
            reader_view::reads_as_bulk(html)
        })
}

/// Public because it is the seam a frontend test asserts on.
pub const FACTS_CLASS: &str = "postio-facts";

fn facts_html(rows: &[reader_view::Fact]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let mut out = format!("<dl class=\"{FACTS_CLASS}\">");
    for fact in rows {
        out.push_str("<dt>");
        escape_into(&mut out, &fact.label);
        out.push_str("</dt><dd>");
        escape_into(&mut out, &fact.value);
        out.push_str("</dd>");
    }
    out.push_str("</dl>");
    out
}

fn escape_into(out: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
}

/// Reader view prefers the sender's plain part over reducing the HTML; reduction is the fallback
/// for HTML-only bulk mail. It never uses the plain part in `Rendering::Original`.
pub fn body_html(body: &MessageBody, remote: RemoteImages, rendering: Rendering) -> Rendered {
    body_html_in(body, remote, rendering, None)
}

/// `scope` makes every `cid:` reference name its own message, required for a conversation document
/// (ADR 0032); `None` is exactly `body_html`.
pub fn body_html_in(
    body: &MessageBody,
    remote: RemoteImages,
    rendering: Rendering,
    scope: Option<&str>,
) -> Rendered {
    crate::reader::cost::bump(&crate::reader::cost::BODIES_SANITISED, 1);
    let html = body.html.as_deref().filter(|html| !html.trim().is_empty());
    let text = body.text.as_deref().filter(|text| !text.trim().is_empty());

    if rendering == Rendering::Reader {
        // The plain part is preferred as the sender's own reduction; when it is their markup
        // flattened, the HTML is the better source (`reads_as_flattened_markup`).
        let text =
            text.filter(|text| html.is_none() || !reader_view::reads_as_flattened_markup(text));
        if let Some(text) = text {
            let lifted = reader_view::lift(text);
            let mut html = facts_html(&lifted.rows);
            html.push_str(&quote::text_to_html(&lifted.body));
            return Rendered {
                html,
                held_back: HeldBack::default(),
                rendering: Rendering::Reader,
                ..Rendered::default()
            };
        }
        if let Some(html) = html {
            // Sanitized first, always: reduction is a readability pass, not a safety one.
            let sanitized = sanitize::sanitize_body_in(html, remote, scope);
            if let Some(cap) = sanitized.over_cap {
                return over_cap(body, cap);
            }
            let reduced = reader_view::reduce(&sanitized.html);
            return Rendered {
                html: reduced.html,
                // Not `sanitized.styles`: reduction strips `style`, `class` and the rest, so a
                // stylesheet would put the layout back halfway. `View original` carries them.
                styles: String::new(),
                held_back: HeldBack {
                    remote_images: sanitized.remote_blocked,
                    trackers: sanitized.trackers,
                },
                rendering: Rendering::Reader,
                links_kept: reduced.links_kept,
                links_dropped: reduced.links_dropped,
                over_cap: None,
                treated: None,
            };
        }
    }

    if let Some(html) = html {
        let sanitized = sanitize::sanitize_body_in(html, remote, scope);
        if let Some(cap) = sanitized.over_cap {
            return over_cap(body, cap);
        }
        return Rendered {
            html: on_canvas(
                &format!(
                    r#"<div class="{ORIGINAL_CLASS}">{}</div>"#,
                    quote::fold_html_quotes(&sanitized.html)
                ),
                &sanitized,
            ),
            styles: sanitized.styles,
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

/// Not a body sanitised: there is no sanitiser here, and the fallback is built on every load on the
/// interface thread.
pub fn plain_text_body_html(text: &str) -> String {
    if text.trim().is_empty() {
        String::new()
    } else {
        quote::text_to_html(text)
    }
}

fn over_cap(body: &MessageBody, cap: postio_body::Cap) -> Rendered {
    let html = match body.text.as_deref().filter(|text| !text.trim().is_empty()) {
        Some(text) => quote::text_to_html(text),
        None => "<p>This message is too large to display.</p>".to_owned(),
    };
    Rendered {
        html,
        over_cap: Some(cap),
        ..Rendered::default()
    }
}

pub const APP_COLOURS_CLASS: &str = "postio-app-colours";

/// Paper by the person's choice rather than the rule's: correspondence has no page margins, so the
/// sheet gives some.
pub const LETTER_CLASS: &str = "postio-letter";

/// A sheet drawn "as sent" gets a browser's default 8px page margin unless the sender set their
/// own.
pub const PAGE_MARGIN_CLASS: &str = "postio-page-margin";

fn sets_page_margin(canvas: &sanitize::Canvas) -> bool {
    canvas.style.split(';').any(|declaration| {
        declaration.split_once(':').is_some_and(|(property, _)| {
            let property = property.trim().to_ascii_lowercase();
            property.starts_with("margin") || property.starts_with("padding")
        })
    })
}

/// `chosen` is the person's choice (`⇧O`, or "Always for this sender") and wins over the rule;
/// `None` lets `treatment::classify` decide. Plain text is always app colours. Always
/// `Rendering::Original`.
pub fn body_html_treated(
    body: &MessageBody,
    remote: RemoteImages,
    chosen: Option<Treatment>,
    scope: Option<&str>,
) -> Rendered {
    let Some(html) = body.html.as_deref().filter(|html| !html.trim().is_empty()) else {
        return Rendered {
            treated: Some(Treated::TEXT),
            ..body_html_in(body, remote, Rendering::Original, scope)
        };
    };
    crate::reader::cost::bump(&crate::reader::cost::BODIES_SANITISED, 1);
    let sanitized = sanitize::sanitize_body_in(html, remote, scope);
    if let Some(cap) = sanitized.over_cap {
        return Rendered {
            treated: Some(Treated::TEXT),
            ..over_cap(body, cap)
        };
    }
    treat_sanitized(&sanitized, chosen)
}

fn treat_sanitized(sanitized: &sanitize::Sanitized, chosen: Option<Treatment>) -> Rendered {
    crate::reader::cost::bump(&crate::reader::cost::BODIES_TREATED, 1);
    let trigger = treatment::paper_trigger(sanitized);
    let classified = if trigger.is_some() {
        Treatment::Paper
    } else {
        Treatment::AppColours
    };
    let shown = chosen.unwrap_or(classified);
    let (html, styles) = match shown {
        Treatment::AppColours => (
            format!(
                r#"<div class="{APP_COLOURS_CLASS}">{}</div>"#,
                quote::fold_html_quotes(&treatment::app_colours(&sanitized.html))
            ),
            treatment::app_colours_css(&sanitized.styles),
        ),
        Treatment::Paper => {
            let letter = if classified != Treatment::Paper {
                format!(" {LETTER_CLASS}")
            } else if !sets_page_margin(&sanitized.canvas) {
                format!(" {PAGE_MARGIN_CLASS}")
            } else {
                String::new()
            };
            (
                on_canvas(
                    &format!(
                        r#"<div class="{ORIGINAL_CLASS}{letter}">{}</div>"#,
                        quote::fold_html_quotes(&sanitized.html)
                    ),
                    sanitized,
                ),
                treatment::light_only(&sanitized.styles),
            )
        }
    };
    Rendered {
        html,
        styles,
        held_back: HeldBack {
            remote_images: sanitized.remote_blocked,
            trackers: sanitized.trackers,
        },
        rendering: Rendering::Original,
        treated: Some(Treated {
            shown,
            classified,
            trigger,
            html: true,
        }),
        ..Rendered::default()
    }
}

/// Postio-owned and in the reserved namespace, so a sender cannot wear it; plain text and reader
/// view are never wrapped in it.
pub const ORIGINAL_CLASS: &str = "postio-original";

/// The sanitizer's output is a fragment, so what `<html>` and `<body>` said about the page arrives
/// as a `Canvas` and is put back as `.postio-canvas`. A sender who styled no page gets no wrapper.
pub fn on_canvas(content: &str, sanitized: &sanitize::Sanitized) -> String {
    let canvas = &sanitized.canvas;
    if canvas.style.is_empty() && sanitized.color_scheme.is_none() {
        return content.to_owned();
    }
    let mut out = String::from(r#"<div class="postio-canvas""#);
    if !canvas.style.is_empty() {
        out.push_str(r#" style=""#);
        escape_into(&mut out, &canvas.style);
        out.push('"');
    }
    if let Some(scheme) = sanitized.color_scheme {
        out.push_str(r#" data-postio-color-scheme=""#);
        out.push_str(scheme.as_str());
        out.push('"');
    }
    out.push('>');
    out.push_str(content);
    out.push_str("</div>");
    out
}

/// A visible edge between what Postio wrote and what arrived, so markup imitating application
/// chrome is harder to pass off (#323).
/// Postio's own absent/empty states bypass this and call `wrap_document` directly.
pub fn contain_body(content: &str) -> String {
    contain_body_in(content, None)
}

/// The name is what `sanitize::message_selector` matches; without it the scoped selector names
/// nothing and the message renders unstyled.
pub fn contain_body_in(content: &str, scope: Option<&str>) -> String {
    let class = sanitize::BODY_CLASS;
    match scope {
        Some(scope) => format!(
            r#"<div class="{class}" {}="{}">{content}</div>"#,
            sanitize::MESSAGE_ATTRIBUTE,
            {
                let mut escaped = String::with_capacity(scope.len());
                escape_into(&mut escaped, scope);
                escaped
            }
        ),
        None => format!(r#"<div class="{class}">{content}</div>"#),
    }
}

/// Bounds the sender's markup in `.postio-body`, appends the scroll markers and wraps in the
/// hardened template, in one place so no frontend forgets a step: forgetting `contain_body` loses a
/// security affordance (#323). Quote folds get their ids.
pub fn document_for(content: &str, styles: &str, remote: RemoteImages, sheet: Sheet) -> String {
    wrap_document(
        &format!(
            "{}{}{}",
            senders_stylesheet(styles),
            number_quote_folds(&contain_body(content), ""),
            scroll_markers()
        ),
        remote,
        sheet,
    )
}

const QUOTE_FOLD: &str = "<details class=\"postio-quote\"";

/// Numbered `<prefix>q<n>` in document order: what the renderer opens a fold by. In a conversation
/// the prefix is the message's anchor, so no two messages share one.
pub(crate) fn number_quote_folds(content: &str, prefix: &str) -> String {
    let fold = super::thread::FOLD_ATTRIBUTE;
    let mut parts = content.split(QUOTE_FOLD);
    let mut out = String::with_capacity(content.len());
    out.push_str(parts.next().unwrap_or_default());
    for (n, rest) in parts.enumerate() {
        out.push_str(&format!("{QUOTE_FOLD} {fold}=\"{prefix}q{n}\""));
        out.push_str(rest);
    }
    out
}

pub fn document_for_treated(
    content: &str,
    styles: &str,
    remote: RemoteImages,
    treatment: Treatment,
) -> String {
    wrap_document(
        &format!(
            "{}{}{}",
            senders_stylesheet(styles),
            number_quote_folds(&contain_body_treated(content, treatment), ""),
            scroll_markers()
        ),
        remote,
        Sheet::Theme,
    )
}

pub fn contain_body_treated(content: &str, treatment: Treatment) -> String {
    format!(
        r#"<div class="{}" {TREATMENT_ATTRIBUTE}="{}">{content}</div>"#,
        sanitize::BODY_CLASS,
        treatment.attribute_value()
    )
}

/// After Postio's sheet, not before: a message renders as its sender built it, so the sender's rule
/// wins a genuine collision.
pub fn senders_stylesheet(styles: &str) -> String {
    if styles.trim().is_empty() {
        return String::new();
    }
    // `</style` is the one sequence that ends the element; refused anyway so a future change to
    // `postio_body::styles` cannot turn a stylesheet into markup.
    let safe = styles.replace("</style", "");
    format!("<style>\n{safe}</style>")
}

/// A sanitizer bug degrades to broken markup, not a live request. `style-src` names no host, which
/// is load-bearing since #1326:
/// `'unsafe-inline'` permits no fetch.
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

#[derive(Clone, Debug)]
pub struct Prepared {
    scope: String,
    body: MessageBody,
    remote: RemoteImages,
    verdict: bool,
    rendering: Rendering,
    rendered: Rendered,
    treated: Option<Box<[Rendered; 2]>>,
}

impl Prepared {
    pub fn serves(&self, body: &MessageBody, remote: RemoteImages, rendering: Rendering) -> bool {
        self.remote == remote && self.rendering == rendering && self.body == *body
    }

    pub fn verdict_for(&self, body: &MessageBody) -> Option<bool> {
        (self.body == *body).then_some(self.verdict)
    }

    pub fn rendered(&self) -> &Rendered {
        &self.rendered
    }

    pub fn treated_for(
        &self,
        body: &MessageBody,
        remote: RemoteImages,
        chosen: Option<Treatment>,
    ) -> Option<&Rendered> {
        if self.remote != remote || self.body != *body {
            return None;
        }
        let [app, paper] = &**self.treated.as_ref()?;
        let shown = chosen
            .or_else(|| app.treated.map(|treated| treated.classified))
            .unwrap_or(Treatment::AppColours);
        Some(match shown {
            Treatment::AppColours => app,
            Treatment::Paper => paper,
        })
    }
}

/// Sanitised once, classified and drawn both ways, so opening a message and switching its treatment
/// are free of the work.
/// For a worker; no reader-view verdict, since a treating reader has none.
pub fn prepare_treated(body: &MessageBody, remote: RemoteImages) -> Prepared {
    let both = match body.html.as_deref().filter(|html| !html.trim().is_empty()) {
        None => {
            let text = body_html_treated(body, remote, None, None);
            [text.clone(), text]
        }
        Some(html) => {
            crate::reader::cost::bump(&crate::reader::cost::BODIES_SANITISED, 1);
            let sanitized = sanitize::sanitize_body_in(html, remote, None);
            match sanitized.over_cap {
                Some(cap) => {
                    let capped = Rendered {
                        treated: Some(Treated::TEXT),
                        ..over_cap(body, cap)
                    };
                    [capped.clone(), capped]
                }
                None => [
                    treat_sanitized(&sanitized, Some(Treatment::AppColours)),
                    treat_sanitized(&sanitized, Some(Treatment::Paper)),
                ],
            }
        }
    };
    Prepared {
        scope: String::new(),
        body: body.clone(),
        remote,
        verdict: false,
        rendering: Rendering::Original,
        rendered: both[0].clone(),
        treated: Some(Box::new(both)),
    }
}

pub fn prepare(scope: &str, body: &MessageBody, remote: RemoteImages) -> Prepared {
    let verdict = suits_reader_view(body);
    let rendering = opening_rendering();
    Prepared {
        scope: scope.to_owned(),
        body: body.clone(),
        remote,
        verdict,
        rendering,
        rendered: body_html_in(body, remote, rendering, Some(scope)),
        treated: None,
    }
}

/// A conversation redraws whenever anything queues one, and each redraw put every body through the
/// sanitiser again on the main thread (#1605).
/// The key is the body, image policy and rendering; the body is compared by value.
#[derive(Debug, Default)]
pub struct RenderCache {
    held: std::collections::HashMap<String, Held>,
    judged: std::collections::HashMap<String, (MessageBody, bool)>,
    /// Apart from `held` so trimming to the thread on screen does not throw away the thread about
    /// to be.
    offered: std::collections::HashMap<String, Prepared>,
}

const OFFERED_LIMIT: usize = 512;

#[derive(Debug)]
struct Held {
    body: MessageBody,
    remote: RemoteImages,
    rendering: Rendering,
    rendered: Rendered,
}

impl RenderCache {
    pub fn render(
        &mut self,
        scope: &str,
        body: &MessageBody,
        remote: RemoteImages,
        rendering: Rendering,
    ) -> Rendered {
        if let Some(held) = self.held.get(scope)
            && held.remote == remote
            && held.rendering == rendering
            && held.body == *body
        {
            return held.rendered.clone();
        }
        let offered = self.offered.remove(scope).filter(|prepared| {
            prepared.remote == remote && prepared.rendering == rendering && prepared.body == *body
        });
        let rendered = match offered {
            Some(prepared) => prepared.rendered,
            None => body_html_in(body, remote, rendering, Some(scope)),
        };
        self.held.insert(
            scope.to_owned(),
            Held {
                body: body.clone(),
                remote,
                rendering,
                rendered: rendered.clone(),
            },
        );
        rendered
    }

    /// Added to, not replaced: a thread's bodies are offered one at a time. A drawn one is taken
    /// out; the rest are dropped wholesale past `OFFERED_LIMIT`.
    pub fn offer(&mut self, prepared: Vec<Prepared>) {
        if self.offered.len() + prepared.len() > OFFERED_LIMIT {
            self.offered.clear();
        }
        self.offered.extend(
            prepared
                .into_iter()
                .map(|prepared| (prepared.scope.clone(), prepared)),
        );
    }

    /// The parse the sanitiser's cache spares, asked first: whether a message is bulk decides how
    /// it renders.
    pub fn suits_reader_view(&mut self, scope: &str, body: &MessageBody) -> bool {
        if let Some((judged, verdict)) = self.judged.get(scope)
            && judged == body
        {
            return *verdict;
        }
        let verdict = match self.offered.get(scope) {
            Some(prepared) if prepared.body == *body => prepared.verdict,
            _ => suits_reader_view(body),
        };
        self.judged
            .insert(scope.to_owned(), (body.clone(), verdict));
        verdict
    }

    pub fn keep_only<'a>(&mut self, scopes: impl IntoIterator<Item = &'a str>) {
        let keep: std::collections::HashSet<&str> = scopes.into_iter().collect();
        self.held.retain(|scope, _| keep.contains(scope.as_str()));
        self.judged.retain(|scope, _| keep.contains(scope.as_str()));
    }

    pub fn len(&self) -> usize {
        self.held.len()
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }
}

#[cfg(test)]
mod render_cache_tests {
    use super::*;

    fn body(html: &str) -> MessageBody {
        MessageBody {
            text: None,
            html: Some(html.to_owned()),
        }
    }

    #[test]
    fn a_message_prepared_ahead_is_shown_without_parsing_it_again() {
        let newsletter = body("<table><tr><td>Weekly digest</td></tr></table>");
        let prepared = prepare("9", &newsletter, RemoteImages::Blocked);
        let expected = prepared.rendered.clone();
        let rendering = prepared.rendering;

        let mut cache = RenderCache::default();
        cache.offer(vec![prepared]);
        cache.keep_only(["1"]);

        let (judged, sanitised) = (
            crate::test_support::bulk_judged(),
            crate::test_support::bodies_sanitised(),
        );
        let verdict = cache.suits_reader_view("9", &newsletter);
        let drawn = cache.render("9", &newsletter, RemoteImages::Blocked, rendering);
        assert_eq!(drawn, expected);
        assert_eq!(verdict, rendering == Rendering::Reader);
        assert_eq!(
            crate::test_support::bulk_judged() - judged,
            0,
            "judged again"
        );
        assert_eq!(
            crate::test_support::bodies_sanitised() - sanitised,
            0,
            "sanitised again"
        );

        cache.offer(vec![prepare("9", &newsletter, RemoteImages::Blocked)]);
        let reply = body("<p>A reply.</p>");
        let sanitised = crate::test_support::bodies_sanitised();
        cache.render("9", &reply, RemoteImages::Blocked, Rendering::Original);
        assert_eq!(crate::test_support::bodies_sanitised() - sanitised, 1);
    }

    #[test]
    fn bodies_offered_one_at_a_time_are_all_kept_until_drawn() {
        let first = body("<p>One.</p>");
        let second = body("<p>Two.</p>");
        let mut cache = RenderCache::default();
        cache.offer(vec![prepare("1", &first, RemoteImages::Blocked)]);
        cache.offer(vec![prepare("2", &second, RemoteImages::Blocked)]);

        let sanitised = crate::test_support::bodies_sanitised();
        cache.render("1", &first, RemoteImages::Blocked, Rendering::Original);
        cache.render("2", &second, RemoteImages::Blocked, Rendering::Original);
        assert_eq!(
            crate::test_support::bodies_sanitised() - sanitised,
            0,
            "an earlier offer was thrown away by a later one"
        );
    }

    #[test]
    fn a_message_is_judged_for_reader_view_once_until_its_body_changes() {
        let mut cache = RenderCache::default();
        let newsletter = body("<table><tr><td>Weekly digest</td></tr></table>");
        let before = crate::test_support::bulk_judged();
        let first = cache.suits_reader_view("7", &newsletter);
        let again = cache.suits_reader_view("7", &newsletter);
        assert_eq!(first, again);
        assert_eq!(
            crate::test_support::bulk_judged() - before,
            1,
            "judged twice"
        );

        let changed = body("<p>A reply, in full.</p>");
        cache.suits_reader_view("7", &changed);
        assert_eq!(
            crate::test_support::bulk_judged() - before,
            2,
            "a new body is a new question"
        );
    }

    #[test]
    fn a_message_is_sanitised_once_until_something_about_it_changes() {
        let mut cache = RenderCache::default();
        let before = crate::test_support::bodies_sanitised();
        let first = cache.render(
            "1",
            &body("<p>hello</p>"),
            RemoteImages::Blocked,
            Rendering::Original,
        );
        let again = cache.render(
            "1",
            &body("<p>hello</p>"),
            RemoteImages::Blocked,
            Rendering::Original,
        );
        assert_eq!(first, again);
        assert_eq!(
            crate::test_support::bodies_sanitised() - before,
            1,
            "the same message under the same decisions was sanitised twice"
        );

        cache.render(
            "1",
            &body("<p>hello, edited</p>"),
            RemoteImages::Blocked,
            Rendering::Original,
        );
        cache.render(
            "1",
            &body("<p>hello, edited</p>"),
            RemoteImages::Allowed,
            Rendering::Original,
        );
        cache.render(
            "1",
            &body("<p>hello, edited</p>"),
            RemoteImages::Allowed,
            Rendering::Reader,
        );
        assert_eq!(
            crate::test_support::bodies_sanitised() - before,
            4,
            "a new body, a new image policy and a new rendering are each drawn anew"
        );
    }

    #[test]
    fn the_cache_keeps_only_the_messages_it_was_last_told_about() {
        let mut cache = RenderCache::default();
        cache.render(
            "1",
            &body("<p>a</p>"),
            RemoteImages::Blocked,
            Rendering::Original,
        );
        cache.render(
            "2",
            &body("<p>b</p>"),
            RemoteImages::Blocked,
            Rendering::Original,
        );
        cache.keep_only(["2"]);
        assert_eq!(cache.len(), 1, "a message that left the thread is not held");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plate_for_an_unfetched_body_follows_the_link() {
        use postio_core::{ConnectionState, FailureReason};
        for (link, reaches) in [
            (None, true),
            (Some(ConnectionState::Online), true),
            (Some(ConnectionState::Connecting), false),
            (Some(ConnectionState::Offline), false),
            (
                Some(ConnectionState::Failing {
                    reason: FailureReason::Auth,
                }),
                false,
            ),
        ] {
            assert_eq!(reaches_server(link), reaches, "{link:?}");
            let plate = unfetched(false, !reaches_server(link));
            assert_eq!(
                plate,
                if reaches {
                    Absent::Partial
                } else {
                    Absent::Offline
                },
                "{link:?}"
            );
        }
        assert_eq!(
            unfetched(true, true),
            Absent::Failed,
            "a refusal is the fact"
        );
    }

    #[test]
    fn a_body_that_did_not_decode_is_caveated_and_a_clean_one_is_not() {
        assert_eq!(decode_caveat(true), Some(DECODE_CAVEAT));
        assert_eq!(decode_caveat(false), None);
        assert!(
            DECODE_CAVEAT.contains("could not be decoded"),
            "the caveat has to say what is wrong with the words below it"
        );
    }

    /// Two constants, one written into HTML and one into CSS; renaming one leaves every scoping
    /// test passing against a selector that names nothing (#1326).
    #[test]
    fn the_container_carries_what_the_stylesheet_selector_matches() {
        let container = contain_body_in("<p>hi</p>", Some("7"));
        let selector = sanitize::message_selector(Some("7"));

        let class = selector
            .split(['[', ':'])
            .next()
            .expect("the selector names a class")
            .trim_start_matches('.');
        assert!(
            container.contains(&format!(r#"class="{class}""#)),
            "the selector matches .{class} and the container is {container}"
        );
        let attribute = selector
            .split_once('[')
            .and_then(|(_, rest)| rest.strip_suffix(']'))
            .expect("the selector names the message");
        let (name, value) = attribute.split_once('=').expect("as name=\"value\"");
        assert!(
            container.contains(&format!("{name}={value}")),
            "the selector matches [{attribute}] and the container is {container}"
        );
    }

    #[test]
    fn a_senders_stylesheet_reaches_the_document() {
        let body = postio_model::message::MessageBody {
            text: None,
            html: Some("<style>p { color: rgb(1, 2, 3) }</style><p>hi</p>".to_owned()),
        };
        let rendered = body_html_in(&body, RemoteImages::Blocked, Rendering::Original, Some("7"));
        assert!(
            rendered.styles.contains("rgb(1, 2, 3)"),
            "the sanitizer kept it: {}",
            rendered.styles
        );

        let document = document_for(
            &rendered.html,
            &rendered.styles,
            RemoteImages::Blocked,
            Sheet::Theme,
        );
        assert!(
            document.contains("rgb(1, 2, 3)"),
            "and the document carries it: {document}"
        );
        assert!(
            document.contains(&sanitize::message_selector(Some("7"))),
            "still scoped when it gets there: {document}"
        );
    }

    /// Reader view drops the sender's stylesheet (#1326): a `<style>` would put the layout back
    /// halfway.
    #[test]
    fn reader_view_keeps_no_stylesheet_of_the_senders() {
        let html = "<style>p { color: rgb(1, 2, 3) }</style><p>hi</p>";
        let body = postio_model::message::MessageBody {
            text: None,
            html: Some(html.to_owned()),
        };

        let reduced = body_html_in(&body, RemoteImages::Blocked, Rendering::Reader, Some("7"));
        assert_eq!(reduced.rendering, Rendering::Reader);
        assert!(
            reduced.styles.is_empty(),
            "reader view strips the sender's styling and then hands back \
             their stylesheet: {}",
            reduced.styles
        );

        let original = body_html_in(&body, RemoteImages::Blocked, Rendering::Original, Some("7"));
        assert!(
            original.styles.contains("rgb(1, 2, 3)"),
            "the original must still carry it, or the assertion above passes \
             because nothing is ever carried: {}",
            original.styles
        );
    }

    #[test]
    fn a_stylesheet_cannot_close_its_own_element() {
        let body = postio_model::message::MessageBody {
            text: None,
            html: Some("<style>p { color: red }</style:x><p>hi</p>".to_owned()),
        };
        let rendered = body_html_in(&body, RemoteImages::Blocked, Rendering::Original, Some("7"));
        let document = document_for(
            &rendered.html,
            &rendered.styles,
            RemoteImages::Blocked,
            Sheet::Theme,
        );
        assert!(
            !document.contains("</style><script") && !document.contains("<script"),
            "{document}"
        );
    }

    /// `fold_html_quotes` has its own tests; this asserts it is called from `body_html_in`.
    #[test]
    fn a_quote_reaches_the_document_folded() {
        let body = postio_model::message::MessageBody {
            text: None,
            html: Some("<p>my reply</p><blockquote><p>what they wrote</p></blockquote>".to_owned()),
        };
        let rendered = body_html_in(&body, RemoteImages::Blocked, Rendering::Original, None);

        assert!(
            rendered.html.contains("<details"),
            "the quote did not fold, so `fold_html_quotes` is no longer \
             reached from the path that draws a message: {}",
            rendered.html
        );
        let before_fold = rendered
            .html
            .split("<details")
            .next()
            .unwrap_or_default()
            .to_owned();
        assert!(
            before_fold.contains("my reply"),
            "the prose folded away with the quote: {}",
            rendered.html
        );
        assert!(
            !rendered.html.contains("<script"),
            "folding must not need script: {}",
            rendered.html
        );
    }

    #[test]
    fn a_single_messages_quote_folds_carry_ids_and_line_counts() {
        let document_of = |body: MessageBody| {
            let rendered = body_html(&body, RemoteImages::Blocked, Rendering::Original);
            document_for(
                &rendered.html,
                &rendered.styles,
                RemoteImages::Blocked,
                Sheet::Theme,
            )
        };
        let html = document_of(MessageBody {
            text: None,
            html: Some(
                "<p>my reply</p><blockquote><p>one</p><p>two</p></blockquote>\
                 <p>and more</p><blockquote><p>three</p></blockquote>"
                    .to_owned(),
            ),
        });
        for fold in [
            r#"<details class="postio-quote" data-postio-fold="q0"><summary>2 quoted lines</summary>"#,
            r#"<details class="postio-quote" data-postio-fold="q1"><summary>1 quoted line</summary>"#,
        ] {
            assert!(html.contains(fold), "{fold} is not in {html}");
        }
        let plain = document_of(MessageBody {
            text: Some("Fine by me.\n> your words\n> and more of them\n".to_owned()),
            html: None,
        });
        let fold = r#"<details class="postio-quote" data-postio-fold="q0"><summary>2 quoted lines</summary>"#;
        assert!(plain.contains(fold), "{fold} is not in {plain}");
    }

    /// `style-src` must never name a host (#1383): `@import url(https://tracker/…)` is a request
    /// made when the stylesheet parses, an open-rate beacon with no `<img>`. Unreachable today;
    /// this locks it for when `<style>` blocks are admitted.
    #[test]
    fn a_stylesheet_has_nowhere_to_fetch_from() {
        for remote in [RemoteImages::Blocked, RemoteImages::Allowed] {
            let policy = content_security_policy(remote);
            let style_src = policy
                .split(';')
                .map(str::trim)
                .find(|directive| directive.starts_with("style-src"))
                .unwrap_or_else(|| panic!("no style-src at all in {policy:?}"));
            assert_eq!(
                style_src, "style-src 'unsafe-inline'",
                "style-src must permit inline declarations and name no source \
                 to fetch from -- with {remote:?} images it read {style_src:?}"
            );
        }
    }

    #[test]
    fn images_and_fonts_still_have_their_sources() {
        let blocked = content_security_policy(RemoteImages::Blocked);
        assert!(
            !blocked.contains("https:"),
            "images are blocked until the reader says otherwise: {blocked}"
        );
        let allowed = content_security_policy(RemoteImages::Allowed);
        assert!(
            allowed.contains("img-src") && allowed.contains("https:"),
            "and they arrive once consented, or consent means nothing: {allowed}"
        );
        assert!(
            allowed.contains(&format!("font-src {FONT_SCHEME}:")),
            "the reader's own fonts are served, not inlined (ADR 0023): {allowed}"
        );
    }

    /// A document must not carry identical bulk (#1341): ~1.21 MB of base64 fonts were re-decoded
    /// on every switch (#749) until ADR 0023 moved them behind `postio-font:`. Re-inlining looks
    /// like a tidy-up and keeps every other test green.
    #[test]
    fn a_document_carries_font_references_and_not_font_bytes() {
        let before = crate::test_support::largest_document();
        let document = document_for("<p>hi</p>", "", RemoteImages::Blocked, Sheet::Theme);

        assert!(
            document.contains("@font-face"),
            "the document no longer names any face, so rendered text will not \
             inherit Postio typography at all"
        );
        assert!(
            document.contains(&format!("url({FONT_SCHEME}:")),
            "the faces are named but not served over {FONT_SCHEME}:"
        );

        assert!(
            !document.contains("data:font"),
            "a font is inlined as a data URI again — this is #768 returning"
        );
        assert!(
            !document.contains(";base64,"),
            "something is base64-encoded into every document, which is the \
             shape of the 1.21 MB #749 measured"
        );

        // A bound, not an exact size: a one-paragraph document is ~16 KB and the regression was
        // ~1.21 MB.
        const CEILING: usize = 128 * 1024;
        assert!(
            document.len() < CEILING,
            "a one-paragraph document is {} bytes, over the {CEILING}-byte \
             ceiling. Something identical for every message is being embedded \
             in each of them",
            document.len()
        );

        assert!(
            crate::test_support::largest_document() > before
                || crate::test_support::largest_document() >= document.len() as u64,
            "the counter did not see this document, so it cannot notice the \
             regression it exists for"
        );
    }

    use postio_model::message::MessageBody;

    #[test]
    fn only_the_original_of_something_reader_view_offered_gets_the_senders_sheet() {
        assert_eq!(sheet_for(Rendering::Original, true), Sheet::Senders);
        assert_eq!(sheet_for(Rendering::Original, false), Sheet::Theme);
        assert_eq!(sheet_for(Rendering::Reader, true), Sheet::Theme);
        assert_eq!(sheet_for(Rendering::Reader, false), Sheet::Theme);
    }

    #[test]
    fn an_ordinary_document_carries_no_sheet_of_its_own() {
        let document = document_for("<p>hi</p>", "", RemoteImages::Blocked, Sheet::Theme);
        assert!(
            !document.contains(SENDERS_SHEET_CLASS),
            "the theme document must not carry the sender's sheet: {document}"
        );
    }

    #[test]
    fn the_senders_sheet_lights_the_body_box_and_leaves_the_chrome_alone() {
        let document = document_for("<p>hi</p>", "", RemoteImages::Blocked, Sheet::Senders);
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

    #[test]
    fn the_senders_sheet_is_the_generated_light_palette_and_not_a_second_copy() {
        let document = document_for("<p>hi</p>", "", RemoteImages::Blocked, Sheet::Senders);
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
        assert!(
            sheet.contains("--r-accent:"),
            "the accent is what `a` resolves against: {sheet}"
        );
    }

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
        assert_eq!(
            drawn(&body).html,
            r#"<div class="postio-original"><p>rich</p></div>"#
        );
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
        assert_eq!(
            drawn(&body).held_back,
            HeldBack {
                remote_images: 1,
                trackers: 0
            }
        );
    }

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

        let plain = MessageBody {
            text: Some("hello".to_owned()),
            html: None,
        };
        assert!(!suits_reader_view(&plain));
    }

    fn shipping_notice() -> String {
        "Your order has shipped.\n\
         \n\
         tracking: EXTEST0042199317\n\
         item: Type-C Upgrade Small Board Replacement x 1\n\
         ship to: 1 Example Way, Springfield\n\
         \n\
         Follow the parcel from your orders page.\n"
            .to_owned()
    }

    #[test]
    fn reader_view_lifts_the_facts_above_the_body_copy() {
        let body = MessageBody {
            text: Some(shipping_notice()),
            html: Some(campaign()),
        };
        let rendered = body_html(&body, RemoteImages::Blocked, Rendering::Reader);

        let block = rendered
            .html
            .find(FACTS_CLASS)
            .expect("the facts block is in the markup");
        let copy = rendered
            .html
            .find("Follow the parcel")
            .expect("and so is the body copy");
        assert!(
            block < copy,
            "the canvas draws the facts above the body, not below it: {}",
            rendered.html
        );
        for expected in [
            "tracking",
            "EXTEST0042199317",
            "ship to",
            "1 Example Way, Springfield",
        ] {
            assert!(
                rendered.html.contains(expected),
                "`{expected}` is missing from {}",
                rendered.html
            );
        }

        assert_eq!(
            rendered.html.matches("EXTEST0042199317").count(),
            1,
            "the tracking number is drawn twice: {}",
            rendered.html
        );
        assert!(
            !rendered.html.contains("tracking:"),
            "the lifted line is still in the body copy: {}",
            rendered.html
        );
        assert!(
            rendered.html.contains("Your order has shipped."),
            "the prose around the block survived: {}",
            rendered.html
        );
    }

    #[test]
    fn a_plain_part_with_no_block_grows_nothing() {
        let body = MessageBody {
            text: Some("Hi Ada,\n\nFriday works for me.\n".to_owned()),
            html: Some(campaign()),
        };
        let rendered = body_html(&body, RemoteImages::Blocked, Rendering::Reader);
        assert!(
            !rendered.html.contains(FACTS_CLASS),
            "ordinary prose grew a table: {}",
            rendered.html
        );
    }

    #[test]
    fn nothing_is_lifted_out_of_html() {
        let body = MessageBody {
            text: None,
            html: Some(
                "<table><tr><td><table>\
                 <tr><td>tracking</td><td>EXTEST0042199317</td></tr>\
                 <tr><td>item</td><td>One Small Board</td></tr>\
                 <tr><td>ship to</td><td>1 Example Way, Springfield</td></tr>\
                 </table></td></tr></table>"
                    .to_owned(),
            ),
        };
        let rendered = body_html(&body, RemoteImages::Blocked, Rendering::Reader);
        assert!(
            !rendered.html.contains(FACTS_CLASS),
            "a block was lifted out of markup: {}",
            rendered.html
        );
    }

    #[test]
    fn the_original_rendering_has_no_facts_block() {
        let body = MessageBody {
            text: Some(shipping_notice()),
            html: None,
        };
        let rendered = body_html(&body, RemoteImages::Blocked, Rendering::Original);
        assert!(!rendered.html.contains(FACTS_CLASS));
    }

    #[test]
    fn a_blocked_image_is_still_reported_when_the_markup_is_reduced() {
        let mut html = campaign();
        html.push_str(r#"<img src="https://tracker.example.org/o.gif">"#);
        let body = MessageBody {
            text: None,
            html: Some(html),
        };
        let rendered = body_html(&body, RemoteImages::Blocked, Rendering::Reader);
        assert_eq!(rendered.held_back.total(), 1);
    }

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

        for name in [
            "",
            "Barlow-Regular",
            "../fonts/barlow/Barlow-Regular.ttf",
            "/etc/passwd",
        ] {
            assert!(font_bytes(name).is_none(), "{name} resolved to something");
        }
    }

    #[test]
    fn the_document_is_proportional_to_the_message_not_to_the_font_catalogue() {
        let document = document_for("<p>hi</p>", "", RemoteImages::Blocked, Sheet::Theme);
        assert!(
            !document.contains("data:font/"),
            "the faces are still travelling with the document"
        );
        assert!(
            document.len() < 64 * 1024,
            "a one-line message renders a {} byte document",
            document.len()
        );
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
            Absent::Failed,
        ];
        let texts: Vec<String> = states.iter().map(|state| absent_html(*state)).collect();
        for (index, text) in texts.iter().enumerate() {
            for other in texts.iter().skip(index + 1) {
                assert_ne!(text, other);
            }
        }
        assert!(absent_html(Absent::Offline).contains("<kbd>F5</kbd>"));
        assert!(absent_html(Absent::Missing).contains("<kbd>F5</kbd>"));
        assert!(!absent_html(Absent::ForeignDraft).contains("<kbd>"));
        let failed = absent_html(Absent::Failed);
        assert!(failed.contains("could not be downloaded"), "{failed}");
        assert!(!failed.contains("Downloading"), "{failed}");
    }

    #[test]
    fn an_absence_plate_announces_itself() {
        for state in [
            Absent::Partial,
            Absent::Offline,
            Absent::Missing,
            Absent::Empty,
            Absent::ForeignDraft,
            Absent::Failed,
        ] {
            let html = absent_html(state);
            assert!(
                html.contains("role=\"status\""),
                "{state:?} draws a plate a screen reader will not announce"
            );
            assert!(
                html.contains("aria-live=\"polite\""),
                "{state:?} would interrupt rather than wait its turn"
            );
        }
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
        assert_eq!(HeldBack::default().summary(), "");
    }
}

#[cfg(test)]
mod reader_view_prefers_markup_over_its_own_flattening {
    use super::*;
    use postio_model::message::MessageBody;

    #[test]
    fn markdown_in_the_plain_part_does_not_reach_the_document() {
        let body = MessageBody {
            text: Some(
                "Assignment.[Bridges Practice #1](https://app.example.test/a/849) 3:59 pm"
                    .to_owned(),
            ),
            html: Some(
                "<html><body><table><tr><td><h1>Daily summary</h1>\
                 <p>Assignment: <a href=\"https://app.example.test/a/849\">\
                 Bridges Practice #1</a> 3:59 pm</p></td></tr></table></body></html>"
                    .to_owned(),
            ),
        };

        let rendered = body_html_in(&body, RemoteImages::Blocked, Rendering::Reader, None);

        assert!(
            !rendered.html.contains("](https://"),
            "the converter's link syntax must not reach the reader: {}",
            rendered.html
        );
        assert!(
            rendered.html.contains("Bridges Practice #1"),
            "and the text it named is still there: {}",
            rendered.html
        );
        assert!(
            rendered
                .html
                .contains("href=\"https://app.example.test/a/849\""),
            "as a link, which is what the markup had all along: {}",
            rendered.html
        );
    }

    #[test]
    fn prose_still_takes_the_plain_part() {
        let body = MessageBody {
            text: Some("Follow the parcel. Only the plain part says this.".to_owned()),
            html: Some("<html><body><p>Only the markup says this.</p></body></html>".to_owned()),
        };

        let rendered = body_html_in(&body, RemoteImages::Blocked, Rendering::Reader, None);

        assert!(
            rendered.html.contains("Only the plain part says this."),
            "prose keeps the plain part, which is the only path `lift` runs \
             on and so the only one with a facts block: {}",
            rendered.html
        );
        assert!(
            !rendered.html.contains("Only the markup says this."),
            "and does not also draw the markup: {}",
            rendered.html
        );
    }
    #[test]
    fn paging_stops_at_both_ends_rather_than_wrapping() {
        assert_eq!(page_after(0, false), 0);
        assert_eq!(page_after(SCROLL_MARKERS - 1, true), SCROLL_MARKERS - 1);
    }

    #[test]
    fn a_page_turn_moves_one_marker() {
        assert_eq!(page_after(0, true), 1);
        assert_eq!(page_after(7, true), 8);
        assert_eq!(page_after(7, false), 6);
    }

    #[test]
    fn the_fragment_names_the_anchor_the_document_laid_down() {
        let markers = scroll_markers();
        assert!(markers.contains(&format!("id=\"{}\"", page_fragment(0))));
        assert!(markers.contains(&format!("id=\"{}\"", page_fragment(SCROLL_MARKERS - 1))));
    }
}

#[cfg(test)]
mod warming_tests {
    use super::*;

    #[test]
    fn the_senders_canvas_is_emitted_around_their_content() {
        let body = MessageBody {
            html: Some(
                r##"<head><meta name="color-scheme" content="light dark"></head><body bgcolor="#ffffff" text="#222"><p>Hi</p></body>"##
                    .to_owned(),
            ),
            ..MessageBody::default()
        };
        let rendered = body_html_in(&body, RemoteImages::Blocked, Rendering::Original, Some("7"));
        assert!(
            rendered.html.starts_with(r#"<div class="postio-canvas""#),
            "{}",
            rendered.html
        );
        assert!(
            rendered.html.contains("background-color: #ffffff"),
            "{}",
            rendered.html
        );
        assert!(rendered.html.contains("color: #222"), "{}", rendered.html);
        assert!(
            rendered
                .html
                .contains(r#"data-postio-color-scheme="light dark""#),
            "{}",
            rendered.html
        );
        let plain = MessageBody {
            html: Some("<p>Hi</p>".to_owned()),
            ..MessageBody::default()
        };
        let plain = body_html_in(
            &plain,
            RemoteImages::Blocked,
            Rendering::Original,
            Some("7"),
        );
        assert!(
            !plain.html.contains("postio-canvas"),
            "no canvas, no wrapper: {}",
            plain.html
        );
    }

    #[test]
    fn bulk_mail_opens_in_its_original_layout() {
        let newsletter = postio_model::test_corpus::load("html-newsletter")
            .parse()
            .body;
        assert!(
            suits_reader_view(&newsletter),
            "the fixture must read as bulk for this to mean anything"
        );
        assert_eq!(
            prepare("7", &newsletter, RemoteImages::Blocked).rendering,
            Rendering::Original
        );
        assert_eq!(opening_rendering(), Rendering::Original);
    }

    #[test]
    fn original_markup_is_set_apart_from_postios_typography() {
        let html = MessageBody {
            html: Some("<p>Hi</p>".to_owned()),
            ..MessageBody::default()
        };
        let original = body_html_in(&html, RemoteImages::Blocked, Rendering::Original, Some("7"));
        assert!(
            original.html.contains(r#"<div class="postio-original">"#),
            "{}",
            original.html
        );
        let reduced = body_html_in(&html, RemoteImages::Blocked, Rendering::Reader, Some("7"));
        assert!(
            !reduced.html.contains("postio-original"),
            "{}",
            reduced.html
        );
        let text = MessageBody {
            text: Some("Hi".to_owned()),
            ..MessageBody::default()
        };
        let plain = body_html_in(&text, RemoteImages::Blocked, Rendering::Original, Some("7"));
        assert!(!plain.html.contains("postio-original"), "{}", plain.html);
        let css = include_str!("../../data/reader.css");
        for reverted in [
            ".postio-original *",
            ".postio-original p",
            ".postio-original a",
            ".postio-original blockquote",
        ] {
            assert!(
                css.contains(reverted),
                "reader.css must revert Postio's type under {reverted}"
            );
        }
    }
}

#[cfg(test)]
mod over_cap_tests {
    use super::*;

    fn deep() -> String {
        format!("{}x{}", "<div>".repeat(2_000), "</div>".repeat(2_000))
    }

    #[test]
    fn an_over_cap_body_shows_its_plain_text_alternative() {
        let body = MessageBody {
            text: Some("the plain alternative".to_owned()),
            html: Some(deep()),
        };
        for rendering in [Rendering::Original, Rendering::Reader] {
            let rendered = body_html_in(&body, RemoteImages::Blocked, rendering, None);
            assert!(
                rendered.html.contains("the plain alternative"),
                "{rendering:?} showed {:?}",
                rendered.html
            );
            if rendering == Rendering::Original {
                assert_eq!(rendered.over_cap, Some(postio_body::Cap::Depth));
            }
        }
    }

    #[test]
    fn an_over_cap_body_with_no_alternative_is_not_blank() {
        let body = MessageBody {
            text: None,
            html: Some(deep()),
        };
        let rendered = body_html_in(&body, RemoteImages::Blocked, Rendering::Original, None);
        assert!(!rendered.html.trim().is_empty(), "the message opened blank");
        assert_eq!(rendered.over_cap, Some(postio_body::Cap::Depth));
    }
}

#[cfg(test)]
mod no_webkit_tests {
    #[test]
    fn the_reader_stylesheets_name_no_webkit_extension() {
        for (name, css) in [
            ("reader.css", include_str!("../../data/reader.css")),
            ("thread.css", include_str!("../../data/thread.css")),
        ] {
            assert!(
                !css.contains("-webkit-"),
                "{name} still has a -webkit- rule"
            );
        }
    }
}

#[cfg(test)]
mod treatment_tests {

    use super::*;

    fn html(markup: &str) -> MessageBody {
        MessageBody {
            text: None,
            html: Some(markup.to_owned()),
        }
    }

    const NEWSLETTER: &str = "<html><head><style>body { color: #222 } \
        @media (prefers-color-scheme: dark) { p { color: #eee } }</style></head>\
        <body bgcolor=\"#f6f1e7\"><table width=\"640\"><tr><td><p>Issue 48</p></td></tr></table></body></html>";

    const LETTER: &str = "<p style=\"color:black;font-family:Calibri\">Hi everyone,</p>";

    fn treated(shown: Treatment, classified: Treatment, trigger: Option<Trigger>) -> Treated {
        Treated {
            shown,
            classified,
            trigger,
            html: true,
        }
    }

    #[test]
    fn the_line_names_app_colours_and_offers_the_original() {
        let words = render_mode_words(
            treated(Treatment::AppColours, Treatment::AppColours, None),
            None,
        )
        .expect("an HTML body gets a line");
        assert_eq!(words.title, "App colours");
        assert_eq!(words.detail, "sender colours and fonts removed");
        assert_eq!(words.action, "Show original");
        assert!(!words.offer_always, "nothing was chosen to remember");
    }

    #[test]
    fn the_line_says_why_the_rule_chose_paper() {
        let background = render_mode_words(
            treated(
                Treatment::Paper,
                Treatment::Paper,
                Some(Trigger::PageBackground),
            ),
            None,
        )
        .expect("a line");
        assert_eq!(background.title, "Original layout, on paper");
        assert_eq!(background.detail, "this message sets its own background");
        assert_eq!(background.action, "Use app colours");
        let layout = render_mode_words(
            treated(Treatment::Paper, Treatment::Paper, Some(Trigger::WideTable)),
            None,
        )
        .expect("a line");
        assert_eq!(layout.detail, "this message lays out its own page");
    }

    #[test]
    fn a_choice_against_the_rule_is_named_and_can_be_remembered() {
        let chosen =
            render_mode_words(treated(Treatment::Paper, Treatment::AppColours, None), None)
                .expect("a line");
        assert_eq!(chosen.title, "Original colours, on paper");
        assert_eq!(chosen.detail, "as the sender styled it");
        assert!(chosen.offer_always);
        let remembered = render_mode_words(
            treated(Treatment::Paper, Treatment::AppColours, None),
            Some(Treatment::Paper),
        )
        .expect("a line");
        assert_eq!(remembered.detail, "always for this sender");
        assert!(!remembered.offer_always, "already remembered");
        let back = render_mode_words(
            treated(Treatment::AppColours, Treatment::AppColours, None),
            Some(Treatment::Paper),
        )
        .expect("a line");
        assert!(back.offer_always);
    }

    #[test]
    fn a_page_keeps_the_margins_its_sender_set_or_a_browsers() {
        let draw =
            |page: &str| body_html_treated(&html(page), RemoteImages::Blocked, None, None).html;
        let defaulted = draw("<body style=\"background:#fff4e0\"><p>A sale on now</p></body>");
        assert!(defaulted.contains(PAGE_MARGIN_CLASS), "{defaulted}");
        let own = draw("<body style=\"margin:0;background:#fff4e0\"><p>A sale</p></body>");
        assert!(!own.contains(PAGE_MARGIN_CLASS), "{own}");
        assert!(!own.contains(LETTER_CLASS), "{own}");
    }

    #[test]
    fn plain_text_has_no_line() {
        assert_eq!(render_mode_words(Treated::TEXT, None), None);
    }

    #[test]
    fn a_prepared_body_answers_every_choice_as_drawing_it_would() {
        let text = MessageBody {
            text: Some("Hello.\n".to_owned()),
            html: None,
        };
        for body in [html(NEWSLETTER), html(LETTER), text] {
            let prepared = prepare_treated(&body, RemoteImages::Blocked);
            for chosen in [None, Some(Treatment::AppColours), Some(Treatment::Paper)] {
                assert_eq!(
                    prepared.treated_for(&body, RemoteImages::Blocked, chosen),
                    Some(&body_html_treated(
                        &body,
                        RemoteImages::Blocked,
                        chosen,
                        None
                    )),
                    "{chosen:?}"
                );
            }
            assert!(
                prepared
                    .treated_for(&body, RemoteImages::Allowed, None)
                    .is_none(),
                "served under a policy it was not prepared for"
            );
        }
    }

    #[test]
    fn plain_text_is_app_colours_with_nothing_to_switch_to() {
        let body = MessageBody {
            text: Some("Hello.\n".to_owned()),
            html: None,
        };
        let rendered = body_html_treated(&body, RemoteImages::Blocked, None, None);
        assert_eq!(rendered.treated, Some(Treated::TEXT));
        assert!(rendered.html.contains("Hello."));
    }

    #[test]
    fn mail_that_paints_its_page_is_drawn_as_sent_on_paper() {
        let rendered = body_html_treated(&html(NEWSLETTER), RemoteImages::Blocked, None, None);
        let treated = rendered.treated.expect("treated");
        assert_eq!(treated.shown, Treatment::Paper);
        assert_eq!(treated.classified, Treatment::Paper);
        assert_eq!(treated.trigger, Some(Trigger::PageBackground));
        assert!(rendered.html.contains(ORIGINAL_CLASS), "{}", rendered.html);
        assert!(
            !rendered.html.contains(LETTER_CLASS),
            "the rule's paper got a letter's margins"
        );
        assert!(rendered.html.contains("postio-canvas"), "the page was lost");
        assert!(rendered.styles.contains("#222"), "{}", rendered.styles);
        assert!(
            !rendered.styles.contains("#eee"),
            "the sender's dark design reached the white sheet: {}",
            rendered.styles
        );
    }

    #[test]
    fn correspondence_is_drawn_in_app_colours() {
        let rendered = body_html_treated(&html(LETTER), RemoteImages::Blocked, None, None);
        let treated = rendered.treated.expect("treated");
        assert_eq!(treated.shown, Treatment::AppColours);
        assert!(treated.html);
        assert!(
            rendered.html.contains(APP_COLOURS_CLASS),
            "{}",
            rendered.html
        );
        assert!(!rendered.html.contains("Calibri") && !rendered.html.contains("black"));
    }

    #[test]
    fn a_choice_wins_over_the_rule_and_a_chosen_sheet_gives_a_letter_margins() {
        let paper = body_html_treated(
            &html(LETTER),
            RemoteImages::Blocked,
            Some(Treatment::Paper),
            None,
        );
        let treated = paper.treated.expect("treated");
        assert_eq!(
            (treated.shown, treated.classified),
            (Treatment::Paper, Treatment::AppColours)
        );
        assert!(paper.html.contains(LETTER_CLASS), "{}", paper.html);
        assert!(
            paper.html.contains("Calibri"),
            "paper is as sent: {}",
            paper.html
        );

        let app = body_html_treated(
            &html(NEWSLETTER),
            RemoteImages::Blocked,
            Some(Treatment::AppColours),
            None,
        );
        assert_eq!(app.treated.map(|t| t.shown), Some(Treatment::AppColours));
        assert!(!app.html.contains("postio-canvas"), "{}", app.html);
        assert!(!app.html.contains("f6f1e7"), "{}", app.html);
    }

    #[test]
    fn the_container_names_its_treatment_and_the_sheet_styles_it() {
        for treatment in [Treatment::AppColours, Treatment::Paper] {
            let document = document_for_treated("<p>x</p>", "", RemoteImages::Blocked, treatment);
            let stamp = format!(
                "class=\"{}\" {TREATMENT_ATTRIBUTE}=\"{}\"",
                sanitize::BODY_CLASS,
                treatment.attribute_value()
            );
            assert!(document.contains(&stamp), "{document}");
            assert!(
                document.contains(&format!(
                    "[{TREATMENT_ATTRIBUTE}=\"{}\"]",
                    treatment.attribute_value()
                )),
                "no rule styles {treatment:?}"
            );
        }
    }
}
