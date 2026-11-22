//! One type for what filing knows, shared by `postio-sync` and `postio-classify`: separate copies
//! lost the
//! conversation field, and a message with no conversation is never filtered. Only what is known at
//! filing; no body.

use crate::promoted::PromotedHeaders;
use crate::{MailboxRole, Message, ThreadId};

/// One message as it was filed.
#[derive(Debug, Clone, Copy)]
pub struct FiledMessage<'a> {
    /// Its own `thread_id` may be unset: sync threads a message after storing it. Use `thread`.
    pub message: &'a Message,
    /// The conversation threading put it in, or `None` when nothing has
    /// placed it -- which the guards read as a reason to leave it alone.
    pub thread: Option<ThreadId>,
    pub role: MailboxRole,
}

impl<'a> FiledMessage<'a> {
    pub fn unsubscribe_offered(&self) -> Option<bool> {
        self.promoted().map(|said| said.unsubscribe_offered)
    }

    pub fn automation(&self) -> Option<u8> {
        self.promoted().map(|said| said.automation)
    }

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
