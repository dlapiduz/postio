//! The frame screens 05 and 06 draw around the composer: the header with
//! Close, the heading and Send; the Labels row; and the footer.
//!
//! The composer inside it is the classic app's, whole (FR-050): every
//! control here calls one of its verbs, and every key a control shows is
//! read from the keymap in force (constitution II).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use chrono::{DateTime, Local, Utc};
use gtk::{gio, glib};
use postio_core::{CommandId, Keymap};
use postio_model::{Draft, DraftKind, Label, LabelId};
use postio_ui::hints;
use postio_widgets::composer::Composer;
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::{S2, S3, S4};
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

/// What the footer says will be sent: "Plain text · 58 words", or "Rich
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

/// "Draft saved locally 16:12", for a save that landed at `at`.
pub fn saved_at(at: DateTime<Utc>) -> String {
    format!(
        "Draft saved locally {}",
        at.with_timezone(&Local).format("%H:%M")
    )
}

/// The frame's widgets.
pub struct Frame {
    pub header: gtk::CenterBox,
    pub footer: gtk::Box,
    pub labels_row: gtk::Box,
    close: gtk::Button,
    title: gtk::Label,
    saved: gtk::Label,
    detach: gtk::Button,
    send_later: gtk::MenuButton,
    send: gtk::Button,
    labels: gtk::Box,
    from_thread: gtk::Label,
    attach: gtk::Button,
    words: gtk::Label,
    /// The labels whose names the Labels row can draw, by id.
    known: RefCell<HashMap<LabelId, Label>>,
    /// Whether the labels drawn are the thread's, untouched (screen 06).
    thread_labels: Cell<bool>,
}

impl Frame {
    /// The frame around `composer`, its keys read from `keymap`.
    pub fn new(composer: &Composer, keymap: &Keymap) -> Rc<Self> {
        let close = gtk::Button::new();
        postio_widgets::widgets::button::style(&close, Kind::Secondary, Size::Small);
        close.add_css_class("focus-compose-close");
        let title = gtk::Label::new(Some(title(DraftKind::New)));
        title.add_css_class("focus-compose-title");
        title.set_accessible_role(gtk::AccessibleRole::Heading);
        let saved = gtk::Label::new(None);
        saved.add_css_class("focus-compose-saved");
        saved.set_accessible_role(gtk::AccessibleRole::Status);
        let heading = gtk::Box::new(gtk::Orientation::Vertical, 0);
        heading.append(&title);
        heading.append(&saved);

        let detach = gtk::Button::from_icon_name("window-new-symbolic");
        detach.add_css_class("flat");
        detach.set_tooltip_text(Some("Write in a window of its own"));
        detach.update_property(&[gtk::accessible::Property::Label(
            "Write in a window of its own",
        )]);
        let send_later = gtk::MenuButton::new();
        postio_widgets::widgets::button::style(&send_later, Kind::Ghost, Size::Small);
        send_later.set_label("Send later");
        send_later.add_css_class("focus-compose-send-later");
        let send = gtk::Button::new();
        postio_widgets::widgets::button::style(&send, Kind::Primary, Size::Small);
        send.add_css_class("focus-compose-send");
        let trailing = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        trailing.append(&detach);
        trailing.append(&send_later);
        trailing.append(&send);

        let header = gtk::CenterBox::new();
        header.add_css_class("focus-compose-header");
        header.set_start_widget(Some(&close));
        header.set_center_widget(Some(&heading));
        header.set_end_widget(Some(&trailing));

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

        let attach = gtk::Button::new();
        postio_widgets::widgets::button::style(&attach, Kind::Ghost, Size::Small);
        let words = gtk::Label::new(None);
        words.add_css_class("dim-label");
        words.add_css_class("focus-compose-words");
        words.set_hexpand(true);
        words.set_xalign(1.0);
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        footer.add_css_class("focus-compose-footer");
        footer.append(&attach);
        footer.append(&words);

        let frame = Rc::new(Frame {
            header,
            footer,
            labels_row,
            close,
            title,
            saved,
            detach,
            send_later,
            send,
            labels,
            from_thread,
            attach,
            words,
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
        self.send_later.set_create_popup_func(|button| {
            let menu = gio::Menu::new();
            for (label, when) in postio_ui::schedule::schedule_presets(Local::now()) {
                let item = gio::MenuItem::new(Some(label), None);
                item.set_action_and_target_value(
                    Some("focus-send-later.choose"),
                    Some(&when.with_timezone(&Utc).timestamp_millis().to_variant()),
                );
                menu.append_item(&item);
            }
            button.set_popover(Some(&gtk::PopoverMenu::from_model(Some(&menu))));
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

        // What the header and the footer say follows the draft.
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
                frame.saved.set_text("");
                frame.follow(&composer.draft());
                frame.draw_labels(&composer);
            }
        });
    }

    /// Redraw the keycaps from `keymap`.
    pub fn set_keymap(&self, keymap: &Keymap) {
        let key = |command| hints::key(keymap, command);
        self.close.set_child(Some(&keyhint::labelled(
            "Close",
            key(CommandId::Back).as_deref(),
        )));
        self.send.set_child(Some(&keyhint::labelled(
            "Send",
            key(CommandId::Send).as_deref(),
        )));
        self.attach.set_child(Some(&keyhint::labelled(
            "Attach",
            key(CommandId::AttachFile).as_deref(),
        )));
    }

    /// The heading and the footer, for `draft` as it stands.
    fn follow(&self, draft: &Draft) {
        self.title.set_text(title(draft.kind));
        self.words.set_text(&summary(draft));
    }

    /// A save landed at `at`.
    pub fn saved(&self, at: DateTime<Utc>) {
        self.saved.set_text(&saved_at(at));
    }

    /// Open the Send later menu, as its key does.
    pub fn pop_send_later(&self) {
        self.send_later.popup();
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
            let remove = gtk::Button::from_icon_name("window-close-symbolic");
            remove.add_css_class("flat");
            remove.add_css_class("circular");
            let spoken = format!("Remove the label {}", label.name);
            remove.set_tooltip_text(Some(&spoken));
            remove.update_property(&[gtk::accessible::Property::Label(&spoken)]);
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
    pub fn commands() -> [CommandId; 5] {
        [
            CommandId::Back,
            CommandId::Send,
            CommandId::ScheduleSend,
            CommandId::AttachFile,
            CommandId::DetachComposer,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_model::AccountId;

    #[test]
    fn the_footer_counts_the_words_that_will_be_sent_and_says_which_kind() {
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
    fn the_header_names_a_composition_as_the_screens_do() {
        assert_eq!(title(DraftKind::New), "New message");
        assert_eq!(title(DraftKind::ReplyAll), "Reply to all");
    }
}
