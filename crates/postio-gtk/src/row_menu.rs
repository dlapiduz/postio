//! The list row's right-click menu: the row's verbs, each with its key. The menu runs nothing
//! itself: a press hands the command to the window,
//! which runs it through the same `act` its key reaches. Verbs, order and what a right-click does
//! to the cursor are in `specs/007-postio-focus/screens.md`.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_core::{CommandId, Keymap};
use postio_widgets::widgets::{Kind, Size, keyhint, space::S1};

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

/// Delete is apart at the end, where a slip of the pointer is least likely to land on it.
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
        verb(CommandId::Unsnooze, "Unsnooze", "focus-row-menu-unsnooze"),
        verb(
            CommandId::RemindIfNoReply,
            "Remind if no reply\u{2026}",
            "focus-row-menu-remind",
        ),
        verb(CommandId::ToggleRead, "Mark read", "focus-row-menu-read"),
        verb(CommandId::Flag, "Flag", "focus-row-menu-flag"),
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

const ONE_MESSAGE: &[CommandId] = &[
    CommandId::OpenMessage,
    CommandId::Reply,
    CommandId::ReplyAll,
    CommandId::Forward,
];

#[derive(Debug, Clone, Copy, Default)]
pub struct Facts {
    pub unread: bool,
    pub flagged: bool,
    pub snoozed: bool,
}

type Handler = Rc<dyn Fn(CommandId)>;

pub struct RowMenu {
    popover: gtk::Popover,
    heading: gtk::Label,
    items: Vec<(CommandId, gtk::Button, gtk::Label)>,
    rules: Vec<(std::ops::Range<usize>, gtk::Separator)>,
    handler: RefCell<Option<Handler>>,
    outside: RefCell<Option<(glib::WeakRef<gtk::Widget>, gtk::GestureClick)>>,
}

impl RowMenu {
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
        // As the pickers: no grab, which a compositor may dismiss at once where no pointer serial
        // was given; the window's keyboard reaches the menu through `press`, and a press outside
        // closes it.
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

    pub fn commands() -> Vec<CommandId> {
        GROUPS
            .iter()
            .flat_map(|verbs| verbs.iter().map(|verb| verb.command))
            .collect()
    }

    pub fn set_keymap(&self, keymap: &Keymap) {
        for (command, _, cap) in &self.items {
            let key = keymap.binding(*command).unwrap_or_default();
            cap.set_text(key);
            cap.set_visible(!key.is_empty());
        }
        crate::a11y::teach_shortcuts(&self.popover);
    }

    pub fn connect_command(&self, handler: impl Fn(CommandId) + 'static) {
        self.handler.replace(Some(Rc::new(handler)));
    }

    /// `selected` names the selection it acts on ("3 selected"), or is `None` when it acts on the
    /// row alone.
    pub fn open(
        &self,
        parent: &impl IsA<gtk::Widget>,
        at: &gdk::Rectangle,
        selected: Option<&str>,
        facts: Facts,
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
            let words = match *command {
                CommandId::ToggleRead if selected.is_some() || facts.unread => Some("Mark read"),
                CommandId::ToggleRead => Some("Mark unread"),
                CommandId::Flag if selected.is_none() && facts.flagged => Some("Unflag"),
                CommandId::Flag => Some("Flag"),
                _ => None,
            };
            if let Some(words) = words {
                if let Some(label) = button
                    .child()
                    .and_then(|row| row.first_child())
                    .and_downcast::<gtk::Label>()
                {
                    label.set_text(words);
                }
                button.update_property(&[gtk::accessible::Property::Label(words)]);
            }
            button.set_visible(
                (selected.is_none() || !ONE_MESSAGE.contains(command))
                    && (*command != CommandId::Unsnooze || facts.snoozed),
            );
        }
        let shown = |items: &[(CommandId, gtk::Button, gtk::Label)]| {
            items.iter().any(|(_, button, _)| offered(button))
        };
        for (group, rule) in &self.rules {
            rule.set_visible(
                shown(&self.items[..group.start]) && shown(&self.items[group.clone()]),
            );
        }
        self.popover.set_pointing_to(Some(at));
        self.watch_outside(parent);
        self.popover.popup();
        if let Some((_, first, _)) = self.items.iter().find(|(_, b, _)| offered(b)) {
            first.grab_focus();
        }
    }

    pub fn close(&self) {
        self.popover.popdown();
    }

    pub fn is_open(&self) -> bool {
        self.popover.is_visible()
    }

    pub fn widget(&self) -> &gtk::Popover {
        &self.popover
    }

    /// Whether the key was the menu's; the arrows, Enter and Tab are left to GTK, which walks the
    /// menu's buttons with them.
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
                    .any(|(id, button, _)| *id == command && offered(button)) =>
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

    /// The popup is its own surface, so a press the window sees is outside it.
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

/// Its own visible flag, not whether it is on screen, which it is not until the menu pops up.
fn offered(item: &gtk::Button) -> bool {
    item.property::<bool>("visible")
}

impl std::fmt::Debug for RowMenu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RowMenu").finish_non_exhaustive()
    }
}
