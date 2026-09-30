//! SC-007 across the corpus: every text-bearing fixture can be copied,
//! found, hit and followed, headlessly, through the renderer's snapshot.
//! The widget tests exercise one fixture each; this is what makes "every
//! text-bearing message" true.

use std::sync::Arc;

use postio_body::RemoteImages;
use postio_model::test_corpus;
use postio_render::{LinkTarget, RenderRequest, RenderedDocument, Resources, Theme, Viewport};
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

/// Each fixture as the reader composes it, rendered; `None` for one with no
/// body to read.
fn rendered(fixture: &test_corpus::Fixture) -> Option<RenderedDocument> {
    let parsed = postio_model::mime::parse(fixture.bytes());
    if parsed.body.is_empty() {
        return None;
    }
    let body = document::body_html_in(
        &parsed.body,
        RemoteImages::Blocked,
        Rendering::Original,
        Some("1"),
    );
    let html = document::document_for(
        &body.html,
        &body.styles,
        RemoteImages::Blocked,
        document::sheet_for(Rendering::Original, false),
    );
    let request = RenderRequest {
        generation: 1,
        document: html,
        plain_text: parsed.body.text.clone().unwrap_or_default(),
        over_cap: body.over_cap,
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
    let doc = postio_render::render(&request, fonts());
    (!doc.text.text.trim().is_empty()).then_some(doc)
}

#[test]
fn every_text_bearing_fixture_supports_copy_find_hit_and_links() {
    let mut swept = 0;
    for fixture in test_corpus::all() {
        let Some(doc) = rendered(fixture) else {
            continue;
        };
        swept += 1;
        let name = fixture.name();
        let index = &doc.text;
        let len = index.text.chars().count();
        assert_eq!(
            index.slice(0..len),
            index.text,
            "{name}: copying everything"
        );

        // A word from the message's own text: find it, and see it drawn
        // inside the message.
        let word = index
            .text
            .split(|c: char| !c.is_alphanumeric())
            .find(|w| w.chars().count() >= 4)
            .unwrap_or_else(|| panic!("{name}: no word to look for in {:?}", index.text));
        let matches = index.find(word);
        assert!(!matches.is_empty(), "{name}: {word:?} is not found");
        for found in &matches {
            for rect in index.rects(found.clone()) {
                assert!(
                    doc.messages.iter().any(|m| m.rect.union(rect) == m.rect),
                    "{name}: a match of {word:?} at {rect:?} is outside every message"
                );
            }
        }

        for cluster in &index.clusters {
            let hit = index
                .hit(cluster.rect.center())
                .unwrap_or_else(|| panic!("{name}: nothing hit at {:?}", cluster.rect));
            assert!(
                (cluster.range.start..=cluster.range.end).contains(&hit),
                "{name}: hit {hit} is outside cluster {:?}",
                cluster.range
            );
        }

        for link in &doc.links {
            match &link.target {
                LinkTarget::External(url) => assert!(
                    matches!(url.scheme(), "http" | "https" | "mailto"),
                    "{name}: {url} is not a link to follow"
                ),
                LinkTarget::Verb { .. } | LinkTarget::Fragment { .. } => {}
            }
            assert!(
                doc.messages
                    .iter()
                    .any(|m| m.rect.union(link.rect) == m.rect),
                "{name}: a link at {:?} is outside every message",
                link.rect
            );
        }
    }
    assert!(swept >= 20, "only {swept} fixtures were swept");
}
