use std::time::Duration;

use gtk::prelude::*;
use postio_body::{Block, DocPosition, Document, Inline, ListItem, ListKind};
use postio_widgets::composer::native::NativeSurface;
use postio_widgets::composer::surface::ComposerSurface;
use postio_widgets::storyboard::deliver::drain;

fn surface(coalesce: Duration, text: &str) -> NativeSurface {
    let surface = NativeSurface::with_coalesce(std::rc::Rc::new(|_: &str| None), coalesce);
    surface.load(Document::from_text(text));
    let buffer = surface.view().buffer();
    buffer.place_cursor(&buffer.end_iter());
    surface
}

fn typed(surface: &NativeSurface, text: &str) {
    for c in text.chars() {
        surface
            .view()
            .buffer()
            .insert_interactive_at_cursor(&c.to_string(), true);
        drain();
    }
}

pub fn a_typing_run_is_one_step_and_undo_puts_the_caret_back() {
    let surface = surface(Duration::MAX, "hi ");
    typed(&surface, "there");
    assert_eq!(surface.document(), Document::from_text("hi there"));
    surface.undo();
    assert_eq!(
        surface.document(),
        Document::from_text("hi "),
        "the whole run"
    );
    assert_eq!(
        surface.selection().head,
        DocPosition::new([0], 3),
        "the caret where it began"
    );
    surface.redo();
    assert_eq!(surface.document(), Document::from_text("hi there"));
    assert_eq!(surface.selection().head, DocPosition::new([0], 8));
}

pub fn past_the_runs_window_every_keystroke_is_a_step() {
    let surface = surface(Duration::ZERO, "");
    typed(&surface, "ab");
    surface.undo();
    assert_eq!(surface.document(), Document::from_text("a"));
}

pub fn a_conversion_is_its_own_step_after_the_run_that_typed_it() {
    let surface = surface(Duration::MAX, "");
    typed(&surface, "- ");
    let bullet = Document {
        blocks: vec![Block::List {
            kind: ListKind::Bullet,
            items: vec![ListItem::new(vec![Block::Paragraph(Vec::new())])],
        }],
    };
    assert_eq!(surface.document(), bullet);
    typed(&surface, "milk");
    surface.undo();
    assert_eq!(surface.document(), bullet, "the typing after it first");
    surface.undo();
    assert_eq!(
        surface.document(),
        Document {
            blocks: vec![Block::Paragraph(vec![Inline::Text("- ".into())])]
        },
        "then the conversion, back to what was typed"
    );
    surface.undo();
    assert_eq!(surface.document(), Document::from_text(""));
}
