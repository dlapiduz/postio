//! Sanitizing a message body before the `WebView`: defense in depth, since the primary control is
//! that the reader gives it no live path to a remote host.
//! `ammonia`'s defaults remove `<script>` and `on*` handlers; a sender's CSS is kept (`style`
//! through `contain_declarations`, `<style>` scoped by `crate::styles`).
//! `<iframe>`, `<object>`, `<embed>`, `<svg>`, `<math>`, `<noscript>` and `<title>` go with their
//! contents (a `<noscript>` fallback parses as markup with scripting off and would load a tracking
//! pixel). `cid:` becomes `CID_SCHEME` and remote `src` is dropped unless `RemoteImages::Allowed`.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use ammonia::Builder;
use html5ever::driver::ParseOpts;
use html5ever::parse_document;
use html5ever::serialize::{SerializeOpts, TraversalScope, serialize};
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{Handle, NodeData, RcDom, SerializableHandle};

/// The class every sender's content is wrapped in (`contain_body`). Postio's chrome sits outside
/// it, so a scoped rule cannot hide the blocked-images notice.
pub const BODY_CLASS: &str = "postio-body";

/// Two constants with `message_selector` because one goes into HTML and the other into CSS; a test
/// in `postio_ui::reader::document` holds them together.
pub const MESSAGE_ATTRIBUTE: &str = "data-postio-message";

/// `None` is the single-message reader. A scope is escaped, not trusted: a quote in an id would
/// close the attribute selector and free every rule after it.
pub fn message_selector(scope: Option<&str>) -> String {
    match scope {
        Some(scope) => format!(
            ".{BODY_CLASS}[{MESSAGE_ATTRIBUTE}=\"{}\"]",
            escape_css_string(scope)
        ),
        None => format!(".{BODY_CLASS}"),
    }
}

/// Per message, so two messages in one conversation cannot share an id. Inside the `postio-`
/// namespace, which a sender may not write (`is_postio_name`).
pub fn sender_id_prefix(scope: Option<&str>) -> String {
    match scope {
        Some(scope) => format!("postio-s{scope}-"),
        None => "postio-s-".to_owned(),
    }
}

/// Case-insensitive, since a sender needs only one spelling Postio's CSS happens to match.
fn is_postio_name(name: &str) -> bool {
    name.get(..7)
        .is_some_and(|start| start.eq_ignore_ascii_case("postio-"))
}

fn escape_css_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

pub const CID_SCHEME: &str = "postio-cid";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RemoteImages {
    #[default]
    Blocked,
    Allowed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sanitized {
    pub html: String,
    /// Separate from `html`: the sender's `<style>` never survives; this is CSS Postio parsed and
    /// rewrote. Empty without a `<style>` block.
    pub styles: String,
    /// Whether nonzero decides if the remote-image banner has anything to say; the parts panel's
    /// held-back count uses the number.
    pub remote_blocked: u32,
    /// Disjoint from `remote_blocked`, so the panel can say "3 remote images and 1 likely tracker";
    /// see `is_likely_tracker` for the heuristic's limits.
    pub trackers: u32,
    pub canvas: Canvas,
    pub color_scheme: Option<ColorScheme>,
    /// Each refusal once, in `REFUSALS` order: counted, not silently lost.
    pub refusals: Vec<Refused>,
    /// A body over a cap is not handed on: `html` and `styles` are empty, and the reader shows the
    /// plain-text alternative with a notice.
    pub over_cap: Option<Cap>,
}

/// Bounds the work a message can ask of a renderer that cannot be stopped mid-layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cap {
    Elements,
    Depth,
    Bytes,
}

pub const MAX_ELEMENTS: usize = 50_000;
pub const MAX_DEPTH: usize = 256;
pub const MAX_BYTES: usize = 2 * 1024 * 1024;

/// Lifted onto the message's own box because the sanitizer's output is a fragment with no `<body>`;
/// a `body { … }` rule in a stylesheet is already resolved by `crate::styles`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Canvas {
    pub style: String,
    pub background: Option<String>,
    pub text: Option<String>,
    pub link: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorScheme {
    Light,
    Dark,
    LightDark,
}

impl ColorScheme {
    pub fn as_str(self) -> &'static str {
        match self {
            ColorScheme::Light => "light",
            ColorScheme::Dark => "dark",
            ColorScheme::LightDark => "light dark",
        }
    }
}

impl Sanitized {
    fn over(cap: Cap) -> Sanitized {
        Sanitized {
            html: String::new(),
            styles: String::new(),
            remote_blocked: 0,
            trackers: 0,
            canvas: Canvas::default(),
            color_scheme: None,
            refusals: Vec::new(),
            over_cap: Some(cap),
        }
    }

    /// What the banner asks: a message whose only remote reference was a beacon still has something
    /// to show if the user insists.
    pub fn held_back(&self) -> u32 {
        self.remote_blocked + self.trackers
    }
}

