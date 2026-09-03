//! The message verbs, as the command bus runs them.
//!
//! Archive, delete, move, flag and mark-unread are the daily vocabulary, and
//! every one of them takes the same shape: resolve what the invocation is
//! aimed at, write SQLite, enqueue the operation the server will eventually
//! see, push an undo entry, emit the events the panes repaint from. Nothing
//! here awaits the network — `postio-sync` drains the queue on its own, later
//! and somewhere else — which is what makes these work on a train and what
//! makes them reconcile when the link comes back.
//!
//! # Why they live in the composition root
//!
//! A handler needs the store, and `postio-core` is not allowed to know what
//! SQLite is. `postio-gtk` is not allowed to either. This crate is the one
//! that knows both halves exist, so this is where the verb meets the
//! database.
//!
//! # `run` takes a command, not an `Invocation`
//!
//! [`Invocation`](postio_core::dispatch::Invocation) can only be built by the
//! bus, so a handler written against it can only be tested through a running
//! tokio runtime. Taking the command and the sink instead makes every verb an
//! ordinary function over a throwaway database, and leaves the registration
//! below as the only thing the bus shape touches.
//!
//! # Undo replays inverses directly
//!
//! An entry carries its inverse as [`Command`]s, and [`Actions::undo`] applies
//! them through the same machinery the original action used — but with
//! [`Recording::Replay`], so nothing is pushed back onto the stack. Sending
//! them through the bus instead would record an undo of the undo, and `u` `u`
//! would toggle rather than walk back through history.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use chrono::{Duration, Utc};
use postio_core::bridge::EventSink;
use postio_core::dispatch::{CommandError, DispatcherBuilder};
use postio_core::state::{Resolved, SharedState, ViewScope};
use postio_core::undo::{UndoEntry, UndoKind, UndoStack};
use postio_core::{Command, CommandId, Event, MessageTarget};
use postio_model::ids::DraftId;
use postio_model::mailbox::MailboxRole;
use postio_model::{
    AccountId, DraftState, Flag, FlagSet, MailboxId, Message, MessageId, Operation,
    OperationTarget, ThreadId,
};
use postio_storage::repository::{
    ColumnFlag, DraftRepository, FlagSource, MailboxRepository, MessageRepository, MessageSet,
    OperationQueueRepository, ThreadOrder, ThreadRepository,
};
use postio_storage::{Database, PooledConnection, WritePermit, WritePriority};

/// The commands this module answers.
///
/// Named once so that the registration and the match in [`Actions::act`]
/// cannot drift apart — a wired command with no arm reports "not wired up
/// yet" from inside the thing that is supposed to be wiring it up.
const WIRED: &[CommandId] = &[
    CommandId::Archive,
    CommandId::ArchiveThread,
    CommandId::Delete,
    CommandId::Move,
    CommandId::Flag,
    CommandId::MarkUnread,
    CommandId::Snooze,
    CommandId::Unsnooze,
    CommandId::MarkSent,
    CommandId::Undo,
];

/// How long [`Command::Snooze`] hides a message for, with no duration picker
/// yet to ask for anything else.
///
/// #493's own scope note: a picker mirroring `ScheduleMenu`
/// (`crates/postio-gtk/src/composer.rs`) is natural follow-up work once a
/// single sensible default has proven the rest of the feature out — the same
/// sequencing #6 already used to split scheduled send from snooze in the
/// first place.
const DEFAULT_SNOOZE: Duration = Duration::hours(3);

/// Whether a verb is being performed or replayed backwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Recording {
    /// The user did this. Push it onto the stack and offer to take it back.
    Record,
    /// Undo is doing this. It is already history.
    Replay,
    /// The app did this on the user's behalf, and nobody asked for it.
    ///
    /// Repaints like any other change, but raises no toast and takes no place
    /// on the undo stack. The dwell mark (#71) is the case: one per message
    /// the cursor rests on, so recording them would bury the verb the user
    /// actually wants `u` to reach, and a toast each would be a banner that
    /// never goes away.
    Incidental,
}

impl Recording {
    /// Whether this belongs on the undo stack and deserves a toast.
    fn records(self) -> bool {
        self == Recording::Record
    }
}

/// Where a relocation is sending its messages.
#[derive(Debug, Clone, Copy)]
enum Destination {
    /// The account's folder for this purpose, whatever it is called.
    Role(MailboxRole),
    /// This folder, named by whoever invoked the command.
    Mailbox(MailboxId),
}

/// What a verb is aimed at, once app state and the store have both had a say.
///
/// The second variant is the whole of `postio-agr.1`. A whole-mailbox
/// selection arrives here as a predicate and has to leave here as a predicate:
/// turning it into rows — even briefly, even only to count them — is the
/// mailbox-sized read docs/PRODUCT.md §18 forbids and the 16 ms budget cannot afford.
enum Aim {
    /// These messages, read out of the store.
    Rows(Vec<Message>),
    /// One predicate per account, each resolved in one statement.
    ///
    /// Split by account here rather than further down because everything
    /// below this line is an account's: which folder `Archive` names, which
    /// queue the rows go on, which run `u` takes back, and which account the
    /// events announcing the work say it happened in (ADR 0005 Q11). A
    /// single-account view yields exactly one unit and costs nothing; the
    /// unified view is why the shape is a list (#811).
    Bulk(Vec<BulkUnit>),
}

/// One account's share of a bulk aim.
struct BulkUnit {
    /// What to act on, within this account.
    set: MessageSet,
    /// Whose account it is. Read from the mailbox, or carried by a scope
    /// that already knows it, because there is no row to read.
    account: AccountId,
    /// The folder those messages are in now, when they are all in one.
    ///
    /// `None` for a smart folder or the aggregate, whose rows are spread
    /// across folders — a move out of one has to be grouped by source
    /// before it can be enqueued (#52).
    from: Option<MailboxId>,
}

/// What a verb did, once it had done it.
///
/// Separated from announcing it so undo can replay the same work without
/// offering to undo the undo.
struct Applied {
    kind: UndoKind,
    /// The account the verb ran in. Every event announcing the work names it
    /// (ADR 0005 Q11); a unified-scope action later becomes one `Applied` per
    /// account, expanded in the store.
    account: AccountId,
    /// Every message the unit touched, in the order it touched them — empty
    /// for a bulk unit, which knows how many it touched and not which.
    messages: Vec<MessageId>,
    /// How many messages it touched. `messages.len()` unless it was bulk.
    count: usize,
    /// Rows that left a mailbox, grouped by the mailbox they left.
    removed: Vec<(MailboxId, Vec<MessageId>)>,
    /// The mailbox that gained rows, when one did. The list showing it has to
    /// reload or an undone archive would not reappear until the next sync.
    arrived: Option<MailboxId>,
    /// Mailboxes whose contents changed wholesale.
    ///
    /// A bulk action reports itself this way instead of through `removed`: the
    /// list has to reload the folder, because naming the eighty thousand rows
    /// that left it is the read the predicate exists to avoid.
    reloaded: Vec<MailboxId>,
    /// Rows that changed in place: flags, read state.
    changed: Vec<MessageId>,
    /// What takes it back.
    inverse: Vec<Command>,
}

/// Everything a verb needs: the store to write, the state to resolve targets
/// against, and the history to push onto.
///
/// Cheap to clone — a `Database` is a connection pool behind a handle, and the
/// other two are shared by construction — because the bus holds one of these
/// per registered command.
#[derive(Clone)]
pub struct Actions {
    database: Database,
    state: SharedState,
    undo: Arc<Mutex<UndoStack>>,
}

impl Actions {
    /// Verbs over `database`, resolving their targets against `state`.
    pub fn new(database: Database, state: SharedState) -> Self {
        Actions {
            database,
            state,
            undo: Arc::new(Mutex::new(UndoStack::new())),
        }
    }

    /// Run one invocation, reporting through `events`.
    ///
    /// `Err` is what the user sees: the bus turns a rejection into a quiet
    /// hint and a failure into something louder.
    ///
    pub fn run(&self, command: &Command, events: &EventSink) -> Result<(), CommandError> {
        match command {
            Command::Undo => self.undo(events),
            // Same verb, different provenance — see `Command::MarkReadOnDwell`.
            // The `Recording` is most of the difference; the rest is that a
            // rejection is not worth saying out loud. `set_flag` rejects with
            // "Already set" when nothing changes, and the cursor resting on
            // mail that has already been read is the *ordinary* case once a
            // mailbox has been worked through — a quiet hint every time would
            // be a hint that never stops. A vanished row rejects the same way
            // and deserves the same silence. A `Failed` still gets through,
            // because a store that will not write is worth hearing about.
            dwell @ Command::MarkReadOnDwell { .. } => {
                match self.act(dwell, events, Recording::Incidental) {
                    Err(CommandError::Rejected(_)) => Ok(()),
                    other => other,
                }
            }
            other => self.act(other, events, Recording::Record),
        }
    }

    /// Do one verb, and say what it did.
    fn act(
        &self,
        command: &Command,
        events: &EventSink,
        recording: Recording,
    ) -> Result<(), CommandError> {
        let applied: Vec<Applied> = match command {
            Command::Archive { target } => self.relocate(
                target,
                Destination::Role(MailboxRole::Archive),
                UndoKind::Archive,
            )?,
            Command::ArchiveThread { thread } => {
                let thread = match thread {
                    Some(thread) => *thread,
                    None => self.thread_in_view()?,
                };
                self.relocate(
                    &MessageTarget::Thread(thread),
                    Destination::Role(MailboxRole::Archive),
                    UndoKind::Archive,
                )?
            }
            Command::Delete { target } => self.relocate(
                target,
                Destination::Role(MailboxRole::Trash),
                UndoKind::Delete,
            )?,
            Command::Move { target, to } => {
                // A move with no destination is not a move that failed; it
                // is half a request — `None` means "ask the user". Nothing
                // asks yet; `postio-agr.2` is the folder picker.
                let to = to.ok_or_else(|| {
                    CommandError::rejected("Pick a folder to move to — drag the rows onto one")
                })?;
                self.relocate(target, Destination::Mailbox(to), UndoKind::Move)?
            }
            Command::Flag { target, flagged } => self.set_flag(target, Flag::Flagged, *flagged)?,
            // `\Seen` is stored the other way up from how the verb reads:
            // marking unread is clearing a flag, not setting one.
            Command::MarkUnread { target, unread } => {
                self.set_flag(target, Flag::Seen, unread.map(|unread| !unread))?
            }
            Command::Snooze { target } => vec![self.snooze(target, Utc::now() + DEFAULT_SNOOZE)?],
            Command::Unsnooze { target } => vec![self.unsnooze(target)?],
            // Deliberately `Some(true)` rather than a toggle: a dwell says
            // "this was read", never "flip whatever it was".
            Command::MarkSent { draft } => vec![self.mark_sent(*draft)?],
            Command::MarkReadOnDwell { message } => self.set_flag(
                &MessageTarget::Messages(vec![*message]),
                Flag::Seen,
                Some(true),
            )?,
            other => {
                return Err(CommandError::rejected(format!(
                    "`{}` is not wired up yet",
                    other.id()
                )));
            }
        };
        for unit in applied {
            self.announce(unit, events, recording);
        }
        Ok(())
    }

    /// Take back the last undoable unit.
    ///
    /// Nothing to take back is a rejection rather than a failure: pressing `u`
    /// on a fresh session is an ordinary thing to do, and it deserves a
    /// sentence, not a dialog.
    fn undo(&self, events: &EventSink) -> Result<(), CommandError> {
        let entry = self
            .stack()
            .undo()
            .ok_or_else(|| CommandError::rejected("Nothing to undo"))?;
        // A cross-account move is not a move, and its undo is not a replay.
        // The undo stack records source rows; the saga table is the only
        // place that knows the move spanned two accounts, so it is asked
        // before the inverse commands are (#531, ADR 0005 Q9).
        if let Some(cancelled) = self.cancel_cross_account_moves(entry.messages(), events)? {
            events.emit(Event::UndoPerformed {
                description: cancelled,
            });
            return Ok(());
        }
        for command in entry.inverse() {
            self.act(command, events, Recording::Replay)?;
        }
        events.emit(Event::UndoPerformed {
            description: entry.description(),
        });
        Ok(())
    }

    /// Withdraw the cross-account moves still open for `messages`, if any.
    ///
    /// `None` when none of them started a saga, which is every ordinary
    /// undo: the caller then replays the inverse commands as before.
    ///
    /// **Only before the copy is confirmed.** Up to that point nothing has
    /// reached either server — the source row is hidden and the target row is
    /// provisional, both local — so cancelling is bookkeeping and can be
    /// complete: withdraw both queue operations, delete the provisional copy,
    /// show the source row again, and end the saga in `aborted`, the phase
    /// whose whole meaning is *nothing was deleted*.
    ///
    /// Once the copy **is** confirmed the message is on two servers, and
    /// taking it back means running the saga backwards from the confirmed
    /// target UID — the inverse saga ADR 0005 Q9 describes, which does not
    /// exist yet. So this refuses, out loud.
    ///
    /// It refuses rather than doing the local half, and that is the point:
    /// un-hiding the source row while the target keeps its copy would leave
    /// the same message on two servers with nothing recording that it should
    /// not be, and this is the one operation in the product that can lose
    /// mail. Before #531 the refusal was not there either — `u` replayed an
    /// empty inverse and reported success, which is the same wrong answer
    /// with better manners.
    fn cancel_cross_account_moves(
        &self,
        messages: &[MessageId],
        events: &EventSink,
    ) -> Result<Option<String>, CommandError> {
        let (mut connection, _permit) = self.connect()?;
        let open = postio_storage::repository::CrossAccountMoveRepository::new(&connection)
            .open_for_sources(messages)
            .map_err(store_failure)?;
        if open.is_empty() {
            return Ok(None);
        }
        if open
            .iter()
            .any(|saga| saga.phase != postio_storage::repository::MovePhase::Copying)
        {
            return Err(CommandError::rejected(
                "That move has already reached the other account, and taking it \
                 back is not something Postio can do yet",
            ));
        }

        let mut sources: Vec<MessageId> = Vec::new();
        let mut reloaded: BTreeSet<MailboxId> = BTreeSet::new();
        let mut accounts: BTreeSet<postio_model::ids::AccountId> = BTreeSet::new();
        let transaction = connection.transaction().map_err(store_failure)?;
        {
            let repository = MessageRepository::new(&transaction);
            let queue = OperationQueueRepository::new(&transaction);
            let sagas = postio_storage::repository::CrossAccountMoveRepository::new(&transaction);
            for saga in &open {
                // The queue rows first, so a crash between here and the end
                // cannot leave an operation pointing at a row that is gone.
                for target in [saga.source_message, saga.target_message]
                    .into_iter()
                    .flatten()
                {
                    while let Some(operation) = queue
                        .pending_for(postio_model::OperationTarget::Message(target))
                        .map_err(store_failure)?
                    {
                        queue.delete(operation.id).map_err(store_failure)?;
                    }
                }
                if let Some(copy) = saga.target_message {
                    repository.delete(&[copy]).map_err(store_failure)?;
                }
                if let Some(source) = saga.source_message {
                    repository
                        .set_deleted_locally(&[source], false)
                        .map_err(store_failure)?;
                    sources.push(source);
                }
                sagas
                    .transition(saga.id, postio_storage::repository::MovePhase::Aborted)
                    .map_err(store_failure)?;
                if let Some(mailbox) = saga.source_mailbox {
                    reloaded.insert(mailbox);
                }
                if let Some(mailbox) = saga.target_mailbox {
                    reloaded.insert(mailbox);
                }
                if let Some(account) = saga.source_account {
                    accounts.insert(account);
                }
                if let Some(account) = saga.target_account {
                    accounts.insert(account);
                }
            }
        }
        transaction.commit().map_err(store_failure)?;

        // Both mailboxes changed: one got a row back, the other lost one.
        for account in accounts {
            for mailbox in &reloaded {
                events.emit(Event::MessageListChanged {
                    account,
                    mailbox: *mailbox,
                });
            }
        }

        let count = sources.len();
        let plural = if count == 1 { "message" } else { "messages" };
        Ok(Some(format!("Cancelled the move of {count} {plural}")))
    }

