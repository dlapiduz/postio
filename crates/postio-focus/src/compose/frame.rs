//! The frame around the composer, the message dialog's family (T221;
//! screens.md, "The composer"): a header bar of Detach, the title with
//! what will be sent under it, and the shared X; an action row of the
//! composer's verbs, Send first where Reply sits in the message dialog's;
//! and the Labels row.
//!
//! The composer inside it is the classic app's, whole (FR-050): every
//! control here calls one of its verbs, and every key a control shows is
//! read from the keymap in force (constitution II), compacted as the
//! message dialog's are (`hints::short`).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use chrono::{DateTime, Local, Utc};
use gtk::{gio, glib};
use postio_core::{CommandId, Keymap};
use postio_model::{Draft, DraftKind, Label, LabelId};
use postio_ui::focus_dialog;
use postio_ui::hints;
use postio_widgets::composer::Composer;
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::{S2, S4};
use postio_widgets::widgets::{Kind, Size};

/// What the header calls a composition (contracts/focus-surface.md,
/// "Compose"): "New message", or what it answers.
pub fn title(kind: DraftKind) -> &'static str {
    match kind {
        DraftKind::New => "New message",
        DraftKind::Reply => "Reply",
        DraftKind::ReplyAll => "Reply to all",
        DraftKind::Forward => "Forward",
    }
}

/// What the subtitle says will be sent: "Plain text · 58 words", or "Rich
/// text" once the body carries structure the text part cannot (the
/// composer sends an HTML part only then).
pub fn summary(draft: &Draft) -> String {
    let words = draft
        .body
        .text
        .as_deref()
        .map(|text| postio_model::signature::split(text).0)
        .unwrap_or_default()
        .split_whitespace()
        .count();
    let kind = if draft.body.html.is_some() {
        "Rich text"
    } else {
        "Plain text"
    };
    let words = match words {
        1 => "1 word".to_owned(),
        count => format!("{count} words"),
    };
    format!("{kind} \u{b7} {words}")
}

/// The header's subtitle: what will be sent, then what has happened to the
/// draft, when anything has -- "Plain text · 58 words · Draft saved locally
/// 16:12" -- as the message dialog's says "Message 5 of 60 · thread of 6".
pub fn subtitle(summary: &str, note: &str) -> String {
    if note.is_empty() {
        summary.to_owned()
    } else {
        format!("{summary} \u{b7} {note}")
    }
}

/// "Draft saved locally 16:12", for a save that landed at `at`.
pub fn saved_at(at: DateTime<Utc>) -> String {
    format!(
        "Draft saved locally {}",
        at.with_timezone(&Local).format("%H:%M")
    )
}

/// What the reminder verb says: "Remind", as the message dialog's action
/// row names the same command, and the day once one is chosen ("Remind ·
/// Tue 29 Sep"), in the person's own time zone. The whole of it -- "if no
/// reply" -- is the verb's tooltip and accessible name ([`remind_meaning`]),
/// the palette's title and the picker's heading; the row has room for the
/// day, not for both (T221).
pub fn remind_words(at: Option<DateTime<Utc>>) -> String {
    match at {
        Some(at) => format!(
            "Remind \u{b7} {}",
            at.with_timezone(&Local).format("%a %-d %b")
        ),
        None => "Remind".to_owned(),
    }
}

/// What the reminder verb means, said in full: "Remind if no reply", and
/// the day once one is chosen.
pub fn remind_meaning(at: Option<DateTime<Utc>>) -> String {
    match at {
        Some(at) => format!(
            "Remind if no reply \u{b7} {}",
            at.with_timezone(&Local).format("%a %-d %b")
        ),
        None => "Remind if no reply".to_owned(),
    }
}

/// The key a composer control shows for `command`: the one of its keys
/// that carries a modifier, since a bare letter is typed in the composer
/// (contracts/keymap.md: `mod+h` is the composer's key for `h`).
fn composer_key(keymap: &Keymap, command: CommandId) -> Option<String> {
    let bindings = keymap.bindings(command);
    bindings
        .iter()
        .find(|binding| binding.contains('+'))
        .or_else(|| bindings.first())
        .cloned()
}

