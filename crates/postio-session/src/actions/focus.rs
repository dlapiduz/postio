//! Postio Focus's own verbs, carried out like every other verb here. They live beside the rest
//! because undo replays an entry's inverse through `Actions::act` (docs/ARCHITECTURE.md §5).

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use postio_calendar::{Answer, Invitation, Method};
use postio_core::dispatch::CommandError;
use postio_core::undo::UndoKind;
use postio_core::{Command, MessageTarget};
use postio_model::listing::{InviteAnswer, MarkerKind};
use postio_model::{
    Account, AccountId, Draft, DraftKind, EmailAddress, Identity, MailboxId, MailboxRole, Message,
    MessageId, ThreadId,
};
use postio_storage::Connection;
use postio_storage::repository::{
    AccountRepository, DigestRepository, DraftRepository, FilterDecisionRepository, FilterReason,
    InviteState, MailboxRepository, MarkerRepository, ReminderRepository, ThreadRepository,
};

use super::{Actions, Aim, Applied, Destination, RSVP_WINDOW, mailbox_for, store_failure};

impl Actions {
    /// Wait for a reply in each targeted conversation until `at`, or with `None` stop waiting.
    ///
    /// One reminder stands per conversation; setting one moves it. The way back names each
    /// conversation's previous time, `None` where there was none. Local only, and repaints nothing.
    pub(super) async fn remind(
        &self,
        target: &MessageTarget,
        at: Option<DateTime<Utc>>,
    ) -> Result<Applied, CommandError> {
        let (mut connection, _permit) = self.connect().await?;
        let rows = match self.aim(&connection, target).await? {
            Aim::Rows(rows) => rows,
            Aim::Bulk(_) => {
                return Err(CommandError::rejected(
                    "Select the threads to be reminded about",
                ));
            }
        };
        let mut conversations: BTreeMap<ThreadId, MessageId> = BTreeMap::new();
        let mut account = None;
        for row in &rows {
            let Some(thread) = row.thread_id else {
                continue;
            };
            conversations.entry(thread).or_insert(row.id);
            account.get_or_insert(row.account_id);
        }
        let Some(account) = account else {
            return Err(CommandError::rejected(
                "That message is not in a thread yet, so there is nothing to wait on",
            ));
        };

        let now = Utc::now();
        let mut previous: BTreeMap<Option<DateTime<Utc>>, Vec<MessageId>> = BTreeMap::new();
        let mut surfaced = false;
        let transaction = connection.transaction().await.map_err(store_failure)?;
        {
            let reminders = ReminderRepository::new(&transaction);
            for (thread, anchor) in &conversations {
                let standing = reminders.standing(*thread).await.map_err(store_failure)?;
                surfaced |= standing
                    .as_ref()
                    .is_some_and(|reminder| reminder.fired_at.is_some());
                previous
                    .entry(standing.as_ref().map(|reminder| reminder.due_at))
                    .or_default()
                    .push(*anchor);
                match at {
                    Some(due) => {
                        reminders
                            .set(*thread, *anchor, due, now)
                            .await
                            .map_err(store_failure)?;
                    }
                    None => {
                        reminders.clear(*thread).await.map_err(store_failure)?;
                    }
                }
            }
        }
        transaction.commit().await.map_err(store_failure)?;

        let anchors: Vec<MessageId> = conversations.into_values().collect();
        Ok(Applied {
            contacts_changed: None,
            rows: None,
            lasts: None,
            surfaced,
            account,
            kind: if at.is_some() {
                UndoKind::Remind
            } else {
                UndoKind::Unremind
            },
            count: anchors.len(),
            messages: anchors,
            removed: Vec::new(),
            arrived: None,
            reloaded: Vec::new(),
            changed: Vec::new(),
            mailboxes_changed: false,
            inverse: previous
                .into_iter()
                .map(|(at, anchors)| Command::RemindIfNoReply {
                    target: MessageTarget::Messages(anchors),
                    at,
                })
                .collect(),
        })
    }

