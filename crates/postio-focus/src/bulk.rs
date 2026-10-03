//! The bulk bar: what is selected, and what to do with it
//! (contracts/focus-surface.md, "The window": 44 px, while anything is
//! selected).
//!
//! The count; Archive, Snooze, Mark read, Digest these…, Label, Move and Delete,
//! each with its key; and on the right the selection's own keys. Every
//! button is the shared action bar's, so a press runs the command's id
//! through the window's one `act`, the path its key takes.

use std::rc::Rc;

use gtk::prelude::*;
use postio_core::{CommandId, Keymap};
use postio_ui::hints;
use postio_widgets::widgets::{Action, ActionBar, KeyLine};

/// What the bar offers: `postio_ui::focus_dialog::BULK`, each button named
/// `focus-bulk-<slug>`.
fn actions() -> &'static [Action] {
    crate::verbs::actions("focus-bulk-", postio_ui::focus_dialog::BULK)
}

/// The bar, its count and its keys.
pub struct Bulk {
    root: gtk::Box,
    count: gtk::Label,
    actions: Rc<ActionBar>,
    keys: KeyLine,
}

impl Bulk {
    /// The bar, hidden until something is selected, its keys from `keymap`.
    pub fn new(keymap: &Keymap) -> Self {
        let count = gtk::Label::new(None);
        count.add_css_class("focus-bulk-count");
        let actions = ActionBar::new(actions(), "focus-bulk-actions");
        let keys = KeyLine::new("focus-bulk-keys");
        actions.append_trailing(keys.widget());
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.add_css_class("focus-bulk-bar");
        root.append(&count);
        let bar = actions.widget();
        bar.set_hexpand(true);
        root.append(&bar);
        root.set_visible(false);
        let bulk = Bulk {
            root,
            count,
            actions,
            keys,
        };
        bulk.set_keymap(keymap);
        bulk
    }

    /// Every command the bar has a button for.
    pub fn commands() -> Vec<CommandId> {
        actions().iter().map(|action| action.command).collect()
    }

    /// The bar, to place under the list.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Run `handler` with the command a button stands for when it is pressed.
    pub fn connect_command(&self, handler: impl Fn(CommandId) + 'static) {
        self.actions
            .connect_command(move |command| handler(command.id()));
    }

    /// Read every key from `keymap`.
    pub fn set_keymap(&self, keymap: &Keymap) {
        self.actions.set_keymap(keymap);
        let said: Vec<_> = [
            hints::hint(keymap, CommandId::ToggleSelection, "toggle"),
            hints::pair(
                keymap,
                CommandId::ExtendSelectionDown,
                CommandId::ExtendSelectionUp,
                "extend",
            ),
            hints::hint(keymap, CommandId::Back, "clear"),
        ]
        .into_iter()
        .flatten()
        .collect();
        self.keys.set(&said);
    }

    /// What the bar says while it is up: `None` while it is hidden. Read off
    /// the widgets, for a storyboard's observation.
    pub fn summary(&self) -> Option<String> {
        self.root
            .is_visible()
            .then(|| self.count.text().to_string())
    }

    /// Say what is selected, or hide the bar when nothing is.
    pub fn set_summary(&self, summary: Option<&str>) {
        match summary {
            Some(said) => {
                self.count.set_text(said);
                self.root.set_visible(true);
            }
            None => self.root.set_visible(false),
        }
    }
}
