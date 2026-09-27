//! The snapshot a render hands the UI (research R7): message, link and
//! fold boxes the widget can act on without asking the engine anything.

use std::sync::Arc;

use postio_body::RemoteImages;
use postio_render::{LinkTarget, RenderRequest, RenderedDocument, Resources, Theme, Viewport};
use postio_ui::reader::document::Sheet;
use postio_ui::reader::thread::{Entry, conversation_document};

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

const BODIES: [(&str, &str); 3] = [
    (
        "7",
        "<p>See <a href=\"https://example.com/plan\">the plan</a> and \
         <a href=\"mailto:ada@example.com\">write to Ada</a>.</p>\
         <p><a href=\"javascript:alert(1)\">not a link</a></p>",
    ),
    (
        "11",
        "<p><a href=\"#budget\">Jump to the budget</a></p>\
         <h2 id=\"budget\">Budget</h2><p>Three lines.</p>",
    ),
    ("15", "<p>Thanks, both.</p>"),
];

fn snapshot() -> RenderedDocument {
    let sanitized: Vec<(&str, String)> = BODIES
        .iter()
        .map(|(scope, html)| {
            let out =
                postio_body::sanitize::sanitize_body_in(html, RemoteImages::Blocked, Some(scope));
            (*scope, out.html)
        })
        .collect();
    let entries: Vec<Entry<'_>> = sanitized
        .iter()
        .enumerate()
        .map(|(i, (scope, body))| Entry {
            scope,
            sender: ["Ada", "Grace", "Quinn"][i],
            address: "someone@example.com",
            when: "09:14",
            preview: "the first line",
            expanded: true,
            draft: false,
            mine: false,
            latest: i == 2,
            blocked: 0,
            styles: "",
            recipients: "",
            cc: "",
            body,
        })
        .collect();
    let document = conversation_document(&entries, RemoteImages::Blocked, Sheet::Theme);
    let request = RenderRequest {
        generation: 1,
        document,
        plain_text: String::new(),
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
    postio_render::render(&request, fonts())
}

#[test]
fn one_message_box_per_container_in_document_order_without_overlap() {
    let doc = snapshot();
    let scopes: Vec<&str> = doc.messages.iter().map(|m| m.scope.as_str()).collect();
    assert_eq!(scopes, ["7", "11", "15"]);
    for pair in doc.messages.windows(2) {
        assert!(
            pair[0].rect.height() > 0.0,
            "{} has no height",
            pair[0].scope
        );
        assert!(
            pair[0].rect.y1 <= pair[1].rect.y0,
            "{} overlaps {}",
            pair[0].scope,
            pair[1].scope
        );
    }
}

#[test]
fn links_are_only_external_verbs_or_fragments() {
    let doc = snapshot();
    let external: Vec<String> = doc
        .links
        .iter()
        .filter_map(|link| match &link.target {
            LinkTarget::External(url) => Some(url.to_string()),
            _ => None,
        })
        .collect();
    assert!(
        external.contains(&"https://example.com/plan".to_owned()),
        "{external:?}"
    );
    assert!(
        external.contains(&"mailto:ada@example.com".to_owned()),
        "{external:?}"
    );
    for url in &external {
        assert!(
            ["https:", "http:", "mailto:"]
                .iter()
                .any(|s| url.starts_with(s)),
            "{url} is not a link a reader may follow"
        );
    }
    assert!(
        doc.links
            .iter()
            .any(|l| matches!(&l.target, LinkTarget::Fragment { scope, .. } if scope == "11")),
        "the in-message jump is a fragment of its own message"
    );
    assert!(
        doc.links
            .iter()
            .any(|l| matches!(l.target, LinkTarget::Verb { .. })),
        "the thread's reply and forward verbs are links"
    );
    for link in &doc.links {
        assert!(link.rect.area() > 0.0, "{:?} has no box", link.target);
    }
}

#[test]
fn every_thread_fold_has_a_box() {
    let doc = snapshot();
    let ids: Vec<&str> = doc.folds.iter().map(|f| f.id.as_str()).collect();
    for scope in ["7", "11", "15"] {
        let anchor = postio_ui::reader::thread::message_anchor(scope);
        assert!(
            ids.contains(&anchor.as_str()),
            "no fold for {scope}: {ids:?}"
        );
    }
    assert!(
        doc.folds
            .iter()
            .all(|f| f.open && f.summary_rect.area() > 0.0)
    );
}

#[test]
fn one_render_and_at_most_two_style_passes() {
    let doc = snapshot();
    assert_eq!(doc.counts.renders, 1);
    assert!(doc.counts.style_passes <= 2);
}

#[test]
fn the_low_resolution_copy_is_a_quarter_scale() {
    let doc = snapshot();
    let expected = (doc.size.width * doc.scale * 0.25).ceil() as u32;
    assert_eq!(doc.low_res.width, expected);
    assert!(doc.low_res.rgba.len() <= 16 * 1024 * 1024);
}

/// A fragment link names an element the snapshot knows the place of.
#[test]
fn a_fragment_link_has_a_place_to_go() {
    let doc = snapshot();
    let id = doc
        .links
        .iter()
        .find_map(|l| match &l.target {
            LinkTarget::Fragment { id, .. } => Some(id.clone()),
            _ => None,
        })
        .expect("the jump to the budget");
    let (_, y) = doc
        .anchors
        .iter()
        .find(|(anchor, _)| *anchor == id)
        .expect("the element it names is in the snapshot's anchors");
    // The heading, not the link's own "Jump to the budget": find folds case.
    let heading = doc.text.rects(
        doc.text
            .find("Budget")
            .last()
            .cloned()
            .expect("the heading"),
    )[0];
    assert!(
        (heading.y0 - y).abs() < 30.0,
        "{id} is at {y}, its heading at {}",
        heading.y0
    );
}
