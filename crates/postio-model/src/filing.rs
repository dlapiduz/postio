//! A message as filing knows it (spec 007, `contracts/engine.md`, "The
//! filing pass").
//!
//! Two crates meet over it: `postio-sync`, whose incremental passes hand
//! every arrival to a filing pass inside the transaction that stored it, and
//! `postio-classify`, which decides what Focus does with each one. They
//! used to have a type each, with the same name and different fields, and a
//! copy between them is where a field goes missing -- the conversation did,
//! and a message with no conversation is never filtered. So this is the one
//! type, and it lives in the leaf both already depend on.
//!
//! Only what is known when a message is filed: the envelope, `References`
//! and `List-Id`, the flags (`$Junk` among them), what the promoted headers
//! said (research R8), the structure, the folder's role, and the thread. No
//! body: filing happens before one is fetched.

use crate::promoted::PromotedHeaders;
use crate::{MailboxRole, Message, ThreadId};

/// One message as it was filed.
#[derive(Debug, Clone, Copy)]
pub struct FiledMessage<'a> {
    /// The message as the store holds it, with its id.
    ///
    /// Its own `thread_id` is not the conversation to read: sync threads a
    /// message after it stores it, so the row it hands over may not carry
    /// one yet. [`FiledMessage::thread`] does.
    pub message: &'a Message,
    /// The conversation threading put it in, or `None` when nothing has
    /// placed it -- which the guards read as a reason to leave it alone.
    pub thread: Option<ThreadId>,
    /// The role of the folder it was filed into.
    pub role: MailboxRole,
}

impl<'a> FiledMessage<'a> {
    /// Whether it carries a `List-Unsubscribe` (`messages.unsubscribe_offered`),
    /// or `None` while that is not known.
    pub fn unsubscribe_offered(&self) -> Option<bool> {
        self.promoted().map(|said| said.unsubscribe_offered)
    }

    /// What its `Precedence` and `Auto-Submitted` say, as
    /// `messages.automation`'s bits ([`crate::promoted`]), or `None` while
    /// that is not known.
    pub fn automation(&self) -> Option<u8> {
        self.promoted().map(|said| said.automation)
    }

    /// Whether its structure holds a `text/calendar` part: an invitation, or
    /// an answer to one.
    pub fn has_calendar(&self) -> bool {
        self.message
            .attachments
            .iter()
            .any(|part| part.mime_type.eq_ignore_ascii_case("text/calendar"))
    }

    fn promoted(&self) -> Option<PromotedHeaders> {
        self.message.promoted
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::promoted::{AUTO_GENERATED, PRECEDENCE_BULK};
    use crate::{AccountId, Attachment, MailboxId, MessageId};

    fn filed(message: &Message) -> FiledMessage<'_> {
        FiledMessage {
            message,
            thread: None,
            role: MailboxRole::Inbox,
        }
    }

    #[test]
    fn what_the_promoted_headers_said_is_read_off_the_message() {
        let mut message = Message::new(AccountId::new(1), MailboxId::new(1), Utc::now());
        assert_eq!(filed(&message).unsubscribe_offered(), None, "not known");
        assert_eq!(filed(&message).automation(), None, "not known");

        message.promoted = Some(PromotedHeaders {
            unsubscribe_offered: true,
            automation: PRECEDENCE_BULK | AUTO_GENERATED,
        });
        assert_eq!(filed(&message).unsubscribe_offered(), Some(true));
        assert_eq!(
            filed(&message).automation(),
            Some(PRECEDENCE_BULK | AUTO_GENERATED)
        );
    }

    #[test]
    fn a_calendar_part_in_the_structure_is_an_invitation_s() {
        let mut message = Message::new(AccountId::new(1), MailboxId::new(1), Utc::now());
        assert!(!filed(&message).has_calendar());

        let part = |mime_type: &str| vec![Attachment::new(MessageId::new(1), mime_type, 2_048)];
        message.attachments = part("text/calendar");
        assert!(filed(&message).has_calendar());
        message.attachments = part("TEXT/CALENDAR");
        assert!(filed(&message).has_calendar(), "a MIME type is caseless");
        message.attachments = part("application/pdf");
        assert!(!filed(&message).has_calendar());
    }
}
