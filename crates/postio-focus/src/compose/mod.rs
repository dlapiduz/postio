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
use postio_widgets::composer::Composer;

pub use frame::{saved_at, summary, title};
pub use seams::Current;

use crate::window::FocusWindow;

/// The size screens 05 and 06 draw the dialog at.
pub const SIZE: (i32, i32) = (980, 820);

/// The dialog's name, so the window can tell it from another dialog.
pub const NAME: &str = "focus-compose";

/// Focus's composition: one composer, in one dialog.
pub struct Compose {
    composer: Composer,
    frame: Rc<frame::Frame>,
    host: Rc<host::DialogHost>,
    resume: seams::Resume,
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

        let slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
        slot.set_vexpand(true);
        let layout = adw::ToolbarView::new();
        layout.add_top_bar(&frame.header);
        layout.set_content(Some(&slot));
        layout.add_bottom_bar(&frame.footer);
        let dialog = adw::Dialog::builder()
            .content_width(SIZE.0)
            .content_height(SIZE.1)
            .child(&layout)
            .build();
        dialog.set_widget_name(NAME);
        dialog.add_css_class("focus-compose");

        let host = Rc::new(host::DialogHost::new(window, dialog.clone(), slot));
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
        Rc::new(Compose {
            composer,
            frame,
            host,
            resume,
        })
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
