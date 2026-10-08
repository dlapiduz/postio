//! Writing and replying (research R2, slice 7; spec 007 US3, screens 05
//! and 06).
//!
//! Moved from `postio-gtk`'s window (`refuses_reply`'s arm, the draft a row
//! opens, `offered_on_open_draft`, `settle_send` and the add-account offer)
//! and from `postio-widgets`' composer (its debounced autosave, and the
//! generation that keeps one composition's save off the next one's row).
//! The draft's words stay the toolkit's -- the fields and the editing
//! surface hold them, and the frontend saves them -- so what is here is
//! when: when the composer opens and what it answers, when what is written
//! is saved, and what is said when a composition ends.
//!
//! A *composition* is one draft's time in the composer, numbered from 1.
//! It begins when the composer is put on the stack with something to write
//! and ends when it leaves -- Esc, its window's close button, another
//! secondary window replacing it on the Mac (M4), or another composition
//! taking its place. An edit arms the autosave's one timer ([`AUTOSAVE`]);
//! each later edit re-arms it, so a burst of typing is one save. A
//! composition that ends having been written in is saved at once, and once
//! that save has landed the toast says the draft was kept ("Draft saved
//! locally 16:12"). One untouched closes without a word, as GTK's did.

use std::time::Duration;

use postio_core::CommandId;
use postio_model::{DraftId, DraftState, MessageId};
use postio_ui::focus_target;

use crate::cursor::Rows;
use crate::feed::Step;
use crate::{FocusController, Intent, Request, SurfaceKind, ToastKind};

/// How long the composer waits after an edit before saving what is
/// written: a burst of keystrokes is one save, and a crash loses at most
/// this much of a sentence (`postio-widgets`' `AUTOSAVE_DEBOUNCE`).
pub const AUTOSAVE: Duration = Duration::from_millis(1500);

/// What the composer is asked to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ComposerKind {
    /// A new message, answering nothing.
    New,
    /// A reply to the message's sender.
    Reply,
    /// A reply to everyone on the message.
    ReplyAll,
    /// The message, forwarded.
    Forward,
    /// The draft behind the message, to go on writing.
    Draft,
}

/// One draft's time in the composer.
#[derive(Debug, Clone, Copy)]
struct Composition {
    /// Which, counting from 1: what [`Intent::SaveDraft`] names.
    number: u64,
    /// Whether anything has been written in it: what makes ending it a
    /// save, and the save a toast.
    written: bool,
    /// The autosave's timer, while an edit is waiting out the quiet period.
    pending: Option<u64>,
}

/// The composer, as the controller keeps it.
#[derive(Debug, Default)]
pub(crate) struct Compose {
    /// The composition in the composer, while it is open.
    open: Option<Composition>,
    /// Compositions begun so far.
    begun: u64,
    /// Timers handed out so far: a token is never reused.
    timers: u64,
    /// Compositions that ended written, whose last save has not landed:
    /// its landing is said in the toast.
    ending: Vec<u64>,
    /// The accounts are known and there are none: nothing to write from.
    no_account: bool,
}

impl Compose {
    /// Whether the accounts Focus writes from, as last said, are none.
    pub(crate) fn set_no_account(&mut self, none: bool) {
        self.no_account = none;
    }
}

/// Whether `id` is one of the verbs this slice answers.
pub(crate) fn compose_key(id: CommandId) -> bool {
    matches!(
        id,
        CommandId::Compose
            | CommandId::Reply
            | CommandId::ReplyAll
            | CommandId::Forward
            | CommandId::CancelSend
            | CommandId::RetrySend
            | CommandId::MarkSent
    )
}

impl FocusController {
    /// `c`, `e`, `E`, `f` and the send verbs, from wherever they were
    /// pressed; `None` when `id` is none of them, or the surface on top has
    /// nothing for them to aim at and its own rules apply.
    pub(crate) fn compose_verb(&mut self, id: CommandId, rows: &dyn Rows) -> Option<Vec<Step>> {
        let kind = match id {
            CommandId::Compose => return Some(self.write(ComposerKind::New, None)),
            CommandId::Reply => ComposerKind::Reply,
            CommandId::ReplyAll => ComposerKind::ReplyAll,
            CommandId::Forward => ComposerKind::Forward,
            CommandId::CancelSend | CommandId::RetrySend | CommandId::MarkSent => {
                let message = self.aimed_for_writing(rows)?;
                return Some(self.settle(id, message, rows));
            }
            _ => return None,
        };
        let message = self.aimed_for_writing(rows)?;
        // Not on mail still on its way out (#1749): it has no other party
        // yet, and answering it would be replying to oneself.
        if kind != ComposerKind::Forward
            && focus_target::refuses_reply(self.send_state_of(message, rows))
        {
            return Some(vec![Step::Show(Intent::Toast {
                text: focus_target::NO_REPLY_TO_OUTGOING.to_owned(),
                kind: ToastKind::Notice,
            })]);
        }
        Some(self.write(kind, Some(message)))
    }

