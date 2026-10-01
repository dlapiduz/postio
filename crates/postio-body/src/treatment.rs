//! How a message body is shown: in the app's own colours, or as sent, on
//! paper (the message dialog redesign, T205-T214 in
//! `specs/007-postio-focus/tasks.md`).
//!
//! The rule is that the sender's colours never meet the app's surface, so
//! every body gets exactly one of the two, in light mode as in dark. Which one
//! decides the reading column's width too, which is why the decision is a
//! value of its own rather than a flag inside the renderer.
//!
//! Everything here is a pure function of markup that has **already been
//! sanitised** ([`crate::sanitize`]): scripts, forms, handlers and remote
//! content are gone before anything here looks at it, so neither treatment
//! can bring one back.
//!
//! * [`classify`] decides: paper if the HTML paints its own page, has a
//!   fixed-width layout table at least [`LAYOUT_TABLE_MIN`] wide, or an image
//!   wider than [`WIDE_IMAGE`]; app colours otherwise (T210).
//! * [`app_colours`] and [`app_colours_css`] are the app-colours treatment:
//!   the sender's colours, backgrounds, faces, sizes and line heights go, the
//!   structure stays (T211).
//! * [`light_only`] is the paper treatment's half of the sender's stylesheet:
//!   a sheet that is always light keeps only the light design (T212).

use std::collections::HashSet;

use html5ever::driver::ParseOpts;
use html5ever::parse_document;
use html5ever::serialize::{SerializeOpts, TraversalScope, serialize};
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::{Attribute, LocalName, QualName, ns};
use markup5ever_rcdom::{Handle, NodeData, RcDom, SerializableHandle};

use crate::sanitize::{BODY_CLASS, Sanitized, split_declarations};

/// One body's treatment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Treatment {
    /// The sender's colours, backgrounds and fonts removed; the app's ink on
    /// the dialog's surface, in a 480px column. Plain text always, and HTML
    /// that does not paint its own page.
    #[default]
    AppColours,
    /// Rendered as sent on a white sheet, dimmed a little in dark mode and
    /// never inverted, in a column up to 640px. HTML that paints its own page.
    Paper,
}

/// The attribute the reader stamps on a body's container to say which
/// treatment it is drawn in, and what the renderer reads back.
///
/// Postio's own name: the sanitiser keeps no `data-` attribute a sender
/// writes, so a message cannot claim a treatment for itself.
pub const TREATMENT_ATTRIBUTE: &str = "data-postio-treatment";

impl Treatment {
    /// The value [`TREATMENT_ATTRIBUTE`] carries for this treatment.
    pub fn attribute_value(self) -> &'static str {
        match self {
            Treatment::AppColours => "app",
            Treatment::Paper => "paper",
        }
    }

    /// The treatment a [`TREATMENT_ATTRIBUTE`] value names, if it names one.
    pub fn from_attribute(value: &str) -> Option<Treatment> {
        match value {
            "app" => Some(Treatment::AppColours),
            "paper" => Some(Treatment::Paper),
            _ => None,
        }
    }

    /// The other one: what `⇧O` switches to.
    pub fn other(self) -> Treatment {
        match self {
            Treatment::AppColours => Treatment::Paper,
            Treatment::Paper => Treatment::AppColours,
        }
    }
}

/// Why a body was put on paper: the first of the rule's triggers it met.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// A background colour or image on the page itself: `<body>` or
    /// `<html>`, or a `body` rule in the sender's stylesheet.
    PageBackground,
    /// A background on a block that holds most of the message's text: a
    /// wrapper table, a container `div`.
    CoveringBackground,
    /// A layout table, or a cell in one, fixed at [`LAYOUT_TABLE_MIN`]
    /// pixels or wider.
    WideTable,
    /// An image wider than [`WIDE_IMAGE`] pixels.
    WideImage,
}

impl Trigger {
    /// Whether this trigger is about colour (the page paints itself) rather
    /// than about layout: what the render-mode line says about it.
    pub fn is_background(self) -> bool {
        matches!(self, Trigger::PageBackground | Trigger::CoveringBackground)
    }
}

/// The narrowest fixed layout table that puts a body on paper, in CSS
/// pixels. A layout this wide was built for a page, not for a 480px column.
pub const LAYOUT_TABLE_MIN: f32 = 480.0;

/// An image wider than this, in CSS pixels, puts a body on paper.
pub const WIDE_IMAGE: f32 = 300.0;

/// The share of a message's text a background has to sit behind to count as
/// covering most of the content.
pub const MOST_OF_THE_CONTENT: f32 = 0.5;

/// A background at or above this relative luminance is a client's default
/// white page, not a design: a reply stamped `bgcolor="#ffffff"` is
/// correspondence (`html-white-page-reply` in the corpus), and putting it on
/// paper would only take the app's ink away from it.
pub const PLAIN_WHITE: f64 = 0.95;

/// Which treatment a sanitised HTML body gets (T210).
///
/// Paper if any of [`paper_trigger`]'s triggers is met; app colours
/// otherwise. The same answer in light mode and dark: nothing here knows the
/// theme, so a message looks the same in both apart from the surface.
pub fn classify(sanitized: &Sanitized) -> Treatment {
    match paper_trigger(sanitized) {
        Some(_) => Treatment::Paper,
        None => Treatment::AppColours,
    }
}

/// The treatment a whole message body gets: app colours when it has no HTML
/// to speak of, else [`classify`] over its HTML, sanitised with remote
/// images blocked.
///
/// The reader classifies the sanitised result it already has, under the
/// policy it drew with; this is the same rule for a caller that holds only
/// the body.
pub fn classify_body(body: &postio_model::message::MessageBody) -> Treatment {
    match body.html.as_deref().filter(|html| !html.trim().is_empty()) {
        Some(html) => classify(&crate::sanitize::sanitize_body(
            html,
            crate::sanitize::RemoteImages::Blocked,
        )),
        None => Treatment::AppColours,
    }
}