/// Refusals are enumerable: a property is refused by appearing in `REFUSED` with a reason, never by
/// a judgement on the render path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Containment,
    Privacy,
    NoScript,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Refused {
    Element(&'static str),
    Attribute(&'static str),
    Property(&'static str),
    AtRule(&'static str),
    Unit(&'static str),
    Resource(&'static str),
}

pub const ON_HANDLERS: &str = "on*";
pub const POSTIO_NAMES: &str = "postio- names";
pub const SCRIPT_URLS: &str = "javascript: URLs";
pub const REMOTE_IMAGE: &str = "remote image";
/// A `postio-cid:` the sender wrote rather than Postio: naming a part by Postio's scheme, which
/// under ADR 0032 can be another message's.
pub const FOREIGN_PARTS: &str = "postio-cid: references";
/// An embedded image is the message's own and allowed; a `data:` link is a page opened from inside
/// mail.
pub const NON_IMAGE_DATA: &str = "non-image data: URIs";

/// One list so removals are enumerable: a test provokes every entry and requires it refused and
/// reported in `Sanitized::refusals`; `REFUSED`, `REFUSED_AT_RULES` and `REFUSED_UNITS` are held to
/// it.
pub const REFUSALS: &[(Refused, Refusal)] = &[
    (Refused::Element("script"), Refusal::NoScript),
    (Refused::Element("noscript"), Refusal::Privacy),
    (Refused::Element("iframe"), Refusal::Privacy),
    (Refused::Element("object"), Refusal::NoScript),
    (Refused::Element("embed"), Refusal::NoScript),
    (Refused::Element("svg"), Refusal::NoScript),
    (Refused::Element("math"), Refusal::Containment),
    (Refused::Element("title"), Refusal::Containment),
    (Refused::Element("link"), Refusal::Privacy),
    (Refused::Element("base"), Refusal::Containment),
    (Refused::Element("meta"), Refusal::Containment),
    (Refused::Element("form"), Refusal::Privacy),
    (Refused::Element("input"), Refusal::Privacy),
    (Refused::Element("button"), Refusal::Privacy),
    (Refused::Element("textarea"), Refusal::Privacy),
    (Refused::Element("select"), Refusal::Privacy),
    (Refused::Attribute(ON_HANDLERS), Refusal::NoScript),
    (Refused::Attribute(SCRIPT_URLS), Refusal::NoScript),
    (Refused::Attribute(POSTIO_NAMES), Refusal::Containment),
    (Refused::Property("position"), Refusal::Containment),
    (Refused::Property("z-index"), Refusal::Containment),
    (Refused::Property("top"), Refusal::Containment),
    (Refused::Property("right"), Refusal::Containment),
    (Refused::Property("bottom"), Refusal::Containment),
    (Refused::Property("left"), Refusal::Containment),
    (Refused::Property("inset"), Refusal::Containment),
    (Refused::Property("inset-block"), Refusal::Containment),
    (Refused::Property("inset-block-start"), Refusal::Containment),
    (Refused::Property("inset-block-end"), Refusal::Containment),
    (Refused::Property("inset-inline"), Refusal::Containment),
    (
        Refused::Property("inset-inline-start"),
        Refusal::Containment,
    ),
    (Refused::Property("inset-inline-end"), Refusal::Containment),
    (Refused::Property("transform"), Refusal::Containment),
    (Refused::Property("translate"), Refusal::Containment),
    (Refused::Property("rotate"), Refusal::Containment),
    (Refused::Property("scale"), Refusal::Containment),
    (Refused::AtRule("import"), Refusal::Privacy),
    (Refused::AtRule("font-face"), Refusal::Privacy),
    (Refused::AtRule("namespace"), Refusal::Containment),
    (Refused::AtRule("charset"), Refusal::Containment),
    (Refused::AtRule("page"), Refusal::Containment),
    (Refused::Unit("vw"), Refusal::Containment),
    (Refused::Unit("vh"), Refusal::Containment),
    (Refused::Unit("vmin"), Refusal::Containment),
    (Refused::Unit("vmax"), Refusal::Containment),
    (Refused::Unit("svw"), Refusal::Containment),
    (Refused::Unit("svh"), Refusal::Containment),
    (Refused::Unit("lvw"), Refusal::Containment),
    (Refused::Unit("lvh"), Refusal::Containment),
    (Refused::Unit("dvw"), Refusal::Containment),
    (Refused::Unit("dvh"), Refusal::Containment),
    (Refused::Resource(REMOTE_IMAGE), Refusal::Privacy),
    (Refused::Resource(FOREIGN_PARTS), Refusal::Containment),
    (Refused::Resource(NON_IMAGE_DATA), Refusal::Containment),
];

#[derive(Debug, Default)]
pub(crate) struct Tally {
    pub(crate) blocked: AtomicU32,
    refused: Mutex<Vec<Refused>>,
    scope: Option<String>,
}

impl Tally {
    pub(crate) fn for_scope(scope: Option<&str>) -> Tally {
        Tally {
            scope: scope.map(str::to_owned),
            ..Tally::default()
        }
    }

    pub(crate) fn part_uri(&self, content_id: &str) -> String {
        let id = percent_encode(
            content_id
                .trim()
                .trim_start_matches('<')
                .trim_end_matches('>'),
        );
        match &self.scope {
            // The separator is a literal `/`; the encoded id can never hold one (see
            // `sanitize_body_in`).
            Some(scope) => format!("{CID_SCHEME}:{scope}/{id}"),
            None => format!("{CID_SCHEME}:{id}"),
        }
    }

    pub(crate) fn is_own_part(&self, value: &str) -> bool {
        let Some(rest) = value
            .strip_prefix(CID_SCHEME)
            .and_then(|rest| rest.strip_prefix(':'))
        else {
            return false;
        };
        let id = match &self.scope {
            Some(scope) => rest
                .strip_prefix(scope.as_str())
                .and_then(|rest| rest.strip_prefix('/')),
            None => Some(rest),
        };
        id.is_some_and(|id| !id.is_empty() && !id.contains('/'))
    }

    pub(crate) fn block(&self) {
        self.blocked.fetch_add(1, Ordering::Relaxed);
        self.refuse(Refused::Resource(REMOTE_IMAGE));
    }

    pub(crate) fn refuse(&self, what: Refused) {
        let mut refused = self
            .refused
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !refused.contains(&what) {
            refused.push(what);
        }
    }

    fn refusals(&self) -> Vec<Refused> {
        let refused = self
            .refused
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        REFUSALS
            .iter()
            .map(|(what, _)| *what)
            .filter(|what| refused.contains(what))
            .collect()
    }
}

impl Refused {
    pub fn reason(self) -> Refusal {
        REFUSALS
            .iter()
            .find(|(what, _)| *what == self)
            .map(|(_, reason)| *reason)
            .expect("every Refused a sanitizer reports is listed in REFUSALS")
    }
}

/// The attributes a table-based layout is built from. The counterpart of `REFUSED`: nothing here
/// reaches beyond the message's block or the network.
pub(crate) const TABLE_LAYOUT: &[&str] = &[
    "width",
    "height",
    "align",
    "valign",
    "bgcolor",
    "cellpadding",
    "cellspacing",
    "border",
    "colspan",
    "rowspan",
    "span",
];

/// Matched on the property name only; a refusal drops that one declaration, not the rest of the
/// rule.
pub const REFUSED: &[(&str, Refusal)] = &[
    // `position` and `z-index` lift content out of the box `contain_body` draws (#323), the edge a
    // reader uses to tell Postio's words from a sender's.
    ("position", Refusal::Containment),
    // Offsets mean nothing without a position in a browser, but Blitz applies them to a static box
    // and lifts it out of its message.
    ("top", Refusal::Containment),
    ("right", Refusal::Containment),
    ("bottom", Refusal::Containment),
    ("left", Refusal::Containment),
    ("inset", Refusal::Containment),
    ("inset-block", Refusal::Containment),
    ("inset-block-start", Refusal::Containment),
    ("inset-block-end", Refusal::Containment),
    ("inset-inline", Refusal::Containment),
    ("inset-inline-start", Refusal::Containment),
    ("inset-inline-end", Refusal::Containment),
    // Moves what is painted without moving layout, so a message could draw over what sits beside
    // it.
    ("transform", Refusal::Containment),
    ("translate", Refusal::Containment),
    ("rotate", Refusal::Containment),
    ("scale", Refusal::Containment),
    // Stacking order would let a message draw over the application's own chrome.
    ("z-index", Refusal::Containment),
];

/// Same shape as `REFUSED`. Not exhaustive: `crate::styles` admits a named set (`@media`,
/// `@supports`, `@container`, `@layer`, `@keyframes`) and refuses the rest by omission; this lists
/// those with a reason worth writing down.
pub const REFUSED_AT_RULES: &[(&str, Refusal)] = &[
    // Fetches when the stylesheet parses, with the referer and the reader's IP, and needs no
    // `<img>`.
    ("import", Refusal::Privacy),
    // A remote `src` fetches when a glyph is needed; ADR 0023 serves Postio's own faces.
    ("font-face", Refusal::Privacy),
    // Rebinds element names, which could make a scoped selector match something else.
    ("namespace", Refusal::Containment),
    ("charset", Refusal::Containment),
    ("page", Refusal::Containment),
];

/// Units that answer to the window, not the message's box: `width` is unremarkable until `100vw`.
/// `top`, `right`, `bottom`, `left` and `inset` are absent on purpose: with `position` refused
/// every element is `static` and an offset does nothing. Refuse what grants the power, not what
/// depends on it.
pub const REFUSED_UNITS: &[&str] = &[
    "vw", "vh", "vmin", "vmax", "svw", "svh", "lvw", "lvh", "dvw", "dvh",
];

/// `cid:` becomes a `CID_SCHEME` URI the reader resolves against local parts; a dangling reference
/// resolves to nothing (the corpus has one on purpose).
pub fn sanitize_body(html: &str, remote: RemoteImages) -> Sanitized {
    sanitize_body_in(html, remote, None)
}

/// ADR 0032 puts a thread in one document, where two messages may each carry a part called `logo`
/// and a sender could name another message's `Content-ID`. `scope` stamps the message on every
/// rewritten reference.
/// `/` is a safe separator because `percent_encode` escapes it. A scope the body already carries is
/// percent-encoded like any sender text. `None` is the single-message reader.
pub fn sanitize_body_in(html: &str, remote: RemoteImages, scope: Option<&str>) -> Sanitized {
    if html.len() > MAX_BYTES {
        return Sanitized::over(Cap::Bytes);
    }
    if nests_past(html, PRESCAN_DEPTH) {
        return Sanitized::over(Cap::Depth);
    }
    let scope_for_styles = scope.map(str::to_owned);
    let scope = scope.map(str::to_owned);
    let tally = Arc::new(Tally::for_scope(scope.as_deref()));
    let counter = Arc::clone(&tally);
    let tracker_count = Arc::new(AtomicU32::new(0));
    let trackers = Arc::clone(&tracker_count);

    // Which remote references look like beacons, decided before ammonia runs: `attribute_filter`
    // sees one attribute at a time, so it cannot know an `<img>`'s declared size.
    // Skipped when nothing is blocked (a second parse of a large newsletter is not free). Runs on
    // the one parse `document_facts` makes after the caps, since this walk recurses.
    // Stylesheets, page and `<meta>` are taken from the DOM before ammonia removes them; the scoped
    // result is returned beside the markup (see `Sanitized::styles`).
    let (facts, dom) = match document_facts(html, &tally) {
        Ok(parsed) => parsed,
        Err(cap) => return Sanitized::over(cap),
    };
    let beacons = if remote == RemoteImages::Blocked {
        likely_tracker_sources(&dom.document)
    } else {
        HashSet::new()
    };

    let mut builder = Builder::default();
    builder
        .rm_tags([
            "link", "base", "meta", "form", "input", "button", "textarea", "select",
        ])
        // `title` is not in `rm_tags`, which would keep its text: the sender's document title would
        // become the first line of the message.
        .add_clean_content_tags([
            "iframe", "object", "embed", "svg", "math", "noscript", "title",
        ])
        // `cid` must be allowed on input too: ammonia checks `url_schemes` against the original
        // value before `attribute_filter` can rewrite it. `CID_SCHEME` is listed for intent only.
        .add_url_schemes(["cid", CID_SCHEME, "data"])
        // Inline declarations need no scoping, as they sit inside the container `contain_body`
        // draws; they still need the refusals in `contain_declarations`.
        .add_generic_attributes(["style"])
        // #1545: a sender's stylesheet selects on its own classes and ids, so stripping them killed
        // every such rule. A sender may not wear Postio's names and ids are rewritten per message
        // (`rewrite_attribute`).
        .add_generic_attributes(["class", "id"])
        // Table-layout attributes: HTML email is table-based because that is what Outlook renders,
        // and dropping them collapsed ordinary newsletters into one column. Containment is enforced
        // elsewhere (`contain_body`'s overflow, #1334, and `contain_declarations`), and none reach
        // the network.
        .add_tags(["colgroup", "col"])
        // `<font>` keeps only the style `crate::hints` wrote from its attributes; `<center>` is the
        // one layout element mail still uses.
        .add_tags(["font", "center"])
        .add_tag_attributes("table", TABLE_LAYOUT)
        .add_tag_attributes("thead", TABLE_LAYOUT)
        .add_tag_attributes("tbody", TABLE_LAYOUT)
        .add_tag_attributes("tfoot", TABLE_LAYOUT)
        .add_tag_attributes("tr", TABLE_LAYOUT)
        .add_tag_attributes("td", TABLE_LAYOUT)
        .add_tag_attributes("th", TABLE_LAYOUT)
        .add_tag_attributes("colgroup", TABLE_LAYOUT)
        .add_tag_attributes("col", TABLE_LAYOUT)
        .add_tag_attributes("img", ["width", "height", "align"])
        .attribute_filter(move |element, attribute, value| {
            rewrite_attribute(
                element,
                attribute,
                value,
                remote,
                &counter,
                &trackers,
                &beacons,
                scope.as_deref(),
            )
        });

    let hinted = crate::hints::apply(&dom.document).then(|| serialize_document(&dom));
    let input = hinted.as_deref().unwrap_or(html);
    let selector = message_selector(scope_for_styles.as_deref());
    let canvas = facts.page.canvas(remote, &tally);
    let mut styles = match &canvas.link {
        Some(link) => format!("{selector} a {{ color: {link} }}\n"),
        None => String::new(),
    };
    styles.push_str(&crate::styles::scope_into(
        &facts.stylesheets,
        &selector,
        &sender_id_prefix(scope_for_styles.as_deref()),
        remote,
        &tally,
    ));

    Sanitized {
        html: builder.clean(input).to_string().trim_start().to_owned(),
        styles,
        remote_blocked: tally.blocked.load(Ordering::Relaxed),
        trackers: tracker_count.load(Ordering::Relaxed),
        canvas,
        color_scheme: facts.color_scheme,
        refusals: tally.refusals(),
        over_cap: None,
    }
}

#[derive(Default)]
struct DocumentFacts {
    /// Joined in document order, since they are scoped identically and a browser cascades them that
    /// way. A `<style>` inside `<template>` or a removed subtree is still this sender's CSS.
    stylesheets: String,
    page: Page,
    color_scheme: Option<ColorScheme>,
}

#[derive(Default)]
struct Page {
    bgcolor: Option<String>,
    text: Option<String>,
    link: Option<String>,
    html_style: Option<String>,
    body_style: Option<String>,
}

fn colour(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|v| is_colour(v))
}

impl Page {
    /// The body's own style outranks its attributes, and both outrank `<html>`'s, as in a browser.
    fn canvas(&self, remote: RemoteImages, tally: &Tally) -> Canvas {
        let from_style = |style: &Option<String>, properties: &[&str]| {
            style
                .as_deref()
                .and_then(|style| declared_colour(style, properties))
        };
        let background = from_style(&self.body_style, &["background-color", "background"])
            .or_else(|| colour(&self.bgcolor).map(str::to_owned))
            .or_else(|| from_style(&self.html_style, &["background-color", "background"]));
        let text = from_style(&self.body_style, &["color"])
            .or_else(|| colour(&self.text).map(str::to_owned))
            .or_else(|| from_style(&self.html_style, &["color"]));
        let link = colour(&self.link).map(str::to_owned);

        let mut declarations: Vec<String> = Vec::new();
        if let Some(background) = &background {
            declarations.push(format!("background-color: {background}"));
        }
        if let Some(text) = &text {
            declarations.push(format!("color: {text}"));
        }
        for style in [&self.html_style, &self.body_style].into_iter().flatten() {
            let kept = contain_declarations(style, remote, tally);
            if !kept.is_empty() {
                declarations.push(kept);
            }
        }
        Canvas {
            style: declarations.join("; "),
            background,
            text,
            link,
        }
    }
}

fn document_facts(html: &str, tally: &Tally) -> Result<(DocumentFacts, RcDom), Cap> {
    let dom = parse_document(RcDom::default(), ParseOpts::default()).one(html);
    // Before anything recursive walks the tree: a message nested tens of thousands deep overflowed
    // the stack, which aborts with no unwind.
    if let Some(cap) = over_cap(&dom.document) {
        return Err(cap);
    }
    let mut facts = DocumentFacts::default();
    collect_facts(&dom.document, &mut facts, tally);
    Ok((facts, dom))
}

/// Above the real cap on purpose: the scan cannot see every end tag HTML implies, so it stops only
/// what is plainly beyond `MAX_DEPTH`.
const PRESCAN_DEPTH: usize = MAX_DEPTH * 4;

/// Not counted as nesting, or an ordinary message of unclosed `<p>`s would look ten thousand deep.
const FLAT: &[&str] = &[
    "p", "li", "dt", "dd", "tr", "td", "th", "option", "optgroup", "tbody", "thead", "tfoot",
    "colgroup", "rb", "rt", "rp", "area", "base", "br", "col", "embed", "hr", "img", "input",
    "link", "meta", "source", "track", "wbr",
];

/// One linear scan: the tree builder's work grows with the square of depth (forty thousand levels
/// took over thirty seconds), so the cap cannot wait for the parse.
fn nests_past(html: &str, limit: usize) -> bool {
    let bytes = html.as_bytes();
    let mut depth = 0usize;
    let mut at = 0;
    while let Some(offset) = bytes[at..].iter().position(|b| *b == b'<') {
        at += offset + 1;
        if bytes[at..].starts_with(b"!--") {
            at = find(bytes, at, b"-->").unwrap_or(bytes.len());
            continue;
        }
        let closing = bytes.get(at) == Some(&b'/');
        let start = at + usize::from(closing);
        let len = bytes[start..]
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric())
            .count();
        if len == 0 {
            continue;
        }
        let name = html[start..start + len].to_ascii_lowercase();
        let end = find(bytes, start, b">").unwrap_or(bytes.len());
        at = end;
        if FLAT.contains(&name.as_str()) {
            continue;
        }
        if closing {
            depth = depth.saturating_sub(1);
            continue;
        }
        if bytes[..end].ends_with(b"/") {
            continue;
        }
        if name == "script" || name == "style" {
            at = find(bytes, at, format!("</{name}").as_bytes()).unwrap_or(bytes.len());
            continue;
        }
        depth += 1;
        if depth > limit {
            return true;
        }
    }
    false
}

fn find(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
        .map(|offset| from + offset)
}

/// Iterative, so deep input cannot overflow the stack.
fn over_cap(root: &Handle) -> Option<Cap> {
    let mut elements = 0usize;
    let mut stack: Vec<(Handle, usize)> = vec![(root.clone(), 0)];
    while let Some((node, depth)) = stack.pop() {
        let depth = if matches!(node.data, NodeData::Element { .. }) {
            elements += 1;
            depth + 1
        } else {
            depth
        };
        if elements > MAX_ELEMENTS {
            return Some(Cap::Elements);
        }
        if depth > MAX_DEPTH {
            return Some(Cap::Depth);
        }
        stack.extend(
            node.children
                .borrow()
                .iter()
                .map(|child| (child.clone(), depth)),
        );
    }
    None
}

fn serialize_document(dom: &RcDom) -> String {
    let mut bytes = Vec::new();
    let handle: SerializableHandle = dom.document.clone().into();
    let options = SerializeOpts {
        traversal_scope: TraversalScope::ChildrenOnly(None),
        ..SerializeOpts::default()
    };
    let _ = serialize(&mut bytes, &handle, options);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn refused_element(name: &str) -> Option<&'static str> {
    REFUSALS.iter().find_map(|(what, _)| match what {
        Refused::Element(element) if element.eq_ignore_ascii_case(name) => Some(*element),
        _ => None,
    })
}

fn collect_facts(node: &Handle, facts: &mut DocumentFacts, tally: &Tally) {
    if let NodeData::Element { name, attrs, .. } = &node.data {
        let element = &*name.local;
        let attrs = attrs.borrow();
        let get = |wanted: &str| {
            attrs
                .iter()
                .find(|attr| str::eq_ignore_ascii_case(&attr.name.local, wanted))
                .map(|attr| attr.value.to_string())
        };
        if let Some(refused) = refused_element(element) {
            tally.refuse(Refused::Element(refused));
        }
        for attr in attrs.iter() {
            let name = &*attr.name.local;
            if name.len() > 2
                && name
                    .get(..2)
                    .is_some_and(|on| on.eq_ignore_ascii_case("on"))
            {
                tally.refuse(Refused::Attribute(ON_HANDLERS));
            }
            if name.eq_ignore_ascii_case("href") && runs_script(&attr.value) {
                tally.refuse(Refused::Attribute(SCRIPT_URLS));
            }
        }
        if element.eq_ignore_ascii_case("style") {
            for child in node.children.borrow().iter() {
                if let NodeData::Text { contents } = &child.data {
                    facts.stylesheets.push_str(&contents.borrow());
                    facts.stylesheets.push('\n');
                }
            }
            return;
        }
        if element.eq_ignore_ascii_case("html") {
            facts.page.html_style = get("style");
        } else if element.eq_ignore_ascii_case("body") {
            facts.page.bgcolor = get("bgcolor");
            facts.page.text = get("text");
            facts.page.link = get("link");
            facts.page.body_style = get("style");
        } else if element.eq_ignore_ascii_case("meta")
            && facts.color_scheme.is_none()
            && get("name").is_some_and(|name| {
                name.eq_ignore_ascii_case("color-scheme")
                    || name.eq_ignore_ascii_case("supported-color-schemes")
            })
        {
            facts.color_scheme = get("content").as_deref().and_then(color_scheme);
        }
    }
    for child in node.children.borrow().iter() {
        collect_facts(child, facts, tally);
    }
}

/// Browsers ignore ASCII whitespace and control characters inside the scheme.
fn runs_script(url: &str) -> bool {
    let compact: String = url
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && !c.is_ascii_control())
        .take(11)
        .collect();
    compact.to_ascii_lowercase().starts_with("javascript:")
}

fn color_scheme(content: &str) -> Option<ColorScheme> {
    let words: Vec<String> = content
        .split_ascii_whitespace()
        .map(str::to_ascii_lowercase)
        .collect();
    let light = words.iter().any(|w| w == "light");
    let dark = words.iter().any(|w| w == "dark");
    match (light, dark) {
        (true, true) => Some(ColorScheme::LightDark),
        (false, true) => Some(ColorScheme::Dark),
        (true, false) => Some(ColorScheme::Light),
        (false, false) => None,
    }
}

fn declared_colour(style: &str, properties: &[&str]) -> Option<String> {
    let declarations: Vec<(String, String)> = split_declarations(style)
        .into_iter()
        .filter_map(|declaration| {
            let (property, value) = declaration.split_once(':')?;
            Some((
                property.trim().to_ascii_lowercase(),
                value.trim().to_owned(),
            ))
        })
        .collect();
    properties.iter().find_map(|wanted| {
        declarations
            .iter()
            .rev()
            .find(|(property, _)| property == wanted)
            .map(|(_, value)| value.trim_end_matches("!important").trim().to_owned())
            .filter(|value| is_colour(value))
    })
}

/// Conservative on purpose: the value goes into a `style` Postio writes and an attribute is
/// attacker-controlled (`red;position:fixed` is not a colour, nor is anything with `url(`).
pub(crate) fn is_colour(value: &str) -> bool {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        return matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit());
    }
    if value.len() <= 32 && !value.is_empty() && value.chars().all(|c| c.is_ascii_alphabetic()) {
        return true;
    }
    let lower = value.to_ascii_lowercase();
    ["rgb(", "rgba(", "hsl(", "hsla("].iter().any(|function| {
        lower
            .strip_prefix(function)
            .and_then(|rest| rest.strip_suffix(')'))
            .is_some_and(|inner| {
                inner
                    .chars()
                    .all(|c| c.is_ascii_digit() || " ,./%-".contains(c))
            })
    })
}

#[allow(clippy::too_many_arguments)]
fn rewrite_attribute<'u>(
    _element: &str,
    attribute: &str,
    value: &'u str,
    remote: RemoteImages,
    tally: &Tally,
    tracker_count: &AtomicU32,
    beacons: &HashSet<String>,
    scope: Option<&str>,
) -> Option<Cow<'u, str>> {
    if attribute == "style" {
        let kept = contain_declarations(value, remote, tally);
        return (!kept.is_empty()).then_some(Cow::Owned(kept));
    }
    if attribute == "class" {
        let kept: Vec<&str> = value
            .split_ascii_whitespace()
            .filter(|class| {
                let postio = is_postio_name(class);
                if postio {
                    tally.refuse(Refused::Attribute(POSTIO_NAMES));
                }
                !postio
            })
            .collect();
        return (!kept.is_empty()).then(|| Cow::Owned(kept.join(" ")));
    }
    if attribute == "id" {
        let id = value.trim();
        let prefix = sender_id_prefix(scope);
        if id.len() > prefix.len() && id.starts_with(&prefix) {
            return Some(Cow::Owned(id.to_owned()));
        }
        if is_postio_name(id) {
            tally.refuse(Refused::Attribute(POSTIO_NAMES));
            return None;
        }
        if id.is_empty() {
            return None;
        }
        return Some(Cow::Owned(format!("{prefix}{id}")));
    }
    if is_data_uri(value) {
        // An embedded image is the message's own; any other `data:` URI is not kept.
        if attribute == "src" && is_data_image(value) {
            return Some(Cow::Borrowed(value));
        }
        tally.refuse(Refused::Resource(NON_IMAGE_DATA));
        return None;
    }
    if attribute == "href"
        && let Some(fragment) = value.trim().strip_prefix('#')
        && !fragment.is_empty()
    {
        let prefix = sender_id_prefix(scope);
        if fragment.len() > prefix.len() && fragment.starts_with(&prefix) {
            return Some(Cow::Owned(format!("#{fragment}")));
        }
        return Some(Cow::Owned(format!("#{prefix}{fragment}")));
    }
    if attribute != "src" {
        return Some(Cow::Borrowed(value));
    }
    // Postio's own scheme written by the sender names the reader's internals (under ADR 0032,
    // another message's parts): dropped, not rewritten, unless it is this message's own.
    if tally.is_own_part(value) {
        return Some(Cow::Borrowed(value));
    }
    if names_foreign_parts(value) {
        tally.refuse(Refused::Resource(FOREIGN_PARTS));
        return None;
    }
    if let Some(id) = value.strip_prefix("cid:") {
        return Some(Cow::Owned(tally.part_uri(id)));
    }
    if is_remote(value) && remote == RemoteImages::Blocked {
        // One or the other, never both: the panel adds them up.
        if beacons.contains(value.trim()) {
            tracker_count.fetch_add(1, Ordering::Relaxed);
            tally.refuse(Refused::Resource(REMOTE_IMAGE));
        } else {
            tally.block();
        }
        return None;
    }
    Some(Cow::Borrowed(value))
}

