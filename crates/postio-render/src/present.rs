//! The theme rule applied to a laid-out document (research R10): each
//! message classified, its presentation's backgrounds set, and every text
//! run's colour repaired to the floor against what is actually behind it.
//!
//! The result is a plan for a second layout: one user-agent stylesheet of
//! `!important` rules -- which outrank any author rule, a sender's own
//! `!important` included -- keyed by an attribute set on just the nodes it
//! touches. A `DocumentMutator` style change does not restyle in Blitz
//! 0.3.0-beta.2, so the document is laid out afresh with the sheet. The
//! same markup parses to the same node ids, which is what lets the marks be
//! placed by id.

use std::collections::HashMap;

use blitz_dom::{BaseDocument, LocalName, Node, NodeId, local_name};

use crate::theme::{self, MessageFacts, Presentation, Rgb};
use crate::{RenderRequest, Scope};

/// The attribute the second layout's overrides select on.
pub(crate) const MARK: &str = "data-postio-n";

/// What the second layout needs, and what the first one decided.
#[derive(Default)]
pub(crate) struct Plan {
    /// The overrides, as a user-agent stylesheet.
    pub(crate) css: String,
    /// The nodes the stylesheet names, and the mark each gets.
    pub(crate) marks: Vec<(NodeId, usize)>,
    /// How each message was presented, by scope.
    pub(crate) presentations: HashMap<Scope, Presentation>,
    /// Text runs whose colour was changed.
    pub(crate) repaired: u32,
    /// Messages where high contrast cannot reach its floor on paper: they
    /// open in Reader view instead (FR-013b).
    pub(crate) unreachable: Vec<Scope>,
    /// How many nodes the first layout had, to check the second matches.
    pub(crate) nodes: usize,
}

impl Plan {
    pub(crate) fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }
}

#[derive(Default)]
struct Rules {
    marks: HashMap<NodeId, usize>,
    order: Vec<NodeId>,
    decls: HashMap<NodeId, Vec<String>>,
}

impl Rules {
    fn add(&mut self, node: NodeId, decl: String) {
        if !self.marks.contains_key(&node) {
            self.marks.insert(node, self.order.len());
            self.order.push(node);
        }
        self.decls.entry(node).or_default().push(decl);
    }
}

fn css(rgb: Rgb) -> String {
    let [r, g, b] = rgb.to_u8();
    format!("rgb({r},{g},{b})")
}

pub(crate) fn srgb(colour: &style::color::AbsoluteColor) -> (Rgb, f32) {
    let [r, g, b, a] = *colour
        .to_color_space(style::color::ColorSpace::Srgb)
        .raw_components();
    (
        Rgb {
            r: f64::from(r).clamp(0.0, 1.0),
            g: f64::from(g).clamp(0.0, 1.0),
            b: f64::from(b).clamp(0.0, 1.0),
        },
        a,
    )
}

/// A node's own background colour, and its alpha.
fn background(node: &Node) -> Option<(Rgb, f32)> {
    let styles = node.primary_styles()?;
    let current = styles.clone_color();
    Some(srgb(
        &styles
            .get_background()
            .background_color
            .resolve_to_absolute(&current),
    ))
}

/// The mean colour of a node's first decoded background image, if any.
fn image_ground(node: &Node) -> Option<Rgb> {
    let element = node.element_data()?;
    for image in element.background_images.iter().flatten() {
        if let blitz_dom::node::ImageData::Raster(raster) = &image.image {
            let data: &[u8] = raster.data.as_ref();
            let (mut sum, mut weight) = ([0.0f64; 3], 0.0f64);
            for px in data.as_chunks::<4>().0 {
                let a = f64::from(px[3]) / 255.0;
                for c in 0..3 {
                    sum[c] += f64::from(px[c]) * a;
                }
                weight += a;
            }
            if weight > 0.0 {
                return Some(Rgb::from_u8(
                    (sum[0] / weight) as u8,
                    (sum[1] / weight) as u8,
                    (sum[2] / weight) as u8,
                ));
            }
        }
    }
    None
}

