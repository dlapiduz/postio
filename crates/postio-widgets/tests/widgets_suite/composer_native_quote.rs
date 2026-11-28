use gtk::gdk;
use gtk::prelude::*;
use postio_body::{Block, DocPosition, Document, Inline};
use postio_widgets::composer::surface::ComposerSurface;

use super::composer_native_input::shown;
use super::composer_native_render::tags_at;

fn reply() -> Document {
    Document {
        blocks: vec![
            Block::Paragraph(vec![Inline::Text("Thanks".into())]),
            Block::Quoted(postio_body::quote_of(
                None,
                "first line\nsecond line\nthird line\nfourth line\nfifth line",
                "quote",
            )),
            Block::Paragraph(vec![Inline::Text("after".into())]),
        ],
    }
}

pub fn the_quote_is_read_only_and_typing_into_it_changes_nothing() {
    let shown = shown(reply());
    let buffer = shown.surface.view().buffer();
    let text = shown.text();
    let inside = text[..text.find("second").unwrap()].chars().count() as i32;
    let mut at = buffer.iter_at_offset(inside);
    assert!(
        !buffer.insert_interactive(&mut at, "x", true),
        "the view refuses it"
    );
    postio_widgets::storyboard::deliver::drain();
    assert_eq!(shown.surface.document(), reply());
    shown.assert_in_step();
}

pub fn backspace_after_the_quote_removes_it_whole_and_undo_gives_it_back() {
    let shown = shown(reply());
    shown.at(&[2], 0);
    shown.press(gdk::Key::BackSpace, gdk::ModifierType::empty());
    let mut without = reply();
    without.blocks.remove(1);
    assert_eq!(shown.surface.document(), without);
    assert_eq!(shown.surface.selection().head, DocPosition::new([1], 0));
    shown.surface.undo();
    assert_eq!(shown.surface.document(), reply());
    shown.assert_in_step();
}

pub fn the_quote_opens_folded_to_its_first_lines() {
    let shown = shown(reply());
    assert!(shown.surface.quote_folded());
    assert_eq!(tags_at(&shown.surface, "first line"), ["quoted"]);
    assert_eq!(
        tags_at(&shown.surface, "fourth"),
        ["quote-folded", "quoted"],
        "folded away"
    );
    shown.surface.set_quote_folded(false);
    assert_eq!(tags_at(&shown.surface, "fourth"), ["quoted"], "and open");
}
