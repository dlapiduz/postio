//! One request, laid out, recorded and measured -- synchronously, on the
//! caller's thread. The render thread (research R6) runs this; nothing else
//! should call it on the UI thread.

use std::sync::Arc;

use anyrender::ImageRenderer as _;
use blitz_dom::DocumentConfig;
use blitz_traits::shell::ColorScheme;

use crate::fonts::FontSet;
use crate::{Outcome, RenderCounts, RenderRequest, RenderedDocument};

/// What relative URLs resolve against. Without a base Blitz resolves
/// against a `data:` URL, which cannot be one, and panics on the first
/// `<img src="x">` (research R1).
pub const BASE_URL: &str = "postio-message://message/";

/// What this renderer asks of every document, above the author's rules.
///
/// A message's box does not scroll inside the snapshot: the view has one
/// scroller, the page. Content wider than the pane widens the document
/// instead, and the view scrolls it sideways (FR-021a, SC-009). The shared
/// `reader.css` keeps `overflow-x: auto` for the macOS reader's web view.
///
/// What `overflow-x: auto` also did there, implicitly, is kept explicitly:
/// the box clips vertically and is its own formatting context, so a
/// sender's negative margin or float cannot reach the next message
/// (#1346). `flow-root` keeps a float's height inside the box, so the clip
/// cuts only what escaped it.
const RENDERER_CSS: &str = ".postio-body { overflow-x: visible !important; \
                            overflow-y: clip !important; display: flow-root !important; }";

/// Lay out and record `request`'s document, drawing with `fonts`.
pub fn render(request: &RenderRequest, fonts: &FontSet) -> RenderedDocument {
    render_unless(request, fonts, &|| false).expect("a render that is never stale finishes")
}

/// [`render`], giving up between passes once `stale` says nobody will see
/// the result: a layout cannot be interrupted, but a superseded render
/// need not start another (T218).
pub(crate) fn render_unless(
    request: &RenderRequest,
    fonts: &FontSet,
    stale: &dyn Fn() -> bool,
) -> Option<RenderedDocument> {
    let scale = request.viewport.hidpi_scale;
    let mut zoom = request.viewport.zoom;
    let mut doc = lay_out(request, fonts, None, zoom);
    let (resolved, unresolved) = request.resources.counts();
    let placeholdered = request.resources.placeholdered();
    // The theme rule (research R10): classify, then repair against what is
    // actually painted, in a second layout if anything changes.
    let plan = crate::present::plan(&doc, request);
    let mut style_passes = 1;
    if !plan.is_empty() {
        if stale() {
            return None;
        }
        let second = lay_out(request, fonts, Some(&plan), zoom);
        // The same markup parses to the same nodes; if it somehow did not,
        // the marks would land on the wrong ones, so keep the first layout.
        if second.tree().len() == plan.nodes {
            doc = second;
            style_passes = 2;
        }
    }
    // Paper wider than the column is zoomed to fit (T207, T212): a 640px
    // newsletter in a 480px column is drawn whole and smaller, never cut
    // off and never scrolled sideways -- until fitting would take it under
    // `PAPER_FIT_FLOOR`, below which it stays at that scale and scrolls.
    // The fit multiplies the reader's own zoom rather than replacing it, so
    // Ctrl+plus still means "larger" on a fitted sheet.
    let fit = paper_fit(&doc);
    if fit < 1.0 {
        if stale() {
            return None;
        }
        zoom *= fit;
        let plan = (!plan.is_empty()).then_some(&plan);
        doc = lay_out(request, fonts, plan, zoom);
        style_passes += 1;
    }
    // Zoom (research R11): the document lays out at the pane's width over
    // the zoom, and is painted at the device scale times the zoom. The
    // snapshot's geometry is then in the view's own pixels -- CSS pixels
    // times the zoom -- so the widget never does zoom arithmetic.
    let mut size = doc.root_element().final_layout().size;
    // Wider than the pane -- a fixed 600px table at 150% -- widens the
    // document, so the view scrolls it sideways rather than losing it.
    size.width = size.width.max(content_width(&doc));
    let (width, height) = (
        (f64::from(size.width) * scale * zoom).ceil() as u32,
        (f64::from(size.height) * scale * zoom).ceil() as u32,
    );
    if stale() {
        return None;
    }
    let mut display_list = anyrender::Scene::new();
    blitz_paint::paint_scene(
        &mut display_list,
        &mut doc,
        scale * zoom,
        width,
        height,
        0,
        0,
    );
    let counts = RenderCounts {
        renders: 1,
        style_passes,
        nodes: u32::try_from(doc.tree().len()).unwrap_or(u32::MAX),
        repaired_runs: plan.repaired,
        resources_resolved: resolved,
        resources_unresolved: unresolved,
        images_placeholdered: placeholdered,
        display_list_commands: u32::try_from(display_list.commands.len()).unwrap_or(u32::MAX),
        ..RenderCounts::default()
    };
    let mut messages = crate::snapshot::messages(&doc);
    for message in &mut messages {
        if let Some(presentation) = plan.presentations.get(&message.scope) {
            message.presentation = *presentation;
        }
    }
    let mut document = RenderedDocument {
        generation: request.generation,
        size: kurbo::Size::new(f64::from(size.width) * zoom, f64::from(size.height) * zoom),
        scale,
        display_list,
        low_res: Raster {
            width: 0,
            height: 0,
            rgba: Vec::new(),
        },
        text: crate::text_index::build(&doc),
        links: crate::snapshot::links(&doc),
        messages,
        folds: crate::snapshot::folds(&doc),
        anchors: crate::snapshot::anchors(&doc),
        counts,
        outcome: Outcome::Rendered,
        needs_reader_view: plan.unreachable,
        fit,
        _live: crate::Live::new(),
    };
    zoom_geometry(&mut document, zoom);
    document.low_res = low_res(&document);
    Some(document)
}