fn has_image(node: &Node) -> bool {
    node.primary_styles().is_some_and(|s| {
        s.get_background()
            .background_image
            .0
            .iter()
            .any(|i| !matches!(i, style::values::computed::image::Image::None))
    })
}

fn element_of(doc: &BaseDocument, id: NodeId) -> Option<&Node> {
    let node = doc.get_node(id)?;
    if node.is_element() {
        Some(node)
    } else {
        node.parent.and_then(|parent| doc.get_node(parent))
    }
}

/// The colour painted behind `id` (research R10): its ancestors'
/// backgrounds composited, nearest last, over the first opaque one --
/// with `overrides` standing in for what the presentation changed, and an
/// image's mean colour for a ground that is a picture.
fn ground(doc: &BaseDocument, id: NodeId, overrides: &HashMap<NodeId, Option<Rgb>>) -> Rgb {
    let mut layers: Vec<(Rgb, f64)> = Vec::new();
    let mut base = Rgb::from_u8(255, 255, 255);
    let mut at = doc.get_node(id);
    while let Some(node) = at {
        let layer = match overrides.get(&node.id) {
            Some(Some(rgb)) => Some((*rgb, 1.0)),
            Some(None) => None,
            None => match image_ground(node) {
                Some(mean) => Some((mean, 1.0)),
                None => background(node)
                    .filter(|(_, alpha)| *alpha > 0.0)
                    .map(|(rgb, alpha)| (rgb, f64::from(alpha))),
            },
        };
        if let Some((rgb, alpha)) = layer {
            if alpha >= 0.999 {
                base = rgb;
                break;
            }
            layers.push((rgb, alpha));
        }
        at = node.parent.and_then(|parent| doc.get_node(parent));
    }
    layers.iter().rev().fold(base, |under, (over, a)| Rgb {
        r: over.r * a + under.r * (1.0 - a),
        g: over.g * a + under.g * (1.0 - a),
        b: over.b * a + under.b * (1.0 - a),
    })
}

fn descendants(doc: &BaseDocument, root: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        out.push(id);
        if let Some(node) = doc.get_node(id) {
            stack.extend(node.children.iter().copied());
        }
    }
    out
}

/// What the stylesheets in the document say about dark support for the
/// message scoped `scope`.
fn styles_declare_dark(sheets: &str, scope: &str) -> bool {
    let selector = if scope.is_empty() {
        String::new()
    } else {
        format!("data-postio-message=\"{scope}\"")
    };
    sheets.split("@media").skip(1).any(|block| {
        let head = block.split('{').next().unwrap_or("");
        head.contains("prefers-color-scheme") && head.contains("dark") && block.contains(&selector)
    })
}

fn facts(
    doc: &BaseDocument,
    container: &Node,
    sheets: &str,
    scope: &str,
) -> (MessageFacts, Option<NodeId>) {
    let all = descendants(doc, container.id);
    let canvas = all.iter().copied().find(|id| {
        doc.get_node(*id).is_some_and(|n| {
            n.element_data().is_some_and(|e| {
                e.attr(local_name!("class"))
                    .is_some_and(|c| c.split_whitespace().any(|c| c == "postio-canvas"))
            })
        })
    });
    let canvas_colour = canvas
        .and_then(|id| doc.get_node(id))
        .and_then(background)
        .filter(|(_, alpha)| *alpha > 0.5)
        .map(|(rgb, _)| rgb);
    let inner_background = all.iter().copied().any(|id| {
        id != container.id
            && Some(id) != canvas
            && doc.get_node(id).is_some_and(|n| {
                n.is_element() && (background(n).is_some_and(|(_, a)| a > 0.5) || has_image(n))
            })
    });
    let scheme = canvas
        .and_then(|id| doc.get_node(id))
        .and_then(|n| n.attr(LocalName::from("data-postio-color-scheme")))
        .is_some_and(|s| s.contains("dark"));
    (
        MessageFacts {
            canvas: canvas_colour,
            inner_background,
            declares_dark: scheme || styles_declare_dark(sheets, scope),
        },
        canvas,
    )
}

