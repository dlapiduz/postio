//! A row of `KeycapButton`s, identical on every screen. Each button runs `Command::default_for` on
//! the same `CommandId` as its keybinding and hands it to `connect_command`: one path to the
//! composer and queue.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use postio_core::{Command, CommandId, Keymap};

use super::keycap::KeycapButton;

type CommandHandler = Box<dyn Fn(Command)>;

#[derive(Clone, Copy, Debug)]
pub struct Action {
    pub command: CommandId,
    pub label: &'static str,
    pub class: &'static str,
    pub primary: bool,
    /// The label stays the accessible name and tooltip. A platform icon name, not a drawn asset
    /// (`PRODUCT.md` section 19).
    pub icon: Option<&'static str>,
}

impl Action {
    pub const fn new(command: CommandId, label: &'static str, class: &'static str) -> Self {
        Self {
            command,
            label,
            class,
            primary: false,
            icon: None,
        }
    }

    pub const fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    pub const fn primary(mut self) -> Self {
        self.primary = true;
        self
    }
}

pub struct ActionBar {
    root: gtk::Box,
    buttons: Vec<Rc<KeycapButton>>,
    commands: RefCell<Vec<CommandHandler>>,
}

impl ActionBar {
    /// Returned as `Rc` because a button's `clicked` handler calls back into `self`, weakly.
    pub fn new(actions: &[Action], class: &str) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        root.add_css_class("postio-action-bar");
        root.add_css_class(class);

        let buttons: Vec<Rc<KeycapButton>> = actions
            .iter()
            .map(|action| {
                let button = Rc::new(match action.icon {
                    Some(icon) => KeycapButton::with_icon(
                        Some(action.command),
                        action.label,
                        icon,
                        action.class,
                        action.primary,
                    ),
                    None => KeycapButton::new(
                        Some(action.command),
                        action.label,
                        action.class,
                        action.primary,
                    ),
                });
                KeycapButton::arm(&button);
                // Non-primary verbs in a bar are secondary buttons with visible edges, not ghosts
                // (#1174).
                if !action.primary {
                    super::button::style(
                        &button.widget(),
                        super::button::Kind::Secondary,
                        super::button::Size::Regular,
                    );
                }
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

        // Defaults until `Window::apply_keymap` runs, so a hint never depends on call order.
        this.set_keymap(Keymap::defaults());
        this
    }

    /// Separates a bar's verbs from the status a surface puts beside them.
    pub fn append_trailing(&self, widget: &impl IsA<gtk::Widget>) {
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        self.root.append(&spacer);
        self.root.append(widget);
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    pub fn set_visible(&self, visible: bool) {
        self.root.set_visible(visible);
    }

    pub fn is_visible(&self) -> bool {
        self.root.is_visible()
    }

    pub fn set_keymap(&self, keymap: &Keymap) {
        for button in &self.buttons {
            let Some(id) = button.command() else { continue };
            button.set_key(keymap.binding(id));
        }
    }

    pub fn button(&self, command: CommandId) -> Option<&Rc<KeycapButton>> {
        self.buttons
            .iter()
            .find(|button| button.command() == Some(command))
    }

    /// The same `Command` the keyboard's binding for the verb produces.
    pub fn connect_command(&self, handler: impl Fn(Command) + 'static) {
        self.commands.borrow_mut().push(Box::new(handler));
    }

    pub fn press(&self, command: CommandId) {
        if let Some(button) = self.button(command) {
            button.press();
        }
    }

    fn run(&self, command: CommandId) {
        let command = Command::default_for(command);
        for handler in self.commands.borrow().iter() {
            handler(command.clone());
        }
    }
}

/// `None` where the user cleared a binding or lost it to a collision. Free so a rebind is testable
/// without a display.
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