    /// Answer the invitation `message` carries, or the focused message's, and send the reply
    /// (research R9).
    ///
    /// The reply is written as a draft queued with a not-before time `RSVP_WINDOW` away, so nothing
    /// reaches the network before the window closes and the undo entry (cancel the send) lasts
    /// exactly that long. It goes out as the identity the invitation names among its attendees,
    /// spelled as the invitation spells it.
    pub(super) async fn answer(
        &self,
        message: Option<MessageId>,
        answer: Answer,
    ) -> Result<Applied, CommandError> {
        let blobs = self
            .blobs
            .clone()
            .ok_or_else(|| CommandError::rejected("Answering an invitation needs Postio Focus"))?;
        let (mut connection, _permit) = self.connect().await?;
        let row = match message {
            Some(id) => self.rows(&connection, vec![id]).await?.remove(0),
            None => match self.aim(&connection, &MessageTarget::Selection).await? {
                Aim::Rows(mut rows) => rows.remove(0),
                Aim::Bulk(_) => {
                    return Err(CommandError::rejected("Pick the invitation to answer"));
                }
            },
        };
        let marker = MarkerRepository::new(&connection)
            .get(row.id)
            .await
            .map_err(store_failure)?
            .filter(|marker| marker.kind == MarkerKind::Invite)
            .ok_or_else(|| {
                CommandError::rejected("That message carries no invitation to answer")
            })?;
        match marker.invite_state {
            Some(InviteState::Cancelled) => {
                return Err(CommandError::rejected(
                    "That event was cancelled, so there is nothing to answer",
                ));
            }
            Some(InviteState::Past) => {
                return Err(CommandError::rejected(
                    "That event is over, so there is nothing to answer",
                ));
            }
            Some(InviteState::Open) | None => {}
        }
        if let Some(given) = marker.answer {
            return Err(CommandError::rejected(match given {
                InviteAnswer::Accepting | InviteAnswer::Declining => {
                    "Your answer is on its way; undo takes it back while it waits"
                }
                InviteAnswer::Accepted => "You accepted that invitation already",
                InviteAnswer::Declined => "You declined that invitation already",
            }));
        }
        let (invitation, object) = invitation_of(&blobs, &row)
            .await
            .filter(|(invitation, _)| invitation.method == Method::Request)
            .ok_or_else(|| {
                CommandError::rejected("Postio does not have that invitation's calendar yet")
            })?;
        let account = AccountRepository::new(&connection)
            .get(row.account_id)
            .await
            .map_err(store_failure)?
            .ok_or_else(|| CommandError::rejected("That message's account is no longer here"))?;
        let Some((identity, attendee)) = attendee_among(&account, &invitation) else {
            return Err(CommandError::rejected(
                "That invitation does not name any address you send from, so there is \
                 nobody to answer as",
            ));
        };
        let organizer = invitation.organizer.clone().ok_or_else(|| {
            CommandError::rejected("That invitation names no organiser to answer")
        })?;

        let (kind, pending, subject, said) = match answer {
            Answer::Accept => (
                UndoKind::Accept,
                InviteAnswer::Accepting,
                "Accepted",
                "accepted",
            ),
            Answer::Decline => (
                UndoKind::Decline,
                InviteAnswer::Declining,
                "Declined",
                "declined",
            ),
        };
        let ics = postio_calendar::reply(&invitation, &attendee, answer);
        let mut draft = Draft::new(account.id);
        draft.use_identity(identity);
        draft.kind = DraftKind::Reply;
        draft.in_reply_to = Some(row.id);
        draft.thread_id = row.thread_id;
        draft.to = vec![organizer];
        let event = invitation
            .summary
            .clone()
            .or_else(|| row.subject.clone())
            .unwrap_or_default();
        draft.subject = format!("{subject}: {event}");
        let who = if identity.display_name.trim().is_empty() {
            identity.address.address.clone()
        } else {
            identity.display_name.clone()
        };
        draft.body.text = Some(format!("{who} has {said} this invitation."));
        draft.calendar_reply = Some(String::from_utf8_lossy(&ics).into_owned());

        let now = Utc::now();
        let until = now
            + chrono::Duration::from_std(RSVP_WINDOW).map_err(|_| {
                CommandError::rejected("That answer's window is longer than Postio can hold")
            })?;
        let transaction = connection.transaction().await.map_err(store_failure)?;
        DraftRepository::new(&transaction)
            .queue_send_at(&mut draft, now, until)
            .await
            .map_err(store_failure)?;
        MarkerRepository::new(&transaction)
            .answer(row.id, Some(pending), Some(until))
            .await
            .map_err(store_failure)?;
        transaction.commit().await.map_err(store_failure)?;
        self.put_answered(&account, row.id, &object, answer).await;

        Ok(Applied {
            contacts_changed: None,
            rows: None,
            lasts: Some(RSVP_WINDOW),
            surfaced: false,
            account: account.id,
            kind,
            count: 1,
            messages: vec![row.id],
            removed: Vec::new(),
            arrived: None,
            reloaded: Vec::new(),
            changed: vec![row.id],
            mailboxes_changed: true,
            inverse: vec![Command::CancelSend {
                draft: Some(draft.id),
            }],
        })
    }

