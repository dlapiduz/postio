//! Verbs on the list, their aim, and the cursor when mail leaves or comes
//! back (research R2, slice 4; contract invariant 4).
//!
//! Moved from `postio-gtk`'s window. A verb goes to the selection when there
//! is one and the cursor's row otherwise (`postio_ui::focus_target`); an
//! archive or delete hands the cursor to the first row below its own that
//! stays; what the selection named has been acted on, so it goes; the
//! host's words are the toast; and an undo puts the cursor on the row it
//! brought back once the list has it.

use postio_core::state::Selection;
use postio_core::{Command, CommandId, Event, MessageTarget};
use postio_model::{AccountId, DeliveryId, MessageId};
use postio_ui::focus_target::{Aim, AimRow, Dispatch};

use crate::cursor::{Cursor, RowFacts, Rows};
use crate::feed::Step;
use crate::{Intent, Request};

/// Everything the view shows, but these: a predicate the host resolves,
/// never a list of what is on screen. Over the inboxes of `accounts`; or,
/// with `query` -- ⇧X in a search's results (spec 010 US5, the data
/// model's `Aim::Matching`) -- over every conversation the query matches,
/// walked as the results' own match is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Everything {
    /// The accounts Focus's inbox is made of.
    pub accounts: Vec<AccountId>,
    /// Rows taken back out of the selection: for a query, each row's best
    /// message, which stands for its conversation.
    pub except: Vec<MessageId>,
    /// The search whose every match is selected, instead of the inbox.
    pub query: Option<postio_search::ParsedQuery>,
}

/// Where a verb would go: each of `aims`, or -- with `everything` -- a
/// whole-view selection the host resolves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aimed {
    /// Where it goes, in the order to send it.
    pub aims: Vec<MessageTarget>,
    /// A whole-view selection, instead of `aims`.
    pub everything: Option<Everything>,
}

/// How a toast is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToastKind {
    /// A verb ran.
    Completed {
        /// Whether Undo can take it back.
        undoable: bool,
        /// How long the toast stays, when it is not the usual: an answer's
        /// lasts as long as the answer can be taken back (spec 007 FR-102).
        seconds: Option<u32>,
    },
    /// An undo was applied.
    Undone,
    /// A command could not run: a quiet hint, not an alarm.
    Notice,
    /// What is missing, with the one way to put it right: a button
    /// labelled `label` that runs `command` ("Add account").
    Offer {
        /// The button's words.
        label: String,
        /// What the button runs.
        command: CommandId,
    },
}

impl ToastKind {
    /// How long a toast of this kind stays, in seconds: long enough to
    /// read and reach for Undo, far short of the undo stack's own expiry,
    /// so Undo works on once the toast has gone -- unless its Undo lasts a
    /// window of its own, which it then stays exactly as long as (spec 007
    /// FR-102). The Mac's toast and GTK's are both this long.
    pub fn seconds(&self) -> u32 {
        match self {
            ToastKind::Completed {
                seconds: Some(seconds),
                ..
            } => *seconds,
            _ => postio_ui::focus_target::TOAST_SECONDS,
        }
    }
}

/// Rows an undo will bring back, waiting for the list to have them.
#[derive(Debug, Clone)]
struct Restoring {
    rows: Vec<MessageId>,
    /// Whether the list stood at its top: it goes back there.
    at_top: bool,
    /// Landings waited so far; the first after an undo may be older reads.
    waited: u8,
}

/// The rows an undo can bring back, at most this many: only the last unit
/// can be undone with any confidence about where it was.
const REMEMBERED: usize = 64;

/// The list's verbs, and what they leave behind.
#[derive(Debug, Default)]
pub(crate) struct Verbs {
    /// Rows the last archives and deletes took out.
    removed: Vec<MessageId>,
    restoring: Option<Restoring>,
    /// Whether the last landing put the cursor on a row an undo brought
    /// back, and nobody has asked since.
    restored: bool,
    /// Whether the last command was an answer to an invitation.
    answering: bool,
    /// Whether the list stands at its top (a fact the frontend reports).
    at_top: bool,
}

impl Verbs {
    pub(crate) fn set_at_top(&mut self, at_top: bool) {
        self.at_top = at_top;
    }

    /// Whether an undo is waiting for its rows to be in the list.
    pub(crate) fn restoring(&self) -> bool {
        self.restoring.is_some()
    }

    /// Whether the cursor was put on a row an undo brought back since this
    /// was last asked.
    pub(crate) fn take_restored(&mut self) -> bool {
        std::mem::take(&mut self.restored)
    }

