//! What a Focus row and the list around it say, with no toolkit in it
//! (spec 007 US1; contracts/focus-surface.md, "Rows").
//!
//! The row draws in one `snapshot()` in `postio-focus`; the rules behind
//! what it draws are here, where a unit test proves them in milliseconds:
//! which day heading a row sits under, how many label pills fit, and when
//! the conversation's count is worth a badge.

use chrono::{Datelike, NaiveDate};

/// The most label pills a row draws; the open message shows them all
/// (spec Edge Cases, "Labels").
pub const MAX_PILLS: usize = 2;

/// The heading a day's rows sit under: `Today · Saturday 26 September`,
/// `Yesterday · Friday 25 September`, then the day alone, with its year
/// once it is not this one.
pub fn day_heading(day: NaiveDate, today: NaiveDate) -> String {
    let date = if day.year() == today.year() {
        day.format("%A %-d %B").to_string()
    } else {
        day.format("%A %-d %B %Y").to_string()
    };
    match (today - day).num_days() {
        0 => format!("Today \u{b7} {date}"),
        1 => format!("Yesterday \u{b7} {date}"),
        _ => date,
    }
}

/// The badge a conversation's count draws, or none for a conversation of
/// one: the badge says how big the conversation is, and one is not big.
pub fn count_badge(message_count: u32) -> Option<String> {
    (message_count > 1).then(|| message_count.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("a real date")
    }

    #[test]
    fn a_heading_names_today_and_yesterday_and_then_the_day() {
        let today = day(2026, 9, 26);
        assert_eq!(day_heading(today, today), "Today · Saturday 26 September");
        assert_eq!(
            day_heading(day(2026, 9, 25), today),
            "Yesterday · Friday 25 September"
        );
        assert_eq!(
            day_heading(day(2026, 9, 23), today),
            "Wednesday 23 September"
        );
        assert_eq!(
            day_heading(day(2025, 12, 31), today),
            "Wednesday 31 December 2025",
            "a day in another year says which"
        );
    }

    #[test]
    fn a_conversation_of_one_has_no_badge() {
        assert_eq!(count_badge(1), None);
        assert_eq!(count_badge(0), None);
        assert_eq!(count_badge(3).as_deref(), Some("3"));
    }
}
