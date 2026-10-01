//! The list row's right-click menu (T199): the row's verbs, as the open
//! message's toolbar and the bulk bar offer them, each with its key.
//!
//! The menu runs nothing itself. A press hands the command's id to the
//! window, which runs it through the same `act` its key reaches: one verb,
//! one meaning, from a key, the command bar, a toolbar or this menu. Which
//! verbs, in which order, and what a right-click does to the cursor and the
//! selection, are recorded in `specs/007-postio-focus/screens.md`
//! ("The row menu").

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_core::{CommandId, Keymap};
use postio_widgets::widgets::{Kind, Size, keyhint, space::S1};

/// One verb: its command, the words the menu says, and the class a test
/// finds it by.
struct Verb {
    command: CommandId,
    label: &'static str,
    class: &'static str,
}

const fn verb(command: CommandId, label: &'static str, class: &'static str) -> Verb {
    Verb {
        command,
        label,
        class,
    }
}

/// The menu, in groups: open; answer; triage (the bulk bar's first verbs,
/// in its order); file; and Delete apart at the end, where a slip of the
/// pointer is least likely to land on it.
const GROUPS: &[&[Verb]] = &[
    &[verb(CommandId::OpenMessage, "Open", "focus-row-menu-open")],
    &[
        verb(CommandId::Reply, "Reply", "focus-row-menu-reply"),
        verb(CommandId::ReplyAll, "Reply all", "focus-row-menu-reply-all"),
        verb(CommandId::Forward, "Forward", "focus-row-menu-forward"),
    ],
    &[
        verb(CommandId::Archive, "Archive", "focus-row-menu-archive"),
        verb(CommandId::Snooze, "Snooze\u{2026}", "focus-row-menu-snooze"),
        verb(
            CommandId::RemindIfNoReply,
            "Remind if no reply\u{2026}",
            "focus-row-menu-remind",
        ),
        verb(CommandId::ToggleRead, "Mark read", "focus-row-menu-read"),
    ],
    &[
        verb(CommandId::AddLabel, "Label\u{2026}", "focus-row-menu-label"),
        verb(CommandId::Move, "Move\u{2026}", "focus-row-menu-move"),
        verb(
            CommandId::DigestRule,
            "Digest mail like this\u{2026}",
            "focus-row-menu-digest",
        ),
    ],
    &[verb(CommandId::Delete, "Delete", "focus-row-menu-delete")],
];

/// The verbs a reply needs a single message for: offered on one row, not
/// on a selection, as the open message's toolbar is for one message.
const ONE_MESSAGE: &[CommandId] = &[
    CommandId::OpenMessage,
    CommandId::Reply,
    CommandId::ReplyAll,
    CommandId::Forward,
];

/// What a press asks the window to run.
type Handler = Rc<dyn Fn(CommandId)>;

/// The menu, built once and shown over whichever row was right-clicked.
pub struct RowMenu {
    popover: gtk::Popover,
    heading: gtk::Label,
    /// Each verb's button and the cap that shows its key.
    items: Vec<(CommandId, gtk::Button, gtk::Label)>,
    /// The rule above each group but the first, and the items that group
    /// holds: shown when the group and something above it are.
    rules: Vec<(std::ops::Range<usize>, gtk::Separator)>,
    handler: RefCell<Option<Handler>>,
    /// Watching the window for a press outside the menu.
    outside: RefCell<Option<(glib::WeakRef<gtk::Widget>, gtk::GestureClick)>>,
}

impl RowMenu {
    /// The menu, its keys read from `keymap`.
    pub fn new(keymap: &Keymap) -> Rc<Self> {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.add_css_class("focus-row-menu");
        let heading = gtk::Label::new(None);
        heading.add_css_class("focus-row-menu-heading");
        heading.set_xalign(0.0);
        heading.set_visible(false);
        column.append(&heading);
        let mut items = Vec::new();
        let mut rules = Vec::new();
        for (group, verbs) in GROUPS.iter().enumerate() {
            if group > 0 {
                let rule = gtk::Separator::new(gtk::Orientation::Horizontal);
                rule.add_css_class("focus-row-menu-rule");
                column.append(&rule);
                let start = items.len();
                rules.push((start..start + verbs.len(), rule));
            }
            for verb in *verbs {
                let words = gtk::Label::new(Some(verb.label));
                words.set_xalign(0.0);
                words.set_hexpand(true);
                let cap = keyhint::cap("");
                let row = gtk::Box::new(gtk::Orientation::Horizontal, S1 * 4);
                row.append(&words);
                row.append(&cap);
                let button = gtk::Button::new();
                postio_widgets::widgets::button::style(&button, Kind::Ghost, Size::Regular);
                button.add_css_class("focus-row-menu-item");
                button.add_css_class(verb.class);
                button.set_child(Some(&row));
                button.update_property(&[gtk::accessible::Property::Label(verb.label)]);
                column.append(&button);
                items.push((verb.command, button, cap));
            }
        }
        let popover = gtk::Popover::builder()
            .child(&column)
            .has_arrow(false)
            .position(gtk::PositionType::Bottom)
            .halign(gtk::Align::Start)
            .build();
        popover.add_css_class("focus-row-menu-popover");
        popover.set_widget_name("focus-row-menu");
        // As the pickers: no grab, which a compositor may dismiss at once
        // where no pointer serial was given; the window's keyboard reaches
        // the menu through `press`, and a press outside closes it.
        popover.set_autohide(false);
        popover.set_accessible_role(gtk::AccessibleRole::Menu);

        let menu = Rc::new(RowMenu {
            popover,
            heading,
            items,
            rules,
            handler: RefCell::default(),
            outside: RefCell::default(),
        });
        for (command, button, _) in &menu.items {
            let command = *command;
            let weak = Rc::downgrade(&menu);
            button.connect_clicked(move |_| {
                if let Some(menu) = weak.upgrade() {
                    menu.run(command);
                }
            });
        }
        menu.set_keymap(keymap);
        menu
    }

