//! Layer 3: files a message away before its body exists, from structure and headers, asked in
//! order: the server's `$Junk`, the automated-senders table, then headers. An invitation is never
//! filed on a guess and anything else stays in the inbox; a header not yet known is no evidence.

use postio_model::Flag;
use postio_model::promoted::{AUTO_SUBMITTED, PRECEDENCE_BULK, PRECEDENCE_JUNK};

use crate::input::FiledMessage;
use crate::outcome::{Layer, Reason, ReasonKind, SourceName};
use crate::rules::Rules;

pub(crate) fn reason(filed: &FiledMessage<'_>, rules: &dyn Rules) -> Option<Reason> {
    let message = filed.message;
    if message.flags.contains(&Flag::Junk) {
        return Some(Reason {
            kind: ReasonKind::Spam,
            source: source_of(filed),
            layer: Layer::Server,
        });
    }
    // An invitation is a real action: past the server's verdict, nothing here files one away.
    if filed.has_calendar() {
        return None;
    }
    if let Some((sender, kind)) = message
        .from
        .iter()
        .filter_map(|from| rules.senders().find(from))
        .find_map(|sender| Some((sender, sender.reason()?)))
    {
        return Some(Reason {
            kind,
            source: sender.source().cloned().or_else(|| source_of(filed)),
            layer: Layer::Senders,
        });
    }
    let automation = filed.automation().unwrap_or(0);
    let kind = if automation & AUTO_SUBMITTED != 0 {
        ReasonKind::Notification
    } else if automation & (PRECEDENCE_BULK | PRECEDENCE_JUNK) != 0
        || (filed.unsubscribe_offered() == Some(true) && message.list_id.is_none())
    {
        ReasonKind::Promotion
    } else {
        return None;
    };
    Some(Reason {
        kind,
        source: source_of(filed),
        layer: Layer::Header,
    })
}

fn source_of(filed: &FiledMessage<'_>) -> Option<SourceName> {
    let from = filed.message.from.first()?;
    from.name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .or_else(|| from.domain().map(str::to_ascii_lowercase))
        .map(SourceName)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use postio_model::promoted::{PRECEDENCE_BULK, PromotedHeaders};
    use postio_model::{AccountId, EmailAddress, MailboxId, MailboxRole, Message};

    use super::*;
    use crate::senders::Senders;

    struct Shipped;

    impl Rules for Shipped {
        fn senders(&self) -> &Senders {
            Senders::shipped()
        }
    }

    fn statement(automation: u8) -> Message {
        let mut message = Message::new(AccountId::new(1), MailboxId::new(1), Utc::now());
        message.from = vec![EmailAddress::new(
            Some("Bank"),
            "statements@email.bank.example",
        )];
        message.promoted = Some(PromotedHeaders {
            unsubscribe_offered: false,
            automation,
        });
        message
    }

    fn filed(message: &Message) -> Option<Reason> {
        reason(
            &FiledMessage {
                message,
                thread: None,
                role: MailboxRole::Inbox,
            },
            &Shipped,
        )
    }

    #[test]
    fn a_machine_the_table_gives_no_reason_is_left_to_the_headers() {
        assert_eq!(filed(&statement(0)), None, "a statement stays in the inbox");
        assert_eq!(
            filed(&statement(PRECEDENCE_BULK)).map(|reason| (reason.kind, reason.layer)),
            Some((ReasonKind::Promotion, Layer::Header)),
            "and its headers still decide"
        );
    }
}
