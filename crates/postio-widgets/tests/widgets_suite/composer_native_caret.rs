use gtk::prelude::*;
use postio_body::{Block, DocPosition, Document, EditIntent, Inline, ListItem, ListKind, project};
use postio_widgets::composer::surface::ComposerSurface;

use super::composer_native_input::shown;

fn p(s: &str) -> Block {
    Block::Paragraph(vec![Inline::Text(s.into())])
}

fn draft() -> Document {
    Document {
        blocks: vec![
            Block::List {
                kind: ListKind::Bullet,
                items: vec![
                    ListItem::new(vec![p("milk")]),
                    ListItem::new(vec![p("eggs")]),
                ],
            },
            Block::Quoted(postio_body::quote_of(None, "Earlier words", "quote")),
            p("after"),
        ],
    }
}

pub fn a_caret_on_a_bullet_lands_on_the_items_text() {
    let shown = shown(draft());
    let buffer = shown.surface.view().buffer();
    buffer.place_cursor(&buffer.iter_at_offset(0));
    assert_eq!(buffer.cursor_position(), 2, "past the bullet and its space");
    assert_eq!(
        shown.surface.selection().head,
        DocPosition::new([0, 0, 0], 0)
    );
}

pub fn home_skips_the_bullet_and_end_stays_on_the_text() {
    let shown = shown(draft());
    shown.at(&[0, 1, 0], 3);
    let view = shown.surface.view();
    view.emit_move_cursor(gtk::MovementStep::DisplayLineEnds, -1, false);
    assert_eq!(
        shown.surface.selection().head,
        DocPosition::new([0, 1, 0], 0),
        "Home"
    );
    view.emit_move_cursor(gtk::MovementStep::DisplayLineEnds, 1, false);
    assert_eq!(
        shown.surface.selection().head,
        DocPosition::new([0, 1, 0], 4),
        "End"
    );
}

pub fn a_click_in_the_reply_quote_lands_after_it() {
    let shown = shown(draft());
    let buffer = shown.surface.view().buffer();
    let text = shown.text();
    let inside = text[..text.find("words").unwrap()].chars().count();
    buffer.place_cursor(&buffer.iter_at_offset(inside as i32));
    assert_eq!(shown.surface.selection().head, DocPosition::new([2], 0));
}

pub fn a_selection_keeps_its_place_across_a_patch_of_another_block() {
    let shown = shown(draft());
    let buffer = shown.surface.view().buffer();
    let projection = project(&shown.surface.document(), None);
    let held = DocPosition::new([2], 3);
    let mark = buffer.create_mark(
        None,
        &buffer.iter_at_offset(projection.offset_of(&held) as i32),
        true,
    );
    shown.at(&[0, 0, 0], 4);
    shown
        .surface
        .dispatch(EditIntent::Insert(" and oat milk".into()));
    shown.surface.dispatch(EditIntent::SplitBlock);
    let now = project(&shown.surface.document(), None);
    assert_eq!(
        now.position_of(buffer.iter_at_mark(&mark).offset() as u32),
        held
    );
    shown.assert_in_step();
}
