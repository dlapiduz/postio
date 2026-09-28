//! Postio Focus's own verbs (specs/007-postio-focus): the commands only
//! Focus offers, carried out the way every verb here is -- resolve the aim,
//! write the store, push the undo entry, emit the events.
//!
//! They live beside the rest of the vocabulary rather than in the host
//! because undo replays an entry's inverse through [`Actions::act`]: a verb
//! whose way back is its own command, the other direction, has to be one
//! `act` knows (docs/ARCHITECTURE.md §5).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use postio_core::dispatch::CommandError;
use postio_core::undo::UndoKind;
use postio_core::{Command, MessageTarget};
use postio_model::{MessageId, ThreadId};
use postio_storage::repository::ReminderRepository;

use super::{Actions, Aim, Applied, store_failure};

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
}
