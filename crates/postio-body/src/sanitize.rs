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
//! * `<iframe>`, `<object>`, `<embed>`, `<svg>`, `<math>`, `<noscript>` and
//!   `<title>` are removed tag-and-contents too. `<noscript>` is the
//!   interesting one: with scripting off, the HTML spec has a browser parse
//!   its content as *markup* rather than inert text, which turns a sender's
//!   "if you can't run our JavaScript, at least load this" fallback into
//!   exactly the tracking pixel disabling JavaScript was supposed to stop.
//!   `<title>` is the dull one and was the long-lived one: dropping the
//!   element while keeping its text put the sender's document title in the
//!   body, at the top of most bulk mail.
//! * Every `src` is rewritten: a `cid:` reference becomes the app's own
//!   [`CID_SCHEME`], and — unless the caller passes [`RemoteImages::Allowed`]
//!   — anything pointing at a remote host is dropped outright rather than
//!   left for the network layer to refuse.
//! * `href` is never touched. Links stay live text; `view.rs` intercepts the
//!   click and hands it to the system browser instead of ever navigating the
//!   pane.

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

/// The class every sender's content is wrapped in (`contain_body`), and the
/// outermost thing a sender's own CSS is allowed to name.
///
/// Postio's chrome -- the message head, the blocked-images notice, the
/// per-message actions -- sits *outside* it, so a scoped rule naming any of
/// them is well-formed and matches nothing. That is the point: a message must
/// not be able to hide the notice saying its images were blocked.
pub const BODY_CLASS: &str = "postio-body";

/// The attribute `contain_body` stamps on that container to name the message.
///
/// [`message_selector`] is the other half. They are two constants rather than
/// one because one is written into HTML and the other into CSS, and a test in
/// `postio_ui::reader::document` asserts the container carries what the
/// selector matches -- the joint is exactly where this would rot silently.
pub const MESSAGE_ATTRIBUTE: &str = "data-postio-message";

/// The selector that confines a sender's stylesheet to their own message.
///
/// `None` is the single-message reader: one message in the document, so the
/// container alone is enough. A scope is a message's own database id, and it
/// is escaped rather than trusted -- an id carrying a quote would otherwise
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

/// The prefix every sender `id` is rewritten under (spec 006 FR-005).
///
/// Per message, so two messages in one conversation document cannot share
/// an id and one sender's `#header` cannot aim at another's element. Inside
/// Postio's own `postio-` namespace on purpose: a sender may not write a
/// `postio-` name ([`is_postio_name`]), so nothing a sender writes can
/// produce one of these, and none of Postio's own ids (`m-<scope>` for a
/// message, `pos-<n>` for a scroll marker) starts this way.
pub fn sender_id_prefix(scope: Option<&str>) -> String {
    match scope {
        Some(scope) => format!("postio-s{scope}-"),
        None => "postio-s-".to_owned(),
    }
}

/// Whether a class or id belongs to Postio's own namespace.
///
/// ASCII case-insensitive, because an HTML class match is case-sensitive but
/// a sender needs only one spelling that Postio's own CSS happens to match.
fn is_postio_name(name: &str) -> bool {
    name.get(..7)
        .is_some_and(|start| start.eq_ignore_ascii_case("postio-"))
}

/// A string safe to sit inside a double-quoted CSS string.
fn escape_css_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

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
    /// The sender's own stylesheets, rewritten so nothing in them reaches
    /// outside this message ([`crate::styles`]).
    ///
    /// Separate from [`Sanitized::html`] on purpose: the sender's `<style>`
    /// element never survives, and what a reader emits is CSS Postio parsed
    /// and rewrote itself. Passing the tag through would leave the engine
    /// reading the sender's text rather than Postio's, which is the whole
    /// difference between admitting a stylesheet and trusting one.
    ///
    /// Empty for a message with no `<style>` block, which is most of them.
    pub styles: String,
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
    /// The page the sender styled, lifted off `<html>` and `<body>` (spec 006
    /// FR-006). Empty when the sender styled no page.
    pub canvas: Canvas,
    /// The `color-scheme` the sender declared, if any (spec 006 FR-013(a)).
    pub color_scheme: Option<ColorScheme>,
    /// What this message asked for that was refused, each once, in
    /// [`REFUSALS`] order: counted, not silently lost (spec 006 FR-003).
    pub refusals: Vec<Refused>,
}

