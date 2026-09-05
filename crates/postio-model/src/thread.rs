//! Threads: conversations reconstructed locally.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::address::EmailAddress;
use crate::ids::{AccountId, LabelId, MailboxId, MessageId, ThreadId};

/// Fallback timestamp for a thread that has no messages yet.
fn epoch() -> DateTime<Utc> {
    DateTime::from_timestamp(0, 0).expect("the Unix epoch is a valid timestamp")
}

/// A conversation.
///
/// Threading is a first-class *local* concept: threads are reconstructed by the
/// JWZ pass over `Message-ID`, `In-Reply-To`, `References` and normalized
/// subjects, using server-provided threading only as a hint. This type is the
/// denormalized result the message list renders from — the tree structure lives
/// in the threading pass, which builds it from
/// [`Message::reference_chain`](crate::Message::reference_chain).
///
/// The aggregate fields are a cache of the thread's messages and are only
/// meaningful when recomputed together with `message_ids`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    /// Local id.
    pub id: ThreadId,
    /// Owning account. Threads never span accounts.
    pub account_id: AccountId,
    /// Normalized subject of the thread's root message.
    pub subject: Option<String>,
    /// Member messages, oldest first.
    pub message_ids: Vec<MessageId>,
    /// Distinct participants, in first-seen order.
    pub participants: Vec<EmailAddress>,
    /// Every mailbox the thread has a message in.
    pub mailbox_ids: Vec<MailboxId>,
    /// Union of the labels on the thread's messages.
    pub labels: Vec<LabelId>,
    /// Number of member messages.
    pub message_count: u32,
    /// Number of member messages without `\Seen`.
    pub unread_count: u32,
    /// Whether any member message has an attachment.
    pub has_attachments: bool,
    /// Whether any member message carries `\Flagged`.
    pub is_flagged: bool,
    /// Date of the oldest member message.
    pub first_at: DateTime<Utc>,
    /// Date of the newest member message; the list sorts on this.
    pub last_at: DateTime<Utc>,
}

impl Thread {
    /// Builds an empty, unpersisted thread.
    pub fn new(account_id: AccountId) -> Self {
        Self {
            id: ThreadId::UNASSIGNED,
            account_id,
            subject: None,
            message_ids: Vec::new(),
            participants: Vec::new(),
            mailbox_ids: Vec::new(),
            labels: Vec::new(),
            message_count: 0,
            unread_count: 0,
            has_attachments: false,
            is_flagged: false,
            first_at: epoch(),
            last_at: epoch(),
        }
    }

    /// Whether the thread has no member messages.
    pub fn is_empty(&self) -> bool {
        self.message_ids.is_empty()
    }

    /// Whether any member message is unread.
    pub fn has_unread(&self) -> bool {
        self.unread_count > 0
    }

    /// The oldest member message, i.e. the thread's root.
    pub fn root_message_id(&self) -> Option<MessageId> {
        self.message_ids.first().copied()
    }

    /// The newest member message.
    pub fn latest_message_id(&self) -> Option<MessageId> {
        self.message_ids.last().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(message_ids: Vec<MessageId>) -> Thread {
        Thread {
            message_ids,
            ..Thread::new(AccountId::new(1))
        }
    }

    #[test]
    fn root_and_latest_are_none_for_an_empty_thread() {
        let empty = Thread::new(AccountId::new(1));
        assert_eq!(empty.root_message_id(), None);
        assert_eq!(empty.latest_message_id(), None);
    }

    #[test]
    fn root_is_the_oldest_member_and_latest_is_the_newest() {
        // `message_ids` is oldest-first (the struct's own doc comment), so
        // root reads the front of the list and latest reads the back --
        // the two must not answer the same end twice, which is the bug a
        // copy-pasted `.first()` for both would produce.
        let ids = vec![MessageId::new(1), MessageId::new(2), MessageId::new(3)];
        let thread = thread(ids);

        assert_eq!(thread.root_message_id(), Some(MessageId::new(1)));
        assert_eq!(thread.latest_message_id(), Some(MessageId::new(3)));
    }

    #[test]
    fn a_single_message_thread_is_its_own_root_and_latest() {
        let thread = thread(vec![MessageId::new(7)]);

        assert_eq!(thread.root_message_id(), Some(MessageId::new(7)));
        assert_eq!(thread.latest_message_id(), Some(MessageId::new(7)));
    }
}