/// The frame's widgets.
pub struct Frame {
    pub header: gtk::CenterBox,
    /// The composer's verbs, under the header, as the message dialog's
    /// action row is under its.
    pub actions: gtk::Box,
    pub labels_row: gtk::Box,
    close: gtk::Button,
    title: gtk::Label,
    /// What will be sent, and what happened to the draft.
    subtitle: gtk::Label,
    /// "Plain text · 58 words", as the draft stands.
    summary: RefCell<String>,
    /// "Draft saved locally 16:12", or what opening the draft did.
    note: RefCell<String>,
    detach: gtk::Button,
    send_later: gtk::Button,
    /// Send later's menu, hung on its button and rebuilt each time it opens.
    send_later_menu: gtk::PopoverMenu,
    send: gtk::Button,
    labels: gtk::Box,
    from_thread: gtk::Label,
    attach: gtk::Button,
    /// "Remind if no reply", and when (US3 scenario 5).
    pub remind: gtk::Button,
    /// The key the reminder control shows, and the time it says.
    remind_key: RefCell<Option<String>>,
    remind_at: Cell<Option<DateTime<Utc>>>,
    /// The labels whose names the Labels row can draw, by id.
    known: RefCell<HashMap<LabelId, Label>>,
    /// Whether the labels drawn are the thread's, untouched (screen 06).
    thread_labels: Cell<bool>,
}

impl Frame {
    /// The frame around `composer`, its keys read from `keymap`.
    pub fn new(composer: &Composer, keymap: &Keymap) -> Rc<Self> {
        // The header bar: the message dialog's anatomy (T206), Detach where
        // its steps are, the title centred, the shared X at the right
        // (T192). No verb reaches the title from either side.
        let close = postio_widgets::widgets::close_button();
        close.add_css_class("focus-compose-close");
        let title = gtk::Label::new(Some(title(DraftKind::New)));
        title.add_css_class("focus-compose-title");
        title.set_accessible_role(gtk::AccessibleRole::Heading);
        title.set_ellipsize(pango::EllipsizeMode::End);
        let subtitle = gtk::Label::new(None);
        subtitle.add_css_class("focus-compose-subtitle");
        subtitle.set_accessible_role(gtk::AccessibleRole::Status);
        subtitle.set_ellipsize(pango::EllipsizeMode::End);
        let heading = gtk::Box::new(gtk::Orientation::Vertical, 0);
        heading.add_css_class("focus-compose-heading");
        heading.append(&title);
        heading.append(&subtitle);

        let detach = postio_widgets::widgets::icon_button(
            "window-new-symbolic",
            "Write in a window of its own",
        );
        detach.add_css_class("focus-compose-detach");

        for widget in [
            detach.upcast_ref::<gtk::Widget>(),
            heading.upcast_ref(),
            close.upcast_ref(),
        ] {
            widget.set_valign(gtk::Align::Center);
        }
        let header = gtk::CenterBox::new();
        header.add_css_class("focus-compose-header");
        header.set_start_widget(Some(&detach));
        header.set_center_widget(Some(&heading));
        header.set_end_widget(Some(&close));

        // The action row: Send, the one primary (a plain raised button in
        // Focus, FR-091), then the quiet verbs the message dialog's row
        // draws. A button with a menu hung on it rather than a
        // `GtkMenuButton`, whose own nodes the button sheet never reaches:
        // Send later draws as every other verb here does.
        let send = gtk::Button::new();
        postio_widgets::widgets::button::style(&send, Kind::Primary, Size::Regular);
        send.add_css_class("focus-compose-send");
        let send_later = gtk::Button::new();
        postio_widgets::widgets::button::style(&send_later, Kind::Ghost, Size::Regular);
        send_later.add_css_class("focus-compose-send-later");
        let send_later_menu = gtk::PopoverMenu::from_model(None::<&gio::MenuModel>);
        send_later_menu.set_parent(&send_later);
        send_later_menu.set_has_arrow(false);
        send_later.connect_destroy({
            let menu = send_later_menu.clone();
            move |_| menu.unparent()
        });
        let attach = gtk::Button::new();
        postio_widgets::widgets::button::style(&attach, Kind::Ghost, Size::Regular);
        attach.add_css_class("focus-compose-attach");
        let remind = gtk::Button::new();
        postio_widgets::widgets::button::style(&remind, Kind::Ghost, Size::Regular);
        remind.add_css_class("focus-compose-remind");
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        actions.add_css_class("focus-compose-actions");
        for button in [&send, &send_later, &attach, &remind] {
            // Each as tall as its own words, not as the row.
            button.set_valign(gtk::Align::Center);
            actions.append(button);
        }

        // The Labels row, drawn the way the composer draws its own rows.
        let labels_row = gtk::Box::new(gtk::Orientation::Horizontal, S4);
        labels_row.add_css_class("postio-compose-row");
        labels_row.add_css_class("focus-compose-labels");
        let name = gtk::Label::new(Some("Labels"));
        name.add_css_class("postio-compose-label");
        name.set_xalign(0.0);
        name.set_width_chars(8);
        name.set_accessible_role(gtk::AccessibleRole::Presentation);
        let labels = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        labels.set_hexpand(true);
        labels.update_property(&[gtk::accessible::Property::Label("Labels")]);
        let from_thread = gtk::Label::new(Some("from the thread"));
        from_thread.add_css_class("dim-label");
        from_thread.set_visible(false);
        labels_row.append(&name);
        labels_row.append(&labels);
        labels_row.append(&from_thread);

        let frame = Rc::new(Frame {
            header,
            actions,
            labels_row,
            close,
            title,
            subtitle,
            summary: RefCell::default(),
            note: RefCell::default(),
            detach,
            send_later,
            send_later_menu,
            send,
            labels,
            from_thread,
            attach,
            remind,
            remind_key: RefCell::default(),
            remind_at: Cell::default(),
            known: RefCell::default(),
            thread_labels: Cell::new(false),
        });
        frame.set_keymap(keymap);
        frame.wire(composer);
        frame
    }

