//! What the cases share: a corpus message as the reader composes it, and
//! a main loop turned until something is on screen.

use std::sync::Arc;

use gtk::glib;
use postio_body::RemoteImages;
use postio_model::test_corpus;
use postio_render::Resources;
use postio_ui::reader::document::{self, Rendering};
use postio_widgets::body_view::Content;

/// The render deadline for every case that is not about the deadline: the
/// production bound, scaled by `POSTIO_TEST_PATIENCE`, so a debug build on a
/// busy runner does not fall back by accident (spec 006 research R6).
pub fn reader_deadline() -> std::time::Duration {
    postio_test_support::scaled(postio_widgets::body_view::DEFAULT_RENDER_DEADLINE)
}

/// The corpus message `name` as the single-message reader composes it, with
/// its parts and faces.
pub fn content(name: &str) -> Content {
    let parsed = postio_model::mime::parse(test_corpus::load(name).bytes());
    let resources = Resources::new();
    for part in &parsed.parts {
        if let Some(cid) = &part.attachment.content_id {
            resources.insert_part(None, cid, part.content.clone());
        }
    }
    for face in document::FACES {
        resources.insert_font(face.name, face.bytes);
    }
    let body = document::body_html_in(
        &parsed.body,
        RemoteImages::Blocked,
        Rendering::Original,
        None,
    );
    Content {
        document: document::document_for(
            &body.html,
            &body.styles,
            RemoteImages::Blocked,
            document::sheet_for(Rendering::Original, false),
        ),
        resources: Arc::new(resources),
        plain_text: parsed.body.text.unwrap_or_default(),
        over_cap: body.over_cap,
    }
}

/// A conversation of one message, `scope`, whose body is `body`: composed by
/// the reader's own thread document, with its quoted text folded.
pub fn conversation(scope: &str, body: &str) -> Content {
    use postio_ui::reader::thread::{Entry, conversation_document};
    let sanitized =
        postio_body::sanitize::sanitize_body_in(body, RemoteImages::Blocked, Some(scope));
    let folded = postio_body::fold_html_quotes(&sanitized.html);
    let entry = Entry {
        scope,
        sender: "Ada",
        address: "ada@example.com",
        when: "09:14",
        preview: "the first line",
        expanded: true,
        draft: false,
        mine: false,
        latest: false,
        blocked: 0,
        styles: "",
        recipients: "",
        cc: "",
        body: &folded,
    };
    let mut content = content("plain-text-simple");
    content.document =
        conversation_document(&[entry], RemoteImages::Blocked, document::Sheet::Theme);
    content
}

/// One turn of the main loop, blocking until something happens. The
/// heartbeat bounds the block: the frame clock paints on the compositor's
/// frame callbacks, which only a blocking iteration waits for.
fn pump() {
    let heartbeat = glib::timeout_add_local(std::time::Duration::from_millis(10), || {
        glib::ControlFlow::Continue
    });
    glib::MainContext::default().iteration(true);
    heartbeat.remove();
}

/// Run the main loop until `done`, painting frames; false on timeout.
pub fn until(done: impl Fn() -> bool) -> bool {
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    while !done() && std::time::Instant::now() < deadline {
        pump();
    }
    done()
}