/// Whole declarations: refusing `position` must not discard the `color` beside it, which would
/// render the message wrongly with no visible cause.
pub(crate) fn contain_declarations(value: &str, remote: RemoteImages, tally: &Tally) -> String {
    let mut kept: Vec<Cow<'_, str>> = Vec::new();
    for declaration in split_declarations(value) {
        let Some((property, declared)) = declaration.split_once(':') else {
            continue;
        };
        let property = property.trim().to_ascii_lowercase();
        let declared = declared.trim();

        if let Some((refused, _)) = REFUSED.iter().find(|(refused, _)| *refused == property) {
            tally.refuse(Refused::Property(refused));
            continue;
        }
        if let Some(unit) = viewport_unit(declared) {
            tally.refuse(Refused::Unit(unit));
            continue;
        }
        // Before the single-URL handling, which reads only the first `url(` and misses
        // `image-set("https://..")`, an escaped `\75 rl(` or `src("..")` (#1926). Any remote
        // reference drops the declaration.
        if remote == RemoteImages::Blocked && names_remote(declared) {
            tally.block();
            continue;
        }
        if names_other_parts(declared, tally) {
            tally.refuse(Refused::Resource(FOREIGN_PARTS));
            continue;
        }
        if let Some(url) = css_url(declared) {
            if is_remote(&url) && remote == RemoteImages::Blocked {
                tally.block();
                continue;
            }
            if let Some(id) = url.trim().strip_prefix("cid:") {
                let property = declaration.split_once(':').map_or("", |(p, _)| p.trim());
                let rewritten = replace_css_url(declared, &tally.part_uri(id));
                kept.push(Cow::Owned(format!("{property}: {rewritten}")));
                continue;
            }
        }
        kept.push(Cow::Borrowed(declaration.trim()));
    }
    kept.join("; ")
}

