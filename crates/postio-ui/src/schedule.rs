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
//! A digest's due time is computed here too ([`next_due`]), on the calendar
//! and through `parse_when`'s rule for a wall-clock time, so that it keeps
//! its time across a change of the clocks.
//!
//! A moment both tables offer is worded once, here, too (spec C14): 6pm is
//! "Later today" in the snooze picker and the schedule-send picker alike,
//! and "Tomorrow evening" once today's has passed ([`evening`]).

use chrono::{DateTime, Datelike, Days, Duration, Local, Months, NaiveDate, TimeZone, Weekday};
use postio_config::Due;
use postio_search::date::resolve_local;

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
/// "Later today" rolls to tomorrow's evening, and says so, once 6pm today
/// is behind `now`.
/// "Monday morning" always means a Monday strictly after today: opening the
/// picker on a Monday offers next week's, not the one already underway.
pub fn schedule_presets(now: DateTime<Local>) -> [(&'static str, DateTime<Local>); 4] {
    [
        ("In 1 hour", now + Duration::hours(1)),
        evening(now),
        ("Tomorrow morning", tomorrow_morning(now)),
        ("Monday morning", monday_morning(now)),
    ]
}

/// The snooze picker's four times (spec US5 scenario 1, screen 11).
///
/// Snoozed mail leaves the inbox and comes back at the chosen time. The
/// first is the schedule-send picker's evening, in its words ([`evening`]);
/// "Next week" is this weekday a week on, in the morning.
pub fn snooze_presets(now: DateTime<Local>) -> [(&'static str, DateTime<Local>); 4] {
    [
        evening(now),
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

/// The evening preset both pickers offer, worded as what it is: "Later
/// today" while 6pm today is still ahead, and "Tomorrow evening" once it
/// is not.
///
/// Spec C14 asked for one wording. "Later today" is the screen's and the
/// spec's own (screen 11, US5 scenario 1), and the schedule-send picker had
/// the other; the words were chosen to match the design, and the roll, so
/// that neither picker names today for a time that is tomorrow.
fn evening(now: DateTime<Local>) -> (&'static str, DateTime<Local>) {
    let at = this_evening(now);
    if at.date_naive() == now.date_naive() {
        ("Later today", at)
    } else {
        ("Tomorrow evening", at)
    }
}

/// 6pm today, or tomorrow's once today's is behind `now`.
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

/// When a digest rule next comes due after `after` (spec 007 T132,
/// contracts/config.md): the first time its cadence, day and time name that
/// is later than `after`.
///
/// `after` is the rule's last delivery, or when it was made, and its zone is
/// the one the rule's time is read in: `Local` for every caller, and a zone
/// with daylight saving for the tests that prove this.
///
/// Days are stepped on the calendar, never as 24-hour spans: across a change
/// of the clocks a day is 23 or 25 hours, and a week of 168 lands an hour
/// off the rule's time (the presets' #1700). The time is resolved by
/// [`parse_when`](postio_search::date::parse_when)'s own rule,
/// [`resolve_local`]: a time the clocks skip is pushed forward by the gap,
/// and one they repeat is its first occurrence. A day's time comes due
/// once, so a daily rule delivered at the first 02:30 of the night the
/// clocks go back is next due the next night, not at the second 02:30.
///
/// `None` only past the end of the calendar.
pub fn next_due<Tz: TimeZone>(due: &Due, after: &DateTime<Tz>) -> Option<DateTime<Tz>> {
    let zone = after.timezone();
    let (Due::Daily { at } | Due::Weekly { at, .. } | Due::Monthly { at, .. }) = *due;
    let mut day = first_day(due, after.date_naive())?;
    loop {
        let local = day.and_time(at);
        // Already come today, however the clocks repeat it.
        let come = zone
            .from_local_datetime(&local)
            .earliest()
            .is_some_and(|first| first <= *after);
        if !come && let Some(when) = resolve_local(&zone, local, after).filter(|when| when > after)
        {
            return Some(when);
        }
        day = next_day(due, day)?;
    }
}

/// The first day on or after `today` that `due` names.
fn first_day(due: &Due, today: NaiveDate) -> Option<NaiveDate> {
    match *due {
        Due::Daily { .. } => Some(today),
        Due::Weekly { day, .. } => {
            let ahead =
                (day.num_days_from_monday() + 7 - today.weekday().num_days_from_monday()) % 7;
            today.checked_add_days(Days::new(u64::from(ahead)))
        }
        Due::Monthly { day, .. } => {
            let this_month = today.with_day(day)?;
            if this_month >= today {
                Some(this_month)
            } else {
                this_month.checked_add_months(Months::new(1))
            }
        }
    }
}

/// The next day after `day` that `due` names, stepped on the calendar.
fn next_day(due: &Due, day: NaiveDate) -> Option<NaiveDate> {
    match due {
        Due::Daily { .. } => day.checked_add_days(Days::new(1)),
        Due::Weekly { .. } => day.checked_add_days(Days::new(7)),
        Due::Monthly { .. } => day.checked_add_months(Months::new(1)),
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
    fn the_evening_is_worded_alike_in_both_tables() {
        // Spec C14, settled: one table, one wording. Snooze's first preset
        // and schedule-send's second are the same moment, and say so in
        // the same words, whether the evening is still ahead or has passed.
        for now in [
            local_at(2026, 9, 26, 9, 0),
            local_at(2026, 9, 26, 17, 56),
            local_at(2026, 9, 26, 19, 0),
            local_at(2026, 9, 26, 23, 55),
        ] {
            assert_eq!(snooze_presets(now)[0], schedule_presets(now)[1], "at {now}");
        }
    }

    #[test]
    fn a_passed_evening_is_tomorrows_and_says_so() {
        // "Later today" for an instant tomorrow would be wrong about the
        // one thing it names.
        let now = local_at(2026, 9, 26, 19, 0);
        assert_eq!(
            snooze_presets(now)[0],
            ("Tomorrow evening", local_at(2026, 9, 27, 18, 0))
        );
        assert_eq!(
            schedule_presets(local_at(2026, 9, 26, 9, 0))[1],
            ("Later today", local_at(2026, 9, 26, 18, 0))
        );
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

/// `next_due` in a zone with daylight saving (spec 007 T132): a copy of
/// `postio_search::date`'s test zone, because `Local` cannot promise one on
/// whatever machine runs this.
#[cfg(test)]
mod digest_due_tests {
    use chrono::{
        DateTime, Duration, FixedOffset, MappedLocalTime, NaiveDate, NaiveDateTime, NaiveTime,
        TimeZone, Weekday,
    };
    use postio_config::Due;

    use super::next_due;

    /// Central European time for 2026: CET (+01:00) until 29 March 01:00
    /// UTC, CEST (+02:00) until 25 October 01:00 UTC, then CET again. 02:30
    /// on 29 March never happens there, and 02:30 on 25 October happens
    /// twice.
    #[derive(Debug, Clone, Copy)]
    struct Cet2026;

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .and_then(|date| date.and_hms_opt(h, min, 0))
            .expect("a real date")
    }

    fn hours(h: i32) -> FixedOffset {
        FixedOffset::east_opt(h * 3600).expect("a real offset")
    }

    impl TimeZone for Cet2026 {
        type Offset = FixedOffset;

        fn from_offset(_: &FixedOffset) -> Self {
            Cet2026
        }

        fn offset_from_utc_datetime(&self, at: &NaiveDateTime) -> FixedOffset {
            if *at >= utc(2026, 3, 29, 1, 0) && *at < utc(2026, 10, 25, 1, 0) {
                hours(2)
            } else {
                hours(1)
            }
        }

        fn offset_from_utc_date(&self, at: &NaiveDate) -> FixedOffset {
            self.offset_from_utc_datetime(&at.and_hms_opt(0, 0, 0).expect("midnight"))
        }

        fn offset_from_local_datetime(
            &self,
            local: &NaiveDateTime,
        ) -> MappedLocalTime<FixedOffset> {
            let fits = |offset: FixedOffset| {
                let at = *local - Duration::seconds(i64::from(offset.local_minus_utc()));
                self.offset_from_utc_datetime(&at) == offset
            };
            match (fits(hours(2)), fits(hours(1))) {
                (true, true) => MappedLocalTime::Ambiguous(hours(2), hours(1)),
                (true, false) => MappedLocalTime::Single(hours(2)),
                (false, true) => MappedLocalTime::Single(hours(1)),
                (false, false) => MappedLocalTime::None,
            }
        }

        fn offset_from_local_date(&self, local: &NaiveDate) -> MappedLocalTime<FixedOffset> {
            self.offset_from_local_datetime(&local.and_hms_opt(0, 0, 0).expect("midnight"))
        }
    }

    /// A wall-clock time in the test zone, which must exist exactly once.
    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Cet2026> {
        Cet2026
            .with_ymd_and_hms(y, m, d, h, min, 0)
            .single()
            .expect("an unambiguous wall-clock time")
    }

    fn time(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).expect("a time")
    }

    /// Spec US10 scenario 1: a weekly rule due Sunday 09:00, and mail held
    /// on Wednesday.
    #[test]
    fn a_weekly_digest_comes_due_on_its_day_at_its_time() {
        let sunday = Due::Weekly {
            day: Weekday::Sun,
            at: time(9, 0),
        };
        let wednesday = at(2026, 9, 23, 12, 0);
        assert_eq!(next_due(&sunday, &wednesday), Some(at(2026, 9, 27, 9, 0)));
        // Delivered then, it is next due a week on, not again that morning.
        let delivered = at(2026, 9, 27, 9, 0);
        assert_eq!(next_due(&sunday, &delivered), Some(at(2026, 10, 4, 9, 0)));
    }

    /// The clocks go forward on Sunday 29 March: a week is 167 hours, and
    /// the digest still comes at 09:00. Stepped as 168 hours it would come
    /// at 10:00.
    #[test]
    fn a_weekly_digest_keeps_its_time_when_the_clocks_go_forward() {
        let sunday = Due::Weekly {
            day: Weekday::Sun,
            at: time(9, 0),
        };
        let delivered = at(2026, 3, 22, 9, 0);
        let next = next_due(&sunday, &delivered).expect("a next time");
        assert_eq!(next.naive_local(), utc(2026, 3, 29, 9, 0));
        assert_eq!(next - delivered, Duration::hours(167));
    }

    /// The clocks go back on Sunday 25 October: a week is 169 hours, and the
    /// digest still comes at 09:00. Stepped as 168 hours it would come at
    /// 08:00.
    #[test]
    fn a_weekly_digest_keeps_its_time_when_the_clocks_go_back() {
        let sunday = Due::Weekly {
            day: Weekday::Sun,
            at: time(9, 0),
        };
        let delivered = at(2026, 10, 18, 9, 0);
        let next = next_due(&sunday, &delivered).expect("a next time");
        assert_eq!(next.naive_local(), utc(2026, 10, 25, 9, 0));
        assert_eq!(next - delivered, Duration::hours(169));
    }

    /// `parse_when`'s rule: a time the clocks skip is pushed forward by the
    /// gap, so 02:30 on 29 March is 03:30 CEST.
    #[test]
    fn a_time_the_clocks_skip_is_pushed_forward_by_the_gap() {
        let night = Due::Daily { at: time(2, 30) };
        let next = next_due(&night, &at(2026, 3, 28, 12, 0)).expect("a next time");
        assert_eq!(next, Cet2026.from_utc_datetime(&utc(2026, 3, 29, 1, 30)));
        assert_eq!(next.naive_local(), utc(2026, 3, 29, 3, 30));
    }

    /// A time the clocks repeat is its first occurrence, and it comes due
    /// once: delivered at the first 02:30 on 25 October, the rule is next
    /// due on the 26th, not an hour later at the second.
    #[test]
    fn a_time_the_clocks_repeat_comes_due_once_at_its_first_occurrence() {
        let night = Due::Daily { at: time(2, 30) };
        let first = next_due(&night, &at(2026, 10, 24, 12, 0)).expect("a next time");
        assert_eq!(first, Cet2026.from_utc_datetime(&utc(2026, 10, 25, 0, 30)));
        let next = next_due(&night, &first).expect("a next time");
        assert_eq!(next, at(2026, 10, 26, 2, 30));
    }

    /// A daily rule is due later today while its time is ahead, and
    /// tomorrow once it is not.
    #[test]
    fn a_daily_digest_comes_today_while_its_time_is_ahead() {
        let four = Due::Daily { at: time(16, 0) };
        assert_eq!(
            next_due(&four, &at(2026, 9, 26, 8, 0)),
            Some(at(2026, 9, 26, 16, 0))
        );
        assert_eq!(
            next_due(&four, &at(2026, 9, 26, 16, 0)),
            Some(at(2026, 9, 27, 16, 0))
        );
    }

    /// A monthly rule comes on its day of the month, this month while it is
    /// ahead, and next month's once it is not.
    #[test]
    fn a_monthly_digest_comes_on_its_day_of_the_month() {
        let third = Due::Monthly {
            day: 3,
            at: time(9, 0),
        };
        assert_eq!(
            next_due(&third, &at(2026, 1, 2, 12, 0)),
            Some(at(2026, 1, 3, 9, 0))
        );
        assert_eq!(
            next_due(&third, &at(2026, 1, 3, 9, 0)),
            Some(at(2026, 2, 3, 9, 0))
        );
        assert_eq!(
            next_due(&third, &at(2026, 12, 20, 9, 0)),
            Some(at(2027, 1, 3, 9, 0))
        );
    }
}
