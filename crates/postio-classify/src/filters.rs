//! Layer 3's filter rules (T122, research R8 and R12): what files a message
//! away before its body exists, from its own structure and headers.
//!
//! Three kinds of evidence, in the order they are asked, because each says
//! more than the one after it:
//!
//! 1. **The server's verdict.** `$Junk` among the flags: spam, decided by
//!    the server ([`Layer::Server`]).
//! 2. **The automated-senders table.** An address the table knows names
//!    its own reason -- a receipt is a receipt, whatever headers it carries
//!    ([`Layer::Senders`]). The table is data, never code (FR-114).
//! 3. **The headers** ([`Layer::Header`]), as the promoted fields read them
//!    (`postio_model::promoted`):
//!    - `Auto-Submitted`: a machine sent it, so a notification;
//!    - `Precedence: bulk` or `junk`: sent to many, so a promotion;
//!    - `List-Unsubscribe` with no `List-Id`: a sender's own mailing, so a
//!      promotion.
//!
//! **An invitation is never filed on a guess.** Past the server's verdict,
//! mail with a calendar part stays: an invitation is a real action, and it
//! comes to the inbox with its marker (FR-100, FR-122).
//!
//! **Anything else stays in the inbox** (FR-112). A person's letter, and a
//! discussion list -- `List-Id` with `Precedence: list`, which is people
//! writing through a list rather than a sender mailing its customers -- are
//! not what filtering is for. A header not yet known is no evidence either
//! way: JMAP learns the three only from the body, and a first sync never
//! asks for them.
//!
//! The source shown after a reason ("notification · Forge") is the table's
//! own name for the sender when it gives one, else the sender's display
//! name, else its domain.

use postio_model::Flag;
use postio_model::promoted::{AUTO_SUBMITTED, PRECEDENCE_BULK, PRECEDENCE_JUNK};

use crate::input::FiledMessage;
use crate::outcome::{Layer, Reason, ReasonKind, SourceName};
use crate::rules::Rules;

/// Why `filed` would be filed away, by its structure and headers alone, or
/// `None` when nothing it carries says so.
pub(crate) fn reason(filed: &FiledMessage<'_>, rules: &dyn Rules) -> Option<Reason> {
    let message = filed.message;
    if message.flags.contains(&Flag::Junk) {
        return Some(Reason {
            kind: ReasonKind::Spam,
            source: source_of(filed),
            layer: Layer::Server,
        });
    }
    // An invitation is a real action (FR-100, FR-122): past the server's
    // own verdict, nothing here files one away.
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

/// Who the message is from, as a filtered row names it: the sender's
/// display name, or its domain when it gives none.
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