    /// When `draft`, whose send was just cancelled, is a reply to an invitation, take the answer
    /// back whole. Returns the messages whose rows repaint.
    pub(super) async fn withdraw_answer(
        &self,
        connection: &Connection,
        draft: &Draft,
    ) -> Result<Vec<MessageId>, CommandError> {
        if draft.calendar_reply.is_none() {
            return Ok(Vec::new());
        }
        DraftRepository::new(connection)
            .discard(draft.id, Utc::now())
            .await
            .map_err(store_failure)?;
        let Some(invitation) = draft.in_reply_to else {
            return Ok(Vec::new());
        };
        MarkerRepository::new(connection)
            .answer(invitation, None, None)
            .await
            .map_err(store_failure)?;
        postio_storage::repository::CalendarRepository::new(connection)
            .remove_answered(invitation)
            .await
            .map_err(store_failure)?;
        Ok(vec![invitation])
    }

    async fn put_answered(
        &self,
        account: &Account,
        message: MessageId,
        object: &[u8],
        answer: Answer,
    ) {
        let partstat = match answer {
            Answer::Accept => "ACCEPTED",
            Answer::Decline => "DECLINED",
        };
        let placed = async {
            let connection = self.database.connect().await?;
            let calendars = postio_storage::repository::CalendarRepository::new(&connection);
            let calendar = match calendars.default_for(account.id).await? {
                Some(calendar) => calendar,
                None => calendars.local().await?.id,
            };
            Ok::<_, postio_storage::Error>(calendar)
        }
        .await;
        let Ok(calendar) = placed else {
            tracing::warn!(
                message = message.get(),
                "no calendar for an answered invitation"
            );
            return;
        };
        let stored = crate::calendar::store_object(
            &self.database,
            crate::calendar::NewObject {
                calendar,
                ics: object,
                href: None,
                etag: None,
                my_partstat: Some(partstat.to_owned()),
                source_message: Some(message),
            },
            &chrono::Local,
            Utc::now(),
        )
        .await;
        if stored.is_err() {
            tracing::warn!(
                message = message.get(),
                "an answered invitation's event was not stored"
            );
        }
    }
}

/// How many markers of one kind dismissed in one sender's mail make Focus take it as a correction.
pub const STOPS_AFTER: u32 = 3;

