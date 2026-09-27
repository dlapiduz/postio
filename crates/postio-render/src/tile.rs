//! Tiles (research R8): the recorded display list rasterised a band at a
//! time, so the widget holds only what is on screen and near it, however
//! tall the message.

use anyrender::ImageRenderer as _;

use crate::RenderedDocument;

/// One band of a document, in device pixels at the snapshot's scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileSpec {
    /// The band's top, from the document's top.
    pub top: u32,
    /// The band's height; the last one may run past the document's end.
    pub height: u32,
}

impl TileSpec {
    /// The `index`th band of `height` pixels.
    pub fn nth(index: u32, height: u32) -> TileSpec {
        TileSpec {
            top: index * height,
            height,
        }
    }
}

/// The document's width in device pixels: every tile's width.
pub fn tile_width(doc: &RenderedDocument) -> u32 {
    (doc.size.width * doc.scale).ceil().max(1.0) as u32
}

/// Rasterise `tile` of `doc` into `out`, premultiplied RGBA, row by row:
/// `tile_width(doc) * tile.height * 4` bytes. Pure, and callable from any
/// thread: the display list is plain data.
pub fn rasterize_tile(doc: &RenderedDocument, tile: TileSpec, out: &mut [u8]) {
    let width = tile_width(doc);
    let mut renderer = anyrender_vello_cpu::VelloCpuImageRenderer::new(width, tile.height);
    renderer.render(
        |scene| {
            anyrender::PaintScene::append_scene(
                scene,
                doc.display_list.clone(),
                kurbo::Affine::translate((0.0, -f64::from(tile.top))),
            )
        },
        out,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn document() -> RenderedDocument {
        let fonts = crate::fonts::FontSet::new(crate::fonts::Bundled {
            faces: Vec::new(),
            sans: "Liberation Sans",
            mono: "Liberation Mono",
        });
        let html = "<!DOCTYPE html><html><body style=\"margin:0\">\
            <div style=\"height:130px;background:linear-gradient(#f00,#00f)\"></div>\
            <div style=\"height:97px;border:3px solid #0a0;border-radius:12px\"></div>\
            <div style=\"height:61px;background:#123456\"></div></body></html>";
        let request = crate::RenderRequest {
            generation: 1,
            document: html.to_owned(),
            plain_text: String::new(),
            over_cap: None,
            resources: Arc::new(crate::Resources::new()),
            viewport: crate::Viewport {
                width: 300.0,
                hidpi_scale: 1.5,
                zoom: 1.0,
            },
            theme: crate::Theme::default(),
            darkened: Vec::new(),
            toggled_folds: Vec::new(),
            reader_view: Vec::new(),
        };
        crate::render(&request, &fonts)
    }

    /// Tiles stacked are the full raster, byte for byte: a seam would show
    /// as a line across the message wherever two tiles meet.
    #[test]
    fn stacked_tiles_are_the_full_raster() {
        let doc = document();
        let full = crate::rasterize(&doc);
        let width = tile_width(&doc);
        assert_eq!(width, full.width);
        let height = 64;
        let mut stacked = Vec::new();
        let mut index = 0;
        while index * height < full.height {
            let mut tile = vec![0u8; (width * height * 4) as usize];
            rasterize_tile(&doc, TileSpec::nth(index, height), &mut tile);
            stacked.extend_from_slice(&tile);
            index += 1;
        }
        stacked.truncate(full.rgba.len());
        assert!(
            stacked == full.rgba,
            "the stacked tiles differ from the full raster"
        );
    }
}
