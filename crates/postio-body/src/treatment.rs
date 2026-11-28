//! How a message body is shown: in the app's own colours (`app_colours`), or as sent, on paper. The
//! sender's colours never meet the app's surface, so every body gets exactly one, in light and
//! dark; `classify` decides, and the decision also fixes the column width.
//! Everything is a pure function of markup already sanitised, so neither treatment can bring back a
//! script or remote content.

use std::collections::HashSet;

use html5ever::driver::ParseOpts;
use html5ever::parse_document;
use html5ever::serialize::{SerializeOpts, TraversalScope, serialize};
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::{Attribute, LocalName, QualName, ns};
use markup5ever_rcdom::{Handle, NodeData, RcDom, SerializableHandle};

use crate::sanitize::{BODY_CLASS, Sanitized, split_declarations};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Treatment {
    #[default]
    AppColours,
    Paper,
}

/// Postio's own name: the sanitiser keeps no `data-` attribute a sender writes, so a message cannot
/// claim a treatment for itself.
pub const TREATMENT_ATTRIBUTE: &str = "data-postio-treatment";

impl Treatment {
    pub fn attribute_value(self) -> &'static str {
        match self {
            Treatment::AppColours => "app",
            Treatment::Paper => "paper",
        }
    }

    pub fn from_attribute(value: &str) -> Option<Treatment> {
        match value {
            "app" => Some(Treatment::AppColours),
            "paper" => Some(Treatment::Paper),
            _ => None,
        }
    }

    pub fn other(self) -> Treatment {
        match self {
            Treatment::AppColours => Treatment::Paper,
            Treatment::Paper => Treatment::AppColours,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    PageBackground,
    CoveringBackground,
    WideTable,
    WideImage,
}

impl Trigger {
    pub fn is_background(self) -> bool {
        matches!(self, Trigger::PageBackground | Trigger::CoveringBackground)
    }
}

/// A layout this wide was built for a page, not a 480px column.
pub const LAYOUT_TABLE_MIN: f32 = 480.0;

pub const WIDE_IMAGE: f32 = 300.0;

pub const MOST_OF_THE_CONTENT: f32 = 0.5;

/// At or above this luminance a background is a client's default white page, not a design: a reply
/// stamped `bgcolor="#ffffff"` is correspondence, and paper would take the app's ink away.
pub const PLAIN_WHITE: f64 = 0.95;

/// The same answer in light and dark: nothing here knows the theme.
pub fn classify(sanitized: &Sanitized) -> Treatment {
    match paper_trigger(sanitized) {
        Some(_) => Treatment::Paper,
        None => Treatment::AppColours,
    }
}

/// The page's own background comes from `Sanitized::canvas` (lifted off `<body>`) and the scoped
/// stylesheet's `body` rule; the rest from the markup. A remote background image counts only while
/// remote images are allowed.
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

#[derive(Default)]
struct BackgroundRules {
    page: bool,
    classes: HashSet<String>,
    ids: HashSet<String>,
}

impl BackgroundRules {
    fn from(styles: &str) -> BackgroundRules {
        let mut rules = BackgroundRules::default();
        for item in items(styles) {
            // Top level only: a background under a media query is a responsive or dark-mode
            // variant.
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

fn unscoped(selector: &str) -> &str {
    let Some(rest) = selector.strip_prefix(&format!(".{BODY_CLASS}")) else {
        return selector;
    };
    let rest = match rest.strip_prefix('[') {
        Some(attribute) => attribute.split_once(']').map_or("", |(_, after)| after),
        None => rest,
    };
    rest.trim()
}

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
        let element = &*name.local;
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

/// A message with no text, such as one picture, is covered by any painted block around it.
fn covers_most(inside: usize, total: usize) -> bool {
    if total == 0 {
        return true;
    }
    inside as f32 >= total as f32 * MOST_OF_THE_CONTENT
}

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
        .find(|attr| str::eq_ignore_ascii_case(&attr.name.local, wanted))
        .map(|attr| attr.value.trim().to_owned())
}

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

/// Percentages and font-relative units are not fixed widths and answer `None`.
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Colour {
    pub rgb: [u8; 3],
    pub alpha: f32,
}

impl Colour {
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

    pub fn chroma(self) -> f64 {
        let max = self.rgb.iter().copied().max().unwrap_or(0);
        let min = self.rgb.iter().copied().min().unwrap_or(0);
        f64::from(max - min) / 255.0
    }
}

/// `None` for anything else, `transparent` included; see `colour_paints`.
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

/// An unreadable colour (`hsl()`, an unusual name) counts as painting: the sender chose something.
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

/// Colour, page and type. `background-*` includes images, since a picture behind text is the same
/// problem as a colour.
fn stripped(property: &str) -> bool {
    matches!(
        property,
        "color" | "font-family" | "font-size" | "font" | "line-height" | "background"
    ) || property.starts_with("background-")
}

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

/// A colour on one of these may be meant (a red "URGENT") and is kept for the renderer's contrast
/// floor; a colour on a block is the sender's ink and goes.
const PHRASING: &[&str] = &[
    "span", "font", "b", "strong", "i", "em", "u", "mark", "small", "s", "del", "ins", "sub",
    "sup", "code", "cite", "q",
];

/// Below it the colour is a grey (black, charcoal, silver) and says only "this is my ink".
pub const DELIBERATE_CHROMA: f64 = 0.3;

/// A data table that draws a grid, which the app's stylesheet redraws in its own hairlines; a
/// layout table gets none.
pub const GRID_CLASS: &str = "postio-grid";

/// Strips every colour, background, face, size and line height as declarations and attributes,
/// keeping structure. A deliberate colour on an inline element outside a link is kept for the
/// renderer's contrast floor.
/// A grid table gets `GRID_CLASS` and loses its own borders, padding and width; paragraphs lose
/// margins, and an empty one (an office client's `&nbsp;`) goes.
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
    let _ = serialize(&mut bytes, &handle, options);
    String::from_utf8_lossy(&bytes).into_owned()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    Light,
    Dark,
}

impl Scheme {
    fn media(self) -> &'static str {
        match self {
            Scheme::Light => "light",
            Scheme::Dark => "dark",
        }
    }
}

/// The app's ink needs no field: a colour that fails the floor is taken out of the markup and the
/// text falls back to the stylesheet's ink.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    pub scheme: Scheme,
    pub background: [u8; 3],
}