/// The page a sender styled: what `<html>` and `<body>` said, lifted onto the
/// message's own box because the sanitizer's output is a fragment and has no
/// `<body>` to carry it (spec 006 FR-006).
///
/// Only the attributes and inline style of the two elements. A `body { … }`
/// rule in the sender's stylesheet needs no lifting: `crate::styles` already
/// resolves it to the message's container.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Canvas {
    /// Declarations for the message's box, contained like any a sender
    /// writes: `bgcolor` and `text` as colours, then `<html>`'s and
    /// `<body>`'s own `style`.
    pub style: String,
    /// The page's background colour as written, if it is a colour.
    pub background: Option<String>,
    /// The page's text colour as written, if it is a colour.
    pub text: Option<String>,
    /// The page's link colour (`link=`), emitted as a scoped `a` rule.
    pub link: Option<String>,
}

/// A `color-scheme` a sender declared in `<meta name="color-scheme">`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorScheme {
    /// Light only.
    Light,
    /// Dark only.
    Dark,
    /// Both: the sender styled a dark variant.
    LightDark,
}

impl ColorScheme {
    /// The value as it goes in `data-postio-color-scheme`.
    pub fn as_str(self) -> &'static str {
        match self {
            ColorScheme::Light => "light",
            ColorScheme::Dark => "dark",
            ColorScheme::LightDark => "light dark",
        }
    }
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

/// Why a declaration a sender wrote does not reach the screen.
///
/// Spec FR-019b permits exactly these two reasons and no others. The point is
/// not the enum, it is that the set is *enumerable*: a property is refused by
/// appearing in [`REFUSED`] with a reason beside it, never by a judgement made
/// somewhere on the render path where no test can find it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// It would let the message act on something outside its own box —
    /// the container `crate::sanitize` puts every sender's content in.
    Containment,
    /// It would let the message reach the network or report on the reader.
    Privacy,
    /// It would run, or is only there to run: script, handlers, active
    /// content (spec 006 FR-002).
    NoScript,
}

