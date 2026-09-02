//! A pasted image, through the composer: issue #341.
//!
//! The paste seam hands the bytes out (the blob store lives above this
//! crate), gets back an attachment with a `Content-ID`, and the composer
//! records it on the draft, shows it in the attachment list, and puts the
//! image at the caret — where it renders through the same `postio-cid:`
//! lookup a resumed draft uses.
//!
//! Its own file: GTK is single-threaded and initialised once, so one
//! `#[test]` per integration binary. See `gtk_composer.rs`.

use crate::settle as pump;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::prelude::*;
use postio_body::{Block, Inline};
use postio_gtk::composer;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::Attachment;
use postio_model::attachment::Disposition;
use postio_model::ids::MessageId;

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

/// A real PNG, the way the paste path makes one.
fn png_bytes() -> Vec<u8> {
    let pixels = glib::Bytes::from_owned(vec![255u8; 16]);
    let texture = gdk::MemoryTexture::new(2, 2, gdk::MemoryFormat::R8g8b8a8, &pixels, 8);
    texture.save_to_png_bytes().to_vec()
}

pub fn a_pasted_image_becomes_an_inline_attachment_and_renders_at_the_caret() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under the headless runner to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let window = Window::default();
    window.present();
    pump();

    let composer = composer::install(&window);

    // The store the app would provide: bytes by content id, in memory.
    let stored: Rc<RefCell<HashMap<String, Vec<u8>>>> = Rc::new(RefCell::new(HashMap::new()));
    composer.connect_inline_image({
        let stored = stored.clone();
        move |bytes, _mime, then| {
            let content_id = format!("pasted-{}@postio.invalid", stored.borrow().len() + 1);
            stored
                .borrow_mut()
                .insert(content_id.clone(), bytes.clone());
            let mut attachment =
                Attachment::new(MessageId::UNASSIGNED, "image/png", bytes.len() as u64);
            attachment.filename = Some("pasted-image.png".to_owned());
            attachment.disposition = Disposition::Inline;
            attachment.content_id = Some(content_id);
            then(Some(attachment));
        }
    });
    composer.connect_attachment_bytes({
        let stored = stored.clone();
        move |attachment: &Attachment| {
            attachment
                .content_id
                .as_deref()
                .and_then(|id| stored.borrow().get(id).cloned())
        }
    });

    window.handle_key(
        gdk::Key::from_name("c").unwrap(),
        gdk::ModifierType::empty(),
    );
    pump();
    assert!(composer.is_open());

    // Type first, as a hand would have: the paste lands at the caret the
    // typing left behind.
    composer.test_set_body("see ");
    composer.test_paste_image_bytes(png_bytes());

    // The record gains the image…
    settle("the pasted image to reach the document", || {
        composer.document().blocks.iter().any(|block| {
            matches!(block, Block::Paragraph(inlines) if inlines.iter().any(|inline| matches!(
                inline,
                Inline::Image { content_id, .. }
                    if content_id.as_str() == "pasted-1@postio.invalid"
            )))
        })
    });
    // …the draft gains the inline attachment…
    let draft = composer.draft();
    let attachment = draft
        .attachments
        .iter()
        .find(|attachment| attachment.content_id.as_deref() == Some("pasted-1@postio.invalid"))
        .expect("the attachment rides the draft");
    assert_eq!(attachment.disposition, Disposition::Inline);
    // …and pixels actually arrive, through the draft-aware cid lookup.
    settle("the pasted image to render from the store", || {
        composer.test_body_eval(
            "String(document.images.length === 1 && document.images[0].naturalWidth > 0)",
        ) == "true"
    });

    // The wire form of the draft carries the cid reference, which is what
    // outgoing.rs's multipart emission inlines.
    let html = draft.body.html.expect("an image is structure");
    assert!(
        html.contains("src=\"cid:pasted-1%40postio.invalid\"")
            || html.contains("src=\"cid:pasted-1@postio.invalid\""),
        "{html}"
    );
}
