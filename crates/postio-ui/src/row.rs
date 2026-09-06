//! Canvas 1b's row geometry, per density.
//!
//! Plain numbers in logical pixels, keyed off `postio_config::Density`. They
//! live here rather than in either frontend because "compact" has to mean the
//! same thing in both: a row that is 26px on one platform and 34 on the other
//! is not a shared setting, it is two settings with one name in the file.
//!
//! Type and colour are not here — those come from each toolkit's cascade.
//! This is only the layout the row arranges them in.

use chrono::{DateTime, Datelike, Local, Utc};
use postio_config::Density;
use postio_core::{CommandId, Keymap};
use postio_model::EmailAddress;

/// Canvas 1b's row geometry for one density, in logical pixels.
///
/// Type and colour come from the cascade ([`Palette`]); this is the layout
/// the snapshot arranges them in, which a hand-drawn widget owns the way a
/// `GtkBox` owns its spacing. The airy numbers are measured straight off the
/// canvas; the other two tighten the same anatomy rather than changing it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// Space above and below the row's content.
    pub pad_y: f32,
    /// How far in the content starts, accent edge included, so a row does
    /// not shift sideways when the selection lands on it.
    pub inset: f32,
    /// The avatar chip, square.
    pub avatar: f32,
    /// Between the avatar and the text column.
    pub gap: f32,
    /// Between the sender line and the subject.
    pub subject_gap: f32,
    /// Between the snippet and the key hints the focused row reveals.
    pub hints_gap: f32,
    /// Whether the snippet line is drawn at all.
    pub snippet: bool,
}

impl Metrics {
    /// The geometry `density` asks for.
    pub fn for_density(density: Density) -> Self {
        match density {
            Density::Airy => Metrics {
                pad_y: 11.0,
                inset: 21.0,
                avatar: 30.0,
                gap: 12.0,
                subject_gap: 3.0,
                hints_gap: 7.0,
                snippet: true,
            },
            Density::Comfortable => Metrics {
                pad_y: 8.0,
                inset: 18.0,
                avatar: 26.0,
                gap: 10.0,
                subject_gap: 2.0,
                hints_gap: 5.0,
                snippet: true,
            },
            // The tightest setting is for triage, where the question is how
            // many subjects fit on screen. The snippet is the line that
            // costs the most and answers it least.
            Density::Compact => Metrics {
                pad_y: 5.0,
                inset: 15.0,
                avatar: 22.0,
                gap: 9.0,
                subject_gap: 1.0,
                hints_gap: 4.0,
                snippet: false,
            },
        }
    }
}

/// The initials the avatar chip shows for `from`.
///
/// Two letters: the initials of the first two words of a display name, or
/// the first two letters of a single word. With no display name the local
/// part stands in, which is what makes a mailing list read as `LK` rather
/// than as a shrug.
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

/// The timestamp column: relative for today, absolute beyond.
///
/// Canvas 1b draws `09:14` and `Thu`. Past the week it becomes a date, and
/// past the year it carries the year, because "12 Aug" two years ago is a
/// lie the eye believes.
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

/// The commands the focused row hints at, and the labels the canvas gives
/// them — canvas order, not registry order.
/// Two, not three. `t` used to be here, hinting at the drill-in column that
/// a thread row could open; the conversation is what the reading pane shows
/// the moment the cursor lands on the row, so there is no third verb to
/// announce (#1003).
const HINT_COMMANDS: [(CommandId, &str); 2] =
    [(CommandId::Reply, "reply"), (CommandId::Archive, "archive")];

/// The key hints the focused row announces, as `(key, label)` pairs.
///
/// Read from the keymap rather than from the registry's defaults, so a
/// rebinding reaches the hint — a row that taught the wrong key would be
/// worse than one that taught none.
pub fn hints(keymap: &Keymap) -> Vec<(String, &'static str)> {
    HINT_COMMANDS
        .iter()
        .filter_map(|(command, label)| {
            keymap
                .binding(*command)
                .map(|key| (key.to_string(), *label))
        })
        .collect()
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
        // Canvas 1b draws `09:14` and `Thu`. The year appears only once the
        // message is not from this one, because "12 Aug" two years ago is a
        // lie the eye believes.
        //
        // Every instant is built in the *local* zone and converted to UTC, not
        // the other way round: written the obvious way this asserted "09:14"
        // against a machine four hours off UTC and got "05:14". The function
        // renders in the reader's zone, so the test has to think in it too.
        let now = Local.with_ymd_and_hms(2026, 9, 6, 18, 0, 0).unwrap();
        let at = |y, m, d, h, mi| {
            Local
                .with_ymd_and_hms(y, m, d, h, mi, 0)
                .unwrap()
                .with_timezone(&Utc)
        };

        assert_eq!(timestamp(at(2026, 9, 6, 9, 14), now), "09:14");
        // Three days back is a weekday name, not a clock and not a date.
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