/// The least a paper body is scaled to fit its column: postio-body's, so
/// the Mac's web view fits paper to the same floor.
pub const PAPER_FIT_FLOOR: f64 = postio_body::treatment::PAPER_FIT_FLOOR;

/// The scale that fits every paper body in the document into its own box,
/// at least [`PAPER_FIT_FLOOR`]; 1.0 when each already fits, or when nothing
/// in the document is on paper.
///
/// Measured against the sheet, not the page: the sheet is as wide as the
/// column, and what overflows it is the sender's layout. Zooming by the
/// sheet's share of the layout lays the document out wider by exactly as
/// much, so the layout fits the sheet at the new scale.
fn paper_fit(doc: &blitz_dom::BaseDocument) -> f64 {
    let selector = format!(
        "div.{}[{}=\"{}\"]",
        postio_body::sanitize::BODY_CLASS,
        postio_body::treatment::TREATMENT_ATTRIBUTE,
        postio_body::treatment::Treatment::Paper.attribute_value()
    );
    let sheets = doc.query_selector_all(&selector).unwrap_or_default();
    sheets
        .into_iter()
        .filter_map(|id| {
            let node = doc.get_node(id)?;
            let left = node.absolute_position(0.0, 0.0).x;
            let width = f64::from(node.final_layout().size.width);
            let wide = f64::from(right_edge(doc, id) - left);
            Some(fit_scale(wide, width))
        })
        .fold(1.0, f64::min)
}

/// How far `wide` CSS pixels of content must be scaled to fit `width`,
/// within `[PAPER_FIT_FLOOR, 1.0]`. Half a pixel of slack: a layout that
/// rounds a fraction past the column is not one to shrink.
pub fn fit_scale(wide: f64, width: f64) -> f64 {
    if wide <= width + 0.5 {
        return 1.0;
    }
    (width / wide).clamp(PAPER_FIT_FLOOR, 1.0)
}

/// The right edge of the furthest laid-out box, in CSS pixels.
fn content_width(doc: &blitz_dom::BaseDocument) -> f32 {
    right_edge(doc, doc.root_element().id)
}

/// The right edge of the furthest laid-out box under `root`, `root`'s own
/// included, in CSS pixels.
fn right_edge(doc: &blitz_dom::BaseDocument, root: blitz_dom::NodeId) -> f32 {
    let mut widest = 0.0f32;
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let Some(node) = doc.get_node(id) else {
            continue;
        };
        if node.is_element() {
            let size = node.final_layout().size;
            if size.width > 0.0 && size.height > 0.0 {
                widest = widest.max(node.absolute_position(0.0, 0.0).x + size.width);
            }
        }
        stack.extend(node.children.iter().copied());
    }
    widest
}

/// Every rect of the snapshot from CSS pixels into the view's, at `zoom`.
fn zoom_geometry(document: &mut RenderedDocument, zoom: f64) {
    if zoom == 1.0 {
        return;
    }
    let at = |rect: &mut crate::Rect| {
        *rect = crate::Rect::new(
            rect.x0 * zoom,
            rect.y0 * zoom,
            rect.x1 * zoom,
            rect.y1 * zoom,
        )
    };
    document
        .text
        .clusters
        .iter_mut()
        .for_each(|c| at(&mut c.rect));
    document.links.iter_mut().for_each(|l| at(&mut l.rect));
    document.messages.iter_mut().for_each(|m| at(&mut m.rect));
    document
        .folds
        .iter_mut()
        .for_each(|f| at(&mut f.summary_rect));
    document.anchors.iter_mut().for_each(|(_, y)| *y *= zoom);
}

