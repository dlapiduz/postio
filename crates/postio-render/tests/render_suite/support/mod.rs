//! What the renderer's integration tests share: the process's fonts, and a
//! corpus fixture composed the way the reader composes it.

#![allow(dead_code)]

use std::sync::{Arc, OnceLock};

use postio_body::RemoteImages;
use postio_model::test_corpus;
use postio_render::fonts::{Bundled, FontSet};
use postio_render::{RenderRequest, RenderedDocument, Resources, Theme, Viewport};
use postio_ui::reader::document::{self, Rendering};

pub fn fonts() -> &'static FontSet {
    static FONTS: OnceLock<FontSet> = OnceLock::new();
    FONTS.get_or_init(|| {
        FontSet::new(Bundled {
            faces: document::FACES.iter().map(|face| face.bytes).collect(),
            sans: "Barlow",
            mono: "IBM Plex Mono",
        })
    })
}

pub const LIGHT: Theme = Theme {
    dark: false,
    high_contrast: false,
};
pub const DARK: Theme = Theme {
    dark: true,
    high_contrast: false,
};
pub const HIGH_CONTRAST: Theme = Theme {
    dark: true,
    high_contrast: true,
};

/// A request for `html` in `theme`, with no resources beyond the faces.
pub fn request_for(html: String, theme: Theme) -> RenderRequest {
    let resources = Resources::new();
    for face in document::FACES {
        resources.insert_font(face.name, face.bytes);
    }
    RenderRequest {
        generation: 1,
        document: html,
        plain_text: String::new(),
        fallback: None,
        over_cap: None,
        resources: Arc::new(resources),
        viewport: Viewport {
            width: 800.0,
            hidpi_scale: 1.0,
            zoom: 1.0,
        },
        theme,
        darkened: Vec::new(),
        toggled_folds: Vec::new(),
        reader_view: Vec::new(),
    }
}

/// A fixture as the reader composes it, scoped as message `scope` of a
/// conversation, with its own parts, in `theme`; `None` with no body.
pub fn request(name: &str, theme: Theme) -> Option<RenderRequest> {
    request_as(name, theme, Rendering::Original)
}

/// [`request`], composed as `rendering`: Reader view for FR-013b.
pub fn request_as(name: &str, theme: Theme, rendering: Rendering) -> Option<RenderRequest> {
    let parsed = postio_model::mime::parse(test_corpus::load(name).bytes());
    if parsed.body.is_empty() {
        return None;
    }
    let body = document::body_html_in(&parsed.body, RemoteImages::Blocked, rendering, None);
    let html = document::document_for(
        &body.html,
        &body.styles,
        RemoteImages::Blocked,
        document::sheet_for(rendering, false),
    );
    let mut request = request_for(html, theme);
    for part in &parsed.parts {
        if let Some(cid) = &part.attachment.content_id {
            request
                .resources
                .insert_part(None, cid, part.content.clone());
        }
    }
    request.plain_text = parsed.body.text.unwrap_or_default();
    request.over_cap = body.over_cap;
    Some(request)
}

pub fn render(request: &RenderRequest) -> RenderedDocument {
    postio_render::render(request, fonts())
}
