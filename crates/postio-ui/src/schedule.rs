//! The times a picker offers: when a scheduled send leaves, when snoozed
//! mail comes back, and when a reminder falls due.
//!
//! One table, shared so every app's pickers offer the same times computed
//! the same way (spec FR-043): the classic composer's popover and the
//! terminal's list read [`schedule_presets`], and Focus's snooze and remind
//! pickers read [`snooze_presets`] and [`remind_presets`]. Where two of them
//! mean the same moment -- this evening, tomorrow morning, Monday morning --
//! they compute it with the same function, so they cannot drift apart.
//!
//! Two of those shared moments are still worded two ways: the schedule-send
//! picker says "This evening" where the snooze picker says "Later today"
//! (spec C14). Which one both say is `/ux-architect`'s call; until it is
//! made, a test holds them to the same instant, so making it changes words
//! and nothing else.

use chrono::{DateTime, Datelike, Duration, Local, Weekday};

/// A preset must land at least this far ahead of `now` to be offered as
/// "today" rather than rolling to tomorrow — a picker opened one minute
/// before 6pm must not offer "this evening" for an instant already gone.
const MIN_SCHEDULE_LEAD: Duration = Duration::minutes(5);

/// `day` at the given wall-clock hour and minute, in `day`'s own local zone.
///
/// A DST transition can make a wall-clock time ambiguous or nonexistent;
/// falling back to `day` itself rather than panicking keeps a schedule-send
/// picker from crashing the composer on the two days a year this can happen,
/// at the cost of an odd-looking preset on exactly those days.
fn at_local_time(day: DateTime<Local>, hour: u32, minute: u32) -> DateTime<Local> {
    day.date_naive()
        .and_hms_opt(hour, minute, 0)
        .and_then(|naive| naive.and_local_timezone(Local).single())
        .unwrap_or(day)
}

