//! One request, laid out, recorded and measured -- synchronously, on the
//! caller's thread. The render thread (research R6) runs this; nothing else
//! should call it on the UI thread.

use std::sync::Arc;

use anyrender::ImageRenderer as _;
use blitz_dom::DocumentConfig;
use blitz_traits::shell::ColorScheme;

use crate::fonts::FontSet;
use crate::{Outcome, RenderCounts, RenderRequest, RenderedDocument, TextIndex};

/// What relative URLs resolve against. Without a base Blitz resolves
/// against a `data:` URL, which cannot be one, and panics on the first
/// `<img src="x">` (research R1).
pub const BASE_URL: &str = "postio-message://message/";

/// Lay out and record `request`'s document, drawing with `fonts`.
pub fn render(request: &RenderRequest, fonts: &FontSet) -> RenderedDocument {
    let viewport = &request.viewport;
    let scale = viewport.hidpi_scale;
    let config = DocumentConfig {
        viewport: Some(blitz_traits::shell::Viewport::new(
            (viewport.width * scale).round() as u32,
            (900.0 * scale).round() as u32,
            scale as f32,
            if request.theme.dark {
                ColorScheme::Dark
            } else {
                ColorScheme::Light
            },
        )),
        base_url: Some(BASE_URL.to_owned()),
        font_ctx: Some(fonts.context()),
        net_provider: Some(Arc::clone(&request.resources) as _),
        ..Default::default()
    };
    let mut doc = blitz_html::HtmlDocument::from_html(&request.document, config).into_inner();
    toggle_folds(&mut doc, &request.toggled_folds);
    doc.resolve(0.0);
    let size = doc.root_element().final_layout().size;
    let (width, height) = (
        (f64::from(size.width) * scale).ceil() as u32,
        (f64::from(size.height) * scale).ceil() as u32,
    );
    let mut display_list = anyrender::Scene::new();
    blitz_paint::paint_scene(&mut display_list, &mut doc, scale, width, height, 0, 0);
    let (resolved, unresolved) = request.resources.counts();
    let counts = RenderCounts {
        renders: 1,
        style_passes: 1,
        nodes: u32::try_from(doc.tree().len()).unwrap_or(u32::MAX),
        resources_resolved: resolved,
        resources_unresolved: unresolved,
        display_list_commands: u32::try_from(display_list.commands.len()).unwrap_or(u32::MAX),
        ..RenderCounts::default()
    };
    RenderedDocument {
        generation: request.generation,
        size: kurbo::Size::new(f64::from(size.width), f64::from(size.height)),
        scale,
        display_list,
        text: TextIndex::default(),
        links: Vec::new(),
        messages: Vec::new(),
        folds: Vec::new(),
        counts,
        outcome: Outcome::Rendered,
    }
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
    let width = (doc.size.width * doc.scale).ceil().max(1.0) as u32;
    let height = (doc.size.height * doc.scale).ceil().max(1.0) as u32;
    let mut renderer = anyrender_vello_cpu::VelloCpuImageRenderer::new(width, height);
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    renderer.render(
        |scene| {
            anyrender::PaintScene::append_scene(
                scene,
                doc.display_list.clone(),
                kurbo::Affine::IDENTITY,
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
