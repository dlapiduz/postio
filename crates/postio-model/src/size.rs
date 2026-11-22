//! A draft over the provider's limit used to be queued, sent and rejected after the composer had
//! closed. The limit comes from account configuration, never the SMTP `SIZE` capability
//! (`postio-smtp` relies on no capability list). Inline images and attached files share one total.

use std::fmt;

use crate::Draft;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TooLarge {
    pub total: u64,
    pub limit: u64,
    pub over_by: u64,
    pub largest: Vec<Item>,
}

/// Removing the largest is usually the whole answer; a refusal that lists every part is not read.
pub const NAMED: usize = 3;

/// Smaller parts are noise in a sentence that has to be acted on.
const WORTH_NAMING: u64 = 64 * 1024;

/// Both alternatives, because both go on the wire.
pub fn of_body(draft: &Draft) -> u64 {
    let text = draft.body.text.as_deref().map(str::len).unwrap_or(0);
    let html = draft.body.html.as_deref().map(str::len).unwrap_or(0);
    (text + html) as u64
}

/// An estimate of content, not encoded bytes (base64 adds a third), so it never refuses mail the
/// server would take and may pass one it will not: a false refusal puts Postio between someone and
/// their mail.
pub fn of(draft: &Draft) -> u64 {
    of_body(draft) + draft.attachments.iter().map(|part| part.size).sum::<u64>()
}

/// `None` for `limit` means no configured ceiling and nothing is checked: a guessed number would
/// refuse mail the server would take.
pub fn check(draft: &Draft, limit: Option<u64>) -> Option<TooLarge> {
    let limit = limit?;
    let total = of(draft);
    if total <= limit {
        return None;
    }

    let mut largest: Vec<Item> = draft
        .attachments
        .iter()
        .filter(|part| part.size >= WORTH_NAMING)
        .map(|part| Item {
            name: part
                .filename
                .clone()
                .unwrap_or_else(|| "an unnamed attachment".to_owned()),
            size: part.size,
        })
        .collect();
    largest.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
    largest.truncate(NAMED);

    Some(TooLarge {
        total,
        limit,
        over_by: total - limit,
        largest,
    })
}

/// Powers of ten, not two: a provider's published limit is "25 MB".
fn human(n: u64) -> String {
    const MB: u64 = 1_000_000;
    const KB: u64 = 1_000;
    if n >= MB {
        let tenths = (n * 10).div_ceil(MB);
        if tenths.is_multiple_of(10) {
            format!("{} MB", tenths / 10)
        } else {
            format!("{}.{} MB", tenths / 10, tenths % 10)
        }
    } else if n >= KB {
        format!("{} KB", n.div_ceil(KB))
    } else {
        format!("{n} bytes")
    }
}

impl fmt::Display for TooLarge {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            out,
            "This message is {} and the limit is {} — {} too big.",
            human(self.total),
            human(self.limit),
            human(self.over_by),
        )?;
        match self.largest.split_first() {
            None => Ok(()),
            Some((first, rest)) => {
                write!(
                    out,
                    " The largest is {} ({})",
                    first.name,
                    human(first.size)
                )?;
                for item in rest {
                    write!(out, ", then {} ({})", item.name, human(item.size))?;
                }
                write!(out, ".")
            }
        }
    }
}
