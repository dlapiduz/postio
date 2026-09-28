//! Postio Focus's own verbs (specs/007-postio-focus): the commands only
//! Focus offers, carried out the way every verb here is -- resolve the aim,
//! write the store, push the undo entry, emit the events.
//!
//! They live beside the rest of the vocabulary rather than in the host
//! because undo replays an entry's inverse through [`Actions::act`]: a verb
//! whose way back is its own command, the other direction, has to be one
//! `act` knows (docs/ARCHITECTURE.md §5).

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
    AccountRepository, DraftRepository, FilterDecisionRepository, InviteState, MarkerRepository,
    ReminderRepository,
};

use super::{Actions, Aim, Applied, Destination, RSVP_WINDOW, mailbox_for, store_failure};

impl Actions {
    /// Wait for a reply in each targeted conversation until `at`, or with
    /// `None` stop waiting (spec 007 US5, FR-044, FR-045).
    ///
    /// One reminder stands per conversation, however many of its messages
    /// were targeted: it waits on a reply to the conversation. Setting one
    /// where one stands moves it to the new time. The way back names each
    /// conversation's previous time, `None` where there was none, so undo
    /// puts back exactly what stood.
    ///
    /// Local only, like a snooze: no server hears of a reminder. It changes
    /// no list either -- the conversation stays where it is until the
    /// reminder fires -- so nothing is repainted.
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
                    "Select the conversations to be reminded about",
                ));
            }
        };
        // The first targeted message of each conversation is the one the
        // reminder waits on a reply to.
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
                "That message is not in a conversation yet, so there is nothing to wait on",
            ));
        };

        let now = Utc::now();
        let mut previous: BTreeMap<Option<DateTime<Utc>>, Vec<MessageId>> = BTreeMap::new();
        let transaction = connection.transaction().await.map_err(store_failure)?;
        {
            let reminders = ReminderRepository::new(&transaction);
            for (thread, anchor) in &conversations {
                let standing = reminders.standing(*thread).await.map_err(store_failure)?;
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
            lasts: None,
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

    /// Answer the invitation `message` carries, or the focused message's,
    /// and send the reply (spec 007 US8, FR-102, research R9).
    ///
    /// Local-first, like a send: the reply is written as a draft and queued
    /// with a not-before time [`RSVP_WINDOW`] away, and the marker says
    /// `accepting` or `declining` until then. Nothing reaches the network
    /// here, and nothing can before the window closes, so the undo entry --
    /// which cancels the send -- lasts exactly that long; after it, the
    /// answer stands, and `mod+z` reaches whatever was done before it.
    ///
    /// The reply goes out from the identity the invitation names among its
    /// attendees, spelled as the invitation spells it: answering as some
    /// other address would be a stranger's reply to the organiser's
    /// calendar. With no such identity there is nobody to answer as.
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
        let invitation = invitation_of(&blobs, &row).await.ok_or_else(|| {
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

        Ok(Applied {
            lasts: Some(RSVP_WINDOW),
            account: account.id,
            kind,
            count: 1,
            messages: vec![row.id],
            removed: Vec::new(),
            arrived: None,
            reloaded: Vec::new(),
            // The row repaints with its answer, and the Outbox holds the
            // reply while it waits.
            changed: vec![row.id],
            mailboxes_changed: true,
            inverse: vec![Command::CancelSend {
                draft: Some(draft.id),
            }],
        })
    }

    /// When `draft`, whose send was just cancelled, is the reply to an
    /// invitation, take the answer back whole: the reply goes -- it was
    /// never the person's to edit -- and the invitation is unanswered again.
    /// Answers the messages whose rows repaint.
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
        Ok(vec![invitation])
    }
}

/// How many markers of one kind the person dismisses in one sender's mail
/// before Focus takes it as a correction: that this sender's mail does not
/// ask that of them (FR-108, the plan's answer to "how far a dismissal
/// teaches").
pub const STOPS_AFTER: u32 = 3;

impl Actions {
    /// Take the markers off the targeted messages as wrong, or bring them
    /// back (spec 007 FR-108, US12 scenario 5).
    ///
    /// A dismissal is for good on its own message: the body stage never
    /// writes over one. And it teaches: once the person has dismissed
    /// [`STOPS_AFTER`] markers of one kind in one sender's mail, the
    /// correction goes into `[focus.filter] stop_markers`, written as an
    /// editor saves it, and both detectors leave that kind alone in that
    /// sender's mail from then on. Bringing markers back takes the
    /// correction back when it drops the count below the line it crossed.
    /// Only question and to-do markers teach; an invitation's comes from
    /// its calendar part.
    ///
    /// The correction is the person's decision, so it lives in the file and
    /// not the store; a file Focus cannot write leaves the dismissal
    /// standing and the correction unwritten, which is said in the log by
    /// outcome only.
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
        // How many markers of each kind, in each sender's mail, this took
        // off or put back.
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
                        .write(move |text| {
                            postio_config::focus_edit::set_stop_marker(text, &sender, kind, present)
                                .map_err(|_| {
                                    "Postio could not write that correction to config.toml"
                                        .to_owned()
                                })
                        })
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
            lasts: None,
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
            // The row draws its marker, or no longer does.
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
    /// Put filtered messages back in the inbox and never filter their
    /// senders again, or, with `restored` false, file them away again as
    /// they were (spec 007 US9 scenario 4, FR-116).
    ///
    /// Three things, and one undo takes back all three:
    ///
    /// - the message moves to its account's inbox, with the server's move
    ///   queued, as any move is;
    /// - its decision is marked restored rather than deleted: the store
    ///   keeps what Focus decided and what the person said of it (SC-012),
    ///   and undo puts back the very reason it had;
    /// - its sender joins `[focus.filter] never`, written as an editor saves
    ///   it, and the filing pass reads the file as written at once.
    ///
    /// On the way back a sender is unpinned only when no other restore of
    /// theirs still stands behind the pin.
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
        // Where each account's messages go: its inbox, or back to its
        // archive. Resolved before anything is written, as `relocate` does.
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
                for row in &rows {
                    if !decisions
                        .restore(row.id, restored.then_some(now))
                        .await
                        .map_err(store_failure)?
                    {
                        continue;
                    }
                    ids.push(row.id);
                    senders.extend(row.from.iter().map(EmailAddress::normalized));
                    if row.mailbox_id != destination {
                        moved.entry(row.mailbox_id).or_default().push(row.id);
                    }
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
                lasts: None,
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
                let written = self
                    .focus
                    .write(move |text| {
                        postio_config::focus_edit::set_never(text, &sender, restored).map_err(
                            |_| "Postio could not write that sender to config.toml".to_owned(),
                        )
                    })
                    .await;
                if let Err(reason) = written {
                    tracing::warn!(%reason, "Focus could not write a restored sender: {reason}");
                }
            }
        }
        Ok(applied)
    }
}

/// The name `[focus.filter] stop_markers` gives a marker kind whose
/// dismissals teach: a question or a to-do. An invitation's marker comes
/// from its calendar part and a reminder's from its time, and dismissing
/// either says nothing about what the sender's mail asks.
fn teaches(kind: MarkerKind) -> Option<&'static str> {
    match kind {
        MarkerKind::Question => Some("question"),
        MarkerKind::Todo => Some("todo"),
        MarkerKind::Invite | MarkerKind::NoReply => None,
    }
}

/// The marker kind a stop names.
fn marker_kind(name: &str) -> MarkerKind {
    if name == "todo" {
        MarkerKind::Todo
    } else {
        MarkerKind::Question
    }
}

/// The invitation `row`'s calendar part carries, when the part is on this
/// machine and is a request: what can be answered. Read off the async
/// runtime's threads, as parsing is.
async fn invitation_of(blobs: &postio_storage::BlobStore, row: &Message) -> Option<Invitation> {
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
                .filter(|invitation| invitation.method == Method::Request)
        })
    })
    .await
    .ok()
    .flatten()
}

/// The identity of `account` that `invitation` names among its attendees,
/// and the attendee as the invitation spells it: who answers, and as whom.
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