/// The first paper trigger a sanitised body meets, if any.
///
/// The page's own background is read from [`Sanitized::canvas`], because the
/// sanitiser lifts it off `<body>`, and from the sender's scoped stylesheet
/// ([`Sanitized::styles`]), where a `body` rule names the message's own
/// container. Everything else is read from the markup.
///
/// Read after sanitising, so a remote background image counts only while
/// remote images are allowed: blocked, it is not in the markup to paint.
pub fn paper_trigger(sanitized: &Sanitized) -> Option<Trigger> {
    let canvas = &sanitized.canvas;
    let page_paints = canvas.background.as_deref().is_some_and(colour_paints)
        || declarations(&canvas.style)
            .iter()
            .any(|(property, value)| background_paints(property, value));
    let rules = BackgroundRules::from(&sanitized.styles);
    if page_paints || rules.page {
        return Some(Trigger::PageBackground);
    }
    let dom = parse_document(RcDom::default(), ParseOpts::default()).one(sanitized.html.as_str());
    let total = text_length(&dom.document);
    let mut found: Option<Trigger> = None;
    find_triggers(&dom.document, total, &rules, &mut found);
    found
}

/// What the sender's scoped stylesheet paints: the page itself, or blocks
/// named by a class or id.
#[derive(Default)]
struct BackgroundRules {
    /// A rule on the message's container -- a `body` or `html` rule, before
    /// scoping -- paints a background.
    page: bool,
    classes: HashSet<String>,
    ids: HashSet<String>,
}

impl BackgroundRules {
    fn from(styles: &str) -> BackgroundRules {
        let mut rules = BackgroundRules::default();
        for item in items(styles) {
            // Top level only: a background under a media query is a
            // responsive or dark-mode variant, not the page as designed.
            let Item::Block { prelude, body } = item else {
                continue;
            };
            if prelude.trim_start().starts_with('@') {
                continue;
            }
            if !declarations(body)
                .iter()
                .any(|(property, value)| background_paints(property, value))
            {
                continue;
            }
            for selector in prelude.split(',') {
                let rest = unscoped(selector.trim());
                if rest.is_empty() {
                    rules.page = true;
                } else if let Some(last) = rest.split_whitespace().last() {
                    // The element a rule lands on is named by its last
                    // compound; a simple `.class`, `tag.class` or `#id` is
                    // all this looks for.
                    if let Some((_, class)) = last.split_once('.') {
                        rules.classes.insert(class.to_owned());
                    } else if let Some((_, id)) = last.split_once('#') {
                        rules.ids.insert(id.to_owned());
                    }
                }
            }
        }
        rules
    }
}

/// A scoped selector without the message-container prefix the sanitiser
/// put in front of it ([`crate::sanitize::message_selector`]).
fn unscoped(selector: &str) -> &str {
    let Some(rest) = selector.strip_prefix(&format!(".{BODY_CLASS}")) else {
        return selector;
    };
    // `[data-postio-message="…"]` when the message is scoped.
    let rest = match rest.strip_prefix('[') {
        Some(attribute) => attribute.split_once(']').map_or("", |(_, after)| after),
        None => rest,
    };
    rest.trim()
}

/// The elements a background on which can cover "most of the content": the
/// blocks mail lays itself out with, not a highlighted word.
const CONTAINERS: &[&str] = &[
    "div", "table", "tbody", "thead", "tfoot", "tr", "td", "th", "center", "section", "article",
    "main", "header", "footer", "body",
];

fn find_triggers(
    node: &Handle,
    total: usize,
    rules: &BackgroundRules,
    found: &mut Option<Trigger>,
) {
    if let NodeData::Element { name, attrs, .. } = &node.data {
        let element = name.local.as_ref();
        let attrs = attrs.borrow();
        let get = |wanted: &str| attribute(&attrs, wanted);
        let declared = declarations(&get("style").unwrap_or_default());
        let width = get("width")
            .as_deref()
            .and_then(px_length)
            .into_iter()
            .chain(
                declared
                    .iter()
                    .filter(|(property, _)| matches!(property.as_str(), "width" | "min-width"))
                    .filter_map(|(_, value)| px_length(value)),
            )
            .fold(0.0f32, f32::max);
        let paints = || {
            get("bgcolor").as_deref().is_some_and(colour_paints)
                || declared
                    .iter()
                    .any(|(property, value)| background_paints(property, value))
                || get("class").is_some_and(|classes| {
                    classes
                        .split_ascii_whitespace()
                        .any(|class| rules.classes.contains(class))
                })
                || get("id").is_some_and(|id| rules.ids.contains(id.as_str()))
        };

        let candidate =
            if CONTAINERS.contains(&element) && paints() && covers_most(text_length(node), total) {
                Some(Trigger::CoveringBackground)
            } else if matches!(element, "table" | "td" | "th") && width >= LAYOUT_TABLE_MIN {
                Some(Trigger::WideTable)
            } else if element == "img" && width > WIDE_IMAGE {
                Some(Trigger::WideImage)
            } else {
                None
            };
        if let Some(trigger) = candidate {
            // The colour triggers outrank the layout ones: they are what the
            // render-mode line explains first.
            let better = match found {
                None => true,
                Some(current) => !current.is_background() && trigger.is_background(),
            };
            if better {
                *found = Some(trigger);
            }
            if trigger.is_background() {
                return;
            }
        }
    }
    for child in node.children.borrow().iter() {
        if found.is_some_and(Trigger::is_background) {
            return;
        }
        find_triggers(child, total, rules, found);
    }
}