    /// Every command the menu offers.
    pub fn commands() -> Vec<CommandId> {
        GROUPS
            .iter()
            .flat_map(|verbs| verbs.iter().map(|verb| verb.command))
            .collect()
    }

    /// Read every key from `keymap`: a verb with no binding shows no cap.
    pub fn set_keymap(&self, keymap: &Keymap) {
        for (command, _, cap) in &self.items {
            let key = keymap.binding(*command).unwrap_or_default();
            cap.set_text(key);
            cap.set_visible(!key.is_empty());
        }
        crate::a11y::teach_shortcuts(&self.popover);
    }

    /// Run `handler` with the command a pressed verb stands for.
    pub fn connect_command(&self, handler: impl Fn(CommandId) + 'static) {
        self.handler.replace(Some(Rc::new(handler)));
    }

    /// Show the menu from `parent`, at `at` in its coordinates.
    ///
    /// `selected` is how the selection it will act on is named ("3
    /// selected"), or `None` when it acts on the row alone; `unread` says
    /// how to word the read verb for that one row.
    pub fn open(
        &self,
        parent: &impl IsA<gtk::Widget>,
        at: &gdk::Rectangle,
        selected: Option<&str>,
        unread: bool,
    ) {
        let parent = parent.as_ref();
        if self.popover.parent().as_ref() != Some(parent) {
            if self.popover.parent().is_some() {
                self.popover.unparent();
            }
            self.popover.set_parent(parent);
            parent.connect_destroy({
                let popover = self.popover.downgrade();
                move |_| {
                    if let Some(popover) = popover.upgrade() {
                        popover.unparent();
                    }
                }
            });
        }
        self.heading.set_text(selected.unwrap_or_default());
        self.heading.set_visible(selected.is_some());
        for (command, button, _) in &self.items {
            if *command == CommandId::ToggleRead {
                let words = if selected.is_some() || unread {
                    "Mark read"
                } else {
                    "Mark unread"
                };
                if let Some(label) = button
                    .child()
                    .and_then(|row| row.first_child())
                    .and_downcast::<gtk::Label>()
                {
                    label.set_text(words);
                }
                button.update_property(&[gtk::accessible::Property::Label(words)]);
            }
            button.set_visible(selected.is_none() || !ONE_MESSAGE.contains(command));
        }
        // A group's rule shows when something above it and in it does.
        let shown = |items: &[(CommandId, gtk::Button, gtk::Label)]| {
            items.iter().any(|(_, button, _)| button.is_visible())
        };
        for (group, rule) in &self.rules {
            rule.set_visible(
                shown(&self.items[..group.start]) && shown(&self.items[group.clone()]),
            );
        }
        self.popover.set_pointing_to(Some(at));
        self.watch_outside(parent);
        self.popover.popup();
        if let Some((_, first, _)) = self.items.iter().find(|(_, b, _)| b.is_visible()) {
            first.grab_focus();
        }
    }

    /// Close the menu, running nothing.
    pub fn close(&self) {
        self.popover.popdown();
    }

    /// Whether the menu is up.
    pub fn is_open(&self) -> bool {
        self.popover.is_visible()
    }

    /// The popover, for a test to read.
    pub fn widget(&self) -> &gtk::Popover {
        &self.popover
    }

    /// A key the window was given while the menu is up: Escape closes it,
    /// and a verb's own key runs that verb from the menu. Whether the key
    /// was the menu's; the rest (the arrows, Enter, Tab) are left to GTK,
    /// which walks the menu's buttons with them.
    pub fn press(&self, command: Option<CommandId>, key: gdk::Key) -> bool {
        if key == gdk::Key::Escape {
            self.close();
            return true;
        }
        match command {
            Some(command)
                if self
                    .items
                    .iter()
                    .any(|(id, button, _)| *id == command && button.is_visible()) =>
            {
                self.run(command);
                true
            }
            _ => false,
        }
    }

    fn run(&self, command: CommandId) {
        self.close();
        let handler = self.handler.borrow().clone();
        if let Some(handler) = handler {
            handler(command);
        }
    }

    /// Close the menu on a press in the window it opened over: the popup is
    /// its own surface, so a press the window sees is outside it.
    fn watch_outside(&self, parent: &gtk::Widget) {
        let Some(root) = parent.root().map(|root| root.upcast::<gtk::Widget>()) else {
            return;
        };
        let watching = self
            .outside
            .borrow()
            .as_ref()
            .and_then(|(window, _)| window.upgrade())
            .is_some_and(|window| window == root);
        if watching {
            return;
        }
        let gesture = gtk::GestureClick::new();
        gesture.set_button(0);
        gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
        gesture.connect_pressed({
            let popover = self.popover.downgrade();
            move |_, _, _, _| {
                if let Some(popover) = popover.upgrade().filter(|popover| popover.is_visible()) {
                    popover.popdown();
                }
            }
        });
        root.add_controller(gesture.clone());
        self.outside.replace(Some((root.downgrade(), gesture)));
    }
}

impl std::fmt::Debug for RowMenu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RowMenu").finish_non_exhaustive()
    }
}
