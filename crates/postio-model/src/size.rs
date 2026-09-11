//! How big a draft is, and whether that is too big.
//!
//! There was no client-side size check. A draft over the provider's limit was
//! queued, sent, and rejected by the server — after the composer had closed,
//! which is the shape of failure FR-056 exists to prevent. What the person is
//! left holding is a `Failed` draft and the job of working out for themselves
//! which of six attachments to remove.
//!
//! # Where the limit comes from
//!
//! The account's configuration, and never the SMTP `SIZE` capability
//! (research §3). `postio-smtp` is explicit that Postio's compliance argument
//! is that it "announces nothing and relies on nothing, and a capability list
//! is exactly the thing that erodes that one `if server_supports` at a time" —
//! so a refusal conditional on a capability is that erosion. A configured
//! number is also the form Principle VII requires: providers are data.
//!
//! # One total, not two
//!
//! Inline images and attached files share one budget, settled by the
//! maintainer on 2026-09-10. Two budgets is the arrangement where a message
//! under both is still over what the server will take, which is the only
//! limit that actually exists. They are one list in the model already, so
//! this is arithmetic rather than a rule.

use std::fmt;

use crate::Draft;

/// A part worth naming in a refusal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// What to call it, as the person would recognise it.
    pub name: String,
    /// Its size in bytes.
    pub size: u64,
}

/// Why a draft may not be sent as it stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TooLarge {
    /// What the whole message comes to.
    pub total: u64,
    /// What the account allows.
    pub limit: u64,
    /// How far over. Stated rather than left as arithmetic, because it is the
    /// number that says how much has to go.
    pub over_by: u64,
    /// The biggest parts, largest first — at most [`NAMED`] of them.
    pub largest: Vec<Item>,
}

/// How many parts a refusal names.
///
/// Three. Removing the largest is usually the whole answer, and a refusal
/// that lists every part is one nobody reads.
pub const NAMED: usize = 3;

/// A part smaller than this is not worth naming: removing it would not
/// change the outcome, and it is noise in a sentence that has to be acted on.
const WORTH_NAMING: u64 = 64 * 1024;

/// What the draft's body contributes.
///
/// Both alternatives, because both go on the wire: a `multipart/alternative`
/// carries the text and the HTML, and a long quoted reply is not free.
pub fn of_body(draft: &Draft) -> u64 {
    let text = draft.body.text.as_deref().map(str::len).unwrap_or(0);
    let html = draft.body.html.as_deref().map(str::len).unwrap_or(0);
    (text + html) as u64
}

/// What the whole message comes to, near enough to refuse on.
///
/// Deliberately an estimate of the *content*, not of the encoded bytes. Base64
/// inflates a part by about a third and MIME headers add their own, so the
/// true figure on the wire is larger — which means this never refuses a
/// message the server would have accepted, and may pass one it will not. That
/// asymmetry is the right way round: a false refusal is Postio standing
/// between someone and their mail on its own authority, while a false pass
/// leaves them exactly where they are today, with the server's own answer.
pub fn of(draft: &Draft) -> u64 {
    of_body(draft) + draft.attachments.iter().map(|part| part.size).sum::<u64>()
}

/// Whether `draft` may be sent, given the account's `limit`.
///
/// `None` for `limit` means the account carries no configured ceiling, and
/// then this checks nothing. A guessed number would refuse mail the server
/// would have taken, and the person has no way to tell Postio's opinion from
/// their provider's rule.
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
    // Largest first, and by name where two are the same size, so the sentence
    // does not reorder itself between two runs over the same draft.
    largest.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
    largest.truncate(NAMED);

    Some(TooLarge {
        total,
        limit,
        over_by: total - limit,
        largest,
    })
}

/// `n` bytes, as a person reads them.
///
/// Powers of ten, not of two: a provider's published limit is "25 MB", and a
/// refusal quoting 23.8 MB against it invites the reasonable conclusion that
/// Postio cannot count.
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
    /// The refusal as a sentence, because a struct nobody renders names
    /// nothing and FR-056 is about what the person is told.
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
