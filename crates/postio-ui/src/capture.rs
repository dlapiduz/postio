//! What a capture -- a task or a note written to the vault from a message --
//! is made from and what it says, with no toolkit in it.

use chrono::NaiveDate;
use postio_model::MessageId;
use postio_model::listing::{MarkerKind, MarkerWhen};

use crate::focus_list::FocusRow;

/// The message a capture is made from, as the row or the open message says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The message, which the line's link opens.
    pub message: MessageId,
    /// Who sent it.
    pub sender: String,
    /// Its subject.
    pub subject: String,
    /// When it arrived, as a person reads it.
    pub when: String,
    /// The sentence its marker quotes, verbatim, when it has one.
    pub sentence: Option<String>,
    /// The day the mail says it is due, when it says.
    pub due: Option<NaiveDate>,
}

/// What a capture is made from: the row under the cursor when it is the
/// message aimed at, with its marker's sentence and day; otherwise only the
/// open message's title. `None` when the row is a digest, which is no one
/// message.
pub fn source(
    message: MessageId,
    cursor: Option<&FocusRow>,
    open_title: &str,
    now: chrono::DateTime<chrono::Local>,
) -> Option<Source> {
    let Some(row) = cursor.filter(|row| row.id() == message) else {
        return Some(Source {
            message,
            sender: String::new(),
            subject: open_title.to_owned(),
            when: String::new(),
            sentence: None,
            due: None,
        });
    };
    let summary = &row.as_conversation()?.summary;
    let representative = &summary.representative;
    let marker = summary.marker.as_ref();
    Some(Source {
        message,
        sender: representative
            .from
            .as_ref()
            .map(|from| from.display().to_owned())
            .unwrap_or_default(),
        subject: representative.subject.clone().unwrap_or_default(),
        when: crate::row::timestamp(summary.last_at, now),
        sentence: marker.and_then(|marker| marker.excerpt.clone()),
        due: marker.and_then(|marker| match marker.when {
            Some(MarkerWhen::Due(at)) if marker.kind == MarkerKind::Todo => {
                Some(at.with_timezone(&chrono::Local).date_naive())
            }
            _ => None,
        }),
    })
}
