//! The reading pane's action bar (#498): Reply, Reply all, Forward and
//! Archive, under the body — the pointer's way to the same four verbs `e`,
//! `E`, `f` and `a` already reach from the keyboard. Before this the reading
//! pane had no click target for any of them at all: a mouse user reading a
//! message had nothing to click, and nothing on screen said the keys existed.
//!
//! # One path to the composer and the queue
//!
//! Every button here runs [`Command::default_for`] on the same [`CommandId`]
//! the keybinding does, and hands it out through [`ReaderActions::connect_command`]
//! for whoever mounts the reader to act on — the same shape
//! [`crate::list_view::MessageListView::connect_command`] already uses for a row's
//! hover actions and context menu. There is no second "reply from a button"
//! implementation to keep in step with the real one.
//!
//! # Keys the bar shows are the keys that work
//!
//! Each button's hint comes from [`ReaderActions::set_keymap`], not a
//! hard-coded letter — a `[keys]` rebind changes what the button says the
//! same moment it changes what the keyboard does.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use postio_core::{Command, CommandId, Keymap};

type CommandHandler = Box<dyn Fn(Command)>;

/// One button: the command it runs, its label, the CSS class a test finds it
/// by, and whether it gets the primary (`suggested-action`) treatment.
const BUTTONS: [(CommandId, &str, &str, bool); 4] = [
    (
        CommandId::Reply,
        "Reply",
        "postio-reader-action-reply",
        true,
    ),
    (
        CommandId::ReplyAll,
        "Reply all",
        "postio-reader-action-reply-all",
        false,
    ),
    (
        CommandId::Forward,
        "Forward",
        "postio-reader-action-forward",
        false,
    ),
    (
        CommandId::Archive,
        "Archive",
        "postio-reader-action-archive",
        false,
    ),
];

/// The bar itself. Hidden by default — [`super::view::Reader`] shows it only
/// while a message actually occupies the pane, and re-hides it the moment
/// the pane empties or the composer takes over.
pub struct ReaderActions {
    root: gtk::Box,
    buttons: Vec<(CommandId, gtk::Button, gtk::Label)>,
    commands: RefCell<Vec<CommandHandler>>,
}

impl ReaderActions {
    /// Builds the bar. Returned as `Rc` because a button's own `clicked`
    /// handler needs to call back into `self`, the same reason
    /// `composer::Completion::install` does — see there for why that must be
    /// a weak reference rather than a cycle.
    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        root.add_css_class("postio-reader-actions");
        root.set_visible(false);

        let mut buttons = Vec::new();
        for (id, label, class, primary) in BUTTONS {
            let hint = gtk::Label::new(None);
            hint.add_css_class("postio-keyhint");
            // A class of its own, distinct from the button's: a test finding
            // widgets by class needs to tell "the button" and "the label
            // showing its key" apart.
            hint.add_css_class(&format!("{class}-hint"));
            hint.set_accessible_role(gtk::AccessibleRole::Presentation);
            hint.set_visible(false);

            let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            content.append(&gtk::Label::new(Some(label)));
            content.append(&hint);

            let button = gtk::Button::new();
            button.set_child(Some(&content));
            button.add_css_class(class);
            button.update_property(&[gtk::accessible::Property::Label(label)]);
            if primary {
                button.add_css_class("suggested-action");
            } else {
                button.add_css_class("flat");
                button.add_css_class("postio-ghost");
            }
            root.append(&button);
            buttons.push((id, button, hint));
        }

        // The canvas right-aligns a thread-position status after these four
        // ("2/6 · n next in thread"); the reader has no notion of thread
        // position yet, so this only reserves the space rather than
        // fabricating a count. See #498's PR for the reasoning.
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        root.append(&spacer);

        let this = Rc::new(Self {
            root,
            buttons,
            commands: RefCell::new(Vec::new()),
        });

        for (id, button, _) in &this.buttons {
            let id = *id;
            button.connect_clicked(glib::clone!(
                #[weak(rename_to = actions)]
                this,
                move |_| actions.run(id)
            ));
        }