impl Actions {
    /// Take the markers off the targeted messages as wrong, or bring them back.
    ///
    /// Dismissing `STOPS_AFTER` markers of one kind in one sender's mail writes `[focus.filter]
    /// stop_markers`; bringing them back undoes it when the count drops below. Only question and
    /// to-do markers teach. The correction lives in the file, not the store; if the file cannot be
    /// written the dismissal stands.
    pub(super) async fn dismiss(
        &self,
        target: &MessageTarget,
        dismissed: bool,
    ) -> Result<Applied, CommandError> {
        let (mut connection, _permit) = self.connect().await?;
        let rows = match self.aim(&connection, target).await? {
            Aim::Rows(rows) => rows,
            Aim::Bulk(_) => {
                return Err(CommandError::rejected(
                    "Select the messages whose markers to dismiss",
                ));
            }
        };
        let now = Utc::now();
        let mut touched = Vec::new();
        let mut account = None;
        let mut taught: BTreeMap<(String, &'static str), u32> = BTreeMap::new();
        let transaction = connection.transaction().await.map_err(store_failure)?;
        {
            let markers = MarkerRepository::new(&transaction);
            for row in &rows {
                let Some(marker) = markers.get(row.id).await.map_err(store_failure)? else {
                    continue;
                };
                if marker.dismissed_at.is_some() == dismissed {
                    continue;
                }
                markers
                    .dismiss(row.id, dismissed.then_some(now))
                    .await
                    .map_err(store_failure)?;
                touched.push(row.id);
                account.get_or_insert(row.account_id);
                if let Some(kind) = teaches(marker.kind) {
                    for sender in &row.from {
                        *taught.entry((sender.normalized(), kind)).or_default() += 1;
                    }
                }
            }
        }
        transaction.commit().await.map_err(store_failure)?;
        let Some(account) = account else {
            return Err(CommandError::rejected(if dismissed {
                "There is no marker there to dismiss"
            } else {
                "There is no dismissed marker there to bring back"
            }));
        };

        if self.focus.is_on() {
            for ((sender, kind), changed) in taught {
                let now_dismissed = MarkerRepository::new(&connection)
                    .dismissed_from(&sender, marker_kind(kind))
                    .await
                    .map_err(store_failure)?;
                let present = if dismissed {
                    now_dismissed >= STOPS_AFTER
                } else if now_dismissed < STOPS_AFTER && now_dismissed + changed >= STOPS_AFTER {
                    false
                } else {
                    continue;
                };
                if !dismissed || present {
                    let written = self
                        .focus
                        .stop_marker(&self.database, &sender, kind, present)
                        .await;
                    if let Err(reason) = written {
                        tracing::warn!(
                            %reason,
                            "Focus could not write a marker correction: {reason}"
                        );
                    }
                }
            }
        }

        Ok(Applied {
            contacts_changed: None,
            rows: None,
            lasts: None,
            surfaced: false,
            account,
            kind: if dismissed {
                UndoKind::DismissMarker
            } else {
                UndoKind::UndismissMarker
            },
            count: touched.len(),
            messages: touched.clone(),
            removed: Vec::new(),
            arrived: None,
            reloaded: Vec::new(),
            changed: touched.clone(),
            mailboxes_changed: false,
            inverse: vec![Command::DismissMarker {
                target: MessageTarget::Messages(touched),
                dismissed: !dismissed,
            }],
        })
    }
}

impl Actions {
    /// Put filtered messages back in the inbox and never filter their senders again, or with
    /// `restored` false file them away again.
    ///
    /// One undo takes back all three: the move to the inbox, the decision marked restored (not
    /// deleted, SC-012), and the sender added to `[focus.filter] never`. A sender is unpinned on
    /// the way back only when no other restore of theirs stands behind the pin.
    pub(super) async fn restore(
        &self,
        target: &MessageTarget,
        restored: bool,
    ) -> Result<Vec<Applied>, CommandError> {
        let (mut connection, _permit) = self.connect().await?;
        let rows = match self.aim(&connection, target).await? {
            Aim::Rows(rows) => rows,
            Aim::Bulk(_) => {
                return Err(CommandError::rejected("Select the messages to restore"));
            }
        };
        let mut by_account: BTreeMap<AccountId, Vec<Message>> = BTreeMap::new();
        for row in rows {
            by_account.entry(row.account_id).or_default().push(row);
        }
        let to = if restored {
            Destination::Role(MailboxRole::Inbox)
        } else {
            Destination::Role(MailboxRole::Archive)
        };
        let mut destinations = BTreeMap::new();
        for account in by_account.keys() {
            destinations.insert(*account, mailbox_for(&connection, *account, to).await?);
        }

        let now = Utc::now();
        let mut applied = Vec::new();
        let mut senders: BTreeSet<String> = BTreeSet::new();
        for (account, rows) in by_account {
            let destination = destinations[&account];
            let mut moved: BTreeMap<MailboxId, Vec<MessageId>> = BTreeMap::new();
            let mut ids = Vec::new();
            let transaction = connection.transaction().await.map_err(store_failure)?;
            {
                let decisions = FilterDecisionRepository::new(&transaction);
                // What the account's other machines are told (#2076).
                let mut marking: BTreeMap<Option<FilterReason>, Vec<&Message>> = BTreeMap::new();
                for row in &rows {
                    if !decisions
                        .restore(row.id, restored.then_some(now))
                        .await
                        .map_err(store_failure)?
                    {
                        continue;
                    }
                    let reason = if restored {
                        None
                    } else {
                        decisions
                            .get(row.id)
                            .await
                            .map_err(store_failure)?
                            .map(|decision| decision.reason)
                    };
                    marking.entry(reason).or_default().push(row);
                    ids.push(row.id);
                    senders.extend(row.from.iter().map(EmailAddress::normalized));
                    if row.mailbox_id != destination {
                        moved.entry(row.mailbox_id).or_default().push(row.id);
                    }
                }
                // Before the move, which clears what the server knows the message by.
                for (reason, rows) in &marking {
                    postio_storage::actions::mark_filtered(
                        &transaction,
                        account,
                        rows,
                        *reason,
                        now,
                    )
                    .await
                    .map_err(store_failure)?;
                }
                if !moved.is_empty() {
                    postio_storage::actions::relocate(
                        &transaction,
                        account,
                        &moved,
                        destination,
                        postio_storage::actions::Relocation::Move,
                        now,
                    )
                    .await
                    .map_err(store_failure)?;
                }
            }
            transaction.commit().await.map_err(store_failure)?;
            if ids.is_empty() {
                continue;
            }
            applied.push(Applied {
                contacts_changed: None,
                rows: None,
                lasts: None,
                surfaced: false,
                account,
                kind: if restored {
                    UndoKind::Restore
                } else {
                    UndoKind::Refilter
                },
                count: ids.len(),
                messages: ids.clone(),
                removed: moved.into_iter().collect(),
                arrived: Some(destination),
                reloaded: Vec::new(),
                changed: Vec::new(),
                mailboxes_changed: false,
                inverse: vec![Command::RestoreFiltered {
                    target: MessageTarget::Messages(ids),
                    restored: !restored,
                }],
            });
        }
        if applied.is_empty() {
            return Err(CommandError::rejected(if restored {
                "Those messages are not in Filtered"
            } else {
                "Those messages were not restored from Filtered"
            }));
        }

        if self.focus.is_on() {
            for sender in senders {
                if !restored
                    && FilterDecisionRepository::new(&connection)
                        .restored_from(&sender)
                        .await
                        .map_err(store_failure)?
                        > 0
                {
                    continue;
                }
                let written = self.focus.pin(&self.database, &sender, restored).await;
                if let Err(reason) = written {
                    tracing::warn!(%reason, "Focus could not write a restored sender: {reason}");
                }
            }
        }
        Ok(applied)
    }
}

impl Actions {
    /// File away what is already in every inbox, by Focus's filtering rules (FR-118); it moves what
    /// `sweep_preview` counted.
    ///
    /// A window at a time, each its own transaction and write permit, so a large inbox never holds
    /// the writer long; one undo unit for all of it.
    pub(super) async fn sweep(&self) -> Result<Vec<Applied>, CommandError> {
        let config = self
            .focus
            .config()
            .ok_or_else(|| CommandError::rejected("Filtering the inbox needs Postio Focus"))?;
        let pass = postio_sync::FocusFiling::sweeping(&config);
        let inboxes = {
            let reader = self.database.read().await.map_err(store_failure)?;
            ThreadRepository::new(&reader)
                .unified_inboxes()
                .await
                .map_err(store_failure)?
        };
        let mut applied = Vec::new();
        for (account, inbox) in inboxes {
            let mut moved = Vec::new();
            let mut archive = None;
            let mut after = None;
            loop {
                let (mut connection, _permit) = self.connect().await?;
                let rows = crate::focus::sweep_window(&connection, inbox, &mut after)
                    .await
                    .map_err(store_failure)?;
                if rows.is_empty() {
                    break;
                }
                if archive.is_none() {
                    archive = MailboxRepository::new(&connection)
                        .by_role(account, MailboxRole::Archive)
                        .await
                        .map_err(store_failure)?
                        .map(|mailbox| mailbox.id);
                }
                let transaction = connection.transaction().await.map_err(store_failure)?;
                let effects = pass
                    .file_away(&transaction, &crate::focus::in_the_inbox(&rows))
                    .await
                    .map_err(|_| CommandError::failed("Postio could not filter the inbox"))?;
                transaction.commit().await.map_err(store_failure)?;
                moved.extend(effects.filtered);
            }
            let Some(archive) = archive.filter(|_| !moved.is_empty()) else {
                continue;
            };
            applied.push(Applied {
                contacts_changed: None,
                rows: None,
                lasts: None,
                surfaced: false,
                account,
                kind: UndoKind::Sweep,
                count: moved.len(),
                messages: moved.clone(),
                removed: vec![(inbox, moved.clone())],
                arrived: Some(archive),
                reloaded: Vec::new(),
                changed: Vec::new(),
                mailboxes_changed: false,
                inverse: vec![Command::UnsweepInbox {
                    target: MessageTarget::Messages(moved),
                }],
            });
        }
        if applied.is_empty() {
            return Err(CommandError::rejected(
                "Nothing in the inbox would be filtered",
            ));
        }
        Ok(applied)
    }

