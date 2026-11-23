//! Compose, reply and forward: the shared composer in a dialog over the list. Focus has no composer
//! of its own;
//! this is the frame, the host ([`host`]) and the seams ([`seams`]).

mod frame;
mod host;
mod seams;
pub mod thread;

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use postio_client::Client;
use postio_core::{CommandId, Keymap};
use postio_model::ids::AccountId;
use postio_ui::focus_dialog;
use postio_widgets::composer::Composer;
use postio_widgets::widgets::pickers::{Picker, When, WhenPicker};

pub use postio_ui::compose::{remind_meaning, remind_words, saved_at, subtitle, summary, title};
pub use seams::Current;

use crate::window::FocusWindow;

pub const NAME: &str = "focus-compose";

pub struct Compose {
    composer: Composer,
    frame: Rc<frame::Frame>,
    host: Rc<host::DialogHost>,
    resume: seams::Resume,
    resume_draft: seams::ResumeDraft,
    account: AccountId,
    remind: Rc<WhenPicker>,
    pub thread: Rc<thread::ThreadParts>,
}

impl Compose {
    pub fn new(
        window: &FocusWindow,
        client: &Client,
        account: AccountId,
        current: Current,
    ) -> Rc<Self> {
        let keymap = window.keymap();
        let composer = Composer::new();
        // The frame carries the heading and verbs; the composer's own rows would say everything
        // twice.
        composer.set_framed(true);
        composer.set_recipient_chips(true);
        composer.set_keymap(&keymap);
        let frame = frame::Frame::new(&composer, &keymap);
        composer.add_field_row(&frame.labels_row);

        composer.flow_in_column();

        let slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
        slot.set_vexpand(true);
        let column = adw::Clamp::builder()
            .child(&slot)
            .maximum_size(focus_dialog::column_width(
                focus_dialog::DIALOG_MAX,
                postio_body::treatment::Treatment::AppColours,
            ))
            .tightening_threshold(focus_dialog::column_width(
                focus_dialog::DIALOG_MAX,
                postio_body::treatment::Treatment::AppColours,
            ))
            .vexpand(true)
            .build();
        column.add_css_class("focus-compose-column");
        let layout = adw::ToolbarView::new();
        layout.add_css_class("focus-compose-surface");
        layout.add_top_bar(&frame.header);
        layout.add_top_bar(&frame.actions);
        let thread = thread::ThreadParts::new();
        layout.add_top_bar(&thread.older);
        layout.add_top_bar(&thread.chooser);
        layout.set_content(Some(&column));
        let dialog = adw::Dialog::builder()
            .content_width(focus_dialog::dialog_width(host::WINDOW.0))
            .content_height(focus_dialog::dialog_height(host::WINDOW.1))
            .child(&layout)
            .build();
        dialog.set_widget_name(NAME);
        dialog.add_css_class("focus-compose");

        let host = Rc::new(host::DialogHost::new(
            window,
            dialog.clone(),
            slot,
            column,
            layout.clone().upcast(),
        ));
        composer.mount_on(Rc::clone(&host) as Rc<dyn postio_widgets::composer::ComposerHost>);
        composer.below_body().append(&thread.forwarded);
        host.slot.append(&thread.newer);
        // Escape reaching the dialog, or a click outside, mean `Esc`: close, keeping the draft.
        dialog.set_can_close(false);
        dialog.connect_close_attempt(glib::clone!(
            #[weak]
            composer,
            move |_| composer.dispatch(CommandId::Back)
        ));
        // The composer holds its host and the host's slot holds the composer: a cycle that outlives
        // the window unless cut on destroy.
        window.connect_destroy(glib::clone!(
            #[weak]
            composer,
            move |_| composer.unmount()
        ));
        let toasting = window.downgrade();
        let say: seams::Say = Rc::new(move |sentence, undo| {
            if let Some(window) = toasting.upgrade() {
                window.show_removable(sentence, move || undo());
            }
        });
        // Escape from a composition that holds writing keeps it and says so: silence reads as a
        // draft thrown away.
        let keeping = window.downgrade();
        composer.connect_closed(move |outcome| {
            if outcome != postio_widgets::composer::Closing::Keep {
                return;
            }
            if let Some(window) = keeping.upgrade() {
                let key = postio_ui::hints::key(&window.keymap(), CommandId::GoToDrafts);
                window.say(&postio_ui::compose::kept_note(key.as_deref()));
            }
        });
        composer.connect_opened(glib::clone!(
            #[weak]
            thread,
            move || thread.focus_chooser()
        ));
        let (resume, resume_draft) = seams::wire(&composer, &frame, client, account, current, say);
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
            resume_draft,
            account,
            remind,
            thread,
        });
        let weak = Rc::downgrade(&compose);
        compose.frame.remind.connect_clicked(move |_| {
            if let Some(compose) = weak.upgrade() {
                compose.dispatch(CommandId::RemindIfNoReply);
            }
        });
        compose
    }

    pub fn open_picker(&self) -> Option<Rc<Picker>> {
        let picker = self.remind.picker();
        picker.is_open().then(|| Rc::clone(picker))
    }

    fn open_remind(&self) {
        let subject = self.composer.draft().subject;
        let target = if subject.trim().is_empty() {
            "This message".to_owned()
        } else {
            subject
        };
        self.remind
            .open(&self.frame.remind, None, &target, postio_ui::clock::now());
    }

    pub fn sync_forwarded(&self, cap: Option<&str>) {
        if self.thread.quoted_expanded() {
            self.composer.unfold_forwarded();
            self.thread.show_forwarded(&[], None);
        } else {
            let rows = self.composer.fold_forwarded();
            self.thread.show_forwarded(&rows, cap);
        }
    }

    pub fn composer(&self) -> &Composer {
        &self.composer
    }

    pub fn dialog(&self) -> Option<adw::Dialog> {
        (self.host.showing() && !self.host.in_pane()).then(|| self.host.dialog.clone())
    }

    pub fn is_showing(&self) -> bool {
        self.host.showing()
    }

    pub fn in_pane(&self) -> bool {
        self.host.showing() && self.host.in_pane()
    }

    pub fn place(&self, slot: Option<&gtk::Box>) {
        self.host.place(slot);
    }

    pub fn dispatch(&self, id: CommandId) {
        match id {
            CommandId::ScheduleSend if self.composer.is_open() => self.frame.pop_send_later(),
            CommandId::RemindIfNoReply if self.composer.is_open() => self.open_remind(),
            _ => self.host.run(id),
        }
    }

    pub fn open_draft(&self, message: postio_model::MessageId) {
        (self.resume)(message);
    }

    pub fn open_thread_draft(&self, draft: postio_model::DraftId) {
        (self.resume_draft)(draft);
    }

    /// One composition at a time: a link arriving mid-composition puts the keyboard back in the
    /// open draft rather than replacing what was typed.
    pub fn open_mailto(&self, mailto: postio_model::mailto::Mailto) {
        if self.composer.is_open() {
            tracing::info!("a mailto link arrived while a composition was open; kept the open one");
        }
        self.composer.open(mailto.into_draft(self.account));
    }

    pub fn set_keymap(&self, keymap: &Keymap) {
        self.composer.set_keymap(keymap);
        self.frame.set_keymap(keymap);
        self.remind.set_keymap(keymap);
    }

    pub fn warm(&self) {
        self.composer.warm();
    }

    pub fn controls() -> Vec<CommandId> {
        let mut commands = frame::Frame::commands().to_vec();
        commands.extend([
            CommandId::Bold,
            CommandId::Italic,
            CommandId::Heading1,
            CommandId::BulletList,
            CommandId::NumberedList,
            CommandId::QuoteBlock,
            CommandId::InsertLink,
            // The source view's toggle, at the toolbar's right end.
            CommandId::ToggleMarkdownSource,
            CommandId::InsertImage,
            CommandId::CopyFields,
            CommandId::ShowNewer,
            CommandId::ReplyToLatest,
            CommandId::ExpandQuoted,
            CommandId::ForwardWhat1,
            CommandId::ForwardWhat2,
            CommandId::ForwardWhat3,
        ]);
        commands
    }
}