    /// A command the verbs answer for the open message alone -- not the
    /// selection, which the message's window does not show -- or `None`
    /// when it is not one of theirs. An archive or delete is remembered for
    /// the undo; stepping past it is the open message's.
    pub(crate) fn command_on(&mut self, id: CommandId, row: &RowFacts) -> Option<Vec<Step>> {
        let Dispatch::OnMail(command) = postio_ui::focus_target::dispatch(id)? else {
            return None;
        };
        let at = AimRow {
            id: row.id,
            threads: row.threads.clone(),
            digest: row.digest,
        };
        let Aim::Targets(aims) = postio_ui::focus_target::aim_by(
            &Selection::These(Vec::new()),
            &Default::default(),
            Some(&at),
        ) else {
            return Some(Vec::new());
        };
        if aims.is_empty() {
            return Some(Vec::new());
        }
        if matches!(command, Command::Archive { .. } | Command::Delete { .. }) {
            self.note_removed(&Selection::These(Vec::new()), Some(&at));
        }
        Some(vec![Step::Ask(Request::Send {
            command,
            aims,
            everything: None,
        })])
    }

    /// A command the verbs answer, or `None` when it is not one of theirs.
    pub(crate) fn command(
        &mut self,
        id: CommandId,
        cursor: &mut Cursor,
        rows: &dyn Rows,
        total: u32,
    ) -> Option<Vec<Step>> {
        let steps = match id {
            CommandId::AcceptInvite | CommandId::DeclineInvite => {
                let Some(row) = cursor.row(rows).filter(|row| !row.digest) else {
                    return Some(Vec::new());
                };
                let message = Some(row.id);
                self.post(match id {
                    CommandId::DeclineInvite => Command::DeclineInvite { message },
                    _ => Command::AcceptInvite { message },
                })
            }
            CommandId::DismissMarker => self.send(
                Command::DismissMarker {
                    target: MessageTarget::Selection,
                    dismissed: true,
                },
                cursor,
                rows,
                total,
            ),
            _ => match postio_ui::focus_target::dispatch(id)? {
                Dispatch::OnMail(command) => self.send(command, cursor, rows, total),
                Dispatch::Plain(command) => self.post(command),
            },
        };
        Some(steps)
    }

    /// Send `command`, aimed as the selection or the cursor says, and let
    /// the selection go: what it named has been acted on.
    pub(crate) fn send(
        &mut self,
        command: Command,
        cursor: &mut Cursor,
        rows: &dyn Rows,
        total: u32,
    ) -> Vec<Step> {
        let selection = cursor.selection();
        let at = Self::row_aimed_at(cursor, rows);
        let Aimed { aims, everything } = Self::aimed(cursor, rows);
        if everything.is_none() && aims.is_empty() {
            return Vec::new();
        }
        let mut steps = Vec::new();
        if matches!(command, Command::Archive { .. } | Command::Delete { .. }) {
            self.note_removed(&selection, at.as_ref());
            steps.extend(Self::past_removed(&selection, cursor, rows));
        }
        steps.extend(cursor.clear(total));
        steps.push(Step::Ask(Request::Send {
            command,
            aims,
            everything,
        }));
        steps
    }

    /// The cursor's row as a verb aims at it.
    fn row_aimed_at(cursor: &Cursor, rows: &dyn Rows) -> Option<AimRow> {
        cursor.row(rows).map(|row| AimRow {
            id: row.id,
            threads: row.threads,
            digest: row.digest,
        })
    }

    /// Where a verb goes: the selection when there is one, the cursor's row
    /// otherwise. A row folded from several accounts is every copy
    /// (`MessageTarget::Threads`, T161); a message in no conversation is
    /// itself. A whole-view selection is a predicate the host resolves over
    /// the inboxes, never a list of what happens to be on screen.
    pub(crate) fn aimed(cursor: &Cursor, rows: &dyn Rows) -> Aimed {
        let at = Self::row_aimed_at(cursor, rows);
        match postio_ui::focus_target::aim_by(&cursor.selection(), cursor.reach(), at.as_ref()) {
            Aim::Everything { except } => Aimed {
                aims: Vec::new(),
                everything: Some(Everything {
                    accounts: cursor.accounts().to_vec(),
                    except,
                    query: None,
                }),
            },
            Aim::Targets(aims) => Aimed {
                aims,
                everything: None,
            },
        }
    }

