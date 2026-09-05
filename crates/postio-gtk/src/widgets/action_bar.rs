//! A row of [`KeycapButton`]s, identical on every screen.
//!
//! The canvas's rule (turn 7) is that an action bar looks the same wherever
//! it appears: real buttons with keycap hints, one primary, no bare
//! monospace text links. The reading pane had one, built by hand; the
//! conversation pane's per-message buttons were three bare `gtk::Button`s
//! with no caps at all, and its footer had none. Three surfaces, one shape.
//!
//! # One path to the composer and the queue
//!
//! Every button here runs [`Command::default_for`] on the same [`CommandId`]
//! the keybinding does, and hands it out through [`ActionBar::connect_command`]
//! for whoever mounts the bar to act on. There is no second "reply from a
//! button" implementation to keep in step with the real one.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use postio_core::{Command, CommandId, Keymap};

use super::keycap::KeycapButton;

type CommandHandler = Box<dyn Fn(Command)>;

/// One button in a bar: the command it runs, its label, the CSS class a test
/// finds it by, and whether it gets the primary treatment.
#[derive(Clone, Copy, Debug)]
pub struct Action {
    pub command: CommandId,
    pub label: &'static str,
    pub class: &'static str,
    pub primary: bool,
}

impl Action {
    /// A secondary button — the common case.
    pub const fn new(command: CommandId, label: &'static str, class: &'static str) -> Self {
        Self {
            command,
            label,
            class,
            primary: false,
        }
    }

    /// The one verb the bar exists for.
    pub const fn primary(mut self) -> Self {
        self.primary = true;
        self
    }
}

/// The bar itself.
pub struct ActionBar {
    root: gtk::Box,
    buttons: Vec<Rc<KeycapButton>>,
    commands: RefCell<Vec<CommandHandler>>,
}

impl ActionBar {
    /// Build a bar over `actions`, in the order given.
    ///
    /// Returned as `Rc` because a button's own `clicked` handler needs to
    /// call back into `self` — the same reason `composer::Completion::install`
    /// does, and weakly for the same reason.
    pub fn new(actions: &[Action], class: &str) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        root.add_css_class("postio-action-bar");
        root.add_css_class(class);

        let buttons: Vec<Rc<KeycapButton>> = actions
            .iter()
            .map(|action| {
                let button = Rc::new(KeycapButton::new(
                    Some(action.command),
                    action.label,
                    action.class,
                    action.primary,
                ));
                KeycapButton::arm(&button);
                root.append(&button.widget());
                button
            })
            .collect();

        let this = Rc::new(Self {
            root,
            buttons,
            commands: RefCell::new(Vec::new()),
        });

        for button in &this.buttons {
            let Some(id) = button.command() else { continue };
            let weak = Rc::downgrade(&this);
            button.connect_clicked(move || {
                if let Some(bar) = weak.upgrade() {
                    bar.run(id);
                }
            });
        }

        // A sensible key hint from the moment the bar exists, the same
        // defensive default the list row keeps for its own hints:
        // `Window::apply_keymap` is what keeps this current once a real
        // config loads, but nothing here should depend on that call having
        // already happened by the time anyone looks.
        this.set_keymap(&Keymap::resolve(&Default::default()));
        this
    }

    /// Push everything after this point to the right-hand end.
    ///
    /// What separates a bar's verbs from the status a surface puts beside
    /// them — the conversation footer's hint line, the reader's thread
    /// position.
    pub fn append_trailing(&self, widget: &impl IsA<gtk::Widget>) {
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        self.root.append(&spacer);
        self.root.append(widget);
    }

    /// The widget to place in a pane.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// Show or hide the whole bar.
    pub fn set_visible(&self, visible: bool) {
        self.root.set_visible(visible);
    }

    /// Whether the bar is on screen.
    pub fn is_visible(&self) -> bool {
        self.root.is_visible()
    }

    /// Re-cap every button from the live keymap.
    pub fn set_keymap(&self, keymap: &Keymap) {
        for button in &self.buttons {
            let Some(id) = button.command() else { continue };
            button.set_key(keymap.binding(id));
        }
    }

    /// One button, by the command it runs.
    pub fn button(&self, command: CommandId) -> Option<&Rc<KeycapButton>> {
        self.buttons
            .iter()
            .find(|button| button.command() == Some(command))
    }

    /// Called with the invocation whenever a button is pressed — the same
    /// [`Command`] the keyboard's binding for the same verb would produce.
    pub fn connect_command(&self, handler: impl Fn(Command) + 'static) {
        self.commands.borrow_mut().push(Box::new(handler));
    }

    /// Press a button without a pointer, for a test.
    pub fn press(&self, command: CommandId) {
        if let Some(button) = self.button(command) {
            button.press();
        }
    }

    /// Run `command` against whatever the surface is showing.
    fn run(&self, command: CommandId) {
        let command = Command::default_for(command);
        for handler in self.commands.borrow().iter() {
            handler(command.clone());
        }
    }
}

/// The keys `actions` currently carry — `None` where the user cleared a
/// binding entirely, or lost it to a collision.
///
/// A free function, decoupled from the widgets [`ActionBar::set_keymap`]
/// updates from it, so a rebind reaching a bar is testable without a display
/// — the same split `crate::row`'s `hints_for` makes for the focused row.
pub fn keys(actions: &[Action], keymap: &Keymap) -> Vec<(CommandId, Option<String>)> {
    actions
        .iter()
        .map(|action| {
            (
                action.command,
                keymap.binding(action.command).map(str::to_owned),
            )
        })
        .collect()
}
