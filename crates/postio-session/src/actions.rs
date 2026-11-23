//! The message verbs, as the command bus runs them: resolve the target, write SQLite, enqueue the
//! server operation, push undo, emit events.
//! Nothing here awaits the network. Undo replays inverses with `Recording::Replay` instead of going
//! through the bus, or `u` `u` would toggle.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use chrono::{Duration, Utc};
use postio_core::bridge::EventSink;
use postio_core::dispatch::{CommandError, DispatcherBuilder};
use postio_core::state::{Resolved, SharedState, ViewScope};
use postio_core::undo::{UndoEntry, UndoKind, UndoStack};
use postio_core::{
    Command, CommandId, ContactAddressAction, ContactEditAction, ContactGroupNewAction,
    ContactJoinAction, ContactNewAction, Event, MessageTarget,
};
use postio_model::ids::DraftId;
use postio_model::mailbox::MailboxRole;
use postio_model::{
    AccountId, AddressId, ContactGroupId, ContactId, ContactState, DraftState, Flag, FlagSet,
    LabelId, MailboxId, Message, MessageId, Operation, OperationTarget, ThreadId,
};
use postio_storage::repository::{
    ColumnFlag, ContactGroupRepository, ContactRepository, DraftRepository, FlagSource,
    LabelRepository, MailboxRepository, MailboxRoleRepository, MessageRepository, MessageSet,
    OperationQueueRepository, ThreadOrder, ThreadRepository,
};
use postio_storage::{Checkout, Store, WritePermit, WritePriority};

mod focus;
mod keys;

fn now() -> chrono::DateTime<Utc> {
    postio_ui::clock::now().with_timezone(&Utc)
}

/// Named once so the registration and the match in `Actions::act` cannot drift apart.
///
/// Public so a frontend can prove every command it passes on is one of these.
pub const WIRED: &[CommandId] = &[
    CommandId::Archive,
    CommandId::Delete,
    CommandId::Move,
    CommandId::Flag,
    CommandId::ToggleRead,
    CommandId::Snooze,
    CommandId::Unsnooze,
    CommandId::RemindIfNoReply,
    // After Unsnooze, matching the registry's order: `bus.wired()` reports in registry order
    // and a test compares the two lists directly.
    CommandId::AddLabel,
    CommandId::AcceptInvite,
    CommandId::DeclineInvite,
    CommandId::StopDigestingSender,
    CommandId::DismissMarker,
    CommandId::MarkSent,
    CommandId::RetrySend,
    CommandId::CancelSend,
    CommandId::Undo,
    CommandId::MapMailboxRole,
    CommandId::RestoreFiltered,
    CommandId::AcceptKey,
    CommandId::ForgetKey,
    CommandId::SweepInbox,
    CommandId::ContactSetArrival,
    CommandId::ContactJoin,
    CommandId::ContactAddAddress,
    CommandId::ContactDetachAddress,
    CommandId::ContactSetPreferred,
    CommandId::ContactNew,
    CommandId::ContactEdit,
    CommandId::ContactDelete,
    CommandId::ContactRestore,
    CommandId::SuggestionDismiss,
    CommandId::ContactGroupNew,
    CommandId::ContactGroupRename,
    CommandId::ContactGroupAdd,
    CommandId::ContactGroupRemove,
    CommandId::MuteThread,
    CommandId::SplitThread,
];

/// How long `Command::Snooze` hides a message when it names no time.
const DEFAULT_SNOOZE: Duration = Duration::hours(3);

pub use postio_core::command::RSVP_WINDOW;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Recording {
    Record,
    Replay,
    /// Repaints but raises no toast and takes no undo entry: the dwell mark fires per message, and
    /// recording each would bury the verb `u` should reach.
    Incidental,
}

impl Recording {
    async fn records(self) -> bool {
        self == Recording::Record
    }
}

#[derive(Debug, Clone, Copy)]
enum Destination {
    Role(MailboxRole),
    Mailbox(MailboxId),
}

/// A whole-mailbox selection stays a predicate: turning it into rows is the mailbox-sized read the
/// 16 ms budget forbids.
enum Aim {
    Rows(Vec<Message>),
    /// Split by account because the folder `Archive` names, the queue and the events' account are
    /// all per account (ADR 0005 Q11).
    Bulk(Vec<BulkUnit>),
}

struct BulkUnit {
    set: MessageSet,
    account: AccountId,
    /// `None` for a smart folder or the aggregate, whose rows span folders; a move out of one is
    /// grouped by source (#52).
    from: Option<MailboxId>,
}

struct Applied {
    kind: UndoKind,
    account: AccountId,
    /// Empty for a bulk unit, which knows how many it touched and not which.
    messages: Vec<MessageId>,
    count: usize,
    rows: Option<usize>,
    removed: Vec<(MailboxId, Vec<MessageId>)>,
    /// The list showing it must reload, or an undone archive would not reappear until the next
    /// sync.
    arrived: Option<MailboxId>,
    /// A bulk action reports here instead of `removed`: naming the rows that left is the read the
    /// predicate avoids.
    reloaded: Vec<MailboxId>,
    changed: Vec<MessageId>,
    mailboxes_changed: bool,
    /// `Some(true)` when names shown in mail could have changed, so every list and reader re-reads
    /// them.
    contacts_changed: Option<bool>,
    inverse: Vec<Command>,
    /// Set when shorter than the undo stack's expiry: an answer's reply leaves once its window
    /// closes.
    lasts: Option<std::time::Duration>,
    surfaced: bool,
}

impl Applied {
    fn changed_one(
        kind: UndoKind,
        account: AccountId,
        message: MessageId,
        inverse: Command,
    ) -> Applied {
        Applied {
            kind,
            account,
            messages: vec![message],
            count: 1,
            rows: None,
            removed: Vec::new(),
            arrived: None,
            reloaded: Vec::new(),
            changed: vec![message],
            mailboxes_changed: false,
            contacts_changed: None,
            inverse: vec![inverse],
            lasts: None,
            surfaced: false,
        }
    }
}

fn conversations<'a>(messages: impl Iterator<Item = &'a Message>) -> usize {
    messages
        .map(|message| match message.thread_id {
            Some(thread) => (Some(thread), None),
            None => (None, Some(message.id)),
        })
        .collect::<std::collections::BTreeSet<_>>()
        .len()
}

#[derive(Clone)]
pub struct Actions {
    database: Store,
    state: SharedState,
    undo: Arc<Mutex<UndoStack>>,
    /// `None` for a bus composed without them; Focus verbs then say they cannot run.
    blobs: Option<postio_storage::BlobStore>,
    focus: crate::focus::FocusSettings,
}

impl Actions {
    pub fn new(database: Store, state: SharedState) -> Self {
        Actions {
            database,
            state,
            undo: Arc::new(Mutex::new(UndoStack::new())),
            blobs: None,
            focus: crate::focus::FocusSettings::default(),
        }
    }

    #[must_use]
    pub fn with_focus(mut self, focus: crate::focus::FocusSettings) -> Self {
        self.focus = focus;
        self
    }

    #[must_use]
    pub fn with_blob_store(mut self, blobs: postio_storage::BlobStore) -> Self {
        self.blobs = Some(blobs);
        self
    }

    /// `Err` is what the user sees: a rejection becomes a quiet hint, a failure something louder.
    pub async fn run(&self, command: &Command, events: &EventSink) -> Result<(), CommandError> {
        match command {
            Command::Undo => self.undo(events).await,
            // A rejection is silent here: `set_flag` rejects "Already set" when nothing changes,
            // which is the ordinary case once a mailbox is worked through. A `Failed` still gets
            // through.
            dwell @ Command::MarkReadOnDwell { .. } => {
                match self
                    .act_again_if_busy(dwell, events, Recording::Incidental)
                    .await
                {
                    Err(CommandError::Rejected(_)) => Ok(()),
                    other => other,
                }
            }
            other => {
                self.act_again_if_busy(other, events, Recording::Record)
                    .await
            }
        }
    }