    // ── The two shapes every verb reduces to ─────────────────────────────

    /// Move messages between folders. Archive, delete and move are all this.
    ///
    /// The source mailbox is read per message rather than assumed from the
    /// list in view: a selection can span folders in the unified view, and
    /// undo has to put each message back where *it* was rather than where the
    /// first one was.
    fn relocate(
        &self,
        target: &MessageTarget,
        to: Destination,
        kind: UndoKind,
    ) -> Result<Vec<Applied>, CommandError> {
        let (mut connection, _permit) = self.connect()?;
        match self.aim(&connection, target)? {
            // One unit per account, which is what `Applied::account` has
            // always said a unified-scope action becomes. A selection made in
            // a unified view can span accounts, and each message has to land
            // in *its own* account's folder: a role destination resolves per
            // account, and filing one account's mail into another's Archive
            // would move it on a server it does not belong to (#182).
            Aim::Rows(rows) => {
                let mut by_account: BTreeMap<AccountId, Vec<Message>> = BTreeMap::new();
                for row in rows {
                    by_account.entry(row.account_id).or_default().push(row);
                }
                // Every destination resolves *before* anything is written.
                // Each account's relocation commits its own transaction, so a
                // failure discovered half way through would leave some of the
                // selection moved and the rest not, with an error claiming
                // the whole thing failed. An account with no Archive folder
                // has to stop the action while stopping it is still free.
                for account in by_account.keys() {
                    mailbox_for(&connection, *account, to)?;
                }

                let mut units = Vec::with_capacity(by_account.len());
                let mut nothing_to_do = None;
                for (_, rows) in by_account {
                    match self.relocate_rows(&mut connection, rows, to, kind) {
                        Ok(unit) => units.push(unit),
                        // Everything in this account was already filed. That
                        // is not a failure for the accounts that were not —
                        // but if *no* account had anything to do, the user
                        // still deserves the sentence.
                        Err(CommandError::Rejected(reason)) => nothing_to_do = Some(reason),
                        Err(error) => return Err(error),
                    }
                }
                match (units.is_empty(), nothing_to_do) {
                    (true, Some(reason)) => Err(CommandError::Rejected(reason)),
                    _ => Ok(units),
                }
            }
            // One unit per account, for the same reason the rows path
            // groups: `to` names a *role*, and the folder it resolves to is a
            // different one in each account.
            Aim::Bulk(units) => {
                // Same pre-flight as the rows path, and for the same reason:
                // each unit commits its own transaction, so an account with
                // no Archive has to stop the action while stopping it is
                // still free.
                for unit in &units {
                    mailbox_for(&connection, unit.account, to)?;
                }
                let mut applied = Vec::with_capacity(units.len());
                let mut nothing_to_do = None;
                for unit in units {
                    match self.relocate_set(
                        &mut connection,
                        unit.set,
                        unit.account,
                        unit.from,
                        to,
                        kind,
                    ) {
                        Ok(one) => applied.push(one),
                        Err(CommandError::Rejected(reason)) => nothing_to_do = Some(reason),
                        Err(error) => return Err(error),
                    }
                }
                match (applied.is_empty(), nothing_to_do) {
                    (true, Some(reason)) => Err(CommandError::Rejected(reason)),
                    _ => Ok(applied),
                }
            }
        }
    }

    /// Move a whole mailbox's worth of messages, without ever naming them.
    ///
    /// Four statements, whatever the mailbox holds: count it for the toast,
    /// write the queue rows, move the messages, and — on the way back — the
    /// same again against the run of queue rows the first pass wrote. That run
    /// is what makes one `u` enough: see [`OperationRange`].
    ///
    /// [`OperationRange`]: postio_model::OperationRange
    fn relocate_set(
        &self,
        connection: &mut PooledConnection,
        set: MessageSet,
        account: AccountId,
        from: Option<MailboxId>,
        to: Destination,
        kind: UndoKind,
    ) -> Result<Applied, CommandError> {
        let destination = mailbox_for(connection, account, to)?;
        if from == Some(destination) {
            return Err(CommandError::rejected("Already there"));
        }

        // Which folders the rows are actually in.
        //
        // A folder selection answers itself. A smart folder does not: its
        // rows are spread across the account, and the queue's `Operation`
        // payload carries **one** `from` for the whole run `enqueue_set`
        // writes — so a move out of Flagged has to be enqueued a source
        // folder at a time or every row would claim to have come from
        // whichever folder was named first. That is one `SELECT DISTINCT`
        // over the same index, so it costs the folders the set spans and not
        // the messages in it.
        let sources = match from {
            Some(from) => vec![from],
            None => {
                let mut sources = MessageRepository::new(connection)
                    .mailboxes_of_set(&set)
                    .map_err(store_failure)?;
                // Whatever is already filed where it is going is not moving.
                // Enqueueing it would tell the server to move a message onto
                // itself and put it inside the run `u` takes back.
                sources.retain(|mailbox| *mailbox != destination);
                sources
            }
        };
        if sources.is_empty() {
            return Err(CommandError::rejected("There is nothing here to move"));
        }

        let at = Utc::now();
        let transaction = connection.transaction().map_err(store_failure)?;
        let mut count = 0usize;
        let mut inverse = Vec::new();
        let mut reloaded = vec![destination];
        for source in sources {
            // Each group is still a predicate: `within` is a column
            // comparison, not a list of ids.
            let group = set.clone().within(source);
            let operation = match kind {
                UndoKind::Delete => Operation::Delete {
                    from: source,
                    trash: destination,
                },
                _ => Operation::Move {
                    from: source,
                    to: destination,
                },
            };
            // Enqueue before moving: the predicate for a whole-view selection
            // names the rows as they are now, and after the move it does not.
            let Some(range) = OperationQueueRepository::new(&transaction)
                .enqueue_set(account, &group, &operation, at)
                .map_err(store_failure)?
            else {
                continue;
            };
            count += MessageRepository::new(&transaction)
                .move_set(&group, destination)
                .map_err(store_failure)?;
            reloaded.push(source);
            inverse.push(Command::Move {
                target: MessageTarget::Batch {
                    range,
                    account,
                    from: Some(destination),
                },
                // Back to the folder this group came from, not to one folder
                // for all of them.
                to: Some(source),
            });
        }
        if count == 0 {
            // Nothing was written, so nothing has to be unwound — dropping
            // the transaction rolls back the queue rows any empty group
            // might have left.
            return Err(CommandError::rejected("There is nothing here to move"));
        }
        transaction.commit().map_err(store_failure)?;

        Ok(Applied {
            account,
            kind,
            messages: Vec::new(),
            count,
            removed: Vec::new(),
            arrived: None,
            // Every end reloads. None of them can be told which rows moved,
            // and the one they moved into is as changed as the ones they left.
            reloaded,
            changed: Vec::new(),
            inverse,
        })
    }

    /// Move messages this handler has already read.
    fn relocate_rows(
        &self,
        connection: &mut PooledConnection,
        rows: Vec<Message>,
        to: Destination,
        kind: UndoKind,
    ) -> Result<Applied, CommandError> {
        // Every row here belongs to one account — `relocate` grouped them —
        // so the role destination resolves once, against that account's own
        // folders rather than against whichever account came first in a
        // selection that might span several (#182).
        let account = rows[0].account_id;
        debug_assert!(
            rows.iter().all(|row| row.account_id == account),
            "relocate_rows is single-account by construction; `relocate` groups"
        );
        // A destination in another account is not a move at all — there is
        // no server-side operation between two servers. It becomes the
        // three-phase saga of ADR 0005 Q9, and the row here is only its
        // local-first half: the message appears there and leaves here
        // immediately, and the saga reconciles.
        if let Destination::Mailbox(mailbox) = to {
            let target = MailboxRepository::new(connection)
                .get(mailbox)
                .map_err(store_failure)?
                .ok_or_else(|| CommandError::rejected("That folder no longer exists"))?;
            if target.account_id != account {
                return self.cross_account_relocate(connection, rows, &target, kind);
            }
        }
        let destination = mailbox_for(connection, account, to)?;

        let mut by_source: BTreeMap<MailboxId, Vec<MessageId>> = BTreeMap::new();
        for message in &rows {
            // Already filed: not a failure, just nothing to do for this row.
            if message.mailbox_id == destination {
                continue;
            }
            by_source
                .entry(message.mailbox_id)
                .or_default()
                .push(message.id);
        }
        if by_source.is_empty() {
            return Err(CommandError::rejected("Already there"));
        }

        let at = Utc::now();
        let transaction = connection.transaction().map_err(store_failure)?;
        {
            let messages = MessageRepository::new(&transaction);
            let queue = OperationQueueRepository::new(&transaction);
            for (source, ids) in &by_source {
                let operation = match kind {
                    UndoKind::Delete => Operation::Delete {
                        from: *source,
                        trash: destination,
                    },
                    _ => Operation::Move {
                        from: *source,
                        to: destination,
                    },
                };
                // The local write and its queue row in one transaction: a
                // queue row without its write tells the server about
                // something the user never saw, and a write without its row
                // silently never reaches the server. The queue row comes
                // FIRST — enqueue snapshots the rows' server coordinates,
                // and the move nulls them (#289).
                //
                // One statement per source mailbox rather than one per
                // message: a multi-select spanning a handful of folders is a
                // handful of `enqueue_many` calls, not one `enqueue` per row.
                queue
                    .enqueue_many(account, ids, &operation, at)
                    .map_err(store_failure)?;
                messages.move_to(ids, destination).map_err(store_failure)?;
            }
        }
        transaction.commit().map_err(store_failure)?;

        let removed: Vec<(MailboxId, Vec<MessageId>)> = by_source.into_iter().collect();
        let messages: Vec<MessageId> = removed
            .iter()
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect();
        Ok(Applied {
            account,
            kind,
            count: messages.len(),
            messages,
            inverse: removed
                .iter()
                .map(|(source, ids)| Command::Move {
                    target: MessageTarget::Messages(ids.clone()),
                    to: Some(*source),
                })
                .collect(),
            arrived: Some(destination),
            reloaded: Vec::new(),
            changed: Vec::new(),
            removed,
        })
    }

    /// The local-first half of a cross-account move (#188, ADR 0005 Q9).
    ///
    /// One saga per message: a provisional row appears in the target
    /// account's mailbox at once, the source row is hidden at once, and two
    /// queue operations — the copy on the target's queue, the remove on the
    /// source's — carry the server work. The remove cannot run until the
    /// copy is confirmed; that ordering lives in the saga table and its
    /// drainer, not here.
    ///
    /// No undo entry yet: the inverse of a saga is the inverse saga, driven
    /// from the confirmed target UID, and `u` offering a plain move back
    /// would be offering something the servers cannot do. Filed as the
    /// follow-up this function's PR names.
    fn cross_account_relocate(
        &self,
        connection: &mut PooledConnection,
        rows: Vec<Message>,
        target: &postio_model::Mailbox,
        kind: UndoKind,
    ) -> Result<Applied, CommandError> {
        let account = rows[0].account_id;
        let at = Utc::now();
        let transaction = connection.transaction().map_err(store_failure)?;
        let mut moved: Vec<MessageId> = Vec::new();
        let mut by_source: BTreeMap<MailboxId, Vec<MessageId>> = BTreeMap::new();
        {
            let messages = MessageRepository::new(&transaction);
            let queue = OperationQueueRepository::new(&transaction);
            let sagas = postio_storage::repository::CrossAccountMoveRepository::new(&transaction);
            for row in &rows {
                // The provisional copy the user sees in the target at once.
                // The raw `.eml` and the payloads are content-addressed, so
                // the copy shares those bytes; the body text is copied, since
                // ADR 0020 put it in the row. That is a second copy of a value
                // whose median is 325 bytes, against a saga that already holds
                // the whole message's raw source.
                let mut copy = row.clone();
                copy.id = MessageId::UNASSIGNED;
                copy.account_id = target.account_id;
                copy.mailbox_id = target.id;
                copy.thread_id = None;
                copy.server.uid = None;
                copy.server.uid_validity = None;
                let copy_id = messages.create(&mut copy).map_err(store_failure)?;
                if let Some(body) = messages.body(row.id).map_err(store_failure)? {
                    messages
                        .set_body(copy_id, &body, row.sync.body_state)
                        .map_err(store_failure)?;
                }

                let saga = sagas
                    .create(&postio_storage::repository::NewCrossAccountMove {
                        source_message: row.id,
                        source_account: account,
                        source_mailbox: row.mailbox_id,
                        target_account: target.account_id,
                        target_mailbox: target.id,
                        target_message: Some(copy_id),
                        raw_blob_id: row
                            .raw_blob_id
                            .as_ref()
                            .map(|blob| blob.as_str().to_owned()),
                        rfc_message_id: row
                            .rfc_message_id
                            .as_ref()
                            .map(|id| id.as_str().to_owned()),
                    })
                    .map_err(store_failure)?;

                queue
                    .enqueue(
                        target.account_id,
                        postio_model::OperationTarget::Message(copy_id),
                        &Operation::CrossAccountCopy { saga },
                        at,
                    )
                    .map_err(store_failure)?;
                queue
                    .enqueue(
                        account,
                        postio_model::OperationTarget::Message(row.id),
                        &Operation::CrossAccountRemove { saga },
                        at,
                    )
                    .map_err(store_failure)?;
                messages
                    .set_deleted_locally(&[row.id], true)
                    .map_err(store_failure)?;

                moved.push(row.id);
                by_source.entry(row.mailbox_id).or_default().push(row.id);
            }
        }
        transaction.commit().map_err(store_failure)?;

        Ok(Applied {
            account,
            kind,
            count: moved.len(),
            messages: moved,
            // Deliberately empty: see the doc comment. `u` says "nothing to
            // undo" rather than pretending a saga is a move.
            inverse: Vec::new(),
            arrived: None,
            reloaded: Vec::new(),
            changed: Vec::new(),
            removed: by_source.into_iter().collect(),
        })
    }