/// One kind of thing the sanitizer refuses (spec 006 FR-005).
///
/// Each names what a sender wrote, not what Postio did about it; the reason
/// is beside it in [`REFUSALS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Refused {
    /// An element, removed with or without its text.
    Element(&'static str),
    /// A kind of attribute or attribute value.
    Attribute(&'static str),
    /// A CSS property ([`REFUSED`]).
    Property(&'static str),
    /// A CSS at-rule ([`REFUSED_AT_RULES`]).
    AtRule(&'static str),
    /// A CSS length unit ([`REFUSED_UNITS`]).
    Unit(&'static str),
    /// A kind of resource reference.
    Resource(&'static str),
}

/// Every `on…` event handler attribute.
pub const ON_HANDLERS: &str = "on*";
/// A class or id in Postio's own `postio-` namespace.
pub const POSTIO_NAMES: &str = "postio- names";
/// A link whose URL runs script: `javascript:`.
pub const SCRIPT_URLS: &str = "javascript: URLs";
/// A remote image, by `src`, `background` or CSS `url()`, while blocked.
pub const REMOTE_IMAGE: &str = "remote image";
/// A `postio-cid:` reference written by the sender rather than rewritten by
/// Postio: an attempt to name a part by Postio's own scheme, which under
/// ADR 0032's conversation document can be another message's.
pub const FOREIGN_PARTS: &str = "postio-cid: references";

/// Everything the sanitizer refuses, with the reason it may (spec 006 FR-005,
/// 001 FR-019b).
///
/// One list, so the set of removals is enumerable and a test can walk it:
/// every entry is provoked and must be both refused and reported in
/// [`Sanitized::refusals`]. [`REFUSED`], [`REFUSED_AT_RULES`] and
/// [`REFUSED_UNITS`] stay where the code that applies them reads them; a test
/// holds them to this list.
pub const REFUSALS: &[(Refused, Refusal)] = &[
    // Elements.
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
    // Attributes.
    (Refused::Attribute(ON_HANDLERS), Refusal::NoScript),
    (Refused::Attribute(SCRIPT_URLS), Refusal::NoScript),
    (Refused::Attribute(POSTIO_NAMES), Refusal::Containment),
    // CSS.
    (Refused::Property("position"), Refusal::Containment),
    (Refused::Property("z-index"), Refusal::Containment),
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
    // Resources.
    (Refused::Resource(REMOTE_IMAGE), Refusal::Privacy),
    (Refused::Resource(FOREIGN_PARTS), Refusal::Containment),
];

/// What one sanitize pass counted and refused, shared by the attribute
/// filter, the canvas and the stylesheet scoper.
#[derive(Debug, Default)]
pub(crate) struct Tally {
    /// Remote references held back, for the banner and the parts panel.
    pub(crate) blocked: AtomicU32,
    refused: Mutex<Vec<Refused>>,
    /// The message this pass sanitizes, for `cid:` rewriting in CSS.
    scope: Option<String>,
}

impl Tally {
    /// A tally for the message named `scope` (`None`: the single-message
    /// reader).
    pub(crate) fn for_scope(scope: Option<&str>) -> Tally {
        Tally {
            scope: scope.map(str::to_owned),
            ..Tally::default()
        }
    }

    /// `cid:` as this message's own part URI.
    pub(crate) fn part_uri(&self, content_id: &str) -> String {
        let id = percent_encode(
            content_id
                .trim()
                .trim_start_matches('<')
                .trim_end_matches('>'),
        );
        match &self.scope {
            // The separator is a literal `/`, and the encoded id can never
            // hold one -- see `sanitize_body_in`.
            Some(scope) => format!("{CID_SCHEME}:{scope}/{id}"),
            None => format!("{CID_SCHEME}:{id}"),
        }
    }

    /// A remote image held back: counted, and reported as refused.
    pub(crate) fn block(&self) {
        self.blocked.fetch_add(1, Ordering::Relaxed);
        self.refuse(Refused::Resource(REMOTE_IMAGE));
    }

    /// Note that `what` was refused. Each kind is reported once.
    pub(crate) fn refuse(&self, what: Refused) {
        let mut refused = self
            .refused
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !refused.contains(&what) {
            refused.push(what);
        }
    }

    /// Everything refused, in [`REFUSALS`] order.
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
    /// Why this is refused, from [`REFUSALS`].
    pub fn reason(self) -> Refusal {
        REFUSALS
            .iter()
            .find(|(what, _)| *what == self)
            .map(|(_, reason)| *reason)
            .expect("every Refused a sanitizer reports is listed in REFUSALS")
    }
}

/// The attributes a table-based layout is built from (spec FR-019a).
///
/// `width` and `height` carry the column proportions, `align`/`valign` the
/// placement, `bgcolor` the colour, and `cellpadding`/`cellspacing`/`border`
/// the spacing — between them four of the five things FR-019a says must
/// survive, for the one layout technique email actually uses.
///
/// The counterpart of [`REFUSED`]: that list is what a *declaration* may not
/// say, for a stated containment or privacy reason. This is what an
/// *attribute* may say, and nothing here reaches beyond the message's own
/// block or the network.
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

/// Every CSS property a sender may not set, with the reason it may not.
///
/// Matched on the property name only. A refusal drops that one declaration
/// and leaves the rest of the sender's rule alone: refusing `position` is not
/// licence to discard the `color` written beside it.
pub const REFUSED: &[(&str, Refusal)] = &[
    // Both position against something outside the message: the viewport, or
    // an ancestor the message does not own. Either one lifts content out of
    // the box `contain_body` draws around it (#323), which is the edge a
    // reader uses to tell Postio's words from a sender's.
    ("position", Refusal::Containment),
    // Stacking order is how a message would draw *over* the application's own
    // chrome rather than beside it.
    ("z-index", Refusal::Containment),
];

/// Every at-rule a sender may not use, with the reason it may not.
///
/// The counterpart of [`REFUSED`] for the other half of a stylesheet, and
/// deliberately the same shape and the same [`Refusal`] enum: spec FR-019b's
/// point is that the set of refusals is *enumerable*, and two tables in one
/// place is one place. Nothing here is refused by a judgement made somewhere
/// on the render path.
///
/// Not exhaustive, and does not need to be. [`crate::styles`] admits a named
/// set — `@media`, `@supports`, `@container`, `@layer`, `@keyframes` — and
/// refuses everything else by omission, which is the safe direction: a CSS
/// feature Postio has never heard of is not one it can reason about the reach
/// of. This table is the subset that has been thought about and has a reason
/// worth writing down.
pub const REFUSED_AT_RULES: &[(&str, Refusal)] = &[
    // Fetches when the stylesheet parses, carrying the referer and the
    // reader's IP. It needs no `<img>`, so neither `contain_declarations` nor
    // the document's `img-src` touches it.
    ("import", Refusal::Privacy),
    // Same fetch, one indirection later: a `src` naming a remote host is a
    // request made the moment a glyph is needed. ADR 0023 has Postio serve
    // its own faces rather than fetch them; a sender does not get an
    // exception to that.
    ("font-face", Refusal::Privacy),
    // Rebinds what element names mean, which is the one thing that could make
    // a scoped selector match something other than what it reads as.
    ("namespace", Refusal::Containment),
    // Both speak for the whole document rather than for one message in it.
    ("charset", Refusal::Containment),
    ("page", Refusal::Containment),
];

/// Not here on purpose: `top`, `right`, `bottom`, `left` and `inset`.
///
/// They were in the first draft of this table and should not have been. They
/// offset an element against its containing block, and with `position`
/// refused every element stays `static`, where an offset does nothing at all.
/// Refusing them buys no containment and costs fidelity — a sender's
/// `top: 0` inside their own relatively-positioned card is ordinary layout.
/// Refuse what grants the power, not what depends on it.
///
/// Units that answer to the window rather than to the message's own box.
/// A refusal by *value* rather than by property, because `width` is
/// unremarkable until it is `100vw` — at which point a message is deciding
/// how wide the reading pane is.
pub const REFUSED_UNITS: &[&str] = &[
    "vw", "vh", "vmin", "vmax", "svw", "svh", "lvw", "lvh", "dvw", "dvh",
];

/// Sanitize one HTML body for the reading pane.
///
/// `cid:` references become [`CID_SCHEME`] URIs; `postio_gtk::reader::scheme` resolves
/// those against the message's local parts (or answers 404 for a dangling
/// reference — the corpus has one on purpose).
pub fn sanitize_body(html: &str, remote: RemoteImages) -> Sanitized {
    sanitize_body_in(html, remote, None)
}

/// [`sanitize_body`], naming the message the body belongs to.
///
/// A `postio-cid:` URI names a `Content-ID` and nothing else, which is exact
/// while one document is one message. ADR 0032 puts a whole thread in one
/// document, and then it is not: two messages in a thread may each carry a
/// part called `logo`, and a sender may reference a `Content-ID` they know
/// belongs to someone else's message in the same thread.
///
/// `scope` stamps the message on every rewritten reference, so the handler
/// resolves against *that* message's parts rather than against whichever one
/// happens to be open. `/` is the separator and is safe: `percent_encode`
/// escapes it, so an encoded `Content-ID` never contains a literal one and
/// the split cannot be confused however odd the id.
///
/// The scope is Postio's and is applied on the way out, so a body that
/// arrives already naming a message does not keep that name — it is
/// percent-encoded into the id like any other sender text.
///
/// `None` is the single-message reader, and produces exactly what
/// [`sanitize_body`] always did.
pub fn sanitize_body_in(html: &str, remote: RemoteImages, scope: Option<&str>) -> Sanitized {
    // Owned: the filter is a `'static` closure and cannot borrow the caller's.
    let scope_for_styles = scope.map(str::to_owned);
    let scope = scope.map(str::to_owned);
    let tally = Arc::new(Tally::for_scope(scope.as_deref()));
    let counter = Arc::clone(&tally);
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
        // `title` is here rather than in `rm_tags` because the two differ in
        // exactly the way that matters: `rm_tags` drops the element and keeps
        // its text, which for a `<title>` means the sender's document title
        // becomes the first line of their message.
        .add_clean_content_tags([
            "iframe", "object", "embed", "svg", "math", "noscript", "title",
        ])
        // `cid` has to be allowed on the *input* side too: ammonia checks
        // `url_schemes` against a URL attribute's original value before
        // `attribute_filter` ever runs, so a `cid:` reference would be
        // stripped before this module gets a chance to rewrite it to
        // `CID_SCHEME`. Nothing re-validates the filter's output against
        // `url_schemes` afterward, so `CID_SCHEME` itself does not need to be
        // listed here — it is added anyway, for the reader it is documenting
        // intent to.
        .add_url_schemes(["cid", CID_SCHEME])
        // The sender's own styling, admitted on every element (spec FR-019).
        // An inline declaration needs no scoping of its own: it applies to
        // the element it sits on, which is already inside the container
        // `contain_body` draws around this message. What it still needs is
        // the refusals below, which is what `contain_declarations` is for.
        .add_generic_attributes(["style"])
        // Spec 006 FR-005, #1545: a sender's stylesheet selects on the classes
        // and ids its own markup carries, so stripping them made every such
        // rule dead. Kept, except that a sender may not wear Postio's names
        // and every id is rewritten per message -- see `rewrite_attribute`.
        .add_generic_attributes(["class", "id"])
        // The layout attributes a table-based message arranges itself with
        // (spec FR-019a). HTML email is table-based because that is what
        // renders in Outlook, so dropping these did not cost an exotic
        // newsletter its polish -- it collapsed the ordinary one into a single
        // column. Ammonia's per-tag defaults do not carry them and
        // `add_generic_attributes` only added `style`.
        //
        // FR-019b is why they come back rather than staying dropped: a
        // refusal needs a stated reason, containment or privacy, and
        // "ammonia's default list did not mention it" is neither. What makes
        // admitting them safe is that containment is enforced elsewhere and
        // does not depend on this list -- `contain_body`'s non-visible
        // overflow holds an over-wide table inside its own message's block
        // (#1334), and `contain_declarations` refuses the properties that
        // escape one. None of these reach the network.
        .add_tags(["colgroup", "col"])
        // `<font>` keeps only the style `crate::hints` wrote from its
        // attributes; `<center>` is the one layout element mail still uses.
        // Neither reaches beyond its own text.
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

    // Taken from the DOM before ammonia runs, because ammonia removes
    // `<style>` tag-and-contents, returns a fragment with no `<html>` or
    // `<body>` left to read, and removes `<meta>`. One walk collects all
    // three. The scoped result is returned beside the markup rather than
    // spliced back into it -- see `Sanitized::styles`.
    let (facts, dom) = document_facts(html, &tally);
    // Presentational attributes into the style they mean (spec 006 FR-007),
    // on the same parse; serialized again only if one was found.
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
        html: builder.clean(input).to_string(),
        styles,
        remote_blocked: tally.blocked.load(Ordering::Relaxed),
        trackers: tracker_count.load(Ordering::Relaxed),
        canvas,
        color_scheme: facts.color_scheme,
        refusals: tally.refusals(),
    }
}

/// What the sanitizer needs from the whole document before ammonia reduces
/// it to a fragment: the sender's stylesheets, their page, and the colour
/// scheme they declared.
#[derive(Default)]
struct DocumentFacts {
    /// Every `<style>` element's text, in document order, joined -- joined
    /// rather than kept apart because they are scoped identically and a
    /// browser would cascade them in this order anyway. `<style>` inside
    /// `<template>` or an already-removed subtree is not special-cased: it is
    /// still this sender's CSS, and it still ends up scoped to this sender's
    /// message.
    stylesheets: String,
    page: Page,
    color_scheme: Option<ColorScheme>,
}

/// What `<html>` and `<body>` said about the page, raw.
#[derive(Default)]
struct Page {
    bgcolor: Option<String>,
    text: Option<String>,
    link: Option<String>,
    html_style: Option<String>,
    body_style: Option<String>,
}

/// An attribute's value, if it is a colour.
fn colour(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|v| is_colour(v))
}

impl Page {
    /// The canvas: colours validated as colours, styles contained like any
    /// declaration a sender writes. The body's own style outranks its
    /// attributes, and both outrank `<html>`'s, as they would in a browser.
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

fn document_facts(html: &str, tally: &Tally) -> (DocumentFacts, RcDom) {
    let dom = parse_document(RcDom::default(), ParseOpts::default()).one(html);
    let mut facts = DocumentFacts::default();
    collect_facts(&dom.document, &mut facts, tally);
    (facts, dom)
}

/// The document as markup again, after the hints rewrote it.
fn serialize_document(dom: &RcDom) -> String {
    let mut bytes = Vec::new();
    let handle: SerializableHandle = dom.document.clone().into();
    let options = SerializeOpts {
        traversal_scope: TraversalScope::ChildrenOnly(None),
        ..SerializeOpts::default()
    };
    // Writing into a Vec cannot fail.
    let _ = serialize(&mut bytes, &handle, options);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The refused elements, as [`REFUSALS`] names them.
fn refused_element(name: &str) -> Option<&'static str> {
    REFUSALS.iter().find_map(|(what, _)| match what {
        Refused::Element(element) if element.eq_ignore_ascii_case(name) => Some(*element),
        _ => None,
    })
}

fn collect_facts(node: &Handle, facts: &mut DocumentFacts, tally: &Tally) {
    if let NodeData::Element { name, attrs, .. } = &node.data {
        let element = name.local.as_ref();
        let attrs = attrs.borrow();
        let get = |wanted: &str| {
            attrs
                .iter()
                .find(|attr| attr.name.local.as_ref().eq_ignore_ascii_case(wanted))
                .map(|attr| attr.value.to_string())
        };
        // What ammonia will remove without a filter ever seeing it, noted
        // here so the report says so (spec 006 FR-003).
        if let Some(refused) = refused_element(element) {
            tally.refuse(Refused::Element(refused));
        }
        for attr in attrs.iter() {
            let name = attr.name.local.as_ref();
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

/// Whether a URL runs script when followed, however its scheme is spelled:
/// browsers ignore ASCII whitespace and control characters inside it.
fn runs_script(url: &str) -> bool {
    let compact: String = url
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && !c.is_ascii_control())
        .take(11)
        .collect();
    compact.to_ascii_lowercase().starts_with("javascript:")
}

/// A `color-scheme` value, read the way CSS reads it: the keywords present,
/// in any order, with `only` and `normal` meaning nothing to declare.
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

/// The colour a style declares for the first of `properties` that holds a
/// colour and nothing else.
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

/// Whether a value is a colour and nothing more: a hex colour, a keyword, or
/// an `rgb()`/`hsl()`-family function of numbers.
///
/// Conservative on purpose. The value goes into a `style` Postio writes, and
/// an attribute is attacker-controlled text: `red;position:fixed` is not a
/// colour, and neither is anything with a `url(` in it.
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
        if is_postio_name(id) {
            tally.refuse(Refused::Attribute(POSTIO_NAMES));
            return None;
        }
        if id.is_empty() {
            return None;
        }
        return Some(Cow::Owned(format!("{}{id}", sender_id_prefix(scope))));
    }
    if attribute == "href"
        && let Some(fragment) = value.trim().strip_prefix('#')
        && !fragment.is_empty()
    {
        // An in-message link follows its target's rewritten id. A link out
        // of the message is not a fragment and is never touched.
        return Some(Cow::Owned(format!(
            "#{}{fragment}",
            sender_id_prefix(scope)
        )));
    }
    if attribute != "src" {
        return Some(Cow::Borrowed(value));
    }
    // Postio's own scheme, written by the sender. There is no legitimate
    // reason for it to appear in arriving markup -- `cid:` is what a message
    // uses -- and it is an attempt to address the reader's internals: under
    // ADR 0032's one-document conversation it names *another message's*
    // parts, and even in a single-message document it reaches past what the
    // rewrite below decides. Dropped rather than rewritten, because a
    // reference nobody can justify is not one to guess the intent of.
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

/// Keep the declarations a sender may set, drop the ones they may not.
///
/// Whole declarations, one at a time. Refusing `position` is not licence to
/// discard the `color` written beside it — a message that loses its palette
/// because it also tried to pin itself is a message rendered wrongly, and the
/// user cannot tell that from a sender who never set a colour.
pub(crate) fn contain_declarations(value: &str, remote: RemoteImages, tally: &Tally) -> String {
    let mut kept: Vec<Cow<'_, str>> = Vec::new();
    for declaration in split_declarations(value) {
        let Some((property, declared)) = declaration.split_once(':') else {
            // Not a declaration at all. Dropped rather than guessed at.
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
        if let Some(url) = css_url(declared) {
            if names_foreign_parts(&url) {
                tally.refuse(Refused::Resource(FOREIGN_PARTS));
                continue;
            }
            if is_remote(&url) && remote == RemoteImages::Blocked {
                // Counted with the images, because that is what it is: the
                // panel says "6 remote images blocked" and a background is
                // one of them.
                tally.block();
                continue;
            }
            if let Some(id) = url.trim().strip_prefix("cid:") {
                // A sender's own inline part, named from CSS: the same
                // rewrite a `src` gets, so it resolves against this message.
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

/// Split on `;`, except inside `url(...)` or a quoted string.
///
/// A naive `split(';')` is wrong on the one value that matters most here:
/// `url(data:image/png;base64,...)` carries a semicolon of its own, and
/// cutting there turns an inline image into two fragments of nonsense.
fn split_declarations(value: &str) -> Vec<&str> {
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

/// Whether a value sizes itself against the window rather than its own box.
///
/// Matched as a unit suffix on a number, so a `font-family: "Vivaldi"` is not
/// mistaken for one on the strength of containing `vi`.
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

/// Whether a reference uses Postio's own part scheme, however it is spelled.
fn names_foreign_parts(value: &str) -> bool {
    value
        .trim_start()
        .to_ascii_lowercase()
        .starts_with(&format!("{CID_SCHEME}:"))
}

/// `value` with its first `url(...)` pointing at `uri` instead.
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

/// The URL a value references, if it references one.
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
pub(crate) fn is_remote(value: &str) -> bool {
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

    /// FR-019a: what a sender arranges MUST reach the screen (#1396).
    ///
    /// The maintainer's fourth requirement for this pane was *"the mail should
    /// render the layout the sender intended"*, and the spec names the
    /// minimum: structural layout, colour, typographic emphasis and font
    /// choice, and spacing — *"a message that arranges itself in three columns
    /// MUST appear in three columns"*.
    ///
    /// The table case is the one that matters. HTML email is table-based
    /// because that is what renders in Outlook; a newsletter laying itself out
    /// in `display:flex` is the rarity. Dropping table attributes passes the
    /// technique almost nobody uses and fails the one almost everybody does.
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

    /// The other five of FR-019a's list, which arrive as inline declarations
    /// and already survived — asserted so that tightening
    /// `contain_declarations` for a containment reason cannot quietly take
    /// one of them with it.
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

    /// A `<style>` element does not survive, and neither does its `@import`
    /// (#1383).
    ///
    /// This is where the reader's only route to a CSS-borne fetch is closed
    /// today. `@import` is valid only inside a stylesheet, and the admitted
    /// route for a sender's styling is the inline `style` *attribute*, which
    /// cannot carry one — so the element going is what makes
    /// `style-src 'unsafe-inline'` an unexercised second layer rather than the
    /// only thing standing between a sender and an open-rate beacon that needs
    /// no pixel.
    ///
    /// Written down because #1326 is about admitting `<style>` blocks, and the
    /// day that lands this test should fail and be replaced by one that proves
    /// the `@import` is stripped from a stylesheet Postio does admit.
    #[test]
    fn a_style_element_is_admitted_scoped_and_never_as_markup() {
        // Replaces `a_style_element_and_its_import_do_not_survive` (#1383),
        // which asserted `<style>` was dropped whole. That was true and is
        // the reason a sender had no route to a CSS-borne fetch at all; #1326
        // admits the block, so the assertion that still matters is that
        // `@import` is stripped from a stylesheet Postio now *does* serve.
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
        // The control: the attribute route is admitted too, so the assertions
        // above are about `<style>` rather than about styling being dropped
        // wholesale -- which would make them pass for the wrong reason.
        assert!(
            clean.html.contains("color:green") || clean.html.contains("color: green"),
            "an inline style attribute must still survive, or this test is \
             passing because nothing styled anything: {}",
            clean.html
        );
    }

    #[test]
    fn a_scope_cannot_break_out_of_the_selector_it_names() {
        // A scope is a message's own database id, so a quote in one is not a
        // sender's doing today. It is escaped anyway, for the same reason
        // `message_anchor` escapes it: an unescaped value would close the
        // attribute selector and free every rule written after it, and the
        // day one of those assumptions stops holding is not the day to find
        // out.
        let selector = message_selector(Some(r#"a" ] , * { color: red } x["#));
        let opening = format!(".{BODY_CLASS}[{MESSAGE_ATTRIBUTE}=\"");
        let inside = selector
            .strip_prefix(&opening)
            .and_then(|rest| rest.strip_suffix("\"]"))
            .unwrap_or_else(|| panic!("not one attribute selector: {selector}"));
        // Escaped pairs removed first, so what is counted is the quotes that
        // would actually close the string. Counting raw `"` would call the
        // escaped one a breakout and the test would fail on correct code.
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
        // The single-message pane has one message and no scope, but the
        // chrome outside `.postio-body` is the same chrome and a sender must
        // not be able to name it there either.
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
        // This assertion used to be `remote_blocked == 0`, with the reason
        // "a stripped <style> is not a stripped *image*: the banner has
        // nothing to report about a message that never referenced one".
        // #1326 makes that premise false. The message did reference one --
        // in CSS -- and now that Postio parses the CSS it can see that. The
        // banner should say so, because from the reader's side a background
        // that did not load is exactly as blocked as an `<img>` that did not.
        assert_eq!(out.remote_blocked, 1);
    }

    #[test]
    fn an_inline_style_survives_so_the_senders_layout_does() {
        // Was `an_inline_style_attribute_is_stripped_so_postio_css_always_wins`.
        // Postio's CSS no longer always wins: a message renders as its sender
        // built it (spec FR-019, FR-019a), and a newsletter that arrives as
        // one column when it was written as three is the thing that decision
        // exists to fix.
        let out = sanitize_body(r#"<p style="color:red">hi</p>"#, RemoteImages::Blocked);
        assert!(out.html.contains("color"), "{}", out.html);
    }

    #[test]
    fn layout_colour_and_spacing_all_survive() {
        // FR-019a's floor, in one message: structural layout, colour,
        // typographic emphasis and spacing.
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
        // `fixed` and `sticky` both position against something outside the
        // message's own box, which is how a message escapes the container
        // `contain_body` puts it in (#323).
        for escape in ["position:fixed", "position: sticky"] {
            let out = sanitize_body(
                &format!(r#"<p style="{escape};top:0">hi</p>"#),
                RemoteImages::Blocked,
            );
            assert!(
                !out.html.contains("position"),
                "{escape} survived: {}",
                out.html
            );
            // The rest of the declaration is untouched -- refusing a property
            // is not licence to drop the ones beside it.
            assert!(out.html.contains("top"), "{}", out.html);
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
        // Viewport units answer to the window, not to the message's box, so
        // `100vw` is a message deciding how wide the pane is.
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
    fn every_refused_property_states_a_reason() {
        // FR-019b: a property may be refused for containment or for privacy,
        // and for nothing else. "Dropped because it was easier" is what this
        // test exists to make impossible to add quietly.
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
        // With scripting off, unwrapped <noscript> content is exactly the
        // beacon disabling JavaScript was meant to stop.
        let out = sanitize_body(
            r#"<noscript><img src="https://tracker.example.org/o.gif"></noscript><p>body</p>"#,
            RemoteImages::Allowed,
        );
        assert_eq!(out.html, "<p>body</p>");
    }

    /// A `<title>` is the document's name, not a line of the message.
    ///
    /// Ammonia drops an element that is not allow-listed but *keeps its text*
    /// — that is what makes unwrapping safe for a `<span>`. For a `<title>` it
    /// means the sender's document title arrives as the first words of their
    /// body, which is how `html-newsletter.eml` came to render "Harbour
    /// Lantern Weekly - Issue 214" above its own masthead. Every template
    /// builder emits one, so this is most bulk mail, not an edge.
    ///
    /// `<title>` alone, because it is the only head element with text that
    /// was not already handled: ammonia's own defaults remove `<script>` and
    /// `<style>` tag-and-contents, and `link`, `base` and `meta` are stripped
    /// as elements and carry no text to keep.
    #[test]
    fn a_documents_title_is_not_a_line_of_its_body() {
        let out = sanitize_body(
            "<html><head><title>Weekly digest</title></head>\
             <body><p>body</p></body></html>",
            RemoteImages::Blocked,
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

#[cfg(test)]
mod conversation_scope_tests {
    use super::*;

    /// One document holding a whole thread needs `cid:` to say *whose*.
    ///
    /// A `postio-cid:` URI names a `Content-ID` and nothing else, and the
    /// handler resolves it against whichever message is open. That is exact
    /// when one document is one message. Put a thread in one document (ADR
    /// 0032) and it is ambiguous: two messages may each carry a part called
    /// `logo`, and a sender may reference a `Content-ID` they know belongs to
    /// somebody else's message in the same thread.
    ///
    /// So a scoped sanitize stamps the message on the URI. The separator is
    /// `/`, which is safe because `percent_encode` escapes it -- a
    /// `Content-ID` can never contain a literal one, so the split is
    /// unambiguous however odd the id.
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

    /// Unscoped is what a single-message reader still asks for, unchanged.
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

    /// The scope is Postio's, never the sender's: it is stamped on the way
    /// out, so markup that arrives already naming another message cannot
    /// keep it.
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

    /// #1545: a sender's stylesheet selects on the classes and ids its own
    /// markup carries, and stripping them made every such rule dead. Spec
    /// 006 FR-005: kept, unless keeping one is a containment problem.
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

    /// A sender element may not wear Postio's own names. `postio-latest` is
    /// the marker on the newest message and `postio-blocked` the notice that
    /// images were held back: a message dressing itself in either could
    /// impersonate the chrome (spec 006 FR-025, 001 FR-025).
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

    /// Two messages in one conversation document both saying `id="header"`
    /// would be two elements with one id, and one sender's `#header` rule
    /// would be aimed at the other's element. Rewritten per message, and the
    /// sender's in-message links and selectors follow.
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
        // A link out of the message is not a fragment and is left alone.
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

    /// The single-message reader has no scope, and still must not produce an
    /// id Postio uses itself: `m-<scope>` for a message, `pos-<n>` for scroll
    /// markers. A sender `id="7"` must not become `m-7`.
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

    /// Spec 006 FR-006: the page a sender styled is their canvas.
    ///
    /// Mail puts its white page on `<body>`, and cleaning the body as a
    /// fragment dropped `<body>` with everything on it, while the dark text
    /// colours written inside survived -- which is exactly black text on
    /// the dark reader ground.
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

    /// An attribute is attacker-controlled text that ends up in a `style`:
    /// only a colour is a colour.
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

    /// The canvas is contained like any declaration a sender writes.
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

    /// Spec 006 FR-013(a): a sender who declares dark support gets their own
    /// dark design, so the declaration has to survive `<meta>` removal.
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

    /// A snippet that asks for `what`, and the text that must not survive it.
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
            Refused::Resource(other) => panic!("no probe for resource {other}"),
        }
    }

    /// Spec 006 FR-005 / 001 FR-019b: every removal is listed with its reason,
    /// and every listed removal is one the sanitizer actually makes -- and
    /// says it made. "Dropped because it was easier" is not in the list
    /// because it is not a reason.
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

    /// The per-kind tables stay where the code that applies them reads them;
    /// this is the one list that says what they add up to.
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

    /// A message with nothing to refuse reports nothing.
    #[test]
    fn an_innocent_message_reports_no_refusals() {
        let clean = sanitize_body(
            r#"<p style="color:#123" class="lede">Hello</p>"#,
            RemoteImages::Blocked,
        );
        assert!(clean.refusals.is_empty(), "{:?}", clean.refusals);
    }
}
