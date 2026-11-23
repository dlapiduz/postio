//! Writing and replying: when the composer opens, when what is written is saved (one autosave
//! timer, re-armed by each edit), and what is said when a composition ends.

use std::time::Duration;

use postio_core::CommandId;
use postio_model::thread_view::ThreadView;
use postio_model::{DraftId, DraftState, ForwardForm, Forwarded, MessageId};
use postio_ui::compose::{self as words, OlderReplyFacts};
use postio_ui::focus_target;

use crate::cursor::Rows;
use crate::feed::Step;
use crate::{DraftEdit, FocusController, Intent, Request, SurfaceKind, ToastKind};

/// A burst of keystrokes is one save; a crash loses at most this much of a sentence.
pub const AUTOSAVE: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ComposerKind {
    New,
    Reply,
    ReplyAll,
    Forward {
        form: postio_model::ForwardForm,
    },
    Draft,
    /// An unsent reply shown in a thread: it has no message of its own (`Intent::Composer`'s
    /// `message` is `None`).
    ThreadDraft {
        draft: DraftId,
    },
}

#[derive(Debug, Clone, Copy)]
struct Composition {
    number: u64,
    written: bool,
    pending: Option<u64>,
}

#[derive(Debug, Clone)]
struct Chooser {
    form: ForwardForm,
    messages: usize,
    thread: Option<postio_model::ThreadId>,
    /// The composer opened on a form other than This and the engine has
    /// not built it yet: a refusal falls back to This.
    opening: bool,
    of: Option<(String, chrono::DateTime<chrono::Utc>)>,
}

#[derive(Debug, Clone)]
struct Older {
    facts: OlderReplyFacts,
    newer: Vec<MessageId>,
    shown: bool,
}

#[derive(Debug, Default)]
struct ThreadCompose {
    chooser: Option<Chooser>,
    older: Option<Older>,
    quoted_expanded: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Compose {
    next_form: Option<ForwardForm>,
    thread: ThreadCompose,
    open: Option<Composition>,
    begun: u64,
    timers: u64,
    ending: Vec<u64>,
    no_account: bool,
}

impl Compose {
    pub(crate) fn forward_as(&mut self, form: Option<ForwardForm>) {
        self.next_form = form;
    }

    pub(crate) fn set_no_account(&mut self, none: bool) {
        self.no_account = none;
    }
}

pub(crate) fn compose_key(id: CommandId) -> bool {
    matches!(
        id,
        CommandId::Compose
            | CommandId::Reply
            | CommandId::ReplyAll
            | CommandId::Forward
            | CommandId::ReplyToLatest
            | CommandId::CancelSend
            | CommandId::RetrySend
            | CommandId::MarkSent
    )
}

impl FocusController {
    /// `None` when `id` is none of them, or the surface on top has nothing for them to aim at.
    pub(crate) fn compose_verb(&mut self, id: CommandId, rows: &dyn Rows) -> Option<Vec<Step>> {
        let kind = match id {
            CommandId::Compose => return Some(self.write(ComposerKind::New, None)),
            CommandId::Reply => ComposerKind::Reply,
            CommandId::ReplyAll => ComposerKind::ReplyAll,
            CommandId::Forward => ComposerKind::Forward {
                form: self.compose.next_form.unwrap_or_default(),
            },
            CommandId::ReplyToLatest => {
                let message = self.aimed_for_writing(rows)?;
                if focus_target::refuses_reply(self.send_state_of(message, rows)) {
                    return Some(vec![Step::Show(Intent::Toast {
                        text: focus_target::NO_REPLY_TO_OUTGOING.to_owned(),
                        kind: ToastKind::Notice,
                    })]);
                }
                let latest = self.latest_received(message).unwrap_or(message);
                return Some(self.write_in(ComposerKind::Reply, Some(latest), Some(rows)));
            }
            CommandId::CancelSend | CommandId::RetrySend | CommandId::MarkSent => {
                let message = self.aimed_for_writing(rows)?;
                return Some(self.settle(id, message, rows));
            }
            _ => return None,
        };
        let message = self.aimed_for_writing(rows)?;
        // Not on mail still on its way out (#1749): it has no other party
        // yet, and answering it would be replying to oneself.
        if !matches!(kind, ComposerKind::Forward { .. })
            && focus_target::refuses_reply(self.send_state_of(message, rows))
        {
            return Some(vec![Step::Show(Intent::Toast {
                text: focus_target::NO_REPLY_TO_OUTGOING.to_owned(),
                kind: ToastKind::Notice,
            })]);
        }
        Some(self.write_in(kind, Some(message), Some(rows)))
    }

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
        match id {
            CommandId::ForwardWhat1 => return self.forward_form_chosen(ForwardForm::This),
            CommandId::ForwardWhat2 => return self.forward_form_chosen(ForwardForm::Thread),
            CommandId::ForwardWhat3 => return self.forward_form_chosen(ForwardForm::Attachments),
            CommandId::ReplyToLatest => return self.reply_to_latest_asked(),
            CommandId::ShowNewer => return self.show_newer_toggled(),
            CommandId::ExpandQuoted => return self.quoted_toggled(),
            _ => {}
        }
        self.compose_verb(id, rows).unwrap_or_default()
    }

