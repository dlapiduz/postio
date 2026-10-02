//! HiDPI (research R1, R8): a render at the surface's scale is sharp. The
//! spike painted at 1.0 and let the compositor stretch it 2x, which is why
//! it looked blurry on a HiDPI screen.

use std::sync::Arc;

use postio_render::{Raster, RenderRequest, Resources, Theme, Viewport};

fn fonts() -> &'static postio_render::fonts::FontSet {
    static FONTS: std::sync::OnceLock<postio_render::fonts::FontSet> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| {
        postio_render::fonts::FontSet::new(postio_render::fonts::Bundled {
            faces: postio_ui::reader::document::FACES
                .iter()
                .map(|face| face.bytes)
                .collect(),
            sans: "Barlow",
            mono: "IBM Plex Mono",
        })
    })
}

fn raster(scale: f64) -> Raster {
    let request = RenderRequest {
        generation: 1,
        document: "<!DOCTYPE html><html><body style=\"margin:8px;background:#fff;color:#000\">\
            <p style=\"font-size:15px\">The quick brown fox jumps over the lazy dog, twice: \
            the quick brown fox jumps over the lazy dog.</p></body></html>"
            .to_owned(),
        plain_text: String::new(),
        fallback: None,
        over_cap: None,
        resources: Arc::new(Resources::new()),
        viewport: Viewport {
            width: 500.0,
            hidpi_scale: scale,
            zoom: 1.0,
        },
        theme: Theme::default(),
        darkened: Vec::new(),
        toggled_folds: Vec::new(),
        reader_view: Vec::new(),
    };
    postio_render::rasterize(&postio_render::render(&request, fonts()))
}

/// A 1x raster stretched 2x the way a compositor does it: bilinear.
fn stretched(one: &Raster) -> Vec<u8> {
    let (w, h) = (one.width as usize * 2, one.height as usize * 2);
    let mut out = vec![0u8; w * h];
    let grey = |x: usize, y: usize| {
        let i = (y * one.width as usize + x) * 4;
        f64::from(one.rgba[i])
    };
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = ((x as f64 + 0.5) / 2.0 - 0.5, (y as f64 + 0.5) / 2.0 - 0.5);
            let (x0, y0) = (fx.floor().max(0.0) as usize, fy.floor().max(0.0) as usize);
            let (x1, y1) = (
                (x0 + 1).min(one.width as usize - 1),
                (y0 + 1).min(one.height as usize - 1),
            );
            let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
            let v = grey(x0, y0) * (1.0 - tx) * (1.0 - ty)
                + grey(x1, y0) * tx * (1.0 - ty)
                + grey(x0, y1) * (1.0 - tx) * ty
                + grey(x1, y1) * tx * ty;
            out[y * w + x] = v.round() as u8;
        }
    }
    out
}

/// The share of inked pixels that are neither ink nor paper: edge blur.
fn blur(greys: impl Iterator<Item = u8>) -> f64 {
    let (mut inked, mut between) = (0usize, 0usize);
    for g in greys {
        if g < 250 {
            inked += 1;
            if g > 20 {
                between += 1;
            }
        }
    }
    between as f64 / inked.max(1) as f64
}

#[test]
fn a_render_at_scale_2_is_sharper_than_scale_1_stretched() {
    let native = raster(2.0);
    let native_blur = blur(native.rgba.as_chunks::<4>().0.iter().map(|p| p[0]));
    let stretched_blur = blur(stretched(&raster(1.0)).into_iter());
    assert!(
        native_blur < stretched_blur * 0.85,
        "native 2x edge share {native_blur:.3} is not clearly below stretched {stretched_blur:.3}"
    );
}