    /// A command while the composer is on top: Esc ends the composition,
    /// `c`, `e`, `E` and `f` begin another, and every other key is the
    /// composer's own (its fields', its marks', Send's).
    pub(crate) fn composer_command(&mut self, id: CommandId, rows: &dyn Rows) -> Vec<Step> {
        if id == CommandId::Back {
            self.surfaces.dismiss(SurfaceKind::Composer);
            let mut steps = vec![Step::Show(Intent::CloseSurface(SurfaceKind::Composer))];
            if !self.has_surface() {
                steps.push(Step::Show(Intent::KeyboardHome));
            }
            steps.extend(self.end_composition());
            return steps;
        }
        self.compose_verb(id, rows).unwrap_or_default()
    }

    /// Whether the composer, on top, answers `id` here rather than leaving
    /// it to the toolkit's composer.
    pub(crate) fn composer_answers(id: CommandId) -> bool {
        id == CommandId::Back || compose_key(id)
    }

    /// The message `e`, `E`, `f` and the send verbs aim at: the message open
    /// over the list, the email open in the digest's window, or the
    /// cursor's row (`postio_ui::focus_target::aimed_message`). `None` over
    /// a surface that is about something else -- the bar, a picker,
    /// Filtered, capture -- whose own rules then apply.
    fn aimed_for_writing(&self, rows: &dyn Rows) -> Option<MessageId> {
        match self.surfaces.top() {
            Some(SurfaceKind::Message) => self.surfaces.reading(),
            Some(SurfaceKind::Digest) => self.digest_email(),
            None | Some(SurfaceKind::Composer) => {
                let row = rows.facts(self.cursor.position()?)?;
                (!row.digest).then_some(row.id)
            }
            _ => None,
        }
    }

    /// The send state of `message`'s row, when the list holds its row: a
    /// draft being written, on its way, stopped; `None` for received mail.
    fn send_state_of(&self, message: MessageId, rows: &dyn Rows) -> Option<DraftState> {
        let row = rows.row(rows.position_of(message)?)?;
        row.as_conversation()?.summary.representative.send_state
    }

    /// Whether the open message is a draft on its way or stopped whose
    /// action row offers `id` (T239).
    fn offered_on_open_draft(&self, id: CommandId, rows: &dyn Rows) -> bool {
        let Some(reading) = self.surfaces.reading() else {
            return false;
        };
        postio_ui::focus_dialog::send_verbs(self.send_state_of(reading, rows))
            .is_some_and(|verbs| verbs.contains(&id))
    }

    /// `Return` on the open message: Edit, when it is a draft on its way or
    /// stopped that offers it; `None` for mail that opens to be read.
    pub(crate) fn edit_open_draft(&mut self, rows: &dyn Rows) -> Option<Vec<Step>> {
        if !self.offered_on_open_draft(CommandId::OpenMessage, rows) {
            return None;
        }
        let reading = self.surfaces.reading()?;
        Some(self.write(ComposerKind::Draft, Some(reading)))
    }

    /// Open the composer on `kind`, answering `message`: the composition in
    /// it ends first, saved, and on the Mac the composer takes the place of
    /// the secondary window that was open (M4). A new message with no
    /// account to write from offers to add one instead (T172).
    pub(crate) fn write(&mut self, kind: ComposerKind, message: Option<MessageId>) -> Vec<Step> {
        if kind == ComposerKind::New && self.compose.no_account {
            return vec![Step::Show(Intent::Toast {
                text: focus_target::NO_ACCOUNT_TO_WRITE_FROM.to_owned(),
                kind: ToastKind::Offer {
                    label: focus_target::ADD_ACCOUNT.to_owned(),
                    command: CommandId::AddAccount,
                },
            })];
        }
        // Before the composer is refilled: what it holds is the ending
        // composition's, and saved after the refill it would be the new one.
        let mut steps = self.end_composition();
        // A draft is written, not read: the message it was open in closes,
        // on Linux too, as GTK's window closed its reading dialog.
        if kind == ComposerKind::Draft && self.surfaces.dismiss(SurfaceKind::Message) {
            steps.push(Step::Show(Intent::CloseSurface(SurfaceKind::Message)));
        }
        steps.extend(
            self.surfaces
                .opened(SurfaceKind::Composer, self.policy.caps.stacking),
        );
        self.begin_composition();
        steps.push(Step::Show(Intent::Composer { kind, message }));
        steps
    }