/// `--r-ground` in `postio-ui/data/reader-tokens.css`, copied because `postio-body` cannot depend
/// on `postio-ui`; keep the two in step.
pub const LIGHT_SURFACE: Surface = Surface {
    scheme: Scheme::Light,
    background: [0xf5, 0xf5, 0xf8],
};

/// `--r-ground` under `prefers-color-scheme: dark` in `postio-ui/data/reader-tokens.css`; keep in
/// step.
pub const DARK_SURFACE: Surface = Surface {
    scheme: Scheme::Dark,
    background: [0x2e, 0x2e, 0x31],
};

pub const SURFACES: [Surface; 2] = [LIGHT_SURFACE, DARK_SURFACE];

pub const CONTRAST_FLOOR: f64 = 4.5;

/// Below this scale text would be too small to read, so the sheet stays at it and the view scrolls
/// sideways. Every renderer fits paper to this number.
pub const PAPER_FIT_FLOOR: f64 = 0.85;

pub const KEPT_CLASS: &str = "postio-kept-";

pub fn contrast_ratio(a: Colour, b: Colour) -> f64 {
    let (la, lb) = (a.luminance(), b.luminance());
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// The contrast guard decided in the markup, for a webview that cannot run `postio-render`'s guard.
/// Each element still carrying an inline `color` loses it and gets `postio-kept-N`; one appended
/// `<style>` gives that class the colour only in the schemes whose surface it reaches
/// `CONTRAST_FLOOR` against, so it holds with JavaScript off.
/// Alpha is judged composited over each surface; an unparseable colour is dropped; markup with no
/// kept colour comes back byte for byte.
pub fn guard_kept_colours(html: &str, surfaces: &[Surface]) -> String {
    let dom = parse_document(RcDom::default(), ParseOpts::default()).one(html);
    let Some(body) = find_body(&dom.document) else {
        return html.to_owned();
    };
    let mut rules = Vec::new();
    let mut changed = false;
    guard_node(&body, surfaces, &mut rules, &mut changed);
    if !changed {
        return html.to_owned();
    }
    let mut bytes = Vec::new();
    let handle: SerializableHandle = body.into();
    let options = SerializeOpts {
        traversal_scope: TraversalScope::ChildrenOnly(None),
        ..SerializeOpts::default()
    };
    let _ = serialize(&mut bytes, &handle, options);
    let mut out = String::from_utf8_lossy(&bytes).into_owned();
    let sheet: String = rules.concat();
    if !sheet.is_empty() {
        out.push_str("<style>");
        out.push_str(&sheet);
        out.push_str("</style>");
    }
    out
}

fn guard_node(node: &Handle, surfaces: &[Surface], rules: &mut Vec<String>, changed: &mut bool) {
    if let NodeData::Element { attrs, .. } = &node.data {
        let mut attrs = attrs.borrow_mut();
        let colour = attrs
            .iter()
            .find(|attr| str::eq_ignore_ascii_case(&attr.name.local, "style"))
            .and_then(|style| {
                declarations(&style.value)
                    .into_iter()
                    .find(|(property, _)| property == "color")
            });
        if let Some((_, value)) = colour {
            *changed = true;
            if let Some(style) = attrs
                .iter_mut()
                .find(|attr| str::eq_ignore_ascii_case(&attr.name.local, "style"))
            {
                let rest: Vec<String> = declarations(&style.value)
                    .into_iter()
                    .filter(|(property, _)| property != "color")
                    .map(|(property, value)| format!("{property}: {value}"))
                    .collect();
                style.value = StrTendril::from(rest.join("; "));
            }
            attrs.retain(|attr| {
                !(str::eq_ignore_ascii_case(&attr.name.local, "style")
                    && attr.value.trim().is_empty())
            });
            let reads: Vec<Scheme> = parse_colour(&value)
                .map(|colour| {
                    surfaces
                        .iter()
                        .filter(|surface| reads_on(colour, surface))
                        .map(|surface| surface.scheme)
                        .collect()
                })
                .unwrap_or_default();
            if let (Some(colour), false) = (parse_colour(&value), reads.is_empty()) {
                let class = format!("{KEPT_CLASS}{}", rules.len());
                add_class(&mut attrs, &class);
                let [r, g, b] = colour.rgb;
                let paint = if colour.alpha < 1.0 {
                    format!("rgba({r}, {g}, {b}, {})", colour.alpha)
                } else {
                    format!("#{r:02x}{g:02x}{b:02x}")
                };
                let rule = format!(".{class} {{ color: {paint}; }}");
                rules.push(if reads.len() == surfaces.len() {
                    rule
                } else {
                    let media: Vec<String> = reads
                        .iter()
                        .map(|scheme| {
                            format!(
                                "@media (prefers-color-scheme: {}) {{ {rule} }}",
                                scheme.media()
                            )
                        })
                        .collect();
                    media.concat()
                });
            }
        }
    }
    for child in node.children.borrow().iter() {
        guard_node(child, surfaces, rules, changed);
    }
}

fn reads_on(colour: Colour, surface: &Surface) -> bool {
    let a = f64::from(colour.alpha);
    let mix =
        |over: u8, under: u8| (f64::from(over) * a + f64::from(under) * (1.0 - a)).round() as u8;
    let [r, g, b] = colour.rgb;
    let [ur, ug, ub] = surface.background;
    let drawn = Colour {
        rgb: [mix(r, ur), mix(g, ug), mix(b, ub)],
        alpha: 1.0,
    };
    let ground = Colour {
        rgb: surface.background,
        alpha: 1.0,
    };
    contrast_ratio(drawn, ground) >= CONTRAST_FLOOR
}

fn find_body(node: &Handle) -> Option<Handle> {
    if let NodeData::Element { name, .. } = &node.data
        && (&*name.local) == "body"
    {
        return Some(node.clone());
    }
    node.children.borrow().iter().find_map(find_body)
}

fn treat(node: &Handle, in_link: bool, in_grid: bool) {
    let mut in_link = in_link;
    let mut in_grid = in_grid;
    if let NodeData::Element { name, attrs, .. } = &node.data {
        let element = name.local.to_string();
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
                .any(|stripped| str::eq_ignore_ascii_case(&attr.name.local, stripped))
        });
        let table_part = matches!(
            element.as_str(),
            "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th"
        );
        if in_grid && table_part {
            attrs.retain(|attr| {
                !["border", "cellpadding", "cellspacing", "width"]
                    .iter()
                    .any(|gone| str::eq_ignore_ascii_case(&attr.name.local, gone))
            });
        }
        let keeps_colour = !in_link && PHRASING.contains(&element.as_str());
        if let Some(style) = attrs
            .iter_mut()
            .find(|attr| str::eq_ignore_ascii_case(&attr.name.local, "style"))
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
            !(str::eq_ignore_ascii_case(&attr.name.local, "style") && attr.value.trim().is_empty())
        });
        if element == "table" && in_grid {
            add_class(&mut attrs, GRID_CLASS);
        }
    }
    node.children.borrow_mut().retain(|child| !is_spacer(child));
    for child in node.children.borrow().iter() {
        treat(child, in_link, in_grid);
    }
}

