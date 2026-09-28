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

/// What a marked row's second line says (contracts/focus-surface.md,
/// "Rows"): the kind chip, the date, the quoted sentence, and the actions
/// that answer it -- or, once answered or when it cannot be, what is true.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerLine {
    /// The kind chip: "Invite", "Question", "To-do" or "No reply".
    pub chip: &'static str,
    /// The date: "Tue 29 Sep · 10:00–10:45", "Wed 30 Sep", "since Sat 26 Sep".
    pub date: Option<String>,
    /// The sentence the marker is about, verbatim, for a question or to-do.
    pub quote: Option<String>,
    /// What stands where the actions would: "Accepted", "Cancelled", "Past".
    pub status: Option<&'static str>,
    /// The commands that answer it, in order, each with its button's words.
    pub actions: Vec<(postio_core::CommandId, &'static str)>,
}

/// The second line for `marker`, as of `now`, in `zone`.
///
/// An invitation's "past" is read from its end time here, when the row is
/// drawn: the marker is not flipped as time passes (T110), so a row that
/// has been on screen across the end of its event stops offering an answer
/// without anything being written.
pub fn marker_line<Tz: chrono::TimeZone>(
    marker: &postio_model::listing::MarkerSummary,
    now: chrono::DateTime<chrono::Utc>,
    zone: &Tz,
) -> MarkerLine
where
    Tz::Offset: std::fmt::Display,
{
    use postio_core::CommandId;
    use postio_model::listing::{InviteAnswer, MarkerKind, MarkerWhen};

    let day =
        |at: &chrono::DateTime<chrono::Utc>| at.with_timezone(zone).format("%a %-d %b").to_string();
    let date = marker.when.as_ref().map(|when| match when {
        MarkerWhen::Event { starts_at, ends_at } => format!(
            "{} \u{b7} {}\u{2013}{}",
            day(starts_at),
            starts_at.with_timezone(zone).format("%H:%M"),
            ends_at.with_timezone(zone).format("%H:%M"),
        ),
        MarkerWhen::Due(at) if marker.kind == MarkerKind::NoReply => format!("since {}", day(at)),
        MarkerWhen::Due(at) => day(at),
    });
    let quote = match marker.kind {
        MarkerKind::Question | MarkerKind::Todo => marker.excerpt.clone(),
        MarkerKind::Invite | MarkerKind::NoReply => None,
    };
    let (chip, status, actions) = match marker.kind {
        MarkerKind::Invite => {
            let over = matches!(
                marker.when,
                Some(MarkerWhen::Event { ends_at, .. }) if ends_at <= now
            );
            let status = match marker.answer {
                Some(InviteAnswer::Accepting | InviteAnswer::Accepted) => Some("Accepted"),
                Some(InviteAnswer::Declining | InviteAnswer::Declined) => Some("Declined"),
                None if marker.cancelled => Some("Cancelled"),
                None if over => Some("Past"),
                None => None,
            };
            let actions = if status.is_none() {
                vec![
                    (CommandId::AcceptInvite, "Accept"),
                    (CommandId::DeclineInvite, "Decline"),
                ]
            } else {
                Vec::new()
            };
            ("Invite", status, actions)
        }
        MarkerKind::Question => ("Question", None, vec![(CommandId::Reply, "Reply")]),
        // Task joins Snooze once Obsidian exists (milestone 3, spec C9).
        MarkerKind::Todo => ("To-do", None, vec![(CommandId::Snooze, "Snooze")]),
        MarkerKind::NoReply => ("No reply", None, vec![(CommandId::Reply, "Reply")]),
    };
    MarkerLine {
        chip,
        date,
        quote,
        status,
        actions,
    }
}

#[cfg(test)]
mod marker_tests {
    use chrono::{TimeZone, Utc};
    use postio_core::CommandId;
    use postio_model::listing::{InviteAnswer, MarkerKind, MarkerSummary, MarkerWhen};

    use super::*;

