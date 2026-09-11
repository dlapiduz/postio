//! A pasted image, through the composer: issue #341.
//!
//! # What this reaches, and what it does not
//!
//! Three gestures put a picture in the body and they converge on
//! `add_inline_image`: a paste, a drop of an image file, and
//! `CommandId::InsertImage` from the toolbar or the palette. This drives the
//! paste from the clipboard and the chooser from a file on disk, so both
//! halves either side of that convergence are covered.
//!
//! Two bindings are not, and are small and visible rather than hidden: the
//! `ctrl+v` match arm that calls `paste_image`, and the body's `DropTarget`
//! for a `FileList`. Both need an input device or a drag this suite has no way
//! to synthesise. What can be checked without one is checked.
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
use crate::settle_until as settle;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_body::{Block, Inline};
use postio_gtk::composer;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::Attachment;
use postio_model::attachment::Disposition;
use postio_model::ids::MessageId;

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

    // ── Through the clipboard, which is where the gesture starts ─────────
    //
    // Everything above hands the bytes to `add_inline_image` directly, which
    // is the tail of a paste and not a paste: it never asks the clipboard for
    // anything, so it cannot fail when the clipboard half is broken. What
    // `ctrl+v` actually does is check the clipboard for a texture, claim the
    // keystroke if there is one, and decode it -- and that is what this
    // drives.
    let pixels = glib::Bytes::from_owned(vec![128u8; 16]);
    let texture = gdk::MemoryTexture::new(2, 2, gdk::MemoryFormat::R8g8b8a8, &pixels, 8);
    display.clipboard().set_texture(&texture);
    settle("the clipboard to hold a texture", || {
        display
            .clipboard()
            .formats()
            .contains_type(gdk::Texture::static_type())
    });

    let before = composer.test_attachment_count();
    assert!(
        composer.test_paste(),
        "a clipboard holding pixels must be claimed by the composer -- \
         letting it through writes an unresolvable URL into the DOM (#341)"
    );
    settle("the pasted texture to become a part", || {
        composer.test_attachment_count() > before
    });
    let pasted = composer
        .draft()
        .attachments
        .into_iter()
        .next_back()
        .expect("the pasted image rides the draft");
    assert_eq!(pasted.disposition, Disposition::Inline);
    assert!(
        pasted.content_id.is_some(),
        "a pasted image with no Content-ID cannot be referenced from the body"
    );

    // ── FR-049 and FR-051: chosen, not only pasted ───────────────────────
    //
    // Pasting and dropping reached this and nothing else did, so an image in
    // the body was the one of FR-049's three outcomes with no command and no
    // control — absent from the palette and the `?` sheet, and out of reach
    // for anyone who neither pastes nor drops. `CommandId::InsertImage` and
    // a toolbar button beside the link one close that; the registry tests
    // cover the command's reach, and what is asserted here is the half after
    // the chooser, since `gtk::FileDialog` does not open headlessly.
    let directory = tempfile::tempdir().expect("a directory");
    let image = directory.path().join("gauge.png");
    std::fs::write(&image, png_bytes()).expect("write the image");

    let before = composer.test_attachment_count();
    composer.test_insert_image_file(&image);
    settle("the chosen image to be inlined", || {
        composer.test_attachment_count() > before
    });

    let inlined = composer
        .draft()
        .attachments
        .into_iter()
        .next_back()
        .expect("the chosen image rides the draft");
    assert_eq!(
        inlined.disposition,
        Disposition::Inline,
        "a chosen image was attached alongside instead of placed in the body, \
         which is the distinction FR-049 exists to keep"
    );
    assert_eq!(
        inlined.mime_type, "image/png",
        "the type was guessed from the name rather than the bytes, and the \
         declaration is all the recipient's client has to go on"
    );

    // ── And a file that is not an image is refused, with somewhere to go ──
    let text = directory.path().join("notes.txt");
    std::fs::write(&text, b"not a picture").expect("write the text file");
    let count = composer.test_attachment_count();
    composer.test_insert_image_file(&text);
    settle("the refusal to be said", || !composer.status().is_empty());

    assert_eq!(
        composer.test_attachment_count(),
        count,
        "a text file was inlined as an image"
    );
    assert!(
        composer.status().contains("Attach file"),
        "the refusal does not say what to do instead, which leaves the \
         person with a file and no way to send it: {}",
        composer.status()
    );
}
