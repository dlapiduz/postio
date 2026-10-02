//! Compose, reply and forward (spec 007 US3; screens 05 and 06): the classic
//! app's composer, whole, in a dialog over the list.
//!
//! Focus has no composer of its own (FR-050). What is here is the frame the
//! screens draw around it, the dialog it is hosted in ([`host`]), and its
//! seams answered through Focus's client ([`seams`]). Rich text,
//! attachments, identities, signatures, drafts, send later, the outbox and
//! detaching to a window of its own are all the composer's.

mod frame;
mod host;
mod seams;

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use postio_client::Client;
use postio_core::{CommandId, Keymap};
use postio_model::ids::AccountId;
use postio_ui::focus_dialog;
use postio_widgets::composer::Composer;
use postio_widgets::widgets::pickers::{Picker, When, WhenPicker};

pub use frame::{remind_meaning, remind_words, saved_at, subtitle, summary, title};
pub use seams::Current;

use crate::window::FocusWindow;

/// The dialog's name, so the window can tell it from another dialog.
pub const NAME: &str = "focus-compose";

/// Focus's composition: one composer, in one dialog.
pub struct Compose {
    composer: Composer,
    frame: Rc<frame::Frame>,
    host: Rc<host::DialogHost>,
    resume: seams::Resume,
    /// "Remind if no reply", opened at the action row by `mod+h` or a click
    /// (US3 scenario 5): the remind picker the row uses, choosing for the
    /// draft instead of for a conversation.
    remind: Rc<WhenPicker>,
}

impl Compose {
    /// The composer, mounted in its dialog over `window`, writing as
    /// `account` through `client`; `current` names the message `e`, `E` and
    /// `f` answer.
    pub fn new(
        window: &FocusWindow,
        client: &Client,
        account: AccountId,
        current: Current,
    ) -> Rc<Self> {
        let keymap = window.keymap();
        let composer = Composer::new();
        // The frame carries the heading and the verbs; the composer's own
        // rows for them would say everything twice.
        composer.set_framed(true);
        // Recipients as chips, name and address (T079, screens 05 and 06).
        composer.set_recipient_chips(true);
        composer.set_keymap(&keymap);
        let frame = frame::Frame::new(&composer, &keymap);
        composer.add_field_row(&frame.labels_row);

        // The body draws on the dialog's surface, in its ink, from the
        // column's edge (T221), as the open message's body does.
        composer.flow_in_column();

        // One column for the fields, the toolbar and the body: the message
        // dialog's app colours column, centred (T207, T221).
        let slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
        slot.set_vexpand(true);
        let column = adw::Clamp::builder()
            .child(&slot)
            .maximum_size(focus_dialog::COLUMN_APP_COLOURS)
            .tightening_threshold(focus_dialog::COLUMN_APP_COLOURS)
            .vexpand(true)
            .build();
        column.add_css_class("focus-compose-column");
        let layout = adw::ToolbarView::new();
        layout.add_css_class("focus-compose-surface");
        layout.add_top_bar(&frame.header);
        layout.add_top_bar(&frame.actions);
        layout.set_content(Some(&column));
        // The message dialog's size rule (T205), fitted to the window as it
        // is presented and as it is resized (`host`).
        let dialog = adw::Dialog::builder()
            .content_width(focus_dialog::dialog_width(host::WINDOW.0))
            .content_height(focus_dialog::dialog_height(host::WINDOW.1))
            .child(&layout)
            .build();
        dialog.set_widget_name(NAME);
        dialog.add_css_class("focus-compose");

        let host = Rc::new(host::DialogHost::new(window, dialog.clone(), slot, column));
        composer.mount_on(Rc::clone(&host) as Rc<dyn postio_widgets::composer::ComposerHost>);
        // The dialog's own ways out -- Escape reaching it, a click outside
        // -- mean what `Esc` means: close, keeping the draft.
        dialog.set_can_close(false);
        dialog.connect_close_attempt(glib::clone!(
            #[weak]
            composer,
            move |_| composer.dispatch(CommandId::Back)
        ));
        let resume = seams::wire(&composer, &frame, client, account, current);
        let remind = WhenPicker::new(&keymap, When::Remind);
        remind.connect_chosen(glib::clone!(
            #[weak]
            composer,
            move |at| composer.set_remind_at(Some(at.to_utc()))
        ));
        let compose = Rc::new(Compose {
            composer,
            frame,
            host,
            resume,
            remind,
        });
        let weak = Rc::downgrade(&compose);
        compose.frame.remind.connect_clicked(move |_| {
            if let Some(compose) = weak.upgrade() {
                compose.dispatch(CommandId::RemindIfNoReply);
            }
        });
        compose
    }

    /// The remind picker at the action row, while it is open.
    pub fn open_picker(&self) -> Option<Rc<Picker>> {
        let picker = self.remind.picker();
        picker.is_open().then(|| Rc::clone(picker))
    }

    /// Open the remind picker at its verb in the action row, naming the draft.
    fn open_remind(&self) {
        let subject = self.composer.draft().subject;
        let target = if subject.trim().is_empty() {
            "This message".to_owned()
        } else {
            subject
        };
        self.remind
            .open(&self.frame.remind, None, &target, chrono::Local::now());
    }

    /// The composer.
    pub fn composer(&self) -> &Composer {
        &self.composer
    }

    /// The dialog, while it is over the window.
    pub fn dialog(&self) -> Option<adw::Dialog> {
        self.host.showing().then(|| self.host.dialog.clone())
    }

    /// Run `id`, as a key or a control asked: the composer's own verbs, and
    /// Send later's menu, which the frame draws.
    pub fn dispatch(&self, id: CommandId) {
        match id {
            CommandId::ScheduleSend if self.composer.is_open() => self.frame.pop_send_later(),
            CommandId::RemindIfNoReply if self.composer.is_open() => self.open_remind(),
            _ => self.host.run(id),
        }
    }

    /// Open the draft behind the Drafts row `message` for editing.
    pub fn open_draft(&self, message: postio_model::MessageId) {
        (self.resume)(message);
    }

    /// Take `keymap` as the keys in force.
    pub fn set_keymap(&self, keymap: &Keymap) {
        self.composer.set_keymap(keymap);
        self.frame.set_keymap(keymap);
        self.remind.set_keymap(keymap);
    }

    /// Start the editing surface before anybody asks to write (#1216).
    pub fn warm(&self) {
        self.composer.warm();
    }

    /// The commands the dialog's own controls run, for registry parity.
    pub fn controls() -> Vec<CommandId> {
        let mut commands = frame::Frame::commands().to_vec();
        // The composer's formatting toolbar, which it draws itself.
        commands.extend([
            CommandId::Bold,
            CommandId::Italic,
            CommandId::BulletList,
            CommandId::NumberedList,
            CommandId::QuoteBlock,
            CommandId::InsertLink,
            CommandId::InsertImage,
            CommandId::CopyFields,
        ]);
        commands
    }
}