    pub(super) async fn unsweep(
        &self,
        target: &MessageTarget,
    ) -> Result<Vec<Applied>, CommandError> {
        let (mut connection, _permit) = self.connect().await?;
        let rows = match self.aim(&connection, target).await? {
            Aim::Rows(rows) => rows,
            Aim::Bulk(_) => return Err(CommandError::rejected("Nothing to put back")),
        };
        let mut by_account: BTreeMap<AccountId, Vec<Message>> = BTreeMap::new();
        for row in rows {
            by_account.entry(row.account_id).or_default().push(row);
        }
        let now = Utc::now();
        let mut applied = Vec::new();
        for (account, rows) in by_account {
            let inbox =
                mailbox_for(&connection, account, Destination::Role(MailboxRole::Inbox)).await?;
            let mut moved: BTreeMap<MailboxId, Vec<MessageId>> = BTreeMap::new();
            let transaction = connection.transaction().await.map_err(store_failure)?;
            {
                let decisions = FilterDecisionRepository::new(&transaction);
                for row in &rows {
                    decisions.delete(row.id).await.map_err(store_failure)?;
                    if row.mailbox_id != inbox {
                        moved.entry(row.mailbox_id).or_default().push(row.id);
                    }
                }
                // Unmarked for the account's other machines too (#2076), before the move clears
                // what the server knows it by.
                let unmarked: Vec<&Message> = rows.iter().collect();
                postio_storage::actions::mark_filtered(&transaction, account, &unmarked, None, now)
                    .await
                    .map_err(store_failure)?;
                if !moved.is_empty() {
                    postio_storage::actions::relocate(
                        &transaction,
                        account,
                        &moved,
                        inbox,
                        postio_storage::actions::Relocation::Move,
                        now,
                    )
                    .await
                    .map_err(store_failure)?;
                }
            }
            transaction.commit().await.map_err(store_failure)?;
            let ids: Vec<MessageId> = rows.iter().map(|row| row.id).collect();
            applied.push(Applied {
                contacts_changed: None,
                rows: None,
                lasts: None,
                surfaced: false,
                account,
                kind: UndoKind::Sweep,
                count: ids.len(),
                messages: ids,
                removed: moved.into_iter().collect(),
                arrived: Some(inbox),
                reloaded: Vec::new(),
                changed: Vec::new(),
                mailboxes_changed: false,
                inverse: Vec::new(),
            });
        }
        Ok(applied)
    }
}

impl Actions {
    /// Archive every message a digest holds and the digest's row, or with `archived` false put the
    /// row back. One undo unit.
    pub(super) async fn archive_digest(
        &self,
        delivery: postio_model::DeliveryId,
        archived: bool,
    ) -> Result<Vec<Applied>, CommandError> {
        // Its own connection and permit, released before the move takes its own: the write gate is
        // not re-entrant.
        let messages = {
            let (connection, _permit) = self.connect().await?;
            let digests = DigestRepository::new(&connection);
            if !archived {
                digests
                    .reopen_delivery(delivery)
                    .await
                    .map_err(store_failure)?;
                return Ok(Vec::new());
            }
            digests
                .delivery_messages(delivery)
                .await
                .map_err(store_failure)?
        };
        let mut applied = if messages.is_empty() {
            Vec::new()
        } else {
            match self
                .relocate(
                    &MessageTarget::Messages(messages),
                    Destination::Role(MailboxRole::Archive),
                    UndoKind::Archive,
                )
                .await
            {
                Ok(applied) => applied,
                Err(CommandError::Rejected(_)) => Vec::new(),
                Err(error) => return Err(error),
            }
        };
        let (connection, _permit) = self.connect().await?;
        if !DigestRepository::new(&connection)
            .archive_delivery(delivery, Utc::now())
            .await
            .map_err(store_failure)?
        {
            return Err(CommandError::rejected("That digest is no longer here"));
        }
        match applied.last_mut() {
            Some(last) => {
                last.inverse.push(Command::ArchiveDigest {
                    delivery,
                    archived: false,
                });
                for unit in &mut applied {
                    unit.surfaced = true;
                }
            }
            None => {
                return Err(CommandError::rejected(
                    "That digest's messages are archived already",
                ));
            }
        }
        Ok(applied)
    }