/// A plain-text scheme scan backs the tokens for spellings a later parser might accept.
fn names_remote(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if ["http://", "https://", "ftp://"]
        .iter()
        .any(|scheme| lower.contains(scheme))
    {
        return true;
    }
    names_url(value, &is_remote)
}

/// Every URL in the value, not its first `url(`: a second one or an escape named another message's.
fn names_other_parts(value: &str, tally: &Tally) -> bool {
    let lower = value.to_ascii_lowercase();
    let spelled = lower.match_indices(CID_SCHEME).any(|(at, _)| {
        let reference = value[at..]
            .split(|c: char| c.is_whitespace() || "()'\",".contains(c))
            .next()
            .unwrap_or_default();
        !tally.is_own_part(reference)
    });
    spelled
        || names_url(value, &|url| {
            names_foreign_parts(url) && !tally.is_own_part(url)
        })
}

/// As CSS tokens, so an escaped `\75 rl(` is a URL and a string given to `image-set()` counts.
fn names_url(value: &str, names: &dyn Fn(&str) -> bool) -> bool {
    fn walk(parser: &mut cssparser::Parser<'_>, names: &dyn Fn(&str) -> bool) -> bool {
        use cssparser::Token;
        while let Ok(token) = parser.next_including_whitespace_and_comments().cloned() {
            match token {
                Token::QuotedString(text) | Token::UnquotedUrl(text) if names(&text) => {
                    return true;
                }
                Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock => {
                    let inner = parser
                        .parse_nested_block(|nested| {
                            Ok::<_, cssparser::ParseError<()>>(walk(nested, names))
                        })
                        .unwrap_or(false);
                    if inner {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }
    walk(&mut cssparser::Parser::new(value), names)
}

/// `url(data:image/png;base64,...)` carries its own semicolon, so a naive `split(';')` would cut an
/// inline image in two.
pub(crate) fn split_declarations(value: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut start = 0usize;
    for (at, character) in value.char_indices() {
        match character {
            '\'' | '"' if quote == Some(character) => quote = None,
            '\'' | '"' if quote.is_none() => quote = Some(character),
            '(' if quote.is_none() => depth += 1,
            ')' if quote.is_none() => depth = depth.saturating_sub(1),
            ';' if quote.is_none() && depth == 0 => {
                out.push(&value[start..at]);
                start = at + character.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&value[start..]);
    out.into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect()
}

/// Matched as a unit suffix on a number, so `font-family: "Vivaldi"` is not mistaken for `vi`.
fn viewport_unit(value: &str) -> Option<&'static str> {
    let lowered = value.to_ascii_lowercase();
    let bytes = lowered.as_bytes();
    REFUSED_UNITS.iter().copied().find(|unit| {
        lowered.match_indices(unit).any(|(at, _)| {
            let before = at > 0 && bytes[at - 1].is_ascii_digit();
            let after = bytes
                .get(at + unit.len())
                .is_none_or(|next| !next.is_ascii_alphanumeric());
            before && after
        })
    })
}

fn is_data_uri(value: &str) -> bool {
    value
        .trim_start()
        .get(..5)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("data:"))
}

fn is_data_image(value: &str) -> bool {
    value
        .trim_start()
        .get(5..11)
        .is_some_and(|kind| kind.eq_ignore_ascii_case("image/"))
}

fn names_foreign_parts(value: &str) -> bool {
    value
        .trim_start()
        .to_ascii_lowercase()
        .starts_with(&format!("{CID_SCHEME}:"))
}

fn replace_css_url(value: &str, uri: &str) -> String {
    let lower = value.to_ascii_lowercase();
    let Some(start) = lower.find("url(") else {
        return value.to_owned();
    };
    let Some(end) = value[start..].find(')').map(|end| start + end) else {
        return value.to_owned();
    };
    format!("{}url({uri}){}", &value[..start], &value[end + 1..])
}

fn css_url(value: &str) -> Option<String> {
    let start = value.to_ascii_lowercase().find("url(")? + 4;
    let rest = &value[start..];
    let end = rest.find(')')?;
    Some(
        rest[..end]
            .trim()
            .trim_matches(|c| c == '\'' || c == '"')
            .to_string(),
    )
}

/// Returned as URL strings, not a count: counting happens where blocking happens, since a reference
/// this walk sees but the sanitizer removes for another reason must not count.
/// A URL used as both picture and beacon counts as a beacon both times.
fn likely_tracker_sources(document: &Handle) -> HashSet<String> {
    let mut found = HashSet::new();
    collect_beacons(document, &mut found);
    found
}

fn collect_beacons(node: &Handle, found: &mut HashSet<String>) {
    if let NodeData::Element { name, attrs, .. } = &node.data
        && str::eq_ignore_ascii_case(&name.local, "img")
    {
        let attrs = attrs.borrow();
        let get = |wanted: &str| {
            attrs
                .iter()
                .find(|attr| str::eq_ignore_ascii_case(&attr.name.local, wanted))
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

/// Settled by the maintainer (#174): declared dimensions of 2px or less in either axis, or a
/// declaration that it is not shown. Nothing reads the host or path: a vendor list is provider
/// hard-coding and mislabels real pictures.
/// It under-counts on purpose, since a beacon with no declared size looks like an ordinary image.
/// Both kinds are blocked identically, so being wrong costs a noun, not a request.
fn is_likely_tracker(width: Option<String>, height: Option<String>, style: Option<String>) -> bool {
    const BEACON_PX: f32 = 2.0;

    let tiny = |value: &str| length_px(value).is_some_and(|px| px <= BEACON_PX);

    if width.as_deref().is_some_and(tiny) || height.as_deref().is_some_and(tiny) {
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
            "display" if value == "none" => return true,
            "visibility" if value == "hidden" => return true,
            _ => {}
        }
    }
    false
}

/// `None` for a percentage, `auto`, `em` or nonsense: a length this cannot read is not evidence of
/// a beacon.
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

/// Case-insensitive (RFC 3986 §3.1, as WebKit resolves it): a pixel spelled `HTTPS://` was once
/// left in and reported as nothing held back (#147).
/// ASCII only: a scheme is ASCII by grammar, and `to_lowercase` would allocate for every attribute
/// of every message.
pub(crate) fn is_remote(value: &str) -> bool {
    let value = value.trim();
    ["http://", "https://", "ftp://"]
        .iter()
        // `get`, not a slice: an attribute value is attacker-controlled and may start with a
        // multi-byte character, which panics `value[..8]`.
        .any(|scheme| {
            value
                .get(..scheme.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(scheme))
        })
        || value.starts_with("//")
}

/// RFC 3986's unreserved set passes unescaped, so an odd Content-ID cannot yield a URI the scheme
/// handler parses differently.
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

    /// A message that arranges itself in three columns must appear in three columns: dropping table
    /// attributes fails the technique almost everybody uses (#1396).
    #[test]
    fn a_senders_layout_reaches_the_screen() {
        let three_columns = concat!(
            r#"<table width="100%" cellpadding="8" cellspacing="0" border="0">"#,
            r##"<tr><td width="33%" align="center" valign="top" bgcolor="#eef">one</td>"##,
            r#"<td width="33%">two</td><td width="34%">three</td></tr></table>"#,
        );
        let clean = sanitize_body(three_columns, RemoteImages::Blocked).html;
        for kept in [
            r#"width="100%""#,
            r#"width="33%""#,
            r#"cellpadding="8""#,
            r#"cellspacing="0""#,
            r#"align="center""#,
            r#"valign="top""#,
            r##"bgcolor="#eef""##,
        ] {
            assert!(
                clean.contains(kept),
                "a three-column message lost {kept}, so it does not appear in \
                 three columns: {clean}"
            );
        }
    }

    /// Asserted so tightening `contain_declarations` for containment cannot quietly take colour,
    /// emphasis or spacing with it.
    #[test]
    fn colour_emphasis_and_spacing_reach_the_screen_too() {
        for (what, html, kept) in [
            (
                "colour",
                r##"<p style="color:#c0392b;background:#fff8f0">warm</p>"##,
                "color:#c0392b",
            ),
            (
                "font choice",
                r#"<p style="font-family:Georgia,serif">serif</p>"#,
                "font-family:Georgia,serif",
            ),
            (
                "emphasis",
                r#"<p style="font-weight:700;font-style:italic">loud</p>"#,
                "font-weight:700",
            ),
            (
                "spacing",
                r#"<div style="margin:24px;line-height:1.8">airy</div>"#,
                "margin:24px",
            ),
            (
                "alignment",
                r#"<div style="text-align:center">middle</div>"#,
                "text-align:center",
            ),
        ] {
            let clean = sanitize_body(html, RemoteImages::Blocked).html;
            assert!(clean.contains(kept), "{what} did not survive: {clean}");
        }
    }

    #[test]
    fn a_style_element_is_admitted_scoped_and_never_as_markup() {
        let hostile = r##"<style>@import url(https://tracker.example.net/s.css);
             p { color: red }</style><p style="color:green">text</p>"##;
        let clean = sanitize_body_in(hostile, RemoteImages::Allowed, Some("7"));

        assert!(
            !clean.styles.contains("@import") && !clean.styles.contains("tracker.example.net"),
            "a sender's stylesheet import survived: {}",
            clean.styles
        );
        assert!(
            clean
                .styles
                .contains(&format!("{} p", message_selector(Some("7")))),
            "the rule must arrive scoped to its own message: {}",
            clean.styles
        );
        assert!(
            !clean.html.contains("<style") && !clean.html.contains("color: red"),
            "the element itself must never reach the markup -- Postio emits \
             the CSS it parsed, it does not pass the sender's tag through: {}",
            clean.html
        );
        // The control: inline `style` is admitted too, so the assertions are about `<style>` and
        // not about styling being dropped.
        assert!(
            clean.html.contains("color:green") || clean.html.contains("color: green"),
            "an inline style attribute must still survive, or this test is \
             passing because nothing styled anything: {}",
            clean.html
        );
    }

    #[test]
    fn a_scope_cannot_break_out_of_the_selector_it_names() {
        // A scope is a database id, so a quote in one is not a sender's doing today; it is escaped
        // anyway, since an unescaped value would close the attribute selector.
        let selector = message_selector(Some(r#"a" ] , * { color: red } x["#));
        let opening = format!(".{BODY_CLASS}[{MESSAGE_ATTRIBUTE}=\"");
        let inside = selector
            .strip_prefix(&opening)
            .and_then(|rest| rest.strip_suffix("\"]"))
            .unwrap_or_else(|| panic!("not one attribute selector: {selector}"));
        // Escaped pairs removed first, so only quotes that would close the string are counted.
        assert!(
            !inside.replace("\\\\", "").replace("\\\"", "").contains('"'),
            "an unescaped quote closes the string and frees every rule after \
             it: {selector}"
        );
    }

    #[test]
    fn two_messages_stylesheets_cannot_reach_each_other() {
        let sheet = "<style>p { color: red }</style><p>hi</p>";
        let one = sanitize_body_in(sheet, RemoteImages::Blocked, Some("1"));
        let two = sanitize_body_in(sheet, RemoteImages::Blocked, Some("2"));
        assert_ne!(
            one.styles, two.styles,
            "the same sheet in two messages must be scoped to each"
        );
        assert!(one.styles.contains(message_selector(Some("1")).as_str()));
        assert!(!one.styles.contains(message_selector(Some("2")).as_str()));
    }

    #[test]
    fn a_remote_reference_in_a_stylesheet_is_counted_with_the_images() {
        let clean = sanitize_body_in(
            "<style>p { background-image: url(https://tracker.example/p.gif) }</style>",
            RemoteImages::Blocked,
            Some("1"),
        );
        assert!(
            !clean.styles.contains("tracker.example"),
            "{}",
            clean.styles
        );
        assert_eq!(
            clean.remote_blocked, 1,
            "the parts panel says `n remote images blocked`, and a CSS \
             background is one of them"
        );
    }

    #[test]
    fn the_unscoped_reader_still_gets_a_container_selector() {
        // The single-message pane has no scope, but the chrome outside `.postio-body` is the same
        // and must not be nameable.
        let clean = sanitize_body(
            "<style>.postio-blocked { display: none }</style>",
            RemoteImages::Blocked,
        );
        assert!(
            clean.styles.starts_with(&message_selector(None)),
            "{}",
            clean.styles
        );
    }

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
        let out = sanitize_body(
            r#"<img src="https://example.com/b.gif" style="display:none">"#,
            RemoteImages::Blocked,
        );
        assert_eq!(out.trackers, 1);
    }

    #[test]
    fn a_url_that_merely_looks_like_a_beacon_is_not_one() {
        // Path- and domain-based guessing mislabels real pictures, and a vendor list rots.
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
    fn a_style_tag_never_reaches_the_markup() {
        let out = sanitize_body(
            "<style>.hero{background:url('https://tracker.example.org/bg.jpg')}</style><p>ok</p>",
            RemoteImages::Blocked,
        );
        assert_eq!(out.html, "<p>ok</p>", "the element does not survive");
        assert!(!out.html.contains("tracker.example.org"));
        assert!(
            !out.styles.contains("tracker.example.org"),
            "{}",
            out.styles
        );
        // A CSS background reference counts with the images: to the reader it is as blocked as an
        // `<img>` (#1326).
        assert_eq!(out.remote_blocked, 1);
    }

    #[test]
    fn an_inline_style_survives_so_the_senders_layout_does() {
        let out = sanitize_body(r#"<p style="color:red">hi</p>"#, RemoteImages::Blocked);
        assert!(out.html.contains("color"), "{}", out.html);
    }

    #[test]
    fn layout_colour_and_spacing_all_survive() {
        let out = sanitize_body(
            r#"<div style="display:flex;gap:12px;width:60%"><p style="color:#c00;font-weight:700;margin:8px">hi</p></div>"#,
            RemoteImages::Blocked,
        );
        for surviving in ["display", "gap", "width", "color", "font-weight", "margin"] {
            assert!(
                out.html.contains(surviving),
                "{surviving} did not survive: {}",
                out.html
            );
        }
    }

    #[test]
    fn a_message_cannot_pin_itself_over_the_application() {
        for escape in ["position:fixed", "position: sticky"] {
            let out = sanitize_body(
                &format!(r#"<p style="{escape};top:0;color:red">hi</p>"#),
                RemoteImages::Blocked,
            );
            assert!(
                !out.html.contains("position"),
                "{escape} survived: {}",
                out.html
            );
            assert!(!out.html.contains("top"), "{}", out.html);
            assert!(out.html.contains("color:red"), "{}", out.html);
        }
    }

    #[test]
    fn a_message_cannot_lift_itself_above_the_chrome() {
        let out = sanitize_body(
            r#"<p style="z-index:99999;color:red">hi</p>"#,
            RemoteImages::Blocked,
        );
        assert!(!out.html.contains("z-index"), "{}", out.html);
        assert!(out.html.contains("color"), "{}", out.html);
    }

    #[test]
    fn a_message_cannot_size_itself_against_the_window() {
        let out = sanitize_body(
            r#"<p style="width:100vw;height:100vh;padding:4px">hi</p>"#,
            RemoteImages::Blocked,
        );
        assert!(!out.html.contains("100vw"), "{}", out.html);
        assert!(!out.html.contains("100vh"), "{}", out.html);
        assert!(out.html.contains("padding"), "{}", out.html);
    }

    #[test]
    fn a_remote_url_in_a_style_is_held_back_like_a_remote_img() {
        let out = sanitize_body(
            r#"<p style="background-image:url(https://tracker.example.org/o.gif);color:red">hi</p>"#,
            RemoteImages::Blocked,
        );
        assert!(
            !out.html.contains("tracker.example.org"),
            "a style reached the network: {}",
            out.html
        );
        assert!(out.html.contains("color"), "{}", out.html);
    }

    #[test]
    fn an_uppercase_url_in_a_style_is_held_back_too() {
        // RUSTSEC-2026-0373: ammonia matched a quoted `url("")` case-sensitively; this is the
        // advisory's payload.
        for style in [
            r#"background:URL("https://evil.example/ping");color:red"#,
            r#"background:UrL('https://evil.example/ping');color:red"#,
            r#"background-image:URL(https://evil.example/ping);color:red"#,
        ] {
            let out = sanitize_body(
                &format!(r#"<div style='{style}'>ok</div>"#),
                RemoteImages::Blocked,
            );
            assert!(
                !out.html.contains("evil.example"),
                "a style reached the network: {}",
                out.html
            );
            assert!(out.html.contains("ok"), "{}", out.html);
        }
    }

    #[test]
    fn every_refused_property_states_a_reason() {
        assert!(
            !REFUSED.is_empty(),
            "the refused set is the whole of the containment story"
        );
        for (property, reason) in REFUSED {
            assert!(
                matches!(reason, Refusal::Containment | Refusal::Privacy),
                "{property} is refused for no stated reason"
            );
        }
    }

    #[test]
    fn noscript_content_is_removed_not_unwrapped() {
        let out = sanitize_body(
            r#"<noscript><img src="https://tracker.example.org/o.gif"></noscript><p>body</p>"#,
            RemoteImages::Allowed,
        );
        assert_eq!(out.html, "<p>body</p>");
    }

    /// Ammonia keeps the text of an unlisted element, so a `<title>` became the first words of the
    /// body (`html-newsletter.eml` rendered its issue title above its masthead).
    #[test]
    fn a_documents_title_is_not_a_line_of_its_body() {
        let out = sanitize_body(
            "<html><head><title>Weekly digest</title></head>\
             <body><p>body</p></body></html>",
            RemoteImages::Blocked,
        );
        assert_eq!(out.html, "<p>body</p>");
    }

    /// #147, from the `sanitize_html` fuzz target: schemes are case-insensitive (RFC 3986 §3.1), so
    /// a pixel spelled `HTTPS://` was fetched while nothing was reported held back.
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

    /// Slicing `value[..8]` panics when byte 8 lands inside a multi-byte character, and attribute
    /// values are attacker-controlled.
    #[test]
    fn a_multibyte_attribute_value_does_not_split_a_character() {
        for value in ["日本語です", "é", "\u{fffd}\u{fffd}\u{fffd}", "🐟🐟🐟"] {
            let html = format!(r#"<img src="{value}">"#);
            let out = sanitize_body(&html, RemoteImages::Blocked);
            assert_eq!(out.remote_blocked, 0, "{value} is not remote");
        }
    }

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

#[cfg(test)]
mod conversation_scope_tests {
    use super::*;

    #[test]
    fn a_scoped_body_stamps_its_message_on_every_cid() {
        let html = r#"<img src="cid:logo"><img src="cid:a/b">"#;
        let scoped = sanitize_body_in(html, RemoteImages::Blocked, Some("42"));
        assert!(
            scoped.html.contains("postio-cid:42/logo"),
            "an inline image did not carry its message: {}",
            scoped.html
        );
        assert!(
            scoped.html.contains("postio-cid:42/a%2Fb"),
            "a Content-ID containing a slash must stay escaped, or the split \
             would read it as another message: {}",
            scoped.html
        );
    }

    #[test]
    fn an_unscoped_body_is_exactly_what_it_always_was() {
        let html = r#"<img src="cid:logo">"#;
        assert_eq!(
            sanitize_body_in(html, RemoteImages::Blocked, None).html,
            sanitize_body(html, RemoteImages::Blocked).html,
        );
        assert!(
            sanitize_body(html, RemoteImages::Blocked)
                .html
                .contains("postio-cid:logo")
        );
    }

    #[test]
    fn a_sender_cannot_name_another_message() {
        let html = r#"<img src="cid:9/secret"><img src="postio-cid:9/secret">"#;
        let scoped = sanitize_body_in(html, RemoteImages::Blocked, Some("42"));
        assert!(
            !scoped.html.contains("postio-cid:9/secret"),
            "a sender's own scope survived sanitising: {}",
            scoped.html
        );
        assert!(
            scoped.html.contains("postio-cid:42/9%2Fsecret"),
            "the rewritten reference should be scoped to this message: {}",
            scoped.html
        );
    }

    #[test]
    fn a_second_pass_keeps_the_parts_the_first_named() {
        let html =
            r#"<img src="cid:logo@example.com" alt=""><p style="background:url(cid:bg)">t</p>"#;
        for scope in [None, Some("42")] {
            let once = sanitize_body_in(html, RemoteImages::Blocked, scope);
            assert!(once.html.contains("postio-cid:"), "{}", once.html);
            let twice = sanitize_body_in(&once.html, RemoteImages::Blocked, scope);
            assert_eq!(twice.html, once.html, "scope {scope:?}");
            assert!(twice.refusals.is_empty(), "{:?}", twice.refusals);
        }
    }

    #[test]
    fn a_second_pass_keeps_the_ids_the_first_prefixed() {
        let html = r##"<p id="masthead">t</p><a href="#masthead">top</a>"##;
        for scope in [None, Some("42")] {
            let once = sanitize_body_in(html, RemoteImages::Blocked, scope);
            let twice = sanitize_body_in(&once.html, RemoteImages::Blocked, scope);
            assert_eq!(twice.html, once.html, "scope {scope:?}");
        }
        let other = sanitize_body_in(
            r##"<p id="postio-s7-masthead">t</p>"##,
            RemoteImages::Blocked,
            Some("42"),
        );
        assert!(
            !other.html.contains("postio-s7-"),
            "message 42 wore message 7's names: {}",
            other.html
        );
    }

    #[test]
    fn a_style_cannot_name_another_message_past_its_first_url() {
        for html in [
            r#"<p style="background-image: url(cid:a), url(postio-cid:9/secret)">t</p>"#,
            r#"<p style="background-image: url(postio\2d cid:9/secret)">t</p>"#,
            r#"<p style="background-image: image-set('postio-cid:9/secret' 1x)">t</p>"#,
        ] {
            let clean = sanitize_body_in(html, RemoteImages::Blocked, Some("42"));
            assert!(
                !clean.html.contains("9/secret"),
                "{html} kept: {}",
                clean.html
            );
        }
    }

    #[test]
    fn a_second_pass_still_refuses_another_scopes_parts() {
        let html =
            r#"<img src="postio-cid:9/secret"><p style="background:url(postio-cid:9/bg)">t</p>"#;
        let clean = sanitize_body_in(html, RemoteImages::Blocked, Some("42"));
        assert!(!clean.html.contains("postio-cid:9"), "{}", clean.html);
        let unscoped = sanitize_body_in(html, RemoteImages::Blocked, None);
        assert!(!unscoped.html.contains("postio-cid:9"), "{}", unscoped.html);
        let other = sanitize_body_in(
            r#"<img src="postio-cid:logo">"#,
            RemoteImages::Blocked,
            Some("42"),
        );
        assert!(
            !other.html.contains("postio-cid:logo"),
            "an unscoped part is no part of message 42: {}",
            other.html
        );
    }

    #[test]
    fn a_senders_classes_and_ids_survive() {
        let clean = sanitize_body_in(
            r#"<div class="notice wide" id="masthead">Hi</div>"#,
            RemoteImages::Blocked,
            Some("7"),
        );
        assert!(
            clean.html.contains(r#"class="notice wide""#),
            "{}",
            clean.html
        );
        assert!(
            clean.html.contains(r#"id="postio-s7-masthead""#),
            "{}",
            clean.html
        );
    }

    /// `postio-latest` and `postio-blocked` are chrome markers; a message wearing either could
    /// impersonate the chrome.
    #[test]
    fn a_sender_cannot_wear_postios_names() {
        let clean = sanitize_body_in(
            r#"<p class="lede POSTIO-latest postio-blocked" id="postio-x">Hi</p><span id="Postio-y">!</span>"#,
            RemoteImages::Blocked,
            Some("7"),
        );
        let lowered = clean.html.to_ascii_lowercase();
        assert!(clean.html.contains(r#"class="lede""#), "{}", clean.html);
        assert!(!lowered.contains("postio-latest"), "{}", clean.html);
        assert!(!lowered.contains("postio-blocked"), "{}", clean.html);
        assert!(!lowered.contains("postio-x"), "{}", clean.html);
        assert!(!lowered.contains("postio-y"), "{}", clean.html);
    }

    #[test]
    fn ids_are_per_message_and_their_links_and_selectors_follow() {
        let html = r##"<style>#header { color: #123456 }</style><a href="#header">top</a><h1 id="header">News</h1>"##;
        let seven = sanitize_body_in(html, RemoteImages::Blocked, Some("7"));
        let eight = sanitize_body_in(html, RemoteImages::Blocked, Some("8"));
        assert!(
            seven.html.contains(r#"id="postio-s7-header""#),
            "{}",
            seven.html
        );
        assert!(
            eight.html.contains(r#"id="postio-s8-header""#),
            "{}",
            eight.html
        );
        assert!(
            seven.html.contains(r##"href="#postio-s7-header""##),
            "{}",
            seven.html
        );
        assert!(
            seven.styles.contains("#postio-s7-header"),
            "{}",
            seven.styles
        );
        assert!(!seven.styles.contains("#header "), "{}", seven.styles);
        let out = sanitize_body_in(
            r##"<a href="https://example.com/#header">out</a>"##,
            RemoteImages::Blocked,
            Some("7"),
        );
        assert!(
            out.html.contains(r##"href="https://example.com/#header""##),
            "{}",
            out.html
        );
    }

    /// An unscoped sender `id="7"` must not become `m-7`, Postio's own id for a message.
    #[test]
    fn an_unscoped_id_cannot_collide_with_postios_own() {
        let clean = sanitize_body(
            r#"<p id="7">x</p><p id="pos-1">y</p>"#,
            RemoteImages::Blocked,
        );
        assert!(clean.html.contains(r#"id="postio-s-7""#), "{}", clean.html);
        assert!(
            clean.html.contains(r#"id="postio-s-pos-1""#),
            "{}",
            clean.html
        );
    }

    /// Mail puts its white page on `<body>`; cleaning as a fragment dropped it while the dark text
    /// colours survived, giving black text on the dark ground.
    #[test]
    fn the_senders_page_becomes_the_messages_canvas() {
        let clean = sanitize_body_in(
            r##"<html style="color:#333"><body bgcolor="#ffffff" text="#222" link="#06c" style="background:#fafafa;margin:0"><p>Hi</p></body></html>"##,
            RemoteImages::Blocked,
            Some("7"),
        );
        assert_eq!(
            clean.canvas.background.as_deref(),
            Some("#fafafa"),
            "style wins over bgcolor"
        );
        assert_eq!(clean.canvas.text.as_deref(), Some("#222"));
        assert_eq!(clean.canvas.link.as_deref(), Some("#06c"));
        assert!(
            clean.canvas.style.contains("background-color: #fafafa"),
            "{}",
            clean.canvas.style
        );
        assert!(
            clean.canvas.style.contains("color: #222"),
            "{}",
            clean.canvas.style
        );
        assert!(
            clean.canvas.style.contains("margin:0"),
            "{}",
            clean.canvas.style
        );
        assert!(
            clean.styles.contains(&format!(
                "{} a {{ color: #06c }}",
                message_selector(Some("7"))
            )),
            "the link colour is a scoped rule: {}",
            clean.styles
        );
    }

    #[test]
    fn a_bgcolor_alone_is_the_canvas_and_html_is_the_fallback() {
        let body = sanitize_body(r#"<body bgcolor="White">x</body>"#, RemoteImages::Blocked);
        assert_eq!(body.canvas.background.as_deref(), Some("White"));
        let html = sanitize_body(
            r#"<html style="background-color:#eef1f4"><body>x</body></html>"#,
            RemoteImages::Blocked,
        );
        assert_eq!(html.canvas.background.as_deref(), Some("#eef1f4"));
        let none = sanitize_body("<p>x</p>", RemoteImages::Blocked);
        assert_eq!(none.canvas, Canvas::default());
    }

    #[test]
    fn a_canvas_attribute_that_is_not_a_colour_is_ignored() {
        let clean = sanitize_body(
            r#"<body bgcolor="red;position:fixed" text="url(https://beacon.example.com/x)">x</body>"#,
            RemoteImages::Blocked,
        );
        assert_eq!(clean.canvas.background, None);
        assert_eq!(clean.canvas.text, None);
        assert!(
            !clean.canvas.style.contains("position"),
            "{}",
            clean.canvas.style
        );
        assert!(
            !clean.canvas.style.contains("beacon"),
            "{}",
            clean.canvas.style
        );
    }

    #[test]
    fn the_canvas_style_goes_through_the_same_refusals() {
        let clean = sanitize_body(
            r#"<body style="position:fixed;width:100vw;background-image:url(https://beacon.example.com/b.png);color:#111">x</body>"#,
            RemoteImages::Blocked,
        );
        assert!(
            !clean.canvas.style.contains("position"),
            "{}",
            clean.canvas.style
        );
        assert!(
            !clean.canvas.style.contains("100vw"),
            "{}",
            clean.canvas.style
        );
        assert!(
            !clean.canvas.style.contains("beacon"),
            "{}",
            clean.canvas.style
        );
        assert!(
            clean.canvas.style.contains("color:#111"),
            "{}",
            clean.canvas.style
        );
        assert_eq!(
            clean.remote_blocked, 1,
            "a body background is a held-back image too"
        );
    }

    #[test]
    fn a_declared_color_scheme_is_carried() {
        for (content, expected) in [
            ("light dark", Some(ColorScheme::LightDark)),
            ("dark light", Some(ColorScheme::LightDark)),
            ("dark", Some(ColorScheme::Dark)),
            ("only light", Some(ColorScheme::Light)),
            ("normal", None),
        ] {
            let html = format!(
                r#"<head><meta name="color-scheme" content="{content}"></head><body>x</body>"#
            );
            assert_eq!(
                sanitize_body(&html, RemoteImages::Blocked).color_scheme,
                expected,
                "{content}"
            );
        }
        assert_eq!(
            sanitize_body("<p>x</p>", RemoteImages::Blocked).color_scheme,
            None
        );
    }

    fn provoke(what: Refused) -> (String, String) {
        match what {
            Refused::Element(name) => {
                let void = ["meta", "link", "base", "input"].contains(&name);
                let html = if void {
                    format!("<{name} data-probe=\"1\"><p>kept</p>")
                } else {
                    format!("<{name} data-probe=\"1\">inside</{name}><p>kept</p>")
                };
                (html, format!("<{name}"))
            }
            Refused::Attribute(ON_HANDLERS) => (
                r#"<p onclick="go()">t</p>"#.to_owned(),
                "onclick".to_owned(),
            ),
            Refused::Attribute(POSTIO_NAMES) => (
                r#"<p class="postio-latest">t</p>"#.to_owned(),
                "postio-latest".to_owned(),
            ),
            Refused::Attribute(SCRIPT_URLS) => (
                r#"<a href="javascript:go()">t</a>"#.to_owned(),
                "javascript".to_owned(),
            ),
            Refused::Attribute(other) => panic!("no probe for attribute {other}"),
            Refused::Property(name) => (
                format!(r#"<p style="{name}: 1; color: red">t</p>"#),
                format!("{name}:"),
            ),
            Refused::Unit(unit) => (
                format!(r#"<p style="width: 5{unit}">t</p>"#),
                format!("5{unit}"),
            ),
            Refused::AtRule(name) => (
                format!("<style>@{name} x;</style><p>t</p>"),
                format!("@{name}"),
            ),
            Refused::Resource(REMOTE_IMAGE) => (
                r#"<img src="https://beacon.example.com/x.png" alt="">"#.to_owned(),
                "beacon.example.com".to_owned(),
            ),
            Refused::Resource(FOREIGN_PARTS) => (
                r#"<p style="background:url(postio-cid:9/logo)">t</p><img src="POSTIO-CID:9/logo" alt="">"#.to_owned(),
                "postio-cid:9".to_owned(),
            ),
            Refused::Resource(NON_IMAGE_DATA) => (
                r#"<img src="data:text/html,hello" alt=""><a href="data:text/plain,hi">t</a>"#.to_owned(),
                "data:".to_owned(),
            ),
            Refused::Resource(other) => panic!("no probe for resource {other}"),
        }
    }

    #[test]
    fn every_listed_refusal_is_made_and_reported() {
        for (what, reason) in REFUSALS {
            let (html, gone) = provoke(*what);
            let clean = sanitize_body(&html, RemoteImages::Blocked);
            let output = format!("{}\n{}\n{}", clean.html, clean.styles, clean.canvas.style);
            assert!(
                !output
                    .to_ascii_lowercase()
                    .contains(&gone.to_ascii_lowercase()),
                "{what:?} ({reason:?}) is listed as refused but survived: {output}"
            );
            assert!(
                clean.refusals.contains(what),
                "{what:?} was refused but not reported: {:?}",
                clean.refusals
            );
        }
    }

    #[test]
    fn every_reason_is_one_the_spec_permits_and_each_is_used() {
        for reason in [Refusal::Containment, Refusal::Privacy, Refusal::NoScript] {
            assert!(
                REFUSALS.iter().any(|(_, r)| *r == reason),
                "{reason:?} is never used"
            );
        }
    }

    #[test]
    fn the_css_tables_are_reachable_through_the_list() {
        for (property, reason) in REFUSED {
            assert!(
                REFUSALS.contains(&(Refused::Property(property), *reason)),
                "{property}"
            );
        }
        for (rule, reason) in REFUSED_AT_RULES {
            assert!(
                REFUSALS.contains(&(Refused::AtRule(rule), *reason)),
                "@{rule}"
            );
        }
        for unit in REFUSED_UNITS {
            assert!(
                REFUSALS
                    .iter()
                    .any(|(what, _)| *what == Refused::Unit(unit)),
                "{unit}"
            );
        }
    }

    #[test]
    fn inline_svg_is_listed_with_its_reason() {
        assert!(REFUSALS.contains(&(Refused::Element("svg"), Refusal::NoScript)));
    }

    #[test]
    fn an_innocent_message_reports_no_refusals() {
        let clean = sanitize_body(
            r#"<p style="color:#123" class="lede">Hello</p>"#,
            RemoteImages::Blocked,
        );
        assert!(clean.refusals.is_empty(), "{:?}", clean.refusals);
    }

    /// ammonia's default schemes dropped `data:`, so every embedded logo vanished silently.
    #[test]
    fn an_embedded_image_survives_and_nothing_else_embedded_does() {
        let png = "data:image/png;base64,iVBORw0KGgo=";
        let clean = sanitize_body(
            &format!(r#"<img src="{png}" alt="logo">"#),
            RemoteImages::Blocked,
        );
        assert!(clean.html.contains(png), "{}", clean.html);
        assert!(clean.refusals.is_empty(), "{:?}", clean.refusals);

        let page = sanitize_body(
            r#"<img src="data:text/html,<b>x</b>" alt="">"#,
            RemoteImages::Blocked,
        );
        assert!(!page.html.contains("data:"), "{}", page.html);
        assert!(
            page.refusals.contains(&Refused::Resource(NON_IMAGE_DATA)),
            "{:?}",
            page.refusals
        );

        let link = sanitize_body(
            r#"<a href="data:text/html,<script>x</script>">t</a>"#,
            RemoteImages::Blocked,
        );
        assert!(
            !link.html.contains("data:"),
            "a data: link is a page, not an image: {}",
            link.html
        );
    }
}

#[cfg(test)]
mod cap_tests {
    use super::*;

    fn nested(depth: usize) -> String {
        format!("{}x{}", "<div>".repeat(depth), "</div>".repeat(depth))
    }

    #[test]
    fn a_body_within_the_caps_is_not_over_one() {
        let out = sanitize_body(&nested(200), RemoteImages::Blocked);
        assert_eq!(out.over_cap, None);
        assert!(out.html.contains('x'));
    }

    #[test]
    fn a_body_over_50_000_elements_is_over_the_element_cap() {
        let html = "<span>a</span>".repeat(50_001);
        let out = sanitize_body(&html, RemoteImages::Blocked);
        assert_eq!(out.over_cap, Some(Cap::Elements));
        assert!(out.html.is_empty(), "markup over a cap is not handed on");
    }

    #[test]
    fn a_body_nested_deeper_than_256_is_over_the_depth_cap() {
        let out = sanitize_body(&nested(300), RemoteImages::Blocked);
        assert_eq!(out.over_cap, Some(Cap::Depth));
    }

    #[test]
    fn a_body_nested_forty_thousand_deep_does_not_overflow_the_sanitizer() {
        let out = sanitize_body(&nested(40_000), RemoteImages::Blocked);
        assert!(out.over_cap.is_some());
    }

    #[test]
    fn thousands_of_unclosed_paragraphs_are_not_deep() {
        let html = format!("<div>{}</div>", "<p>line<li>item".repeat(3_000));
        let out = sanitize_body(&html, RemoteImages::Blocked);
        assert_eq!(out.over_cap, None);
    }

    #[test]
    fn a_body_over_2_mib_is_over_the_size_cap() {
        let html = format!("<p>{}</p>", "a".repeat(2 * 1024 * 1024));
        let out = sanitize_body(&html, RemoteImages::Blocked);
        assert_eq!(out.over_cap, Some(Cap::Bytes));
    }
}
