//! Verbs on the list, their aim, and the cursor when mail leaves or comes back. A verb goes to the
//! selection when there is one and the cursor's row otherwise; an archive or delete hands the
//! cursor to the first row below that stays; an undo puts it on the row it brought back once the
//! list has it.

use postio_core::state::Selection;
use postio_core::{Command, CommandId, Event, MessageTarget};
use postio_model::{AccountId, DeliveryId, MessageId};
use postio_ui::focus_target::{Aim, AimRow, Dispatch};

use crate::cursor::{Cursor, RowFacts, Rows};
use crate::feed::Step;
use crate::{Intent, Request};

/// A predicate the host resolves, never a list of what is on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Everything {
    pub accounts: Vec<AccountId>,
    pub except: Vec<MessageId>,
    pub query: Option<postio_search::ParsedQuery>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aimed {
    pub aims: Vec<MessageTarget>,
    pub everything: Option<Everything>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToastKind {
    Completed {
        undoable: bool,
        seconds: Option<u32>,
    },
    Undone,
    Notice,
    Offer {
        label: String,
        command: CommandId,
    },
}

impl ToastKind {
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

#[derive(Debug, Clone)]
struct Restoring {
    rows: Vec<MessageId>,
    at_top: bool,
    waited: u8,
}

const REMEMBERED: usize = 64;

#[derive(Debug, Default)]
pub(crate) struct Verbs {
    removed: Vec<MessageId>,
    restoring: Option<Restoring>,
    restored: bool,
    answering: bool,
    at_top: bool,
}

impl Verbs {
    pub(crate) fn set_at_top(&mut self, at_top: bool) {
        self.at_top = at_top;
    }

    pub(crate) fn restoring(&self) -> bool {
        self.restoring.is_some()
    }

    pub(crate) fn restoring_rows(&self) -> Option<Vec<MessageId>> {
        Some(self.restoring.as_ref()?.rows.clone())
    }

    pub(crate) fn restored(&self) -> bool {
        self.restored
    }

    pub(crate) fn take_restored(&mut self) -> bool {
        std::mem::take(&mut self.restored)
    }

    pub(crate) fn command_on(&mut self, id: CommandId, row: &RowFacts) -> Option<Vec<Step>> {
        let Dispatch::OnMail(command) = postio_ui::focus_target::dispatch(id)? else {
            return None;
        };
        let at = AimRow {
            id: row.id,
            threads: row.threads.clone(),
            digest: row.digest,
            child: false,
        };
        let Aim::Targets(aims) = postio_ui::focus_target::aim_by(
            &Selection::These(Vec::new()),
            &Default::default(),
            Some(&at),
            id,
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

    pub(crate) fn command(
        &mut self,
        id: CommandId,
        cursor: &mut Cursor,
        rows: &dyn Rows,
        total: u32,
    ) -> Option<Vec<Step>> {
        let steps = match id {
            CommandId::AcceptInvite | CommandId::DeclineInvite | CommandId::TentativeInvite => {
                let Some(row) = cursor.row(rows).filter(|row| !row.digest) else {
                    return Some(Vec::new());
                };
                let answer = postio_core::command::answer_invite(id, Some(row.id))?;
                self.post(answer)
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
            // The child's own message (D2), whatever is selected; a row
            // that stands for a thread refuses it.
            CommandId::DeleteThisMessage => self.send_as(
                id,
                Command::default_for(CommandId::Delete),
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

    pub(crate) fn send(
        &mut self,
        command: Command,
        cursor: &mut Cursor,
        rows: &dyn Rows,
        total: u32,
    ) -> Vec<Step> {
        let action = command.id();
        self.send_as(action, command, cursor, rows, total)
    }

    fn send_as(
        &mut self,
        action: CommandId,
        command: Command,
        cursor: &mut Cursor,
        rows: &dyn Rows,
        total: u32,
    ) -> Vec<Step> {
        // One message, the cursor's, whatever is selected: nothing else
        // leaves, so no selection is spent and no row is remembered for an
        // undo to bring back.
        if action == CommandId::DeleteThisMessage {
            let at = Self::row_aimed_at(cursor, rows);
            let Aim::Targets(aims) = postio_ui::focus_target::aim_by(
                &Selection::These(Vec::new()),
                &Default::default(),
                at.as_ref(),
                action,
            ) else {
                return Vec::new();
            };
            if aims.is_empty() {
                return Vec::new();
            }
            return vec![Step::Ask(Request::Send {
                command,
                aims,
                everything: None,
            })];
        }
        let selection = cursor.selection();
        let at = Self::row_aimed_at(cursor, rows);
        let Aimed { aims, everything } = Self::aimed(cursor, rows, action);
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

    fn row_aimed_at(cursor: &Cursor, rows: &dyn Rows) -> Option<AimRow> {
        cursor.aim_row(rows)
    }

    /// A whole-view selection is a predicate the host resolves over the inboxes, never a list of
    /// what happens to be on screen.
    pub(crate) fn aimed(cursor: &Cursor, rows: &dyn Rows, action: CommandId) -> Aimed {
        let at = Self::row_aimed_at(cursor, rows);
        match postio_ui::focus_target::aim_by(
            &cursor.selection(),
            cursor.reach(),
            at.as_ref(),
            action,
        ) {
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

    pub(crate) fn post(&mut self, command: Command) -> Vec<Step> {
        if matches!(
            command,
            Command::AcceptInvite { .. }
                | Command::DeclineInvite { .. }
                | Command::TentativeInvite { .. }
        ) {
            self.answering = true;
        }
        vec![Step::Ask(Request::Post(command))]
    }

    pub(crate) fn archive_digest(&mut self, delivery: DeliveryId) -> Vec<Step> {
        self.remember(vec![MessageId::new(-delivery.get())]);
        vec![Step::Ask(Request::Post(Command::ArchiveDigest {
            delivery,
            archived: true,
        }))]
    }

    fn note_removed(&mut self, selection: &Selection, cursor: Option<&AimRow>) {
        let rows: Vec<MessageId> = match selection {
            Selection::These(ids) if !ids.is_empty() => ids.clone(),
            Selection::These(_) => cursor.map(|row| row.id).into_iter().collect(),
            Selection::Everything { .. } => Vec::new(),
        };
        self.remember(rows);
    }

    fn remember(&mut self, rows: Vec<MessageId>) {
        self.removed.extend(rows);
        let excess = self.removed.len().saturating_sub(REMEMBERED);
        self.removed.drain(..excess);
    }

    /// The list slides it up as the rows above go, so it lands on the row that followed, not on the
    /// slot the old row held (#468).
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

    pub(crate) fn event(&mut self, event: &Event) -> Vec<Step> {
        match event {
            Event::ActionCompleted {
                description,
                undoable,
            } => {
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