    /// Stop gathering the targeted message's sender into its digest, or with `stopped` false put
    /// them back.
    ///
    /// The sender's `from:` query leaves the rule in `config.toml` (a rule holding nobody is
    /// removed); undo rewrites it exactly via `KeptRule`.
    pub(super) async fn stop_digesting(
        &self,
        target: &MessageTarget,
        stopped: bool,
        kept: Option<&postio_core::KeptRule>,
    ) -> Result<Applied, CommandError> {
        if !self.focus.is_on() {
            return Err(CommandError::rejected("Digests need Postio Focus"));
        }
        let (connection, _permit) = self.connect().await?;
        let rows = match self.aim(&connection, target).await? {
            Aim::Rows(rows) => rows,
            Aim::Bulk(_) => {
                return Err(CommandError::rejected("Pick a message from the sender"));
            }
        };
        let account = rows[0].account_id;
        if !stopped {
            let kept = kept.ok_or_else(|| {
                CommandError::rejected("There is no rule to put the sender back in")
            })?;
            let rule: postio_config::DigestRule = toml::from_str(&kept.toml)
                .map_err(|_| CommandError::failed("Postio could not read the rule it kept"))?;
            let name = rule.name.clone();
            let position = kept.position as usize;
            self.focus
                .put_digest(
                    &self.database,
                    Some(rule.name.as_str()),
                    &rule,
                    Some(position),
                )
                .await
                .map_err(CommandError::rejected)?;
            let now = Utc::now();
            let digests = DigestRepository::new(&connection);
            for row in &rows {
                digests
                    .hold(row.id, &name, now)
                    .await
                    .map_err(store_failure)?;
            }
            return Ok(Applied {
                contacts_changed: None,
                rows: None,
                lasts: None,
                surfaced: false,
                account,
                kind: UndoKind::ResumeDigesting,
                count: 1,
                messages: Vec::new(),
                removed: Vec::new(),
                arrived: None,
                reloaded: inboxes_of(&rows),
                changed: Vec::new(),
                mailboxes_changed: false,
                inverse: Vec::new(),
            });
        }

        let sender = rows[0]
            .from
            .first()
            .map(EmailAddress::normalized)
            .ok_or_else(|| CommandError::rejected("That message has no sender to stop"))?;
        let config = self
            .focus
            .config()
            .ok_or_else(|| CommandError::rejected("Digests need Postio Focus"))?;
        let Some((position, rule)) = config.digests.iter().enumerate().find(|(_, rule)| {
            rule.queries
                .iter()
                .any(|query| names_sender(query, &sender))
        }) else {
            return Err(CommandError::rejected(
                "That sender is not in a digest rule",
            ));
        };
        let kept = postio_core::KeptRule {
            position: u32::try_from(position).unwrap_or(u32::MAX),
            toml: toml::to_string(rule)
                .map_err(|_| CommandError::failed("Postio could not keep the rule to put back"))?,
        };
        let name = rule.name.clone();
        let mut narrowed = rule.clone();
        narrowed
            .queries
            .retain(|query| !names_sender(query, &sender));
        let theirs = config.contact_digest(&narrowed.name).is_some();
        if narrowed.queries.is_empty() || theirs {
            self.focus
                .remove_digest(&self.database, &narrowed.name)
                .await
                .map_err(CommandError::rejected)?;
        } else {
            self.focus
                .put_digest(
                    &self.database,
                    Some(narrowed.name.as_str()),
                    &narrowed,
                    None,
                )
                .await
                .map_err(CommandError::rejected)?;
        }
        let released = DigestRepository::new(&connection)
            .release_sender(&name, &sender)
            .await
            .map_err(store_failure)?;
        let back = if released.is_empty() {
            rows.iter().map(|row| row.id).collect()
        } else {
            released
        };
        Ok(Applied {
            contacts_changed: None,
            rows: None,
            lasts: None,
            surfaced: false,
            account,
            kind: UndoKind::StopDigesting,
            count: 1,
            messages: Vec::new(),
            removed: Vec::new(),
            arrived: None,
            reloaded: inboxes_of(&rows),
            changed: Vec::new(),
            mailboxes_changed: false,
            inverse: vec![Command::StopDigestingSender {
                target: MessageTarget::Messages(back),
                stopped: false,
                kept: Some(kept),
            }],
        })
    }
}

fn names_sender(query: &str, sender: &str) -> bool {
    let query = query.trim();
    let Some(prefix) = query.get(..5) else {
        return false;
    };
    prefix.eq_ignore_ascii_case("from:")
        && query[5..]
            .trim()
            .trim_matches('"')
            .eq_ignore_ascii_case(sender)
}

fn inboxes_of(rows: &[Message]) -> Vec<MailboxId> {
    let mut mailboxes: Vec<MailboxId> = rows.iter().map(|row| row.mailbox_id).collect();
    mailboxes.sort_unstable();
    mailboxes.dedup();
    mailboxes
}

/// The name `[focus.filter] stop_markers` gives a marker kind whose dismissals teach; invitation
/// and reminder markers say nothing about what the sender's mail asks.
fn teaches(kind: MarkerKind) -> Option<&'static str> {
    match kind {
        MarkerKind::Question => Some("question"),
        MarkerKind::Todo => Some("todo"),
        MarkerKind::Invite | MarkerKind::NoReply => None,
    }
}

