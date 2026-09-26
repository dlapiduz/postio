//! When a scheduled send can be asked to leave: the picker's four times.
//!
//! Shared so the desktop's popover and the terminal's list offer the same
//! times, computed the same way.

use chrono::{DateTime, Datelike, Duration, Local};

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

/// The fixed times The schedule-send picker offers, computed
/// against `now` — recomputed every time the picker opens rather than once,
/// since "in 1 hour" a picker opened yesterday is not "in 1 hour" today.
///
/// "This evening" rolls to tomorrow once 6pm today is behind `now`.
/// "Monday morning" always means a Monday strictly after today: opening the
/// picker on a Monday offers next week's, not the one already underway.
pub fn schedule_presets(now: DateTime<Local>) -> [(&'static str, DateTime<Local>); 4] {
    let in_one_hour = now + Duration::hours(1);

    let mut evening = at_local_time(now, 18, 0);
    if evening < now + MIN_SCHEDULE_LEAD {
        evening = at_local_time(now + Duration::days(1), 18, 0);
    }

    let tomorrow_morning = at_local_time(now + Duration::days(1), 8, 0);

    let days_from_monday = now.weekday().num_days_from_monday() as i64;
    let days_until_monday = if days_from_monday == 0 {
        7
    } else {
        7 - days_from_monday
    };
    let monday_morning = at_local_time(now + Duration::days(days_until_monday), 8, 0);

    [
        ("In 1 hour", in_one_hour),
        ("This evening", evening),
        ("Tomorrow morning", tomorrow_morning),
        ("Monday morning", monday_morning),
    ]
}
