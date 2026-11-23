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
    if let Some(sender) = message
        .from
        .iter()
        .find_map(|from| rules.senders().find(from))
    {
        return Some(Reason {
            kind: sender.reason(),
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