fn marker_kind(name: &str) -> MarkerKind {
    if name == "todo" {
        MarkerKind::Todo
    } else {
        MarkerKind::Question
    }
}

/// The invitation `row`'s calendar part carries, when the part is on this machine and is a request.
/// Read off the async runtime's threads.
pub async fn invitation_of(
    blobs: &postio_storage::BlobStore,
    row: &Message,
) -> Option<(Invitation, Vec<u8>)> {
    let parts: Vec<_> = row
        .attachments
        .iter()
        .filter(|part| part.mime_type.eq_ignore_ascii_case("text/calendar"))
        .filter_map(|part| part.blob_id.clone())
        .collect();
    if parts.is_empty() {
        return None;
    }
    let blobs = blobs.clone();
    tokio::task::spawn_blocking(move || {
        parts.iter().find_map(|part| {
            let ics = blobs.get(part).ok()?;
            postio_calendar::parse(&ics)
                .ok()
                .filter(|invitation| matches!(invitation.method, Method::Request | Method::Cancel))
                .map(|invitation| (invitation, ics))
        })
    })
    .await
    .ok()
    .flatten()
}

fn attendee_among<'a>(
    account: &'a Account,
    invitation: &Invitation,
) -> Option<(&'a Identity, EmailAddress)> {
    invitation.attendees.iter().find_map(|attendee| {
        account
            .identities
            .iter()
            .find(|identity| identity.address.same_address(&attendee.address))
            .map(|identity| (identity, attendee.address.clone()))
    })
}