/// Whether a block holding `inside` of the message's `total` characters of
/// text holds most of it. A message with no text at all -- one picture --
/// is covered by any painted block around it.
fn covers_most(inside: usize, total: usize) -> bool {
    if total == 0 {
        return true;
    }
    inside as f32 >= total as f32 * MOST_OF_THE_CONTENT
}

/// How many non-blank characters of text are under `node`.
fn text_length(node: &Handle) -> usize {
    let own = match &node.data {
        NodeData::Text { contents } => contents
            .borrow()
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '\u{a0}')
            .count(),
        _ => 0,
    };
    own + node
        .children
        .borrow()
        .iter()
        .map(text_length)
        .sum::<usize>()
}

fn attribute(attrs: &[Attribute], wanted: &str) -> Option<String> {
    attrs
        .iter()
        .find(|attr| attr.name.local.as_ref().eq_ignore_ascii_case(wanted))
        .map(|attr| attr.value.trim().to_owned())
}

/// A style's declarations as lowercased property and trimmed value.
fn declarations(style: &str) -> Vec<(String, String)> {
    split_declarations(style)
        .into_iter()
        .filter_map(|declaration| {
            let (property, value) = declaration.split_once(':')?;
            Some((
                property.trim().to_ascii_lowercase(),
                value.trim().to_owned(),
            ))
        })
        .collect()
}

/// A length in CSS pixels, as an attribute (`600`) or a declaration
/// (`600px`, `450pt`) spells one. Percentages and font-relative units are
/// not fixed widths and answer `None`.
pub fn px_length(value: &str) -> Option<f32> {
    let value = value
        .trim()
        .trim_end_matches("!important")
        .trim()
        .to_ascii_lowercase();
    let (number, factor) = if let Some(number) = value.strip_suffix("px") {
        (number, 1.0)
    } else if let Some(number) = value.strip_suffix("pt") {
        (number, 4.0 / 3.0)
    } else {
        (value.as_str(), 1.0)
    };
    number
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|n| n.is_finite() && *n >= 0.0)
        .map(|n| n * factor)
}

/// Whether a declaration paints a background: a `background-color` that
/// [`colour_paints`], or a `background`/`background-image` naming an image
/// or a painting colour.
fn background_paints(property: &str, value: &str) -> bool {
    match property {
        "background-color" => colour_paints(value),
        "background-image" => value.to_ascii_lowercase().contains("url("),
        "background" => {
            let lower = value.to_ascii_lowercase();
            lower.contains("url(") || value_tokens(&lower).iter().any(|t| colour_paints(t))
        }
        _ => false,
    }
}

/// A value's space-separated tokens, keeping a function's arguments with it:
/// `rgb(1, 2, 3) no-repeat` is two tokens.
fn value_tokens(value: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    for (at, character) in value.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            c if c.is_whitespace() && depth == 0 => {
                if let Some(from) = start.take() {
                    out.push(&value[from..at]);
                }
                continue;
            }
            _ => {}
        }
        if start.is_none() {
            start = Some(at);
        }
    }
    if let Some(from) = start {
        out.push(&value[from..]);
    }
    out
}

/// An sRGB colour, 8 bits a channel, and its alpha.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Colour {
    /// Red, green and blue.
    pub rgb: [u8; 3],
    /// Opacity, `0.0..=1.0`.
    pub alpha: f32,
}

impl Colour {
    /// WCAG relative luminance.
    pub fn luminance(self) -> f64 {
        let channel = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let [r, g, b] = self.rgb;
        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    }

    /// How far apart its strongest and weakest channels are, `0.0..=1.0`:
    /// zero for black, white and every grey between.
    pub fn chroma(self) -> f64 {
        let max = self.rgb.iter().copied().max().unwrap_or(0);
        let min = self.rgb.iter().copied().min().unwrap_or(0);
        f64::from(max - min) / 255.0
    }
}

/// A CSS colour as mail writes one: hex, `rgb()`/`rgba()`, or one of the
/// names mail actually uses. `None` for anything else, `transparent`
/// included -- see [`colour_paints`] for what an unknown name means there.
pub fn parse_colour(value: &str) -> Option<Colour> {
    let value = value
        .trim()
        .trim_end_matches("!important")
        .trim()
        .to_ascii_lowercase();
    if let Some(hex) = value.strip_prefix('#') {
        let digit = |at: usize, len: usize| u8::from_str_radix(hex.get(at..at + len)?, 16).ok();
        return match hex.len() {
            3 | 4 => {
                let short = |at| digit(at, 1).map(|d| d * 17);
                let alpha = if hex.len() == 4 {
                    f32::from(short(3)?) / 255.0
                } else {
                    1.0
                };
                Some(Colour {
                    rgb: [short(0)?, short(1)?, short(2)?],
                    alpha,
                })
            }
            6 | 8 => {
                let alpha = if hex.len() == 8 {
                    f32::from(digit(6, 2)?) / 255.0
                } else {
                    1.0
                };
                Some(Colour {
                    rgb: [digit(0, 2)?, digit(2, 2)?, digit(4, 2)?],
                    alpha,
                })
            }
            _ => None,
        };
    }
    for function in ["rgba(", "rgb("] {
        if let Some(inner) = value
            .strip_prefix(function)
            .and_then(|rest| rest.strip_suffix(')'))
        {
            let parts: Vec<&str> = inner
                .split([',', '/', ' '])
                .filter(|part| !part.is_empty())
                .collect();
            let channel = |part: &str| -> Option<u8> {
                let value = match part.strip_suffix('%') {
                    Some(percent) => percent.parse::<f32>().ok()? * 2.55,
                    None => part.parse::<f32>().ok()?,
                };
                Some(value.clamp(0.0, 255.0).round() as u8)
            };
            if parts.len() < 3 {
                return None;
            }
            let alpha = match parts.get(3) {
                Some(part) => match part.strip_suffix('%') {
                    Some(percent) => percent.parse::<f32>().ok()? / 100.0,
                    None => part.parse::<f32>().ok()?,
                },
                None => 1.0,
            };
            return Some(Colour {
                rgb: [channel(parts[0])?, channel(parts[1])?, channel(parts[2])?],
                alpha: alpha.clamp(0.0, 1.0),
            });
        }
    }
    let rgb = match value.as_str() {
        "black" => [0, 0, 0],
        "white" => [255, 255, 255],
        "red" => [255, 0, 0],
        "green" => [0, 128, 0],
        "blue" => [0, 0, 255],
        "navy" => [0, 0, 128],
        "maroon" => [128, 0, 0],
        "purple" => [128, 0, 128],
        "teal" => [0, 128, 128],
        "olive" => [128, 128, 0],
        "orange" => [255, 165, 0],
        "gray" | "grey" => [128, 128, 128],
        "silver" => [192, 192, 192],
        "darkred" => [139, 0, 0],
        "darkblue" => [0, 0, 139],
        "darkgreen" => [0, 100, 0],
        "crimson" => [220, 20, 60],
        "whitesmoke" => [245, 245, 245],
        _ => return None,
    };
    Some(Colour { rgb, alpha: 1.0 })
}

