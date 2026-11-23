use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::address::EmailAddress;
use crate::ids::{AccountId, LabelId, MailboxId, MessageId, ThreadId};

fn epoch() -> DateTime<Utc> {
    DateTime::from_timestamp(0, 0).expect("the Unix epoch is a valid timestamp")
}

/// Reconstructed locally by the JWZ pass over `Message-ID`, `In-Reply-To`, `References` and
/// normalized subjects, with server threading only a hint.
/// The aggregate fields are a cache, meaningful only when recomputed together with `message_ids`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    pub id: ThreadId,
    /// Owning account. Threads never span accounts.
    pub account_id: AccountId,
    pub subject: Option<String>,
    pub message_ids: Vec<MessageId>,
    pub participants: Vec<EmailAddress>,
    pub mailbox_ids: Vec<MailboxId>,
    pub labels: Vec<LabelId>,
    pub message_count: u32,
    pub unread_count: u32,
    pub has_attachments: bool,
    pub is_flagged: bool,
    pub first_at: DateTime<Utc>,
    pub last_at: DateTime<Utc>,
}

impl Thread {
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

    pub fn is_empty(&self) -> bool {
        self.message_ids.is_empty()
    }

    pub fn has_unread(&self) -> bool {
        self.unread_count > 0
    }

    pub fn root_message_id(&self) -> Option<MessageId> {
        self.message_ids.first().copied()
    }

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