/// Parse, style and lay out `request`'s document, with a plan's overrides
/// if there is one.
fn lay_out(
    request: &RenderRequest,
    fonts: &FontSet,
    plan: Option<&crate::present::Plan>,
    zoom: f64,
) -> blitz_dom::BaseDocument {
    let viewport = &request.viewport;
    let scale = viewport.hidpi_scale;
    let mut sheets = vec![blitz_dom::DEFAULT_CSS.to_owned(), RENDERER_CSS.to_owned()];
    if let Some(plan) = plan {
        sheets.push(plan.css.clone());
    }
    let config = DocumentConfig {
        viewport: Some({
            let mut blitz = blitz_traits::shell::Viewport::new(
                (viewport.width * scale).round() as u32,
                (900.0 * scale).round() as u32,
                scale as f32,
                if request.theme.dark {
                    ColorScheme::Dark
                } else {
                    ColorScheme::Light
                },
            );
            // Kept apart from the device scale, never folded into it.
            blitz.set_zoom(zoom as f32);
            blitz
        }),
        base_url: Some(BASE_URL.to_owned()),
        font_ctx: Some(fonts.context()),
        net_provider: Some(Arc::clone(&request.resources) as _),
        ua_stylesheets: Some(sheets),
        ..Default::default()
    };
    let mut doc = blitz_html::HtmlDocument::from_html(&request.document, config).into_inner();
    toggle_folds(&mut doc, &request.toggled_folds);
    if let Some(plan) = plan {
        let name = blitz_dom::QualName::new(
            None,
            blitz_dom::ns!(),
            blitz_dom::LocalName::from(crate::present::MARK),
        );
        let mut mutator = doc.mutate();
        for (id, mark) in &plan.marks {
            mutator.set_attribute(*id, name.clone(), &mark.to_string());
        }
    }
    doc.resolve(0.0);
    doc
}

/// The most bytes the low-resolution copy may take.
const LOW_RES_BUDGET: usize = 16 * 1024 * 1024;

/// The whole document at a quarter of its scale, or smaller still if a
/// quarter would not fit the budget: what shows where a tile is not ready.
fn low_res(doc: &RenderedDocument) -> Raster {
    let (width, height) = (doc.size.width * doc.scale, doc.size.height * doc.scale);
    let mut factor = 0.25;
    let bytes = |f: f64| (width * f).ceil() * (height * f).ceil() * 4.0;
    while bytes(factor) > LOW_RES_BUDGET as f64 {
        factor /= 2.0;
    }
    raster_at(doc, factor)
}

/// A whole document's pixels, at the scale it was rendered for.
#[derive(Clone, Debug)]
pub struct Raster {
    /// Device pixels across.
    pub width: u32,
    /// Device pixels down.
    pub height: u32,
    /// Premultiplied RGBA, row by row.
    pub rgba: Vec<u8>,
}

/// Rasterise all of `doc` at once. Tiles (T056) replace this for the
/// widget; tests and one-shot captures keep it.
pub fn rasterize(doc: &RenderedDocument) -> Raster {
    raster_at(doc, 1.0)
}

/// All of `doc`, at `factor` times the scale it was recorded at.
fn raster_at(doc: &RenderedDocument, factor: f64) -> Raster {
    let width = (doc.size.width * doc.scale * factor).ceil().max(1.0) as u32;
    let height = (doc.size.height * doc.scale * factor).ceil().max(1.0) as u32;
    let mut renderer = anyrender_vello_cpu::VelloCpuImageRenderer::new(width, height);
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    renderer.render(
        |scene| {
            anyrender::PaintScene::append_scene(
                scene,
                doc.display_list.clone(),
                kurbo::Affine::scale(factor),
            )
        },
        &mut rgba,
    );
    Raster {
        width,
        height,
        rgba,
    }
}

/// The attribute `postio-ui` stamps on every `<details>` it composes: the
/// fold's stable id, the same across re-renders.
pub const FOLD_ATTRIBUTE: &str = "data-postio-fold";

/// Flip `open` on every fold `toggled` names, before the first style pass,
/// so the document is laid out once, already in the state asked for.
fn toggle_folds(doc: &mut blitz_dom::BaseDocument, toggled: &[String]) {
    if toggled.is_empty() {
        return;
    }
    let Ok(folds) = doc.query_selector_all(&format!("details[{FOLD_ATTRIBUTE}]")) else {
        return;
    };
    let flips: Vec<(blitz_dom::NodeId, bool)> = folds
        .into_iter()
        .filter_map(|id| {
            let node = doc.get_node(id)?;
            let fold = node.attr(blitz_dom::LocalName::from(FOLD_ATTRIBUTE))?;
            toggled
                .iter()
                .any(|t| t == fold)
                .then(|| (id, node.attr(blitz_dom::local_name!("open")).is_some()))
        })
        .collect();
    let open = blitz_dom::QualName::new(None, blitz_dom::ns!(), blitz_dom::local_name!("open"));
    let mut mutator = doc.mutate();
    for (id, was_open) in flips {
        if was_open {
            mutator.clear_attribute(id, open.clone());
        } else {
            mutator.set_attribute(id, open.clone(), "");
        }
    }
}