/// The senders' stylesheets: every `<style>` outside the `<head>`, which is
/// where `postio-ui` puts them. Postio's own sheets are in the head, and
/// their dark-mode rules say nothing about the sender.
fn sheets(doc: &BaseDocument) -> String {
    let mut out = String::new();
    for id in doc.query_selector_all("body style").unwrap_or_default() {
        if let Some(node) = doc.get_node(id) {
            for child in node.children.iter().copied() {
                if let Some(text) = doc.get_node(child).and_then(|n| n.text_data()) {
                    out.push_str(&text.content);
                    out.push('\n');
                }
            }
        }
    }
    out
}

/// The app-colours treatment's half of the theme rule (T211): the sender's
/// colours are gone from the markup already, and what is left -- the app's
/// ink and accent, and a colour the sender set on purpose -- is held to the
/// floor against what is painted behind it, a colour that misses it drawn
/// in the container's ink instead ([`theme::guard`]). No background is
/// changed. The white canvas an image keeps behind it in dark (FR-015) is
/// the treatment's stylesheet's (`postio-ui`'s `treatment.css`): a mark per
/// image here made every newsletter in dark lay out twice (T218).
fn guard_app_colours(
    doc: &BaseDocument,
    container: &Node,
    floor: f64,
    rules: &mut Rules,
    plan: &mut Plan,
) {
    let Some(ink) = container
        .primary_styles()
        .map(|styles| srgb(&styles.clone_color()).0)
    else {
        return;
    };
    let mut runs = Vec::new();
    crate::snapshot::text_runs(doc, container.id, &mut runs);
    let mut seen = std::collections::HashSet::new();
    let grounds = HashMap::new();
    for (brush, _) in runs {
        let Some(element) = element_of(doc, brush) else {
            continue;
        };
        if !seen.insert(element.id) {
            continue;
        }
        let Some(styles) = element.primary_styles() else {
            continue;
        };
        let (colour, _) = srgb(&styles.clone_color());
        let behind = ground(doc, element.id, &grounds);
        let kept = theme::guard(colour, behind, ink, floor);
        if kept.to_u8() != colour.to_u8() {
            plan.repaired += 1;
            rules.add(element.id, format!("color: {} !important", css(kept)));
        }
    }
}

