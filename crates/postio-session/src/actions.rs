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

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use postio_core::bridge::EventSink;
use postio_core::dispatch::{CommandError, DispatcherBuilder};
use postio_core::state::{Resolved, SharedState};
use postio_core::undo::{UndoEntry, UndoKind, UndoStack};
use postio_core::{Command, CommandId, Event, MessageTarget};
use postio_model::mailbox::MailboxRole;
use postio_model::{
    AccountId, Flag, FlagSet, MailboxId, Message, MessageId, Operation, OperationTarget, ThreadId,
};
use postio_storage::repository::{
    ColumnFlag, FlagSource, MailboxRepository, MessageRepository, MessageSet,
    OperationQueueRepository, ThreadOrder, ThreadRepository,
};
use postio_storage::{Database, PooledConnection};

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
    CommandId::Undo,
];

/// Whether a verb is being performed or replayed backwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Recording {
    /// The user did this. Push it onto the stack and offer to take it back.
    Record,
    /// Undo is doing this. It is already history.
    Replay,
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
    /// A predicate the store resolves in one statement.
    Bulk {
        /// What to act on.
        set: MessageSet,
        /// Whose account it is. Read from the mailbox rather than from a row,
        /// because there is no row to read.
        account: AccountId,
        /// The folder those messages are in now.
        from: MailboxId,
    },
}