    /// Hide the targeted messages from every ordinary list until `until`.
    ///
    /// Row-selection only, the same way `MessageTarget::Selection` bottoms
    /// out for most verbs: a whole-mailbox snooze would need a
    /// `MessageSet::Snoozed`-shaped bulk predicate of its own, which nothing
    /// asks for yet (`view_scope` in `postio-app` deliberately does not offer
    /// `Ctrl+A` inside the Snoozed view either, for the same reason).
    ///
    /// Local only — no queue row, no server ever hears about a snooze — so
    /// unlike [`Self::set_flag`] there is nothing to enqueue, just the write
    /// and the repaint. `removed` rather than `changed`: the row does not
    /// merely look different, it leaves the list its mailbox is showing,
    /// which needs `Event::MessagesRemoved` rather than a per-row patch.
    fn snooze(
        &self,
        target: &MessageTarget,
        until: chrono::DateTime<Utc>,
    ) -> Result<Applied, CommandError> {
        let (connection, _permit) = self.connect()?;
        let rows = match self.aim(&connection, target)? {
            Aim::Rows(rows) => rows,
            Aim::Bulk(_) => {
                return Err(CommandError::rejected("Select the messages to snooze"));
            }
        };
        let account = rows[0].account_id;
        let ids: Vec<MessageId> = rows.iter().map(|message| message.id).collect();
        MessageRepository::new(&connection)
            .snooze(&ids, until)
            .map_err(store_failure)?;

        let mut removed: BTreeMap<MailboxId, Vec<MessageId>> = BTreeMap::new();
        for message in &rows {
            removed
                .entry(message.mailbox_id)
                .or_default()
                .push(message.id);
        }
        Ok(Applied {
            account,
            kind: UndoKind::Snooze,
            count: ids.len(),
            messages: ids.clone(),
            removed: removed.into_iter().collect(),
            arrived: None,
            reloaded: Vec::new(),
            changed: Vec::new(),
            inverse: vec![Command::Unsnooze {
                target: MessageTarget::Messages(ids),
            }],
        })
    }

    /// Cancel a snooze immediately, rather than waiting for it to come due.
    ///
    /// `reloaded` rather than `arrived`: the rows reappear in whatever
    /// mailbox they were already filed in — nothing moved — so this is a
    /// wholesale repaint of each affected folder, potentially more than one
    /// at once for a selection spanning several.
    fn unsnooze(&self, target: &MessageTarget) -> Result<Applied, CommandError> {
        let (connection, _permit) = self.connect()?;
        let rows = match self.aim(&connection, target)? {
            Aim::Rows(rows) => rows,
            Aim::Bulk(_) => {
                return Err(CommandError::rejected("Select the messages to unsnooze"));
            }
        };
        let account = rows[0].account_id;
        let ids: Vec<MessageId> = rows.iter().map(|message| message.id).collect();
        MessageRepository::new(&connection)
            .unsnooze(&ids)
            .map_err(store_failure)?;

        let reloaded: Vec<MailboxId> = rows
            .iter()
            .map(|message| message.mailbox_id)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        Ok(Applied {
            account,
            kind: UndoKind::Unsnooze,
            count: ids.len(),
            messages: ids.clone(),
            removed: Vec::new(),
            arrived: None,
            reloaded,
            changed: Vec::new(),
            inverse: vec![Command::Snooze {
                target: MessageTarget::Messages(ids),
            }],
        })
    }

    /// Set or clear one flag across the target.
    ///
    /// `want` of `None` toggles, and a toggle over more than one row means
    /// *make them agree*: if every row already carries the flag it comes off
    /// them all, otherwise it goes onto them all. The alternative — flipping
    /// each row independently — turns one keystroke over a mixed selection
    /// into a result nobody can predict without reading every row first.
    ///
    /// **Across accounts that is still one decision.** A unified selection
    /// arrives as one bulk unit per account, and deciding the toggle inside
    /// each of them would let a single keystroke flag one account while
    /// unflagging another — the unpredictable result the rule above exists to
    /// prevent, one level up (#811).
    fn set_flag(
        &self,
        target: &MessageTarget,
        flag: Flag,
        want: Option<bool>,
    ) -> Result<Vec<Applied>, CommandError> {
        let (mut connection, _permit) = self.connect()?;
        match self.aim(&connection, target)? {
            Aim::Rows(rows) => self
                .set_flag_rows(&mut connection, rows, flag, want)
                .map(|one| vec![one]),
            Aim::Bulk(units) => {
                let wanted = match want {
                    Some(wanted) => wanted,
                    None => self.agreeing_on(&connection, &units, &flag)?,
                };
                let mut applied = Vec::with_capacity(units.len());
                let mut nothing_to_do = None;
                for unit in units {
                    match self.set_flag_set(
                        &mut connection,
                        unit.set,
                        unit.account,
                        unit.from,
                        flag.clone(),
                        Some(wanted),
                    ) {
                        Ok(one) => applied.push(one),
                        // This account already agreed. That is not a failure
                        // for the accounts that did not — but if none of them
                        // had anything to do, the user still deserves the
                        // sentence.
                        Err(CommandError::Rejected(reason)) => nothing_to_do = Some(reason),
                        Err(error) => return Err(error),
                    }
                }
                match (applied.is_empty(), nothing_to_do) {
                    (true, Some(reason)) => Err(CommandError::Rejected(reason)),
                    _ => Ok(applied),
                }
            }
        }
    }

