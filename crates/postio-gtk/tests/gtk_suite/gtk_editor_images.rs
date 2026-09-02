//! Inline images in the editing surface: issue #341.
//!
//! Three properties, end to end on a real display:
//!
//! 1. A document's [`Inline::Image`] renders through the `postio-cid:`
//!    scheme from the blob source — pixels actually arrive.
//! 2. [`Editor::insert_image`] lands at the caret and crosses the bridge
//!    into the record as an [`Inline::Image`].
//! 3. A pasted remote image — the shape a copy from a browser produces —
//!    is never fetched (the shell's CSP starves it) and never recorded
//!    (`parse` has no representation for it): inline-or-drop, ADR 0003 Q4.
//!
//! One test function: GTK is single-threaded and initialised once.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_body::{Block, ContentId, Document, Inline};
use postio_gtk::editor::Editor;
use webkit6::prelude::*;

fn settle(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(done(), "timed out waiting for {what}");
}

fn eval_str(view: &webkit6::WebView, script: &str) -> String {
    let result: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let slot = result.clone();
    view.evaluate_javascript(
        script,
        None,
        None,
        None::<&gtk::gio::Cancellable>,
        move |outcome| {
            let value = outcome
                .map(|value| value.to_str().to_string())
                .unwrap_or_default();
            *slot.borrow_mut() = Some(value);
        },
    );
    settle("the script to answer", || result.borrow().is_some());
    result.borrow_mut().take().unwrap_or_default()
}

/// A real PNG, made the same way a paste handler will make one.
fn png_bytes() -> Vec<u8> {
    let pixels = glib::Bytes::from_owned(vec![255u8; 16]);
    let texture = gdk::MemoryTexture::new(2, 2, gdk::MemoryFormat::R8g8b8a8, &pixels, 8);
    texture.save_to_png_bytes().to_vec()
}

fn image_paragraph(id: &str, alt: &str) -> Document {
    let mut document = Document::new();
    document.blocks.push(Block::Paragraph(vec![
        Inline::Text("see ".to_owned()),
        Inline::Image {
            content_id: ContentId::parse(id).unwrap(),
            alt: alt.to_owned(),
        },
    ]));
    document
}

pub fn inline_images_render_from_the_blob_store_and_remote_ones_never_load() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under the headless runner to exercise this)");
        return;
    }

    let png = png_bytes();
    let source = Rc::new(move |content_id: &str| {
        (content_id == "photo-1@postio.invalid" || content_id == "photo-2@postio.invalid")
            .then(|| (png.clone(), "image/png".to_owned()))
    });

    let window = gtk::Window::new();
    window.set_default_size(600, 400);
    let editor = Editor::with_coalesce(source, Duration::ZERO);
    window.set_child(Some(editor.widget()));
    window.present();

    // ── the document's image arrives as pixels through postio-cid: ───────
    editor.load(image_paragraph("photo-1@postio.invalid", "the lamp"));
    settle("the inline image to resolve from the blob source", || {
        eval_str(
            editor.widget(),
            "String(document.images.length === 1 && document.images[0].naturalWidth > 0)",
        ) == "true"
    });

    // ── an inserted image lands at the caret and in the record ───────────
    editor.place_caret_start();
    editor.insert_image(
        &ContentId::parse("photo-2@postio.invalid").unwrap(),
        "pasted",
    );
    settle("the inserted image to cross the bridge", || {
        editor.document().blocks.iter().any(|block| {
            matches!(block, Block::Paragraph(inlines) if inlines.iter().any(|inline| matches!(
                inline,
                Inline::Image { content_id, .. }
                    if content_id.as_str() == "photo-2@postio.invalid"
            )))
        })
    });
    settle("both images to render", || {
        eval_str(
            editor.widget(),
            "String([...document.images].filter(i => i.naturalWidth > 0).length)",
        ) == "2"
    });

    // ── a pasted remote image is starved and dropped, never fetched ──────
    // The shape WebKit's native paste writes when someone copies from a
    // browser. Inserted with the editor's own machinery, so it is exactly a
    // paste as the DOM sees one.
    let before = editor.document();
    eval_str(
        editor.widget(),
        "(() => { document.execCommand('insertHTML', false, \
           '<img src=\"https://pixel.tracker.example.org/o.gif\">'); \
           document.dispatchEvent(new Event('input')); return 'done'; })()",
    );
    // The record never gains it: parse has no representation for a remote
    // reference, so the document is unchanged — that is "drop".
    settle("the bridge to report the paste", || {
        eval_str(editor.widget(), "String(document.images.length)") == "3"
    });
    assert_eq!(
        editor.document(),
        before,
        "a remote image reached the record"
    );
    // And "never fetch": the element sits in the working copy, starved by
    // the shell's CSP — no bytes, no dimensions, no request.
    settle("the remote image to be starved by the CSP", || {
        eval_str(
            editor.widget(),
            "(() => { const i = [...document.images].find(i => i.src.startsWith('https')); \
               return i ? String(i.naturalWidth) : 'gone'; })()",
        ) == "0"
    });
}