    pub(crate) fn composer_answers(id: CommandId) -> bool {
        id == CommandId::Back
            || compose_key(id)
            || matches!(
                id,
                CommandId::ForwardWhat1
                    | CommandId::ForwardWhat2
                    | CommandId::ForwardWhat3
                    | CommandId::ShowNewer
                    | CommandId::ExpandQuoted
            )
    }

    /// `None` over a surface about something else (the bar, a picker, Filtered, capture), whose own
    /// rules apply.
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

    fn send_state_of(&self, message: MessageId, rows: &dyn Rows) -> Option<DraftState> {
        let row = rows.row(rows.position_of(message)?)?;
        row.as_conversation()?.summary.representative.send_state
    }

    fn offered_on_open_draft(&self, id: CommandId, rows: &dyn Rows) -> bool {
        let Some(reading) = self.surfaces.reading() else {
            return false;
        };
        postio_ui::focus_dialog::send_verbs(self.send_state_of(reading, rows))
            .is_some_and(|verbs| verbs.contains(&id))
    }

    pub(crate) fn edit_open_draft(&mut self, rows: &dyn Rows) -> Option<Vec<Step>> {
        if !self.offered_on_open_draft(CommandId::OpenMessage, rows) {
            return None;
        }
        let reading = self.surfaces.reading()?;
        Some(self.write(ComposerKind::Draft, Some(reading)))
    }

    /// The composition in it ends first, saved; on the Mac the composer replaces the open secondary
    /// window (M4).
    pub(crate) fn write(&mut self, kind: ComposerKind, message: Option<MessageId>) -> Vec<Step> {
        self.write_in(kind, message, None)
    }

    fn write_in(
        &mut self,
        kind: ComposerKind,
        message: Option<MessageId>,
        rows: Option<&dyn Rows>,
    ) -> Vec<Step> {
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
        // A draft is written, not read: the message it was open in closes.
        if matches!(kind, ComposerKind::Draft | ComposerKind::ThreadDraft { .. })
            && self.surfaces.dismiss(SurfaceKind::Message)
        {
            steps.push(Step::Show(Intent::CloseSurface(SurfaceKind::Message)));
        }
        steps.extend(
            self.surfaces
                .opened(SurfaceKind::Composer, self.policy.caps.stacking),
        );
        self.begin_composition();
        steps.push(Step::Show(Intent::Composer { kind, message }));
        if let Some(message) = message {
            match kind {
                ComposerKind::Forward { form } => {
                    steps.extend(self.offer_chooser(message, rows, form));
                }
                ComposerKind::Reply | ComposerKind::ReplyAll => {
                    steps.extend(self.offer_older_reply(message));
                }
                _ => {}
            }
        }
        steps
    }

    fn thread_view_of(&self, message: MessageId) -> Option<&ThreadView> {
        let has = |view: &ThreadView| view.members.iter().any(|member| member.id == message);
        self.walk
            .as_ref()
            .map(|walk| &walk.view)
            .filter(|view| has(view))
            .or_else(|| {
                self.expansion
                    .as_ref()
                    .map(|expansion| &expansion.view)
                    .filter(|view| has(view))
            })
    }

