use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_body::{Block, Document, ListItem, ListKind, project};

use super::storyboard_support::{show, until};

struct Spike {
    window: gtk::Window,
    view: gtk::TextView,
    im: gtk::IMContext,
    preedits: Rc<RefCell<Vec<String>>>,
    intercepted: Rc<RefCell<Vec<String>>>,
    patching: Rc<Cell<bool>>,
}

fn draft() -> Document {
    Document {
        blocks: vec![
            Block::Paragraph(vec![postio_body::Inline::Text("before".into())]),
            Block::List {
                kind: ListKind::Bullet,
                items: vec![ListItem::new(vec![Block::Paragraph(vec![
                    postio_body::Inline::Text("a".into()),
                ])])],
            },
        ],
    }
}

fn text(view: &gtk::TextView) -> String {
    let buffer = view.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string()
}

fn spike() -> Spike {
    let view = gtk::TextView::new();
    let buffer = view.buffer();
    buffer.set_enable_undo(false);
    buffer.set_text(&project(&draft(), None).text());
    let patching = Rc::new(Cell::new(false));
    let intercepted = Rc::new(RefCell::new(Vec::new()));
    {
        let (patching, intercepted) = (patching.clone(), intercepted.clone());
        buffer.connect_insert_text(move |buffer, _, typed| {
            if !patching.get() {
                intercepted.borrow_mut().push(typed.to_owned());
                buffer.stop_signal_emission_by_name("insert-text");
            }
        });
    }
    let preedits = Rc::new(RefCell::new(Vec::new()));
    {
        let preedits = preedits.clone();
        view.connect_preedit_changed(move |_, preedit| {
            preedits.borrow_mut().push(preedit.to_owned())
        });
    }
    let window = gtk::Window::new();
    window.set_child(Some(&view));
    window.set_default_size(400, 200);
    show(&window);
    view.grab_focus();
    assert!(until(|| view.is_focus()));
    buffer.place_cursor(&buffer.end_iter());
    let im = view
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|c| c.downcast::<gtk::EventControllerKey>().ok())
        .find_map(|keys| keys.im_context())
        .expect("the view's own input method");
    if let Some(multi) = im.downcast_ref::<gtk::IMMulticontext>() {
        multi.set_context_id(Some("gtk-im-context-simple"));
    }
    Spike {
        window,
        view,
        im,
        preedits,
        intercepted,
        patching,
    }
}

use gtk::glib;

impl Spike {
    fn key(&self, key: gdk::Key, state: gdk::ModifierType) -> bool {
        let display = self.view.display();
        let surface = self
            .view
            .native()
            .and_then(|n| n.surface())
            .expect("a surface");
        let device = display
            .default_seat()
            .and_then(|seat| seat.keyboard())
            .expect("a keyboard");
        let keycode = display
            .map_keyval(key)
            .and_then(|keys| keys.first().map(|k| k.keycode()))
            .unwrap_or(0);
        let taken = self
            .im
            .filter_key(true, &surface, &device, 0, keycode, state, 0);
        self.im
            .filter_key(false, &surface, &device, 0, keycode, state, 0);
        let context = glib::MainContext::default();
        while context.iteration(false) {}
        taken
    }

    fn hex(&self, digits: &str) {
        for digit in digits.chars() {
            self.key(
                gdk::Key::from_name(digit.to_string()).expect("a key"),
                gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
            );
        }
    }

    fn start_unicode(&self) -> bool {
        self.key(
            gdk::Key::u,
            gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
        )
    }

    fn patch_first_line(&self) {
        let buffer = self.view.buffer();
        self.patching.set(true);
        let mut start = buffer.start_iter();
        let mut end = buffer.iter_at_offset(6);
        buffer.delete(&mut start, &mut end);
        buffer.insert(&mut buffer.start_iter(), "BEFORE");
        self.patching.set(false);
    }
}

pub fn a_composition_stays_out_of_the_buffer_and_its_commit_is_caught() {
    let spike = spike();
    let at_rest = text(&spike.view);
    assert!(
        spike.start_unicode(),
        "the simple input method took Ctrl+Shift+U"
    );
    spike.hex("65e5");
    let preedits = spike.preedits.borrow().clone();
    assert!(
        preedits.iter().any(|p| p == "u65E5"),
        "the view drew a preedit: {preedits:?}"
    );
    assert_eq!(
        text(&spike.view),
        at_rest,
        "the preedit is not in the buffer"
    );
    assert!(
        spike.intercepted.borrow().is_empty(),
        "nothing committed yet"
    );

    spike.key(gdk::Key::space, gdk::ModifierType::empty());
    assert_eq!(
        *spike.intercepted.borrow(),
        ["日", " "],
        "the commit came through insert-text, then the space that ended it"
    );
    assert_eq!(
        text(&spike.view),
        at_rest,
        "and was stopped there for the engine"
    );
    spike.window.destroy();
}

pub fn a_patch_of_another_line_leaves_a_composition_open() {
    let spike = spike();
    assert!(spike.start_unicode());
    spike.hex("65");
    spike.patch_first_line();
    let last = spike.preedits.borrow().last().cloned().unwrap_or_default();
    assert_eq!(last, "u65", "the preedit survived the patch");
    spike.hex("e5");
    spike.key(gdk::Key::space, gdk::ModifierType::empty());
    assert_eq!(
        *spike.intercepted.borrow(),
        ["日", " "],
        "the commit after the patch is whole"
    );
    assert!(text(&spike.view).starts_with("BEFORE"));
    spike.window.destroy();
}

pub fn escape_cancels_a_composition_and_commits_nothing() {
    let spike = spike();
    let at_rest = text(&spike.view);
    assert!(spike.start_unicode());
    spike.hex("65e5");
    spike.key(gdk::Key::Escape, gdk::ModifierType::empty());
    assert_eq!(
        spike.preedits.borrow().last().map(String::as_str),
        Some(""),
        "the preedit is gone"
    );
    assert!(
        spike.intercepted.borrow().is_empty(),
        "nothing was committed"
    );
    assert_eq!(text(&spike.view), at_rest);
    spike.window.destroy();
}