        // A sensible key hint from the moment the bar exists, the same
        // defensive default `list_view`'s row keeps for its own hints:
        // `Window::apply_keymap` is what keeps this current once a real
        // config loads, but nothing here should depend on that call having
        // already happened by the time anyone looks.
        this.set_keymap(&Keymap::resolve(&Default::default()));

        this
    }

    /// The widget to place under the body.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// Show or hide the whole bar — visible only while a message actually
    /// occupies the pane.
    pub fn set_visible(&self, visible: bool) {
        self.root.set_visible(visible);
    }

    /// The current key for each button, from the live keymap rather than a
    /// hard-coded letter — a rebind in `config.toml` reaches this the same
    /// moment it reaches the keyboard. A command with no binding at all (the
    /// user cleared it) hides that button's hint rather than showing a blank.
    pub fn set_keymap(&self, keymap: &Keymap) {
        for ((_, key), (_, _, hint)) in hints(keymap).into_iter().zip(&self.buttons) {
            match key {
                Some(key) => {
                    hint.set_label(&key);
                    hint.set_visible(true);
                }
                None => hint.set_visible(false),
            }
        }
    }

    /// Called with the invocation whenever a button is pressed — the same
    /// [`Command`] the keyboard's binding for the same verb would produce.
    pub fn connect_command(&self, handler: impl Fn(Command) + 'static) {
        self.commands.borrow_mut().push(Box::new(handler));
    }

    /// Run `id` against whatever the reader is showing — what pressing the
    /// keyboard's own binding for it already means, replayed for the mouse.
    fn run(&self, id: CommandId) {
        let command = Command::default_for(id);
        for handler in self.commands.borrow().iter() {
            handler(command.clone());
        }
    }
}

/// [`BUTTONS`]' commands, each paired with the key `keymap` currently gives
/// it — `None` when the user has cleared a binding entirely rather than
/// merely changed it. A free function, decoupled from the widgets
/// [`ReaderActions::set_keymap`] updates from it, so a rebind reaching the
/// bar is testable without a display — the same split `crate::row`'s
/// `hints_for` makes for the focused row's own key hints.
fn hints(keymap: &Keymap) -> Vec<(CommandId, Option<String>)> {
    BUTTONS
        .iter()
        .map(|(id, ..)| (*id, keymap.binding(*id).map(str::to_owned)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_keys_are_the_ones_the_registry_gives_reply_reply_all_forward_and_archive() {
        let keymap = Keymap::resolve(&Default::default());
        let keys: Vec<_> = hints(&keymap).into_iter().map(|(_, key)| key).collect();
        assert_eq!(
            keys,
            vec![
                Some("e".to_string()),
                Some("E".to_string()),
                Some("f".to_string()),
                Some("a".to_string()),
            ],
            "canvas order: Reply, Reply all, Forward, Archive"
        );
    }

    #[test]
    fn a_rebind_in_keys_reaches_the_bar_not_just_the_resolver() {
        let mut overrides = postio_config::KeyBindings::default();
        overrides
            .overrides_mut()
            .insert("reply".to_string(), "r".to_string());
        let keymap = Keymap::resolve(&overrides);
        let keys: Vec<_> = hints(&keymap).into_iter().map(|(_, key)| key).collect();
        assert_eq!(keys[0], Some("r".to_string()), "Reply picked up the rebind");
        assert_eq!(
            keys[1],
            Some("E".to_string()),
            "Reply all kept its default; only Reply was rebound"
        );
    }

    #[test]
    fn a_key_lost_to_another_command_hides_the_hint_rather_than_showing_a_wrong_one() {
        let mut overrides = postio_config::KeyBindings::default();
        // `undo`'s default is `u`; rebinding it to `a` collides with
        // Archive's own default in every context they share (list, thread,
        // reader), so Archive loses its key rather than being handed one
        // that now runs something else.
        overrides
            .overrides_mut()
            .insert("undo".to_string(), "a".to_string());
        let keymap = Keymap::resolve(&overrides);
        let keys: Vec<_> = hints(&keymap).into_iter().map(|(_, key)| key).collect();
        assert_eq!(
            keys[3], None,
            "Archive has no key to show once undo has taken `a`"
        );
    }
}
