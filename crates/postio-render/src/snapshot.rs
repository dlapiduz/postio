//! What a render hands the UI besides pixels (research R7): the boxes the
//! widget acts on -- messages, links, folds -- read from the laid-out
//! document once, on the render thread, so no gesture ever asks the engine.

use blitz_dom::{BaseDocument, LocalName, Node, NodeId, local_name};
use parley::layout::PositionedLayoutItem;

use crate::{FoldBox, LinkBox, LinkTarget, MessageBox, Presentation, Rect, Verb};

/// The attribute a message container in a conversation carries: its scope.
const MESSAGE_ATTRIBUTE: &str = postio_body::sanitize::MESSAGE_ATTRIBUTE;

/// The reader's own verbs, as `postio-ui`'s thread document writes them
/// (`reader::thread::*_SCHEME`): followed by the message's scope.
const VERBS: &[(&str, Verb)] = &[
    ("postio-allow:", Verb::Allow),
    ("postio-reply:", Verb::Reply),
    ("postio-forward:", Verb::Forward),
    ("postio-continue:", Verb::Continue),
];

/// The border box of a laid-out node, in document CSS pixels.
fn border_box(node: &Node) -> Rect {
    let origin = node.absolute_position(0.0, 0.0);
    let size = node.final_layout().size;
    Rect::new(
        f64::from(origin.x),
        f64::from(origin.y),
        f64::from(origin.x + size.width),
        f64::from(origin.y + size.height),
    )
}

fn elements<'a>(doc: &'a BaseDocument, selector: &str) -> Vec<&'a Node> {
    doc.query_selector_all(selector)
        .map(|ids| ids.into_iter().filter_map(|id| doc.get_node(id)).collect())
        .unwrap_or_default()
}

/// Every message container, in document order. The single-message reader
/// names none, and its one container has the empty scope.
pub(crate) fn messages(doc: &BaseDocument) -> Vec<MessageBox> {
    elements(doc, &format!("div.{}", postio_body::sanitize::BODY_CLASS))
        .into_iter()
        .map(|node| {
            MessageBox {
                scope: node
                    .attr(LocalName::from(MESSAGE_ATTRIBUTE))
                    .unwrap_or_default()
                    .to_owned(),
                rect: border_box(node),
                // The theme rule (research R10) sets this when it runs.
                presentation: Presentation::Styled,
            }
        })
        .collect()
}

/// Every fold the thread document stamped, with its summary's box.
pub(crate) fn folds(doc: &BaseDocument) -> Vec<FoldBox> {
    elements(doc, &format!("details[{}]", crate::FOLD_ATTRIBUTE))
        .into_iter()
        .filter_map(|details| {
            let summary = details.children.iter().copied().find_map(|id| {
                let child = doc.get_node(id)?;
                (child.element_data()?.name.local == local_name!("summary")).then_some(child)
            })?;
            Some(FoldBox {
                id: details
                    .attr(LocalName::from(crate::FOLD_ATTRIBUTE))?
                    .to_owned(),
                summary_rect: border_box(summary),
                open: details.attr(local_name!("open")).is_some(),
            })
        })
        .collect()
}

/// Every element with an `id`, and where it starts: what a fragment link
/// scrolls to.
pub(crate) fn anchors(doc: &BaseDocument) -> Vec<(String, f64)> {
    elements(doc, "[id]")
        .into_iter()
        .filter_map(|node| {
            let id = node.attr(local_name!("id"))?.to_owned();
            Some((id, border_box(node).y0))
        })
        .collect()
}

/// Every link a reader may follow, one box per line it occupies: its text
/// runs, grouped under the `<a>` they belong to, and the box of any link
/// that lays out as a block of its own.
pub(crate) fn links(doc: &BaseDocument) -> Vec<LinkBox> {
    let mut out = Vec::new();
    for anchor in elements(doc, "a[href]") {
        let Some(target) = target(doc, anchor) else {
            continue;
        };
        let own = border_box(anchor);
        if own.area() > 0.0 {
            out.push(LinkBox {
                rect: own,
                target: target.clone(),
            });
        }
    }
    let mut runs = Vec::new();
    text_runs(doc, doc.root_node().id, &mut runs);
    for (node, rect) in runs {
        let Some(anchor) = ancestor(doc, node, |n| {
            n.element_data()
                .is_some_and(|e| e.name.local == local_name!("a"))
                && n.attr(local_name!("href")).is_some()
        }) else {
            continue;
        };
        if let Some(target) = target(doc, anchor) {
            out.push(LinkBox { rect, target });
        }
    }
    out
}

/// Where following `anchor` goes, or `None` if it is not a link a reader
/// follows: `javascript:`, `data:`, a part URI or anything else unknown.
fn target(doc: &BaseDocument, anchor: &Node) -> Option<LinkTarget> {
    let href = anchor.attr(local_name!("href"))?.trim();
    if let Some(id) = href.strip_prefix('#') {
        let scope = ancestor(doc, anchor.id, |n| {
            n.attr(LocalName::from(MESSAGE_ATTRIBUTE)).is_some()
        })?
        .attr(LocalName::from(MESSAGE_ATTRIBUTE))?
        .to_owned();
        return Some(LinkTarget::Fragment {
            scope,
            id: id.to_owned(),
        });
    }
    for (scheme, verb) in VERBS {
        if let Some(scope) = href.strip_prefix(scheme) {
            return Some(LinkTarget::Verb {
                scope: scope.to_owned(),
                verb: *verb,
            });
        }
    }
    let url = url::Url::parse(href).ok()?;
    matches!(url.scheme(), "http" | "https" | "mailto").then_some(LinkTarget::External(url))
}

/// The nearest node from `id` upward, itself included, that `want`s.
fn ancestor(doc: &BaseDocument, id: NodeId, want: impl Fn(&Node) -> bool) -> Option<&Node> {
    let mut at = doc.get_node(id);
    while let Some(node) = at {
        if want(node) {
            return Some(node);
        }
        at = node.parent.and_then(|parent| doc.get_node(parent));
    }
    None
}

/// Every positioned text run: the node its style came from, and its box.
pub(crate) fn text_runs(doc: &BaseDocument, id: NodeId, out: &mut Vec<(NodeId, Rect)>) {
    let Some(node) = doc.get_node(id) else {
        return;
    };
    if node.flags.is_inline_root()
        && let Some(text) = node
            .element_data()
            .and_then(|element| element.inline_layout_data.as_ref())
    {
        let origin = node.absolute_position(0.0, 0.0);
        let layout = node.final_layout();
        let (left, top) = (
            f64::from(origin.x + layout.padding.left + layout.border.left),
            f64::from(origin.y + layout.padding.top + layout.border.top),
        );
        for line in text.layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                if run.advance() <= 0.0 {
                    continue;
                }
                let metrics = run.run().metrics();
                let x = left + f64::from(run.offset());
                let y = top + f64::from(run.baseline() - metrics.ascent);
                out.push((
                    run.style().brush.id,
                    Rect::new(
                        x,
                        y,
                        x + f64::from(run.advance()),
                        y + f64::from(metrics.ascent + metrics.descent),
                    ),
                ));
            }
        }
    }
    for child in node.children.iter().copied() {
        text_runs(doc, child, out);
    }
}
