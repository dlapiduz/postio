//! One request, laid out, recorded and measured synchronously on the caller's thread; never call it
//! on the UI thread.

use std::sync::Arc;

use anyrender::ImageRenderer as _;
use blitz_dom::DocumentConfig;
use blitz_traits::shell::ColorScheme;

use crate::fonts::FontSet;
use crate::{Outcome, RenderCounts, RenderRequest, RenderedDocument};

/// Without a base Blitz resolves against a `data:` URL, which cannot be one, and panics on the
/// first `<img src="x">`.
pub const BASE_URL: &str = "postio-message://message/";

/// A message's box does not scroll inside the snapshot: wider content widens the
/// document and the view scrolls it sideways. The box clips vertically and is its
/// own formatting context, so a sender's negative margin or float cannot reach
/// the next message (#1346).
const RENDERER_CSS: &str = ".postio-body { overflow-x: visible !important; \
                            overflow-y: clip !important; display: flow-root !important; }";

pub fn render(request: &RenderRequest, fonts: &FontSet) -> RenderedDocument {
    render_unless(request, fonts, &|| false).expect("a render that is never stale finishes")
}

/// Gives up between passes once `stale` says nobody will see the result: a layout cannot be
/// interrupted.
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
    // Paper wider than the column is zoomed to fit, down to `PAPER_FIT_FLOOR`,
    // below which it scrolls. The fit multiplies the reader's own zoom, so Ctrl+plus
    // still means "larger" on a fitted sheet.
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
    // The document lays out at the pane's width over the zoom and paints at the
    // device scale times the zoom, so the snapshot's geometry is in the view's pixels.
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

pub const PAPER_FIT_FLOOR: f64 = postio_body::treatment::PAPER_FIT_FLOOR;

/// 1.0 when each fits or nothing is on paper. Measured against the sheet, not the
/// page: zooming by the sheet's share lays the document out wider by exactly as much.
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

/// Half a pixel of slack: a layout that rounds a fraction past the column is not one to shrink.
pub fn fit_scale(wide: f64, width: f64) -> f64 {
    if wide <= width + 0.5 {
        return 1.0;
    }
    (width / wide).clamp(PAPER_FIT_FLOOR, 1.0)
}

fn content_width(doc: &blitz_dom::BaseDocument) -> f32 {
    right_edge(doc, doc.root_element().id)
}

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
            blitz.set_zoom(zoom as f32);
            blitz
        }),
        base_url: Some(BASE_URL.to_owned()),
        font_ctx: Some(fonts.context()),
        net_provider: Some(Arc::clone(&request.resources) as _),
        ua_stylesheets: sheets
            .iter()
            .map(|css| blitz_dom::UserAgentStylesheet::parse(css))
            .collect(),
        ..Default::default()
    };
    let mut doc = blitz_html::HtmlDocument::from_html(&request.document, config).into_inner();
    toggle_folds(&mut doc, &request.toggled_folds);
    mark_missing_parts(&mut doc, &request.resources);
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

/// Left alone it keeps its declared size and draws nothing: a blank band (#1764).
/// Only an `<img>`: a CSS background naming a missing part stays unpainted, since
/// a placeholder tiled behind text would cover it.
fn mark_missing_parts(doc: &mut blitz_dom::BaseDocument, resources: &crate::Resources) {
    let src = blitz_dom::local_name!("src");
    let missing: Vec<blitz_dom::NodeId> = doc
        .query_selector_all("img[src]")
        .unwrap_or_default()
        .into_iter()
        .filter(|id| {
            doc.get_node(*id)
                .and_then(|node| node.attr(src.clone()))
                .is_some_and(|url| resources.is_missing_part(url))
        })
        .collect();
    if missing.is_empty() {
        return;
    }
    let name = blitz_dom::QualName::new(None, blitz_dom::ns!(), src);
    let mut mutator = doc.mutate();
    for id in missing {
        mutator.set_attribute(id, name.clone(), crate::resources::MISSING_PART);
    }
}

const LOW_RES_BUDGET: usize = 16 * 1024 * 1024;

fn low_res(doc: &RenderedDocument) -> Raster {
    let (width, height) = (doc.size.width * doc.scale, doc.size.height * doc.scale);
    let mut factor = 0.25;
    let bytes = |f: f64| (width * f).ceil() * (height * f).ceil() * 4.0;
    while bytes(factor) > LOW_RES_BUDGET as f64 {
        factor /= 2.0;
    }
    raster_at(doc, factor)
}

#[derive(Clone, Debug)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn rasterize(doc: &RenderedDocument) -> Raster {
    raster_at(doc, 1.0)
}

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

/// The fold's stable id, the same across re-renders.
pub const FOLD_ATTRIBUTE: &str = "data-postio-fold";

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