    /// Every control does what its key does: the composer's own verbs.
    fn wire(self: &Rc<Self>, composer: &Composer) {
        self.close.connect_clicked(glib::clone!(
            #[weak]
            composer,
            move |_| composer.dispatch(CommandId::Back)
        ));
        self.send.connect_clicked(glib::clone!(
            #[weak]
            composer,
            move |_| composer.send()
        ));
        self.attach.connect_clicked(glib::clone!(
            #[weak]
            composer,
            move |_| composer.dispatch(CommandId::AttachFile)
        ));
        self.detach.connect_clicked(glib::clone!(
            #[weak]
            composer,
            move |_| composer.dispatch(CommandId::DetachComposer)
        ));

        // Send later: the composer's own presets, rebuilt as the menu opens,
        // since they are relative to the moment it does.
        let frame = Rc::downgrade(self);
        self.send_later.connect_clicked(move |_| {
            if let Some(frame) = frame.upgrade() {
                frame.pop_send_later();
            }
        });
        let actions = gio::SimpleActionGroup::new();
        let choose = gio::SimpleAction::new("choose", Some(glib::VariantTy::INT64));
        choose.connect_activate(glib::clone!(
            #[weak]
            composer,
            move |_, parameter| {
                if let Some(when) = parameter
                    .and_then(|value| value.get::<i64>())
                    .and_then(DateTime::<Utc>::from_timestamp_millis)
                {
                    composer.send_later(when);
                }
            }
        ));
        actions.add_action(&choose);
        self.send_later
            .insert_action_group("focus-send-later", Some(&actions));

        // What the header and the verbs say follows the draft.
        let frame = Rc::downgrade(self);
        composer.connect_changed(move |draft| {
            if let Some(frame) = frame.upgrade() {
                frame.follow(draft);
            }
        });
        let frame = Rc::downgrade(self);
        let weak = composer.downgrade();
        composer.connect_opened(move || {
            if let (Some(frame), Some(composer)) = (frame.upgrade(), weak.upgrade()) {
                frame.thread_labels.set(false);
                frame.note.replace(String::new());
                frame.follow(&composer.draft());
                frame.draw_labels(&composer);
            }
        });
    }