impl Actions {
    pub async fn mute_thread(
        &self,
        thread: ThreadId,
        muted: bool,
        events: &postio_core::bridge::EventSink,
    ) -> Result<(), CommandError> {
        let applied = self.mute(thread, muted, events).await?;
        self.announce(applied, events, super::Recording::Record)
            .await;
        Ok(())
    }

    pub async fn split_thread(
        &self,
        message: MessageId,
        events: &postio_core::bridge::EventSink,
    ) -> Result<ThreadId, CommandError> {
        let (applied, split) = self.split(message, events).await?;
        self.announce(applied, events, super::Recording::Record)
            .await;
        Ok(split)
    }

    pub(super) async fn mute(
        &self,
        thread: ThreadId,
        muted: bool,
        events: &postio_core::bridge::EventSink,
    ) -> Result<Applied, CommandError> {
        let (connection, _permit) = self.connect().await?;
        let threads = ThreadRepository::new(&connection);
        let account = threads
            .account_of(thread)
            .await
            .map_err(store_failure)?
            .ok_or_else(|| CommandError::rejected("That thread is gone"))?;
        let done = if muted {
            threads.mute(thread).await
        } else {
            threads.unmute(thread).await
        };
        done.map_err(|error| match error {
            postio_storage::Error::ForbiddenTransition { .. } => {
                CommandError::rejected("That thread has no message id to remember a mute by")
            }
            other => store_failure(other),
        })?;
        drop(connection);
        events.emit(postio_core::Event::ThreadChanged {
            account,
            thread,
            members_added: Vec::new(),
        });
        tracing::debug!(thread = thread.get(), muted, "a thread's mute changed");
        Ok(thread_applied(
            if muted {
                UndoKind::Mute
            } else {
                UndoKind::Unmute
            },
            account,
            Vec::new(),
            vec![Command::MuteThread {
                thread: Some(thread),
                muted: !muted,
            }],
        ))
    }

    pub(super) async fn split(
        &self,
        message: MessageId,
        events: &postio_core::bridge::EventSink,
    ) -> Result<(Applied, ThreadId), CommandError> {
        let (connection, _permit) = self.connect().await?;
        let threads = ThreadRepository::new(&connection);
        let original = threads
            .thread_id_of(message)
            .await
            .map_err(store_failure)?
            .ok_or_else(|| CommandError::rejected("That message is not in a thread yet"))?;
        let account = threads
            .account_of(original)
            .await
            .map_err(store_failure)?
            .ok_or_else(|| CommandError::rejected("That thread is gone"))?;
        let split = threads.split(message).await.map_err(|error| match error {
            postio_storage::Error::ForbiddenTransition { reason, .. } => {
                CommandError::rejected(format!("That message cannot be split off: {reason}"))
            }
            other => store_failure(other),
        })?;
        drop(connection);
        for thread in [original, split] {
            events.emit(postio_core::Event::ThreadChanged {
                account,
                thread,
                members_added: Vec::new(),
            });
        }
        tracing::debug!(
            split_from = original.get(),
            thread = split.get(),
            "a message was split into a thread of its own"
        );
        Ok((
            thread_applied(
                UndoKind::Split,
                account,
                vec![message],
                vec![Command::SplitThread {
                    message: None,
                    rejoin: Some((split, original)),
                }],
            ),
            split,
        ))
    }

    pub(super) async fn unsplit(
        &self,
        split: ThreadId,
        original: ThreadId,
        events: &postio_core::bridge::EventSink,
    ) -> Result<Applied, CommandError> {
        let (connection, _permit) = self.connect().await?;
        let threads = ThreadRepository::new(&connection);
        let account = threads
            .account_of(original)
            .await
            .map_err(store_failure)?
            .ok_or_else(|| CommandError::rejected("That thread is gone"))?;
        let moved = threads.member_ids(split).await.map_err(store_failure)?;
        threads
            .unsplit(split, original)
            .await
            .map_err(store_failure)?;
        drop(connection);
        events.emit(postio_core::Event::ThreadChanged {
            account,
            thread: original,
            members_added: Vec::new(),
        });
        Ok(thread_applied(
            UndoKind::Unsplit,
            account,
            moved,
            Vec::new(),
        ))
    }
}

fn thread_applied(
    kind: UndoKind,
    account: AccountId,
    messages: Vec<MessageId>,
    inverse: Vec<Command>,
) -> Applied {
    Applied {
        kind,
        account,
        count: messages.len(),
        messages,
        rows: None,
        removed: Vec::new(),
        arrived: None,
        reloaded: Vec::new(),
        changed: Vec::new(),
        mailboxes_changed: false,
        contacts_changed: None,
        inverse,
        lasts: None,
        surfaced: false,
    }
}
