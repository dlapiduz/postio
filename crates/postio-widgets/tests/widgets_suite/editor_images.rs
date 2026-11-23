use crate::support_compose::settle_until as settle;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_body::{Block, ContentId, Document, Inline};
use postio_widgets::composer::editor::Editor;
use webkit6::prelude::*;

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

    editor.load(image_paragraph("photo-1@postio.invalid", "the lamp"));
    let what = "the inline image to resolve from the blob source";
    settle(what, || {
        eval_str(editor.widget(), "String(document.images.length)") == "1"
            && crate::support_compose::cid_image_decoded(
                &eval_str(
                    editor.widget(),
                    &crate::support_compose::image_state_js("document.images[0]"),
                ),
                what,
            )
    });

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

    let before = editor.document();
    eval_str(
        editor.widget(),
        "(() => { document.execCommand('insertHTML', false, \
           '<img src=\"https://pixel.tracker.example.org/o.gif\">'); \
           document.dispatchEvent(new Event('input')); return 'done'; })()",
    );
    settle("the bridge to report the paste", || {
        eval_str(editor.widget(), "String(document.images.length)") == "3"
    });
    assert_eq!(
        editor.document(),
        before,
        "a remote image reached the record"
    );
    settle("the remote image to be starved by the CSP", || {
        eval_str(
            editor.widget(),
            "(() => { const i = [...document.images].find(i => i.src.startsWith('https')); \
               return i ? String(i.naturalWidth) : 'gone'; })()",
        ) == "0"
    });
}
