//! The times pickers offer (schedule send, snooze, remind) and a digest's next due time, in one
//! table so every app agrees.
//! Days step on the calendar, not as 24-hour spans, so times survive a change of the clocks.

use chrono::{DateTime, Datelike, Days, Duration, Months, NaiveDate, NaiveTime, TimeZone, Weekday};
use postio_config::Due;
use postio_search::date::resolve_local;

/// A preset closer than this rolls to tomorrow, so a picker opened a minute before 6pm does not
/// offer a gone instant.
const MIN_SCHEDULE_LEAD: Duration = Duration::minutes(5);

/// Days step on the calendar: across a clock change a day is 23 or 25 hours (#1700). The time
/// resolves by `resolve_local`.
fn at_day<Tz: TimeZone>(now: &DateTime<Tz>, days: i64, hour: u32, minute: u32) -> DateTime<Tz> {
    let day = now
        .date_naive()
        .checked_add_signed(Duration::days(days))
        .unwrap_or_else(|| now.date_naive());
    let naive = day.and_time(NaiveTime::from_hms_opt(hour, minute, 0).unwrap_or_default());
    resolve_local(&now.timezone(), naive, now).unwrap_or_else(|| now.clone())
}

/// Recomputed each time the picker opens. "Monday morning" is strictly after today.
pub fn schedule_presets<Tz: TimeZone>(now: DateTime<Tz>) -> [(&'static str, DateTime<Tz>); 4] {
    [
        ("In 1 hour", now.clone() + Duration::hours(1)),
        evening(&now),
        ("Tomorrow morning", tomorrow_morning(&now)),
        ("Monday morning", monday_morning(&now)),
    ]
}

pub fn snooze_presets<Tz: TimeZone>(now: DateTime<Tz>) -> [(&'static str, DateTime<Tz>); 4] {
    [
        evening(&now),
        ("Tomorrow morning", tomorrow_morning(&now)),
        ("Monday morning", monday_morning(&now)),
        ("Next week", at_day(&now, 7, 8, 0)),
    ]
}

/// Every time is 9am. "In 2 working days" counts Monday to Friday; "End of the week" is the next
/// Friday morning still ahead.
pub fn remind_presets<Tz: TimeZone>(now: DateTime<Tz>) -> [(&'static str, DateTime<Tz>); 4] {
    [
        ("Tomorrow", at_day(&now, 1, 9, 0)),
        (
            "In 2 working days",
            at_day(&now, working_days_after(&now, 2), 9, 0),
        ),
        ("End of the week", end_of_the_week(&now)),
        ("In a week", at_day(&now, 7, 9, 0)),
    ]
}

/// "Later today" while 6pm is ahead, else "Tomorrow evening", so neither picker names today for
/// tomorrow's time.
fn evening<Tz: TimeZone>(now: &DateTime<Tz>) -> (&'static str, DateTime<Tz>) {
    let at = this_evening(now);
    if at.date_naive() == now.date_naive() {
        ("Later today", at)
    } else {
        ("Tomorrow evening", at)
    }
}

fn this_evening<Tz: TimeZone>(now: &DateTime<Tz>) -> DateTime<Tz> {
    let evening = at_day(now, 0, 18, 0);
    if evening < now.clone() + MIN_SCHEDULE_LEAD {
        at_day(now, 1, 18, 0)
    } else {
        evening
    }
}

fn tomorrow_morning<Tz: TimeZone>(now: &DateTime<Tz>) -> DateTime<Tz> {
    at_day(now, 1, 8, 0)
}

/// 8am on the first Monday strictly after today: on a Monday, next week's.
fn monday_morning<Tz: TimeZone>(now: &DateTime<Tz>) -> DateTime<Tz> {
    let days_from_monday = i64::from(now.weekday().num_days_from_monday());
    let days_until_monday = if days_from_monday == 0 {
        7
    } else {
        7 - days_from_monday
    };
    at_day(now, days_until_monday, 8, 0)
}

fn working_days_after<Tz: TimeZone>(now: &DateTime<Tz>, count: u32) -> i64 {
    let mut weekday = now.weekday();
    let mut days = 0;
    let mut left = count;
    while left > 0 {
        weekday = weekday.succ();
        days += 1;
        if !matches!(weekday, Weekday::Sat | Weekday::Sun) {
            left -= 1;
        }
    }
    days
}

fn end_of_the_week<Tz: TimeZone>(now: &DateTime<Tz>) -> DateTime<Tz> {
    let days_to_friday = i64::from(
        (Weekday::Fri.num_days_from_monday() + 7 - now.weekday().num_days_from_monday()) % 7,
    );
    let friday = at_day(now, days_to_friday, 9, 0);
    if friday < now.clone() + MIN_SCHEDULE_LEAD {
        at_day(now, days_to_friday + 7, 9, 0)
    } else {
        friday
    }
}

/// `after` is the rule's last delivery or creation; its zone is the one the rule's time is read in.
/// Days step on the calendar (#1700); a time the clocks skip is pushed forward by the gap, and one
/// they repeat is due once, at its first occurrence.
/// `None` only past the end of the calendar.
pub fn next_due<Tz: TimeZone>(due: &Due, after: &DateTime<Tz>) -> Option<DateTime<Tz>> {
    let zone = after.timezone();
    let (Due::Daily { at } | Due::Weekly { at, .. } | Due::Monthly { at, .. }) = *due;
    let mut day = first_day(due, after.date_naive())?;
    loop {
        let local = day.and_time(at);
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

fn next_day(due: &Due, day: NaiveDate) -> Option<NaiveDate> {
    match due {
        Due::Daily { .. } => day.checked_add_days(Days::new(1)),
        Due::Weekly { .. } => day.checked_add_days(Days::new(7)),
        Due::Monthly { .. } => day.checked_add_months(Months::new(1)),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Local, TimeZone};

    use super::*;

    fn local_at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .expect("an unambiguous local time")
    }

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
            (local_at(2026, 10, 1, 10, 0), local_at(2026, 10, 5, 9, 0)),
            (local_at(2026, 10, 2, 10, 0), local_at(2026, 10, 6, 9, 0)),
            (local_at(2026, 9, 28, 10, 0), local_at(2026, 9, 30, 9, 0)),
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
            (local_at(2026, 10, 2, 8, 0), local_at(2026, 10, 2, 9, 0)),
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

/// A copy of `postio_search::date`'s test zone: `Local` cannot promise daylight saving on every
/// machine.
#[cfg(test)]
mod digest_due_tests {
    use chrono::{
        DateTime, Duration, FixedOffset, MappedLocalTime, NaiveDate, NaiveDateTime, NaiveTime,
        TimeZone, Weekday,
    };
    use postio_config::Due;

    use super::{next_due, remind_presets, schedule_presets, snooze_presets};

    /// CET until 29 March 01:00 UTC, CEST until 25 October 01:00 UTC. 02:30 on 29 March never
    /// happens, on 25 October twice.
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

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Cet2026> {
        Cet2026
            .with_ymd_and_hms(y, m, d, h, min, 0)
            .single()
            .expect("an unambiguous wall-clock time")
    }

    fn time(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).expect("a time")
    }

    #[test]
    fn a_weekly_digest_comes_due_on_its_day_at_its_time() {
        let sunday = Due::Weekly {
            day: Weekday::Sun,
            at: time(9, 0),
        };
        let wednesday = at(2026, 9, 23, 12, 0);
        assert_eq!(next_due(&sunday, &wednesday), Some(at(2026, 9, 27, 9, 0)));
        let delivered = at(2026, 9, 27, 9, 0);
        assert_eq!(next_due(&sunday, &delivered), Some(at(2026, 10, 4, 9, 0)));
    }

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

    #[test]
    fn a_time_the_clocks_skip_is_pushed_forward_by_the_gap() {
        let night = Due::Daily { at: time(2, 30) };
        let next = next_due(&night, &at(2026, 3, 28, 12, 0)).expect("a next time");
        assert_eq!(next, Cet2026.from_utc_datetime(&utc(2026, 3, 29, 1, 30)));
        assert_eq!(next.naive_local(), utc(2026, 3, 29, 3, 30));
    }

    #[test]
    fn a_time_the_clocks_repeat_comes_due_once_at_its_first_occurrence() {
        let night = Due::Daily { at: time(2, 30) };
        let first = next_due(&night, &at(2026, 10, 24, 12, 0)).expect("a next time");
        assert_eq!(first, Cet2026.from_utc_datetime(&utc(2026, 10, 25, 0, 30)));
        let next = next_due(&night, &first).expect("a next time");
        assert_eq!(next, at(2026, 10, 26, 2, 30));
    }

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
    fn seen(presets: &[(&'static str, DateTime<Cet2026>)]) -> Vec<(&'static str, String)> {
        presets
            .iter()
            .map(|(label, when)| {
                (
                    *label,
                    when.naive_local().format("%a %d %b %H:%M").to_string(),
                )
            })
            .collect()
    }

    fn expect(rows: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
        rows.iter()
            .map(|(label, when)| (*label, (*when).to_owned()))
            .collect()
    }

    /// #1700: presets step calendar days, never 24-hour spans.
    #[test]
    fn presets_land_on_the_next_calendar_day_across_a_change_of_the_clocks() {
        type Offered<'a> = Vec<(&'a str, &'a str)>;
        let rows: [(DateTime<Cet2026>, [Offered; 3]); 4] = [
            (
                at(2026, 3, 28, 23, 30),
                [
                    vec![
                        ("Tomorrow evening", "Sun 29 Mar 18:00"),
                        ("Tomorrow morning", "Sun 29 Mar 08:00"),
                        ("Monday morning", "Mon 30 Mar 08:00"),
                    ],
                    vec![
                        ("Tomorrow evening", "Sun 29 Mar 18:00"),
                        ("Tomorrow morning", "Sun 29 Mar 08:00"),
                        ("Monday morning", "Mon 30 Mar 08:00"),
                        ("Next week", "Sat 04 Apr 08:00"),
                    ],
                    vec![
                        ("Tomorrow", "Sun 29 Mar 09:00"),
                        ("In 2 working days", "Tue 31 Mar 09:00"),
                        ("End of the week", "Fri 03 Apr 09:00"),
                        ("In a week", "Sat 04 Apr 09:00"),
                    ],
                ],
            ),
            (
                at(2026, 10, 25, 0, 30),
                [
                    vec![
                        ("Later today", "Sun 25 Oct 18:00"),
                        ("Tomorrow morning", "Mon 26 Oct 08:00"),
                        ("Monday morning", "Mon 26 Oct 08:00"),
                    ],
                    vec![
                        ("Later today", "Sun 25 Oct 18:00"),
                        ("Tomorrow morning", "Mon 26 Oct 08:00"),
                        ("Monday morning", "Mon 26 Oct 08:00"),
                        ("Next week", "Sun 01 Nov 08:00"),
                    ],
                    vec![
                        ("Tomorrow", "Mon 26 Oct 09:00"),
                        ("In 2 working days", "Tue 27 Oct 09:00"),
                        ("End of the week", "Fri 30 Oct 09:00"),
                        ("In a week", "Sun 01 Nov 09:00"),
                    ],
                ],
            ),
            (
                at(2026, 10, 24, 23, 30),
                [
                    vec![
                        ("Tomorrow evening", "Sun 25 Oct 18:00"),
                        ("Tomorrow morning", "Sun 25 Oct 08:00"),
                        ("Monday morning", "Mon 26 Oct 08:00"),
                    ],
                    vec![
                        ("Tomorrow evening", "Sun 25 Oct 18:00"),
                        ("Tomorrow morning", "Sun 25 Oct 08:00"),
                        ("Monday morning", "Mon 26 Oct 08:00"),
                        ("Next week", "Sat 31 Oct 08:00"),
                    ],
                    vec![
                        ("Tomorrow", "Sun 25 Oct 09:00"),
                        ("In 2 working days", "Tue 27 Oct 09:00"),
                        ("End of the week", "Fri 30 Oct 09:00"),
                        ("In a week", "Sat 31 Oct 09:00"),
                    ],
                ],
            ),
            (
                at(2026, 3, 26, 23, 30),
                [
                    vec![
                        ("Tomorrow evening", "Fri 27 Mar 18:00"),
                        ("Tomorrow morning", "Fri 27 Mar 08:00"),
                        ("Monday morning", "Mon 30 Mar 08:00"),
                    ],
                    vec![
                        ("Tomorrow evening", "Fri 27 Mar 18:00"),
                        ("Tomorrow morning", "Fri 27 Mar 08:00"),
                        ("Monday morning", "Mon 30 Mar 08:00"),
                        ("Next week", "Thu 02 Apr 08:00"),
                    ],
                    vec![
                        ("Tomorrow", "Fri 27 Mar 09:00"),
                        ("In 2 working days", "Mon 30 Mar 09:00"),
                        ("End of the week", "Fri 27 Mar 09:00"),
                        ("In a week", "Thu 02 Apr 09:00"),
                    ],
                ],
            ),
        ];
        for (now, [schedule, snooze, remind]) in rows {
            let sending = schedule_presets(now);
            assert_eq!(sending[0].1, now + Duration::hours(1), "from {now}");
            assert_eq!(
                seen(&sending[1..]),
                expect(&schedule),
                "schedule-send from {now}"
            );
            assert_eq!(
                seen(&snooze_presets(now)),
                expect(&snooze),
                "snooze from {now}"
            );
            assert_eq!(
                seen(&remind_presets(now)),
                expect(&remind),
                "remind from {now}"
            );
        }
    }
}