    /// Redraw the keycaps from `keymap`, compacted as the message dialog's
    /// are (`hints::short`: `ctrl+⇧+↵`, not `ctrl+shift+Return`).
    pub fn set_keymap(&self, keymap: &Keymap) {
        let key = |command| hints::key(keymap, command).map(|key| hints::short(&key));
        self.send
            .set_child(Some(&labelled("Send", key(CommandId::Send).as_deref())));
        // Send later opens a menu: its words and key like the others, then
        // the arrow that says so.
        let later = labelled("Send later", key(CommandId::ScheduleSend).as_deref());
        if let Some(later) = later.downcast_ref::<gtk::Box>() {
            let arrow = gtk::Image::from_icon_name("pan-down-symbolic");
            arrow.set_accessible_role(gtk::AccessibleRole::Presentation);
            later.append(&arrow);
        }
        self.send_later.set_child(Some(&later));
        self.attach.set_child(Some(&labelled(
            "Attach",
            key(CommandId::AttachFile).as_deref(),
        )));
        self.remind_key.replace(
            composer_key(keymap, CommandId::RemindIfNoReply).map(|key| hints::short(&key)),
        );
        self.draw_remind(self.remind_at.get());
        self.detach
            .set_tooltip_text(Some(&match key(CommandId::DetachComposer) {
                Some(key) => format!("Write in a window of its own ({key})"),
                None => "Write in a window of its own".to_owned(),
            }));
    }

    /// The reminder control, saying `at` when one is chosen.
    fn draw_remind(&self, at: Option<DateTime<Utc>>) {
        self.remind_at.set(at);
        self.remind.set_child(Some(&labelled(
            &remind_words(at),
            self.remind_key.borrow().as_deref(),
        )));
        let meaning = remind_meaning(at);
        self.remind.set_tooltip_text(Some(&meaning));
        self.remind
            .update_property(&[gtk::accessible::Property::Label(&meaning)]);
    }

    /// The heading and the verbs, for `draft` as it stands.
    fn follow(&self, draft: &Draft) {
        self.title.set_text(title(draft.kind));
        self.summary.replace(summary(draft));
        self.draw_subtitle();
        self.draw_remind(draft.remind_at);
    }

    /// A save landed at `at`.
    pub fn saved(&self, at: DateTime<Utc>) {
        self.note.replace(saved_at(at));
        self.draw_subtitle();
    }

    /// Say `note` under the heading: what happened to the draft as it
    /// opened.
    pub fn note(&self, note: &str) {
        self.note.replace(note.to_owned());
        self.draw_subtitle();
    }

    fn draw_subtitle(&self) {
        self.subtitle
            .set_text(&subtitle(&self.summary.borrow(), &self.note.borrow()));
    }

    /// Open the Send later menu, as its key does.
    pub fn pop_send_later(&self) {
        let menu = gio::Menu::new();
        for (label, when) in postio_ui::schedule::schedule_presets(postio_ui::clock::now()) {
            let item = gio::MenuItem::new(Some(label), None);
            item.set_action_and_target_value(
                Some("focus-send-later.choose"),
                Some(&when.with_timezone(&Utc).timestamp_millis().to_variant()),
            );
            menu.append_item(&item);
        }
        self.send_later_menu.set_menu_model(Some(&menu));
        self.send_later_menu.popup();
    }

    /// Learn the names of `labels`, so the row can draw them.
    pub fn know(&self, labels: impl IntoIterator<Item = Label>) {
        let mut known = self.known.borrow_mut();
        for label in labels {
            known.insert(label.id, label);
        }
    }

    /// Whether every label on `composer`'s draft has a name to draw.
    pub fn knows_all(&self, composer: &Composer) -> bool {
        let known = self.known.borrow();
        composer.labels().iter().all(|id| known.contains_key(id))
    }

    /// The draft starts with its thread's labels (screen 06, FR-053).
    pub fn from_the_thread(self: &Rc<Self>, composer: &Composer) {
        self.thread_labels.set(!composer.labels().is_empty());
        self.draw_labels(composer);
    }