/// Decide, from the first layout, what the second must change.
pub(crate) fn plan(doc: &BaseDocument, request: &RenderRequest) -> Plan {
    let theme = request.theme;
    let floor = theme::floor(theme);
    let sheets = sheets(doc);
    let mut rules = Rules::default();
    let mut plan = Plan {
        nodes: doc.tree().len(),
        ..Plan::default()
    };
    let containers = doc
        .query_selector_all(&format!("div.{}", postio_body::sanitize::BODY_CLASS))
        .unwrap_or_default();
    for container_id in containers {
        let Some(container) = doc.get_node(container_id) else {
            continue;
        };
        let scope = container
            .attr(LocalName::from(postio_body::sanitize::MESSAGE_ATTRIBUTE))
            .unwrap_or_default()
            .to_owned();
        // A body drawn under a treatment (specs/007-postio-focus T211,
        // T212) has had its rule applied already: paper is never touched,
        // and app colours is held to the contrast guard alone.
        match container
            .attr(LocalName::from(postio_body::treatment::TREATMENT_ATTRIBUTE))
            .and_then(postio_body::treatment::Treatment::from_attribute)
        {
            Some(postio_body::treatment::Treatment::Paper) => {
                plan.presentations.insert(
                    scope,
                    if theme.dark {
                        Presentation::Paper
                    } else {
                        Presentation::Styled
                    },
                );
                continue;
            }
            Some(postio_body::treatment::Treatment::AppColours) => {
                plan.presentations.insert(
                    scope,
                    if theme.dark {
                        Presentation::Adapted
                    } else {
                        Presentation::Styled
                    },
                );
                guard_app_colours(doc, container, floor, &mut rules, &mut plan);
                continue;
            }
            None => {}
        }
        let (facts, canvas) = facts(doc, container, &sheets, &scope);
        let presentation = theme::classify(facts, theme, request.darkened.contains(&scope));
        plan.presentations.insert(scope.clone(), presentation);
        let paper = facts.canvas.unwrap_or(Rgb::from_u8(255, 255, 255));
        let mut grounds: HashMap<NodeId, Option<Rgb>> = HashMap::new();
        let all = descendants(doc, container_id);
        match presentation {
            Presentation::Paper => {
                grounds.insert(container_id, Some(paper));
            }
            Presentation::Darkened => {
                grounds.insert(container_id, Some(theme::darken(paper)));
                for id in &all {
                    let Some(node) = doc.get_node(*id) else {
                        continue;
                    };
                    if let Some((rgb, alpha)) = background(node)
                        && alpha > 0.0
                        && *id != container_id
                    {
                        grounds.insert(*id, Some(theme::darken(rgb)));
                    }
                    if let Some(styles) = node.primary_styles() {
                        let border = styles.get_border();
                        if border.border_top_width.0.to_f32_px() > 0.0 {
                            let (rgb, alpha) = srgb(
                                &border
                                    .border_top_color
                                    .resolve_to_absolute(&styles.clone_color()),
                            );
                            if alpha > 0.0 {
                                rules.add(
                                    *id,
                                    format!("border-color: {} !important", css(theme::darken(rgb))),
                                );
                            }
                        }
                    }
                }
            }
            Presentation::Adapted => {
                if let Some(canvas) = canvas {
                    grounds.insert(canvas, None);
                }
            }
            Presentation::Styled | Presentation::SenderDark => {}
        }
        // An image keeps its intended canvas behind it (FR-015): a
        // transparent logo is never composited over the dark ground.
        if matches!(presentation, Presentation::Adapted | Presentation::Darkened) {
            for id in &all {
                if doc
                    .get_node(*id)
                    .and_then(|n| n.element_data())
                    .is_some_and(|e| e.name.local == local_name!("img"))
                {
                    rules.add(*id, format!("background-color: {} !important", css(paper)));
                }
            }
        }
        for (id, rgb) in &grounds {
            match rgb {
                Some(rgb) => rules.add(*id, format!("background-color: {} !important", css(*rgb))),
                None => rules.add(*id, "background-color: transparent !important".to_owned()),
            }
        }
        // Every run, repaired against its ground as the second layout will
        // paint it.
        let mut runs = Vec::new();
        crate::snapshot::text_runs(doc, container_id, &mut runs);
        let mut seen = std::collections::HashSet::new();
        for (brush, _) in runs {
            let Some(element) = element_of(doc, brush) else {
                continue;
            };
            if !seen.insert(element.id) {
                continue;
            }
            let Some(styles) = element.primary_styles() else {
                continue;
            };
            let (colour, _) = srgb(&styles.clone_color());
            let behind = ground(doc, element.id, &grounds);
            let fixed = theme::repair(colour, behind, floor);
            if theme::contrast(fixed, behind) < floor - 1e-6
                && theme.high_contrast
                && presentation == Presentation::Paper
                && !plan.unreachable.contains(&scope)
            {
                plan.unreachable.push(scope.clone());
            }
            if fixed.to_u8() != colour.to_u8() {
                plan.repaired += 1;
                rules.add(element.id, format!("color: {} !important", css(fixed)));
            }
        }
    }
    for id in &rules.order {
        let mark = rules.marks[id];
        plan.marks.push((*id, mark));
        plan.css.push_str(&format!(
            "[{MARK}=\"{mark}\"] {{ {} }}\n",
            rules.decls[id].join("; ")
        ));
    }
    plan
}