    /// The frontend opened the composer itself (a `mailto:` link): a
    /// composition begins, the one it replaces ending first.
    pub(crate) fn composer_opened(&mut self) -> Vec<Step> {
        let steps = self.end_composition();
        self.begin_composition();
        steps
    }

    fn begin_composition(&mut self) {
        self.compose.begun += 1;
        self.compose.open = Some(Composition {
            number: self.compose.begun,
            written: false,
            pending: None,
        });
    }

    /// The composition in the composer ends: saved now when anything was
    /// written in it, its pending timer forgotten.
    fn end_composition(&mut self) -> Vec<Step> {
        let Some(ending) = self.compose.open.take() else {
            return Vec::new();
        };
        if !ending.written {
            return Vec::new();
        }
        self.compose.ending.push(ending.number);
        vec![Step::Show(Intent::SaveDraft {
            composition: ending.number,
        })]
    }

    /// The composer left the stack some way the controller did not take it
    /// off -- its window's close button, another window replacing it (M4):
    /// its composition ends, saved.
    pub(crate) fn composer_gone(&mut self) -> Vec<Step> {
        if self.compose.open.is_none() || self.surfaces.has(SurfaceKind::Composer) {
            return Vec::new();
        }
        self.end_composition()
    }

    /// Something was written: the autosave waits [`AUTOSAVE`] from now.
    pub(crate) fn composer_edited(&mut self) -> Vec<Step> {
        let Some(open) = self.compose.open.as_mut() else {
            return Vec::new();
        };
        open.written = true;
        self.compose.timers += 1;
        let token = self.compose.timers;
        open.pending = Some(token);
        vec![Step::Timer {
            token,
            after: AUTOSAVE,
        }]
    }

    /// A timer ran out: the autosave's, when it is the one still armed.
    pub(crate) fn timer(&mut self, token: u64) -> Vec<Step> {
        let Some(open) = self.compose.open.as_mut() else {
            return Vec::new();
        };
        if open.pending != Some(token) {
            return Vec::new();
        }
        open.pending = None;
        vec![Step::Show(Intent::SaveDraft {
            composition: open.number,
        })]
    }

    /// A save landed. An ended composition's is said: kept, at what time.
    /// A failure is said whichever composition it was.
    pub(crate) fn draft_saved(
        &mut self,
        composition: u64,
        saved: Result<bool, String>,
    ) -> Vec<Step> {
        let ended = match self.compose.ending.iter().position(|c| *c == composition) {
            Some(at) => {
                self.compose.ending.remove(at);
                true
            }
            None => false,
        };
        match saved {
            Ok(true) if ended => {
                let now = self.pickers.clock.unwrap_or_else(postio_ui::clock::now);
                vec![Step::Show(Intent::Toast {
                    text: postio_ui::compose::saved_at(now.to_utc()),
                    kind: ToastKind::Completed {
                        undoable: false,
                        seconds: None,
                    },
                })]
            }
            Ok(_) => Vec::new(),
            Err(sentence) => vec![Step::Show(Intent::Toast {
                text: sentence,
                kind: ToastKind::Notice,
            })],
        }
    }

    /// Cancel, retry or settle the send of the draft behind `message`
    /// (T239): asked which draft first, so the host acts on that one. One
    /// the open message offers moves it out of the list it was opened
    /// from, so it closes and the keyboard goes home; the next row -- a
    /// draft being written, often -- is not opened in its place.
    fn settle(&mut self, id: CommandId, message: MessageId, rows: &dyn Rows) -> Vec<Step> {
        let mut steps = Vec::new();
        if self.surfaces.reading() == Some(message) && self.offered_on_open_draft(id, rows) {
            self.surfaces.dismiss(SurfaceKind::Message);
            steps.push(Step::Show(Intent::CloseSurface(SurfaceKind::Message)));
            if !self.has_surface() {
                steps.push(Step::Show(Intent::KeyboardHome));
            }
        }
        steps.push(Step::Ask(Request::DraftBehind {
            message,
            command: id,
        }));
        steps
    }

    /// Which draft is behind the message a send verb aimed at: settle it,
    /// or say that it is no draft being sent.
    pub(crate) fn draft_behind(&mut self, command: CommandId, draft: Option<DraftId>) -> Vec<Step> {
        match draft {
            Some(draft) => vec![Step::Ask(Request::Post(focus_target::settle_command(
                command,
                Some(draft),
            )))],
            None => vec![Step::Show(Intent::Toast {
                text: focus_target::NOT_BEING_SENT.to_owned(),
                kind: ToastKind::Notice,
            })],
        }
    }
}