    /// Re-runs `act` while the store reports busy: another writer committed between read and write,
    /// and the verb rolled back whole (#1594). Bounded.
    async fn act_again_if_busy(
        &self,
        command: &Command,
        events: &EventSink,
        recording: Recording,
    ) -> Result<(), CommandError> {
        let mut tries = 0u64;
        loop {
            match self.act(command, events, recording).await {
                Err(CommandError::Failed(message))
                    if message == STORE_BUSY && tries < BUSY_RETRIES =>
                {
                    tries += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(10 * tries)).await;
                }
                other => return other,
            }
        }
    }

    async fn act(
        &self,
        command: &Command,
        events: &EventSink,
        recording: Recording,
    ) -> Result<(), CommandError> {
        let applied: Vec<Applied> = match command {
            Command::Archive { target } => {
                self.relocate(
                    target,
                    Destination::Role(MailboxRole::Archive),
                    UndoKind::Archive,
                )
                .await?
            }
            Command::Delete { target } => {
                self.relocate(
                    target,
                    Destination::Role(MailboxRole::Trash),
                    UndoKind::Delete,
                )
                .await?
            }
            Command::Move { target, to } => {
                // `None` means ask the user; the folder picker is postio-agr.2.
                let to = to.ok_or_else(|| {
                    CommandError::rejected("Pick a folder to move to — drag the rows onto one")
                })?;
                self.relocate(target, Destination::Mailbox(to), UndoKind::Move)
                    .await?
            }
            Command::Flag { target, flagged } => {
                self.set_flag(target, Flag::Flagged, *flagged).await?
            }
            // `\Seen` is stored the other way up from how the verb reads:
            // marking unread is clearing a flag, not setting one.
            Command::ToggleRead { target, unread } => {
                self.set_flag(target, Flag::Seen, unread.map(|unread| !unread))
                    .await?
            }
            Command::AddLabel { target, label, on } => {
                // `None` is half a request, like `Move { to: None }`; the window opens the picker
                // first.
                let label = label.ok_or_else(|| CommandError::rejected("Pick a label to add"))?;
                vec![self.set_label(target, label, *on).await?]
            }
            Command::Snooze { target, until } => {
                let until = until.unwrap_or_else(|| now() + DEFAULT_SNOOZE);
                vec![self.snooze(target, until).await?]
            }
            Command::Unsnooze { target } => vec![self.unsnooze(target).await?],
            Command::RemindIfNoReply { target, at } => vec![self.remind(target, *at).await?],
            Command::AcceptInvite { message } => {
                vec![
                    self.answer(*message, postio_calendar::Answer::Accept)
                        .await?,
                ]
            }
            Command::DeclineInvite { message } => {
                vec![
                    self.answer(*message, postio_calendar::Answer::Decline)
                        .await?,
                ]
            }
            Command::DismissMarker { target, dismissed } => {
                vec![self.dismiss(target, *dismissed).await?]
            }
            Command::RestoreFiltered { target, restored } => {
                self.restore(target, *restored).await?
            }
            Command::ArchiveDigest { delivery, archived } => {
                self.archive_digest(*delivery, *archived).await?
            }
            Command::StopDigestingSender {
                target,
                stopped,
                kept,
            } => vec![self.stop_digesting(target, *stopped, kept.as_ref()).await?],
            Command::AcceptKey {
                message,
                accepted,
                kept,
            } => vec![
                self.accept_key(*message, *accepted, kept.as_deref())
                    .await?,
            ],
            Command::ForgetKey {
                message,
                forgotten,
                kept,
            } => vec![self.forget_key(*message, *forgotten, kept.as_ref()).await?],
            Command::SweepInbox => self.sweep().await?,
            Command::UnsweepInbox { target } => self.unsweep(target).await?,
            // Deliberately `Some(true)` rather than a toggle: a dwell says
            // "this was read", never "flip whatever it was".
            Command::MarkSent { draft } => vec![self.mark_sent(*draft).await?],
            Command::RetrySend { draft } => vec![self.retry_send(*draft).await?],
            Command::CancelSend { draft } => vec![self.cancel_send(*draft).await?],
            Command::MapMailboxRole {
                account,
                role,
                path,
            } => vec![
                self.map_mailbox_role(*account, *role, path.as_deref())
                    .await?,
            ],
            Command::ContactJoin { action } => {
                let applied = self.join_contacts(action).await?;
                self.rules_follow_people().await;
                vec![applied]
            }
            Command::ContactAddAddress { action } => {
                let applied = self.contact_address(action).await?;
                self.rules_follow_people().await;
                vec![applied]
            }
            Command::ContactDetachAddress { address } => {
                let address = address.ok_or_else(|| CommandError::rejected("Pick an address"))?;
                let applied = self.detach_address(address).await?;
                self.rules_follow_people().await;
                vec![applied]
            }
            Command::ContactSetPreferred { person, address } => {
                let (Some(person), Some(address)) = (person, address) else {
                    return Err(CommandError::rejected("Pick an address"));
                };
                vec![self.set_preferred(*person, *address).await?]
            }
            Command::ContactNew { action } => {
                let applied = self.new_contact(action).await?;
                self.rules_follow_people().await;
                vec![applied]
            }
            Command::ContactEdit { action } => vec![self.edit_contact(action).await?],
            Command::ContactSetArrival { address, arrival } => {
                let applied = self.set_arrival(address, *arrival).await?;
                self.rules_follow_people().await;
                vec![applied]
            }
            Command::ContactDelete { person, group } => match (person, group) {
                (Some(person), _) => {
                    let applied = self.delete_contact(*person).await?;
                    self.rules_follow_people().await;
                    vec![applied]
                }
                (None, Some(group)) => vec![self.delete_group(*group).await?],
                (None, None) => return Err(CommandError::rejected("Choose someone first")),
            },
            Command::ContactRestore { person, state } => {
                let person =
                    person.ok_or_else(|| CommandError::rejected("Choose someone first"))?;
                let applied = self
                    .restore_contact(person, state.unwrap_or(ContactState::Live))
                    .await?;
                self.rules_follow_people().await;
                vec![applied]
            }
            // Not an undo unit: it hides a suggestion and touches nobody.
            Command::SuggestionDismiss { pair } => {
                let (a, b) =
                    pair.ok_or_else(|| CommandError::rejected("Choose a suggestion first"))?;
                let (connection, _permit) = self.connect().await?;
                ContactRepository::new(&connection)
                    .dismiss(a, b)
                    .await
                    .map_err(contact_failure)?;
                events.emit(Event::ContactsChanged {
                    names_changed: false,
                });
                Vec::new()
            }
            Command::ContactGroupNew { action } => vec![self.new_group(action).await?],
            Command::ContactGroupRename { group, name } => {
                let (Some(group), Some(name)) = (group, name) else {
                    return Err(CommandError::rejected("Name the group"));
                };
                vec![self.rename_group(*group, name).await?]
            }
            Command::ContactGroupAdd { group, people } => {
                let group = group.ok_or_else(|| CommandError::rejected("Choose a group"))?;
                vec![self.group_members(group, people, true).await?]
            }
            Command::ContactGroupRemove { group, people } => {
                let group = group.ok_or_else(|| CommandError::rejected("Choose a group"))?;
                vec![self.group_members(group, people, false).await?]
            }
            Command::MuteThread { thread, muted } => {
                let thread = thread.ok_or_else(|| CommandError::rejected("Choose a thread"))?;
                vec![self.mute(thread, *muted, events).await?]
            }
            Command::SplitThread {
                rejoin: Some((split, original)),
                ..
            } => vec![self.unsplit(*split, *original, events).await?],
            Command::SplitThread {
                message: Some(message),
                rejoin: None,
            } => vec![self.split(*message, events).await?.0],
            Command::SplitThread { .. } => {
                return Err(CommandError::rejected("Choose a message to split off"));
            }
            Command::MarkReadOnDwell { message } => {
                self.set_flag(
                    &MessageTarget::Messages(vec![*message]),
                    Flag::Seen,
                    Some(true),
                )
                .await?
            }
            other => {
                return Err(CommandError::rejected(format!(
                    "`{}` is not wired up yet",
                    other.id()
                )));
            }
        };
        for unit in applied {
            self.announce(unit, events, recording).await;
        }
        Ok(())
    }

    /// What Undo would take back now, in the toast's words, without taking it; `None` when nothing
    /// can be undone.
    pub fn peek_description(&self) -> Option<String> {
        self.undo
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .peek()
            .map(UndoEntry::description)
    }

    /// Nothing to take back is a rejection, not a failure.
    async fn undo(&self, events: &EventSink) -> Result<(), CommandError> {
        let entry = self
            .stack()
            .await
            .undo()
            .ok_or_else(|| CommandError::rejected("Nothing to undo"))?;
        // Not a replay: the undo stack records only source rows, so the saga table is asked first
        // (#531, ADR 0005 Q9).
        if let Some(cancelled) = self
            .cancel_cross_account_moves(entry.messages(), events)
            .await?
        {
            events.emit(Event::UndoPerformed {
                description: cancelled,
            });
            return Ok(());
        }
        for command in entry.inverse() {
            self.act(command, events, Recording::Replay).await?;
        }
        events.emit(Event::UndoPerformed {
            description: entry.undone_description(),
        });
        Ok(())
    }

    /// Withdraw the cross-account moves still open for `messages`; `None` when none started a saga.
    ///
    /// Only before the copy is confirmed. After that it refuses: un-hiding the source while the
    /// target keeps its copy would leave mail on two servers with nothing recording it (#531, ADR
    /// 0005 Q9).
    async fn cancel_cross_account_moves(
        &self,
        messages: &[MessageId],
        events: &EventSink,
    ) -> Result<Option<String>, CommandError> {
        use postio_storage::repository::{CrossAccountMoveRepository, MovePhase};

        // `done` included: a finished move is the one most likely taken back (#531).
        const OPEN: [MovePhase; 4] = [
            MovePhase::Copying,
            MovePhase::Unconfirmed,
            MovePhase::Confirmed,
            MovePhase::Done,
        ];
        // Read turn first: nearly every undo has no saga, and taking the writer to find out made
        // each wait twice (#1607).
        {
            let reader = self.database.read().await.map_err(store_failure)?;
            let open = CrossAccountMoveRepository::new(&reader.checkout())
                .for_sources(messages, &OPEN)
                .await
                .map_err(store_failure)?;
            if open.is_empty() {
                return Ok(None);
            }
        }

        let (mut connection, _permit) = self.connect().await?;
        let sagas = CrossAccountMoveRepository::new(&connection)
            .for_sources(messages, &OPEN)
            .await
            .map_err(store_failure)?;
        if sagas.is_empty() {
            return Ok(None);
        }

        let mut undone = 0usize;
        let mut skipped = 0usize;
        let mut reloaded: BTreeSet<MailboxId> = BTreeSet::new();
        let mut accounts: BTreeSet<postio_model::ids::AccountId> = BTreeSet::new();
        let at = now();
        let transaction = connection.transaction().await.map_err(store_failure)?;
        {
            let repository = MessageRepository::new(&transaction);
            let queue = OperationQueueRepository::new(&transaction);
            let sagas_repository = CrossAccountMoveRepository::new(&transaction);
            for saga in &sagas {
                match saga.phase {
                    MovePhase::Copying => {
                        cancel_one(&repository, &queue, &sagas_repository, saga).await?;
                        undone += 1;
                    }
                    // ADR 0005 Q9: an unprovable copy means stop and ask; an inverse saga on it
                    // would guess. Left as is and counted.
                    MovePhase::Unconfirmed => skipped += 1,
                    // Abort the forward saga first: its pending source removal would delete the
                    // source after the inverse restored it.
                    MovePhase::Confirmed => {
                        withdraw_pending(&queue, saga.source_message).await?;
                        sagas_repository
                            .transition(saga.id, MovePhase::Aborted)
                            .await
                            .map_err(store_failure)?;
                        invert_one(&repository, &queue, &sagas_repository, saga, at).await?;
                        undone += 1;
                    }
                    MovePhase::Done => {
                        invert_one(&repository, &queue, &sagas_repository, saga, at).await?;
                        undone += 1;
                    }
                    MovePhase::Aborted => {}
                }
                for mailbox in [saga.source_mailbox, saga.target_mailbox]
                    .into_iter()
                    .flatten()
                {
                    reloaded.insert(mailbox);
                }
                for account in [saga.source_account, saga.target_account]
                    .into_iter()
                    .flatten()
                {
                    accounts.insert(account);
                }
            }
        }

        // Rolled back, not committed, so a partial undo and a refused one cannot be told apart in
        // the store.
        if undone == 0 {
            transaction.rollback().await.map_err(store_failure)?;
            return Err(CommandError::rejected(
                "That move reached the other account but Postio could not confirm it,                  so taking it back would be a guess. Nothing was changed.",
            ));
        }
        transaction.commit().await.map_err(store_failure)?;

        for account in accounts {
            for mailbox in &reloaded {
                events.emit(Event::MessageListChanged {
                    account,
                    mailbox: *mailbox,
                });
            }
        }

        let plural = if undone == 1 { "message" } else { "messages" };
        // One sentence whatever the phase (ADR 0005 Q9), except the skipped count: that mail is not
        // back.
        Ok(Some(match skipped {
            0 => format!("Moved {undone} {plural} back"),
            skipped => format!(
                "Moved {undone} {plural} back; {skipped} could not be confirmed and \
                 were left where they are"
            ),
        }))
    }

    /// Move messages between folders. The source mailbox is read per message: a unified selection
    /// can span folders, and undo must restore each.
    async fn relocate(
        &self,
        target: &MessageTarget,
        to: Destination,
        kind: UndoKind,
    ) -> Result<Vec<Applied>, CommandError> {
        let (mut connection, _permit) = self.connect().await?;
        match self.aim(&connection, target).await? {
            // One unit per account: a role destination resolves per account, and filing mail into
            // another account's Archive would move it on the wrong server (#182).
            Aim::Rows(rows) => {
                let mut by_account: BTreeMap<AccountId, Vec<Message>> = BTreeMap::new();
                for row in rows {
                    by_account.entry(row.account_id).or_default().push(row);
                }
                // Resolve every destination before writing: each account commits its own
                // transaction, so a failure half way would leave part of the selection moved.
                for account in by_account.keys() {
                    mailbox_for(&connection, *account, to).await?;
                }

                let mut units = Vec::with_capacity(by_account.len());
                let mut nothing_to_do = None;
                for (_, rows) in by_account {
                    match self.relocate_rows(&mut connection, rows, to, kind).await {
                        Ok(unit) => units.push(unit),
                        // Nothing to do in one account is not a failure unless no account had
                        // anything to do.
                        Err(CommandError::Rejected(reason)) => nothing_to_do = Some(reason),
                        Err(error) => return Err(error),
                    }
                }
                match (units.is_empty(), nothing_to_do) {
                    (true, Some(reason)) => Err(CommandError::Rejected(reason)),
                    _ => Ok(units),
                }
            }
            Aim::Bulk(units) => {
                for unit in &units {
                    mailbox_for(&connection, unit.account, to).await?;
                }
                let mut applied = Vec::with_capacity(units.len());
                let mut nothing_to_do = None;
                for unit in units {
                    match self
                        .relocate_set(&mut connection, unit.set, unit.account, unit.from, to, kind)
                        .await
                    {
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

    /// Move a whole mailbox's worth of messages in four statements, never naming them; one `u`
    /// suffices because of the run of queue rows (`OperationRange`).
    async fn relocate_set(
        &self,
        connection: &mut Checkout,
        set: MessageSet,
        account: AccountId,
        from: Option<MailboxId>,
        to: Destination,
        kind: UndoKind,
    ) -> Result<Applied, CommandError> {
        let destination = mailbox_for(connection, account, to).await?;
        if from == Some(destination) {
            return Err(CommandError::rejected("Already there"));
        }

        // A smart folder's rows span folders and the queue payload carries one `from` per run,
        // so enqueue a source folder at a time. One `SELECT DISTINCT` over the same index.
        let sources = match from {
            Some(from) => vec![from],
            None => {
                let mut sources = MessageRepository::new(connection)
                    .mailboxes_of_set(&set)
                    .await
                    .map_err(store_failure)?;
                // Rows already at the destination are not moving; enqueueing them would put them in
                // the run `u` takes back.
                sources.retain(|mailbox| *mailbox != destination);
                sources
            }
        };
        if sources.is_empty() {
            return Err(CommandError::rejected("There is nothing here to move"));
        }

        let at = now();
        let transaction = connection.transaction().await.map_err(store_failure)?;
        let mut count = 0usize;
        let mut inverse = Vec::new();
        let mut reloaded = vec![destination];
        for source in sources {
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
                .await
                .map_err(store_failure)?
            else {
                continue;
            };
            count += MessageRepository::new(&transaction)
                .move_set(&group, destination)
                .await
                .map_err(store_failure)?;
            reloaded.push(source);
            inverse.push(Command::Move {
                target: MessageTarget::Batch {
                    range,
                    account,
                    from: Some(destination),
                },
                to: Some(source),
            });
        }
        if count == 0 {
            return Err(CommandError::rejected("There is nothing here to move"));
        }
        transaction.commit().await.map_err(store_failure)?;

        Ok(Applied {
            rows: None,
            lasts: None,
            surfaced: false,
            account,
            kind,
            messages: Vec::new(),
            count,
            removed: Vec::new(),
            arrived: None,
            reloaded,
            changed: Vec::new(),
            mailboxes_changed: false,
            contacts_changed: None,
            inverse,
        })
    }

    async fn relocate_rows(
        &self,
        connection: &mut Checkout,
        rows: Vec<Message>,
        to: Destination,
        kind: UndoKind,
    ) -> Result<Applied, CommandError> {
        // `relocate` grouped by account, so the role resolves against this account's folders
        // (#182).
        let account = rows[0].account_id;
        debug_assert!(
            rows.iter().all(|row| row.account_id == account),
            "relocate_rows is single-account by construction; `relocate` groups"
        );
        // A destination in another account becomes the three-phase saga of ADR 0005 Q9; this row is
        // its local-first half.
        if let Destination::Mailbox(mailbox) = to {
            let target = MailboxRepository::new(connection)
                .get(mailbox)
                .await
                .map_err(store_failure)?
                .ok_or_else(|| CommandError::rejected("That folder no longer exists"))?;
            if target.account_id != account {
                return self
                    .cross_account_relocate(connection, rows, &target, kind)
                    .await;
            }
        }
        let destination = mailbox_for(connection, account, to).await?;

        let mut by_source: BTreeMap<MailboxId, Vec<MessageId>> = BTreeMap::new();
        // The toast counts the rows chosen, including those already there (#2086).
        let chosen_rows = conversations(rows.iter());
        for message in &rows {
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

        let at = now();
        // Rows and queue rows are `postio-storage`'s half (ADR 0028); this side owns the
        // transaction, and sync hands over its own (ADR 0008 Q3).
        let transaction = connection.transaction().await.map_err(store_failure)?;
        postio_storage::actions::relocate(
            &transaction,
            account,
            &by_source,
            destination,
            match kind {
                UndoKind::Delete => postio_storage::actions::Relocation::Trash,
                _ => postio_storage::actions::Relocation::Move,
            },
            at,
        )
        .await
        .map_err(store_failure)?;
        transaction.commit().await.map_err(store_failure)?;

        let removed: Vec<(MailboxId, Vec<MessageId>)> = by_source.into_iter().collect();
        let messages: Vec<MessageId> = removed
            .iter()
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect();
        Ok(Applied {
            rows: Some(chosen_rows),
            lasts: None,
            surfaced: false,
            account,
            kind,
            count: messages.len(),
            messages,
            mailboxes_changed: false,
            contacts_changed: None,
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

    /// Local-first half of a cross-account move (#188, ADR 0005 Q9): one saga per message; the
    /// remove waits for the copy's confirmation, which the saga table enforces.
    ///
    /// No undo entry yet: the inverse is an inverse saga, which the servers cannot do yet.
    async fn cross_account_relocate(
        &self,
        connection: &mut Checkout,
        rows: Vec<Message>,
        target: &postio_model::Mailbox,
        kind: UndoKind,
    ) -> Result<Applied, CommandError> {
        let account = rows[0].account_id;
        let at = now();
        let transaction = connection.transaction().await.map_err(store_failure)?;
        let mut moved: Vec<MessageId> = Vec::new();
        let mut by_source: BTreeMap<MailboxId, Vec<MessageId>> = BTreeMap::new();
        {
            let messages = MessageRepository::new(&transaction);
            let queue = OperationQueueRepository::new(&transaction);
            let sagas = postio_storage::repository::CrossAccountMoveRepository::new(&transaction);
            for row in &rows {
                // The raw `.eml` and payloads are content-addressed, so the copy shares them; the
                // body text is copied since ADR 0020 put it in the row.
                let mut copy = row.clone();
                copy.id = MessageId::UNASSIGNED;
                copy.account_id = target.account_id;
                copy.mailbox_id = target.id;
                copy.thread_id = None;
                // Reset all server identity at once: clearing fields one by one left the copy
                // claiming account A's `remote_id` and `mod_seq` (#940).
                copy.server = postio_model::ServerIdentifiers::default();
                let copy_id = messages.create(&mut copy).await.map_err(store_failure)?;
                if let Some(body) = messages.body(row.id).await.map_err(store_failure)? {
                    messages
                        .set_body(copy_id, &body, row.sync.body_state)
                        .await
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
                    .await
                    .map_err(store_failure)?;

                queue
                    .enqueue(
                        target.account_id,
                        postio_model::OperationTarget::Message(copy_id),
                        &Operation::CrossAccountCopy { saga },
                        at,
                    )
                    .await
                    .map_err(store_failure)?;
                queue
                    .enqueue(
                        account,
                        postio_model::OperationTarget::Message(row.id),
                        &Operation::CrossAccountRemove { saga },
                        at,
                    )
                    .await
                    .map_err(store_failure)?;
                messages
                    .set_deleted_locally(&[row.id], true)
                    .await
                    .map_err(store_failure)?;

                moved.push(row.id);
                by_source.entry(row.mailbox_id).or_default().push(row.id);
            }
        }
        transaction.commit().await.map_err(store_failure)?;

        Ok(Applied {
            rows: None,
            lasts: None,
            surfaced: false,
            account,
            kind,
            count: moved.len(),
            messages: moved,
            // Deliberately empty: see the doc comment. `u` says "nothing to
            // undo" rather than pretending a saga is a move.
            mailboxes_changed: false,
            contacts_changed: None,
            inverse: Vec::new(),
            arrived: None,
            reloaded: Vec::new(),
            changed: Vec::new(),
            removed: by_source.into_iter().collect(),
        })
    }

    /// Hide the targeted messages from every ordinary list until `until`. Row selection only.
    ///
    /// Local only: no queue row, the server never hears of a snooze. Reports `removed` rather than
    /// `changed` because the row leaves the list.
    async fn snooze(
        &self,
        target: &MessageTarget,
        until: chrono::DateTime<Utc>,
    ) -> Result<Applied, CommandError> {
        let (connection, _permit) = self.connect().await?;
        let rows = match self.aim(&connection, target).await? {
            Aim::Rows(rows) => rows,
            Aim::Bulk(_) => {
                return Err(CommandError::rejected("Select the messages to snooze"));
            }
        };
        let account = rows[0].account_id;
        let ids: Vec<MessageId> = rows.iter().map(|message| message.id).collect();
        MessageRepository::new(&connection)
            .snooze(&ids, until)
            .await
            .map_err(store_failure)?;

        let mut removed: BTreeMap<MailboxId, Vec<MessageId>> = BTreeMap::new();
        for message in &rows {
            removed
                .entry(message.mailbox_id)
                .or_default()
                .push(message.id);
        }
        Ok(Applied {
            rows: None,
            lasts: None,
            surfaced: false,
            account,
            kind: UndoKind::Snooze,
            count: ids.len(),
            messages: ids.clone(),
            removed: removed.into_iter().collect(),
            arrived: None,
            reloaded: Vec::new(),
            changed: Vec::new(),
            mailboxes_changed: false,
            contacts_changed: None,
            inverse: vec![Command::Unsnooze {
                target: MessageTarget::Messages(ids),
            }],
        })
    }

    /// Cancel a snooze now. Reports `reloaded` rather than `arrived`: nothing moved, the rows
    /// reappear in the folders they were already filed in.
    async fn unsnooze(&self, target: &MessageTarget) -> Result<Applied, CommandError> {
        let (connection, _permit) = self.connect().await?;
        // A conversation row in Snoozed stands for the asleep messages, which a thread's visible
        // members leave out.
        let threads = match self.state.read(|app| app.resolve(target)) {
            Some(Resolved::Thread(thread)) => Some(vec![thread]),
            Some(Resolved::Threads(threads)) => Some(threads),
            _ => None,
        };
        let aimed = match threads {
            Some(threads) => {
                let mut ids = Vec::new();
                for thread in threads {
                    let asleep = ThreadRepository::new(&connection)
                        .snoozed_messages(thread)
                        .await
                        .map_err(store_failure)?;
                    ids.extend(asleep.into_iter().map(|row| row.id));
                }
                if ids.is_empty() {
                    return Err(CommandError::rejected("Nothing there is snoozed"));
                }
                Aim::Rows(self.rows(&connection, ids).await?)
            }
            None => self.aim(&connection, target).await?,
        };
        let rows = match aimed {
            Aim::Rows(rows) => rows,
            Aim::Bulk(_) => {
                return Err(CommandError::rejected("Select the messages to unsnooze"));
            }
        };
        let account = rows[0].account_id;
        let ids: Vec<MessageId> = rows.iter().map(|message| message.id).collect();
        MessageRepository::new(&connection)
            .unsnooze(&ids)
            .await
            .map_err(store_failure)?;

        let reloaded: Vec<MailboxId> = rows
            .iter()
            .map(|message| message.mailbox_id)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        // The way back re-snoozes each message until the time it had, grouped by that time; a
        // message that was not asleep stays awake.
        let mut asleep: BTreeMap<chrono::DateTime<Utc>, Vec<MessageId>> = BTreeMap::new();
        for message in &rows {
            if let Some(until) = message.snoozed_until {
                asleep.entry(until).or_default().push(message.id);
            }
        }
        Ok(Applied {
            rows: None,
            lasts: None,
            surfaced: false,
            account,
            kind: UndoKind::Unsnooze,
            count: ids.len(),
            messages: ids,
            removed: Vec::new(),
            arrived: None,
            reloaded,
            changed: Vec::new(),
            mailboxes_changed: false,
            contacts_changed: None,
            inverse: asleep
                .into_iter()
                .map(|(until, ids)| Command::Snooze {
                    target: MessageTarget::Messages(ids),
                    until: Some(until),
                })
                .collect(),
        })
    }

    /// Put a label on the selection, or take it off (#780).
    ///
    /// Writes both the `message_labels` row the UI renders and the IMAP keyword the queue carries;
    /// either alone would be invisible to other clients or have no name or colour.
    /// `want` of `None` toggles, as `Flag` does.
    async fn set_label(
        &self,
        target: &MessageTarget,
        label: LabelId,
        want: Option<bool>,
    ) -> Result<Applied, CommandError> {
        let (mut connection, _permit) = self.connect().await?;
        let rows = match self.aim(&connection, target).await? {
            Aim::Rows(rows) => rows,
            // A whole mailbox would need the counted shape `set_flag_set` has; nothing asks for it
            // yet.
            Aim::Bulk { .. } => {
                return Err(CommandError::rejected(
                    "Labelling a whole mailbox at once is not supported yet",
                ));
            }
        };
        let account = rows[0].account_id;

        let name = {
            let repository = LabelRepository::new(&connection);
            repository
                .get(label)
                .await
                .map_err(store_failure)?
                .ok_or_else(|| CommandError::rejected("That label no longer exists"))?
                .name
        };
        let keyword = Flag::Keyword(name);

        // A loop, not `map().collect()`: the read awaits.
        let carried: Vec<bool> = {
            let repository = LabelRepository::new(&connection);
            let mut carried = Vec::with_capacity(rows.len());
            for message in rows.iter() {
                let labels = repository
                    .for_message(message.id)
                    .await
                    .map_err(store_failure)?;
                carried.push(labels.contains(&label));
            }
            carried
        };
        let wanted = want.unwrap_or_else(|| !carried.iter().all(|has| *has));

        let touched: Vec<&Message> = rows
            .iter()
            .zip(&carried)
            .filter(|(_, has)| **has != wanted)
            .map(|(message, _)| message)
            .collect();
        if touched.is_empty() {
            return Err(CommandError::rejected("Already set"));
        }

        let one: FlagSet = std::iter::once(keyword.clone()).collect();
        let at = now();
        let transaction = connection.transaction().await.map_err(store_failure)?;
        {
            let labels = LabelRepository::new(&transaction);
            let messages = MessageRepository::new(&transaction);
            let queue = OperationQueueRepository::new(&transaction);
            for message in &touched {
                if wanted {
                    labels
                        .attach(message.id, label)
                        .await
                        .map_err(store_failure)?;
                } else {
                    labels
                        .detach(message.id, label)
                        .await
                        .map_err(store_failure)?;
                }
                let mut flags = message.flags.clone();
                if wanted {
                    flags.insert(keyword.clone());
                } else {
                    flags.remove(&keyword);
                }
                messages
                    .set_flags(message.id, &flags, FlagSource::Local)
                    .await
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
                    .await
                    .map_err(store_failure)?;
            }
        }
        transaction.commit().await.map_err(store_failure)?;

        let changed: Vec<MessageId> = touched.iter().map(|message| message.id).collect();
        // Every touched row held the opposite value, so one command takes all back.
        let inverse = Command::AddLabel {
            target: MessageTarget::Messages(changed.clone()),
            label: Some(label),
            on: Some(!wanted),
        };
        Ok(Applied {
            rows: None,
            lasts: None,
            surfaced: false,
            account,
            kind: UndoKind::Label,
            count: changed.len(),
            messages: changed.clone(),
            removed: Vec::new(),
            arrived: None,
            reloaded: Vec::new(),
            changed,
            // A label is a keyword, not a move: no folder gained or lost a row.
            mailboxes_changed: false,
            contacts_changed: None,
            inverse: vec![inverse],
        })
    }

    /// Set or clear one flag across the target. `want` of `None` toggles, and over several rows
    /// that means make them agree.
    ///
    /// Across accounts it is still one decision: a unified selection arrives as one unit per
    /// account, and deciding per unit could flag one account while unflagging another (#811).
    async fn set_flag(
        &self,
        target: &MessageTarget,
        flag: Flag,
        want: Option<bool>,
    ) -> Result<Vec<Applied>, CommandError> {
        let (mut connection, _permit) = self.connect().await?;
        match self.aim(&connection, target).await? {
            Aim::Rows(rows) => self
                .set_flag_rows(&mut connection, rows, flag, want)
                .await
                .map(|one| vec![one]),
            Aim::Bulk(units) => {
                let wanted = match want {
                    Some(wanted) => wanted,
                    None => self.agreeing_on(&connection, &units, &flag).await?,
                };
                let mut applied = Vec::with_capacity(units.len());
                let mut nothing_to_do = None;
                for unit in units {
                    match self
                        .set_flag_set(
                            &mut connection,
                            unit.set,
                            unit.account,
                            unit.from,
                            flag.clone(),
                            Some(wanted),
                        )
                        .await
                    {
                        Ok(one) => applied.push(one),
                        // This account already agreed; that is a failure only if no account had
                        // anything to do.
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

    /// Which way a toggle over `units` goes: on, unless they all carry it. One indexed `count(*)`
    /// per unit, never a read.
    async fn agreeing_on(
        &self,
        connection: &Checkout,
        units: &[BulkUnit],
        flag: &Flag,
    ) -> Result<bool, CommandError> {
        let column = ColumnFlag::of(flag)
            .ok_or_else(|| CommandError::rejected("That flag does not work on a whole mailbox"))?;
        let repository = MessageRepository::new(connection);
        for unit in units {
            let disagreeing = repository
                .count_set(&unit.set.clone().with_flag(column, false))
                .await
                .map_err(store_failure)?;
            if disagreeing > 0 {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Set or clear one flag across a whole mailbox without naming a row: two indexed counts and
    /// three statements.
    ///
    /// Both sides are counted: the toggle needs the disagreeing count, the toast needs the other,
    /// and together they tell an empty mailbox from one that already agrees.
    /// The queue rows are written only over the disagreeing rows, because the queue run is the undo
    /// set and `u` must not clear a flag this action never set.
    async fn set_flag_set(
        &self,
        connection: &mut Checkout,
        set: MessageSet,
        account: AccountId,
        from: Option<MailboxId>,
        flag: Flag,
        want: Option<bool>,
    ) -> Result<Applied, CommandError> {
        // Only the two flags with a column of their own; `Flag` and `ToggleRead` are the only
        // flagging verbs.
        let column = ColumnFlag::of(&flag)
            .ok_or_else(|| CommandError::rejected("That flag does not work on a whole mailbox"))?;
        let repository = MessageRepository::new(connection);
        let carrying = |present: bool| set.clone().with_flag(column, present);
        let without = repository
            .count_set(&carrying(false))
            .await
            .map_err(store_failure)? as usize;
        let with = repository
            .count_set(&carrying(true))
            .await
            .map_err(store_failure)? as usize;
        if without + with == 0 {
            return Err(CommandError::rejected("There is nothing here to change"));
        }

        let wanted = want.unwrap_or(without > 0);
        let changing = carrying(!wanted);
        let count = if wanted { without } else { with };
        if count == 0 {
            return Err(CommandError::rejected("Already set"));
        }

        // Which folders to reload, answered now: a bulk unflag over Flagged empties its own
        // predicate, so a query after the commit would answer "no folders".
        let reloaded = match from {
            Some(from) => vec![from],
            None => MessageRepository::new(connection)
                .mailboxes_of_set(&changing)
                .await
                .map_err(store_failure)?,
        };

        let one: FlagSet = std::iter::once(flag.clone()).collect();
        let operation = if wanted {
            Operation::SetFlags { flags: one }
        } else {
            Operation::ClearFlags { flags: one }
        };
        let at = now();
        let transaction = connection.transaction().await.map_err(store_failure)?;
        // Enqueue before writing: the predicate is "the rows that disagree", and after the write
        // none do.
        let range = OperationQueueRepository::new(&transaction)
            .enqueue_set(account, &changing, &operation, at)
            .await
            .map_err(store_failure)?
            .ok_or_else(|| CommandError::rejected("Already set"))?;
        MessageRepository::new(&transaction)
            .set_flag_on_set(&changing, column, wanted)
            .await
            .map_err(store_failure)?;
        transaction.commit().await.map_err(store_failure)?;

        let target = MessageTarget::Batch {
            range,
            account,
            from,
        };
        let inverse = match flag {
            Flag::Seen => Command::ToggleRead {
                target,
                unread: Some(wanted),
            },
            _ => Command::Flag {
                target,
                flagged: Some(!wanted),
            },
        };
        Ok(Applied {
            rows: None,
            lasts: None,
            surfaced: false,
            account,
            kind: kind_for(&flag, wanted).await,
            messages: Vec::new(),
            count,
            removed: Vec::new(),
            arrived: None,
            reloaded,
            changed: Vec::new(),
            mailboxes_changed: false,
            contacts_changed: None,
            inverse: vec![inverse],
        })
    }

    async fn set_flag_rows(
        &self,
        connection: &mut Checkout,
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

        let at = now();
        // Rows, queue entries and thread recompute are `postio-storage`'s half (ADR 0028); this
        // side owns the transaction.
        let transaction = connection.transaction().await.map_err(store_failure)?;
        let siblings =
            postio_storage::actions::set_flag(&transaction, account, &touched, &flag, wanted, at)
                .await
                .map_err(store_failure)?;
        transaction.commit().await.map_err(store_failure)?;

        let changed: Vec<MessageId> = touched.iter().map(|message| message.id).collect();
        // Repaint wider than the change: a list row is its conversation's representative and
        // `pages_holding` matches row ids, so a sibling marked read left the row unread (#754). The
        // inverse and queue stay as wide as the touch.
        let mut repaint = changed.clone();
        repaint.extend(siblings);
        repaint.sort_unstable();
        repaint.dedup();
        let inverse = match flag {
            Flag::Seen => Command::ToggleRead {
                target: MessageTarget::Messages(changed.clone()),
                unread: Some(wanted),
            },
            _ => Command::Flag {
                target: MessageTarget::Messages(changed.clone()),
                flagged: Some(!wanted),
            },
        };
        Ok(Applied {
            rows: Some(conversations(touched.iter().copied())),
            lasts: None,
            surfaced: false,
            account,
            kind: kind_for(&flag, wanted).await,
            count: changed.len(),
            messages: changed,
            removed: Vec::new(),
            arrived: None,
            reloaded: Vec::new(),
            changed: repaint,
            mailboxes_changed: false,
            contacts_changed: None,
            inverse: vec![inverse],
        })
    }

    async fn announce(&self, applied: Applied, events: &EventSink, recording: Recording) {
        let account = applied.account;
        for (mailbox, messages) in &applied.removed {
            events.emit(Event::MessagesRemoved {
                account,
                mailbox: *mailbox,
                messages: messages.clone(),
            });
        }
        if let Some(mailbox) = applied.arrived {
            // An undone archive has to reappear now, not at the next sync.
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
        if applied.mailboxes_changed {
            events.emit(Event::MailboxesChanged { account });
        }
        if applied.surfaced {
            events.emit(Event::SurfacedChanged);
        }
        if let Some(names_changed) = applied.contacts_changed {
            events.emit(Event::ContactsChanged { names_changed });
        }
        if !recording.records().await {
            return;
        }
        // A bulk unit knows its size, not its members; `UndoEntry::new` would count the empty list
        // and the toast would say nothing happened.
        let entry = if applied.messages.len() == applied.count {
            UndoEntry::new(applied.kind, applied.messages, applied.inverse)
        } else {
            UndoEntry::bulk(applied.kind, applied.count, applied.inverse)
        };
        let entry = match applied.rows {
            Some(rows) if !entry.is_bulk() => entry.acting_on(rows),
            _ => entry,
        };
        let entry = match applied.lasts {
            Some(window) => entry.lasting(window),
            None => entry,
        };
        // Describe from the stack's entry: a burst coalesces into the unit already there.
        let description = self.stack().await.record(entry).description();
        events.emit(Event::ActionCompleted {
            description,
            undoable: true,
        });
    }

    async fn rows(
        &self,
        connection: &Checkout,
        ids: Vec<MessageId>,
    ) -> Result<Vec<Message>, CommandError> {
        let repository = MessageRepository::new(connection);
        let mut rows = Vec::with_capacity(ids.len());
        for id in ids {
            let Some(message) = repository.get(id).await.map_err(store_failure)? else {
                // The row can be gone by the time a key press reaches here.
                continue;
            };
            rows.push(message);
        }
        if rows.is_empty() {
            return Err(CommandError::rejected("Those messages are no longer here"));
        }
        Ok(rows)
    }

    /// What a verb will act on: rows, or a predicate. The whole-mailbox cases (`Everything`,
    /// `Batch`) never become rows.
    async fn aim(
        &self,
        connection: &Checkout,
        target: &MessageTarget,
    ) -> Result<Aim, CommandError> {
        let resolved = self
            .state
            .read(|app| app.resolve(target))
            .ok_or_else(|| CommandError::rejected("Nothing selected"))?;
        let units = match resolved {
            Resolved::Messages(ids) => return self.rows(connection, ids).await.map(Aim::Rows),
            Resolved::Thread(thread) => {
                let ids = thread_messages(connection, thread).await?;
                return self.rows(connection, ids).await.map(Aim::Rows);
            }
            Resolved::Threads(threads) => {
                let mut ids = Vec::new();
                for thread in threads {
                    ids.extend(thread_messages(connection, thread).await?);
                }
                return self.rows(connection, ids).await.map(Aim::Rows);
            }
            Resolved::Everything { scope, except } => match scope {
                ViewScope::Mailbox(mailbox) => vec![BulkUnit {
                    set: MessageSet::InMailbox { mailbox, except },
                    account: account_of(connection, mailbox).await?,
                    from: Some(mailbox),
                }],
                ViewScope::Flagged(account) => vec![BulkUnit {
                    set: MessageSet::Flagged { account, except },
                    account,
                    from: None,
                }],
                // Accounts come off the scope, not looked up again: one that reconnected since was
                // not in the selection the user saw (#811).
                // Each predicate is the account's inbox, which is what Unified shows (#1692).
                // Focus: held digest mail is not listed so not selected; a deselected row is a
                // conversation folded across accounts, so every copy stays out.
                ViewScope::Focus { accounts } => {
                    let (lone, except_threads) = focus_exceptions(connection, &except).await?;
                    let folders = MailboxRepository::new(connection);
                    let mut units = Vec::with_capacity(accounts.len());
                    for account in accounts {
                        let Some(inbox) = folders
                            .by_role(account, MailboxRole::Inbox)
                            .await
                            .map_err(store_failure)?
                        else {
                            continue;
                        };
                        units.push(BulkUnit {
                            set: MessageSet::InFocusInbox {
                                mailbox: inbox.id,
                                except: lone.clone(),
                                except_threads: except_threads.clone(),
                            },
                            account,
                            from: Some(inbox.id),
                        });
                    }
                    units
                }
                ViewScope::Unified { accounts } => {
                    let folders = MailboxRepository::new(connection);
                    let mut units = Vec::with_capacity(accounts.len());
                    for account in accounts {
                        let Some(inbox) = folders
                            .by_role(account, MailboxRole::Inbox)
                            .await
                            .map_err(store_failure)?
                        else {
                            continue;
                        };
                        units.push(BulkUnit {
                            set: MessageSet::InMailbox {
                                mailbox: inbox.id,
                                except: except.clone(),
                            },
                            account,
                            from: Some(inbox.id),
                        });
                    }
                    units
                }
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
        // An aggregate reaching no account must not silently do nothing.
        if units.is_empty() {
            return Err(CommandError::rejected("Nothing selected"));
        }
        Ok(Aim::Bulk(units))
    }

    /// Settle a draft whose send could not be confirmed: it did arrive (ADR 0021 Decision 3, #674).
    ///
    /// `None` means the draft the list is on.
    /// Point one of `account`'s roles at the folder at `path`, or back to automatic (ADR 0035).
    ///
    /// One transaction: the chosen row takes the role and every other row wearing it goes back to
    /// `Regular` (#964). Back to automatic clears the map and leaves the rows as they are. Mail is
    /// never moved (#164).
    async fn map_mailbox_role(
        &self,
        account: Option<AccountId>,
        role: Option<MailboxRole>,
        path: Option<&str>,
    ) -> Result<Applied, CommandError> {
        let Some(account) = account else {
            return Err(CommandError::rejected("Pick an account first"));
        };
        let Some(role) = role else {
            return Err(CommandError::rejected("Pick a role to map"));
        };
        if !role.is_special() || matches!(role, MailboxRole::Inbox | MailboxRole::Flagged) {
            return Err(CommandError::rejected(
                "That is not a role a folder can be mapped to",
            ));
        }

        let (mut connection, _permit) = self.connect().await?;
        let previous = MailboxRoleRepository::new(&connection)
            .for_account(account)
            .await
            .map_err(store_failure)?
            .into_iter()
            .find(|(mapped, _)| *mapped == role)
            .map(|(_, path)| path);
        if previous.as_deref() == path {
            return Err(CommandError::rejected("Already mapped there"));
        }

        let transaction = connection.transaction().await.map_err(store_failure)?;
        let roles = MailboxRoleRepository::new(&transaction);
        let mailboxes = MailboxRepository::new(&transaction);
        match path {
            Some(path) => {
                let chosen = mailboxes
                    .by_path(account, path)
                    .await
                    .map_err(store_failure)?
                    .filter(|mailbox| mailbox.selectable)
                    .ok_or_else(|| CommandError::rejected("This account has no such folder"))?;
                roles
                    .set(account, role, path)
                    .await
                    .map_err(store_failure)?;
                for mut mailbox in mailboxes
                    .list_for_account(account)
                    .await
                    .map_err(store_failure)?
                {
                    let wanted = if mailbox.id == chosen.id {
                        role
                    } else if mailbox.role == role {
                        MailboxRole::Regular
                    } else {
                        continue;
                    };
                    if mailbox.role != wanted {
                        mailbox.role = wanted;
                        mailboxes.update(&mailbox).await.map_err(store_failure)?;
                    }
                }
            }
            None => roles.clear(account, role).await.map_err(store_failure)?,
        }
        transaction.commit().await.map_err(store_failure)?;

        Ok(Applied {
            rows: None,
            lasts: None,
            surfaced: false,
            account,
            kind: UndoKind::MapMailboxRole,
            count: 1,
            messages: Vec::new(),
            removed: Vec::new(),
            arrived: None,
            reloaded: Vec::new(),
            changed: Vec::new(),
            mailboxes_changed: true,
            contacts_changed: None,
            inverse: vec![Command::MapMailboxRole {
                account: Some(account),
                role: Some(role),
                path: previous,
            }],
        })
    }

    /// A join, split, deletion or new person changes which addresses people own: tell the filing
    /// pass and Focus's tasks again (ADR 0048).
    async fn rules_follow_people(&self) {
        if self.focus.is_on()
            && let Err(reason) = self.focus.refresh(&self.database).await
        {
            tracing::warn!(%reason, "could not follow a change to who owns what: {reason}");
        }
    }

    /// Join people, or take a join back. The store's receipt is the inverse, so taking it back is
    /// exact.
    async fn join_contacts(&self, action: &ContactJoinAction) -> Result<Applied, CommandError> {
        let (connection, _permit) = self.connect().await?;
        let contacts = ContactRepository::new(&connection);
        match action {
            ContactJoinAction::Ask => Err(CommandError::rejected("Pick the people to join")),
            ContactJoinAction::Join {
                into,
                others,
                name,
                organization,
            } => {
                if others.is_empty() {
                    return Err(CommandError::rejected("Pick at least two people to join"));
                }
                if name.trim().is_empty() {
                    return Err(CommandError::rejected("A joined contact needs a name"));
                }
                let receipt = contacts
                    .join(*into, others, name.trim(), organization.as_deref())
                    .await
                    .map_err(contact_failure)?;
                Ok(contacts_applied(
                    UndoKind::JoinContacts,
                    true,
                    vec![Command::ContactJoin {
                        action: ContactJoinAction::Unjoin(receipt),
                    }],
                ))
            }
            ContactJoinAction::Unjoin(receipt) => {
                contacts.unjoin(receipt).await.map_err(contact_failure)?;
                Ok(contacts_applied(UndoKind::JoinContacts, true, Vec::new()))
            }
        }
    }

    async fn contact_address(
        &self,
        action: &ContactAddressAction,
    ) -> Result<Applied, CommandError> {
        let (connection, _permit) = self.connect().await?;
        let contacts = ContactRepository::new(&connection);
        match action {
            ContactAddressAction::Ask => Err(CommandError::rejected("Type the address to add")),
            ContactAddressAction::Add { person, address } => {
                // A deleted person's address moves without asking, and undo gives it back.
                let before = contacts
                    .by_address(&address.address)
                    .await
                    .map_err(store_failure)?
                    .map(|owner| (owner.id, owner.state));
                let added = contacts
                    .add_address(*person, address)
                    .await
                    .map_err(contact_failure)?;
                let back = match before {
                    Some((owner, state)) => {
                        let emptied = contacts
                            .get(owner)
                            .await
                            .map_err(store_failure)?
                            .filter(|now| now.addresses.is_empty())
                            .map(|_| state);
                        Command::ContactAddAddress {
                            action: ContactAddressAction::Put {
                                address: added,
                                to: Some(owner),
                                revive: emptied,
                            },
                        }
                    }
                    None => Command::ContactAddAddress {
                        action: ContactAddressAction::Put {
                            address: added,
                            to: None,
                            revive: None,
                        },
                    },
                };
                Ok(contacts_applied(UndoKind::ContactAddress, true, vec![back]))
            }
            ContactAddressAction::Put {
                address,
                to,
                revive,
            } => {
                let moved = match to {
                    Some(to) => contacts.move_address(*address, *to, *revive).await,
                    None => contacts.release_address(*address).await,
                }
                .map_err(contact_failure)?;
                let back = Command::ContactAddAddress {
                    action: ContactAddressAction::Put {
                        address: *address,
                        to: moved.previous,
                        revive: moved.emptied,
                    },
                };
                Ok(contacts_applied(UndoKind::ContactAddress, true, vec![back]))
            }
        }
    }

    async fn detach_address(&self, address: AddressId) -> Result<Applied, CommandError> {
        let (connection, _permit) = self.connect().await?;
        let contacts = ContactRepository::new(&connection);
        let original = contacts
            .owner_of_address(address)
            .await
            .map_err(contact_failure)?
            .ok_or_else(|| CommandError::rejected("That address belongs to nobody"))?;
        contacts
            .detach_address(address)
            .await
            .map_err(contact_failure)?;
        Ok(contacts_applied(
            UndoKind::DetachAddress,
            true,
            vec![Command::ContactAddAddress {
                action: ContactAddressAction::Put {
                    address,
                    to: Some(original),
                    revive: None,
                },
            }],
        ))
    }

    async fn set_preferred(
        &self,
        person: ContactId,
        address: AddressId,
    ) -> Result<Applied, CommandError> {
        let (connection, _permit) = self.connect().await?;
        let previous = ContactRepository::new(&connection)
            .set_preferred(person, address)
            .await
            .map_err(contact_failure)?;
        Ok(contacts_applied(
            UndoKind::PreferredAddress,
            false,
            vec![Command::ContactSetPreferred {
                person: Some(person),
                address: Some(previous),
            }],
        ))
    }

    /// Make a person the user typed in. Undo gives each address back to whoever had it.
    async fn new_contact(&self, action: &ContactNewAction) -> Result<Applied, CommandError> {
        let ContactNewAction::Create { name, addresses } = action else {
            return Err(CommandError::rejected("Type a name and an address"));
        };
        if addresses.is_empty() {
            return Err(CommandError::rejected("A contact needs an address"));
        }
        let (connection, _permit) = self.connect().await?;
        let contacts = ContactRepository::new(&connection);
        let mut before = Vec::with_capacity(addresses.len());
        for address in addresses {
            before.push(
                contacts
                    .by_address(&address.address)
                    .await
                    .map_err(store_failure)?
                    .map(|owner| (owner.id, owner.state)),
            );
        }
        let person = contacts
            .create(name.as_deref(), addresses)
            .await
            .map_err(contact_failure)?;
        let mut inverse = Vec::with_capacity(addresses.len());
        for (address, before) in addresses.iter().zip(before) {
            let Some(id) = contacts
                .get(person)
                .await
                .map_err(store_failure)?
                .and_then(|made| {
                    made.addresses
                        .iter()
                        .find(|a| a.address.address.eq_ignore_ascii_case(&address.address))
                        .map(|a| a.id)
                })
            else {
                continue;
            };
            let back = match before {
                Some((owner, state)) => {
                    let emptied = contacts
                        .get(owner)
                        .await
                        .map_err(store_failure)?
                        .filter(|now| now.addresses.is_empty())
                        .map(|_| state);
                    ContactAddressAction::Put {
                        address: id,
                        to: Some(owner),
                        revive: emptied,
                    }
                }
                None => ContactAddressAction::Put {
                    address: id,
                    to: None,
                    revive: None,
                },
            };
            inverse.push(Command::ContactAddAddress { action: back });
        }
        Ok(contacts_applied(UndoKind::CreateContact, true, inverse))
    }

    async fn edit_contact(&self, action: &ContactEditAction) -> Result<Applied, CommandError> {
        let (connection, _permit) = self.connect().await?;
        let contacts = ContactRepository::new(&connection);
        match action {
            ContactEditAction::Ask => Err(CommandError::rejected("Choose someone to edit")),
            ContactEditAction::Edit { person, edit } => {
                let prior = contacts
                    .edit(*person, edit)
                    .await
                    .map_err(contact_failure)?;
                Ok(contacts_applied(
                    UndoKind::EditContact,
                    true,
                    vec![Command::ContactEdit {
                        action: ContactEditAction::Put {
                            person: *person,
                            fields: prior,
                        },
                    }],
                ))
            }
            ContactEditAction::Switch {
                address,
                switch,
                on,
            } => {
                use postio_storage::repository::ContactSettingsRepository;
                let person = contacts
                    .by_address(address.trim())
                    .await
                    .map_err(contact_failure)?
                    .ok_or_else(|| CommandError::rejected("That is not someone in your mail"))?
                    .id;
                let settings = ContactSettingsRepository::new(&connection);
                let mut held = settings.get(person).await.map_err(contact_failure)?;
                let was = switch.is_on(&held);
                switch.set(&mut held, *on);
                settings.put(person, &held).await.map_err(contact_failure)?;
                Ok(contacts_applied(
                    UndoKind::ContactArrival,
                    false,
                    vec![Command::ContactEdit {
                        action: ContactEditAction::Switch {
                            address: address.clone(),
                            switch: *switch,
                            on: was,
                        },
                    }],
                ))
            }
            ContactEditAction::AddSender { address, details } => {
                let person = contacts
                    .by_address(address.trim())
                    .await
                    .map_err(contact_failure)?
                    .ok_or_else(|| CommandError::rejected("That is not someone in your mail"))?
                    .id;
                let prior = contacts
                    .add_sender(person, details)
                    .await
                    .map_err(contact_failure)?;
                Ok(contacts_applied(
                    UndoKind::CreateContact,
                    true,
                    vec![Command::ContactEdit {
                        action: ContactEditAction::RestoreSender { person, prior },
                    }],
                ))
            }
            ContactEditAction::RestoreSender { person, prior } => {
                contacts
                    .restore_sender(*person, prior)
                    .await
                    .map_err(contact_failure)?;
                Ok(contacts_applied(UndoKind::EditContact, true, Vec::new()))
            }
            ContactEditAction::Detail {
                address,
                kind,
                value,
                keep,
            } => {
                let person = contacts
                    .by_address(address.trim())
                    .await
                    .map_err(contact_failure)?
                    .ok_or_else(|| CommandError::rejected("That is not someone in your mail"))?
                    .id;
                let was = contacts
                    .details(person)
                    .await
                    .map_err(contact_failure)?
                    .into_iter()
                    .find(|d| d.kind == *kind && &d.value == value)
                    .map(|d| d.added);
                contacts
                    .set_detail(person, *kind, value, *keep)
                    .await
                    .map_err(contact_failure)?;
                Ok(contacts_applied(
                    UndoKind::EditContact,
                    false,
                    vec![Command::ContactEdit {
                        action: ContactEditAction::Detail {
                            address: address.clone(),
                            kind: *kind,
                            value: value.clone(),
                            keep: was,
                        },
                    }],
                ))
            }
            ContactEditAction::Ignore { address, ignored } => {
                use postio_storage::repository::ContactSettingsRepository;
                let person = contacts
                    .by_address(address.trim())
                    .await
                    .map_err(contact_failure)?
                    .ok_or_else(|| CommandError::rejected("That is not someone in your mail"))?
                    .id;
                let settings = ContactSettingsRepository::new(&connection);
                let mut held = settings.get(person).await.map_err(contact_failure)?;
                let was = std::mem::replace(&mut held.ignored, *ignored);
                settings.put(person, &held).await.map_err(contact_failure)?;
                Ok(contacts_applied(
                    UndoKind::ContactArrival,
                    false,
                    vec![Command::ContactEdit {
                        action: ContactEditAction::Ignore {
                            address: address.clone(),
                            ignored: was,
                        },
                    }],
                ))
            }
            ContactEditAction::Save { person, form } => {
                let saved = contacts
                    .save_form(*person, form)
                    .await
                    .map_err(contact_failure)?;
                let (kind, inverse) = match saved.prior {
                    Some(prior) => (
                        UndoKind::EditContact,
                        Command::ContactEdit {
                            action: ContactEditAction::Save {
                                person: Some(saved.person),
                                form: Box::new(prior),
                            },
                        },
                    ),
                    None => (
                        UndoKind::CreateContact,
                        Command::ContactDelete {
                            person: Some(saved.person),
                            group: None,
                        },
                    ),
                };
                Ok(contacts_applied(kind, true, vec![inverse]))
            }
            ContactEditAction::Put { person, fields } => {
                contacts
                    .put_fields(*person, fields)
                    .await
                    .map_err(contact_failure)?;
                Ok(contacts_applied(UndoKind::EditContact, true, Vec::new()))
            }
        }
    }

    async fn set_arrival(
        &self,
        address: &str,
        arrival: Option<postio_model::ContactArrival>,
    ) -> Result<Applied, CommandError> {
        use postio_storage::repository::ContactSettingsRepository;
        let (connection, _permit) = self.connect().await?;
        let person = ContactRepository::new(&connection)
            .by_address(address.trim())
            .await
            .map_err(contact_failure)?
            .ok_or_else(|| CommandError::rejected("That is not someone in your mail"))?
            .id;
        let settings = ContactSettingsRepository::new(&connection);
        let mut held = settings.get(person).await.map_err(contact_failure)?;
        let prior = std::mem::replace(&mut held.arrival, arrival);
        settings.put(person, &held).await.map_err(contact_failure)?;
        Ok(contacts_applied(
            UndoKind::ContactArrival,
            false,
            vec![Command::ContactSetArrival {
                address: address.to_owned(),
                arrival: prior,
            }],
        ))
    }

    async fn delete_contact(&self, person: ContactId) -> Result<Applied, CommandError> {
        let (connection, _permit) = self.connect().await?;
        let was = ContactRepository::new(&connection)
            .delete(person)
            .await
            .map_err(contact_failure)?;
        Ok(contacts_applied(
            UndoKind::DeleteContact,
            true,
            vec![Command::ContactRestore {
                person: Some(person),
                state: Some(was),
            }],
        ))
    }

    async fn restore_contact(
        &self,
        person: ContactId,
        state: ContactState,
    ) -> Result<Applied, CommandError> {
        let (connection, _permit) = self.connect().await?;
        ContactRepository::new(&connection)
            .restore(person, state)
            .await
            .map_err(contact_failure)?;
        Ok(contacts_applied(
            UndoKind::RestoreContact,
            true,
            vec![Command::ContactDelete {
                person: Some(person),
                group: None,
            }],
        ))
    }

    async fn new_group(&self, action: &ContactGroupNewAction) -> Result<Applied, CommandError> {
        let (connection, _permit) = self.connect().await?;
        let groups = ContactGroupRepository::new(&connection);
        let id = match action {
            ContactGroupNewAction::Ask => return Err(CommandError::rejected("Name the group")),
            ContactGroupNewAction::Create { name, members } => {
                let name = name.trim();
                if name.is_empty() {
                    return Err(CommandError::rejected("A group needs a name"));
                }
                let mut group = postio_model::ContactGroup::new(name, Utc::now());
                let id = groups.create(&mut group).await.map_err(contact_failure)?;
                groups
                    .add_members(id, members)
                    .await
                    .map_err(store_failure)?;
                id
            }
            ContactGroupNewAction::Restore { group, members } => {
                groups
                    .restore(group, members)
                    .await
                    .map_err(contact_failure)?;
                group.id
            }
        };
        Ok(contacts_applied(
            UndoKind::CreateGroup,
            false,
            vec![Command::ContactDelete {
                person: None,
                group: Some(id),
            }],
        ))
    }

    async fn rename_group(
        &self,
        group: ContactGroupId,
        name: &str,
    ) -> Result<Applied, CommandError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CommandError::rejected("A group needs a name"));
        }
        let (connection, _permit) = self.connect().await?;
        let groups = ContactGroupRepository::new(&connection);
        let before = groups
            .get(group)
            .await
            .map_err(store_failure)?
            .ok_or_else(|| CommandError::rejected("That group is no longer here"))?;
        groups
            .set_name(group, name)
            .await
            .map_err(contact_failure)?;
        Ok(contacts_applied(
            UndoKind::RenameGroup,
            false,
            vec![Command::ContactGroupRename {
                group: Some(group),
                name: Some(before.name),
            }],
        ))
    }

    async fn delete_group(&self, group: ContactGroupId) -> Result<Applied, CommandError> {
        let (connection, _permit) = self.connect().await?;
        let (group, members) = ContactGroupRepository::new(&connection)
            .remove(group)
            .await
            .map_err(store_failure)?
            .ok_or_else(|| CommandError::rejected("That group is no longer here"))?;
        Ok(contacts_applied(
            UndoKind::DeleteGroup,
            false,
            vec![Command::ContactGroupNew {
                action: ContactGroupNewAction::Restore { group, members },
            }],
        ))
    }

    async fn group_members(
        &self,
        group: ContactGroupId,
        people: &[ContactId],
        add: bool,
    ) -> Result<Applied, CommandError> {
        if people.is_empty() {
            return Err(CommandError::rejected("Choose someone first"));
        }
        let (connection, _permit) = self.connect().await?;
        let groups = ContactGroupRepository::new(&connection);
        let changed = if add {
            groups.add_members(group, people).await
        } else {
            groups.remove_members(group, people).await
        }
        .map_err(store_failure)?;
        let inverse = if add {
            Command::ContactGroupRemove {
                group: Some(group),
                people: changed,
            }
        } else {
            Command::ContactGroupAdd {
                group: Some(group),
                people: changed,
            }
        };
        Ok(contacts_applied(
            UndoKind::GroupMembers,
            false,
            vec![inverse],
        ))
    }

    /// Put a send that stopped back on the queue, through the path the composer uses so the draft
    /// state and `Send` operation commit together.
    async fn retry_send(&self, draft: Option<DraftId>) -> Result<Applied, CommandError> {
        let (connection, _permit) = self.connect().await?;
        let drafts = DraftRepository::new(&connection);
        let mut draft = self.stopped_send(&connection, &drafts, draft).await?;

        // `Editing` belongs to the composer, and `Queued`/`Sending` are on their way: retrying
        // those would duplicate the message (ADR 0021).
        if !matches!(draft.state, DraftState::Failed | DraftState::Unconfirmed) {
            return Err(CommandError::rejected(
                "Only a send that stopped can be sent again",
            ));
        }
        if !draft.has_recipients() {
            return Err(CommandError::rejected("That draft has no recipients"));
        }

        let account = draft.account_id;
        drafts
            .queue_send(&mut draft, now())
            .await
            .map_err(store_failure)?;
        Ok(Applied {
            rows: None,
            lasts: None,
            surfaced: false,
            account,
            kind: UndoKind::RetriedSend,
            mailboxes_changed: true,
            contacts_changed: None,
            count: 1,
            messages: Vec::new(),
            removed: Vec::new(),
            arrived: None,
            reloaded: drafts_folder(&connection, account).await?,
            changed: Vec::new(),
            // The way back is `CancelSend`, a command a person can reach.
            inverse: Vec::new(),
        })
    }

    /// Take a queued send back off the queue, leaving the draft editable. Refused once the drainer
    /// has it: ADR 0021's exactly-once never cancels a submission that may have reached the server.
    async fn cancel_send(&self, draft: Option<DraftId>) -> Result<Applied, CommandError> {
        use postio_storage::repository::CancelSendOutcome;

        let (mut connection, _permit) = self.connect().await?;
        let draft = {
            let drafts = DraftRepository::new(&connection);
            self.stopped_send(&connection, &drafts, draft).await?
        };
        let account = draft.account_id;

        // One transaction: an answer to an invitation is withdrawn with its send or not at all, or
        // the due timer would finalise an answer nothing sent.
        let transaction = connection.transaction().await.map_err(store_failure)?;
        match DraftRepository::new(&transaction)
            .cancel_send(draft.id, now())
            .await
            .map_err(store_failure)?
        {
            CancelSendOutcome::Cancelled => {
                let changed = self.withdraw_answer(&transaction, &draft).await?;
                let reloaded = drafts_folder(&transaction, account).await?;
                transaction.commit().await.map_err(store_failure)?;
                Ok(Applied {
                    rows: None,
                    lasts: None,
                    surfaced: false,
                    account,
                    kind: UndoKind::CancelledSend,
                    mailboxes_changed: true,
                    contacts_changed: None,
                    count: 1,
                    messages: Vec::new(),
                    removed: Vec::new(),
                    arrived: None,
                    reloaded,
                    changed,
                    inverse: Vec::new(),
                })
            }
            CancelSendOutcome::NotQueued => Err(CommandError::rejected(
                "That message is not waiting to be sent",
            )),
            // Not a failure of ours: the message may already have arrived, and claiming otherwise
            // breaks exactly-once.
            CancelSendOutcome::AlreadyInFlight => Err(CommandError::rejected(
                "That message is already being sent, so it is too late to stop it",
            )),
        }
    }

    /// The draft `RetrySend` and `CancelSend` act on: the one named, or the one behind the row in
    /// view.
    async fn stopped_send(
        &self,
        connection: &Checkout,
        drafts: &DraftRepository<'_>,
        draft: Option<DraftId>,
    ) -> Result<postio_model::Draft, CommandError> {
        match draft {
            Some(id) => drafts
                .get(id)
                .await
                .map_err(store_failure)?
                .ok_or_else(|| CommandError::rejected("That draft is no longer here")),
            None => {
                let rows = match self.aim(connection, &MessageTarget::Selection).await? {
                    Aim::Rows(rows) => rows,
                    Aim::Bulk { .. } => {
                        return Err(CommandError::rejected("Pick the message this is about"));
                    }
                };
                drafts
                    .by_message(rows[0].id)
                    .await
                    .map_err(store_failure)?
                    .ok_or_else(|| CommandError::rejected("That row is not a message being sent"))
            }
        }
    }

    async fn mark_sent(&self, draft: Option<DraftId>) -> Result<Applied, CommandError> {
        let (connection, _permit) = self.connect().await?;
        let drafts = DraftRepository::new(&connection);

        let draft = match draft {
            Some(id) => drafts
                .get(id)
                .await
                .map_err(store_failure)?
                .ok_or_else(|| CommandError::rejected("That draft is no longer here"))?,
            None => {
                let rows = match self.aim(&connection, &MessageTarget::Selection).await? {
                    Aim::Rows(rows) => rows,
                    Aim::Bulk { .. } => {
                        return Err(CommandError::rejected("Pick the message this is about"));
                    }
                };
                drafts
                    .by_message(rows[0].id)
                    .await
                    .map_err(store_failure)?
                    .ok_or_else(|| {
                        CommandError::rejected("That row is not a draft Postio is unsure about")
                    })?
            }
        };

        // Only `Unconfirmed`: saying "it arrived" about a draft still edited or already sent would
        // lose the draft, since `Sent` stops it being offered for editing.
        if draft.state != DraftState::Unconfirmed {
            return Err(CommandError::rejected(
                "Only a send Postio could not confirm can be marked as sent",
            ));
        }

        drafts
            .set_state(draft.id, DraftState::Sent)
            .await
            .map_err(store_failure)?;
        Ok(Applied {
            rows: None,
            lasts: None,
            surfaced: false,
            account: draft.account_id,
            kind: UndoKind::MarkedSent,
            count: 1,
            messages: Vec::new(),
            removed: Vec::new(),
            arrived: None,
            reloaded: drafts_folder(&connection, draft.account_id).await?,
            changed: Vec::new(),
            // No inverse: every `Command` needs a `CommandId` with a registry entry and key
            // binding, and this settles a claim about the world rather than changing it. A wrong
            // answer is corrected by sending again.
            mailboxes_changed: false,
            contacts_changed: None,
            inverse: Vec::new(),
        })
    }

    /// A connection to write a verb through, plus the right to write ahead of bulk background work
    /// (#425).
    ///
    /// SQLite's `busy_timeout` is a retry loop with no fairness: during a first sync an archive
    /// keystroke lost repeatedly to back-to-back backfill commits. The permit puts a person's write
    /// in front. See `postio_storage::WriteGate`.
    /// The gate is not re-entrant and the connection must be taken before the permit, or a
    /// permit-holder can wait on the pool for a connection a permit-waiter holds; so the pair is
    /// scoped to one call.
    async fn connect(&self) -> Result<(Checkout, WritePermit), CommandError> {
        let connection = self.database.connect().await.map_err(store_failure)?;
        let permit = self
            .database
            .write_gate()
            .acquire(WritePriority::Interactive)
            .await;
        Ok((connection, permit))
    }

    async fn stack(&self) -> std::sync::MutexGuard<'_, UndoStack> {
        // A panicking handler must not cost the application its history.
        self.undo
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// `account`'s Drafts folder, the one list a draft's send verb moves its row within (Drafts and the
/// Outbox are both over it). Empty when the account has none yet.
async fn drafts_folder(
    connection: &postio_storage::Connection,
    account: AccountId,
) -> Result<Vec<MailboxId>, CommandError> {
    Ok(MailboxRepository::new(connection)
        .by_role(account, postio_model::mailbox::MailboxRole::Drafts)
        .await
        .map_err(store_failure)?
        .map(|mailbox| mailbox.id)
        .into_iter()
        .collect())
}

async fn mailbox_for(
    connection: &Checkout,
    account: AccountId,
    to: Destination,
) -> Result<MailboxId, CommandError> {
    match to {
        Destination::Mailbox(id) => Ok(id),
        Destination::Role(role) => MailboxRepository::new(connection)
            .by_role(account, role)
            .await
            .map_err(store_failure)?
            .map(|mailbox| mailbox.id)
            .ok_or_else(|| {
                // Not every server has one; inventing a folder is not this command's decision.
                CommandError::rejected(format!(
                    "This account has no {} folder",
                    role.as_str().to_lowercase()
                ))
            }),
    }
}

async fn focus_exceptions(
    connection: &Checkout,
    except: &[MessageId],
) -> Result<(Vec<MessageId>, Vec<ThreadId>), CommandError> {
    let messages = MessageRepository::new(connection);
    let mut lone = Vec::new();
    let mut threads = Vec::new();
    for id in except {
        match messages.get(*id).await.map_err(store_failure)? {
            Some(Message {
                thread_id: Some(thread),
                ..
            }) => threads.push(thread),
            _ => lone.push(*id),
        }
    }
    if threads.is_empty() {
        return Ok((lone, threads));
    }
    let threads = ThreadRepository::new(connection)
        .with_folded_copies(&threads)
        .await
        .map_err(store_failure)?;
    Ok((lone, threads))
}

async fn thread_messages(
    connection: &Checkout,
    thread: ThreadId,
) -> Result<Vec<MessageId>, CommandError> {
    let rows = ThreadRepository::new(connection)
        .messages(thread, ThreadOrder::Oldest)
        .await
        .map_err(store_failure)?;
    if rows.is_empty() {
        return Err(CommandError::rejected("That thread is empty"));
    }
    Ok(rows.into_iter().map(|row| row.id).collect())
}

async fn kind_for(flag: &Flag, wanted: bool) -> UndoKind {
    match (flag, wanted) {
        (Flag::Seen, true) => UndoKind::MarkRead,
        (Flag::Seen, false) => UndoKind::MarkUnread,
        (_, true) => UndoKind::Flag,
        (_, false) => UndoKind::Unflag,
    }
}

/// A change to the address book, as a unit undo can hold. Recorded as a bulk unit of one so two in
/// a burst never coalesce: joins depend on one another and replaying both inverses together would
/// reorder them.
fn contacts_applied(kind: UndoKind, names_changed: bool, inverse: Vec<Command>) -> Applied {
    Applied {
        kind,
        account: AccountId::UNASSIGNED,
        messages: Vec::new(),
        count: 1,
        removed: Vec::new(),
        arrived: None,
        reloaded: Vec::new(),
        changed: Vec::new(),
        mailboxes_changed: false,
        contacts_changed: Some(names_changed),
        inverse,
        lasts: None,
        rows: None,
        surfaced: false,
    }
}

fn contact_failure(error: postio_storage::Error) -> CommandError {
    match error {
        postio_storage::Error::AddressOwned { .. } => {
            CommandError::rejected("That address belongs to someone else")
        }
        postio_storage::Error::NotFound { .. } => {
            CommandError::rejected("That contact is no longer here")
        }
        postio_storage::Error::ForbiddenTransition { reason, .. } => CommandError::rejected(reason),
        other => store_failure(other),
    }
}

/// A store that would not take the write. The detail goes to stderr, where it carries SQL rather
/// than anyone's mail.
fn store_failure(error: impl Into<postio_storage::Error>) -> CommandError {
    let error: postio_storage::Error = error.into();
    if error.is_busy() {
        // Another writer committed first; `Actions::run` retries (#1594).
        tracing::debug!(%error, "the local store was busy; the verb will run again");
        return CommandError::failed(STORE_BUSY);
    }
    tracing::error!(%error, "the local store refused a write: {error}");
    CommandError::failed("Could not save that change")
}

const BUSY_RETRIES: u64 = 5;

const STORE_BUSY: &str = "Postio was busy saving something else — try that again";

async fn account_of(connection: &Checkout, mailbox: MailboxId) -> Result<AccountId, CommandError> {
    Ok(MailboxRepository::new(connection)
        .get(mailbox)
        .await
        .map_err(store_failure)?
        .ok_or_else(|| CommandError::rejected("That folder is no longer here"))?
        .account_id)
}

#[cfg(test)]
pub async fn dispatcher(actions: Actions) -> postio_core::Dispatcher {
    wire(DispatcherBuilder::new(), actions).build()
}

/// Register every verb this module answers on `builder`, which is taken so other modules' verbs
/// (`Refresh`) can join the same bus.
pub fn wire(builder: DispatcherBuilder, actions: Actions) -> DispatcherBuilder {
    builder.on_each(WIRED.iter().copied(), move |invocation| {
        let actions = actions.clone();
        // Awaited, not spawned: app state and the undo stack see a total order. Anything slow
        // belongs on a spawned task with its own events.
        async move { actions.run(&invocation.command, &invocation.events()).await }
    })
}

/// Withdraw every queue operation still pending for `message`; left in the queue they would reach a
/// server, and an undo at `confirmed` must stop the forward removal.
async fn withdraw_pending(
    queue: &OperationQueueRepository<'_>,
    message: Option<MessageId>,
) -> Result<(), CommandError> {
    let Some(message) = message else {
        return Ok(());
    };
    while let Some(operation) = queue
        .pending_for(postio_model::OperationTarget::Message(message))
        .await
        .map_err(store_failure)?
    {
        queue.delete(operation.id).await.map_err(store_failure)?;
    }
    Ok(())
}

/// Undo a saga nothing has left the machine for: pure bookkeeping.
async fn cancel_one(
    messages: &MessageRepository<'_>,
    queue: &OperationQueueRepository<'_>,
    sagas: &postio_storage::repository::CrossAccountMoveRepository<'_>,
    saga: &postio_storage::repository::CrossAccountMove,
) -> Result<(), CommandError> {
    // Queue rows first, so a crash cannot leave an operation pointing at a deleted row.
    withdraw_pending(queue, saga.source_message).await?;
    withdraw_pending(queue, saga.target_message).await?;
    if let Some(copy) = saga.target_message {
        messages.delete(&[copy]).await.map_err(store_failure)?;
    }
    if let Some(source) = saga.source_message {
        messages
            .set_deleted_locally(&[source], false)
            .await
            .map_err(store_failure)?;
    }
    sagas
        .transition(saga.id, postio_storage::repository::MovePhase::Aborted)
        .await
        .map_err(store_failure)?;
    Ok(())
}

/// Start the inverse of a saga whose copy is proven: the same machine with source and target
/// exchanged (ADR 0026, #531).
///
/// The original hidden row is restored rather than a new one created; phase 1 is idempotent by
/// Message-ID, so a source copy still on the server is found, not duplicated.
async fn invert_one(
    messages: &MessageRepository<'_>,
    queue: &OperationQueueRepository<'_>,
    sagas: &postio_storage::repository::CrossAccountMoveRepository<'_>,
    saga: &postio_storage::repository::CrossAccountMove,
    at: chrono::DateTime<Utc>,
) -> Result<(), CommandError> {
    let (Some(copy), Some(original)) = (saga.target_message, saga.source_message) else {
        // A saga that lost either end cannot be inverted; left alone rather than guessed at.
        return Ok(());
    };
    let (Some(from_account), Some(from_mailbox)) = (saga.target_account, saga.target_mailbox)
    else {
        return Ok(());
    };
    let (Some(to_account), Some(to_mailbox)) = (saga.source_account, saga.source_mailbox) else {
        return Ok(());
    };

    let row = messages
        .get(copy)
        .await
        .map_err(store_failure)?
        .ok_or_else(|| CommandError::rejected("That message is no longer in the store"))?;

    let inverse = sagas
        .create(&postio_storage::repository::NewCrossAccountMove {
            source_message: copy,
            source_account: from_account,
            source_mailbox: from_mailbox,
            target_account: to_account,
            target_mailbox: to_mailbox,
            target_message: Some(original),
            raw_blob_id: row
                .raw_blob_id
                .as_ref()
                .map(|blob| blob.as_str().to_owned()),
            rfc_message_id: row.rfc_message_id.as_ref().map(|id| id.as_str().to_owned()),
        })
        .await
        .map_err(store_failure)?;

    queue
        .enqueue(
            to_account,
            postio_model::OperationTarget::Message(original),
            &Operation::CrossAccountCopy { saga: inverse },
            at,
        )
        .await
        .map_err(store_failure)?;
    // Enqueue while the copy still carries the identity phase 2 wrote, which `source_remote_id`
    // snapshots (#289).
    queue
        .enqueue(
            from_account,
            postio_model::OperationTarget::Message(copy),
            &Operation::CrossAccountRemove { saga: inverse },
            at,
        )
        .await
        .map_err(store_failure)?;

    messages
        .set_deleted_locally(&[original], false)
        .await
        .map_err(store_failure)?;
    messages
        .set_deleted_locally(&[copy], true)
        .await
        .map_err(store_failure)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    const OPEN_PHASES: &[postio_storage::repository::MovePhase] = &[
        postio_storage::repository::MovePhase::Copying,
        postio_storage::repository::MovePhase::Unconfirmed,
        postio_storage::repository::MovePhase::Confirmed,
    ];

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

    struct World {
        database: Store,
        account: Account,
        inbox: MailboxId,
        archive: MailboxId,
        trash: MailboxId,
        actions: Actions,
        state: SharedState,
        sink: EventSink,
        events: EventStream,
        quiet: EventSink,
    }

    struct Elsewhere {
        account: Account,
        inbox: MailboxId,
        archive: MailboxId,
    }

    async fn world() -> World {
        let database = test_support::memory().await;
        let (account, inbox, archive, trash) = {
            let connection = database.connect().await.expect("a connection");
            let (account, inbox) = test_support::account_with_inbox(&connection).await;
            let archive = test_support::mailbox(&connection, &account, "Archive")
                .await
                .id;
            let trash = test_support::mailbox(&connection, &account, "Trash")
                .await
                .id;
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
        async fn message(&self, mailbox: MailboxId, flags: &[Flag]) -> MessageId {
            let connection = self.database.connect().await.expect("a connection");
            let mut message = Message::new(self.account.id, mailbox, Utc::now());
            for flag in flags {
                message.flags.insert(flag.clone());
            }
            MessageRepository::new(&connection)
                .create(&mut message)
                .await
                .expect("a message")
        }

        async fn looking_at(
            &self,
            mailbox: MailboxId,
            selected: &[MessageId],
            focus: Option<MessageId>,
        ) {
            self.state
                .update(&self.quiet, |app: &mut AppState| app.open_mailbox(mailbox));
            self.state.update(&self.quiet, |app: &mut AppState| {
                app.select(selected.to_vec(), focus)
            });
        }

        async fn everything_in(&self, mailbox: MailboxId) {
            self.state
                .update(&self.quiet, |app: &mut AppState| app.open_mailbox(mailbox));
            self.state
                .update(&self.quiet, |app: &mut AppState| app.select_all());
        }

        async fn second_account(&self) -> Elsewhere {
            let connection = self.database.connect().await.expect("a connection");
            let mut account = Account::new(
                "Away",
                postio_model::EmailAddress::new(Some("Away User"), "away@example.com"),
            );
            account.incoming.host = "imap.example.com".to_owned();
            account.outgoing.host = "smtp.example.com".to_owned();
            AccountRepository::new(&connection)
                .create(&mut account)
                .await
                .expect("a second account");
            let inbox = test_support::mailbox(&connection, &account, "INBOX")
                .await
                .id;
            let archive = test_support::mailbox(&connection, &account, "Archive")
                .await
                .id;
            Elsewhere {
                account,
                inbox,
                archive,
            }
        }

        async fn message_for(
            &self,
            account: &Account,
            mailbox: MailboxId,
            flags: &[Flag],
        ) -> MessageId {
            let connection = self.database.connect().await.expect("a connection");
            let mut message = Message::new(account.id, mailbox, Utc::now());
            for flag in flags {
                message.flags.insert(flag.clone());
            }
            MessageRepository::new(&connection)
                .create(&mut message)
                .await
                .expect("a message")
        }

        async fn everything_unified(&self, accounts: &[AccountId]) {
            let accounts = accounts.to_vec();
            self.state.update(&self.quiet, |app: &mut AppState| {
                app.open_view(ViewScope::Unified {
                    accounts: accounts.clone(),
                })
            });
            self.state
                .update(&self.quiet, |app: &mut AppState| app.select_all());
        }

        async fn everything_flagged(&self) {
            let account = self.account.id;
            self.state
                .update(&self.quiet, |app: &mut AppState| app.open_flagged(account));
            self.state
                .update(&self.quiet, |app: &mut AppState| app.select_all());
        }

        async fn flag(&self, message: MessageId, flag: Flag) {
            let connection = self.database.connect().await.expect("a connection");
            let repository = MessageRepository::new(&connection);
            let mut flags = repository
                .get(message)
                .await
                .expect("a read")
                .expect("the message is still there")
                .flags;
            flags.insert(flag);
            repository
                .set_flags(message, &flags, FlagSource::Server)
                .await
                .expect("dress the message");
        }

        async fn count_in(&self, mailbox: MailboxId) -> u32 {
            let connection = self.database.connect().await.expect("a connection");
            MessageRepository::new(&connection)
                .count_set(&MessageSet::in_mailbox(mailbox))
                .await
                .expect("a count")
        }

        async fn run(&self, command: Command) -> Result<(), CommandError> {
            self.actions.run(&command, &self.sink).await
        }

        async fn drained(&self) -> Vec<Event> {
            let mut events = Vec::new();
            while let Some(event) = self.events.try_next() {
                events.push(event);
            }
            events
        }

        async fn two_sent_folders(&self) -> (MailboxId, MailboxId) {
            let connection = self.database.connect().await.expect("a connection");
            let sent = test_support::mailbox(&connection, &self.account, "Sent").await;
            let sent_messages =
                test_support::mailbox(&connection, &self.account, "Sent Messages").await;
            (sent.id, sent_messages.id)
        }

        async fn wearing(&self, role: postio_model::MailboxRole) -> Vec<MailboxId> {
            let connection = self.database.connect().await.expect("a connection");
            MailboxRepository::new(&connection)
                .list_for_account(self.account.id)
                .await
                .expect("a read")
                .into_iter()
                .filter(|mailbox| mailbox.role == role && mailbox.selectable)
                .map(|mailbox| mailbox.id)
                .collect()
        }

        async fn mapped(&self, role: postio_model::MailboxRole) -> Option<String> {
            let connection = self.database.connect().await.expect("a connection");
            postio_storage::repository::MailboxRoleRepository::new(&connection)
                .for_account(self.account.id)
                .await
                .expect("a read")
                .into_iter()
                .find(|(mapped, _)| *mapped == role)
                .map(|(_, path)| path)
        }

        async fn map_sent_to(&self, path: Option<&str>) -> Result<(), CommandError> {
            self.run(Command::MapMailboxRole {
                account: Some(self.account.id),
                role: Some(postio_model::MailboxRole::Sent),
                path: path.map(str::to_owned),
            })
            .await
        }

        async fn mailbox_of(&self, message: MessageId) -> MailboxId {
            let connection = self.database.connect().await.expect("a connection");
            MessageRepository::new(&connection)
                .get(message)
                .await
                .expect("a read")
                .expect("the message is still there")
                .mailbox_id
        }

        async fn label(&self, name: &str) -> postio_model::LabelId {
            let connection = self.database.connect().await.expect("a connection");
            let mut label = postio_model::Label::new(self.account.id, name);
            postio_storage::repository::LabelRepository::new(&connection)
                .create(&mut label)
                .await
                .expect("create a label")
        }

        async fn labels_of(&self, message: MessageId) -> Vec<postio_model::LabelId> {
            let connection = self.database.connect().await.expect("a connection");
            postio_storage::repository::LabelRepository::new(&connection)
                .for_message(message)
                .await
                .expect("a read")
        }

        async fn flags_of(&self, message: MessageId) -> postio_model::FlagSet {
            let connection = self.database.connect().await.expect("a connection");
            MessageRepository::new(&connection)
                .get(message)
                .await
                .expect("a read")
                .expect("the message is still there")
                .flags
        }

        async fn snoozed_until_of(&self, message: MessageId) -> Option<chrono::DateTime<Utc>> {
            let connection = self.database.connect().await.expect("a connection");
            MessageRepository::new(&connection)
                .get(message)
                .await
                .expect("a read")
                .expect("the message is still there")
                .snoozed_until
        }

        async fn queued(&self) -> Vec<(OperationTarget, Operation)> {
            let connection = self.database.connect().await.expect("a connection");
            OperationQueueRepository::new(&connection)
                .pending(self.account.id, Utc::now())
                .await
                .expect("a read")
                .into_iter()
                .map(|row| (row.target, row.operation))
                .collect()
        }
    }

    async fn completion(events: &[Event]) -> Option<(&str, bool)> {
        events.iter().find_map(|event| match event {
            Event::ActionCompleted {
                description,
                undoable,
            } => Some((description.as_str(), *undoable)),
            _ => None,
        })
    }

    fn undone(events: &[Event]) -> Option<String> {
        events.iter().find_map(|event| match event {
            Event::UndoPerformed { description } => Some(description.clone()),
            _ => None,
        })
    }

    /// ADR 0005 Q4 (#182): each message of a unified selection must land in its own account's
    /// Archive, or the queued `Move` hits the wrong server.
    #[tokio::test]
    async fn archiving_across_accounts_files_each_message_in_its_own_archive() {
        let world = world().await;

        let (other_account, other_inbox, other_archive) = {
            let connection = world.database.connect().await.expect("a connection");
            let mut account = postio_model::Account::new(
                "Other",
                postio_model::EmailAddress::new(Some("Other"), "other@example.net"),
            );
            AccountRepository::new(&connection)
                .create(&mut account)
                .await
                .expect("the second account");
            let inbox = test_support::mailbox(&connection, &account, "INBOX").await;
            let archive = test_support::mailbox(&connection, &account, "Archive").await;
            (account, inbox.id, archive.id)
        };

        let mine = world.message(world.inbox, &[]).await;
        let theirs = {
            let connection = world.database.connect().await.expect("a connection");
            let mut message = Message::new(other_account.id, other_inbox, Utc::now());
            MessageRepository::new(&connection)
                .create(&mut message)
                .await
                .expect("a message in the other account")
        };

        world
            .looking_at(world.inbox, &[mine, theirs], Some(mine))
            .await;

        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .await
            .expect("archive");

        assert_eq!(
            world.mailbox_of(mine).await,
            world.archive,
            "the first account's message goes to its own Archive"
        );
        assert_eq!(
            world.mailbox_of(theirs).await,
            other_archive,
            "and the second account's message goes to *its* Archive — not to \
             whichever account happened to be first in the selection"
        );

        // Read against the other account's queue; `world.queued()` sees only the first's.
        let theirs_queued: Vec<(OperationTarget, Operation)> = {
            let connection = world.database.connect().await.expect("a connection");
            OperationQueueRepository::new(&connection)
                .pending(other_account.id, Utc::now())
                .await
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

    #[tokio::test]
    async fn archiving_the_cursor_row_files_it_and_queues_the_move() {
        let world = world().await;
        let message = world.message(world.inbox, &[]).await;
        world.looking_at(world.inbox, &[], Some(message)).await;

        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .await
            .expect("archive");

        assert_eq!(world.mailbox_of(message).await, world.archive);
        assert_eq!(
            world.queued().await,
            vec![(
                OperationTarget::Message(message),
                Operation::Move {
                    from: world.inbox,
                    to: world.archive
                }
            )]
        );

        let events = world.drained().await;
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
        assert_eq!(
            completion(&events).await,
            Some(("Archived 1 message", true))
        );
    }

    #[tokio::test]
    async fn archiving_a_selection_counts_what_was_chosen() {
        let world = world().await;
        let mut messages: Vec<MessageId> = Vec::new();
        for _ in 0..2 {
            messages.push(world.message(world.inbox, &[]).await);
        }
        messages.push(world.message(world.archive, &[]).await);
        world
            .looking_at(world.inbox, &messages, Some(messages[0]))
            .await;
        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .await
            .expect("archive");
        assert_eq!(
            completion(&world.drained().await).await,
            Some(("Archived 3 messages", true)),
        );
    }

    #[tokio::test]
    async fn a_multi_select_archive_is_one_coalesced_undo_entry() {
        let world = world().await;
        let mut messages: Vec<MessageId> = Vec::new();
        for _ in 0..3 {
            messages.push(world.message(world.inbox, &[]).await);
        }
        world
            .looking_at(world.inbox, &messages, Some(messages[0]))
            .await;

        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .await
            .expect("archive");
        assert_eq!(
            completion(&world.drained().await).await,
            Some(("Archived 3 messages", true)),
            "one gesture, one entry, one sentence"
        );

        world.run(Command::Undo).await.expect("undo");

        for message in &messages {
            assert_eq!(
                world.mailbox_of(*message).await,
                world.inbox,
                "one `u` takes all three back"
            );
        }
    }

    #[tokio::test]
    async fn the_undo_top_is_named_without_taking_it_back() {
        let world = world().await;
        assert_eq!(world.actions.peek_description(), None, "nothing yet");
        let mut messages: Vec<MessageId> = Vec::new();
        for _ in 0..3 {
            messages.push(world.message(world.inbox, &[]).await);
        }
        world
            .looking_at(world.inbox, &messages, Some(messages[0]))
            .await;
        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .await
            .expect("archive");
        assert_eq!(
            world.actions.peek_description().as_deref(),
            Some("Archived 3 messages")
        );
        assert_eq!(
            world.actions.peek_description().as_deref(),
            Some("Archived 3 messages"),
            "reading it twice takes nothing back"
        );
        world.run(Command::Undo).await.expect("undo");
        assert_eq!(
            world.actions.peek_description(),
            None,
            "once undone there is nothing left to name"
        );
    }

    #[tokio::test]
    async fn undo_puts_a_message_back_and_queues_the_way_back() {
        let world = world().await;
        let message = world.message(world.inbox, &[]).await;
        world.looking_at(world.inbox, &[], Some(message)).await;
        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .await
            .expect("archive");
        let _ = world.drained().await;

        world.run(Command::Undo).await.expect("undo");

        assert_eq!(world.mailbox_of(message).await, world.inbox);
        assert_eq!(
            world.queued().await.last(),
            Some(&(
                OperationTarget::Message(message),
                Operation::Move {
                    from: world.archive,
                    to: world.inbox
                }
            )),
            "the server has to be told the way back too"
        );
        assert!(world.drained().await.contains(&Event::UndoPerformed {
            description: "Archived 1 message, undone".into(),
        }));
    }

    #[tokio::test]
    async fn archiving_a_thread_takes_every_message_in_it() {
        let world = world().await;
        let first = world.message(world.inbox, &[]).await;
        let second = world.message(world.inbox, &[]).await;
        let thread = {
            let connection = world.database.connect().await.expect("a connection");
            let threads = ThreadRepository::new(&connection);
            let mut thread = postio_model::Thread::new(world.account.id);
            threads.create(&mut thread).await.expect("a thread");
            for message in [first, second] {
                threads
                    .add_message(thread.id, message)
                    .await
                    .expect("membership");
            }
            thread.id
        };
        world.looking_at(world.inbox, &[], Some(first)).await;

        world
            .run(Command::Archive {
                target: MessageTarget::Thread(thread),
            })
            .await
            .expect("archive the thread");

        assert_eq!(world.mailbox_of(first).await, world.archive);
        assert_eq!(
            world.mailbox_of(second).await,
            world.archive,
            "the message the cursor was not on has to move too"
        );
        assert_eq!(world.queued().await.len(), 2);
        assert_eq!(
            completion(&world.drained().await).await,
            Some(("Archived 1 message", true)),
            "one conversation was archived, and the toast counts what was chosen"
        );

        world.run(Command::Undo).await.expect("undo");
        for message in [first, second] {
            assert_eq!(
                world.mailbox_of(message).await,
                world.inbox,
                "the one `u` brings the whole conversation back"
            );
        }
        assert_eq!(
            undone(&world.drained().await),
            Some("Archived 1 message, undone".to_owned()),
            "and says it was undone, not that it was archived"
        );
    }

    #[tokio::test]
    async fn deleting_files_to_trash_as_a_delete_rather_than_a_move() {
        let world = world().await;
        let message = world.message(world.inbox, &[]).await;
        world.looking_at(world.inbox, &[], Some(message)).await;

        world
            .run(Command::Delete {
                target: MessageTarget::Selection,
            })
            .await
            .expect("delete");

        assert_eq!(world.mailbox_of(message).await, world.trash);
        assert_eq!(
            world.queued().await,
            vec![(
                OperationTarget::Message(message),
                Operation::Delete {
                    from: world.inbox,
                    trash: world.trash
                }
            )]
        );
        assert_eq!(
            completion(&world.drained().await).await,
            Some(("Deleted 1 message", true))
        );
    }

    #[tokio::test]
    async fn flagging_a_mixed_selection_makes_it_agree() {
        let world = world().await;
        let flagged = world.message(world.inbox, &[Flag::Flagged]).await;
        let plain = world.message(world.inbox, &[]).await;
        world
            .looking_at(world.inbox, &[flagged, plain], Some(flagged))
            .await;

        world
            .run(Command::Flag {
                target: MessageTarget::Selection,
                flagged: None,
            })
            .await
            .expect("flag");

        assert!(world.flags_of(flagged).await.is_flagged());
        assert!(world.flags_of(plain).await.is_flagged());
        assert_eq!(
            world.queued().await.len(),
            1,
            "only the row that actually changed is worth telling the server about"
        );
        assert_eq!(
            completion(&world.drained().await).await,
            Some(("Flagged 1 message", true))
        );
    }

    #[tokio::test]
    async fn undoing_a_flag_unflags_exactly_what_it_flagged() {
        let world = world().await;
        let message = world.message(world.inbox, &[]).await;
        world.looking_at(world.inbox, &[], Some(message)).await;
        world
            .run(Command::Flag {
                target: MessageTarget::Selection,
                flagged: None,
            })
            .await
            .expect("flag");
        let _ = world.drained().await;

        let before = postio_storage::test_support::gate_log::interactive_requested_here();
        world.run(Command::Undo).await.expect("undo");
        // #1607: the saga probe took a second permit, so a flag undo waited for the writer twice.
        assert_eq!(
            postio_storage::test_support::gate_log::interactive_requested_here() - before,
            1,
            "a flag undo asked for the writer more than once"
        );

        assert!(!world.flags_of(message).await.is_flagged());
        assert!(matches!(
            world.queued().await.last(),
            Some((_, Operation::ClearFlags { .. }))
        ));
    }

    #[tokio::test]
    async fn marking_unread_clears_seen() {
        let world = world().await;
        let message = world.message(world.inbox, &[Flag::Seen]).await;
        world.looking_at(world.inbox, &[], Some(message)).await;

        world
            .run(Command::ToggleRead {
                target: MessageTarget::Selection,
                unread: None,
            })
            .await
            .expect("mark unread");

        assert!(world.flags_of(message).await.is_unread());
        assert!(matches!(
            world.queued().await.first(),
            Some((_, Operation::ClearFlags { .. }))
        ));
        assert_eq!(
            completion(&world.drained().await).await,
            Some(("Marked 1 message as unread", true))
        );
    }

    #[tokio::test]
    async fn snoozing_the_selection_hides_it_and_tells_the_list_it_left() {
        let world = world().await;
        let message = world.message(world.inbox, &[]).await;
        world.looking_at(world.inbox, &[], Some(message)).await;

        world
            .run(Command::Snooze {
                target: MessageTarget::Selection,
                until: None,
            })
            .await
            .expect("snooze");

        assert!(
            world
                .snoozed_until_of(message)
                .await
                .is_some_and(|at| at > Utc::now()),
            "the row must carry a snooze somewhere in the future"
        );
        let events = world.drained().await;
        assert!(events.contains(&Event::MessagesRemoved {
            account: world.account.id,
            mailbox: world.inbox,
            messages: vec![message],
        }));
        assert_eq!(
            completion(&events).await,
            Some(("Snoozed 1 message", true)),
            "an undoable action, the same as every other verb here"
        );
    }

    #[tokio::test]
    async fn undoing_a_snooze_unsnoozes_exactly_what_it_snoozed() {
        let world = world().await;
        let message = world.message(world.inbox, &[]).await;
        world.looking_at(world.inbox, &[], Some(message)).await;
        world
            .run(Command::Snooze {
                target: MessageTarget::Selection,
                until: None,
            })
            .await
            .expect("snooze");
        let _ = world.drained().await;

        world.run(Command::Undo).await.expect("undo");

        assert_eq!(
            world.snoozed_until_of(message).await,
            None,
            "undo is `Unsnooze`, not a second snooze"
        );
    }

    #[tokio::test]
    async fn unsnoozing_clears_it_and_reloads_its_mailbox_rather_than_naming_the_row() {
        let world = world().await;
        let message = world.message(world.inbox, &[]).await;
        world.looking_at(world.inbox, &[], Some(message)).await;
        world
            .run(Command::Snooze {
                target: MessageTarget::Selection,
                until: None,
            })
            .await
            .expect("snooze");
        let _ = world.drained().await;

        world
            .run(Command::Unsnooze {
                target: MessageTarget::Selection,
            })
            .await
            .expect("unsnooze");

        assert_eq!(world.snoozed_until_of(message).await, None);
        let events = world.drained().await;
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

    #[tokio::test]
    async fn snoozing_a_whole_mailbox_is_refused() {
        let world = world().await;
        world.message(world.inbox, &[]).await;
        world.everything_in(world.inbox).await;

        let error = world
            .run(Command::Snooze {
                target: MessageTarget::Selection,
                until: None,
            })
            .await
            .expect_err("a whole-mailbox snooze is not offered");

        assert!(matches!(error, CommandError::Rejected(_)));
    }

    #[tokio::test]
    async fn marking_an_unconfirmed_send_as_sent_settles_the_draft() {
        let world = world().await;
        let id = {
            let connection = world.database.connect().await.expect("a connection");
            let drafts = postio_storage::repository::DraftRepository::new(&connection);
            let mut draft = postio_model::Draft::new(world.account.id);
            draft.to = vec![postio_model::EmailAddress::new(
                None::<String>,
                "grace@example.net",
            )];
            let id = drafts.save(&mut draft).await.expect("save");
            drafts
                .set_state(id, DraftState::Unconfirmed)
                .await
                .expect("the state an interrupted submission leaves");
            id
        };

        world
            .run(Command::MarkSent { draft: Some(id) })
            .await
            .expect("marking it sent applies");

        let connection = world.database.connect().await.expect("a connection");
        assert_eq!(
            postio_storage::repository::DraftRepository::new(&connection)
                .get(id)
                .await
                .expect("read")
                .expect("still there")
                .state,
            DraftState::Sent,
            "the question is settled, so it stops being offered as unsent"
        );
    }

    #[tokio::test]
    async fn send_again_works_on_the_row_in_view_without_naming_a_draft() {
        let world = world().await;
        let (id, ()) = {
            let connection = world.database.connect().await.expect("a connection");
            let drafts = postio_storage::repository::DraftRepository::new(&connection);
            let mut draft = postio_model::Draft::new(world.account.id);
            draft.to = vec![postio_model::EmailAddress::new(
                None::<String>,
                "quinn@example.net",
            )];
            let id = drafts.save(&mut draft).await.expect("save");
            drafts
                .set_state(id, DraftState::Failed)
                .await
                .expect("a send that stopped");
            (id, ())
        };

        let message = world.message(world.inbox, &[]).await;
        {
            let connection = world.database.connect().await.expect("a connection");
            postio_storage::repository::DraftRepository::new(&connection)
                .set_synced_message(id, message)
                .await
                .expect("link the draft to the row that lists it");
        }

        world
            .looking_at(world.inbox, &[message], Some(message))
            .await;

        world
            .run(Command::RetrySend { draft: None })
            .await
            .expect("the button names no draft and must still retry the send");

        let connection = world.database.connect().await.expect("a connection");
        let drafts = postio_storage::repository::DraftRepository::new(&connection);
        assert_eq!(
            drafts.get(id).await.expect("read").expect("there").state,
            DraftState::Queued,
            "the send is back on the queue"
        );
    }

    #[tokio::test]
    async fn a_failed_send_can_be_put_back_on_the_queue() {
        let world = world().await;
        let id = {
            let connection = world.database.connect().await.expect("a connection");
            let drafts = postio_storage::repository::DraftRepository::new(&connection);
            let mut draft = postio_model::Draft::new(world.account.id);
            draft.to = vec![postio_model::EmailAddress::new(
                None::<String>,
                "quinn@example.net",
            )];
            let id = drafts.save(&mut draft).await.expect("save");
            drafts
                .set_state(id, DraftState::Failed)
                .await
                .expect("a send that stopped");
            id
        };

        world
            .run(Command::RetrySend { draft: Some(id) })
            .await
            .expect("retrying applies");

        let connection = world.database.connect().await.expect("a connection");
        let drafts = postio_storage::repository::DraftRepository::new(&connection);
        assert_eq!(
            drafts
                .get(id)
                .await
                .expect("read")
                .expect("still there")
                .state,
            DraftState::Queued,
            "a retried send is on its way again, which is what the Outbox lists"
        );
        let sends = world
            .queued()
            .await
            .into_iter()
            .filter(|(_, operation)| matches!(operation, Operation::Send { draft } if *draft == id))
            .count();
        assert_eq!(sends, 1, "retrying enqueued nothing, so nothing will send");
    }

    #[tokio::test]
    async fn a_draft_being_written_is_not_something_to_retry() {
        let world = world().await;
        let id = {
            let connection = world.database.connect().await.expect("a connection");
            let drafts = postio_storage::repository::DraftRepository::new(&connection);
            let mut draft = postio_model::Draft::new(world.account.id);
            drafts.save(&mut draft).await.expect("save")
        };

        let error = world
            .run(Command::RetrySend { draft: Some(id) })
            .await
            .expect_err("an editable draft is not a stopped send");
        assert!(matches!(error, CommandError::Rejected(_)), "{error:?}");
    }

    #[tokio::test]
    async fn a_queued_send_can_be_taken_back_off_the_queue() {
        let world = world().await;
        let id = {
            let connection = world.database.connect().await.expect("a connection");
            let drafts = postio_storage::repository::DraftRepository::new(&connection);
            let mut draft = postio_model::Draft::new(world.account.id);
            draft.to = vec![postio_model::EmailAddress::new(
                None::<String>,
                "quinn@example.net",
            )];
            drafts.save(&mut draft).await.expect("save");
            drafts
                .queue_send(&mut draft, chrono::Utc::now())
                .await
                .expect("queue it");
            draft.id
        };

        world
            .run(Command::CancelSend { draft: Some(id) })
            .await
            .expect("cancelling applies");

        let connection = world.database.connect().await.expect("a connection");
        let drafts = postio_storage::repository::DraftRepository::new(&connection);
        assert_eq!(
            drafts
                .get(id)
                .await
                .expect("read")
                .expect("still there")
                .state,
            DraftState::Editing,
            "a cancelled send is editable again, not lost"
        );
        let sends = world
            .queued()
            .await
            .into_iter()
            .filter(|(_, operation)| matches!(operation, Operation::Send { draft } if *draft == id))
            .count();
        assert_eq!(
            sends, 0,
            "the operation is still queued, so it will still send"
        );
    }

    #[tokio::test]
    async fn only_an_unconfirmed_send_can_be_marked_as_sent() {
        let world = world().await;
        let id = {
            let connection = world.database.connect().await.expect("a connection");
            let drafts = postio_storage::repository::DraftRepository::new(&connection);
            let mut draft = postio_model::Draft::new(world.account.id);
            drafts.save(&mut draft).await.expect("save")
        };

        let error = world
            .run(Command::MarkSent { draft: Some(id) })
            .await
            .expect_err("an editable draft is not something to mark sent");

        assert!(matches!(error, CommandError::Rejected(_)), "{error:?}");
        assert_eq!(
            postio_storage::repository::DraftRepository::new(
                &world.database.connect().await.expect("a connection")
            )
            .get(id)
            .await
            .expect("read")
            .expect("still there")
            .state,
            DraftState::Editing,
            "and it is left exactly as it was"
        );
    }

    #[tokio::test]
    async fn settling_a_send_says_the_drafts_list_changed() {
        // A list hears that its rows moved only from a list event; without one it kept drawing the
        // moved row.
        let world = world().await;
        let drafts_folder = {
            let connection = world.database.connect().await.expect("a connection");
            test_support::mailbox(&connection, &world.account, "Drafts")
                .await
                .id
        };
        let draft = |state: DraftState| {
            let world = &world;
            async move {
                let connection = world.database.connect().await.expect("a connection");
                let drafts = postio_storage::repository::DraftRepository::new(&connection);
                let mut draft = postio_model::Draft::new(world.account.id);
                draft.to = vec![postio_model::EmailAddress::new(
                    None::<String>,
                    "quinn@example.net",
                )];
                let id = drafts.save(&mut draft).await.expect("save");
                if state == DraftState::Queued {
                    drafts
                        .queue_send(&mut draft, chrono::Utc::now())
                        .await
                        .expect("queue it");
                } else {
                    drafts.set_state(id, state).await.expect("the state");
                }
                id
            }
        };
        let listed = Event::MessageListChanged {
            account: world.account.id,
            mailbox: drafts_folder,
        };
        for (state, command) in [
            (DraftState::Failed, CommandId::RetrySend),
            (DraftState::Queued, CommandId::CancelSend),
            (DraftState::Unconfirmed, CommandId::MarkSent),
        ] {
            let id = draft(state).await;
            world.drained().await;
            let command = match command {
                CommandId::RetrySend => Command::RetrySend { draft: Some(id) },
                CommandId::CancelSend => Command::CancelSend { draft: Some(id) },
                _ => Command::MarkSent { draft: Some(id) },
            };
            world.run(command.clone()).await.expect("it applies");
            assert!(
                world.drained().await.contains(&listed),
                "{command:?} moved a draft and said nothing to the lists over Drafts"
            );
        }
    }

    #[tokio::test]
    async fn a_dwell_mark_reads_the_message_and_tells_the_server() {
        let world = world().await;
        let message = world.message(world.inbox, &[]).await;

        world
            .run(Command::MarkReadOnDwell { message })
            .await
            .expect("the dwell mark applies");

        assert!(world.flags_of(message).await.is_seen());
        assert!(
            matches!(
                world.queued().await.first(),
                Some((_, Operation::SetFlags { .. }))
            ),
            "the server has to hear about it, or the message is unread again \
             on the next device"
        );
    }

    #[tokio::test]
    async fn a_dwell_mark_on_a_thread_member_recomputes_the_threads_unread_count() {
        // `threads.unread_count` is denormalised and unified pages read it, so a `\Seen` write must
        // recompute it (#754).
        let world = world().await;
        let first = world.message(world.inbox, &[]).await;
        let second = world.message(world.inbox, &[]).await;
        let third = world.message(world.inbox, &[]).await;
        let thread = {
            let connection = world.database.connect().await.expect("a connection");
            let threads = ThreadRepository::new(&connection);
            let mut thread = postio_model::Thread::new(world.account.id);
            threads.create(&mut thread).await.expect("a thread");
            for message in [first, second, third] {
                threads
                    .add_message(thread.id, message)
                    .await
                    .expect("membership");
            }
            thread.id
        };

        world
            .run(Command::MarkReadOnDwell { message: first })
            .await
            .expect("the dwell mark applies");

        let connection = world.database.connect().await.expect("a connection");
        let record = ThreadRepository::new(&connection)
            .get(thread)
            .await
            .expect("a read")
            .expect("the thread is still there");
        assert_eq!(
            record.unread_count, 2,
            "the thread's own aggregate has to follow a local read, or the \
             account-scoped page shows the conversation unread for ever"
        );
    }

    #[tokio::test]
    async fn a_dwell_mark_announces_the_conversations_other_members_for_repaint() {
        // The event must carry siblings: `pages_holding` matches row ids, and the row is the
        // representative (#754).
        let world = world().await;
        let first = world.message(world.inbox, &[]).await;
        let second = world.message(world.inbox, &[]).await;
        let third = world.message(world.inbox, &[]).await;
        {
            let connection = world.database.connect().await.expect("a connection");
            let threads = ThreadRepository::new(&connection);
            let mut thread = postio_model::Thread::new(world.account.id);
            threads.create(&mut thread).await.expect("a thread");
            for message in [first, second, third] {
                threads
                    .add_message(thread.id, message)
                    .await
                    .expect("membership");
            }
        }

        world
            .run(Command::MarkReadOnDwell { message: first })
            .await
            .expect("the dwell mark applies");

        let changed: Vec<MessageId> = world
            .drained()
            .await
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
        assert_eq!(
            world.queued().await.len(),
            1,
            "only the marked message goes to the server"
        );
    }

    #[tokio::test]
    async fn a_dwell_mark_leaves_the_undo_stack_alone() {
        let world = world().await;
        let archived = world.message(world.inbox, &[]).await;
        let read = world.message(world.inbox, &[]).await;
        world.looking_at(world.inbox, &[], Some(archived)).await;

        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .await
            .expect("archive");
        assert_ne!(world.mailbox_of(archived).await, world.inbox);

        world
            .run(Command::MarkReadOnDwell { message: read })
            .await
            .expect("the dwell mark applies");

        world.run(Command::Undo).await.expect("undo");
        assert_eq!(
            world.mailbox_of(archived).await,
            world.inbox,
            "`u` reached the dwell mark instead of the archive, so reading a \
             mailbox now costs the user their undo"
        );
        assert!(
            world.flags_of(read).await.is_seen(),
            "and the dwell mark itself is not something `u` takes back"
        );
    }

    #[tokio::test]
    async fn a_dwell_mark_raises_no_toast() {
        let world = world().await;
        let message = world.message(world.inbox, &[]).await;

        world
            .run(Command::MarkReadOnDwell { message })
            .await
            .expect("the dwell mark applies");

        let events = world.drained().await;
        assert_eq!(completion(&events).await, None, "{events:?}");
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::MessagesChanged { .. })),
            "the row still has to repaint, or the list goes on showing it unread"
        );
    }

    #[tokio::test]
    async fn a_dwell_mark_on_a_message_already_read_queues_nothing() {
        let world = world().await;
        let message = world.message(world.inbox, &[Flag::Seen]).await;

        world
            .run(Command::MarkReadOnDwell { message })
            .await
            .expect("a no-op is not an error");

        assert!(world.flags_of(message).await.is_seen());
        assert!(
            world.queued().await.is_empty(),
            "an already-read message must not put a redundant \\Seen on the queue"
        );
    }

    #[tokio::test]
    async fn a_move_with_no_destination_asks_rather_than_guessing() {
        let world = world().await;
        let message = world.message(world.inbox, &[]).await;
        world.looking_at(world.inbox, &[], Some(message)).await;

        let outcome = world
            .run(Command::Move {
                target: MessageTarget::Selection,
                to: None,
            })
            .await;

        assert!(matches!(outcome, Err(CommandError::Rejected(_))));
        assert_eq!(world.mailbox_of(message).await, world.inbox);
    }

    #[tokio::test]
    async fn ctrl_a_then_archive_files_the_whole_mailbox() {
        let world = world().await;
        let mut messages: Vec<MessageId> = Vec::new();
        for _ in 0..30 {
            messages.push(world.message(world.inbox, &[]).await);
        }
        world.everything_in(world.inbox).await;

        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .await
            .expect("archive the mailbox");

        for message in &messages {
            assert_eq!(world.mailbox_of(*message).await, world.archive);
        }
        assert_eq!(
            world.queued().await.len(),
            30,
            "the server has to be told too"
        );
        assert_eq!(
            completion(&world.drained().await).await,
            Some(("Archived 30 messages", true)),
            "one gesture, one sentence, and an undo behind it"
        );
    }

    #[tokio::test]
    async fn ctrl_a_then_unflag_clears_the_whole_flagged_view() {
        let world = world().await;
        let mut flagged: Vec<MessageId> = Vec::new();
        for index in 0..6 {
            let mailbox = if index % 2 == 0 {
                world.inbox
            } else {
                world.archive
            };
            let message = world.message(mailbox, &[]).await;
            world.flag(message, Flag::Flagged).await;
            flagged.push(message);
        }
        let untouched = world.message(world.inbox, &[]).await;
        world.everything_flagged().await;

        world
            .run(Command::Flag {
                target: MessageTarget::Selection,
                flagged: Some(false),
            })
            .await
            .expect("Ctrl+A in Flagged then unflag must not reject");

        for message in &flagged {
            assert!(
                !world.flags_of(*message).await.contains(&Flag::Flagged),
                "every flagged message in the account should have been cleared, \
                 across folders"
            );
        }
        assert!(!world.flags_of(untouched).await.contains(&Flag::Flagged));
        assert_eq!(
            world.queued().await.len(),
            6,
            "one queue row per message, written before the write emptied the \
             predicate they were selected by"
        );
    }

    #[tokio::test]
    async fn a_unified_archive_leaves_out_an_account_the_selection_was_not_scoped_to() {
        // The account is not re-consulted at verb time: one that reconnected since would join a
        // selection the user never saw (ADR 0005 Q10, #811).
        let world = world().await;
        let away = world.second_account().await;
        let mine = world.message(world.inbox, &[]).await;
        let theirs = world.message_for(&away.account, away.inbox, &[]).await;
        world.everything_unified(&[world.account.id]).await;

        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .await
            .expect("Ctrl+A in the unified view then archive must not reject");

        assert_eq!(world.mailbox_of(mine).await, world.archive);
        assert_eq!(
            world.mailbox_of(theirs).await,
            away.inbox,
            "an account the selection was never scoped to must not be archived"
        );
    }

    #[tokio::test]
    async fn a_unified_archive_files_each_account_in_its_own_archive() {
        let world = world().await;
        let away = world.second_account().await;
        let mine = world.message(world.inbox, &[]).await;
        let theirs = world.message_for(&away.account, away.inbox, &[]).await;
        world
            .everything_unified(&[world.account.id, away.account.id])
            .await;

        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .await
            .expect("an archive across both accounts");

        assert_eq!(world.mailbox_of(mine).await, world.archive);
        assert_eq!(
            world.mailbox_of(theirs).await,
            away.archive,
            "each account's mail goes to that account's own Archive"
        );
    }

    #[tokio::test]
    async fn a_unified_flag_write_only_touches_the_accounts_the_scope_names() {
        let world = world().await;
        let away = world.second_account().await;
        let mine = world.message(world.inbox, &[]).await;
        let theirs = world.message_for(&away.account, away.inbox, &[]).await;
        world.everything_unified(&[world.account.id]).await;

        world
            .run(Command::Flag {
                target: MessageTarget::Selection,
                flagged: Some(true),
            })
            .await
            .expect("a bulk flag over the reachable accounts");

        assert!(world.flags_of(mine).await.contains(&Flag::Flagged));
        assert!(
            !world.flags_of(theirs).await.contains(&Flag::Flagged),
            "an account the selection was never scoped to must not be flagged"
        );
    }

    #[tokio::test]
    async fn a_unified_select_all_is_the_inboxes_and_nothing_filed_away() {
        // #1692: Unified is the inboxes, so `Ctrl+A` there must not reach the Archive.
        let world = world().await;
        let away = world.second_account().await;
        let mine = world.message(world.inbox, &[]).await;
        let theirs = world.message_for(&away.account, away.inbox, &[]).await;
        let filed = world.message(world.archive, &[]).await;
        let filed_away = world.message_for(&away.account, away.archive, &[]).await;
        world
            .everything_unified(&[world.account.id, away.account.id])
            .await;

        world
            .run(Command::Flag {
                target: MessageTarget::Selection,
                flagged: Some(true),
            })
            .await
            .expect("a bulk flag over both inboxes");

        assert!(world.flags_of(mine).await.contains(&Flag::Flagged));
        assert!(world.flags_of(theirs).await.contains(&Flag::Flagged));
        for message in [filed, filed_away] {
            assert!(
                !world.flags_of(message).await.contains(&Flag::Flagged),
                "a message filed away is not in Unified, so not in its select-all"
            );
        }
    }

    #[tokio::test]
    async fn a_unified_bulk_action_announces_itself_once_per_account() {
        let world = world().await;
        let away = world.second_account().await;
        world.message(world.inbox, &[]).await;
        world.message_for(&away.account, away.inbox, &[]).await;
        world
            .everything_unified(&[world.account.id, away.account.id])
            .await;

        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .await
            .expect("an archive across both accounts");

        let accounts: Vec<AccountId> = world
            .drained()
            .await
            .into_iter()
            .filter_map(|event| match event {
                Event::MessageListChanged { account, .. } => Some(account),
                _ => None,
            })
            .collect();
        assert!(
            accounts.contains(&world.account.id) && accounts.contains(&away.account.id),
            "both accounts' lists have to hear about it: {accounts:?}"
        );
    }

    #[tokio::test]
    async fn one_undo_puts_a_whole_flagged_view_back() {
        // Ordering trap: queue rows written after the flag came off would leave `u` nothing to take
        // back.
        let world = world().await;
        let mut flagged: Vec<MessageId> = Vec::new();
        for index in 0..4 {
            let mailbox = if index % 2 == 0 {
                world.inbox
            } else {
                world.archive
            };
            let message = world.message(mailbox, &[]).await;
            world.flag(message, Flag::Flagged).await;
            flagged.push(message);
        }
        world.everything_flagged().await;
        world
            .run(Command::Flag {
                target: MessageTarget::Selection,
                flagged: Some(false),
            })
            .await
            .expect("unflag everything");
        let _ = world.drained().await;

        world.run(Command::Undo).await.expect("undo");

        for message in &flagged {
            assert!(
                world.flags_of(*message).await.contains(&Flag::Flagged),
                "undo has to reach the rows the action touched, wherever they \
                 are filed"
            );
        }
    }

    #[tokio::test]
    async fn ctrl_a_then_archive_in_flagged_files_from_every_folder_it_spans() {
        let world = world().await;
        let from_inbox = world.message(world.inbox, &[]).await;
        let from_trash = world.message(world.trash, &[]).await;
        world.flag(from_inbox, Flag::Flagged).await;
        world.flag(from_trash, Flag::Flagged).await;
        let already_there = world.message(world.archive, &[]).await;
        world.flag(already_there, Flag::Flagged).await;
        world.everything_flagged().await;

        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .await
            .expect("archive everything flagged");

        assert_eq!(world.mailbox_of(from_inbox).await, world.archive);
        assert_eq!(world.mailbox_of(from_trash).await, world.archive);
        assert_eq!(
            world.mailbox_of(already_there).await,
            world.archive,
            "it was already there and stayed"
        );

        let queued = world.queued().await;
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

    #[tokio::test]
    async fn undoing_an_archive_out_of_flagged_returns_each_message_to_its_own_folder() {
        let world = world().await;
        let from_inbox = world.message(world.inbox, &[]).await;
        let from_trash = world.message(world.trash, &[]).await;
        world.flag(from_inbox, Flag::Flagged).await;
        world.flag(from_trash, Flag::Flagged).await;
        world.everything_flagged().await;
        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .await
            .expect("archive everything flagged");
        let _ = world.drained().await;

        world.run(Command::Undo).await.expect("undo");

        assert_eq!(
            world.mailbox_of(from_inbox).await,
            world.inbox,
            "back where it came from, not to one folder for all of them"
        );
        assert_eq!(world.mailbox_of(from_trash).await, world.trash);
    }

    #[tokio::test]
    async fn one_undo_takes_a_whole_mailbox_back() {
        let world = world().await;
        let mut messages: Vec<MessageId> = Vec::new();
        for _ in 0..20 {
            messages.push(world.message(world.inbox, &[]).await);
        }
        let elsewhere = world.message(world.archive, &[]).await;
        world.everything_in(world.inbox).await;
        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .await
            .expect("archive the mailbox");
        let _ = world.drained().await;

        world.run(Command::Undo).await.expect("undo");

        for message in &messages {
            assert_eq!(
                world.mailbox_of(*message).await,
                world.inbox,
                "all the way back"
            );
        }
        assert_eq!(
            world.mailbox_of(elsewhere).await,
            world.archive,
            "undo puts back what the action moved, not everything that was in \
             the folder it moved things into"
        );
        assert_eq!(
            world.run(Command::Undo).await,
            Err(CommandError::rejected("Nothing to undo")),
            "one entry, not twenty"
        );
    }

    #[tokio::test]
    async fn a_whole_mailbox_archive_is_one_undo_entry_even_beside_its_neighbours() {
        let world = world().await;
        for _ in 0..5 {
            world.message(world.inbox, &[]).await;
        }
        let other = world.message(world.trash, &[]).await;
        world.everything_in(world.inbox).await;
        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .await
            .expect("archive the mailbox");
        world.looking_at(world.trash, &[], Some(other)).await;
        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .await
            .expect("archive one more");
        let _ = world.drained().await;

        world.run(Command::Undo).await.expect("undo the single one");
        assert_eq!(world.mailbox_of(other).await, world.trash);

        world.run(Command::Undo).await.expect("undo the bulk one");
        assert_eq!(
            world.count_in(world.inbox).await,
            5,
            "the two gestures come back separately"
        );
    }

    #[tokio::test]
    async fn the_rows_taken_back_out_of_a_select_all_stay_where_they_are() {
        let world = world().await;
        let mut messages: Vec<MessageId> = Vec::new();
        for _ in 0..6 {
            messages.push(world.message(world.inbox, &[]).await);
        }
        world.everything_in(world.inbox).await;
        world.state.update(&world.quiet, |app: &mut AppState| {
            app.toggle_selection(messages[2])
        });

        world
            .run(Command::Delete {
                target: MessageTarget::Selection,
            })
            .await
            .expect("delete the rest");

        assert_eq!(world.mailbox_of(messages[2]).await, world.inbox);
        assert_eq!(world.mailbox_of(messages[0]).await, world.trash);
        assert_eq!(
            completion(&world.drained().await).await,
            Some(("Deleted 5 messages", true))
        );
    }

    #[tokio::test]
    async fn a_whole_mailbox_action_tells_both_lists_to_reload() {
        let world = world().await;
        world.message(world.inbox, &[]).await;
        world.everything_in(world.inbox).await;

        world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .await
            .expect("archive the mailbox");

        let events = world.drained().await;
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

    #[tokio::test]
    async fn archiving_a_whole_mailbox_that_is_already_the_archive_is_refused() {
        let world = world().await;
        world.message(world.archive, &[]).await;
        world.everything_in(world.archive).await;

        assert_eq!(
            world
                .run(Command::Archive {
                    target: MessageTarget::Selection,
                })
                .await,
            Err(CommandError::rejected("Already there"))
        );
    }

    #[tokio::test]
    async fn a_whole_mailbox_selection_over_an_empty_mailbox_says_so() {
        let world = world().await;
        world.everything_in(world.inbox).await;

        assert_eq!(
            world
                .run(Command::Archive {
                    target: MessageTarget::Selection,
                })
                .await,
            Err(CommandError::rejected("There is nothing here to move"))
        );
    }

    #[tokio::test]
    async fn ctrl_a_then_mark_read_marks_the_whole_mailbox() {
        let world = world().await;
        let mut messages: Vec<MessageId> = Vec::new();
        for _ in 0..25 {
            messages.push(world.message(world.inbox, &[]).await);
        }
        world.everything_in(world.inbox).await;

        world
            .run(Command::ToggleRead {
                target: MessageTarget::Selection,
                unread: Some(false),
            })
            .await
            .expect("mark the mailbox read");

        for message in &messages {
            assert!(world.flags_of(*message).await.is_seen());
        }
        assert_eq!(
            world.queued().await.len(),
            25,
            "the server has to be told, once per message"
        );
        assert_eq!(
            completion(&world.drained().await).await,
            Some(("Marked 25 messages as read", true))
        );
    }

    #[tokio::test]
    async fn a_whole_mailbox_flag_toggle_makes_them_agree() {
        let world = world().await;
        let mut messages: Vec<MessageId> = Vec::new();
        for _ in 0..6 {
            messages.push(world.message(world.inbox, &[]).await);
        }
        world.flag(messages[0], Flag::Flagged).await;
        world.everything_in(world.inbox).await;

        world
            .run(Command::Flag {
                target: MessageTarget::Selection,
                flagged: None,
            })
            .await
            .expect("toggle the mailbox");
        for message in &messages {
            assert!(
                world.flags_of(*message).await.is_flagged(),
                "they now agree"
            );
        }

        world
            .run(Command::Flag {
                target: MessageTarget::Selection,
                flagged: None,
            })
            .await
            .expect("toggle it back");
        for message in &messages {
            assert!(
                !world.flags_of(*message).await.is_flagged(),
                "agreeing, the toggle takes it off them all"
            );
        }
    }

    #[tokio::test]
    async fn a_whole_mailbox_flag_only_queues_the_rows_it_changed() {
        let world = world().await;
        let already = world.message(world.inbox, &[Flag::Flagged]).await;
        let mut rest: Vec<MessageId> = Vec::new();
        for _ in 0..4 {
            rest.push(world.message(world.inbox, &[]).await);
        }
        world.everything_in(world.inbox).await;

        world
            .run(Command::Flag {
                target: MessageTarget::Selection,
                flagged: Some(true),
            })
            .await
            .expect("flag the mailbox");

        let queued = world.queued().await;
        assert_eq!(queued.len(), 4);
        assert!(
            !queued
                .iter()
                .any(|(target, _)| *target == OperationTarget::Message(already)),
            "it was already flagged; there is nothing to tell the server"
        );
        assert_eq!(
            completion(&world.drained().await).await,
            Some(("Flagged 4 messages", true)),
            "the sentence counts what changed, not what was selected"
        );
        for message in &rest {
            assert!(world.flags_of(*message).await.is_flagged());
        }
    }

    #[tokio::test]
    async fn one_undo_takes_a_whole_mailbox_flag_back() {
        let world = world().await;
        let already = world.message(world.inbox, &[Flag::Flagged]).await;
        let mut rest: Vec<MessageId> = Vec::new();
        for _ in 0..8 {
            rest.push(world.message(world.inbox, &[]).await);
        }
        world.everything_in(world.inbox).await;
        world
            .run(Command::Flag {
                target: MessageTarget::Selection,
                flagged: Some(true),
            })
            .await
            .expect("flag the mailbox");
        let _ = world.drained().await;

        world.run(Command::Undo).await.expect("undo");

        for message in &rest {
            assert!(
                !world.flags_of(*message).await.is_flagged(),
                "all the way back"
            );
        }
        assert!(
            world.flags_of(already).await.is_flagged(),
            "undo takes back what the action flagged, not what was flagged already"
        );
        assert_eq!(
            world.run(Command::Undo).await,
            Err(CommandError::rejected("Nothing to undo")),
            "one entry, not eight"
        );
    }

    #[tokio::test]
    async fn a_whole_mailbox_already_agreeing_with_the_flag_says_so() {
        let world = world().await;
        world.message(world.inbox, &[Flag::Flagged]).await;
        world.message(world.inbox, &[Flag::Flagged]).await;
        world.everything_in(world.inbox).await;

        assert_eq!(
            world
                .run(Command::Flag {
                    target: MessageTarget::Selection,
                    flagged: Some(true),
                })
                .await,
            Err(CommandError::rejected("Already set"))
        );
    }

    #[tokio::test]
    async fn a_whole_mailbox_flag_over_an_empty_mailbox_says_so() {
        let world = world().await;
        world.everything_in(world.inbox).await;

        assert_eq!(
            world
                .run(Command::Flag {
                    target: MessageTarget::Selection,
                    flagged: Some(true),
                })
                .await,
            Err(CommandError::rejected("There is nothing here to change"))
        );
    }

    #[tokio::test]
    async fn a_whole_mailbox_flag_tells_the_list_to_reload_rather_than_naming_rows() {
        let world = world().await;
        world.message(world.inbox, &[]).await;
        world.everything_in(world.inbox).await;

        world
            .run(Command::ToggleRead {
                target: MessageTarget::Selection,
                unread: Some(false),
            })
            .await
            .expect("mark the mailbox read");

        let events = world.drained().await;
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

    #[tokio::test]
    async fn archiving_a_thread_group_archives_every_copy_one_unit_per_account() {
        // #184, ADR 0005 Q2: archiving a deduped unified row must hit every copy, one operation per
        // account queue.
        let world = world().await;
        let (second, second_inbox, second_archive) = {
            let connection = world.database.connect().await.expect("a connection");
            let mut account = postio_model::Account::new(
                "Second",
                postio_model::EmailAddress::new(None::<String>, "grace@example.org"),
            );
            postio_storage::repository::AccountRepository::new(&connection)
                .create(&mut account)
                .await
                .expect("second account");
            let inbox = test_support::mailbox(&connection, &account, "INBOX")
                .await
                .id;
            let archive = test_support::mailbox(&connection, &account, "Archive")
                .await
                .id;
            (account.id, inbox, archive)
        };

        let file = async |account: AccountId, mailbox: MailboxId| -> ThreadId {
            let connection = world.database.connect().await.expect("a connection");
            let mut message = Message::new(account, mailbox, Utc::now());
            message.rfc_message_id = Some(postio_model::RfcMessageId::new("<pair@example.com>"));
            message.subject = Some("Paired".to_owned());
            MessageRepository::new(&connection)
                .create(&mut message)
                .await
                .expect("a message");
            postio_storage::repository::ThreadingRepository::new(&connection, account)
                .thread(&message)
                .await
                .expect("threaded")
                .thread_id
        };
        let first_thread = file(world.account.id, world.inbox).await;
        let second_thread = file(second, second_inbox).await;

        world
            .run(Command::Archive {
                target: MessageTarget::Threads(vec![first_thread, second_thread]),
            })
            .await
            .expect("the group archives");

        let connection = world.database.connect().await.expect("a connection");
        let messages = MessageRepository::new(&connection);
        let in_archive = async |mailbox: MailboxId| -> u32 {
            messages
                .count(&postio_storage::repository::ListQuery {
                    scope: postio_storage::repository::ListScope::Mailbox(mailbox),
                    limit: 10,
                    after: None,
                })
                .await
                .expect("a count")
        };
        assert_eq!(
            in_archive(world.archive).await,
            1,
            "the first account's copy moved"
        );
        assert_eq!(
            in_archive(second_archive).await,
            1,
            "and the second account's copy moved into *its own* Archive"
        );

        let queue = postio_storage::repository::OperationQueueRepository::new(&connection);
        let first_ops = queue
            .pending(world.account.id, Utc::now())
            .await
            .expect("queue");
        let second_ops = queue.pending(second, Utc::now()).await.expect("queue");
        assert_eq!(first_ops.len(), 1, "one operation in the first queue");
        assert_eq!(
            second_ops.len(),
            1,
            "one in the second: per-account, always"
        );
    }

    #[tokio::test]
    async fn a_move_to_another_accounts_folder_starts_the_saga_not_a_move() {
        let world = world().await;
        let (second, second_inbox) = {
            let connection = world.database.connect().await.expect("a connection");
            let mut account = postio_model::Account::new(
                "Second",
                postio_model::EmailAddress::new(None::<String>, "grace@example.org"),
            );
            postio_storage::repository::AccountRepository::new(&connection)
                .create(&mut account)
                .await
                .expect("second account");
            let inbox = test_support::mailbox(&connection, &account, "INBOX")
                .await
                .id;
            (account.id, inbox)
        };
        let message = world.message(world.inbox, &[]).await;
        world.looking_at(world.inbox, &[], Some(message)).await;

        world
            .run(Command::Move {
                target: MessageTarget::Messages(vec![message]),
                to: Some(second_inbox),
            })
            .await
            .expect("the move starts");

        let connection = world.database.connect().await.expect("a connection");
        let queue = postio_storage::repository::OperationQueueRepository::new(&connection);
        let source_ops = queue
            .pending(world.account.id, Utc::now())
            .await
            .expect("queue");
        let target_ops = queue.pending(second, Utc::now()).await.expect("queue");
        assert_eq!(source_ops.len(), 1);
        assert_eq!(
            source_ops[0].operation.op_type(),
            "cross_account_remove",
            "the source queue holds phase 3, and only phase 3"
        );
        assert_eq!(target_ops.len(), 1);
        assert_eq!(target_ops[0].operation.op_type(), "cross_account_copy");

        let messages = MessageRepository::new(&connection);
        let source_row = messages.get(message).await.expect("read").expect("the row");
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
            .await
            .expect("a count");
        assert_eq!(copies, 1, "the provisional copy is already in the target");
    }

    async fn second_account(world: &World) -> (postio_model::ids::AccountId, MailboxId) {
        let connection = world.database.connect().await.expect("a connection");
        let mut account = postio_model::Account::new(
            "Second",
            postio_model::EmailAddress::new(None::<String>, "grace@example.org"),
        );
        postio_storage::repository::AccountRepository::new(&connection)
            .create(&mut account)
            .await
            .expect("second account");
        let inbox = test_support::mailbox(&connection, &account, "INBOX")
            .await
            .id;
        (account.id, inbox)
    }

    /// #940: the provisional copy carries no server identity; asserted on the row as stored, since
    /// the struct is correct until `create` persists all four fields.
    #[tokio::test]
    async fn the_provisional_copy_carries_no_server_identity() {
        let world = world().await;
        let (_second, second_inbox) = second_account(&world).await;

        let source = {
            let connection = world.database.connect().await.expect("a connection");
            let mut message = Message::new(world.account.id, world.inbox, Utc::now());
            message.server.uid = Some(4242.into());
            message.server.uid_validity = Some(9.into());
            message.server.mod_seq = Some(777.into());
            message.server.remote_id = Some(postio_model::RemoteId::new("9:4242"));
            MessageRepository::new(&connection)
                .create(&mut message)
                .await
                .expect("a message the source server has seen")
        };
        world.looking_at(world.inbox, &[], Some(source)).await;

        world
            .run(Command::Move {
                target: MessageTarget::Messages(vec![source]),
                to: Some(second_inbox),
            })
            .await
            .expect("the move starts");

        let connection = world.database.connect().await.expect("a connection");
        let messages = MessageRepository::new(&connection);
        let rows = messages
            .page(&postio_storage::repository::ListQuery {
                scope: postio_storage::repository::ListScope::Mailbox(second_inbox),
                limit: 10,
                after: None,
            })
            .await
            .expect("a read");
        let mut stored: Vec<Message> = Vec::new();
        for row in rows {
            stored.push(
                messages
                    .get(row.id)
                    .await
                    .expect("a read")
                    .expect("the row the page just listed"),
            );
        }
        let copy = stored
            .first()
            .expect("the provisional copy is in the target mailbox");

        assert!(
            !copy.server.is_known_to_server(),
            "the target server has never seen this row and it says otherwise: \
             uid {:?}, remote_id {:?}",
            copy.server.uid,
            copy.server.remote_id
        );
        assert_eq!(
            copy.server,
            postio_model::ServerIdentifiers::default(),
            "a row no server has seen has no server identity -- and clearing \
             the fields one at a time is exactly how two of the four were \
             missed"
        );

        let source_remote = messages
            .get(source)
            .await
            .expect("a read")
            .expect("the source row")
            .server
            .remote_id
            .expect("the source keeps its own identity");
        assert!(
            stored
                .iter()
                .all(|row| row.server.remote_id.as_ref() != Some(&source_remote)),
            "a row in the target mailbox answers to {source_remote}, which is \
             an id account A's server minted"
        );
    }

    async fn advance_saga(
        world: &World,
        source: MessageId,
        phase: postio_storage::repository::MovePhase,
    ) {
        use postio_storage::repository::{CrossAccountMoveRepository, MovePhase};
        let connection = world.database.connect().await.expect("a connection");
        let sagas = CrossAccountMoveRepository::new(&connection);
        let id = sagas
            .for_sources(&[source], OPEN_PHASES)
            .await
            .expect("a read")
            .first()
            .expect("a saga")
            .id;
        let saga = sagas.get(id).await.expect("a read").expect("a saga");
        let queue = OperationQueueRepository::new(&connection);

        if let Some(copy) = saga.target_message {
            while let Some(operation) = queue
                .pending_for(postio_model::OperationTarget::Message(copy))
                .await
                .expect("a read")
            {
                queue
                    .delete(operation.id)
                    .await
                    .expect("the copy operation ran");
            }
        }
        sagas
            .confirm(id, Some(&postio_model::RemoteId::new("77:9")))
            .await
            .expect("the target server proved where it landed");

        if phase == MovePhase::Done {
            if let Some(source) = saga.source_message {
                while let Some(operation) = queue
                    .pending_for(postio_model::OperationTarget::Message(source))
                    .await
                    .expect("a read")
                {
                    queue.delete(operation.id).await.expect("the removal ran");
                }
            }
            sagas
                .transition(id, MovePhase::Done)
                .await
                .expect("the source copy went");
        }
    }

    async fn only_saga(
        world: &World,
        source: MessageId,
    ) -> postio_storage::repository::CrossAccountMove {
        use postio_storage::repository::MovePhase;
        let connection = world.database.connect().await.expect("a connection");
        let mut all = postio_storage::repository::CrossAccountMoveRepository::new(&connection)
            .for_sources(
                &[source],
                &[
                    MovePhase::Copying,
                    MovePhase::Unconfirmed,
                    MovePhase::Confirmed,
                    MovePhase::Done,
                    MovePhase::Aborted,
                ],
            )
            .await
            .expect("a read");
        assert_eq!(all.len(), 1, "the fixture makes exactly one saga");
        all.remove(0)
    }

    /// #531: `u` after a completed cross-account move runs the saga backwards, with source and
    /// target exchanged.
    #[tokio::test]
    async fn undo_after_the_move_completed_starts_the_inverse_saga() {
        use postio_storage::repository::{CrossAccountMoveRepository, MovePhase};
        let world = world().await;
        let (second, second_inbox) = second_account(&world).await;
        let message = world.message(world.inbox, &[]).await;
        world.looking_at(world.inbox, &[], Some(message)).await;

        world
            .run(Command::Move {
                target: MessageTarget::Messages(vec![message]),
                to: Some(second_inbox),
            })
            .await
            .expect("the move starts");
        advance_saga(&world, message, MovePhase::Done).await;
        let forward = only_saga(&world, message).await;
        let copy = forward.target_message.expect("the provisional copy");

        world
            .run(Command::Undo)
            .await
            .expect("undo starts the inverse saga");

        let connection = world.database.connect().await.expect("a connection");
        let messages = MessageRepository::new(&connection);

        assert!(
            !messages
                .get(message)
                .await
                .expect("read")
                .expect("the source row")
                .sync
                .deleted_locally,
            "the message is back in the account it came from, at once"
        );
        assert!(
            messages
                .get(copy)
                .await
                .expect("read")
                .expect("the copy is still a row until its server agrees")
                .sync
                .deleted_locally,
            "and gone from the account it went to"
        );

        let sagas = CrossAccountMoveRepository::new(&connection);
        let inverse = sagas
            .for_sources(&[copy], OPEN_PHASES)
            .await
            .expect("a read")
            .into_iter()
            .find(|saga| saga.id != forward.id)
            .expect("undo started an inverse saga from the copy");

        assert_eq!(
            inverse.source_message,
            Some(copy),
            "the inverse removes the copy account B holds"
        );
        assert_eq!(
            inverse.target_account,
            Some(world.account.id),
            "and restores it to the account the message came from"
        );
        assert_eq!(inverse.target_message, Some(message));
        assert_eq!(inverse.phase, MovePhase::Copying);

        let queue = postio_storage::repository::OperationQueueRepository::new(&connection);
        let on_a: Vec<String> = queue
            .pending(world.account.id, Utc::now())
            .await
            .expect("queue")
            .into_iter()
            .map(|row| row.operation.op_type().to_owned())
            .collect();
        let on_b: Vec<String> = queue
            .pending(second, Utc::now())
            .await
            .expect("queue")
            .into_iter()
            .map(|row| row.operation.op_type().to_owned())
            .collect();
        assert_eq!(
            on_a,
            vec!["cross_account_copy"],
            "account A is asked to take the message back"
        );
        assert_eq!(
            on_b,
            vec!["cross_account_remove"],
            "account B is asked to give it up -- and only after A confirms"
        );
    }

    /// #531: nothing has reached either server, so undo is local bookkeeping and can be complete.
    #[tokio::test]
    async fn undo_of_a_move_that_has_not_reached_a_server_cancels_the_saga() {
        let world = world().await;
        let (second, second_inbox) = second_account(&world).await;
        let message = world.message(world.inbox, &[]).await;
        world.looking_at(world.inbox, &[], Some(message)).await;
        world
            .run(Command::Move {
                target: MessageTarget::Messages(vec![message]),
                to: Some(second_inbox),
            })
            .await
            .expect("the move starts");

        world
            .run(Command::Undo)
            .await
            .expect("undo cancels the saga");

        let connection = world.database.connect().await.expect("a connection");
        let messages = MessageRepository::new(&connection);
        let source_row = messages.get(message).await.expect("read").expect("the row");
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
            .await
            .expect("a count");
        assert_eq!(copies, 0, "the provisional copy in the target is gone");

        let queue = postio_storage::repository::OperationQueueRepository::new(&connection);
        assert!(
            queue
                .pending(world.account.id, Utc::now())
                .await
                .expect("queue")
                .is_empty(),
            "the remove never runs: it was withdrawn, not left to be skipped"
        );
        assert!(
            queue
                .pending(second, Utc::now())
                .await
                .expect("queue")
                .is_empty(),
            "and neither does the copy"
        );

        let phase: String = postio_storage::sql::one(
            &connection,
            "SELECT phase FROM cross_account_moves",
            (),
            |row| postio_storage::sql::RowExt::col(row, 0),
        )
        .await
        .expect("the saga row");
        assert_eq!(
            phase, "aborted",
            "the saga ends in the phase that means nothing was deleted"
        );
    }

    /// #531: undoing at `confirmed` must withdraw the forward removal first, or A's queue deletes
    /// the message the inverse just restored.
    #[tokio::test]
    async fn undo_at_confirmed_withdraws_the_removal_that_would_delete_the_restored_copy() {
        use postio_storage::repository::{CrossAccountMoveRepository, MovePhase};
        let world = world().await;
        let (second, second_inbox) = second_account(&world).await;
        let message = world.message(world.inbox, &[]).await;
        world.looking_at(world.inbox, &[], Some(message)).await;

        world
            .run(Command::Move {
                target: MessageTarget::Messages(vec![message]),
                to: Some(second_inbox),
            })
            .await
            .expect("the move starts");
        advance_saga(&world, message, MovePhase::Confirmed).await;
        let forward = only_saga(&world, message).await;

        {
            let connection = world.database.connect().await.expect("a connection");
            let pending: Vec<String> =
                postio_storage::repository::OperationQueueRepository::new(&connection)
                    .pending(world.account.id, Utc::now())
                    .await
                    .expect("queue")
                    .into_iter()
                    .map(|row| row.operation.op_type().to_owned())
                    .collect();
            assert_eq!(
                pending,
                vec!["cross_account_remove"],
                "the forward removal has to be pending, or this test cannot fail"
            );
        }

        world
            .run(Command::Undo)
            .await
            .expect("undo inverts the saga");

        let connection = world.database.connect().await.expect("a connection");
        let sagas = CrossAccountMoveRepository::new(&connection);
        assert_eq!(
            sagas
                .get(forward.id)
                .await
                .expect("a read")
                .expect("the forward saga")
                .phase,
            MovePhase::Aborted,
            "the forward saga has to stop, or its removal is still legal"
        );

        let queue = postio_storage::repository::OperationQueueRepository::new(&connection);
        let on_a: Vec<String> = queue
            .pending(world.account.id, Utc::now())
            .await
            .expect("queue")
            .into_iter()
            .map(|row| row.operation.op_type().to_owned())
            .collect();
        assert_eq!(
            on_a,
            vec!["cross_account_copy"],
            "account A's queue still holds the forward removal, which will \
             delete the message the inverse just put back: {on_a:?}"
        );
        assert!(
            !MessageRepository::new(&connection)
                .get(message)
                .await
                .expect("read")
                .expect("the source row")
                .sync
                .deleted_locally,
            "and the message is back in the account it came from"
        );
        let _ = second;
    }

    /// #531: one unprovable message must not pin the others; the undo does what it can and names
    /// what it skipped.
    #[tokio::test]
    async fn a_bulk_undo_inverts_what_it_can_and_names_what_it_skipped() {
        use postio_storage::repository::{CrossAccountMoveRepository, MovePhase};
        let world = world().await;
        let (_second, second_inbox) = second_account(&world).await;
        let first = world.message(world.inbox, &[]).await;
        let second_message = world.message(world.inbox, &[]).await;
        world
            .looking_at(world.inbox, &[first, second_message], Some(first))
            .await;

        world
            .run(Command::Move {
                target: MessageTarget::Selection,
                to: Some(second_inbox),
            })
            .await
            .expect("both moves start");

        advance_saga(&world, first, MovePhase::Done).await;
        {
            let connection = world.database.connect().await.expect("a connection");
            let sagas = CrossAccountMoveRepository::new(&connection);
            let id = sagas
                .for_sources(&[second_message], OPEN_PHASES)
                .await
                .expect("a read")
                .first()
                .expect("the second saga")
                .id;
            sagas
                .transition(id, MovePhase::Unconfirmed)
                .await
                .expect("its append could not be proven");
        }

        world
            .run(Command::Undo)
            .await
            .expect("the undo proceeds rather than refusing outright");

        let said = world
            .drained()
            .await
            .into_iter()
            .find_map(|event| match event {
                Event::UndoPerformed { description } => Some(description),
                _ => None,
            })
            .expect("undo says what it did");
        assert!(
            said.contains('1') && said.to_lowercase().contains("could not be confirmed"),
            "the sentence has to name the one it could not take back, or the \
             user believes they have both: {said:?}"
        );

        let connection = world.database.connect().await.expect("a connection");
        let messages = MessageRepository::new(&connection);
        assert!(
            !messages
                .get(first)
                .await
                .expect("read")
                .expect("the first row")
                .sync
                .deleted_locally,
            "the provable one came back"
        );
        assert!(
            messages
                .get(second_message)
                .await
                .expect("read")
                .expect("the second row")
                .sync
                .deleted_locally,
            "and the unprovable one is untouched, exactly where it was"
        );
    }

    /// #531 / ADR 0005 Q9: an unconfirmed append is the one phase undo refuses, out loud.
    #[tokio::test]
    async fn undo_of_an_unconfirmable_move_is_refused_not_faked() {
        let world = world().await;
        let (_second, second_inbox) = second_account(&world).await;
        let message = world.message(world.inbox, &[]).await;
        world.looking_at(world.inbox, &[], Some(message)).await;
        world
            .run(Command::Move {
                target: MessageTarget::Messages(vec![message]),
                to: Some(second_inbox),
            })
            .await
            .expect("the move starts");

        {
            let connection = world.database.connect().await.expect("a connection");
            let sagas = postio_storage::repository::CrossAccountMoveRepository::new(&connection);
            let id = sagas
                .for_sources(&[message], OPEN_PHASES)
                .await
                .expect("read")
                .first()
                .expect("a saga")
                .id;
            sagas
                .transition(id, postio_storage::repository::MovePhase::Unconfirmed)
                .await
                .expect("the append ran and could not be proven");
        }

        let outcome = world.run(Command::Undo).await;

        assert!(
            outcome.is_err(),
            "an unprovable copy cannot be undone by guessing"
        );
        let connection = world.database.connect().await.expect("a connection");
        let source_row = MessageRepository::new(&connection)
            .get(message)
            .await
            .expect("read")
            .expect("the row");
        assert!(
            source_row.sync.deleted_locally,
            "and a refused undo changes nothing"
        );
        let phase: String = postio_storage::sql::one(
            &connection,
            "SELECT phase FROM cross_account_moves",
            (),
            |row| postio_storage::sql::RowExt::col(row, 0),
        )
        .await
        .expect("the saga row");
        assert_eq!(phase, "unconfirmed", "least of all the saga");
    }

    #[tokio::test]
    async fn a_move_within_one_account_stays_a_single_move_operation() {
        let world = world().await;
        let message = world.message(world.inbox, &[]).await;
        world.looking_at(world.inbox, &[], Some(message)).await;

        world
            .run(Command::Move {
                target: MessageTarget::Messages(vec![message]),
                to: Some(world.archive),
            })
            .await
            .expect("the move applies");

        let connection = world.database.connect().await.expect("a connection");
        let queue = postio_storage::repository::OperationQueueRepository::new(&connection);
        let ops = queue
            .pending(world.account.id, Utc::now())
            .await
            .expect("queue");
        assert_eq!(ops.len(), 1, "one operation on one queue");
        assert_eq!(ops[0].operation.op_type(), "move");
        let sagas: i64 = postio_storage::sql::one(
            &connection,
            "SELECT count(*) FROM cross_account_moves",
            (),
            |row| postio_storage::sql::RowExt::col(row, 0),
        )
        .await
        .expect("count");
        assert_eq!(
            sagas, 0,
            "and no saga exists for the common case to pay for"
        );
    }

    #[tokio::test]
    async fn nothing_selected_and_nothing_focused_is_a_rejection() {
        let world = world().await;

        let outcome = world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .await;

        assert_eq!(outcome, Err(CommandError::rejected("Nothing selected")));
    }

    #[tokio::test]
    async fn an_account_with_no_archive_folder_says_so() {
        let world = world().await;
        {
            let connection = world.database.connect().await.expect("a connection");
            let mailboxes = MailboxRepository::new(&connection);
            let archive = mailboxes
                .get(world.archive)
                .await
                .expect("a read")
                .expect("the folder");
            assert_eq!(archive.role, MailboxRole::Archive);
            mailboxes.delete(world.archive).await.expect("a delete");
        }
        let message = world.message(world.inbox, &[]).await;
        world.looking_at(world.inbox, &[], Some(message)).await;

        let outcome = world
            .run(Command::Archive {
                target: MessageTarget::Selection,
            })
            .await;

        assert!(
            matches!(outcome, Err(CommandError::Rejected(_))),
            "inventing a folder because a key was pressed is not this \
             command's decision to make"
        );
    }

    #[tokio::test]
    async fn undo_with_nothing_to_take_back_is_a_rejection() {
        let world = world().await;

        let outcome = world.run(Command::Undo).await;

        assert_eq!(
            outcome,
            Err(CommandError::rejected("Nothing to undo")),
            "a quiet hint, not a failure: `u` on a fresh session is ordinary"
        );
        assert!(world.drained().await.is_empty(), "and nothing happened");
    }

    #[tokio::test]
    async fn undoing_twice_walks_back_through_history_rather_than_toggling() {
        let world = world().await;
        let first = world.message(world.inbox, &[]).await;
        let second = world.message(world.inbox, &[]).await;
        for message in [first, second] {
            world.looking_at(world.inbox, &[], Some(message)).await;
            world
                .run(Command::Delete {
                    target: MessageTarget::Selection,
                })
                .await
                .expect("delete");
        }

        world.run(Command::Undo).await.expect("undo");

        assert_eq!(world.mailbox_of(first).await, world.inbox);
        assert_eq!(world.mailbox_of(second).await, world.inbox);
        assert_eq!(
            world.run(Command::Undo).await,
            Err(CommandError::rejected("Nothing to undo")),
            "a burst is one unit, so there is nothing behind it"
        );
    }

    #[tokio::test]
    async fn every_wired_command_has_a_handler_and_an_arm() {
        let world = world().await;
        let bus = dispatcher(world.actions.clone());
        assert_eq!(bus.await.wired().collect::<Vec<_>>(), WIRED.to_vec());

        let message = world.message(world.inbox, &[]).await;
        for id in WIRED.iter().copied().filter(|id| *id != CommandId::Undo) {
            world.looking_at(world.inbox, &[], Some(message)).await;
            let outcome = world.run(Command::default_for(id)).await;
            assert!(
                !matches!(&outcome, Err(CommandError::Rejected(reason)) if reason.contains("not wired up")),
                "`{id}` is registered on the bus but `act` has no arm for it"
            );
        }
    }

    #[tokio::test]
    async fn the_bus_answers_add_label() {
        // `WIRED` is what subscribes a command to the bus; `AddLabel` had a handler and an arm but
        // was missing from it.
        let world = world().await;
        let bus = dispatcher(world.actions.clone());
        assert!(
            bus.await.wired().any(|id| id == CommandId::AddLabel),
            "AddLabel is not on the bus, so a command carrying a label \
             reaches nothing that writes"
        );
    }

    #[tokio::test]
    async fn a_label_goes_on_the_selection_and_undo_takes_it_off() {
        let world = world().await;
        let message = world.message(world.inbox, &[]).await;
        let work = world.label("Work").await;

        world
            .run(Command::AddLabel {
                target: MessageTarget::Messages(vec![message]),
                label: Some(work),
                on: None,
            })
            .await
            .expect("label it");

        assert_eq!(world.labels_of(message).await, vec![work]);
        assert!(
            world
                .flags_of(message)
                .await
                .contains(&Flag::Keyword("Work".to_owned())),
            "the label is not on the message as a keyword: {:?}",
            world.flags_of(message).await
        );

        world.run(Command::Undo).await.expect("undo");
        assert!(
            world.labels_of(message).await.is_empty(),
            "`u` left the label on, and the registry promises Recovery::Undo"
        );
        assert!(
            !world
                .flags_of(message)
                .await
                .contains(&Flag::Keyword("Work".to_owned())),
            "the keyword outlived the label it stands for"
        );
    }

    #[tokio::test]
    async fn labelling_tells_the_server_and_undo_tells_it_the_other_thing() {
        let world = world().await;
        let message = world.message(world.inbox, &[]).await;
        let work = world.label("Work").await;
        let keyword: postio_model::FlagSet =
            std::iter::once(Flag::Keyword("Work".to_owned())).collect();

        world
            .run(Command::AddLabel {
                target: MessageTarget::Messages(vec![message]),
                label: Some(work),
                on: None,
            })
            .await
            .expect("label it");

        let connection = world.database.connect().await.expect("a connection");
        let queued = OperationQueueRepository::new(&connection)
            .pending(world.account.id, Utc::now())
            .await
            .expect("the queue");
        assert_eq!(
            queued
                .iter()
                .map(|row| row.operation.clone())
                .collect::<Vec<_>>(),
            vec![Operation::SetFlags {
                flags: keyword.clone()
            }],
            "a label is a keyword on the wire, and the queue is how it gets there"
        );
    }

    #[tokio::test]
    async fn a_label_already_on_the_message_is_not_applied_twice() {
        let world = world().await;
        let message = world.message(world.inbox, &[]).await;
        let work = world.label("Work").await;
        let on = |on| Command::AddLabel {
            target: MessageTarget::Messages(vec![message]),
            label: Some(work),
            on: Some(on),
        };

        world.run(on(true)).await.expect("label it");
        assert_eq!(
            world.run(on(true)).await,
            Err(CommandError::rejected("Already set")),
            "a second `L` on a labelled message must not enqueue a second \
             operation, or `u` would take back something nobody did"
        );
        assert_eq!(world.labels_of(message).await, vec![work]);
    }

    #[tokio::test]
    async fn a_label_with_no_label_asks_rather_than_failing() {
        let world = world().await;
        world.message(world.inbox, &[]).await;
        assert_eq!(
            world
                .run(Command::AddLabel {
                    target: MessageTarget::Selection,
                    label: None,
                    on: None,
                })
                .await,
            Err(CommandError::rejected("Pick a label to add"))
        );
    }

    #[tokio::test]
    async fn mapping_a_role_points_it_at_one_folder_and_says_so() {
        let world = world().await;
        let (_sent, sent_messages) = world.two_sent_folders().await;
        assert_eq!(
            world.wearing(postio_model::MailboxRole::Sent).await.len(),
            2,
            "the fixture starts in #943's state: two folders wearing one role"
        );

        world.map_sent_to(Some("Sent Messages")).await.expect("map");

        let events = world.drained().await;
        assert!(
            events.contains(&Event::MailboxesChanged {
                account: world.account.id
            }),
            "the sidebar has to relabel: {events:?}"
        );
        assert_eq!(
            completion(&events).await,
            Some(("Changed a folder's role", true))
        );
        assert_eq!(
            world.wearing(postio_model::MailboxRole::Sent).await,
            vec![sent_messages],
            "exactly one selectable row wears the role, and it is the chosen one"
        );
        assert_eq!(
            world
                .mapped(postio_model::MailboxRole::Sent)
                .await
                .as_deref(),
            Some("Sent Messages"),
            "and the choice is written down where discovery reads it"
        );
    }

    #[tokio::test]
    async fn undoing_a_mapping_restores_the_one_before_it() {
        let world = world().await;
        let (sent, sent_messages) = world.two_sent_folders().await;

        world
            .map_sent_to(Some("Sent Messages"))
            .await
            .expect("first choice");
        world
            .map_sent_to(Some("Sent"))
            .await
            .expect("second choice");
        let _ = world.drained().await;
        assert_eq!(
            world.wearing(postio_model::MailboxRole::Sent).await,
            vec![sent]
        );

        world.run(Command::Undo).await.expect("undo");

        let events = world.drained().await;
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::UndoPerformed { .. })),
            "{events:?}"
        );
        assert_eq!(
            world
                .mapped(postio_model::MailboxRole::Sent)
                .await
                .as_deref(),
            Some("Sent Messages"),
            "undo is the previous entry, not the absence of one"
        );
        assert_eq!(
            world.wearing(postio_model::MailboxRole::Sent).await,
            vec![sent_messages]
        );

        world.run(Command::Undo).await.expect("undo again");
        assert_eq!(
            world.mapped(postio_model::MailboxRole::Sent).await,
            None,
            "and undoing the first choice is back to automatic"
        );
    }

    #[tokio::test]
    async fn mapping_to_a_folder_the_account_does_not_have_is_refused() {
        let world = world().await;
        let (sent, sent_messages) = world.two_sent_folders().await;

        let outcome = world.map_sent_to(Some("Nowhere")).await;

        assert!(
            matches!(outcome, Err(CommandError::Rejected(_))),
            "a folder the server never listed is not a place mail can go: {outcome:?}"
        );
        assert_eq!(world.mapped(postio_model::MailboxRole::Sent).await, None);
        assert_eq!(
            world.wearing(postio_model::MailboxRole::Sent).await,
            vec![sent, sent_messages],
            "nothing moved"
        );
        assert!(
            world.drained().await.is_empty(),
            "and nothing was announced"
        );
    }

    mod contacts {
        use super::*;
        use postio_core::{ContactAddressAction, ContactJoinAction};
        use postio_model::{ContactId, ContactState, EmailAddress};
        use postio_storage::repository::ContactRepository;

        async fn person(world: &World, name: &str, emails: &[&str]) -> ContactId {
            let connection = world.database.connect().await.expect("a connection");
            let addresses: Vec<EmailAddress> = emails
                .iter()
                .map(|e| EmailAddress::new(None::<String>, *e))
                .collect();
            ContactRepository::new(&connection)
                .create(Some(name), &addresses)
                .await
                .expect("a person")
        }

        async fn get(world: &World, id: ContactId) -> Option<postio_model::Contact> {
            let connection = world.database.connect().await.expect("a connection");
            ContactRepository::new(&connection)
                .get(id)
                .await
                .expect("get")
        }

        fn changed(events: &[Event]) -> Option<bool> {
            events.iter().find_map(|event| match event {
                Event::ContactsChanged { names_changed } => Some(*names_changed),
                _ => None,
            })
        }

        #[tokio::test]
        async fn a_join_writes_says_so_and_undoes_exactly() {
            let world = world().await;
            let work = person(&world, "Ada at work", &["ada@work.example"]).await;
            let home = person(&world, "Ada at home", &["ada@home.example"]).await;
            let before = (get(&world, work).await, get(&world, home).await);

            world
                .run(Command::ContactJoin {
                    action: ContactJoinAction::Join {
                        into: work,
                        others: vec![home],
                        name: "Ada Lovelace".into(),
                        organization: None,
                    },
                })
                .await
                .expect("join");
            let events = world.drained().await;
            assert_eq!(
                changed(&events),
                Some(true),
                "a join changes whose name mail shows"
            );
            assert!(events.contains(&Event::ActionCompleted {
                description: "Merged contacts".into(),
                undoable: true,
            }));
            let joined = get(&world, work).await.expect("ada");
            assert_eq!(joined.addresses.len(), 2);
            assert_eq!(
                get(&world, home).await.expect("home").state,
                ContactState::Merged
            );

            world.run(Command::Undo).await.expect("undo");
            assert_eq!(
                (get(&world, work).await, get(&world, home).await),
                before,
                "both people exactly as they were"
            );
            assert_eq!(changed(&world.drained().await), Some(true));
        }

        #[tokio::test]
        async fn two_joins_in_a_burst_are_two_undos() {
            let world = world().await;
            let a = person(&world, "A", &["a@example.com"]).await;
            let b = person(&world, "B", &["b@example.com"]).await;
            let c = person(&world, "C", &["c@example.com"]).await;
            for other in [b, c] {
                world
                    .run(Command::ContactJoin {
                        action: ContactJoinAction::Join {
                            into: a,
                            others: vec![other],
                            name: "A".into(),
                            organization: None,
                        },
                    })
                    .await
                    .expect("join");
            }
            world.run(Command::Undo).await.expect("undo the second");
            assert_eq!(get(&world, c).await.expect("c").state, ContactState::Live);
            assert_eq!(
                get(&world, b).await.expect("b").state,
                ContactState::Merged,
                "the first join stands until it is undone too"
            );
        }

        #[tokio::test]
        async fn detaching_and_undoing_puts_the_address_back() {
            let world = world().await;
            let ada = person(&world, "Ada", &["ada@work.example", "ada@old.example"]).await;
            let old = get(&world, ada)
                .await
                .expect("ada")
                .addresses
                .into_iter()
                .find(|a| a.address.address == "ada@old.example")
                .expect("old")
                .id;

            world
                .run(Command::ContactDetachAddress { address: Some(old) })
                .await
                .expect("detach");
            assert_eq!(get(&world, ada).await.expect("ada").addresses.len(), 1);

            world.run(Command::Undo).await.expect("undo");
            assert_eq!(get(&world, ada).await.expect("ada").addresses.len(), 2);
        }

        #[tokio::test]
        async fn adding_an_address_and_undoing_lets_it_go() {
            let world = world().await;
            let ada = person(&world, "Ada", &["ada@work.example"]).await;

            world
                .run(Command::ContactAddAddress {
                    action: ContactAddressAction::Add {
                        person: ada,
                        address: EmailAddress::new(None::<String>, "ada@home.example"),
                    },
                })
                .await
                .expect("add");
            assert_eq!(get(&world, ada).await.expect("ada").addresses.len(), 2);

            world.run(Command::Undo).await.expect("undo");
            assert_eq!(get(&world, ada).await.expect("ada").addresses.len(), 1);
        }

        #[tokio::test]
        async fn an_owned_address_is_refused_by_add_and_moved_by_put() {
            let world = world().await;
            let ada = person(&world, "Ada", &["ada@example.com"]).await;
            let grace = person(&world, "Grace", &["grace@example.org", "g@example.org"]).await;
            let refused = world
                .run(Command::ContactAddAddress {
                    action: ContactAddressAction::Add {
                        person: ada,
                        address: EmailAddress::new(None::<String>, "grace@example.org"),
                    },
                })
                .await;
            assert!(refused.is_err(), "a live owner is asked, not overruled");

            let address = get(&world, grace)
                .await
                .expect("grace")
                .addresses
                .into_iter()
                .find(|a| a.address.address == "grace@example.org")
                .expect("address")
                .id;
            world
                .run(Command::ContactAddAddress {
                    action: ContactAddressAction::Put {
                        address,
                        to: Some(ada),
                        revive: None,
                    },
                })
                .await
                .expect("move it");
            assert_eq!(get(&world, ada).await.expect("ada").addresses.len(), 2);

            world.run(Command::Undo).await.expect("undo");
            assert_eq!(get(&world, grace).await.expect("grace").addresses.len(), 2);
        }

        #[tokio::test]
        async fn a_preferred_address_change_undoes() {
            let world = world().await;
            let ada = person(&world, "Ada", &["ada@work.example", "ada@home.example"]).await;
            let before = get(&world, ada).await.expect("ada").preferred;
            let other = get(&world, ada)
                .await
                .expect("ada")
                .addresses
                .into_iter()
                .find(|a| a.id != before)
                .expect("the other")
                .id;

            world
                .run(Command::ContactSetPreferred {
                    person: Some(ada),
                    address: Some(other),
                })
                .await
                .expect("prefer");
            assert_eq!(get(&world, ada).await.expect("ada").preferred, other);
            world.run(Command::Undo).await.expect("undo");
            assert_eq!(get(&world, ada).await.expect("ada").preferred, before);
        }

        #[tokio::test]
        async fn an_ask_never_reaches_the_store() {
            let world = world().await;
            assert!(
                world
                    .run(Command::ContactJoin {
                        action: ContactJoinAction::Ask
                    })
                    .await
                    .is_err()
            );
        }

        #[tokio::test]
        async fn making_a_person_and_undoing_leaves_no_one() {
            use postio_core::ContactNewAction;
            let world = world().await;
            world
                .run(Command::ContactNew {
                    action: ContactNewAction::Create {
                        name: Some("Grace Hopper".into()),
                        addresses: vec![EmailAddress::new(None::<String>, "grace@example.org")],
                    },
                })
                .await
                .expect("create");
            let events = world.drained().await;
            assert_eq!(changed(&events), Some(true), "a new name for an address");
            let connection = world.database.connect().await.expect("a connection");
            let contacts = ContactRepository::new(&connection);
            let grace = contacts
                .by_address("grace@example.org")
                .await
                .expect("lookup")
                .expect("grace");
            assert_eq!(grace.state, ContactState::Live);

            world.run(Command::Undo).await.expect("undo");
            assert!(
                contacts
                    .by_address("grace@example.org")
                    .await
                    .expect("lookup")
                    .is_none(),
                "the address belongs to nobody again"
            );
            assert!(
                contacts
                    .complete("gra", 8)
                    .await
                    .expect("complete")
                    .is_empty(),
                "and nobody is offered"
            );
        }

        #[tokio::test]
        async fn an_edit_undoes_to_exactly_what_was_there() {
            use postio_core::ContactEditAction;
            let world = world().await;
            let ada = person(&world, "Ada", &["ada@example.com"]).await;
            let before = get(&world, ada).await;
            world
                .run(Command::ContactEdit {
                    action: ContactEditAction::Edit {
                        person: ada,
                        edit: postio_model::PersonEdit {
                            name: Some("Ada Lovelace".into()),
                            organization: Some("Engines".into()),
                            note: None,
                        },
                    },
                })
                .await
                .expect("edit");
            assert_eq!(changed(&world.drained().await), Some(true));
            assert_eq!(
                get(&world, ada).await.expect("ada").name.as_deref(),
                Some("Ada Lovelace")
            );
            world.run(Command::Undo).await.expect("undo");
            assert_eq!(get(&world, ada).await, before);
        }

        #[tokio::test]
        async fn a_card_switch_applies_at_once_and_undoes() {
            use postio_core::ContactEditAction;
            use postio_model::CardSwitch;
            let world = world().await;
            let ada = person(&world, "Ada", &["ada@example.com"]).await;
            let notifies = |world: &World| {
                let database = world.database.clone();
                async move {
                    let reader = database.read().await.expect("a reader");
                    postio_storage::repository::ContactSettingsRepository::new(&reader)
                        .get(ada)
                        .await
                        .expect("settings")
                        .notify
                }
            };
            assert!(!notifies(&world).await);
            world
                .run(Command::ContactEdit {
                    action: ContactEditAction::Switch {
                        address: "ADA@example.com".into(),
                        switch: CardSwitch::Notify,
                        on: true,
                    },
                })
                .await
                .expect("a switch");
            assert!(notifies(&world).await);
            world.run(Command::Undo).await.expect("undo");
            assert!(!notifies(&world).await);
        }

        #[tokio::test]
        async fn adding_a_sender_keeps_the_signature_and_undo_puts_it_all_back() {
            use postio_core::ContactEditAction;
            use postio_model::SignatureDetails;
            let world = world().await;
            let jonas = person(&world, "Jonas", &["jonas@harbordata.example"]).await;
            world
                .run(Command::ContactEdit {
                    action: ContactEditAction::AddSender {
                        address: "JONAS@harbordata.example".into(),
                        details: SignatureDetails {
                            name: None,
                            title: Some("Partnerships".into()),
                            company: Some("Harbor Data Co.".into()),
                            phones: vec!["+1 555 0142".into()],
                            websites: Vec::new(),
                        },
                    },
                })
                .await
                .expect("added");
            let added = get(&world, jonas).await.expect("jonas");
            assert_eq!(added.title.as_deref(), Some("Partnerships"));
            world.run(Command::Undo).await.expect("undo");
            let undone = get(&world, jonas).await.expect("jonas");
            assert_eq!(undone.title, None);
            assert_eq!(undone.organization, None);
        }

        #[tokio::test]
        async fn ignoring_a_sender_is_a_setting_and_undo_offers_them_again() {
            use postio_core::ContactEditAction;
            let world = world().await;
            let jonas = person(&world, "Jonas", &["jonas@harbordata.example"]).await;
            let ignored = |world: &World| {
                let database = world.database.clone();
                async move {
                    let reader = database.read().await.expect("a reader");
                    postio_storage::repository::ContactSettingsRepository::new(&reader)
                        .get(jonas)
                        .await
                        .expect("settings")
                        .ignored
                }
            };
            world
                .run(Command::ContactEdit {
                    action: ContactEditAction::Ignore {
                        address: "jonas@harbordata.example".into(),
                        ignored: true,
                    },
                })
                .await
                .expect("ignored");
            assert!(ignored(&world).await);
            world.run(Command::Undo).await.expect("undo");
            assert!(!ignored(&world).await);
        }

        #[tokio::test]
        async fn a_saved_form_makes_a_person_and_each_edit_undoes_in_turn() {
            use postio_core::ContactEditAction;
            use postio_model::{ContactForm, ContactKind, FormAddress};
            let world = world().await;
            let mut form = ContactForm::blank();
            form.given = "Grace".into();
            form.family = "Oyelaran".into();
            form.addresses = vec![FormAddress {
                address: "grace@example.org".into(),
                ..FormAddress::blank()
            }];
            world
                .run(Command::ContactEdit {
                    action: ContactEditAction::Save {
                        person: None,
                        form: Box::new(form.clone()),
                    },
                })
                .await
                .expect("a new contact");
            assert_eq!(changed(&world.drained().await), Some(true));
            let database = world.database.clone();
            let connection = database.connect().await.expect("a connection");
            let contacts = ContactRepository::new(&connection);
            let grace = contacts
                .by_address("grace@example.org")
                .await
                .expect("lookup")
                .expect("made");
            assert_eq!(grace.display_name(), "Grace Oyelaran");

            form.kind = ContactKind::Company;
            form.title = "Founder".into();
            world
                .run(Command::ContactEdit {
                    action: ContactEditAction::Save {
                        person: Some(grace.id),
                        form: Box::new(form),
                    },
                })
                .await
                .expect("an edit");
            let edited = get(&world, grace.id).await.expect("grace");
            assert_eq!(edited.kind, ContactKind::Company);
            assert_eq!(edited.title.as_deref(), Some("Founder"));

            world.run(Command::Undo).await.expect("undo the edit");
            let undone = get(&world, grace.id).await.expect("grace");
            assert_eq!(undone.kind, ContactKind::Person);
            assert_eq!(undone.title, None);

            world.run(Command::Undo).await.expect("undo the making");
            assert_eq!(
                get(&world, grace.id).await.expect("kept").state,
                ContactState::Deleted
            );
        }

        #[tokio::test]
        async fn a_change_of_arrival_applies_at_once_and_undoes_to_what_was_there() {
            use postio_model::ContactArrival;
            let world = world().await;
            let ada = person(&world, "Ada", &["ada@example.com"]).await;
            let arrival = |world: &World| {
                let database = world.database.clone();
                async move {
                    let reader = database.read().await.expect("a reader");
                    postio_storage::repository::ContactSettingsRepository::new(&reader)
                        .get(ada)
                        .await
                        .expect("settings")
                        .arrival
                }
            };
            assert_eq!(arrival(&world).await, None);

            world
                .run(Command::ContactSetArrival {
                    address: "Ada@Example.com".into(),
                    arrival: Some(ContactArrival::Digest),
                })
                .await
                .expect("set");
            let events = world.drained().await;
            assert_eq!(arrival(&world).await, Some(ContactArrival::Digest));
            assert!(events.contains(&Event::ActionCompleted {
                description: "Changed where mail arrives".into(),
                undoable: true,
            }));

            world
                .run(Command::ContactSetArrival {
                    address: "ada@example.com".into(),
                    arrival: Some(ContactArrival::Filtered),
                })
                .await
                .expect("set again");
            world.run(Command::Undo).await.expect("undo");
            assert_eq!(arrival(&world).await, Some(ContactArrival::Digest));
            world.run(Command::Undo).await.expect("undo the first");
            assert_eq!(arrival(&world).await, None);

            let stranger = world
                .run(Command::ContactSetArrival {
                    address: "nobody@example.org".into(),
                    arrival: Some(ContactArrival::Inbox),
                })
                .await;
            assert!(stranger.is_err(), "an address the mail never saw is no one");
        }

        #[tokio::test]
        async fn a_delete_undoes_and_a_restore_undoes_too() {
            let world = world().await;
            let ada = person(&world, "Ada", &["ada@example.com"]).await;
            world
                .run(Command::ContactDelete {
                    person: Some(ada),
                    group: None,
                })
                .await
                .expect("delete");
            let events = world.drained().await;
            assert_eq!(changed(&events), Some(true), "her name leaves the mail");
            assert!(events.contains(&Event::ActionCompleted {
                description: "Deleted a contact".into(),
                undoable: true,
            }));
            assert_eq!(
                get(&world, ada).await.expect("ada").state,
                ContactState::Deleted
            );
            world.run(Command::Undo).await.expect("undo");
            assert_eq!(
                get(&world, ada).await.expect("ada").state,
                ContactState::Live
            );

            world
                .run(Command::ContactDelete {
                    person: Some(ada),
                    group: None,
                })
                .await
                .expect("delete again");
            world
                .run(Command::ContactRestore {
                    person: Some(ada),
                    state: None,
                })
                .await
                .expect("restore");
            assert_eq!(
                get(&world, ada).await.expect("ada").state,
                ContactState::Live
            );
            world.run(Command::Undo).await.expect("undo the restore");
            assert_eq!(
                get(&world, ada).await.expect("ada").state,
                ContactState::Deleted
            );
        }

        #[tokio::test]
        async fn a_dismissal_hides_the_pair_and_is_not_an_undo() {
            let world = world().await;
            let a = person(&world, "Ada Lovelace", &["ada@work.example"]).await;
            let b = person(&world, "Ada Lovelace", &["ada@home.example"]).await;
            world
                .run(Command::SuggestionDismiss { pair: Some((a, b)) })
                .await
                .expect("dismiss");
            let events = world.drained().await;
            assert_eq!(changed(&events), Some(false), "no name moved");
            assert!(
                !events
                    .iter()
                    .any(|e| matches!(e, Event::ActionCompleted { undoable: true, .. })),
                "nothing to take back: {events:?}"
            );
            let connection = world.database.connect().await.expect("a connection");
            assert!(
                ContactRepository::new(&connection)
                    .suggestions(10)
                    .await
                    .expect("suggestions")
                    .is_empty()
            );
        }

        #[tokio::test]
        async fn every_group_edit_undoes_exactly() {
            use postio_core::ContactGroupNewAction;
            use postio_storage::repository::ContactGroupRepository;
            let world = world().await;
            let ada = person(&world, "Ada", &["ada@example.com"]).await;
            let grace = person(&world, "Grace", &["grace@example.org"]).await;
            let connection = world.database.connect().await.expect("a connection");
            let groups = ContactGroupRepository::new(&connection);
            let members = async |name: &str| -> Option<Vec<ContactId>> {
                let group = groups
                    .list()
                    .await
                    .expect("list")
                    .into_iter()
                    .find(|g| g.name == name)?;
                Some(
                    groups
                        .members(group.id)
                        .await
                        .expect("members")
                        .iter()
                        .map(|p| p.id)
                        .collect(),
                )
            };

            world
                .run(Command::ContactGroupNew {
                    action: ContactGroupNewAction::Create {
                        name: "Family".into(),
                        members: vec![ada],
                    },
                })
                .await
                .expect("create");
            assert_eq!(changed(&world.drained().await), Some(false));
            assert_eq!(members("Family").await, Some(vec![ada]));
            let family = groups.list().await.expect("list")[0].id;

            world
                .run(Command::ContactGroupAdd {
                    group: Some(family),
                    people: vec![grace, ada],
                })
                .await
                .expect("add");
            world
                .run(Command::ContactGroupRename {
                    group: Some(family),
                    name: Some("Kin".into()),
                })
                .await
                .expect("rename");
            world
                .run(Command::ContactDelete {
                    person: None,
                    group: Some(family),
                })
                .await
                .expect("delete the group");
            assert_eq!(members("Kin").await, None);

            world.run(Command::Undo).await.expect("undo the delete");
            assert_eq!(members("Kin").await, Some(vec![ada, grace]));
            world.run(Command::Undo).await.expect("undo the rename");
            assert_eq!(members("Family").await, Some(vec![ada, grace]));
            world.run(Command::Undo).await.expect("undo the add");
            assert_eq!(
                members("Family").await,
                Some(vec![ada]),
                "only grace, whom the add put there, comes out"
            );
            world
                .run(Command::ContactGroupRemove {
                    group: Some(family),
                    people: vec![ada],
                })
                .await
                .expect("remove");
            assert_eq!(members("Family").await, Some(vec![]));
            world.run(Command::Undo).await.expect("undo the remove");
            assert_eq!(members("Family").await, Some(vec![ada]));
            world.run(Command::Undo).await.expect("undo the create");
            assert_eq!(members("Family").await, None);
        }
    }

    async fn threaded(
        world: &World,
        rfc: &str,
        references: &[&str],
        minutes: i64,
    ) -> (MessageId, postio_model::ThreadId) {
        let connection = world.database.connect().await.expect("a connection");
        let mut message = Message::new(
            world.account.id,
            world.inbox,
            Utc::now() + chrono::TimeDelta::minutes(minutes),
        );
        message.rfc_message_id = Some(postio_model::RfcMessageId::new(format!(
            "<{rfc}@example.com>"
        )));
        message.references = references
            .iter()
            .map(|id| postio_model::RfcMessageId::new(format!("<{id}@example.com>")))
            .collect();
        message.in_reply_to = message.references.last().cloned();
        message.subject = Some("Re: Harbor API".to_owned());
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message");
        let thread =
            postio_storage::repository::ThreadingRepository::new(&connection, world.account.id)
                .thread(&message)
                .await
                .expect("threaded")
                .thread_id;
        (id, thread)
    }

    async fn muted(world: &World, thread: postio_model::ThreadId) -> bool {
        let connection = world.database.connect().await.expect("a connection");
        !ThreadRepository::new(&connection)
            .muted(&[thread])
            .await
            .expect("a read")
            .is_empty()
    }

    #[tokio::test]
    async fn muting_a_thread_is_one_undo_and_says_the_thread_changed() {
        let world = world().await;
        let (_, thread) = threaded(&world, "m1", &[], 1).await;

        world
            .actions
            .mute_thread(thread, true, &world.sink)
            .await
            .expect("mute");

        assert!(muted(&world, thread).await);
        let events = world.drained().await;
        assert!(
            events.iter().any(|event| matches!(
                event,
                Event::ThreadChanged { thread: changed, .. } if *changed == thread
            )),
            "an open window re-reads the thread: {events:?}"
        );
        assert_eq!(completion(&events).await, Some(("Muted the thread", true)));
        assert_eq!(
            world.actions.peek_description().as_deref(),
            Some("Muted the thread")
        );

        world.run(Command::Undo).await.expect("undo");

        assert!(!muted(&world, thread).await, "one undo unmutes");
        assert_eq!(
            undone(&world.drained().await).as_deref(),
            Some("Muted the thread, undone")
        );
    }

    #[tokio::test]
    async fn the_bus_answers_mute_and_split() {
        let world = world().await;
        let bus = dispatcher(world.actions.clone()).await;
        let wired: Vec<_> = bus.wired().collect();
        for id in [CommandId::MuteThread, CommandId::SplitThread] {
            assert!(wired.contains(&id), "{id} is not on the bus");
        }
    }

    #[tokio::test]
    async fn a_posted_mute_is_one_undo_that_unmutes() {
        let world = world().await;
        let (_, thread) = threaded(&world, "m1", &[], 1).await;

        world
            .run(Command::MuteThread {
                thread: Some(thread),
                muted: true,
            })
            .await
            .expect("mute");

        assert!(muted(&world, thread).await);
        assert_eq!(
            world.actions.peek_description().as_deref(),
            Some("Muted the thread"),
            "the command left an undo behind"
        );
        world.run(Command::Undo).await.expect("undo");
        assert!(!muted(&world, thread).await, "one undo unmutes");
    }

    #[tokio::test]
    async fn a_posted_split_is_one_undo_that_rejoins() {
        let world = world().await;
        let (m1, original) = threaded(&world, "m1", &[], 1).await;
        let (m2, _) = threaded(&world, "m2", &["m1"], 2).await;

        world
            .run(Command::SplitThread {
                message: Some(m2),
                rejoin: None,
            })
            .await
            .expect("split");

        let connection = world.database.connect().await.expect("a connection");
        let members = ThreadRepository::new(&connection)
            .member_ids(original)
            .await
            .expect("members");
        assert_eq!(members, vec![m1]);
        assert!(
            world.actions.peek_description().is_some(),
            "an undo is waiting"
        );
        world.run(Command::Undo).await.expect("one undo");
        let members = ThreadRepository::new(&connection)
            .member_ids(original)
            .await
            .expect("members");
        assert_eq!(members, vec![m1, m2]);
    }

    #[tokio::test]
    async fn unmuting_is_undone_by_muting_again() {
        let world = world().await;
        let (_, thread) = threaded(&world, "m1", &[], 1).await;
        world
            .actions
            .mute_thread(thread, true, &world.sink)
            .await
            .expect("mute");
        world
            .actions
            .mute_thread(thread, false, &world.sink)
            .await
            .expect("unmute");
        assert!(!muted(&world, thread).await);

        world.run(Command::Undo).await.expect("undo");

        assert!(muted(&world, thread).await);
    }

    #[tokio::test]
    async fn one_undo_of_a_split_rejoins_the_messages_and_a_reply_follows_the_original() {
        let world = world().await;
        let (m1, original) = threaded(&world, "m1", &[], 1).await;
        let (m2, _) = threaded(&world, "m2", &["m1"], 2).await;
        let (m3, _) = threaded(&world, "m3", &["m1", "m2"], 3).await;
        let (m4, _) = threaded(&world, "m4", &["m1", "m2", "m3"], 4).await;

        let split = world
            .actions
            .split_thread(m3, &world.sink)
            .await
            .expect("split");

        assert_ne!(split, original);
        let events = world.drained().await;
        let changed: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                Event::ThreadChanged { thread, .. } => Some(*thread),
                _ => None,
            })
            .collect();
        assert!(
            changed.contains(&split) && changed.contains(&original),
            "{changed:?}"
        );
        let members = |thread| {
            let database = world.database.clone();
            async move {
                let connection = database.connect().await.expect("a connection");
                ThreadRepository::new(&connection)
                    .member_ids(thread)
                    .await
                    .expect("members")
            }
        };
        assert_eq!(members(split).await, vec![m3, m4]);
        assert_eq!(members(original).await, vec![m1, m2]);

        world.run(Command::Undo).await.expect("one undo");

        assert_eq!(
            members(original).await,
            vec![m1, m2, m3, m4],
            "all four are back"
        );
        let (_, joined) = threaded(&world, "m5", &["m1", "m2", "m3"], 5).await;
        assert_eq!(
            joined, original,
            "a reply to the split message follows the original"
        );
        assert_eq!(
            world.actions.peek_description(),
            None,
            "nothing more to take back"
        );
    }

    #[tokio::test]
    async fn the_first_message_cannot_be_split_off_and_nothing_is_recorded() {
        let world = world().await;
        let (m1, _) = threaded(&world, "m1", &[], 1).await;
        threaded(&world, "m2", &["m1"], 2).await;

        let refused = world.actions.split_thread(m1, &world.sink).await;

        assert!(
            matches!(refused, Err(CommandError::Rejected(_))),
            "{refused:?}"
        );
        assert_eq!(world.actions.peek_description(), None);
    }
}
