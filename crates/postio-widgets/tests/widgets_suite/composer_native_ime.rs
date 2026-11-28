use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use postio_body::{Block, Document, Inline, ListItem, ListKind};
use postio_widgets::composer::surface::{ComposerSurface, Format};

use super::composer_native_input::{Shown, shown};

fn list(text: &str) -> Document {
    Document {
        blocks: vec![
            Block::Paragraph(vec![Inline::Text("before".into())]),
            Block::List {
                kind: ListKind::Bullet,
                items: vec![ListItem::new(vec![Block::Paragraph(vec![Inline::Text(
                    text.into(),
                )])])],
            },
        ],
    }
}

/// The view's own input method, fed key events as a compositor would; with GTK's simple context,
/// Ctrl+Shift+U then hex is a real preedit with no input-method daemon.
struct Ime {
    im: gtk::IMContext,
}

impl Ime {
    fn of(shown: &Shown) -> Self {
        let im = shown
            .surface
            .view()
            .observe_controllers()
            .iter::<glib::Object>()
            .filter_map(Result::ok)
            .filter_map(|c| c.downcast::<gtk::EventControllerKey>().ok())
            .find_map(|keys| keys.im_context())
            .expect("the view's own input method");
        Ime { im }
    }

    fn key(&self, shown: &Shown, key: gdk::Key, state: gdk::ModifierType) {
        let view = shown.surface.view();
        let display = view.display();
        let surface = view.native().and_then(|n| n.surface()).expect("a surface");
        let device = display
            .default_seat()
            .and_then(|s| s.keyboard())
            .expect("a keyboard");
        let keycode = display
            .map_keyval(key)
            .and_then(|keys| keys.first().map(|k| k.keycode()))
            .unwrap_or(0);
        self.im
            .filter_key(true, &surface, &device, 0, keycode, state, 0);
        self.im
            .filter_key(false, &surface, &device, 0, keycode, state, 0);
        postio_widgets::storyboard::deliver::drain();
    }

    fn compose(&self, shown: &Shown, hex: &str) {
        let held = gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK;
        self.key(shown, gdk::Key::u, held);
        for digit in hex.chars() {
            self.key(shown, gdk::Key::from_name(digit.to_string()).unwrap(), held);
        }
    }
}

/// Chosen before any view exists: switching a live view's context left the Wayland one's callbacks
/// pointing at a destroyed widget, which crashed the process at teardown under load.
fn simple_input_method() {
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_im_module(Some("gtk-im-context-simple"));
    }
}

pub fn a_cjk_composition_in_a_list_item_is_one_commit_and_one_undo() {
    simple_input_method();
    let shown = shown(list("a"));
    shown.at(&[1, 0, 0], 1);
    let at_rest = shown.text();
    let ime = Ime::of(&shown);

    ime.compose(&shown, "65e5");
    assert_eq!(
        shown.text(),
        at_rest,
        "the composition is drawn, not in the buffer"
    );
    assert_eq!(shown.surface.document(), list("a"), "nor in the document");
    shown.surface.format(Format::Bold);
    assert_eq!(
        shown.surface.document(),
        list("a"),
        "a command waits for the commit"
    );

    ime.key(&shown, gdk::Key::Return, gdk::ModifierType::empty());
    assert_eq!(
        shown.surface.document(),
        list("a日"),
        "the commit is in the item"
    );
    assert_eq!(shown.text(), "before\n\u{2022} a日");
    assert_eq!(
        shown.surface.selection().head.offset,
        2,
        "the caret after it"
    );
    shown.assert_in_step();

    shown.surface.undo();
    assert_eq!(
        shown.surface.document(),
        list("a"),
        "one undo takes the commit"
    );
    shown.assert_in_step();
}

pub fn escape_cancels_a_composition_and_the_document_is_untouched() {
    simple_input_method();
    let shown = shown(list("a"));
    shown.at(&[1, 0, 0], 1);
    let ime = Ime::of(&shown);
    ime.compose(&shown, "65e5");
    ime.key(&shown, gdk::Key::Escape, gdk::ModifierType::empty());
    assert_eq!(shown.surface.document(), list("a"));
    assert!(!shown.surface.can_undo(), "nothing was recorded");
    shown.type_text("b");
    assert_eq!(
        shown.surface.document(),
        list("ab"),
        "and typing goes on as usual"
    );
}