    /// Which way a toggle over `units` goes: on, unless they all carry it.
    ///
    /// One `count(*)` over an index per unit, never a read — the same trick
    /// [`set_flag_set`] uses within one account, asked across the whole
    /// selection so the answer is one answer.
    ///
    /// [`set_flag_set`]: Actions::set_flag_set
    fn agreeing_on(
        &self,
        connection: &PooledConnection,
        units: &[BulkUnit],
        flag: &Flag,
    ) -> Result<bool, CommandError> {
        let column = ColumnFlag::of(flag)
            .ok_or_else(|| CommandError::rejected("That flag does not work on a whole mailbox"))?;
        let repository = MessageRepository::new(connection);
        for unit in units {
            let disagreeing = repository
                .count_set(&unit.set.clone().with_flag(column, false))
                .map_err(store_failure)?;
            if disagreeing > 0 {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Set or clear one flag across a whole mailbox, without naming a row.
    ///
    /// The bulk twin of [`set_flag_rows`], and the flag half of `postio-agr.1`.
    /// Two indexed counts and three statements, whatever the mailbox holds:
    /// count both sides of the flag, write the queue rows, write the flag.
    ///
    /// # Why both sides get counted
    ///
    /// A toggle means *make them agree*, and over a predicate the only way to
    /// know whether they already do is to ask how many disagree — which is a
    /// `count(*)` over an index rather than a read. The other count is what
    /// the toast says, and the two together are also how an empty mailbox and
    /// a mailbox that already agrees tell themselves apart, which are
    /// different sentences.
    ///
    /// # Why the queue rows are written over the rows that disagree
    ///
    /// The set narrowed by [`MessageSet::with_flag`] is exactly the rows this
    /// changes. Enqueueing over the wider set would tell the server about
    /// messages that already carried the flag and — because the run of queue
    /// rows *is* the undo set — would make `u` clear a flag this action never
    /// set.
    ///
    /// [`set_flag_rows`]: Actions::set_flag_rows
    fn set_flag_set(
        &self,
        connection: &mut PooledConnection,
        set: MessageSet,
        account: AccountId,
        from: Option<MailboxId>,
        flag: Flag,
        want: Option<bool>,
    ) -> Result<Applied, CommandError> {
        // Only the two flags with a column of their own can be written this
        // way; nothing reaches here with another, because `Flag` and
        // `MarkUnread` are the only verbs that flag anything.
        let column = ColumnFlag::of(&flag)
            .ok_or_else(|| CommandError::rejected("That flag does not work on a whole mailbox"))?;
        let repository = MessageRepository::new(connection);
        let carrying = |present: bool| set.clone().with_flag(column, present);
        let without = repository
            .count_set(&carrying(false))
            .map_err(store_failure)? as usize;
        let with = repository
            .count_set(&carrying(true))
            .map_err(store_failure)? as usize;
        if without + with == 0 {
            return Err(CommandError::rejected("There is nothing here to change"));
        }

        // The toggle rule, over a predicate: they disagree, so the flag goes
        // on; they all carry it, so it comes off.
        let wanted = want.unwrap_or(without > 0);
        let changing = carrying(!wanted);
        let count = if wanted { without } else { with };
        if count == 0 {
            return Err(CommandError::rejected("Already set"));
        }

        // Which folders the list has to reload afterwards, answered *now*.
        //
        // A folder selection knows. A smart folder does not, and it cannot be
        // asked later: `MessageSet::Flagged`'s predicate is not stable across
        // this write. Archiving everything flagged leaves the flag alone, so
        // the set still matches afterwards — but a bulk *unflag* over Flagged
        // empties its own predicate, and a query run after the commit would
        // answer "no folders" for the change that touched the most.
        let reloaded = match from {
            Some(from) => vec![from],
            None => MessageRepository::new(connection)
                .mailboxes_of_set(&changing)
                .map_err(store_failure)?,
        };

        let one: FlagSet = std::iter::once(flag.clone()).collect();
        let operation = if wanted {
            Operation::SetFlags { flags: one }
        } else {
            Operation::ClearFlags { flags: one }
        };
        let at = Utc::now();
        let transaction = connection.transaction().map_err(store_failure)?;
        // Enqueue before writing, as a bulk move does: the predicate is "the
        // rows that disagree", and after the write none of them do.
        let range = OperationQueueRepository::new(&transaction)
            .enqueue_set(account, &changing, &operation, at)
            .map_err(store_failure)?
            .ok_or_else(|| CommandError::rejected("Already set"))?;
        MessageRepository::new(&transaction)
            .set_flag_on_set(&changing, column, wanted)
            .map_err(store_failure)?;
        transaction.commit().map_err(store_failure)?;

        // Nothing moved, so `from` is carried through untouched: `None` in a
        // smart folder, where the rows stayed in as many folders as they
        // started in. Undo aims at the queue run either way.
        let target = MessageTarget::Batch {
            range,
            account,
            from,
        };
        let inverse = match flag {
            Flag::Seen => Command::MarkUnread {
                target,
                unread: Some(wanted),
            },
            _ => Command::Flag {
                target,
                flagged: Some(!wanted),
            },
        };
        Ok(Applied {
            account,
            kind: kind_for(&flag, wanted),
            messages: Vec::new(),
            count,
            removed: Vec::new(),
            arrived: None,
            // The rows did not go anywhere, but naming the ones that changed
            // is the read this path exists to avoid — so the list reloads the
            // folders rather than being handed eighty thousand ids.
            reloaded,
            changed: Vec::new(),
            inverse: vec![inverse],
        })
    }

    /// Set or clear one flag across messages this handler has already read.
    fn set_flag_rows(
        &self,
        connection: &mut PooledConnection,
        rows: Vec<Message>,
        flag: Flag,
        want: Option<bool>,
    ) -> Result<Applied, CommandError> {
        let account = rows[0].account_id;
        let wanted =
            want.unwrap_or_else(|| !rows.iter().all(|message| message.flags.contains(&flag)));

        let touched: Vec<&Message> = rows
            .iter()
            .filter(|message| message.flags.contains(&flag) != wanted)
            .collect();
        if touched.is_empty() {
            return Err(CommandError::rejected("Already set"));
        }

        let one: FlagSet = std::iter::once(flag.clone()).collect();
        let at = Utc::now();
        let mut siblings: Vec<MessageId> = Vec::new();
        let transaction = connection.transaction().map_err(store_failure)?;
        {
            let messages = MessageRepository::new(&transaction);
            let queue = OperationQueueRepository::new(&transaction);
            for message in &touched {
                let mut flags = message.flags.clone();
                if wanted {
                    flags.insert(flag.clone());
                } else {
                    flags.remove(&flag);
                }
                messages
                    .set_flags(message.id, &flags, FlagSource::Local)
                    .map_err(store_failure)?;
                let operation = if wanted {
                    Operation::SetFlags { flags: one.clone() }
                } else {
                    Operation::ClearFlags { flags: one.clone() }
                };
                queue
                    .enqueue(
                        account,
                        OperationTarget::Message(message.id),
                        &operation,
                        at,
                    )
                    .map_err(store_failure)?;
            }

            // The threads those rows belong to keep two things the rows
            // alone cannot (#754): denormalised aggregates the
            // account-scoped page reads straight off `threads`, and the
            // membership that names the row the list draws for the
            // conversation. Recompute the first and collect the second
            // while the write is still one transaction, so nothing can
            // observe a read message under an unread thread.
            let threads = ThreadRepository::new(&transaction);
            let mut conversations: Vec<ThreadId> = touched
                .iter()
                .filter_map(|message| message.thread_id)
                .collect();
            conversations.sort_unstable();
            conversations.dedup();
            for thread in &conversations {
                threads.recompute(*thread).map_err(store_failure)?;
                for row in threads
                    .messages(*thread, ThreadOrder::Oldest)
                    .map_err(store_failure)?
                {
                    siblings.push(row.id);
                }
            }
        }
        transaction.commit().map_err(store_failure)?;

        let changed: Vec<MessageId> = touched.iter().map(|message| message.id).collect();
        // What repaints is wider than what changed: the list's row for a
        // conversation is its representative, and `pages_holding` resolves
        // an announcement against *row* ids — so a non-representative
        // member marked read refetched no page and the row went on drawing
        // unread (#754). Naming the siblings is what makes the row
        // findable. The inverse and the queue stay exactly as wide as the
        // touch: repainting is not something `u` takes back, and the server
        // must not hear about flags nobody changed.
        let mut repaint = changed.clone();
        repaint.extend(siblings);
        repaint.sort_unstable();
        repaint.dedup();
        // Every touched row held the opposite value — that is what "touched"
        // means here — so one command takes all of them back.
        let inverse = match flag {
            Flag::Seen => Command::MarkUnread {
                target: MessageTarget::Messages(changed.clone()),
                unread: Some(wanted),
            },
            _ => Command::Flag {
                target: MessageTarget::Messages(changed.clone()),
                flagged: Some(!wanted),
            },
        };
        Ok(Applied {
            account,
            kind: kind_for(&flag, wanted),
            count: changed.len(),
            messages: changed,
            removed: Vec::new(),
            arrived: None,
            reloaded: Vec::new(),
            changed: repaint,
            inverse: vec![inverse],
        })
    }

    // ── Saying what happened ─────────────────────────────────────────────

    /// Emit what the panes repaint from, and record what `u` takes back.
    fn announce(&self, applied: Applied, events: &EventSink, recording: Recording) {
        let account = applied.account;
        for (mailbox, messages) in &applied.removed {
            events.emit(Event::MessagesRemoved {
                account,
                mailbox: *mailbox,
                messages: messages.clone(),
            });
        }
        if let Some(mailbox) = applied.arrived {
            // The folder they landed in is longer than it was, and it may be
            // the one on screen — an undone archive has to reappear now, not
            // at the next sync.
            events.emit(Event::MessageListChanged { account, mailbox });
        }
        for mailbox in &applied.reloaded {
            events.emit(Event::MessageListChanged {
                account,
                mailbox: *mailbox,
            });
        }
        if !applied.changed.is_empty() {
            events.emit(Event::MessagesChanged {
                account,
                messages: applied.changed.clone(),
            });
        }
        if !recording.records() {
            return;
        }
        // A bulk unit knows its size and not its members, so it is recorded as
        // one — `UndoEntry::new` would take the count from a list of rows that
        // is deliberately empty and the toast would say nothing happened.
        let entry = if applied.messages.len() == applied.count {
            UndoEntry::new(applied.kind, applied.messages, applied.inverse)
        } else {
            UndoEntry::bulk(applied.kind, applied.count, applied.inverse)
        };
        // The description comes back from the stack rather than from the
        // entry handed to it: a burst coalesces into the unit already there,
        // and the toast has to say twelve when the unit holds twelve.
        let description = self.stack().record(entry).description();
        events.emit(Event::ActionCompleted {
            description,
            undoable: true,
        });
    }

    // ── Resolving what a verb is aimed at ────────────────────────────────

    /// The messages an invocation acts on, read fresh.
    ///
    /// Never empty on success: a verb with nothing to act on is a rejection,
    /// not a no-op, or `a` on an empty list would look like the application
    /// ignoring the key.
    fn rows(
        &self,
        connection: &PooledConnection,
        ids: Vec<MessageId>,
    ) -> Result<Vec<Message>, CommandError> {
        let repository = MessageRepository::new(connection);
        let mut rows = Vec::with_capacity(ids.len());
        for id in ids {
            let Some(message) = repository.get(id).map_err(store_failure)? else {
                // The list is windowed over a database another half of the
                // application is writing; a row can be gone by the time a
                // key press reaches here.
                continue;
            };
            rows.push(message);
        }
        if rows.is_empty() {
            return Err(CommandError::rejected("Those messages are no longer here"));
        }
        Ok(rows)
    }

    /// What a verb will act on: rows, or a predicate over them.
    ///
    /// The two whole-mailbox cases never become rows here. `Everything` is the
    /// selection the user built with `Ctrl+A`; `Batch` is undo taking one of
    /// those back. Both carry through to the store as SQL.
    fn aim(
        &self,
        connection: &PooledConnection,
        target: &MessageTarget,
    ) -> Result<Aim, CommandError> {
        let resolved = self
            .state
            .read(|app| app.resolve(target))
            .ok_or_else(|| CommandError::rejected("Nothing selected"))?;
        let units = match resolved {
            Resolved::Messages(ids) => return self.rows(connection, ids).map(Aim::Rows),
            Resolved::Thread(thread) => {
                let ids = thread_messages(connection, thread)?;
                return self.rows(connection, ids).map(Aim::Rows);
            }
            Resolved::Threads(threads) => {
                // The unified group: every member thread's messages, in one
                // aim — `relocate`'s per-account split (#182) is what turns
                // the rows into one unit per account queue.
                let mut ids = Vec::new();
                for thread in threads {
                    ids.extend(thread_messages(connection, thread)?);
                }
                return self.rows(connection, ids).map(Aim::Rows);
            }
            Resolved::Everything { scope, except } => match scope {
                ViewScope::Mailbox(mailbox) => vec![BulkUnit {
                    set: MessageSet::InMailbox { mailbox, except },
                    // One row read, and it is a folder rather than a message:
                    // the account is needed to find the Archive, and there is
                    // no message to ask.
                    account: account_of(connection, mailbox)?,
                    from: Some(mailbox),
                }],
                // A smart folder carries its own account, so there is no
                // folder to read one off — and no one folder its rows are in.
                ViewScope::Flagged(account) => vec![BulkUnit {
                    set: MessageSet::Flagged { account, except },
                    account,
                    from: None,
                }],
                // The aggregate, split into one predicate per account it was
                // scoped to. The accounts come off the scope and are not
                // looked up again: they are what the view could show when the
                // gesture was made, and an account that has reconnected since
                // was not part of the selection the user was shown (#811).
                ViewScope::Unified { accounts } => accounts
                    .into_iter()
                    .map(|account| BulkUnit {
                        set: MessageSet::InAccounts {
                            accounts: vec![account],
                            except: except.clone(),
                        },
                        account,
                        from: None,
                    })
                    .collect(),
            },
            Resolved::Batch {
                range,
                account,
                from,
            } => vec![BulkUnit {
                set: MessageSet::Queued(range),
                account,
                from,
            }],
        };
        // An aggregate that could reach no account at all resolves to no
        // units, and a verb that quietly did nothing would be exactly the
        // silence this issue is about.
        if units.is_empty() {
            return Err(CommandError::rejected("Nothing selected"));
        }
        Ok(Aim::Bulk(units))
    }

    /// The thread `A` means: the one the focused message belongs to.
    ///
    /// A whole-mailbox selection has no such message — `Ctrl+A` then `A` is a
    /// gesture with no answer rather than a bulk one, because "the thread of
    /// everything" is not a thing — so it is asked for rather than guessed at.
    fn thread_in_view(&self) -> Result<ThreadId, CommandError> {
        let (connection, _permit) = self.connect()?;
        let rows = match self.aim(&connection, &MessageTarget::Selection)? {
            Aim::Rows(rows) => rows,
            Aim::Bulk(_) => {
                return Err(CommandError::rejected(
                    "Pick a message, and `A` archives its thread",
                ));
            }
        };
        rows[0]
            .thread_id
            .ok_or_else(|| CommandError::rejected("That message is not in a thread"))
    }

    /// Settle a draft whose send could not be confirmed: it did arrive.
    ///
    /// ADR 0021 Decision 3, #674. Postio resolves most of these by itself --
    /// the next sync of Sent finding the reserved `Message-ID` -- but where
    /// the server files no copy the question stands, and the only person who
    /// can answer it is the one who asked the recipient. Without this they
    /// have two exits and both are wrong: discard throws the message away,
    /// and sending again duplicates it.
    ///
    /// `None` means the draft the list is on, resolved through the message
    /// its row carries, so the palette entry works where a draft is visible.
    fn mark_sent(&self, draft: Option<DraftId>) -> Result<Applied, CommandError> {
        let (connection, _permit) = self.connect()?;
        let drafts = DraftRepository::new(&connection);

        let draft = match draft {
            Some(id) => drafts
                .get(id)
                .map_err(store_failure)?
                .ok_or_else(|| CommandError::rejected("That draft is no longer here"))?,
            None => {
                let rows = match self.aim(&connection, &MessageTarget::Selection)? {
                    Aim::Rows(rows) => rows,
                    Aim::Bulk { .. } => {
                        return Err(CommandError::rejected("Pick the message this is about"));
                    }
                };
                drafts
                    .by_message(rows[0].id)
                    .map_err(store_failure)?
                    .ok_or_else(|| {
                        CommandError::rejected("That row is not a draft Postio is unsure about")
                    })?
            }
        };

        // Only the state this exists for. Saying "it arrived" about a draft
        // still being edited, or one already sent, is not a correction --
        // it is a way to lose the draft, since `Sent` is what stops it being
        // offered for editing.
        if draft.state != DraftState::Unconfirmed {
            return Err(CommandError::rejected(
                "Only a send Postio could not confirm can be marked as sent",
            ));
        }

        drafts
            .set_state(draft.id, DraftState::Sent)
            .map_err(store_failure)?;
        Ok(Applied {
            account: draft.account_id,
            kind: UndoKind::MarkedSent,
            count: 1,
            messages: Vec::new(),
            removed: Vec::new(),
            arrived: None,
            reloaded: Vec::new(),
            changed: Vec::new(),
            // No inverse, and #674 asked for one -- worth saying why.
            //
            // An inverse is a `Command`, every `Command` has a `CommandId`,
            // and every `CommandId` needs a registry entry with a keyboard
            // binding (PRODUCT.md §8, asserted by `command_registry.rs`). A
            // "mark unconfirmed again" verb would be a key in the reference
            // and a row in the palette for something no user ever reaches
            // for -- a second command invented to satisfy the shape of the
            // first.
            //
            // And undo is the wrong instrument anyway: this settles a claim
            // about the world rather than changing the world. If the answer
            // was wrong, the honest correction is to send the message again,
            // which is a real act with a real effect, not to un-know
            // something.
            inverse: Vec::new(),
        })
    }

    /// A connection to write a verb through, and the right to write it ahead
    /// of bulk background work.
    ///
    /// # Why a verb needs a permit at all (#425)
    ///
    /// Every verb in this module is a write somebody is waiting for. SQLite
    /// takes one writer at a time and settles a collision with `busy_timeout`,
    /// which is a retry loop rather than a queue — no ordering and no
    /// fairness. During a first sync two backfill lanes commit batches back to
    /// back with no gap between them, so an archive keystroke woke, lost,
    /// backed off further, and lost again: **1.8 seconds** to write one row,
    /// with the connection pool almost idle the whole time. The permit is what
    /// puts a person's write in front of that, by construction rather than by
    /// hoping for a gap. See [`postio_storage::WriteGate`].
    ///
    /// # Why here and not around `run`
    ///
    /// The gate is not re-entrant, and the connection has to be taken *before*
    /// the permit or a permit-holder can end up waiting on the pool for a
    /// connection a permit-waiter is holding. Both rules are satisfied by
    /// scoping the pair to one call: the two verbs that resolve a target
    /// before acting on it (`ArchiveThread`, `Undo`) do so in sequence, never
    /// nested, so each step takes its own permit and releases it.
    ///
    /// The permit covers this module's reads as well as its writes. That costs
    /// a backfill nothing measurable — they are indexed point reads on a
    /// human's schedule, not scans — and it keeps the pairing impossible to
    /// get wrong.
    fn connect(&self) -> Result<(PooledConnection, WritePermit), CommandError> {
        let connection = self.database.connection().map_err(store_failure)?;
        let permit = self
            .database
            .write_gate()
            .acquire(WritePriority::Interactive);
        Ok((connection, permit))
    }

    fn stack(&self) -> std::sync::MutexGuard<'_, UndoStack> {
        // A panicking handler must not cost the application its history; the
        // bus has already reported the panic as an error event.
        self.undo
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Which folder a relocation lands in.
fn mailbox_for(
    connection: &PooledConnection,
    account: AccountId,
    to: Destination,
) -> Result<MailboxId, CommandError> {
    match to {
        Destination::Mailbox(id) => Ok(id),
        Destination::Role(role) => MailboxRepository::new(connection)
            .by_role(account, role)
            .map_err(store_failure)?
            .map(|mailbox| mailbox.id)
            .ok_or_else(|| {
                // Not every server has one, and inventing a folder because a
                // key was pressed is not this command's decision to make.
                CommandError::rejected(format!(
                    "This account has no {} folder",
                    role.as_str().to_lowercase()
                ))
            }),
    }
}

fn thread_messages(
    connection: &PooledConnection,
    thread: ThreadId,
) -> Result<Vec<MessageId>, CommandError> {
    let rows = ThreadRepository::new(connection)
        .messages(thread, ThreadOrder::Oldest)
        .map_err(store_failure)?;
    if rows.is_empty() {
        return Err(CommandError::rejected("That thread is empty"));
    }
    Ok(rows.into_iter().map(|row| row.id).collect())
}

/// What the toast calls this, which depends on which way the flag went.
fn kind_for(flag: &Flag, wanted: bool) -> UndoKind {
    match (flag, wanted) {
        (Flag::Seen, true) => UndoKind::MarkRead,
        (Flag::Seen, false) => UndoKind::MarkUnread,
        (_, true) => UndoKind::Flag,
        (_, false) => UndoKind::Unflag,
    }
}

/// A store that would not take the write.
///
/// Generic over the error so that `postio-storage`'s and SQLite's own — the
/// transaction boundaries are the latter — reach the user the same way.
///
/// The sentence on screen says what happened without saying what to; the
/// detail goes to stderr, where it carries SQL rather than anyone's mail.
fn store_failure(error: impl std::fmt::Display) -> CommandError {
    tracing::error!(%error, "the local store refused a write");
    CommandError::failed("Could not save that change")
}

/// Which account a folder belongs to.
///
/// One row read, and it is a folder rather than a message: a whole-folder
/// selection names no message to ask, and the account is what finds the
/// Archive. A scope that already carries its account does not come here.
fn account_of(
    connection: &PooledConnection,
    mailbox: MailboxId,
) -> Result<AccountId, CommandError> {
    Ok(MailboxRepository::new(connection)
        .get(mailbox)
        .map_err(store_failure)?
        .ok_or_else(|| CommandError::rejected("That folder is no longer here"))?
        .account_id)
}

/// A bus answering this module's verbs and nothing else.
///
/// The application composes a larger one — see [`wire`] — so this exists for
/// the tests below, which are about the verbs rather than about the wiring.
#[cfg(test)]
pub fn dispatcher(actions: Actions) -> postio_core::Dispatcher {
    wire(DispatcherBuilder::new(), actions).build()
}

/// Register every verb this module answers on `builder`.
///
/// The builder is taken rather than made so that a verb belonging to another
/// module — `Refresh`, which is a network pass rather than a local-first write
/// — can join the same bus without this module knowing about it.
pub fn wire(builder: DispatcherBuilder, actions: Actions) -> DispatcherBuilder {
    builder.on_each(WIRED.iter().copied(), move |invocation| {
        let actions = actions.clone();
        // Synchronous on purpose: a local-first verb is a handful of
        // indexed writes and their queue rows, and the bus awaits each
        // handler so app state and the undo stack see a total order.
        // Anything that could actually take time belongs on a spawned
        // task reporting through its own events.
        async move { actions.run(&invocation.command, &invocation.events()) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Utc;
    use postio_core::bridge::{EventStream, event_channel};
    use postio_core::state::AppState;
    use postio_model::mailbox::MailboxRole;
    use postio_model::{Account, Flag, MailboxId, Message, MessageId, Operation, OperationTarget};
    use postio_storage::repository::{
        AccountRepository, MailboxRepository, MessageRepository, OperationQueueRepository,
        ThreadRepository,
    };
    use postio_storage::test_support;

    /// An account with the three folders every verb here reaches for, and a
    /// bus over it.
    struct World {
        database: Database,
        account: Account,
        inbox: MailboxId,
        archive: MailboxId,
        trash: MailboxId,
        actions: Actions,
        state: SharedState,
        sink: EventSink,
        events: EventStream,
        /// Where mirroring the window's own state goes: nowhere. See
        /// `commands::mirror`.
        quiet: EventSink,
    }

    /// A second account, for the tests that are about which one a row is in.
    struct Elsewhere {
        account: Account,
        inbox: MailboxId,
        archive: MailboxId,
    }

    fn world() -> World {
        let database = test_support::memory();
        let (account, inbox, archive, trash) = {
            let connection = database.connection().expect("a connection");
            let (account, inbox) = test_support::account_with_inbox(&connection);
            let archive = test_support::mailbox(&connection, &account, "Archive").id;
            let trash = test_support::mailbox(&connection, &account, "Trash").id;
            (account, inbox, archive, trash)
        };
        let state = SharedState::default();
        let (sink, events) = event_channel();
        let (quiet, _) = event_channel();
        World {
            actions: Actions::new(database.clone(), state.clone()),
            database,
            account,
            inbox,
            archive,
            trash,
            state,
            sink,
            events,
            quiet,
        }
    }

    impl World {
        /// A message in `mailbox`, `flags` already on it.
        fn message(&self, mailbox: MailboxId, flags: &[Flag]) -> MessageId {
            let connection = self.database.connection().expect("a connection");
            let mut message = Message::new(self.account.id, mailbox, Utc::now());
            for flag in flags {
                message.flags.insert(flag.clone());
            }
            MessageRepository::new(&connection)
                .create(&mut message)
                .expect("a message")
        }

        /// What the window would have mirrored: a folder open, rows marked,
        /// the cursor somewhere.
        fn looking_at(&self, mailbox: MailboxId, selected: &[MessageId], focus: Option<MessageId>) {
            self.state
                .update(&self.quiet, |app: &mut AppState| app.open_mailbox(mailbox));
            self.state.update(&self.quiet, |app: &mut AppState| {
                app.select(selected.to_vec(), focus)
            });
        }

        /// What `Ctrl+A` mirrors: a folder open, and the predicate over it.
        fn everything_in(&self, mailbox: MailboxId) {
            self.state
                .update(&self.quiet, |app: &mut AppState| app.open_mailbox(mailbox));
            self.state
                .update(&self.quiet, |app: &mut AppState| app.select_all());
        }

        /// A second account with its own INBOX and Archive.
        ///
        /// The unified view's whole subject is which account a row belongs
        /// to, so these tests need two — and `test_support::account` builds
        /// every one of them at the same address, which the accounts table
        /// will not have twice.
        fn second_account(&self) -> Elsewhere {
            let connection = self.database.connection().expect("a connection");
            let mut account = Account::new(
                "Away",
                postio_model::EmailAddress::new(Some("Away User"), "away@example.com"),
            );
            account.incoming.host = "imap.example.com".to_owned();
            account.outgoing.host = "smtp.example.com".to_owned();
            AccountRepository::new(&connection)
                .create(&mut account)
                .expect("a second account");
            let inbox = test_support::mailbox(&connection, &account, "INBOX").id;
            let archive = test_support::mailbox(&connection, &account, "Archive").id;
            Elsewhere {
                account,
                inbox,
                archive,
            }
        }

        /// A message in another account's `mailbox`.
        fn message_for(&self, account: &Account, mailbox: MailboxId, flags: &[Flag]) -> MessageId {
            let connection = self.database.connection().expect("a connection");
            let mut message = Message::new(account.id, mailbox, Utc::now());
            for flag in flags {
                message.flags.insert(flag.clone());
            }
            MessageRepository::new(&connection)
                .create(&mut message)
                .expect("a message")
        }

        /// What `Ctrl+A` mirrors in the unified view: the aggregate open over
        /// exactly the accounts it could show, and the predicate over that.
        fn everything_unified(&self, accounts: &[AccountId]) {
            let accounts = accounts.to_vec();
            self.state.update(&self.quiet, |app: &mut AppState| {
                app.open_view(ViewScope::Unified {
                    accounts: accounts.clone(),
                })
            });
            self.state
                .update(&self.quiet, |app: &mut AppState| app.select_all());
        }

        /// What `Ctrl+A` mirrors in a smart folder: Flagged open, and the
        /// predicate over it. No mailbox, because there is not one.
        fn everything_flagged(&self) {
            let account = self.account.id;
            self.state
                .update(&self.quiet, |app: &mut AppState| app.open_flagged(account));
            self.state
                .update(&self.quiet, |app: &mut AppState| app.select_all());
        }

        /// Puts one flag on a message that is already stored.
        fn flag(&self, message: MessageId, flag: Flag) {
            let connection = self.database.connection().expect("a connection");
            let repository = MessageRepository::new(&connection);
            let mut flags = repository
                .get(message)
                .expect("a read")
                .expect("the message is still there")
                .flags;
            flags.insert(flag);
            repository
                .set_flags(message, &flags, FlagSource::Server)
                .expect("dress the message");
        }

        fn count_in(&self, mailbox: MailboxId) -> u32 {
            let connection = self.database.connection().expect("a connection");
            MessageRepository::new(&connection)
                .count_set(&MessageSet::in_mailbox(mailbox))
                .expect("a count")
        }

        fn run(&self, command: Command) -> Result<(), CommandError> {
            self.actions.run(&command, &self.sink)
        }

        fn drained(&self) -> Vec<Event> {
            let mut events = Vec::new();
            while let Some(event) = self.events.try_next() {
                events.push(event);
            }
            events
        }

        fn mailbox_of(&self, message: MessageId) -> MailboxId {
            let connection = self.database.connection().expect("a connection");
            MessageRepository::new(&connection)
                .get(message)
                .expect("a read")
                .expect("the message is still there")
                .mailbox_id
        }

        fn flags_of(&self, message: MessageId) -> postio_model::FlagSet {
            let connection = self.database.connection().expect("a connection");
            MessageRepository::new(&connection)
                .get(message)
                .expect("a read")
                .expect("the message is still there")
                .flags
        }

        fn snoozed_until_of(&self, message: MessageId) -> Option<chrono::DateTime<Utc>> {
            let connection = self.database.connection().expect("a connection");
            MessageRepository::new(&connection)
                .get(message)
                .expect("a read")
                .expect("the message is still there")
                .snoozed_until
        }

        /// The queue the sync engine will drain when there is a link again.
        fn queued(&self) -> Vec<(OperationTarget, Operation)> {
            let connection = self.database.connection().expect("a connection");
            OperationQueueRepository::new(&connection)
                .pending(self.account.id, Utc::now())
                .expect("a read")
                .into_iter()
                .map(|row| (row.target, row.operation))
                .collect()
        }
    }

    fn completion(events: &[Event]) -> Option<(&str, bool)> {
        events.iter().find_map(|event| match event {
            Event::ActionCompleted {
                description,
                undoable,
            } => Some((description.as_str(), *undoable)),
            _ => None,
        })
    }

    // ── Archive ──────────────────────────────────────────────────────────

    /// ADR 0005 Q4 (#182). A unified view spans every enabled account, so a
    /// selection made in one can hold messages from several — and each has to
    /// land in *its own* account's Archive.
    ///
    /// The alternative is not a cosmetic slip: filing one account's mail into
    /// another account's folder moves it on a server it does not belong to,
    /// and the queued `Move` carries it there for real once the link is up.
    #[test]
    fn archiving_across_accounts_files_each_message_in_its_own_archive() {
        let world = world();

        // A second account, with its own inbox and its own Archive.
        let (other_account, other_inbox, other_archive) = {
            let connection = world.database.connection().expect("a connection");
            let mut account = postio_model::Account::new(
                "Other",
                postio_model::EmailAddress::new(Some("Other"), "other@example.net"),
            );
            AccountRepository::new(&connection)
                .create(&mut account)
                .expect("the second account");
            let inbox = test_support::mailbox(&connection, &account, "INBOX");
            let archive = test_support::mailbox(&connection, &account, "Archive");
            (account, inbox.id, archive.id)
        };

        let mine = world.message(world.inbox, &[]);
        let theirs = {
            let connection = world.database.connection().expect("a connection");
            let mut message = Message::new(other_account.id, other_inbox, Utc::now());
            MessageRepository::new(&connection)
                .create(&mut message)
                .expect("a message in the other account")
        };

        // Both marked, as a unified view lets them be.
        world.looking_at(world.inbox, &[mine, theirs], Some(mine));

        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .expect("archive");

        assert_eq!(
            world.mailbox_of(mine),
            world.archive,
            "the first account's message goes to its own Archive"
        );
        assert_eq!(
            world.mailbox_of(theirs),
            other_archive,
            "and the second account's message goes to *its* Archive — not to \
             whichever account happened to be first in the selection"
        );

        // The queued moves have to agree, or the server is told the wrong
        // thing once the link comes up. Read against the *other* account's
        // queue: rows are per account, and `world.queued()` only sees the
        // first one's — which is the point.
        let theirs_queued: Vec<(OperationTarget, Operation)> = {
            let connection = world.database.connection().expect("a connection");
            OperationQueueRepository::new(&connection)
                .pending(other_account.id, Utc::now())
                .expect("a read")
                .into_iter()
                .map(|row| (row.target, row.operation))
                .collect()
        };
        assert!(
            theirs_queued.contains(&(
                OperationTarget::Message(theirs),
                Operation::Move {
                    from: other_inbox,
                    to: other_archive
                }
            )),
            "the queued move must name the other account's own folders, and be \
             filed under that account: {theirs_queued:?}"
        );
    }

    #[test]
    fn archiving_the_cursor_row_files_it_and_queues_the_move() {
        // Also the whole of "works offline": nothing here dials anything, and
        // the queue row is what the engine replays when there is a link.
        let world = world();
        let message = world.message(world.inbox, &[]);
        // Nothing marked — a plain click clears the selection — so the verb
        // has to mean the row the cursor is on.
        world.looking_at(world.inbox, &[], Some(message));

        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .expect("archive");

        assert_eq!(world.mailbox_of(message), world.archive);
        assert_eq!(
            world.queued(),
            vec![(
                OperationTarget::Message(message),
                Operation::Move {
                    from: world.inbox,
                    to: world.archive
                }
            )]
        );

        let events = world.drained();
        assert!(events.contains(&Event::MessagesRemoved {
            account: world.account.id,
            mailbox: world.inbox,
            messages: vec![message],
        }));
        assert!(
            events.contains(&Event::MessageListChanged {
                account: world.account.id,
                mailbox: world.archive
            }),
            "the folder they landed in is longer than it was"
        );
        assert_eq!(completion(&events), Some(("Archived 1 message", true)));
    }

    #[test]
    fn a_multi_select_archive_is_one_coalesced_undo_entry() {
        let world = world();
        let messages: Vec<MessageId> = (0..3).map(|_| world.message(world.inbox, &[])).collect();
        world.looking_at(world.inbox, &messages, Some(messages[0]));

        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .expect("archive");
        assert_eq!(
            completion(&world.drained()),
            Some(("Archived 3 messages", true)),
            "one gesture, one entry, one sentence"
        );

        world.run(Command::Undo).expect("undo");

        for message in &messages {
            assert_eq!(
                world.mailbox_of(*message),
                world.inbox,
                "one `u` takes all three back"
            );
        }
    }

    #[test]
    fn undo_puts_a_message_back_and_queues_the_way_back() {
        let world = world();
        let message = world.message(world.inbox, &[]);
        world.looking_at(world.inbox, &[], Some(message));
        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .expect("archive");
        let _ = world.drained();

        world.run(Command::Undo).expect("undo");

        assert_eq!(world.mailbox_of(message), world.inbox);
        assert_eq!(
            world.queued().last(),
            Some(&(
                OperationTarget::Message(message),
                Operation::Move {
                    from: world.archive,
                    to: world.inbox
                }
            )),
            "the server has to be told the way back too"
        );
        assert!(world.drained().contains(&Event::UndoPerformed {
            description: "Archived 1 message".into(),
        }));
    }

    #[test]
    fn archiving_a_thread_takes_every_message_in_it() {
        let world = world();
        let first = world.message(world.inbox, &[]);
        let second = world.message(world.inbox, &[]);
        let thread = {
            let connection = world.database.connection().expect("a connection");
            let threads = ThreadRepository::new(&connection);
            let mut thread = postio_model::Thread::new(world.account.id);
            threads.create(&mut thread).expect("a thread");
            for message in [first, second] {
                threads.add_message(thread.id, message).expect("membership");
            }
            thread.id
        };
        // `A` with the cursor on one of them, and no thread named.
        world.looking_at(world.inbox, &[], Some(first));

        world
            .run(Command::ArchiveThread { thread: None })
            .expect("archive the thread");

        assert_eq!(world.mailbox_of(first), world.archive);
        assert_eq!(
            world.mailbox_of(second),
            world.archive,
            "the message the cursor was not on has to move too"
        );
        assert_eq!(world.queued().len(), 2);
        let _ = thread;
    }

    // ── Delete ───────────────────────────────────────────────────────────

    #[test]
    fn deleting_files_to_trash_as_a_delete_rather_than_a_move() {
        // The two mean different things to the user and the toast says so,
        // even though the local write is the same.
        let world = world();
        let message = world.message(world.inbox, &[]);
        world.looking_at(world.inbox, &[], Some(message));

        world
            .run(Command::Delete {
                target: MessageTarget::Selection,
            })
            .expect("delete");

        assert_eq!(world.mailbox_of(message), world.trash);
        assert_eq!(
            world.queued(),
            vec![(
                OperationTarget::Message(message),
                Operation::Delete {
                    from: world.inbox,
                    trash: world.trash
                }
            )]
        );
        assert_eq!(
            completion(&world.drained()),
            Some(("Deleted 1 message", true))
        );
    }

    // ── Flags ────────────────────────────────────────────────────────────

    #[test]
    fn flagging_a_mixed_selection_makes_it_agree() {
        // Flipping each row independently would turn one keystroke over a
        // mixed selection into a result nobody can predict.
        let world = world();
        let flagged = world.message(world.inbox, &[Flag::Flagged]);
        let plain = world.message(world.inbox, &[]);
        world.looking_at(world.inbox, &[flagged, plain], Some(flagged));

        world
            .run(Command::Flag {
                target: MessageTarget::Selection,
                flagged: None,
            })
            .expect("flag");

        assert!(world.flags_of(flagged).is_flagged());
        assert!(world.flags_of(plain).is_flagged());
        assert_eq!(
            world.queued().len(),
            1,
            "only the row that actually changed is worth telling the server about"
        );
        assert_eq!(
            completion(&world.drained()),
            Some(("Flagged 1 message", true))
        );
    }

    #[test]
    fn undoing_a_flag_unflags_exactly_what_it_flagged() {
        let world = world();
        let message = world.message(world.inbox, &[]);
        world.looking_at(world.inbox, &[], Some(message));
        world
            .run(Command::Flag {
                target: MessageTarget::Selection,
                flagged: None,
            })
            .expect("flag");
        let _ = world.drained();

        world.run(Command::Undo).expect("undo");

        assert!(!world.flags_of(message).is_flagged());
        assert!(matches!(
            world.queued().last(),
            Some((_, Operation::ClearFlags { .. }))
        ));
    }

    #[test]
    fn marking_unread_clears_seen() {
        let world = world();
        let message = world.message(world.inbox, &[Flag::Seen]);
        world.looking_at(world.inbox, &[], Some(message));

        world
            .run(Command::MarkUnread {
                target: MessageTarget::Selection,
                unread: None,
            })
            .expect("mark unread");

        assert!(world.flags_of(message).is_unread());
        assert!(matches!(
            world.queued().first(),
            Some((_, Operation::ClearFlags { .. }))
        ));
        assert_eq!(
            completion(&world.drained()),
            Some(("Marked 1 message as unread", true))
        );
    }

    // ── Snooze (#493) ─────────────────────────────────────────────────────

    #[test]
    fn snoozing_the_selection_hides_it_and_tells_the_list_it_left() {
        let world = world();
        let message = world.message(world.inbox, &[]);
        world.looking_at(world.inbox, &[], Some(message));

        world
            .run(Command::Snooze {
                target: MessageTarget::Selection,
            })
            .expect("snooze");

        assert!(
            world
                .snoozed_until_of(message)
                .is_some_and(|at| at > Utc::now()),
            "the row must carry a snooze somewhere in the future"
        );
        let events = world.drained();
        assert!(events.contains(&Event::MessagesRemoved {
            account: world.account.id,
            mailbox: world.inbox,
            messages: vec![message],
        }));
        assert_eq!(
            completion(&events),
            Some(("Snoozed 1 message", true)),
            "an undoable action, the same as every other verb here"
        );
    }

    #[test]
    fn undoing_a_snooze_unsnoozes_exactly_what_it_snoozed() {
        let world = world();
        let message = world.message(world.inbox, &[]);
        world.looking_at(world.inbox, &[], Some(message));
        world
            .run(Command::Snooze {
                target: MessageTarget::Selection,
            })
            .expect("snooze");
        let _ = world.drained();

        world.run(Command::Undo).expect("undo");

        assert_eq!(
            world.snoozed_until_of(message),
            None,
            "undo is `Unsnooze`, not a second snooze"
        );
    }

    #[test]
    fn unsnoozing_clears_it_and_reloads_its_mailbox_rather_than_naming_the_row() {
        let world = world();
        let message = world.message(world.inbox, &[]);
        world.looking_at(world.inbox, &[], Some(message));
        world
            .run(Command::Snooze {
                target: MessageTarget::Selection,
            })
            .expect("snooze");
        let _ = world.drained();

        world
            .run(Command::Unsnooze {
                target: MessageTarget::Selection,
            })
            .expect("unsnooze");

        assert_eq!(world.snoozed_until_of(message), None);
        let events = world.drained();
        assert!(events.contains(&Event::MessageListChanged {
            account: world.account.id,
            mailbox: world.inbox,
        }));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::MessagesRemoved { .. })),
            "nothing left a mailbox; it came back"
        );
    }

    #[test]
    fn snoozing_a_whole_mailbox_is_refused() {
        // Not offered yet (`aim::view_scope` never resolves `ListScope::Snoozed`
        // into something `Ctrl+A` can select against either) -- see
        // `Actions::snooze`'s own doc comment for why.
        let world = world();
        world.message(world.inbox, &[]);
        world.everything_in(world.inbox);

        let error = world
            .run(Command::Snooze {
                target: MessageTarget::Selection,
            })
            .expect_err("a whole-mailbox snooze is not offered");

        assert!(matches!(error, CommandError::Rejected(_)));
    }

    // ── Settling a send nobody could confirm (#674) ──────────────────────

    #[test]
    fn marking_an_unconfirmed_send_as_sent_settles_the_draft() {
        // The exit that exists because the other two are wrong: discard
        // throws away a message that may have arrived, and sending again
        // duplicates one that did. A user who checked with the recipient
        // needs to be able to say so.
        let world = world();
        let id = {
            let connection = world.database.connection().expect("a connection");
            let drafts = postio_storage::repository::DraftRepository::new(&connection);
            let mut draft = postio_model::Draft::new(world.account.id);
            draft.to = vec![postio_model::EmailAddress::new(
                None::<String>,
                "grace@example.net",
            )];
            let id = drafts.save(&mut draft).expect("save");
            drafts
                .set_state(id, DraftState::Unconfirmed)
                .expect("the state an interrupted submission leaves");
            id
        };

        world
            .run(Command::MarkSent { draft: Some(id) })
            .expect("marking it sent applies");

        let connection = world.database.connection().expect("a connection");
        assert_eq!(
            postio_storage::repository::DraftRepository::new(&connection)
                .get(id)
                .expect("read")
                .expect("still there")
                .state,
            DraftState::Sent,
            "the question is settled, so it stops being offered as unsent"
        );
    }

    #[test]
    fn only_an_unconfirmed_send_can_be_marked_as_sent() {
        // Saying "it arrived" about a draft still being edited is not a
        // correction, it is a way to lose it: `Sent` is what stops a draft
        // being offered for editing.
        let world = world();
        let id = {
            let connection = world.database.connection().expect("a connection");
            let drafts = postio_storage::repository::DraftRepository::new(&connection);
            let mut draft = postio_model::Draft::new(world.account.id);
            drafts.save(&mut draft).expect("save")
        };

        let error = world
            .run(Command::MarkSent { draft: Some(id) })
            .expect_err("an editable draft is not something to mark sent");

        assert!(matches!(error, CommandError::Rejected(_)), "{error:?}");
        assert_eq!(
            postio_storage::repository::DraftRepository::new(
                &world.database.connection().expect("a connection")
            )
            .get(id)
            .expect("read")
            .expect("still there")
            .state,
            DraftState::Editing,
            "and it is left exactly as it was"
        );
    }

    // ── Marking read because you looked at it (#71) ──────────────────────

    #[test]
    fn a_dwell_mark_reads_the_message_and_tells_the_server() {
        // The same local-first shape as every other verb: the flag lands
        // locally and the `\Seen` goes on the queue for the server. Nobody
        // pressed anything — the cursor rested — but the mail still has to be
        // read on every other client afterwards.
        let world = world();
        let message = world.message(world.inbox, &[]);

        world
            .run(Command::MarkReadOnDwell { message })
            .expect("the dwell mark applies");

        assert!(world.flags_of(message).is_seen());
        assert!(
            matches!(
                world.queued().first(),
                Some((_, Operation::SetFlags { .. }))
            ),
            "the server has to hear about it, or the message is unread again \
             on the next device"
        );
    }

    #[test]
    fn a_dwell_mark_on_a_thread_member_recomputes_the_threads_unread_count() {
        // `threads.unread_count` is denormalised, and the account-scoped
        // page reads it straight off `threads` (#754, consequence 4) — so a
        // local `\Seen` write that does not recompute it leaves a unified
        // view drawing the conversation unread for ever. Folder pages
        // recompute live and never noticed.
        let world = world();
        let first = world.message(world.inbox, &[]);
        let second = world.message(world.inbox, &[]);
        let third = world.message(world.inbox, &[]);
        let thread = {
            let connection = world.database.connection().expect("a connection");
            let threads = ThreadRepository::new(&connection);
            let mut thread = postio_model::Thread::new(world.account.id);
            threads.create(&mut thread).expect("a thread");
            for message in [first, second, third] {
                threads.add_message(thread.id, message).expect("membership");
            }
            thread.id
        };

        world
            .run(Command::MarkReadOnDwell { message: first })
            .expect("the dwell mark applies");

        let connection = world.database.connection().expect("a connection");
        let record = ThreadRepository::new(&connection)
            .get(thread)
            .expect("a read")
            .expect("the thread is still there");
        assert_eq!(
            record.unread_count, 2,
            "the thread's own aggregate has to follow a local read, or the \
             account-scoped page shows the conversation unread for ever"
        );
    }

    #[test]
    fn a_dwell_mark_announces_the_conversations_other_members_for_repaint() {
        // The list's row for a conversation is its representative, and
        // `pages_holding` resolves the event's message ids against *row*
        // ids — so marking a non-representative member read refetched no
        // page and the row went on drawing unread (#754, consequence 3).
        // The event has to carry the siblings for the row to be findable.
        let world = world();
        let first = world.message(world.inbox, &[]);
        let second = world.message(world.inbox, &[]);
        let third = world.message(world.inbox, &[]);
        {
            let connection = world.database.connection().expect("a connection");
            let threads = ThreadRepository::new(&connection);
            let mut thread = postio_model::Thread::new(world.account.id);
            threads.create(&mut thread).expect("a thread");
            for message in [first, second, third] {
                threads.add_message(thread.id, message).expect("membership");
            }
        }

        world
            .run(Command::MarkReadOnDwell { message: first })
            .expect("the dwell mark applies");

        let changed: Vec<MessageId> = world
            .drained()
            .into_iter()
            .filter_map(|event| match event {
                Event::MessagesChanged { messages, .. } => Some(messages),
                _ => None,
            })
            .flatten()
            .collect();
        for member in [second, third] {
            assert!(
                changed.contains(&member),
                "the repaint announcement has to name the whole conversation, \
                 or the row standing for it cannot be found: {changed:?}"
            );
        }
        // The *operation* stays one message wide: the siblings are a repaint
        // concern, and a queue row per untouched member would tell the
        // server about flags nobody changed.
        assert_eq!(
            world.queued().len(),
            1,
            "only the marked message goes to the server"
        );
    }

    #[test]
    fn a_dwell_mark_leaves_the_undo_stack_alone() {
        // `u` takes back what *you* did. Reading a mailbox produces one dwell
        // mark per message rested on, so recording them would bury the verb
        // the user actually wants back. Here the archive must still be what
        // `u` reaches, with a dwell mark sitting on top of it.
        let world = world();
        let archived = world.message(world.inbox, &[]);
        let read = world.message(world.inbox, &[]);
        world.looking_at(world.inbox, &[], Some(archived));

        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .expect("archive");
        assert_ne!(world.mailbox_of(archived), world.inbox);

        world
            .run(Command::MarkReadOnDwell { message: read })
            .expect("the dwell mark applies");

        world.run(Command::Undo).expect("undo");
        assert_eq!(
            world.mailbox_of(archived),
            world.inbox,
            "`u` reached the dwell mark instead of the archive, so reading a \
             mailbox now costs the user their undo"
        );
        assert!(
            world.flags_of(read).is_seen(),
            "and the dwell mark itself is not something `u` takes back"
        );
    }

    #[test]
    fn a_dwell_mark_raises_no_toast() {
        // One per message rested on, so a toast each would be a permanent
        // banner over the message list. The row turning from unread to read
        // is the feedback, and it is feedback the user is already looking at.
        let world = world();
        let message = world.message(world.inbox, &[]);

        world
            .run(Command::MarkReadOnDwell { message })
            .expect("the dwell mark applies");

        let events = world.drained();
        assert_eq!(completion(&events), None, "{events:?}");
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::MessagesChanged { .. })),
            "the row still has to repaint, or the list goes on showing it unread"
        );
    }

    #[test]
    fn a_dwell_mark_on_a_message_already_read_queues_nothing() {
        // The cursor resting on mail you have read is the ordinary case once
        // a mailbox has been worked through, and it must not cost a queue row
        // the server would have to be told about.
        let world = world();
        let message = world.message(world.inbox, &[Flag::Seen]);

        world
            .run(Command::MarkReadOnDwell { message })
            .expect("a no-op is not an error");

        assert!(world.flags_of(message).is_seen());
        assert!(
            world.queued().is_empty(),
            "an already-read message must not put a redundant \\Seen on the queue"
        );
    }

    // ── What a verb refuses ──────────────────────────────────────────────

    #[test]
    fn a_move_with_no_destination_asks_rather_than_guessing() {
        let world = world();
        let message = world.message(world.inbox, &[]);
        world.looking_at(world.inbox, &[], Some(message));

        let outcome = world.run(Command::Move {
            target: MessageTarget::Selection,
            to: None,
        });

        assert!(matches!(outcome, Err(CommandError::Rejected(_))));
        assert_eq!(world.mailbox_of(message), world.inbox);
    }

    // ── The whole mailbox at once ────────────────────────────────────────

    #[test]
    fn ctrl_a_then_archive_files_the_whole_mailbox() {
        // The bead: triage on an 81,717-message account. Everything below is
        // about this staying a *query* — but first it has to work.
        let world = world();
        let messages: Vec<MessageId> = (0..30).map(|_| world.message(world.inbox, &[])).collect();
        world.everything_in(world.inbox);

        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .expect("archive the mailbox");

        for message in &messages {
            assert_eq!(world.mailbox_of(*message), world.archive);
        }
        assert_eq!(world.queued().len(), 30, "the server has to be told too");
        assert_eq!(
            completion(&world.drained()),
            Some(("Archived 30 messages", true)),
            "one gesture, one sentence, and an undo behind it"
        );
    }

    // ── The whole *smart* folder at once (#52) ──────────────────────────

    #[test]
    fn ctrl_a_then_unflag_clears_the_whole_flagged_view() {
        // The verb the issue says people most want over Flagged, and the one
        // whose predicate is treacherous: unflagging is the write that empties
        // the very set it is selecting on.
        let world = world();
        let flagged: Vec<MessageId> = (0..6)
            .map(|index| {
                let mailbox = if index % 2 == 0 {
                    world.inbox
                } else {
                    world.archive
                };
                let message = world.message(mailbox, &[]);
                world.flag(message, Flag::Flagged);
                message
            })
            .collect();
        let untouched = world.message(world.inbox, &[]);
        world.everything_flagged();

        world
            .run(Command::Flag {
                target: MessageTarget::Selection,
                flagged: Some(false),
            })
            .expect("Ctrl+A in Flagged then unflag must not reject");

        for message in &flagged {
            assert!(
                !world.flags_of(*message).contains(&Flag::Flagged),
                "every flagged message in the account should have been cleared, \
                 across folders"
            );
        }
        assert!(!world.flags_of(untouched).contains(&Flag::Flagged));
        assert_eq!(
            world.queued().len(),
            6,
            "one queue row per message, written before the write emptied the \
             predicate they were selected by"
        );
    }

    // ── The unified view (#811) ──────────────────────────────────────

    #[test]
    fn a_unified_archive_leaves_out_an_account_the_selection_was_not_scoped_to() {
        // The account was away when `Ctrl+A` was pressed, so it is not in the
        // scope -- and it is deliberately not consulted again here. Were
        // reachability read at verb time instead, an account that reconnected
        // in between would join a selection the user was never shown, which
        // is the same defect pointing the other way (ADR 0005 Q10, #811).
        let world = world();
        let away = world.second_account();
        let mine = world.message(world.inbox, &[]);
        let theirs = world.message_for(&away.account, away.inbox, &[]);
        world.everything_unified(&[world.account.id]);

        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .expect("Ctrl+A in the unified view then archive must not reject");

        assert_eq!(world.mailbox_of(mine), world.archive);
        assert_eq!(
            world.mailbox_of(theirs),
            away.inbox,
            "an account the selection was never scoped to must not be archived"
        );
    }

    #[test]
    fn a_unified_archive_files_each_account_in_its_own_archive() {
        // A bulk verb across accounts is not one action with one destination:
        // "the Archive" is a different folder in each account, and a single
        // `Applied` naming one account could not describe it either.
        let world = world();
        let away = world.second_account();
        let mine = world.message(world.inbox, &[]);
        let theirs = world.message_for(&away.account, away.inbox, &[]);
        world.everything_unified(&[world.account.id, away.account.id]);

        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .expect("an archive across both accounts");

        assert_eq!(world.mailbox_of(mine), world.archive);
        assert_eq!(
            world.mailbox_of(theirs),
            away.archive,
            "each account's mail goes to that account's own Archive"
        );
    }

    #[test]
    fn a_unified_flag_write_only_touches_the_accounts_the_scope_names() {
        let world = world();
        let away = world.second_account();
        let mine = world.message(world.inbox, &[]);
        let theirs = world.message_for(&away.account, away.inbox, &[]);
        world.everything_unified(&[world.account.id]);

        world
            .run(Command::Flag {
                target: MessageTarget::Selection,
                flagged: Some(true),
            })
            .expect("a bulk flag over the reachable accounts");

        assert!(world.flags_of(mine).contains(&Flag::Flagged));
        assert!(
            !world.flags_of(theirs).contains(&Flag::Flagged),
            "an account the selection was never scoped to must not be flagged"
        );
    }

    #[test]
    fn a_unified_bulk_action_announces_itself_once_per_account() {
        // ADR 0005 Q11: every event announcing work names the account it
        // happened in. One event for two accounts would have to name one of
        // them, and the list showing the other would never repaint.
        let world = world();
        let away = world.second_account();
        world.message(world.inbox, &[]);
        world.message_for(&away.account, away.inbox, &[]);
        world.everything_unified(&[world.account.id, away.account.id]);

        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .expect("an archive across both accounts");

        let accounts: Vec<AccountId> = world
            .drained()
            .into_iter()
            .filter_map(|event| match event {
                // A bulk unit reports a folder that changed wholesale rather
                // than naming the rows that left it -- that read is the one
                // the predicate exists to avoid.
                Event::MessageListChanged { account, .. } => Some(account),
                _ => None,
            })
            .collect();
        assert!(
            accounts.contains(&world.account.id) && accounts.contains(&away.account.id),
            "both accounts' lists have to hear about it: {accounts:?}"
        );
    }

    #[test]
    fn one_undo_puts_a_whole_flagged_view_back() {
        // The ordering trap, proved from the other end: if the queue rows had
        // been written after the flag came off, the run would be empty and
        // `u` would have nothing to take back.
        let world = world();
        let flagged: Vec<MessageId> = (0..4)
            .map(|index| {
                let mailbox = if index % 2 == 0 {
                    world.inbox
                } else {
                    world.archive
                };
                let message = world.message(mailbox, &[]);
                world.flag(message, Flag::Flagged);
                message
            })
            .collect();
        world.everything_flagged();
        world
            .run(Command::Flag {
                target: MessageTarget::Selection,
                flagged: Some(false),
            })
            .expect("unflag everything");
        let _ = world.drained();

        world.run(Command::Undo).expect("undo");

        for message in &flagged {
            assert!(
                world.flags_of(*message).contains(&Flag::Flagged),
                "undo has to reach the rows the action touched, wherever they \
                 are filed"
            );
        }
    }

    #[test]
    fn ctrl_a_then_archive_in_flagged_files_from_every_folder_it_spans() {
        // A move out of a smart folder is grouped by source folder, because
        // the queue's `Operation::Move` payload carries one `from` for the
        // whole run it writes. Ungrouped, every row would claim to have come
        // out of whichever folder happened to be named.
        let world = world();
        let from_inbox = world.message(world.inbox, &[]);
        let from_trash = world.message(world.trash, &[]);
        world.flag(from_inbox, Flag::Flagged);
        world.flag(from_trash, Flag::Flagged);
        let already_there = world.message(world.archive, &[]);
        world.flag(already_there, Flag::Flagged);
        world.everything_flagged();

        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .expect("archive everything flagged");

        assert_eq!(world.mailbox_of(from_inbox), world.archive);
        assert_eq!(world.mailbox_of(from_trash), world.archive);
        assert_eq!(
            world.mailbox_of(already_there),
            world.archive,
            "it was already there and stayed"
        );

        let queued = world.queued();
        assert_eq!(
            queued.len(),
            2,
            "the message already in the destination is not moving, so it gets \
             no queue row telling the server to move it onto itself: {queued:#?}"
        );
        let sources: std::collections::BTreeSet<MailboxId> = queued
            .iter()
            .filter_map(|(_, operation)| match operation {
                Operation::Move { from, .. } => Some(*from),
                _ => None,
            })
            .collect();
        assert_eq!(
            sources,
            [world.inbox, world.trash].into_iter().collect(),
            "each queue row names the folder its own message actually came from"
        );
    }

    #[test]
    fn undoing_an_archive_out_of_flagged_returns_each_message_to_its_own_folder() {
        let world = world();
        let from_inbox = world.message(world.inbox, &[]);
        let from_trash = world.message(world.trash, &[]);
        world.flag(from_inbox, Flag::Flagged);
        world.flag(from_trash, Flag::Flagged);
        world.everything_flagged();
        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .expect("archive everything flagged");
        let _ = world.drained();

        world.run(Command::Undo).expect("undo");

        assert_eq!(
            world.mailbox_of(from_inbox),
            world.inbox,
            "back where it came from, not to one folder for all of them"
        );
        assert_eq!(world.mailbox_of(from_trash), world.trash);
    }

    #[test]
    fn one_undo_takes_a_whole_mailbox_back() {
        // The half the bead called harder. The inverse is a predicate over the
        // run of queue rows the archive wrote, so `u` names thirty thousand
        // messages with two integers.
        let world = world();
        let messages: Vec<MessageId> = (0..20).map(|_| world.message(world.inbox, &[])).collect();
        let elsewhere = world.message(world.archive, &[]);
        world.everything_in(world.inbox);
        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .expect("archive the mailbox");
        let _ = world.drained();

        world.run(Command::Undo).expect("undo");

        for message in &messages {
            assert_eq!(world.mailbox_of(*message), world.inbox, "all the way back");
        }
        assert_eq!(
            world.mailbox_of(elsewhere),
            world.archive,
            "undo puts back what the action moved, not everything that was in \
             the folder it moved things into"
        );
        assert_eq!(
            world.run(Command::Undo),
            Err(CommandError::rejected("Nothing to undo")),
            "one entry, not twenty"
        );
    }

    #[test]
    fn a_whole_mailbox_archive_is_one_undo_entry_even_beside_its_neighbours() {
        // `postio-cy0`'s coalescing decides what one `u` covers, and a bulk
        // unit stands alone in it: it cannot say which rows it holds, so
        // folding the next archive into it would hide a separate action
        // behind the same single keystroke.
        let world = world();
        for _ in 0..5 {
            world.message(world.inbox, &[]);
        }
        let other = world.message(world.trash, &[]);
        world.everything_in(world.inbox);
        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .expect("archive the mailbox");
        world.looking_at(world.trash, &[], Some(other));
        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .expect("archive one more");
        let _ = world.drained();

        world.run(Command::Undo).expect("undo the single one");
        assert_eq!(world.mailbox_of(other), world.trash);

        world.run(Command::Undo).expect("undo the bulk one");
        assert_eq!(
            world.count_in(world.inbox),
            5,
            "the two gestures come back separately"
        );
    }

    #[test]
    fn the_rows_taken_back_out_of_a_select_all_stay_where_they_are() {
        let world = world();
        let messages: Vec<MessageId> = (0..6).map(|_| world.message(world.inbox, &[])).collect();
        world.everything_in(world.inbox);
        world.state.update(&world.quiet, |app: &mut AppState| {
            app.toggle_selection(messages[2])
        });

        world
            .run(Command::Delete {
                target: MessageTarget::Selection,
            })
            .expect("delete the rest");

        assert_eq!(world.mailbox_of(messages[2]), world.inbox);
        assert_eq!(world.mailbox_of(messages[0]), world.trash);
        assert_eq!(
            completion(&world.drained()),
            Some(("Deleted 5 messages", true))
        );
    }

    #[test]
    fn a_whole_mailbox_action_tells_both_lists_to_reload() {
        // It cannot name the rows that left, so `MessagesRemoved` is not
        // available to it — and a list that was not told to reload would go on
        // showing eighty thousand messages that are no longer there.
        let world = world();
        world.message(world.inbox, &[]);
        world.everything_in(world.inbox);

        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .expect("archive the mailbox");

        let events = world.drained();
        assert!(events.contains(&Event::MessageListChanged {
            account: world.account.id,
            mailbox: world.inbox
        }));
        assert!(events.contains(&Event::MessageListChanged {
            account: world.account.id,
            mailbox: world.archive
        }));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::MessagesRemoved { .. })),
            "naming the rows is exactly what this path must not do"
        );
    }

    #[test]
    fn archiving_a_whole_mailbox_that_is_already_the_archive_is_refused() {
        let world = world();
        world.message(world.archive, &[]);
        world.everything_in(world.archive);

        assert_eq!(
            world.run(Command::Archive {
                target: MessageTarget::Selection,
            }),
            Err(CommandError::rejected("Already there"))
        );
    }

    #[test]
    fn a_whole_mailbox_selection_over_an_empty_mailbox_says_so() {
        // `Selection::Everything` is never empty as far as app state is
        // concerned — counting it there would be the read it exists to avoid —
        // so the store is the first thing that can tell, and it has to.
        let world = world();
        world.everything_in(world.inbox);

        assert_eq!(
            world.run(Command::Archive {
                target: MessageTarget::Selection,
            }),
            Err(CommandError::rejected("There is nothing here to move"))
        );
    }

    #[test]
    fn ctrl_a_then_mark_read_marks_the_whole_mailbox() {
        // The second thing anyone does to an 81,717-message account, after
        // archiving it. `\Seen` lives in a text column beside a denormalised
        // boolean, so this is a different statement from a bulk move — but it
        // is still one statement, and still never names a row.
        let world = world();
        let messages: Vec<MessageId> = (0..25).map(|_| world.message(world.inbox, &[])).collect();
        world.everything_in(world.inbox);

        world
            .run(Command::MarkUnread {
                target: MessageTarget::Selection,
                unread: Some(false),
            })
            .expect("mark the mailbox read");

        for message in &messages {
            assert!(world.flags_of(*message).is_seen());
        }
        assert_eq!(
            world.queued().len(),
            25,
            "the server has to be told, once per message"
        );
        assert_eq!(
            completion(&world.drained()),
            Some(("Marked 25 messages as read", true))
        );
    }

    #[test]
    fn a_whole_mailbox_flag_toggle_makes_them_agree() {
        // The rule a toggle over more than one row follows, held over a
        // mailbox: if every row already carries the flag it comes off them
        // all, otherwise it goes onto them all. Deciding which is two indexed
        // counts, not a read of the mailbox.
        let world = world();
        let messages: Vec<MessageId> = (0..6).map(|_| world.message(world.inbox, &[])).collect();
        world.flag(messages[0], Flag::Flagged);
        world.everything_in(world.inbox);

        world
            .run(Command::Flag {
                target: MessageTarget::Selection,
                flagged: None,
            })
            .expect("toggle the mailbox");
        for message in &messages {
            assert!(world.flags_of(*message).is_flagged(), "they now agree");
        }

        world
            .run(Command::Flag {
                target: MessageTarget::Selection,
                flagged: None,
            })
            .expect("toggle it back");
        for message in &messages {
            assert!(
                !world.flags_of(*message).is_flagged(),
                "agreeing, the toggle takes it off them all"
            );
        }
    }

    #[test]
    fn a_whole_mailbox_flag_only_queues_the_rows_it_changed() {
        // A queue row for a message that already carried the flag tells the
        // server nothing, and puts that message inside the run undo takes
        // back — so `u` would clear a flag this action never set.
        let world = world();
        let already = world.message(world.inbox, &[Flag::Flagged]);
        let rest: Vec<MessageId> = (0..4).map(|_| world.message(world.inbox, &[])).collect();
        world.everything_in(world.inbox);

        world
            .run(Command::Flag {
                target: MessageTarget::Selection,
                flagged: Some(true),
            })
            .expect("flag the mailbox");

        let queued = world.queued();
        assert_eq!(queued.len(), 4);
        assert!(
            !queued
                .iter()
                .any(|(target, _)| *target == OperationTarget::Message(already)),
            "it was already flagged; there is nothing to tell the server"
        );
        assert_eq!(
            completion(&world.drained()),
            Some(("Flagged 4 messages", true)),
            "the sentence counts what changed, not what was selected"
        );
        for message in &rest {
            assert!(world.flags_of(*message).is_flagged());
        }
    }

    #[test]
    fn one_undo_takes_a_whole_mailbox_flag_back() {
        let world = world();
        let already = world.message(world.inbox, &[Flag::Flagged]);
        let rest: Vec<MessageId> = (0..8).map(|_| world.message(world.inbox, &[])).collect();
        world.everything_in(world.inbox);
        world
            .run(Command::Flag {
                target: MessageTarget::Selection,
                flagged: Some(true),
            })
            .expect("flag the mailbox");
        let _ = world.drained();

        world.run(Command::Undo).expect("undo");

        for message in &rest {
            assert!(!world.flags_of(*message).is_flagged(), "all the way back");
        }
        assert!(
            world.flags_of(already).is_flagged(),
            "undo takes back what the action flagged, not what was flagged already"
        );
        assert_eq!(
            world.run(Command::Undo),
            Err(CommandError::rejected("Nothing to undo")),
            "one entry, not eight"
        );
    }

    #[test]
    fn a_whole_mailbox_already_agreeing_with_the_flag_says_so() {
        let world = world();
        world.message(world.inbox, &[Flag::Flagged]);
        world.message(world.inbox, &[Flag::Flagged]);
        world.everything_in(world.inbox);

        assert_eq!(
            world.run(Command::Flag {
                target: MessageTarget::Selection,
                flagged: Some(true),
            }),
            Err(CommandError::rejected("Already set"))
        );
    }

    #[test]
    fn a_whole_mailbox_flag_over_an_empty_mailbox_says_so() {
        let world = world();
        world.everything_in(world.inbox);

        assert_eq!(
            world.run(Command::Flag {
                target: MessageTarget::Selection,
                flagged: Some(true),
            }),
            Err(CommandError::rejected("There is nothing here to change"))
        );
    }

    #[test]
    fn a_whole_mailbox_flag_tells_the_list_to_reload_rather_than_naming_rows() {
        let world = world();
        world.message(world.inbox, &[]);
        world.everything_in(world.inbox);

        world
            .run(Command::MarkUnread {
                target: MessageTarget::Selection,
                unread: Some(false),
            })
            .expect("mark the mailbox read");

        let events = world.drained();
        assert!(events.contains(&Event::MessageListChanged {
            account: world.account.id,
            mailbox: world.inbox
        }));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::MessagesChanged { .. })),
            "naming the rows is exactly what this path must not do"
        );
    }

    #[test]
    fn archiving_a_thread_group_archives_every_copy_one_unit_per_account() {
        // #184, ADR 0005 Q2: the unified list's deduped row stands for one
        // conversation the user received at two addresses. Archiving it has
        // to hit *every* copy — two operations in two per-account queues —
        // because that is the only answer that matches what the user
        // believes they did. `MessageTarget::Threads` is the group's
        // expansion, and `relocate`'s existing per-account split (#182) is
        // what turns it into per-account units.
        let world = world();
        let (second, second_inbox, second_archive) = {
            let connection = world.database.connection().expect("a connection");
            let mut account = postio_model::Account::new(
                "Second",
                postio_model::EmailAddress::new(None::<String>, "grace@example.org"),
            );
            postio_storage::repository::AccountRepository::new(&connection)
                .create(&mut account)
                .expect("second account");
            let inbox = test_support::mailbox(&connection, &account, "INBOX").id;
            let archive = test_support::mailbox(&connection, &account, "Archive").id;
            (account.id, inbox, archive)
        };

        // The same message at both addresses, threaded in each account.
        let file = |account: AccountId, mailbox: MailboxId| -> ThreadId {
            let connection = world.database.connection().expect("a connection");
            let mut message = Message::new(account, mailbox, Utc::now());
            message.rfc_message_id = Some(postio_model::RfcMessageId::new("<pair@example.com>"));
            message.subject = Some("Paired".to_owned());
            MessageRepository::new(&connection)
                .create(&mut message)
                .expect("a message");
            postio_storage::repository::ThreadingRepository::new(&connection, account)
                .thread(&message)
                .expect("threaded")
                .thread_id
        };
        let first_thread = file(world.account.id, world.inbox);
        let second_thread = file(second, second_inbox);

        world
            .run(Command::Archive {
                target: MessageTarget::Threads(vec![first_thread, second_thread]),
            })
            .expect("the group archives");

        let connection = world.database.connection().expect("a connection");
        let messages = MessageRepository::new(&connection);
        let in_archive = |mailbox: MailboxId| -> u32 {
            messages
                .count(&postio_storage::repository::ListQuery {
                    scope: postio_storage::repository::ListScope::Mailbox(mailbox),
                    limit: 10,
                    after: None,
                })
                .expect("a count")
        };
        assert_eq!(
            in_archive(world.archive),
            1,
            "the first account's copy moved"
        );
        assert_eq!(
            in_archive(second_archive),
            1,
            "and the second account's copy moved into *its own* Archive"
        );

        let queue = postio_storage::repository::OperationQueueRepository::new(&connection);
        let first_ops = queue.pending(world.account.id, Utc::now()).expect("queue");
        let second_ops = queue.pending(second, Utc::now()).expect("queue");
        assert_eq!(first_ops.len(), 1, "one operation in the first queue");
        assert_eq!(
            second_ops.len(),
            1,
            "one in the second: per-account, always"
        );
    }

    #[test]
    fn a_move_to_another_accounts_folder_starts_the_saga_not_a_move() {
        // #188, ADR 0005 Q9: between accounts there is no server-side move.
        // The command's local half is immediate — the message appears in
        // the target account and leaves the source — and the server work is
        // two saga operations, one per account queue, ordered by the saga
        // table so nothing deletes before the copy is confirmed.
        let world = world();
        let (second, second_inbox) = {
            let connection = world.database.connection().expect("a connection");
            let mut account = postio_model::Account::new(
                "Second",
                postio_model::EmailAddress::new(None::<String>, "grace@example.org"),
            );
            postio_storage::repository::AccountRepository::new(&connection)
                .create(&mut account)
                .expect("second account");
            let inbox = test_support::mailbox(&connection, &account, "INBOX").id;
            (account.id, inbox)
        };
        let message = world.message(world.inbox, &[]);
        world.looking_at(world.inbox, &[], Some(message));

        world
            .run(Command::Move {
                target: MessageTarget::Messages(vec![message]),
                to: Some(second_inbox),
            })
            .expect("the move starts");

        let connection = world.database.connection().expect("a connection");
        let queue = postio_storage::repository::OperationQueueRepository::new(&connection);
        let source_ops = queue.pending(world.account.id, Utc::now()).expect("queue");
        let target_ops = queue.pending(second, Utc::now()).expect("queue");
        assert_eq!(source_ops.len(), 1);
        assert_eq!(
            source_ops[0].operation.op_type(),
            "cross_account_remove",
            "the source queue holds phase 3, and only phase 3"
        );
        assert_eq!(target_ops.len(), 1);
        assert_eq!(target_ops[0].operation.op_type(), "cross_account_copy");

        // Local-first: gone from here, visible there, immediately.
        let messages = MessageRepository::new(&connection);
        let source_row = messages.get(message).expect("read").expect("the row");
        assert!(
            source_row.sync.deleted_locally,
            "the source row is hidden at once; the saga reconciles"
        );
        let copies = messages
            .count(&postio_storage::repository::ListQuery {
                scope: postio_storage::repository::ListScope::Mailbox(second_inbox),
                limit: 10,
                after: None,
            })
            .expect("a count");
        assert_eq!(copies, 1, "the provisional copy is already in the target");
    }

    /// A world with a second account and its inbox, for the saga tests.
    fn second_account(world: &World) -> (postio_model::ids::AccountId, MailboxId) {
        let connection = world.database.connection().expect("a connection");
        let mut account = postio_model::Account::new(
            "Second",
            postio_model::EmailAddress::new(None::<String>, "grace@example.org"),
        );
        postio_storage::repository::AccountRepository::new(&connection)
            .create(&mut account)
            .expect("second account");
        let inbox = test_support::mailbox(&connection, &account, "INBOX").id;
        (account.id, inbox)
    }

    /// #531, first half: nothing has reached either server yet, so undo is
    /// local bookkeeping and can be complete.
    ///
    /// Before this, `u` popped the entry, replayed its empty inverse, and
    /// reported success — the message stayed hidden, the saga stayed open,
    /// and the toast said it had been undone. A silent lie is worse than the
    /// refusal the issue thought was there.
    #[test]
    fn undo_of_a_move_that_has_not_reached_a_server_cancels_the_saga() {
        let world = world();
        let (second, second_inbox) = second_account(&world);
        let message = world.message(world.inbox, &[]);
        world.looking_at(world.inbox, &[], Some(message));
        world
            .run(Command::Move {
                target: MessageTarget::Messages(vec![message]),
                to: Some(second_inbox),
            })
            .expect("the move starts");

        world.run(Command::Undo).expect("undo cancels the saga");

        let connection = world.database.connection().expect("a connection");
        let messages = MessageRepository::new(&connection);
        let source_row = messages.get(message).expect("read").expect("the row");
        assert!(
            !source_row.sync.deleted_locally,
            "the source row is visible again -- it never left"
        );

        let copies = messages
            .count(&postio_storage::repository::ListQuery {
                scope: postio_storage::repository::ListScope::Mailbox(second_inbox),
                limit: 10,
                after: None,
            })
            .expect("a count");
        assert_eq!(copies, 0, "the provisional copy in the target is gone");

        let queue = postio_storage::repository::OperationQueueRepository::new(&connection);
        assert!(
            queue
                .pending(world.account.id, Utc::now())
                .expect("queue")
                .is_empty(),
            "the remove never runs: it was withdrawn, not left to be skipped"
        );
        assert!(
            queue.pending(second, Utc::now()).expect("queue").is_empty(),
            "and neither does the copy"
        );

        let phase: String = connection
            .query_row("SELECT phase FROM cross_account_moves", [], |row| {
                row.get(0)
            })
            .expect("the saga row");
        assert_eq!(
            phase, "aborted",
            "the saga ends in the phase that means nothing was deleted"
        );
    }

    /// #531, the other half: once the copy is confirmed the message is on
    /// two servers, and taking it back is the inverse saga rather than
    /// bookkeeping. Refused, out loud, until that exists.
    #[test]
    fn undo_of_a_move_the_other_server_has_already_taken_is_refused_not_faked() {
        let world = world();
        let (_second, second_inbox) = second_account(&world);
        let message = world.message(world.inbox, &[]);
        world.looking_at(world.inbox, &[], Some(message));
        world
            .run(Command::Move {
                target: MessageTarget::Messages(vec![message]),
                to: Some(second_inbox),
            })
            .expect("the move starts");

        // Walk the saga to where the target account genuinely has the mail.
        {
            let connection = world.database.connection().expect("a connection");
            let sagas = postio_storage::repository::CrossAccountMoveRepository::new(&connection);
            let id = sagas
                .open_for_sources(&[message])
                .expect("read")
                .first()
                .expect("a saga")
                .id;
            sagas
                .confirm(id, Some(&postio_model::ids::RemoteId::new("9".to_owned())))
                .expect("confirm");
        }

        let outcome = world.run(Command::Undo);

        assert!(
            outcome.is_err(),
            "a move the other server has taken cannot be undone by bookkeeping"
        );
        let connection = world.database.connection().expect("a connection");
        let source_row = MessageRepository::new(&connection)
            .get(message)
            .expect("read")
            .expect("the row");
        assert!(
            source_row.sync.deleted_locally,
            "and a refused undo changes nothing"
        );
        let phase: String = connection
            .query_row("SELECT phase FROM cross_account_moves", [], |row| {
                row.get(0)
            })
            .expect("the saga row");
        assert_eq!(phase, "confirmed", "least of all the saga");
    }

    #[test]
    fn a_move_within_one_account_stays_a_single_move_operation() {
        // The cheap case must stay cheap (ADR 0005 Q9): nothing about the
        // saga applies inside one account, where the server has a real MOVE.
        let world = world();
        let message = world.message(world.inbox, &[]);
        world.looking_at(world.inbox, &[], Some(message));

        world
            .run(Command::Move {
                target: MessageTarget::Messages(vec![message]),
                to: Some(world.archive),
            })
            .expect("the move applies");

        let connection = world.database.connection().expect("a connection");
        let queue = postio_storage::repository::OperationQueueRepository::new(&connection);
        let ops = queue.pending(world.account.id, Utc::now()).expect("queue");
        assert_eq!(ops.len(), 1, "one operation on one queue");
        assert_eq!(ops[0].operation.op_type(), "move");
        let sagas: i64 = connection
            .query_row("SELECT count(*) FROM cross_account_moves", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(
            sagas, 0,
            "and no saga exists for the common case to pay for"
        );
    }

    #[test]
    fn nothing_selected_and_nothing_focused_is_a_rejection() {
        let world = world();

        let outcome = world.run(Command::Archive {
            target: MessageTarget::Selection,
        });

        assert_eq!(outcome, Err(CommandError::rejected("Nothing selected")));
    }

    #[test]
    fn an_account_with_no_archive_folder_says_so() {
        let world = world();
        {
            let connection = world.database.connection().expect("a connection");
            let mailboxes = MailboxRepository::new(&connection);
            let archive = mailboxes
                .get(world.archive)
                .expect("a read")
                .expect("the folder");
            assert_eq!(archive.role, MailboxRole::Archive);
            mailboxes.delete(world.archive).expect("a delete");
        }
        let message = world.message(world.inbox, &[]);
        world.looking_at(world.inbox, &[], Some(message));

        let outcome = world.run(Command::Archive {
            target: MessageTarget::Selection,
        });

        assert!(
            matches!(outcome, Err(CommandError::Rejected(_))),
            "inventing a folder because a key was pressed is not this \
             command's decision to make"
        );
    }

    // ── Undo ─────────────────────────────────────────────────────────────

    #[test]
    fn undo_with_nothing_to_take_back_is_a_rejection() {
        let world = world();

        let outcome = world.run(Command::Undo);

        assert_eq!(
            outcome,
            Err(CommandError::rejected("Nothing to undo")),
            "a quiet hint, not a failure: `u` on a fresh session is ordinary"
        );
        assert!(world.drained().is_empty(), "and nothing happened");
    }

    #[test]
    fn undoing_twice_walks_back_through_history_rather_than_toggling() {
        // The inverses are applied directly. Replaying them through the bus
        // would record an undo of the undo.
        let world = world();
        let first = world.message(world.inbox, &[]);
        let second = world.message(world.inbox, &[]);
        for message in [first, second] {
            world.looking_at(world.inbox, &[], Some(message));
            world
                .run(Command::Delete {
                    target: MessageTarget::Selection,
                })
                .expect("delete");
            // Past the coalescing window would be better still, but two
            // deletes of different rows inside it are one gesture by design.
        }

        world.run(Command::Undo).expect("undo");

        assert_eq!(world.mailbox_of(first), world.inbox);
        assert_eq!(world.mailbox_of(second), world.inbox);
        assert_eq!(
            world.run(Command::Undo),
            Err(CommandError::rejected("Nothing to undo")),
            "a burst is one unit, so there is nothing behind it"
        );
    }

    // ── Wiring ───────────────────────────────────────────────────────────

    #[test]
    fn every_wired_command_has_a_handler_and_an_arm() {
        // A registry entry with no handler is a palette row that does
        // nothing; a handler for a command `act` does not match reports "not
        // wired up yet" from inside the thing that is supposed to wire it up.
        let world = world();
        let bus = dispatcher(world.actions.clone());
        assert_eq!(bus.wired().collect::<Vec<_>>(), WIRED.to_vec());

        let message = world.message(world.inbox, &[]);
        for id in WIRED.iter().copied().filter(|id| *id != CommandId::Undo) {
            world.looking_at(world.inbox, &[], Some(message));
            let outcome = world.run(Command::default_for(id));
            assert!(
                !matches!(&outcome, Err(CommandError::Rejected(reason)) if reason.contains("not wired up")),
                "`{id}` is registered on the bus but `act` has no arm for it"
            );
        }
    }
}