    /// Send `command` as it is: a verb that aims at nothing. An answer to
    /// an invitation is remembered, so its toast stays as long as its
    /// reply can be taken back.
    pub(crate) fn post(&mut self, command: Command) -> Vec<Step> {
        if matches!(
            command,
            Command::AcceptInvite { .. } | Command::DeclineInvite { .. }
        ) {
            self.answering = true;
        }
        vec![Step::Ask(Request::Post(command))]
    }

    /// Archive a whole delivery: one undo (FR-125). Its row goes with it,
    /// and comes back with the undo.
    pub(crate) fn archive_digest(&mut self, delivery: DeliveryId) -> Vec<Step> {
        self.remember(vec![MessageId::new(-delivery.get())]);
        vec![Step::Ask(Request::Post(Command::ArchiveDigest {
            delivery,
            archived: true,
        }))]
    }

    /// Remember the rows an archive or delete is about to take out: what
    /// the cursor goes back to when the undo brings them back.
    fn note_removed(&mut self, selection: &Selection, cursor: Option<&AimRow>) {
        let rows: Vec<MessageId> = match selection {
            Selection::These(ids) if !ids.is_empty() => ids.clone(),
            // Nothing selected: the cursor's row is what the verb takes.
            Selection::These(_) => cursor.map(|row| row.id).into_iter().collect(),
            // Everything selected is a predicate, with no rows to name.
            Selection::Everything { .. } => Vec::new(),
        };
        self.remember(rows);
    }

    fn remember(&mut self, rows: Vec<MessageId>) {
        self.removed.extend(rows);
        let excess = self.removed.len().saturating_sub(REMEMBERED);
        self.removed.drain(..excess);
    }

    /// The selected rows are about to take the cursor's row with them:
    /// stand the cursor on the first row below it that stays. The list
    /// slides it up as the rows above go, so it lands on the row that
    /// followed, not on the slot the old row held (#468).
    pub(crate) fn past_removed(
        selection: &Selection,
        cursor: &mut Cursor,
        rows: &dyn Rows,
    ) -> Vec<Step> {
        let Selection::These(gone) = selection else {
            return Vec::new();
        };
        let Some(at) = cursor.position() else {
            return Vec::new();
        };
        if gone.is_empty() {
            return Vec::new();
        }
        let below = (at..rows.len()).map(|place| rows.facts(place).map(|row| row.id));
        match postio_ui::selection::survivor_below(below, gone) {
            Some(offset) => cursor.place(at + offset as u32, rows),
            None => Vec::new(),
        }
    }

    /// What the host said about this client's own commands.
    pub(crate) fn event(&mut self, event: &Event) -> Vec<Step> {
        match event {
            Event::ActionCompleted {
                description,
                undoable,
            } => {
                // An answer's Undo works while its reply waits, so its toast
                // stays exactly that long (spec 007 FR-102).
                let seconds = std::mem::take(&mut self.answering)
                    .then(|| u32::try_from(postio_core::command::RSVP_WINDOW.as_secs()))
                    .and_then(Result::ok);
                vec![Step::Show(Intent::Toast {
                    text: description.clone(),
                    kind: ToastKind::Completed {
                        undoable: *undoable,
                        seconds,
                    },
                })]
            }
            Event::UndoPerformed { description } => {
                let rows = std::mem::take(&mut self.removed);
                if !rows.is_empty() {
                    self.restoring = Some(Restoring {
                        rows,
                        at_top: self.at_top,
                        waited: 0,
                    });
                }
                vec![Step::Show(Intent::Toast {
                    text: description.clone(),
                    kind: ToastKind::Undone,
                })]
            }
            Event::CommandRejected { reason, .. } => {
                self.answering = false;
                vec![Step::Show(Intent::Toast {
                    text: reason.clone(),
                    kind: ToastKind::Notice,
                })]
            }
            _ => Vec::new(),
        }
    }

    /// The list has landed: put the cursor on the row an undo brought back,
    /// once it is in the list, and keep the list where it stood.
    pub(crate) fn landed(&mut self, cursor: &mut Cursor, rows: &dyn Rows) -> Vec<Step> {
        let Some(mut restoring) = self.restoring.take() else {
            return Vec::new();
        };
        let found = restoring
            .rows
            .iter()
            .filter_map(|row| rows.position_of(*row))
            .min();
        match found {
            Some(position) => {
                self.restored = true;
                let mut steps = cursor.place(position, rows);
                if restoring.at_top {
                    steps.push(Step::Show(Intent::ListToTop));
                }
                steps
            }
            None => {
                restoring.waited += 1;
                if restoring.waited < 4 {
                    self.restoring = Some(restoring);
                }
                Vec::new()
            }
        }
    }
}
