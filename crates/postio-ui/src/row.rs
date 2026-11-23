//! Row geometry per density, in logical pixels, shared so "compact" means the same on both
//! platforms.

use chrono::{DateTime, Datelike, Local, Utc};
use postio_config::Density;
use postio_core::CommandId;
use postio_model::EmailAddress;

/// In logical pixels. The airy numbers are measured off the canvas; the others tighten the same
/// anatomy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub pad_y: f32,
    /// Includes the accent edge, so a row does not shift sideways when selected.
    pub inset: f32,
    pub avatar: f32,
    pub gap: f32,
    pub subject_gap: f32,
    pub snippet: bool,
}

impl Metrics {
    pub fn for_density(density: Density) -> Self {
        match density {
            Density::Airy => Metrics {
                pad_y: 11.0,
                inset: 21.0,
                avatar: 30.0,
                gap: 12.0,
                subject_gap: 3.0,
                snippet: true,
            },
            Density::Comfortable => Metrics {
                pad_y: 8.0,
                inset: 18.0,
                avatar: 26.0,
                gap: 10.0,
                subject_gap: 2.0,
                snippet: true,
            },
            Density::Compact => Metrics {
                pad_y: 5.0,
                inset: 15.0,
                avatar: 22.0,
                gap: 9.0,
                subject_gap: 1.0,
                snippet: false,
            },
        }
    }
}

/// With no display name the local part stands in, so a mailing list reads `LK`.
pub fn initials(from: Option<&EmailAddress>) -> String {
    let Some(from) = from else {
        return "?".to_string();
    };
    let source = match &from.name {
        Some(name) if !name.trim().is_empty() => name.as_str(),
        _ => from.local_part().unwrap_or(""),
    };
    let words: Vec<&str> = source
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let letters: String = match words.as_slice() {
        [] => return "?".to_string(),
        [one] => one.chars().take(2).collect(),
        [first, second, ..] => first
            .chars()
            .take(1)
            .chain(second.chars().take(1))
            .collect(),
    };
    letters.to_uppercase()
}

/// Past the year it carries the year: "12 Aug" two years ago would mislead.
pub fn timestamp(received: DateTime<Utc>, now: DateTime<Local>) -> String {
    let local = received.with_timezone(&now.timezone());
    let days = (now.date_naive() - local.date_naive()).num_days();
    match days {
        0 => local.format("%H:%M").to_string(),
        1..=6 => local.format("%a").to_string(),
        _ if local.year() == now.year() => local.format("%-d %b").to_string(),
        _ => local.format("%-d %b %y").to_string(),
    }
}

/// The three triage verbs, the same as the bulk bar; always registry commands (`a`, `s`, `d`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowAction {
    Archive,
    Flag,
    Delete,
}

impl RowAction {
    pub const ALL: [RowAction; 3] = [RowAction::Archive, RowAction::Flag, RowAction::Delete];

    pub fn title(self) -> &'static str {
        match self {
            RowAction::Archive => "Archive",
            RowAction::Flag => "Flag",
            RowAction::Delete => "Delete",
        }
    }

    pub fn command(self) -> CommandId {
        match self {
            RowAction::Archive => CommandId::Archive,
            RowAction::Flag => CommandId::Flag,
            RowAction::Delete => CommandId::Delete,
        }
    }
}

/// Words for the user, not the state machine: a row saying only "Draft" made a message that needs
/// you look unfinished (#1491).
pub fn send_state_word(state: postio_model::DraftState) -> &'static str {
    use postio_model::DraftState;
    match state {
        DraftState::Editing => "Draft",
        DraftState::Queued => "Waiting to send",
        DraftState::Sending => "Sending",
        // "Not sent", not "Failed": the reason is on the draft itself (#1487).
        DraftState::Failed => "Not sent",
        // ADR 0021: nobody can say whether it arrived, so the word claims neither.
        DraftState::Unconfirmed => "Not confirmed",
        DraftState::Sent => "Sent",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn addr(name: Option<&str>, address: &str) -> EmailAddress {
        EmailAddress::new(name, address)
    }

    #[test]
    fn initials_are_the_canvas_two_letters() {
        assert_eq!(
            initials(Some(&addr(Some("Lena Tomlin"), "lena@example.com"))),
            "LT"
        );
        assert_eq!(
            initials(Some(&addr(Some("Nadia Okafor"), "nadia@example.com"))),
            "NO"
        );
        assert_eq!(
            initials(Some(&addr(Some("lkml"), "lkml@example.org"))),
            "LK"
        );
        assert_eq!(initials(Some(&addr(None, "buildbot@example.net"))), "BU");
        assert_eq!(initials(None), "?");
    }

    #[test]
    fn a_time_reads_as_the_clock_today_the_day_this_week_and_a_date_beyond() {
        // Instants are built in the local zone: the function renders in the reader's zone, so UTC
        // fixtures fail on offset machines.
        let now = Local.with_ymd_and_hms(2026, 9, 6, 18, 0, 0).unwrap();
        let at = |y, m, d, h, mi| {
            Local
                .with_ymd_and_hms(y, m, d, h, mi, 0)
                .unwrap()
                .with_timezone(&Utc)
        };

        assert_eq!(timestamp(at(2026, 9, 6, 9, 14), now), "09:14");
        let recent = timestamp(at(2026, 9, 3, 9, 14), now);
        assert_eq!(recent.len(), 3, "{recent}");
        assert!(!recent.contains(':'), "{recent}");
        assert_eq!(timestamp(at(2026, 8, 12, 9, 14), now), "12 Aug");
        assert_eq!(timestamp(at(2024, 8, 12, 9, 14), now), "12 Aug 24");
    }

    #[test]
    fn compact_drops_the_snippet_and_every_density_is_tighter_than_the_last() {
        let airy = Metrics::for_density(Density::Airy);
        let snug = Metrics::for_density(Density::Comfortable);
        let compact = Metrics::for_density(Density::Compact);

        assert!(airy.pad_y > snug.pad_y && snug.pad_y > compact.pad_y);
        assert!(airy.avatar > snug.avatar && snug.avatar > compact.avatar);
        assert!(airy.snippet && snug.snippet && !compact.snippet);
    }
}
