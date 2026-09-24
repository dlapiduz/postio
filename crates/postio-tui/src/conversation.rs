//! What the reader shows: one message, or a whole conversation as one
//! scrolling document (ADR 0032's shape, in a terminal).
//!
//! Each member is a header line -- who, when -- and then its body, or a
//! line saying it is on its way. `J`/`K` move between members by scrolling to
//! their headers; the conversation opens on its newest message, as the
//! desktop's does.

use chrono::{DateTime, Local, Utc};
use postio_model::MessageId;
use postio_model::listing::MessageSummary;
use postio_ui::terminal::SafeText;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::reader::{Block, Rendered};

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
    /// Whether its remote images are allowed: this once, or by sender. The
    /// terminal draws no image either way; this is what the notice says.
    pub images_allowed: bool,
    /// Whether it has attachments, as its row says; its parts are asked for
    /// only then.
    pub has_attachments: bool,
    /// Its parts, once asked for.
    pub parts: Vec<postio_model::Attachment>,
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
    /// A member from a list row.
    pub fn from_summary(summary: &MessageSummary) -> Member {
        Member {
            id: summary.id,
            from: SafeText::new(summary.from.as_ref().map_or("", |from| from.display())),
            address: summary.from.as_ref().map(|from| from.address.clone()),
            when: summary.received_at,
            body: None,
            held_back: Default::default(),
            images_allowed: false,
            has_attachments: summary.has_attachments,
            parts: Vec::new(),
        }
    }
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
}

impl Reading {
    /// Every line, top to bottom, and the line each member's header is on.
    pub fn layout(&self, now: DateTime<Local>) -> (Vec<Line<'static>>, Vec<usize>) {
        let mut lines = Vec::new();
        let mut headers = Vec::new();
        let several = self.members.len() > 1;
        for (index, member) in self.members.iter().enumerate() {
            if several {
                if index > 0 {
                    lines.push(Line::default());
                }
                headers.push(lines.len());
                let mark = if index == self.current { "▶ " } else { "  " };
                lines.push(Line::from(vec![
                    Span::raw(mark),
                    Span::styled(
                        member.from.as_str().to_owned(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(format!("  {}", postio_ui::row::timestamp(member.when, now))),
                ]));
            } else {
                headers.push(0);
            }
            if member.images_allowed {
                lines.push(Line::styled(
                    "Remote images allowed — a terminal draws none",
                    Style::default().add_modifier(Modifier::DIM),
                ));
            } else if member.held_back.remote_images + member.held_back.trackers > 0 {
                lines.push(Line::styled(
                    format!("{} · i i to show", member.held_back.summary()),
                    Style::default().add_modifier(Modifier::DIM),
                ));
            }
            match &member.body {
                Some(body) => lines.extend(body.lines()),
                None => lines.push(Line::raw("…")),
            }
            for part in member.attachments() {
                let name = SafeText::new(part.filename.as_deref().unwrap_or(&part.mime_type));
                lines.push(Line::raw(format!(
                    "📎 {} · {}",
                    name.as_str(),
                    postio_ui::format::human_size(part.size)
                )));
            }
        }
        (lines, headers)
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
mod tests {
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
            images_allowed: false,
            has_attachments: false,
            parts: Vec::new(),
        }
    }

    fn text(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.to_string())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn each_member_has_a_header_and_its_body_or_a_wait() {
        let reading = Reading {
            row: MessageId::new(2),
            members: vec![member(1, "Ada", Some("First")), member(2, "Bea", None)],
            current: 1,
        };
        let now = Local.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap();
        let (lines, headers) = reading.layout(now);
        let lines = text(&lines);
        assert_eq!(headers.len(), 2);
        assert!(lines[headers[0]].contains("Ada"), "{lines:?}");
        assert!(
            lines[headers[1]].contains("Bea") && lines[headers[1]].starts_with('▶'),
            "{lines:?}"
        );
        assert!(lines.contains(&"First".to_owned()), "{lines:?}");
        assert!(
            lines.contains(&"…".to_owned()),
            "a body on its way says so: {lines:?}"
        );
    }

    #[test]
    fn a_message_on_its_own_has_no_header_of_its_own() {
        let reading = Reading {
            row: MessageId::new(1),
            members: vec![member(1, "Ada", Some("Only"))],
            current: 0,
        };
        let now = Local.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap();
        let (lines, _) = reading.layout(now);
        assert_eq!(text(&lines), vec!["Only".to_owned()]);
    }
}
