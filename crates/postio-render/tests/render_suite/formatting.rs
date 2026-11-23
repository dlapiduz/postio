//! A formatted message as a Postio reader draws it (spec 014 T020): the
//! golden document's HTML part, through the reader's sanitiser, with every
//! construct still there to be seen.
//!
//! The other half of `postio-body`'s wire contract. That pins what is sent;
//! this asserts that what is sent survives the sanitiser that every
//! received message goes through, so two Postio users see each other's
//! formatting.

use std::sync::Arc;

use postio_body::RemoteImages;
use postio_model::MessageBody;
use postio_render::{RenderRequest, RenderedDocument, Resources, Theme, Viewport};
use postio_ui::reader::document::{self, Rendering};

fn fonts() -> &'static postio_render::fonts::FontSet {
    static FONTS: std::sync::OnceLock<postio_render::fonts::FontSet> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| {
        postio_render::fonts::FontSet::new(postio_render::fonts::Bundled {
            faces: document::FACES.iter().map(|face| face.bytes).collect(),
            sans: "Barlow",
            mono: "IBM Plex Mono",
        })
    })
}

/// The golden message as it arrives: its two parts, read by the reader.
fn received() -> (String, RenderedDocument) {
    let (text, html) = postio_body::render(&postio_body::golden::golden());
    let body = MessageBody {
        text: Some(text),
        html: Some(html),
    };
    let rendered = document::body_html_in(&body, RemoteImages::Blocked, Rendering::Original, None);
    let sheet = document::sheet_for(Rendering::Original, false);
    let page = document::document_for(
        &rendered.html,
        &rendered.styles,
        RemoteImages::Blocked,
        sheet,
    );
    let request = RenderRequest {
        generation: 1,
        document: page,
        plain_text: String::new(),
        fallback: None,
        over_cap: None,
        resources: Arc::new(Resources::new()),
        viewport: Viewport {
            width: 800.0,
            hidpi_scale: 1.0,
            zoom: 1.0,
        },
        theme: Theme::default(),
        darkened: Vec::new(),
        toggled_folds: Vec::new(),
        reader_view: Vec::new(),
    };
    (rendered.html, postio_render::render(&request, fonts()))
}

#[test]
fn every_construct_survives_the_readers_sanitiser() {
    let (html, _) = received();
    for kept in [
        "<h1>",
        "<h2>",
        "<del>",
        "<code>",
        "<ol start=\"7\">",
        "<blockquote>",
        "<pre",
        "<hr",
        "<table",
        "border=\"1\"",
        "align=\"right\"",
        "\u{2610} open task",
        "\u{2611} done task",
        "<br>",
    ] {
        assert!(html.contains(kept), "{kept:?} did not survive: {html}");
    }
}

#[test]
fn every_construct_is_drawn_in_reading_order() {
    let (_, document) = received();
    let text = &document.text.text;
    let mut from = 0;
    for seen in [
        "Plan",
        "struck",
        "code",
        "link",
        "Steps",
        "first",
        "nested",
        "\u{2610} open task",
        "\u{2611} done task",
        "seventh",
        // The reader folds a quote, as it does every quote; the sanitiser
        // case above holds that it is still a `<blockquote>`.
        "2 quoted lines",
        "fn main() {}",
        "Item\tQty",
        "Widget\t3",
        "line one",
        "line two",
    ] {
        let at = text[from..]
            .find(seen)
            .unwrap_or_else(|| panic!("{seen:?} is not drawn after byte {from}: {text:?}"));
        from += at + seen.len();
    }
}
