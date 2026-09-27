//! A composer on a host that is not the classic window (specs/007-postio-focus
//! T023): it takes the host's pane, autosaves once an edit has been quiet
//! for the debounce, and sends through the same seams -- which is what lets
//! Focus's compose dialog be this composer rather than a second one.
//!
//! The host is this test's own: a plain window with one box for a pane. What
//! is asserted is what the composer does -- the drafts its seams hand over,
//! and whether it holds the pane -- never what the host was told.
//!
//! Skips without a display. Nothing here touches the network.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::prelude::*;
use postio_core::{CommandId, Keymap};
use postio_gtk::composer::{Composer, ComposerHost};
use postio_gtk::{fonts, style};
use postio_model::{AccountId, Draft};

/// What a host dispatches its commands to.
type CommandHandlers = RefCell<Vec<Box<dyn Fn(CommandId)>>>;

/// A host with nothing classic in it: a window, a box, and a keymap.
struct TestHost {
    window: gtk::Window,
    pane: gtk::Box,
    /// Whether the composer holds the pane now.
    holding: Cell<bool>,
    commands: CommandHandlers,
}

impl ComposerHost for TestHost {
    fn parent(&self) -> Option<gtk::Window> {
        Some(self.window.clone())
    }

    fn install(&self, composer: &Composer) {
        composer.set_visible(false);
        self.pane.append(composer);
    }

    fn restore(&self, composer: &Composer) {
        self.pane.append(composer);
    }

    fn remove(&self, composer: &Composer) {
        self.pane.remove(composer);
    }

    fn take_pane(&self) {
        self.holding.set(true);
    }

    fn release_pane(&self) {
        self.holding.set(false);
    }

    fn command_for_key(
        &self,
        _key: gdk::Key,
        _state: gdk::ModifierType,
        _window: &gtk::Window,
    ) -> Option<CommandId> {
        None
    }

    fn handle_key(
        &self,
        _key: gdk::Key,
        _state: gdk::ModifierType,
        _window: &gtk::Window,
    ) -> glib::Propagation {
        glib::Propagation::Proceed
    }

    fn add_action(&self, _action: &gtk::gio::SimpleAction) {}

    fn connect_command(&self, handler: Box<dyn Fn(CommandId)>) {
        self.commands.borrow_mut().push(handler);
    }

    fn keymap(&self) -> Keymap {
        Keymap::defaults().clone()
    }

    fn composing(&self, _open: bool, _keymap: &Keymap) {}
}

/// Turn the main loop, timers included, until `done` or `within` passes.
fn until(within: Duration, done: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + postio_test_support::scaled(within);
    while !done() && Instant::now() < deadline {
        glib::MainContext::default().iteration(true);
    }
    done()
}

pub fn a_composer_on_a_test_host_autosaves_and_sends() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    let _ = fonts::install();
    style::install(&display);

    let window = gtk::Window::new();
    window.set_default_size(900, 700);
    let pane = gtk::Box::new(gtk::Orientation::Vertical, 0);
    window.set_child(Some(&pane));
    window.present();
    let host = Rc::new(TestHost {
        window: window.clone(),
        pane,
        holding: Cell::new(false),
        commands: RefCell::default(),
    });

    let composer = Composer::new();
    composer.mount_on(host.clone());
    let saves: Rc<RefCell<Vec<Draft>>> = Rc::default();
    composer.connect_save({
        let saves = saves.clone();
        move |draft| saves.borrow_mut().push(draft.clone())
    });
    let sent: Rc<RefCell<Vec<Draft>>> = Rc::default();
    composer.connect_send({
        let sent = sent.clone();
        move |draft| sent.borrow_mut().push(draft.clone())
    });

    composer.open(Draft::new(AccountId::new(1)));
    assert!(
        until(Duration::from_millis(200), || host.holding.get()),
        "opening did not take the host's pane"
    );

    // An edit, then quiet: exactly one autosave, after the debounce.
    composer.test_set_to("Grace Hollins <grace@example.org>");
    composer.test_set_subject("Survey dates");
    assert!(
        until(Duration::from_secs(4), || !saves.borrow().is_empty()),
        "an edit on a test host was never autosaved"
    );
    assert_eq!(
        saves
            .borrow()
            .last()
            .map(|draft| draft.subject.clone())
            .as_deref(),
        Some("Survey dates"),
        "the autosave did not carry the edit"
    );

    // And the host's `Send`, dispatched to it, sends it.
    for handler in host.commands.borrow().iter() {
        handler(CommandId::Send);
    }
    assert!(
        until(Duration::from_millis(500), || !sent.borrow().is_empty()),
        "the host's Send never reached the composer's send seam"
    );
    assert_eq!(sent.borrow()[0].subject, "Survey dates");
    assert!(
        !host.holding.get(),
        "a sent composer did not give the host's pane back"
    );
    window.destroy();
}