    fn latest_received(&self, message: MessageId) -> Option<MessageId> {
        let view = self.thread_view_of(message)?;
        view.members
            .iter()
            .rev()
            .find(|member| !member.own)
            .map(|member| member.id)
    }

    fn offer_chooser(
        &mut self,
        message: MessageId,
        rows: Option<&dyn Rows>,
        form: ForwardForm,
    ) -> Vec<Step> {
        let (messages, of, thread) = match self.thread_view_of(message) {
            Some(view) => (
                view.members.len(),
                view.members
                    .iter()
                    .find(|member| member.id == message)
                    .map(|member| (words::member_name(member), member.at)),
                Some(view.thread),
            ),
            None => {
                let count = rows
                    .and_then(|rows| rows.row(rows.position_of(message)?))
                    .and_then(|row| row.as_conversation().map(|c| c.summary.message_count));
                let thread = rows
                    .and_then(|rows| rows.facts(rows.position_of(message)?))
                    .and_then(|facts| facts.threads.first().copied());
                (count.unwrap_or(1) as usize, None, thread)
            }
        };
        if messages < 2 {
            return Vec::new();
        }
        let chooser = Chooser {
            form,
            thread,
            opening: form != ForwardForm::This,
            messages,
            of,
        };
        let view = self.chooser_view(&chooser);
        let mut steps = vec![Step::Show(Intent::ForwardChooser(Box::new(view)))];
        // Opened on a form the composer's own draft is not in: the engine
        // builds it, so no frontend picks the call from the form.
        if let (true, Some(thread)) = (chooser.opening, chooser.thread) {
            steps.push(Step::Ask(Request::ForwardThread { thread, form }));
        }
        self.compose.thread.chooser = Some(chooser);
        steps
    }

    fn chooser_view(&self, chooser: &Chooser) -> words::ChooserView {
        words::forward_chooser(
            chooser.form,
            chooser.messages,
            chooser.of.as_ref().map(|(who, at)| (who.as_str(), *at)),
            self.keymap(),
        )
    }

    fn offer_older_reply(&mut self, message: MessageId) -> Vec<Step> {
        let Some(view) = self.thread_view_of(message) else {
            return Vec::new();
        };
        let Some(at) = view.members.iter().position(|member| member.id == message) else {
            return Vec::new();
        };
        let latest = view.members.iter().rposition(|member| !member.own);
        if latest.is_none_or(|latest| at >= latest) {
            return Vec::new();
        }
        let answered = &view.members[at];
        let newer = &view.members[at + 1..];
        let older = Older {
            facts: OlderReplyFacts {
                number: at + 1,
                total: view.members.len(),
                who: words::member_name(answered),
                at: answered.at,
                newer: newer
                    .iter()
                    .map(|member| words::NewerMessage {
                        who: words::member_name(member),
                        at: member.at,
                    })
                    .collect(),
            },
            newer: newer.iter().map(|member| member.id).collect(),
            shown: false,
        };
        let bar = self.older_bar(&older);
        self.compose.thread.older = Some(older);
        vec![Step::Show(Intent::OlderReply(Some(Box::new(bar))))]
    }

    fn older_bar(&self, older: &Older) -> words::OlderReplyView {
        words::older_reply(&older.facts, older.shown, self.keymap())
    }

    pub(crate) fn forward_form_chosen(&mut self, form: ForwardForm) -> Vec<Step> {
        if !self.surfaces.has(SurfaceKind::Composer) {
            return Vec::new();
        }
        match self.compose.thread.chooser.as_ref() {
            Some(chooser) if chooser.form != form => {
                vec![Step::Show(Intent::ComposerDraftWanted(
                    DraftEdit::ForwardForm(form),
                ))]
            }
            _ => Vec::new(),
        }
    }

    fn reply_to_latest_asked(&mut self) -> Vec<Step> {
        if self.compose.thread.older.is_none() {
            return Vec::new();
        }
        vec![Step::Show(Intent::ComposerDraftWanted(
            DraftEdit::ReplyToLatest,
        ))]
    }