    /// Draw the draft's labels as chips, each with its ×.
    pub fn draw_labels(self: &Rc<Self>, composer: &Composer) {
        while let Some(child) = self.labels.first_child() {
            self.labels.remove(&child);
        }
        let known = self.known.borrow();
        let chosen = composer.labels();
        for id in &chosen {
            let Some(label) = known.get(id) else {
                continue;
            };
            let chip = gtk::Box::new(gtk::Orientation::Horizontal, S2);
            chip.add_css_class("postio-chip");
            chip.add_css_class("focus-compose-label-chip");
            let name = gtk::Label::new(Some(&label.name));
            name.add_css_class("heading");
            let spoken = format!("Remove the label {}", label.name);
            let remove = postio_widgets::widgets::icon_button("window-close-symbolic", &spoken);
            remove.add_css_class("circular");
            let id = *id;
            let frame = Rc::downgrade(self);
            remove.connect_clicked(glib::clone!(
                #[weak]
                composer,
                move |_| {
                    let kept: Vec<LabelId> = composer
                        .labels()
                        .into_iter()
                        .filter(|label| *label != id)
                        .collect();
                    composer.set_labels(kept);
                    if let Some(frame) = frame.upgrade() {
                        frame.thread_labels.set(false);
                        frame.draw_labels(&composer);
                    }
                }
            ));
            chip.append(&name);
            chip.append(&remove);
            self.labels.append(&chip);
        }
        self.from_thread
            .set_visible(self.thread_labels.get() && !chosen.is_empty());
    }

    /// The commands the frame's controls run, for registry parity.
    pub fn commands() -> [CommandId; 6] {
        [
            CommandId::Back,
            CommandId::Send,
            CommandId::ScheduleSend,
            CommandId::AttachFile,
            CommandId::DetachComposer,
            CommandId::RemindIfNoReply,
        ]
    }
}

/// A verb's words and its cap, the cap the message dialog's gap from them
/// (`focus_dialog::KEYCAP_GAP`).
fn labelled(text: &str, key: Option<&str>) -> gtk::Widget {
    let widget = keyhint::labelled(text, key);
    if let Some(row) = widget.downcast_ref::<gtk::Box>() {
        row.set_spacing(focus_dialog::KEYCAP_GAP);
    }
    widget
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use postio_model::AccountId;

    #[test]
    fn the_subtitle_counts_the_words_that_will_be_sent_and_says_which_kind() {
        let mut draft = Draft::new(AccountId::UNASSIGNED);
        assert_eq!(summary(&draft), "Plain text \u{b7} 0 words");
        draft.body.text = Some("Hi Ada,\n\nThe sheet is attached.".to_owned());
        assert_eq!(summary(&draft), "Plain text \u{b7} 6 words");
        draft.body.text = Some("One".to_owned());
        assert_eq!(summary(&draft), "Plain text \u{b7} 1 word");
        draft.body.html = Some("<p><b>One</b></p>".to_owned());
        assert_eq!(summary(&draft), "Rich text \u{b7} 1 word");
    }

    #[test]
    fn the_signature_is_not_counted_as_written() {
        let mut draft = Draft::new(AccountId::UNASSIGNED);
        draft.body.text = Some("Looks good.\n\n-- \nAda Norwood\nExample Corp".to_owned());
        assert_eq!(summary(&draft), "Plain text \u{b7} 2 words");
    }

    #[test]
    fn the_subtitle_says_what_will_be_sent_then_what_happened() {
        assert_eq!(
            subtitle("Plain text \u{b7} 0 words", ""),
            "Plain text \u{b7} 0 words"
        );
        assert_eq!(
            subtitle("Plain text \u{b7} 58 words", "Draft saved locally 16:12"),
            "Plain text \u{b7} 58 words \u{b7} Draft saved locally 16:12"
        );
    }

    #[test]
    fn remind_is_the_message_dialogs_word_and_says_its_day() {
        assert_eq!(remind_words(None), "Remind");
        assert_eq!(remind_meaning(None), "Remind if no reply");
        let at = Local
            .with_ymd_and_hms(2026, 9, 29, 9, 0, 0)
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(remind_words(Some(at)), "Remind \u{b7} Tue 29 Sep");
        assert_eq!(
            remind_meaning(Some(at)),
            "Remind if no reply \u{b7} Tue 29 Sep"
        );
    }

    #[test]
    fn the_header_names_a_composition_as_the_screens_do() {
        assert_eq!(title(DraftKind::New), "New message");
        assert_eq!(title(DraftKind::ReplyAll), "Reply to all");
    }
}
