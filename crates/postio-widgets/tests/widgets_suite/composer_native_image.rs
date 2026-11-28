use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_body::{Block, ContentId, DocPosition, Document, EditIntent, Inline};
use postio_widgets::composer::native::NativeSurface;
use postio_widgets::composer::surface::ComposerSurface;

use super::storyboard_support::{show, until};

fn png() -> Vec<u8> {
    let pixels = glib::Bytes::from_owned(vec![255u8; 16]);
    let texture = gdk::MemoryTexture::new(2, 2, gdk::MemoryFormat::R8g8b8a8, &pixels, 8);
    texture.save_to_png_bytes().to_vec()
}

fn cat() -> ContentId {
    ContentId::parse("cat@example.com").unwrap()
}

fn with_image() -> Document {
    Document {
        blocks: vec![Block::Paragraph(vec![
            Inline::Text("a".into()),
            Inline::Image {
                content_id: cat(),
                alt: "a cat".into(),
            },
            Inline::Text("b".into()),
        ])],
    }
}

fn surface() -> (NativeSurface, gtk::Window) {
    let surface = NativeSurface::new(std::rc::Rc::new(|id: &str| {
        (id == "cat@example.com").then(|| (png(), "image/png".to_owned()))
    }));
    let window = gtk::Window::new();
    window.set_child(Some(surface.view()));
    show(&window);
    (surface, window)
}

fn picture_at(surface: &NativeSurface, offset: i32) -> Option<gtk::Picture> {
    let anchor = surface
        .view()
        .buffer()
        .iter_at_offset(offset)
        .child_anchor()?;
    anchor
        .widgets()
        .first()?
        .clone()
        .downcast::<gtk::Picture>()
        .ok()
}

pub fn an_inline_image_is_a_picture_of_its_blob_at_its_place() {
    let (surface, window) = surface();
    surface.load(with_image());
    let picture = picture_at(&surface, 1).expect("an anchored picture where the image is");
    let texture = picture.paintable().expect("drawn from the blob's bytes");
    assert_eq!(
        (texture.intrinsic_width(), texture.intrinsic_height()),
        (2, 2)
    );
    assert_eq!(picture.alternative_text().as_deref(), Some("a cat"));
    window.destroy();
}

pub fn an_inserted_image_is_anchored_and_one_backspace_takes_it_whole() {
    let (surface, window) = surface();
    surface.load(Document {
        blocks: vec![Block::Paragraph(vec![Inline::Text("ab".into())])],
    });
    let buffer = surface.view().buffer();
    buffer.place_cursor(&buffer.iter_at_offset(1));
    surface.insert_image(&cat(), "a cat");
    assert_eq!(surface.document(), with_image());
    assert!(picture_at(&surface, 1).is_some(), "anchored at the caret");
    assert_eq!(
        surface.selection().head,
        DocPosition::new([0], 2),
        "the caret after it"
    );
    assert!(surface.dispatch(EditIntent::DeleteBackward(postio_body::Unit::Char)));
    assert_eq!(
        surface.document(),
        Document {
            blocks: vec![Block::Paragraph(vec![Inline::Text("ab".into())])]
        }
    );
    assert!(until(|| picture_at(&surface, 1).is_none()));
    surface.undo();
    assert!(picture_at(&surface, 1).is_some(), "and undo puts it back");
    window.destroy();
}
