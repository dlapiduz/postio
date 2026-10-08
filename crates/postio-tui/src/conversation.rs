//! What the open message holds: the message, or the messages of its
//! conversation, of which one is shown at a time.
//!
//! A conversation opens on its newest message, as the desktop's does, and
//! `[` and `]` step through the rest; each body is read when it is first
//! shown.

use crate::reader::{Block, Rendered};
use chrono::{DateTime, Utc};
use postio_model::MessageId;
use postio_model::listing::MessageSummary;
use postio_ui::terminal::SafeText;

/// One message of what is being read.
#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    /// Which message.
    pub id: MessageId,
    /// Who sent it.
    pub from: SafeText,
    /// Their address, for the remote-image allow list.
    pub address: Option<String>,
    /// When it arrived.
    pub when: DateTime<Utc>,
    /// Its body, once it has arrived.
    pub body: Option<Rendered>,
    /// What the sanitiser held back from it.
    pub held_back: postio_ui::reader::document::HeldBack,
    /// Its body as it arrived, kept to draw it again the other way.
    pub source: Option<postio_model::MessageBody>,
    /// Whether the person asked for the sender's own markup (`view_original`).
    pub original: bool,
    /// Whether it is drawn in reader view.
    pub reader_view: bool,
    /// Whether its remote images are allowed: this once, or by sender. The
    /// terminal draws no image either way; this is what the notice says.
    pub images_allowed: bool,
    /// Whether its body has been asked for.
    pub asked: bool,
    /// Whether it has attachments, as its row says; its parts are asked for
    /// only then.
    pub has_attachments: bool,
    /// Its parts, once asked for.
    pub parts: Vec<postio_model::Attachment>,
    /// Whom it was written to, once its body has been read.
    pub to: Vec<SafeText>,
    /// Who was copied.
    pub cc: Vec<SafeText>,
}

impl Member {
    /// The parts a person would call attachments: named, or not inline.
    pub fn attachments(&self) -> Vec<&postio_model::Attachment> {
        self.parts
            .iter()
            .filter(|part| {
                part.filename.is_some() || part.disposition != postio_model::Disposition::Inline
            })
            .collect()
    }
}

impl Member {
    /// The one message a list row stands for, its body asked for.
    pub fn from_row(row: &crate::row::Row) -> Member {
        Member {
            id: row.id,
            from: row.from.clone(),
            address: row.address.clone(),
            when: row.when,
            body: None,
            held_back: Default::default(),
            source: None,
            original: false,
            reader_view: false,
            images_allowed: false,
            asked: true,
            has_attachments: row.attachment,
            parts: Vec::new(),
            to: Vec::new(),
            cc: Vec::new(),
        }
    }

    /// A member from a list row.
    pub fn from_summary(summary: &MessageSummary) -> Member {
        Member {
            id: summary.id,
            from: SafeText::new(summary.from.as_ref().map_or("", |from| from.display())),
            address: summary.from.as_ref().map(|from| from.address.clone()),
            when: summary.received_at,
            body: None,
            held_back: Default::default(),
            source: None,
            original: false,
            reader_view: false,
            images_allowed: false,
            asked: false,
            has_attachments: summary.has_attachments,
            parts: Vec::new(),
            to: Vec::new(),
            cc: Vec::new(),
        }
    }
}

/// A message opened for itself: Filtered's row, or what the command bar
/// found. It is not the list's cursor row, so its verbs aim at it and the
/// list's cursor and marks are left alone.
#[derive(Debug, Clone, PartialEq)]
pub struct Own {
    /// What the message is: the header's subject, labels and read state.
    pub row: crate::row::Row,
    /// Where the keyboard goes when the message closes.
    pub back: crate::app::Focus,
}

/// What the reader shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    /// The list row this was opened from.
    pub row: MessageId,
    /// Its messages, oldest first; one for a message on its own.
    pub members: Vec<Member>,
    /// The member the keyboard is on.
    pub current: usize,
    /// Set when it was opened for the message itself rather than for the
    /// cursor's row.
    pub own: Option<Own>,
}

/// What a line of a reading stands for, for a click on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum At {
    /// Nothing to act on.
    Nothing,
    /// A fold marker.
    Fold {
        /// Whose.
        member: usize,
        /// Which block.
        block: usize,
    },
    /// A link, by its destination.
    Link(String),
    /// An attachment's line.
    Part {
        /// Whose.
        member: usize,
        /// Which of its attachments.
        part: usize,
    },
}

impl Reading {
    /// Fold or unfold one fold of one member.
    pub fn toggle_fold(&mut self, member: usize, block: usize) {
        if let Some(body) = self
            .members
            .get_mut(member)
            .and_then(|member| member.body.as_mut())
        {
            body.toggle_fold(block);
        }
    }

    /// Every fold in every member: expand them all, or fold them all again.
    pub fn toggle_folds(&mut self) {
        let any_folded = self
            .members
            .iter()
            .filter_map(|member| member.body.as_ref())
            .any(|body| {
                body.blocks
                    .iter()
                    .any(|block| matches!(block, Block::Fold { folded: true, .. }))
            });
        for body in self
            .members
            .iter_mut()
            .filter_map(|member| member.body.as_mut())
        {
            for block in &mut body.blocks {
                if let Block::Fold { folded, .. } = block {
                    *folded = !any_folded;
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use chrono::TimeZone;

    use super::*;

    fn member(id: i64, from: &str, body: Option<&str>) -> Member {
        Member {
            id: MessageId::new(id),
            from: SafeText::new(from),
            address: None,
            when: Utc.with_ymd_and_hms(2026, 9, 20, 9, 0, 0).unwrap(),
            body: body.map(crate::reader::from_text),
            held_back: Default::default(),
            source: None,
            original: false,
            reader_view: false,
            images_allowed: false,
            asked: false,
            has_attachments: false,
            parts: Vec::new(),
            to: Vec::new(),
            cc: Vec::new(),
        }
    }

    /// A message whose body is `body`.
    pub(crate) fn member_saying(id: i64, body: &str) -> Member {
        member(id, "Ada", Some(body))
    }

    /// A message whose body is `lines` lines long.
    pub(crate) fn member_with_lines(id: i64, lines: usize) -> Member {
        let body: Vec<String> = (0..lines).map(|line| format!("line {line}")).collect();
        member(id, "Ada", Some(&body.join("\n")))
    }
}
