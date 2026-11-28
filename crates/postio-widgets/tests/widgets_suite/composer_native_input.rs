use gtk::gdk;
use gtk::prelude::*;
use postio_body::{Block, DocPosition, Document, Inline, ListItem, ListKind, project};
use postio_core::CommandId;
use postio_model::ids::AccountId;
use postio_model::{Draft, MessageBody};
use postio_widgets::composer::native::NativeSurface;
use postio_widgets::composer::surface::ComposerSurface;
use postio_widgets::storyboard::deliver::{Delivery, deliver, drain};

use super::storyboard_support::{show, until};
use crate::support_compose::{Window, composer};

pub(crate) struct Shown {
    pub surface: NativeSurface,
    pub window: gtk::Window,
}

impl Drop for Shown {
    fn drop(&mut self) {
        self.window.destroy();
    }
}

pub(crate) fn shown(document: Document) -> Shown {
    let surface = NativeSurface::new(std::rc::Rc::new(|_: &str| None));
    surface.load(document);
    let window = gtk::Window::new();
    window.set_child(Some(surface.view()));
    window.set_default_size(500, 300);
    show(&window);
    surface.view().grab_focus();
    assert!(until(|| surface.view().is_focus()));
    Shown { surface, window }
}

impl Shown {
    pub fn at(&self, path: &[u32], offset: u32) {
        let at = project(&self.surface.document(), None).offset_of(&DocPosition::new(path, offset));
        let buffer = self.surface.view().buffer();
        buffer.place_cursor(&buffer.iter_at_offset(at as i32));
    }

    pub fn press(&self, key: gdk::Key, state: gdk::ModifierType) -> Delivery {
        let delivery = deliver(&self.window, key, state);
        drain();
        delivery
    }

    pub fn type_text(&self, text: &str) {
        self.surface
            .view()
            .buffer()
            .insert_interactive_at_cursor(text, true);
        drain();
    }

    pub fn text(&self) -> String {
        let buffer = self.surface.view().buffer();
        buffer
            .slice(&buffer.start_iter(), &buffer.end_iter(), true)
            .to_string()
    }

    /// The buffer is always the projection of the record: anything else is a write that bypassed
    /// the patch.
    pub fn assert_in_step(&self) {
        assert_eq!(self.text(), project(&self.surface.document(), None).text());
    }
}

fn p(s: &str) -> Block {
    Block::Paragraph(if s.is_empty() {
        vec![]
    } else {
        vec![Inline::Text(s.into())]
    })
}

fn bullets(items: &[&str]) -> Block {
    Block::List {
        kind: ListKind::Bullet,
        items: items.iter().map(|s| ListItem::new(vec![p(s)])).collect(),
    }
}

const NONE: gdk::ModifierType = gdk::ModifierType::empty();

pub fn typed_text_goes_through_the_engine_not_the_buffer() {
    let shown = shown(Document {
        blocks: vec![p("ab")],
    });
    shown.at(&[0], 1);
    shown.type_text("X");
    assert_eq!(
        shown.surface.document(),
        Document {
            blocks: vec![p("aXb")]
        }
    );
    shown.assert_in_step();
    assert_eq!(
        shown.surface.selection().head,
        DocPosition::new([0], 2),
        "the caret is after it"
    );
}

pub fn enter_and_shift_enter_split_and_break() {
    let shown = shown(Document {
        blocks: vec![bullets(&["ab"])],
    });
    shown.at(&[0, 0, 0], 1);
    assert!(matches!(
        shown.press(gdk::Key::Return, NONE),
        Delivery::Delivered { .. }
    ));
    assert_eq!(
        shown.surface.document(),
        Document {
            blocks: vec![bullets(&["a", "b"])]
        }
    );
    assert_eq!(
        shown.text(),
        "\u{2022} a\n\u{2022} b",
        "a new bullet is drawn"
    );
    shown.press(gdk::Key::Return, gdk::ModifierType::SHIFT_MASK);
    assert_eq!(
        shown.surface.document().blocks[0],
        Block::List {
            kind: ListKind::Bullet,
            items: vec![
                ListItem::new(vec![p("a")]),
                ListItem::new(vec![Block::Paragraph(vec![
                    Inline::Break,
                    Inline::Text("b".into())
                ])]),
            ],
        }
    );
    shown.assert_in_step();
}

pub fn backspace_and_delete_take_characters_words_and_join_blocks() {
    let shown = shown(Document {
        blocks: vec![p("one two"), p("three")],
    });
    shown.at(&[1], 0);
    shown.press(gdk::Key::BackSpace, NONE);
    assert_eq!(
        shown.surface.document(),
        Document {
            blocks: vec![p("one twothree")]
        }
    );
    shown.press(gdk::Key::BackSpace, gdk::ModifierType::CONTROL_MASK);
    assert_eq!(
        shown.surface.document(),
        Document {
            blocks: vec![p("one three")]
        }
    );
    shown.press(gdk::Key::Delete, NONE);
    assert_eq!(
        shown.surface.document(),
        Document {
            blocks: vec![p("one hree")]
        }
    );
    shown.assert_in_step();
}

pub fn tab_nests_a_list_item_and_elsewhere_moves_on() {
    let shown = shown(Document {
        blocks: vec![bullets(&["a", "b"]), p("after")],
    });
    shown.at(&[0, 1, 0], 1);
    assert!(matches!(
        shown.press(gdk::Key::Tab, NONE),
        Delivery::Delivered { .. }
    ));
    assert_eq!(
        shown.text(),
        "\u{2022} a\n\u{25e6} b\nafter",
        "the item is nested, its bullet one deeper"
    );
    shown.press(gdk::Key::ISO_Left_Tab, gdk::ModifierType::SHIFT_MASK);
    assert_eq!(shown.surface.document().blocks[0], bullets(&["a", "b"]));
    shown.at(&[1], 0);
    let before = shown.surface.document();
    shown.press(gdk::Key::Tab, NONE);
    assert_eq!(
        shown.surface.document(),
        before,
        "Tab outside a list changes nothing"
    );
    shown.assert_in_step();
}

pub fn a_typed_sequence_converts_in_place() {
    let shown = shown(Document {
        blocks: vec![p("")],
    });
    shown.type_text("-");
    shown.type_text(" ");
    assert_eq!(
        shown.surface.document(),
        Document {
            blocks: vec![bullets(&[""])]
        }
    );
    shown.type_text("milk");
    assert_eq!(shown.text(), "\u{2022} milk");
    shown.assert_in_step();
}

pub fn a_composer_command_reaches_its_intent() {
    let window = Window::default();
    window.present();
    let composer = composer::install_native(&window);
    let mut draft = Draft::new(AccountId::new(1));
    draft.body = MessageBody {
        text: Some("hello".into()),
        html: None,
    };
    composer.open(draft);
    drain();
    assert!(composer.is_open());
    let view = composer
        .test_body_widget()
        .downcast::<gtk::TextView>()
        .unwrap();
    let buffer = view.buffer();
    buffer.select_range(&buffer.iter_at_offset(0), &buffer.iter_at_offset(5));
    composer.dispatch(CommandId::Bold);
    assert_eq!(
        composer.document(),
        Document {
            blocks: vec![Block::Paragraph(vec![Inline::Strong(vec![Inline::Text(
                "hello".into()
            )])])]
        }
    );
}