/// Whether a background colour actually paints a page: not `transparent`
/// or a keyword that defers to something else, not fully transparent, and
/// not [`PLAIN_WHITE`]. A colour this cannot read -- `hsl()`, an unusual
/// name -- counts as painting: the sender chose something.
pub fn colour_paints(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    let lower = lower.trim_end_matches("!important").trim();
    if lower.is_empty()
        || matches!(
            lower,
            "transparent"
                | "none"
                | "inherit"
                | "initial"
                | "unset"
                | "revert"
                | "currentcolor"
                | "auto"
        )
    {
        return false;
    }
    match parse_colour(lower) {
        Some(colour) => colour.alpha > 0.0 && colour.luminance() < PLAIN_WHITE,
        None => crate::sanitize::is_colour(lower),
    }
}

/// The properties the app-colours treatment takes away (T211).
///
/// Colour, the page and the type: what made the sender's mail look like the
/// sender's. `background-*` is all of them, image included, because a picture
/// behind text is the same problem as a colour behind it.
fn stripped(property: &str) -> bool {
    matches!(
        property,
        "color" | "font-family" | "font-size" | "font" | "line-height" | "background"
    ) || property.starts_with("background-")
}

/// The attributes the app-colours treatment takes away: colours, faces,
/// sizes and backgrounds, as HTML 4 spelled them.
const STRIPPED_ATTRIBUTES: &[&str] = &[
    "bgcolor",
    "color",
    "face",
    "size",
    "background",
    "text",
    "link",
    "vlink",
    "alink",
];

/// Inline elements whose colour a sender may have chosen to say something --
/// a red "URGENT" -- rather than to set their text colour. A colour on one of
/// these is kept, if it has colour in it at all, for the renderer to hold to
/// the contrast floor; a colour on a block is the sender's ink and goes.
const PHRASING: &[&str] = &[
    "span", "font", "b", "strong", "i", "em", "u", "mark", "small", "s", "del", "ins", "sub",
    "sup", "code", "cite", "q",
];

/// The least [`Colour::chroma`] a kept colour needs: below it the colour is a
/// grey -- the black, charcoal and silver senders set their text in -- and
/// says nothing but "this is my ink".
pub const DELIBERATE_CHROMA: f64 = 0.3;

/// The class the app-colours treatment marks a data table with: one that
/// draws a grid (a header cell, a `border`, ruled cells), which the app's
/// stylesheet redraws in its own hairlines. A layout table gets none and
/// keeps no lines at all.
pub const GRID_CLASS: &str = "postio-grid";