    fn at(day: u32, hour: u32, minute: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, day, hour, minute, 0)
            .single()
            .expect("a real time")
    }

    fn invite(starts: chrono::DateTime<Utc>, ends: chrono::DateTime<Utc>) -> MarkerSummary {
        MarkerSummary {
            kind: MarkerKind::Invite,
            when: Some(MarkerWhen::Event {
                starts_at: starts,
                ends_at: ends,
            }),
            excerpt: None,
            answer: None,
            cancelled: false,
        }
    }

    #[test]
    fn an_open_invitation_offers_accept_and_decline_with_its_time() {
        let line = marker_line(&invite(at(29, 10, 0), at(29, 10, 45)), at(26, 14, 9), &Utc);
        assert_eq!(line.chip, "Invite");
        assert_eq!(
            line.date.as_deref(),
            Some("Tue 29 Sep \u{b7} 10:00\u{2013}10:45")
        );
        assert_eq!(
            line.quote, None,
            "an invitation is about its event, not a sentence"
        );
        assert_eq!(
            line.actions,
            [
                (CommandId::AcceptInvite, "Accept"),
                (CommandId::DeclineInvite, "Decline")
            ]
        );
        assert_eq!(line.status, None);
    }

    #[test]
    fn a_past_invitation_is_read_from_its_end_and_offers_nothing() {
        let line = marker_line(&invite(at(25, 10, 0), at(25, 10, 45)), at(26, 14, 9), &Utc);
        assert!(
            line.actions.is_empty(),
            "the event has ended (US8 scenario 5)"
        );
        assert_eq!(line.status, Some("Past"));
        assert_eq!(
            line.date.as_deref(),
            Some("Fri 25 Sep \u{b7} 10:00\u{2013}10:45")
        );
    }

    #[test]
    fn a_cancelled_or_answered_invitation_says_so_instead_of_offering() {
        let mut cancelled = invite(at(29, 10, 0), at(29, 10, 45));
        cancelled.cancelled = true;
        let line = marker_line(&cancelled, at(26, 14, 9), &Utc);
        assert!(line.actions.is_empty());
        assert_eq!(line.status, Some("Cancelled"));

        let mut answered = invite(at(29, 10, 0), at(29, 10, 45));
        for (answer, said) in [
            (InviteAnswer::Accepting, "Accepted"),
            (InviteAnswer::Accepted, "Accepted"),
            (InviteAnswer::Declining, "Declined"),
            (InviteAnswer::Declined, "Declined"),
        ] {
            answered.answer = Some(answer);
            let line = marker_line(&answered, at(26, 14, 9), &Utc);
            assert!(
                line.actions.is_empty(),
                "{answer:?} offers no second answer"
            );
            assert_eq!(line.status, Some(said));
        }
    }

    #[test]
    fn a_question_quotes_its_sentence_and_offers_reply() {
        let question = MarkerSummary {
            kind: MarkerKind::Question,
            when: None,
            excerpt: Some("Can you approve these by Friday?".into()),
            answer: None,
            cancelled: false,
        };
        let line = marker_line(&question, at(26, 14, 9), &Utc);
        assert_eq!(line.chip, "Question");
        assert_eq!(line.date, None);
        assert_eq!(
            line.quote.as_deref(),
            Some("Can you approve these by Friday?"),
            "verbatim"
        );
        assert_eq!(line.actions, [(CommandId::Reply, "Reply")]);
    }

    #[test]
    fn a_to_do_names_its_day_and_offers_snooze_and_a_reminder_offers_reply() {
        let todo = MarkerSummary {
            kind: MarkerKind::Todo,
            when: Some(MarkerWhen::Due(at(30, 12, 0))),
            excerpt: Some("Please leave comments by Wednesday".into()),
            answer: None,
            cancelled: false,
        };
        let line = marker_line(&todo, at(26, 14, 9), &Utc);
        assert_eq!(line.chip, "To-do");
        assert_eq!(line.date.as_deref(), Some("Wed 30 Sep"));
        assert_eq!(line.actions, [(CommandId::Snooze, "Snooze")]);

        let reminder = MarkerSummary {
            kind: MarkerKind::NoReply,
            when: Some(MarkerWhen::Due(at(26, 9, 0))),
            excerpt: None,
            answer: None,
            cancelled: false,
        };
        let line = marker_line(&reminder, at(28, 14, 9), &Utc);
        assert_eq!(line.chip, "No reply");
        assert_eq!(line.date.as_deref(), Some("since Sat 26 Sep"));
        assert_eq!(line.actions, [(CommandId::Reply, "Reply")]);
    }
}