/// The fixed times the schedule-send picker offers, computed
/// against `now` — recomputed every time the picker opens rather than once,
/// since "in 1 hour" a picker opened yesterday is not "in 1 hour" today.
///
/// "This evening" rolls to tomorrow once 6pm today is behind `now`.
/// "Monday morning" always means a Monday strictly after today: opening the
/// picker on a Monday offers next week's, not the one already underway.
pub fn schedule_presets(now: DateTime<Local>) -> [(&'static str, DateTime<Local>); 4] {
    [
        ("In 1 hour", now + Duration::hours(1)),
        ("This evening", this_evening(now)),
        ("Tomorrow morning", tomorrow_morning(now)),
        ("Monday morning", monday_morning(now)),
    ]
}

/// The snooze picker's four times (spec US5 scenario 1, screen 11).
///
/// Snoozed mail leaves the inbox and comes back at the chosen time. "Later
/// today" is the schedule-send picker's "This evening" under another name
/// (spec C14), rolling to tomorrow's once 6pm is behind `now`; "Next week"
/// is this weekday a week on, in the morning.
pub fn snooze_presets(now: DateTime<Local>) -> [(&'static str, DateTime<Local>); 4] {
    [
        ("Later today", this_evening(now)),
        ("Tomorrow morning", tomorrow_morning(now)),
        ("Monday morning", monday_morning(now)),
        ("Next week", at_local_time(now + Duration::days(7), 8, 0)),
    ]
}

/// The remind-if-no-reply picker's four times (spec US5, screen 12).
///
/// A reminder is a working-hours thing, so every one of them is 9am. "In 2
/// working days" counts Monday to Friday after today, so on a Saturday it
/// means Tuesday. "End of the week" is the next Friday morning still ahead
/// of `now`: this Friday's until 9am on Friday, then next Friday's.
pub fn remind_presets(now: DateTime<Local>) -> [(&'static str, DateTime<Local>); 4] {
    [
        ("Tomorrow", at_local_time(now + Duration::days(1), 9, 0)),
        (
            "In 2 working days",
            at_local_time(working_days_after(now, 2), 9, 0),
        ),
        ("End of the week", end_of_the_week(now)),
        ("In a week", at_local_time(now + Duration::days(7), 9, 0)),
    ]
}

/// 6pm today, or tomorrow's once today's is behind `now`: the schedule-send
/// picker's "This evening" and the snooze picker's "Later today".
fn this_evening(now: DateTime<Local>) -> DateTime<Local> {
    let evening = at_local_time(now, 18, 0);
    if evening < now + MIN_SCHEDULE_LEAD {
        at_local_time(now + Duration::days(1), 18, 0)
    } else {
        evening
    }
}

/// 8am tomorrow.
fn tomorrow_morning(now: DateTime<Local>) -> DateTime<Local> {
    at_local_time(now + Duration::days(1), 8, 0)
}

/// 8am on the first Monday strictly after today: on a Monday, next week's.
fn monday_morning(now: DateTime<Local>) -> DateTime<Local> {
    let days_from_monday = now.weekday().num_days_from_monday() as i64;
    let days_until_monday = if days_from_monday == 0 {
        7
    } else {
        7 - days_from_monday
    };
    at_local_time(now + Duration::days(days_until_monday), 8, 0)
}

/// `now`, moved on by `count` working days: Monday to Friday, counting from
/// the day after today.
fn working_days_after(now: DateTime<Local>, count: u32) -> DateTime<Local> {
    let mut day = now;
    let mut left = count;
    while left > 0 {
        day += Duration::days(1);
        if !matches!(day.weekday(), Weekday::Sat | Weekday::Sun) {
            left -= 1;
        }
    }
    day
}

/// 9am on the first Friday still ahead of `now`.
fn end_of_the_week(now: DateTime<Local>) -> DateTime<Local> {
    let days_to_friday =
        (Weekday::Fri.num_days_from_monday() + 7 - now.weekday().num_days_from_monday()) % 7;
    let friday = at_local_time(now + Duration::days(i64::from(days_to_friday)), 9, 0);
    if friday < now + MIN_SCHEDULE_LEAD {
        at_local_time(now + Duration::days(i64::from(days_to_friday) + 7), 9, 0)
    } else {
        friday
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn local_at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .expect("an unambiguous local time")
    }

    /// Saturday 26 September 2026, 16:09: the clock screen 11 is drawn at.
    fn saturday_afternoon() -> DateTime<Local> {
        local_at(2026, 9, 26, 16, 9)
    }

    fn preset(presets: &[(&'static str, DateTime<Local>); 4], label: &str) -> DateTime<Local> {
        presets
            .iter()
            .find(|(name, _)| *name == label)
            .unwrap_or_else(|| panic!("no `{label}` preset in {presets:?}"))
            .1
    }

    #[test]
    fn snooze_offers_screen_elevens_four_times_on_a_saturday_afternoon() {
        assert_eq!(
            snooze_presets(saturday_afternoon()),
            [
                ("Later today", local_at(2026, 9, 26, 18, 0)),
                ("Tomorrow morning", local_at(2026, 9, 27, 8, 0)),
                ("Monday morning", local_at(2026, 9, 28, 8, 0)),
                ("Next week", local_at(2026, 10, 3, 8, 0)),
            ]
        );
    }

    #[test]
    fn remind_offers_its_four_times_on_a_saturday_afternoon() {
        assert_eq!(
            remind_presets(saturday_afternoon()),
            [
                ("Tomorrow", local_at(2026, 9, 27, 9, 0)),
                ("In 2 working days", local_at(2026, 9, 29, 9, 0)),
                ("End of the week", local_at(2026, 10, 2, 9, 0)),
                ("In a week", local_at(2026, 10, 3, 9, 0)),
            ]
        );
    }

    #[test]
    fn later_today_is_the_schedule_pickers_this_evening_in_other_words() {
        // Spec C14: one table, one wording still to choose. Until
        // `/ux-architect` chooses it, the two labels must at least name the
        // same instant -- including when the evening has passed and both
        // roll to tomorrow's -- so choosing is a change of words only.
        for now in [
            local_at(2026, 9, 26, 9, 0),
            local_at(2026, 9, 26, 17, 56),
            local_at(2026, 9, 26, 19, 0),
            local_at(2026, 9, 26, 23, 55),
        ] {
            assert_eq!(
                preset(&snooze_presets(now), "Later today"),
                preset(&schedule_presets(now), "This evening"),
                "at {now}"
            );
        }
    }

    #[test]
    fn two_working_days_step_over_the_weekend() {
        for (now, expected) in [
            // Thursday: Friday is one, Monday two.
            (local_at(2026, 10, 1, 10, 0), local_at(2026, 10, 5, 9, 0)),
            // Friday: Monday and Tuesday.
            (local_at(2026, 10, 2, 10, 0), local_at(2026, 10, 6, 9, 0)),
            // Monday: Tuesday and Wednesday.
            (local_at(2026, 9, 28, 10, 0), local_at(2026, 9, 30, 9, 0)),
            // Sunday: Monday and Tuesday.
            (local_at(2026, 9, 27, 10, 0), local_at(2026, 9, 29, 9, 0)),
        ] {
            assert_eq!(
                preset(&remind_presets(now), "In 2 working days"),
                expected,
                "from {now}"
            );
        }
    }

    #[test]
    fn the_end_of_the_week_is_the_next_friday_morning_still_ahead() {
        for (now, expected) in [
            (local_at(2026, 10, 1, 10, 0), local_at(2026, 10, 2, 9, 0)),
            // Friday before nine: this morning is still ahead.
            (local_at(2026, 10, 2, 8, 0), local_at(2026, 10, 2, 9, 0)),
            // Too close to call it ahead, and after it: next Friday.
            (local_at(2026, 10, 2, 8, 58), local_at(2026, 10, 9, 9, 0)),
            (local_at(2026, 10, 2, 10, 0), local_at(2026, 10, 9, 9, 0)),
        ] {
            assert_eq!(
                preset(&remind_presets(now), "End of the week"),
                expected,
                "from {now}"
            );
        }
    }

    #[test]
    fn every_snooze_and_remind_preset_is_ahead_of_now() {
        for now in [
            local_at(2026, 9, 26, 0, 5),
            saturday_afternoon(),
            local_at(2026, 9, 27, 23, 55),
            local_at(2026, 9, 28, 17, 59),
            local_at(2026, 10, 2, 23, 59),
        ] {
            for (label, when) in snooze_presets(now).into_iter().chain(remind_presets(now)) {
                assert!(when > now, "{label} is not ahead of {now}: {when}");
            }
        }
    }
}
