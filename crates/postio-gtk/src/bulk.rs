//! Every button is the shared action bar's, so a press runs the command through the window's one
//! `act`, the path its key takes.

use std::rc::Rc;

use gtk::prelude::*;
use postio_core::{CommandId, Keymap};
use postio_ui::focus_dialog::Verb;
use postio_ui::hints;
use postio_widgets::widgets::{Action, ActionBar, KeyLine};

fn actions() -> &'static [Action] {
    crate::verbs::actions("focus-bulk-", postio_ui::focus_dialog::BULK)
}

pub struct Bulk {
    root: gtk::Box,
    count: gtk::Label,
    actions: Rc<ActionBar>,
    keys: KeyLine,
}

impl Bulk {
    pub fn new(keymap: &Keymap) -> Self {
        Self::offering(actions(), keymap)
    }

    pub fn with_verbs(verbs: &'static [Verb], keymap: &Keymap) -> Self {
        Self::offering(crate::verbs::actions("focus-bulk-", verbs), keymap)
    }

    fn offering(offered: &'static [Action], keymap: &Keymap) -> Self {
        let count = gtk::Label::new(None);
        count.add_css_class("focus-bulk-count");
        let actions = ActionBar::new(offered, "focus-bulk-actions");
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

    pub fn commands() -> Vec<CommandId> {
        actions().iter().map(|action| action.command).collect()
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn connect_command(&self, handler: impl Fn(CommandId) + 'static) {
        self.actions
            .connect_command(move |command| handler(command.id()));
    }

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

    /// `None` while the bar is hidden.
    pub fn summary(&self) -> Option<String> {
        self.root
            .is_visible()
            .then(|| self.count.text().to_string())
    }

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