    pub(crate) fn composer_draft(
        &mut self,
        edit: DraftEdit,
        draft: Box<postio_model::Draft>,
    ) -> Vec<Step> {
        if self.compose.open.is_none() {
            return Vec::new();
        }
        vec![Step::Ask(match edit {
            DraftEdit::ForwardForm(form) => Request::SetForwardForm { draft, form },
            DraftEdit::ReplyToLatest => Request::ReplyToLatest { draft },
        })]
    }

    pub(crate) fn forwarded(
        &mut self,
        form: ForwardForm,
        answer: Result<Forwarded, String>,
    ) -> Vec<Step> {
        let notice = |text: String| {
            vec![Step::Show(Intent::Toast {
                text,
                kind: ToastKind::Notice,
            })]
        };
        match answer {
            Ok(Forwarded::NotLocal(missing)) => {
                let mut steps = notice(words::forward_not_local(form, missing.len()));
                if let Some(chooser) = self.compose.thread.chooser.as_mut()
                    && chooser.opening
                {
                    chooser.opening = false;
                    chooser.form = ForwardForm::This;
                    let chooser = chooser.clone();
                    steps.push(Step::Show(Intent::ForwardChooser(Box::new(
                        self.chooser_view(&chooser),
                    ))));
                }
                steps
            }
            Err(sentence) => notice(sentence),
            Ok(Forwarded::Draft(draft)) => {
                let Some(chooser) = self.compose.thread.chooser.as_mut() else {
                    return Vec::new();
                };
                chooser.form = form;
                chooser.opening = false;
                let chooser = chooser.clone();
                vec![
                    Step::Show(Intent::ForwardChooser(Box::new(
                        self.chooser_view(&chooser),
                    ))),
                    Step::Show(Intent::ComposerDraft(draft)),
                ]
            }
        }
    }

    pub(crate) fn replied_to_latest(
        &mut self,
        answer: Result<Box<postio_model::Draft>, String>,
    ) -> Vec<Step> {
        match answer {
            Err(sentence) => vec![Step::Show(Intent::Toast {
                text: sentence,
                kind: ToastKind::Notice,
            })],
            Ok(draft) => {
                let Some(older) = self.compose.thread.older.take() else {
                    return Vec::new();
                };
                let mut steps = vec![Step::Show(Intent::OlderReply(None))];
                if older.shown {
                    steps.push(Step::Show(Intent::NewerMessages(None)));
                }
                steps.push(Step::Show(Intent::ComposerDraft(draft)));
                steps
            }
        }
    }

    fn show_newer_toggled(&mut self) -> Vec<Step> {
        let Some(older) = self.compose.thread.older.as_mut() else {
            return Vec::new();
        };
        older.shown = !older.shown;
        let (shown, ids) = (older.shown, older.newer.clone());
        let bar = self.older_bar(self.compose.thread.older.as_ref().expect("just toggled"));
        vec![
            Step::Show(Intent::NewerMessages(shown.then_some(ids))),
            Step::Show(Intent::OlderReply(Some(Box::new(bar)))),
        ]
    }

    fn quoted_toggled(&mut self) -> Vec<Step> {
        if self.compose.open.is_none() {
            return Vec::new();
        }
        let expanded = !self.compose.thread.quoted_expanded;
        self.compose.thread.quoted_expanded = expanded;
        vec![Step::Show(Intent::QuotedExpanded(expanded))]
    }

    pub(crate) fn composer_opened(&mut self) -> Vec<Step> {
        let steps = self.end_composition();
        self.begin_composition();
        steps
    }

    fn begin_composition(&mut self) {
        self.compose.thread = ThreadCompose::default();
        self.compose.begun += 1;
        self.compose.open = Some(Composition {
            number: self.compose.begun,
            written: false,
            pending: None,
        });
    }

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

    /// The composer left the stack some way the controller did not take it off (its window's close
    /// button, another window replacing it (M4)): its composition ends, saved.
    pub(crate) fn composer_gone(&mut self) -> Vec<Step> {
        if self.compose.open.is_none() || self.surfaces.has(SurfaceKind::Composer) {
            return Vec::new();
        }
        self.end_composition()
    }

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

    /// One the open message offers moves out of the list it was opened from, so it closes and the
    /// keyboard goes home; the next row is not opened in its place.
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
