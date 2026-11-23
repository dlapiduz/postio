//! The one message on screen. Its thread is the controller's (ADR 0045): the terminal holds only
//! what drawing this message needs, read when it is shown.

use crate::reader::{Block, Rendered};
use chrono::{DateTime, Utc};
use postio_model::MessageId;
use postio_ui::terminal::SafeText;

#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    pub id: MessageId,
    pub from: SafeText,
    /// For the remote-image allow list.
    pub address: Option<String>,
    pub when: DateTime<Utc>,
    pub body: Option<Rendered>,
    pub held_back: postio_ui::reader::document::HeldBack,
    /// Kept to draw it again the other way (`view_original`).
    pub source: Option<postio_model::MessageBody>,
    pub original: bool,
    pub reader_view: bool,
    /// The terminal draws no image either way; this is what the notice says.
    pub images_allowed: bool,
    pub parts: Vec<postio_model::Attachment>,
    pub to: Vec<SafeText>,
    pub cc: Vec<SafeText>,
    /// Ordinary mail has none (#1928).
    pub warning: Option<SafeText>,
}

impl Member {
    pub fn new(id: MessageId) -> Member {
        Member {
            id,
            from: SafeText::default(),
            address: None,
            when: DateTime::default(),
            body: None,
            held_back: Default::default(),
            source: None,
            original: false,
            reader_view: false,
            images_allowed: false,
            parts: Vec::new(),
            to: Vec::new(),
            cc: Vec::new(),
            warning: None,
        }
    }

    pub fn from_row(row: &crate::row::Row) -> Member {
        Member {
            from: row.from.clone(),
            address: row.address.clone(),
            when: row.when,
            ..Member::new(row.id)
        }
    }

    /// The header as the store has it; every field of it came from a sender.
    pub fn address(&mut self, row: &postio_model::Message, warning: Option<&str>) {
        let safe = |people: &[postio_model::EmailAddress]| -> Vec<_> {
            people
                .iter()
                .map(|address| SafeText::new(address.display()))
                .collect()
        };
        if let Some(from) = row.from.first() {
            self.from = SafeText::new(from.display());
            self.address = Some(from.address.clone());
        }
        self.when = row.date.unwrap_or(row.received_at);
        self.to = safe(&row.to);
        self.cc = safe(&row.cc);
        self.parts = row.attachments.clone();
        self.warning = warning.map(SafeText::new);
    }

    /// The parts a person would call attachments: named, or not inline.
    pub fn attachments(&self) -> Vec<&postio_model::Attachment> {
        self.parts
            .iter()
            .filter(|part| {
                part.filename.is_some() || part.disposition != postio_model::Disposition::Inline
            })
            .collect()
    }

    pub fn toggle_fold(&mut self, block: usize) {
        if let Some(body) = self.body.as_mut() {
            body.toggle_fold(block);
        }
    }

    /// Expand every fold, or fold them all again.
    pub fn toggle_folds(&mut self) {
        let Some(body) = self.body.as_mut() else {
            return;
        };
        let any_folded = body
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Fold { folded: true, .. }));
        for block in &mut body.blocks {
            if let Block::Fold { folded, .. } = block {
                *folded = !any_folded;
            }
        }
    }
}

/// Filtered's row, or what the command bar found: not the list's cursor row, so its verbs aim at
/// it and the list's cursor and marks are left alone.
#[derive(Debug, Clone, PartialEq)]
pub struct Own {
    pub row: crate::row::Row,
    pub back: crate::app::Focus,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    /// The list row it was opened from, which a step through its thread keeps.
    pub row: MessageId,
    pub shown: Member,
    pub own: Option<Own>,
}

impl Reading {
    pub fn of(row: &crate::row::Row) -> Reading {
        Reading {
            row: row.id,
            shown: Member::from_row(row),
            own: None,
        }
    }
}

/// What a line of the message stands for, for a click on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum At {
    Nothing,
    Fold(usize),
    Link(String),
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn member_saying(id: i64, body: &str) -> Member {
        Member {
            from: SafeText::new("Ada"),
            body: Some(crate::reader::from_text(body)),
            ..Member::new(MessageId::new(id))
        }
    }
}