/// The app-colours treatment of sanitised markup (T211).
///
/// Takes away every colour, background, face, size and line height the
/// sender set, as declarations and as attributes, and keeps the structure:
/// bold, italic, underline, headings, lists, blockquotes, tables, links and
/// inline images. Three things beyond the list, each so the app's rhythm is
/// the one the eye reads:
///
/// * A colour on an inline element that has colour in it
///   ([`DELIBERATE_CHROMA`]) is kept, outside a link: the renderer keeps it
///   only where it reaches the contrast floor against the surface, and
///   draws it in ink otherwise.
/// * A table that draws a grid is marked [`GRID_CLASS`], and loses its own
///   borders, padding and width so the app's rules draw it.
/// * Paragraphs lose their margins, and an empty one -- the `&nbsp;` an
///   office client puts between paragraphs -- goes: the app's 12px gap is
///   the gap.
pub fn app_colours(html: &str) -> String {
    let dom = parse_document(RcDom::default(), ParseOpts::default()).one(html);
    let Some(body) = find_body(&dom.document) else {
        return String::new();
    };
    treat(&body, false, false);
    let mut bytes = Vec::new();
    let handle: SerializableHandle = body.into();
    let options = SerializeOpts {
        traversal_scope: TraversalScope::ChildrenOnly(None),
        ..SerializeOpts::default()
    };
    // Writing into a Vec cannot fail.
    let _ = serialize(&mut bytes, &handle, options);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn find_body(node: &Handle) -> Option<Handle> {
    if let NodeData::Element { name, .. } = &node.data
        && name.local.as_ref() == "body"
    {
        return Some(node.clone());
    }
    node.children.borrow().iter().find_map(find_body)
}

fn treat(node: &Handle, in_link: bool, in_grid: bool) {
    let mut in_link = in_link;
    let mut in_grid = in_grid;
    if let NodeData::Element { name, attrs, .. } = &node.data {
        let element = name.local.as_ref().to_owned();
        if element == "a" {
            in_link = true;
        }
        if element == "table" {
            in_grid = draws_grid(node);
        }
        let mut attrs = attrs.borrow_mut();
        attrs.retain(|attr| {
            !STRIPPED_ATTRIBUTES
                .iter()
                .any(|stripped| attr.name.local.as_ref().eq_ignore_ascii_case(stripped))
        });
        let table_part = matches!(
            element.as_str(),
            "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th"
        );
        if in_grid && table_part {
            attrs.retain(|attr| {
                !["border", "cellpadding", "cellspacing", "width"]
                    .iter()
                    .any(|gone| attr.name.local.as_ref().eq_ignore_ascii_case(gone))
            });
        }
        let keeps_colour = !in_link && PHRASING.contains(&element.as_str());
        if let Some(style) = attrs
            .iter_mut()
            .find(|attr| attr.name.local.as_ref().eq_ignore_ascii_case("style"))
        {
            let kept: Vec<String> = declarations(&style.value)
                .into_iter()
                .filter(|(property, value)| {
                    if property == "color" {
                        return keeps_colour
                            && parse_colour(value)
                                .is_some_and(|c| c.chroma() >= DELIBERATE_CHROMA);
                    }
                    if stripped(property) {
                        return false;
                    }
                    if element == "p" && (property == "margin" || property.starts_with("margin-")) {
                        return false;
                    }
                    !(in_grid
                        && table_part
                        && (property.starts_with("border")
                            || property.starts_with("padding")
                            || property == "width"))
                })
                .map(|(property, value)| format!("{property}: {value}"))
                .collect();
            style.value = StrTendril::from(kept.join("; "));
        }
        attrs.retain(|attr| {
            !(attr.name.local.as_ref().eq_ignore_ascii_case("style")
                && attr.value.trim().is_empty())
        });
        if element == "table" && in_grid {
            add_class(&mut attrs, GRID_CLASS);
        }
    }
    // Spacer paragraphs go: whitespace and `&nbsp;` with nothing else in.
    node.children.borrow_mut().retain(|child| !is_spacer(child));
    for child in node.children.borrow().iter() {
        treat(child, in_link, in_grid);
    }
}

fn add_class(attrs: &mut Vec<Attribute>, class: &str) {
    if let Some(existing) = attrs
        .iter_mut()
        .find(|attr| attr.name.local.as_ref().eq_ignore_ascii_case("class"))
    {
        let joined = format!("{} {class}", existing.value.trim());
        existing.value = StrTendril::from(joined.trim().to_owned());
        return;
    }
    attrs.push(Attribute {
        name: QualName::new(None, ns!(), LocalName::from("class")),
        value: StrTendril::from(class),
    });
}

/// Whether a table draws a grid -- a data table -- rather than arranging a
/// page: a header cell, a `border`, or a cell that rules itself. Nested
/// tables answer for themselves.
fn draws_grid(table: &Handle) -> bool {
    fn bordered(attrs: &[Attribute]) -> bool {
        let border_attribute = attribute(attrs, "border")
            .and_then(|b| b.parse::<f32>().ok())
            .is_some_and(|b| b > 0.0);
        let border_style = attribute(attrs, "style").is_some_and(|style| {
            declarations(&style).iter().any(|(property, value)| {
                property.starts_with("border")
                    && !property.ends_with("collapse")
                    && !property.ends_with("spacing")
                    && !matches!(value.as_str(), "none" | "0" | "0px" | "hidden")
            })
        });
        border_attribute || border_style
    }
    fn walk(node: &Handle, top: bool) -> bool {
        if let NodeData::Element { name, attrs, .. } = &node.data {
            let element = name.local.as_ref();
            if element == "table" && !top {
                return false;
            }
            if element == "th" {
                return true;
            }
            if matches!(element, "table" | "td") && bordered(&attrs.borrow()) {
                return true;
            }
        }
        node.children
            .borrow()
            .iter()
            .any(|child| walk(child, false))
    }
    walk(table, true)
}

/// A paragraph with nothing in it but blank space: what an office client
/// writes between paragraphs instead of a margin.
fn is_spacer(node: &Handle) -> bool {
    let NodeData::Element { name, .. } = &node.data else {
        return false;
    };
    if name.local.as_ref() != "p" {
        return false;
    }
    fn blank(node: &Handle) -> bool {
        match &node.data {
            NodeData::Text { contents } => contents
                .borrow()
                .chars()
                .all(|c| c.is_whitespace() || c == '\u{a0}'),
            NodeData::Element { name, .. } => {
                matches!(
                    name.local.as_ref(),
                    "span" | "font" | "b" | "i" | "o:p" | "br" | "p"
                ) && node.children.borrow().iter().all(blank)
            }
            NodeData::Comment { .. } => true,
            _ => false,
        }
    }
    blank(node)
}

/// The app-colours treatment of the sender's scoped stylesheet: every
/// declaration [`app_colours`] would take from a `style` attribute is taken
/// from the rules too, and a rule left with nothing goes. The rest -- a
/// hidden preheader's `display: none`, a column's width -- stays.
pub fn app_colours_css(css: &str) -> String {
    let mut out = String::new();
    for item in items(css) {
        match item {
            Item::Statement(statement) => {
                out.push_str(statement.trim());
                out.push_str(";\n");
            }
            Item::Block { prelude, body } if prelude.trim_start().starts_with('@') => {
                let inner = app_colours_css(body);
                if !inner.trim().is_empty() {
                    out.push_str(&format!("{} {{\n{inner}}}\n", prelude.trim()));
                }
            }
            Item::Block { prelude, body } => {
                let kept: Vec<String> = declarations(body)
                    .into_iter()
                    .filter(|(property, _)| !stripped(property))
                    .map(|(property, value)| format!("{property}: {value}"))
                    .collect();
                if !kept.is_empty() {
                    out.push_str(&format!("{} {{ {} }}\n", prelude.trim(), kept.join("; ")));
                }
            }
        }
    }
    out
}

/// The paper treatment of the sender's scoped stylesheet: the sheet is
/// always light (`color-scheme: light`), so a sender's dark-mode rules would
/// be painting a dark design onto white paper. A
/// `prefers-color-scheme: dark` block goes, and a `prefers-color-scheme:
/// light` one applies always, as it would on a light page.
pub fn light_only(css: &str) -> String {
    let mut out = String::new();
    for item in items(css) {
        match item {
            Item::Statement(statement) => {
                out.push_str(statement.trim());
                out.push_str(";\n");
            }
            Item::Block { prelude, body } => {
                let compact: String = prelude
                    .to_ascii_lowercase()
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect();
                let media = compact.starts_with("@media");
                if media && compact.contains("prefers-color-scheme:dark") {
                    continue;
                }
                if media && compact.contains("prefers-color-scheme:light") {
                    out.push_str(&format!("@media all {{\n{}}}\n", light_only(body)));
                } else if prelude.trim_start().starts_with('@') {
                    out.push_str(&format!("{} {{\n{}}}\n", prelude.trim(), light_only(body)));
                } else {
                    out.push_str(&format!("{} {{{}}}\n", prelude.trim(), body));
                }
            }
        }
    }
    out
}

/// One top-level piece of a stylesheet.
#[derive(Debug, PartialEq, Eq)]
enum Item<'a> {
    /// A rule or an at-rule with a block: what precedes the `{`, and what is
    /// inside it.
    Block { prelude: &'a str, body: &'a str },
    /// An at-rule ending in `;`, without it.
    Statement(&'a str),
}

/// Where the quoted string or comment at `at` ends, or `at` itself if there
/// is none there.
fn skip_quoted(css: &str, at: usize) -> usize {
    let bytes = css.as_bytes();
    match bytes[at] {
        b'"' | b'\'' => {
            let quote = bytes[at];
            let mut scan = at + 1;
            while scan < bytes.len() && bytes[scan] != quote {
                scan += if bytes[scan] == b'\\' { 2 } else { 1 };
            }
            (scan + 1).min(bytes.len())
        }
        b'/' if css[at..].starts_with("/*") => css[at + 2..]
            .find("*/")
            .map_or(bytes.len(), |end| at + 2 + end + 2),
        _ => at,
    }
}

/// A stylesheet's top-level pieces. Quotes, parentheses and comments are
/// respected, so a `;` inside `url(data:…;base64,…)` or a `}` inside a
/// string ends nothing. What the sanitiser emits is CSS it wrote itself, but
/// this does not lean on that.
fn items(css: &str) -> Vec<Item<'_>> {
    let bytes = css.as_bytes();
    let mut out = Vec::new();
    let mut at = 0usize;
    loop {
        while at < bytes.len() && (bytes[at].is_ascii_whitespace() || bytes[at] == b';') {
            at += 1;
        }
        if at < bytes.len() && css[at..].starts_with("/*") {
            at = skip_quoted(css, at);
            continue;
        }
        if at >= bytes.len() {
            break;
        }
        let start = at;
        let mut parens = 0usize;
        let mut opened = None;
        while at < bytes.len() {
            let skipped = skip_quoted(css, at);
            if skipped != at {
                at = skipped;
                continue;
            }
            match bytes[at] {
                b'(' => parens += 1,
                b')' => parens = parens.saturating_sub(1),
                b';' if parens == 0 => break,
                b'{' if parens == 0 => {
                    opened = Some(at);
                    break;
                }
                _ => {}
            }
            at += 1;
        }
        let Some(open) = opened else {
            out.push(Item::Statement(css[start..at].trim()));
            at += 1;
            continue;
        };
        let mut depth = 0usize;
        let mut close = bytes.len();
        let mut scan = open;
        while scan < bytes.len() {
            let skipped = skip_quoted(css, scan);
            if skipped != scan {
                scan = skipped;
                continue;
            }
            match bytes[scan] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = scan;
                        break;
                    }
                }
                _ => {}
            }
            scan += 1;
        }
        out.push(Item::Block {
            prelude: &css[start..open],
            body: &css[open + 1..close],
        });
        at = close + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sanitize::{RemoteImages, sanitize_body};

    fn treatment(html: &str) -> Treatment {
        classify(&sanitize_body(html, RemoteImages::Blocked))
    }

    fn trigger(html: &str) -> Option<Trigger> {
        paper_trigger(&sanitize_body(html, RemoteImages::Blocked))
    }

    const WORDS: &str = "<p>Hi Ada, the figures are in. Talk on Monday about the travel line.</p>";

    #[test]
    fn plain_correspondence_is_app_colours() {
        assert_eq!(treatment(WORDS), Treatment::AppColours);
        assert_eq!(trigger(WORDS), None);
    }

    #[test]
    fn a_body_background_colour_is_paper() {
        let html = format!("<html><body bgcolor=\"#f6f1e7\">{WORDS}</body></html>");
        assert_eq!(trigger(&html), Some(Trigger::PageBackground));
        let html = format!("<body style=\"background-color: #eeeeee\">{WORDS}</body>");
        assert_eq!(trigger(&html), Some(Trigger::PageBackground));
        let html = format!("<body style=\"background: #20232a\">{WORDS}</body>");
        assert_eq!(treatment(&html), Treatment::Paper);
    }

    #[test]
    fn a_body_background_image_is_paper() {
        let html = format!("<body style=\"background-image: url(cid:paper)\">{WORDS}</body>");
        assert_eq!(trigger(&html), Some(Trigger::PageBackground));
    }

    #[test]
    fn a_white_page_is_correspondence_not_paper() {
        for page in [
            "bgcolor=\"#ffffff\"",
            "bgcolor=\"white\"",
            "style=\"background-color:#fff\"",
            "style=\"background:transparent\"",
            "style=\"background-color: rgba(0,0,0,0)\"",
        ] {
            let html = format!("<body {page}>{WORDS}</body>");
            assert_eq!(treatment(&html), Treatment::AppColours, "{page}");
        }
    }

    #[test]
    fn a_body_rule_in_the_senders_stylesheet_is_paper() {
        let html = format!(
            "<html><head><style>body {{ background: #f4f4f4; }}</style></head><body>{WORDS}</body></html>"
        );
        assert_eq!(trigger(&html), Some(Trigger::PageBackground));
    }

    #[test]
    fn a_wrapper_table_with_a_background_is_paper() {
        let html = format!("<table bgcolor=\"#e8f0ea\"><tr><td>{WORDS}{WORDS}</td></tr></table>");
        assert_eq!(trigger(&html), Some(Trigger::CoveringBackground));
    }

    #[test]
    fn a_class_rule_that_paints_most_of_the_content_is_paper() {
        let html = format!(
            "<html><head><style>.wrap {{ background-color: #1f2a44 }}</style></head>\
             <body><div class=\"wrap\">{WORDS}{WORDS}</div><p>ok</p></body></html>"
        );
        assert_eq!(trigger(&html), Some(Trigger::CoveringBackground));
    }

    #[test]
    fn a_tinted_header_cell_or_highlighted_word_is_not_paper() {
        let html = format!(
            "{WORDS}{WORDS}<table><tr><th style=\"background:#d9e2f3\">Entrance</th></tr>\
             <tr><td>Main lobby</td></tr></table>\
             <p>Please <span style=\"background:yellow\">confirm</span>.</p>"
        );
        assert_eq!(trigger(&html), None);
    }

    #[test]
    fn a_layout_table_at_least_480_wide_is_paper() {
        let at = |width: &str| {
            trigger(&format!(
                "<table width=\"{width}\"><tr><td>{WORDS}</td></tr></table>"
            ))
        };
        assert_eq!(at("480"), Some(Trigger::WideTable));
        assert_eq!(at("600"), Some(Trigger::WideTable));
        assert_eq!(at("479"), None);
        assert_eq!(at("100%"), None);
        let styled = format!("<table style=\"width: 640px\"><tr><td>{WORDS}</td></tr></table>");
        assert_eq!(trigger(&styled), Some(Trigger::WideTable));
        let cell = format!("<table width=\"100%\"><tr><td width=\"600\">{WORDS}</td></tr></table>");
        assert_eq!(trigger(&cell), Some(Trigger::WideTable));
    }

    #[test]
    fn an_image_wider_than_300_is_paper() {
        let at = |img: &str| trigger(&format!("{WORDS}<img {img} alt=\"\">"));
        assert_eq!(
            at("src=\"cid:hero\" width=\"301\""),
            Some(Trigger::WideImage)
        );
        assert_eq!(at("src=\"cid:hero\" width=\"300\""), None);
        assert_eq!(
            at("src=\"cid:hero\" style=\"width: 560px\""),
            Some(Trigger::WideImage)
        );
        // A remote image is stripped of its source while blocked, and keeps
        // the width it was laid out for.
        assert_eq!(
            at("src=\"https://img.example.com/hero.png\" width=\"600\""),
            Some(Trigger::WideImage)
        );
    }

    #[test]
    fn a_background_outranks_a_layout_trigger() {
        let html = format!(
            "<table width=\"600\"><tr><td>{WORDS}</td></tr></table>\
             <div style=\"background:#123456\">{WORDS}{WORDS}{WORDS}</div>"
        );
        assert_eq!(trigger(&html), Some(Trigger::CoveringBackground));
    }

    #[test]
    fn colours_parse_and_paint_as_mail_writes_them() {
        assert_eq!(parse_colour("#C00000").map(|c| c.rgb), Some([192, 0, 0]));
        assert_eq!(parse_colour("#abc").map(|c| c.rgb), Some([170, 187, 204]));
        assert_eq!(
            parse_colour("rgb(204, 204, 204)").map(|c| c.rgb),
            Some([204, 204, 204])
        );
        assert_eq!(parse_colour("rgba(0,0,0,0)").map(|c| c.alpha), Some(0.0));
        assert!(!colour_paints("transparent"));
        assert!(!colour_paints("#ffffff"));
        assert!(!colour_paints("#fafafa"));
        assert!(colour_paints("#f6f1e7"));
        assert!(colour_paints("#f4f4f4"));
        assert!(colour_paints("hsl(210, 40%, 20%)"));
        assert_eq!(px_length("600"), Some(600.0));
        assert_eq!(px_length("600px"), Some(600.0));
        assert_eq!(px_length("450pt"), Some(600.0));
        assert_eq!(px_length("100%"), None);
        assert_eq!(px_length("40em"), None);
    }

    fn app(html: &str) -> String {
        app_colours(&sanitize_body(html, RemoteImages::Blocked).html)
    }

    #[test]
    fn app_colours_strips_the_senders_colours_faces_and_sizes() {
        let out = app(
            "<div style=\"color:#000000;background-color:#ffffff;font-family:Calibri;\
             font-size:11pt;line-height:1.2;text-align:center\">\
             <font color=\"#333333\" face=\"Arial\" size=\"2\">Hello</font>\
             <table bgcolor=\"#eeeeee\"><tr><td style=\"background:url(cid:x) #ccc\">cell</td></tr></table></div>",
        );
        for gone in [
            "color:",
            "background",
            "font-family",
            "font-size",
            "line-height",
            "bgcolor",
            "face=",
            "size=",
            "Calibri",
            "#333333",
        ] {
            assert!(!out.contains(gone), "{gone} survived: {out}");
        }
        assert!(out.contains("text-align: center"), "layout was lost: {out}");
        assert!(out.contains("Hello") && out.contains("cell"), "{out}");
    }

    #[test]
    fn app_colours_keeps_the_structure() {
        let html = "<h2>Plan</h2><p><b>bold</b> <i>italic</i> <u>under</u></p>\
                    <ul><li>one</li></ul><ol><li>two</li></ol><blockquote>quoted</blockquote>\
                    <table><tr><td>cell</td></tr></table><a href=\"https://example.com/\">link</a>\
                    <img src=\"cid:logo\" alt=\"logo\">";
        let out = app(html);
        for kept in [
            "<h2>",
            "<b>",
            "<i>",
            "<u>",
            "<ul>",
            "<ol>",
            "<li>",
            "<blockquote>",
            "<table>",
            "<td>",
            "<a href=\"https://example.com/\"",
            "<img",
        ] {
            assert!(out.contains(kept), "{kept} was lost: {out}");
        }
        assert!(
            !out.contains(GRID_CLASS),
            "a plain table drew a grid: {out}"
        );
    }

    #[test]
    fn a_coloured_word_is_kept_for_the_contrast_guard_and_ink_is_not() {
        let out = app(
            "<p><span style=\"color:#C00000\">URGENT</span> <span style=\"color:black\">ink</span> \
             <span style=\"color:#595959\">grey</span> <a href=\"https://example.com/\">\
             <span style=\"color:#0563C1\">a link</span></a></p>",
        );
        assert!(out.contains("color: #C00000"), "the red was lost: {out}");
        for gone in ["black", "#595959", "#0563C1"] {
            assert!(!out.contains(gone), "{gone} survived: {out}");
        }
    }

    #[test]
    fn a_grid_table_is_marked_and_a_layout_table_is_not() {
        let out = app(
            "<table border=\"1\" cellpadding=\"0\" style=\"border-collapse:collapse;width:300px\">\
             <tr><td style=\"border:solid black 1pt;padding:0 5pt\">a</td></tr></table>\
             <table><tr><td><table><tr><th>h</th></tr></table></td></tr></table>",
        );
        assert_eq!(out.matches(GRID_CLASS).count(), 2, "{out}");
        assert!(
            !out.contains("solid black"),
            "the sender's grid survived: {out}"
        );
        assert!(!out.contains("padding"), "{out}");
        assert!(!out.contains("300px"), "{out}");
        assert!(
            out.contains("<table><tbody><tr><td><table class=\"postio-grid\">"),
            "the layout table around a grid was marked, or the grid was not: {out}"
        );
    }

    #[test]
    fn spacer_paragraphs_and_paragraph_margins_go() {
        let out = app(
            "<p style=\"margin:0\">one</p><p style=\"margin:0\"><span>&nbsp;</span></p>\
             <p class=\"MsoNormal\">&nbsp;</p><p style=\"margin:0 0 0 12pt\">two</p>",
        );
        assert_eq!(out.matches("<p").count(), 2, "{out}");
        assert!(!out.contains("margin"), "{out}");
    }

    #[test]
    fn the_senders_stylesheet_loses_the_same_properties() {
        let css = ".postio-body .MsoNormal { margin: 0; font-family: Calibri; color: black }\n\
                   .postio-body a { color: #0563C1 }\n\
                   .postio-body .pre { display: none !important }\n\
                   @media (max-width: 600px) { .postio-body .col { width: 100%; background: #eee } }\n";
        let out = app_colours_css(css);
        assert!(
            !out.contains("Calibri") && !out.contains("black") && !out.contains("#0563C1"),
            "{out}"
        );
        assert!(!out.contains(" a {"), "an emptied rule was kept: {out}");
        assert!(out.contains("display: none !important"), "{out}");
        assert!(out.contains("margin: 0"), "{out}");
        assert!(
            out.contains("@media (max-width: 600px)") && out.contains("width: 100%"),
            "{out}"
        );
        assert!(!out.contains("#eee"), "{out}");
    }

    #[test]
    fn paper_keeps_only_the_light_design() {
        let css = ".postio-body p { color: #222 }\n\
                   @media (prefers-color-scheme: dark) { .postio-body p { color: #eee } }\n\
                   @media (prefers-color-scheme : light) { .postio-body h1 { color: #111 } }\n\
                   @media (max-width: 600px) { .postio-body td { display: block } }\n";
        let out = light_only(css);
        assert!(!out.contains("#eee"), "the dark design survived: {out}");
        assert!(out.contains("@media all") && out.contains("#111"), "{out}");
        assert!(
            out.contains("color: #222") && out.contains("display: block"),
            "{out}"
        );
    }

    #[test]
    fn the_attribute_names_both_treatments_and_nothing_else() {
        for treatment in [Treatment::AppColours, Treatment::Paper] {
            assert_eq!(
                Treatment::from_attribute(treatment.attribute_value()),
                Some(treatment)
            );
            assert_ne!(treatment.other(), treatment);
            assert_eq!(treatment.other().other(), treatment);
        }
        assert_eq!(Treatment::from_attribute("dark"), None);
    }

    #[test]
    fn the_stylesheet_scanner_respects_strings_and_urls() {
        let css =
            "a { background: url(data:image/png;base64,AA==) } b { content: \"}\" } @import x;";
        let found = items(css);
        assert_eq!(found.len(), 3, "{found:?}");
        assert_eq!(found[2], Item::Statement("@import x"));
    }
}