/// What a verb did, once it had done it.
///
/// Separated from announcing it so undo can replay the same work without
/// offering to undo the undo.
struct Applied {
    kind: UndoKind,
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
    pub fn run(&self, command: &Command, events: &EventSink) -> Result<(), CommandError> {
        match command {
            Command::Undo => self.undo(events),
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
        let applied = match command {
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
            other => {
                return Err(CommandError::rejected(format!(
                    "`{}` is not wired up yet",
                    other.id()
                )));
            }
        };
        self.announce(applied, events, recording);
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
        for command in entry.inverse() {
            self.act(command, events, Recording::Replay)?;
        }
        events.emit(Event::UndoPerformed {
            description: entry.description(),
        });
        Ok(())
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
    ) -> Result<Applied, CommandError> {
        let mut connection = self.connect()?;
        match self.aim(&connection, target)? {
            Aim::Rows(rows) => self.relocate_rows(&mut connection, rows, to, kind),
            Aim::Bulk { set, account, from } => {
                self.relocate_set(&mut connection, set, account, from, to, kind)
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
        from: MailboxId,
        to: Destination,
        kind: UndoKind,
    ) -> Result<Applied, CommandError> {
        let destination = mailbox_for(connection, account, to)?;
        if destination == from {
            return Err(CommandError::rejected("Already there"));
        }
        // The one number a bulk action is allowed to know, and the only thing
        // that needs it is the sentence in the toast.
        let count = MessageRepository::new(connection)
            .count_set(&set)
            .map_err(store_failure)? as usize;
        if count == 0 {
            return Err(CommandError::rejected("There is nothing here to move"));
        }

        let operation = match kind {
            UndoKind::Delete => Operation::Delete {
                from,
                trash: destination,
            },
            _ => Operation::Move {
                from,
                to: destination,
            },
        };
        let at = Utc::now();
        let transaction = connection.transaction().map_err(store_failure)?;
        // Enqueue before moving: the predicate for a whole-mailbox selection
        // is "the rows in this folder", and after the move there are none.
        let range = OperationQueueRepository::new(&transaction)
            .enqueue_set(account, &set, &operation, at)
            .map_err(store_failure)?
            .ok_or_else(|| CommandError::rejected("There is nothing here to move"))?;
        MessageRepository::new(&transaction)
            .move_set(&set, destination)
            .map_err(store_failure)?;
        transaction.commit().map_err(store_failure)?;

        Ok(Applied {
            kind,
            messages: Vec::new(),
            count,
            removed: Vec::new(),
            arrived: None,
            // Both ends reload. Neither can be told which rows moved, and the
            // one they moved into is as changed as the one they left.
            reloaded: vec![from, destination],
            changed: Vec::new(),
            inverse: vec![Command::Move {
                target: MessageTarget::Batch {
                    range,
                    from: destination,
                },
                to: Some(from),
            }],
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
        let account = rows[0].account_id;
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
                // The local write and its queue row in one transaction: a
                // queue row without its write tells the server about
                // something the user never saw, and a write without its row
                // silently never reaches the server.
                messages.move_to(ids, destination).map_err(store_failure)?;
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
                // One statement per source mailbox rather than one per
                // message: a multi-select spanning a handful of folders is a
                // handful of `enqueue_many` calls, not one `enqueue` per row.
                queue
                    .enqueue_many(account, ids, &operation, at)
                    .map_err(store_failure)?;
            }
        }
        transaction.commit().map_err(store_failure)?;

        let removed: Vec<(MailboxId, Vec<MessageId>)> = by_source.into_iter().collect();
        let messages: Vec<MessageId> = removed
            .iter()
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect();
        Ok(Applied {
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

    /// Set or clear one flag across the target.
    ///
    /// `want` of `None` toggles, and a toggle over more than one row means
    /// *make them agree*: if every row already carries the flag it comes off
    /// them all, otherwise it goes onto them all. The alternative — flipping
    /// each row independently — turns one keystroke over a mixed selection
    /// into a result nobody can predict without reading every row first.
    fn set_flag(
        &self,
        target: &MessageTarget,
        flag: Flag,
        want: Option<bool>,
    ) -> Result<Applied, CommandError> {
        let mut connection = self.connect()?;
        match self.aim(&connection, target)? {
            Aim::Rows(rows) => self.set_flag_rows(&mut connection, rows, flag, want),
            Aim::Bulk { set, account, from } => {
                self.set_flag_set(&mut connection, set, account, from, flag, want)
            }
        }
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
        from: MailboxId,
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

        let target = MessageTarget::Batch { range, from };
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
            kind: kind_for(&flag, wanted),
            messages: Vec::new(),
            count,
            removed: Vec::new(),
            arrived: None,
            // The rows did not go anywhere, but naming the ones that changed
            // is the read this path exists to avoid — so the list reloads the
            // folder rather than being handed eighty thousand ids.
            reloaded: vec![from],
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
        }
        transaction.commit().map_err(store_failure)?;

        let changed: Vec<MessageId> = touched.iter().map(|message| message.id).collect();
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
            kind: kind_for(&flag, wanted),
            count: changed.len(),
            messages: changed.clone(),
            removed: Vec::new(),
            arrived: None,
            reloaded: Vec::new(),
            changed,
            inverse: vec![inverse],
        })
    }

    // ── Saying what happened ─────────────────────────────────────────────

    /// Emit what the panes repaint from, and record what `u` takes back.
    fn announce(&self, applied: Applied, events: &EventSink, recording: Recording) {
        for (mailbox, messages) in &applied.removed {
            events.emit(Event::MessagesRemoved {
                mailbox: *mailbox,
                messages: messages.clone(),
            });
        }
        if let Some(mailbox) = applied.arrived {
            // The folder they landed in is longer than it was, and it may be
            // the one on screen — an undone archive has to reappear now, not
            // at the next sync.
            events.emit(Event::MessageListChanged { mailbox });
        }
        for mailbox in &applied.reloaded {
            events.emit(Event::MessageListChanged { mailbox: *mailbox });
        }
        if !applied.changed.is_empty() {
            events.emit(Event::MessagesChanged {
                messages: applied.changed.clone(),
            });
        }
        if recording == Recording::Replay {
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
        let (set, from) = match resolved {
            Resolved::Messages(ids) => return self.rows(connection, ids).map(Aim::Rows),
            Resolved::Thread(thread) => {
                let ids = thread_messages(connection, thread)?;
                return self.rows(connection, ids).map(Aim::Rows);
            }
            Resolved::Everything { mailbox, except } => {
                (MessageSet::InMailbox { mailbox, except }, mailbox)
            }
            Resolved::Batch { range, from } => (MessageSet::Queued(range), from),
        };
        // One row read, and it is a folder rather than a message: the account
        // is needed to find the Archive, and there is no message to ask.
        let account = MailboxRepository::new(connection)
            .get(from)
            .map_err(store_failure)?
            .ok_or_else(|| CommandError::rejected("That folder is no longer here"))?
            .account_id;
        Ok(Aim::Bulk { set, account, from })
    }

    /// The thread `A` means: the one the focused message belongs to.
    ///
    /// A whole-mailbox selection has no such message — `Ctrl+A` then `A` is a
    /// gesture with no answer rather than a bulk one, because "the thread of
    /// everything" is not a thing — so it is asked for rather than guessed at.
    fn thread_in_view(&self) -> Result<ThreadId, CommandError> {
        let connection = self.connect()?;
        let rows = match self.aim(&connection, &MessageTarget::Selection)? {
            Aim::Rows(rows) => rows,
            Aim::Bulk { .. } => {
                return Err(CommandError::rejected(
                    "Pick a message, and `A` archives its thread",
                ));
            }
        };
        rows[0]
            .thread_id
            .ok_or_else(|| CommandError::rejected("That message is not in a thread"))
    }

    fn connect(&self) -> Result<PooledConnection, CommandError> {
        self.database.connection().map_err(store_failure)
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
        MailboxRepository, MessageRepository, OperationQueueRepository, ThreadRepository,
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
            mailbox: world.inbox,
            messages: vec![message],
        }));
        assert!(
            events.contains(&Event::MessageListChanged {
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
            mailbox: world.inbox
        }));
        assert!(events.contains(&Event::MessageListChanged {
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