fn add_class(attrs: &mut Vec<Attribute>, class: &str) {
    if let Some(existing) = attrs
        .iter_mut()
        .find(|attr| str::eq_ignore_ascii_case(&attr.name.local, "class"))
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
            let element = &*name.local;
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

fn is_spacer(node: &Handle) -> bool {
    let NodeData::Element { name, .. } = &node.data else {
        return false;
    };
    if (&*name.local) != "p" {
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
                    &*name.local,
                    "span" | "font" | "b" | "i" | "o:p" | "br" | "p"
                ) && node.children.borrow().iter().all(blank)
            }
            NodeData::Comment { .. } => true,
            _ => false,
        }
    }
    blank(node)
}

/// Rules left with nothing go; the rest (a hidden preheader's `display: none`, a column's width)
/// stays.
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

/// The paper sheet is always light, so a sender's `prefers-color-scheme: dark` block goes and a
/// `light` one applies always.
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

#[derive(Debug, PartialEq, Eq)]
enum Item<'a> {
    Block { prelude: &'a str, body: &'a str },
    Statement(&'a str),
}

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

/// Quotes, parentheses and comments are respected, so a `;` inside `url(data:…;base64,…)` or a `}`
/// inside a string ends nothing.
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

    #[test]
    fn contrast_ratio_of_black_on_white_is_21() {
        let black = parse_colour("#000").unwrap();
        let white = parse_colour("#ffffff").unwrap();
        assert!((contrast_ratio(black, white) - 21.0).abs() < 1e-9);
        assert!((contrast_ratio(white, black) - 21.0).abs() < 1e-9);
        assert!((contrast_ratio(white, white) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_fixture_colours_read_where_the_tests_say() {
        let on = |hex: &str, s: Surface| {
            contrast_ratio(
                parse_colour(hex).unwrap(),
                Colour {
                    rgb: s.background,
                    alpha: 1.0,
                },
            )
        };
        assert!(on("#cc0000", LIGHT_SURFACE) >= CONTRAST_FLOOR);
        assert!(on("#cc0000", DARK_SURFACE) < CONTRAST_FLOOR);
        assert!(on("#990000", LIGHT_SURFACE) >= CONTRAST_FLOOR);
        assert!(on("#990000", DARK_SURFACE) < CONTRAST_FLOOR);
        assert!(on("#ff8080", DARK_SURFACE) >= CONTRAST_FLOOR);
        assert!(on("#ff8080", LIGHT_SURFACE) < CONTRAST_FLOOR);
        assert!(on("#808000", LIGHT_SURFACE) < CONTRAST_FLOOR);
        assert!(on("#808000", DARK_SURFACE) < CONTRAST_FLOOR);
    }

    fn kept(colour: &str) -> String {
        format!("<p>Please <span style=\"color: {colour}\">confirm</span> today.</p>")
    }

    #[test]
    fn a_red_that_reads_on_light_stays_in_light_only() {
        let out = guard_kept_colours(&kept("#cc0000"), &SURFACES);
        assert!(out.contains("class=\"postio-kept-0\""), "{out}");
        assert!(
            !out.contains("style=\"color"),
            "inline colour is gone: {out}"
        );
        assert!(
            out.contains(
                "@media (prefers-color-scheme: light) { .postio-kept-0 { color: #cc0000; } }"
            ),
            "{out}"
        );
        assert!(!out.contains("prefers-color-scheme: dark"), "{out}");
    }

    #[test]
    fn a_colour_that_reads_on_every_surface_given_gets_a_plain_rule() {
        let out = guard_kept_colours(&kept("#cc0000"), &[LIGHT_SURFACE]);
        assert!(out.contains(".postio-kept-0 { color: #cc0000; }"), "{out}");
        assert!(!out.contains("@media"), "{out}");
    }

    #[test]
    fn a_colour_failing_on_dark_is_scoped_to_light() {
        let out = guard_kept_colours(&kept("#990000"), &SURFACES);
        assert!(
            out.contains(
                "@media (prefers-color-scheme: light) { .postio-kept-0 { color: #990000; } }"
            ),
            "{out}"
        );
        assert!(!out.contains("prefers-color-scheme: dark"), "{out}");
    }

    #[test]
    fn a_colour_failing_on_light_is_scoped_to_dark() {
        let out = guard_kept_colours(&kept("#ff8080"), &SURFACES);
        assert!(out.contains("prefers-color-scheme: dark"), "{out}");
        assert!(!out.contains("prefers-color-scheme: light"), "{out}");
    }

    #[test]
    fn a_colour_failing_on_both_is_removed_entirely() {
        let out = guard_kept_colours(&kept("#808000"), &SURFACES);
        assert!(!out.contains("808000"), "{out}");
        assert!(!out.contains("postio-kept"), "{out}");
        assert!(!out.contains("<style>"), "{out}");
        assert!(out.contains("confirm"), "the words stay: {out}");
    }

    #[test]
    fn markup_with_no_kept_colour_is_returned_unchanged() {
        let html = "<p>Hello <b>Ada</b>, <a href=\"https://example.com/\">see this</a>.</p>";
        assert_eq!(guard_kept_colours(html, &SURFACES), html);
    }

    #[test]
    fn classes_count_up_in_document_order_and_keep_other_styles() {
        let html = "<p><span style=\"color: #cc0000; font-weight: bold\" class=\"x\">a</span> \
                    <span style=\"color: #990000\">b</span></p>";
        let out = guard_kept_colours(html, &SURFACES);
        assert!(out.contains("class=\"x postio-kept-0\""), "{out}");
        assert!(out.contains("font-weight: bold"), "{out}");
        assert!(out.contains("postio-kept-1\">b"), "{out}");
        assert_eq!(out, guard_kept_colours(html, &SURFACES), "deterministic");
    }

    #[test]
    fn the_guard_composes_with_app_colours() {
        let sent =
            "<p style=\"color: #333333\">Hi <font style=\"color: #cc0000\">URGENT</font></p>";
        let out = guard_kept_colours(&app_colours(sent), &SURFACES);
        assert!(out.contains("postio-kept-0"), "{out}");
        assert!(!out.contains("333333"), "{out}");
    }
}
