//! What a Focus row and the list around it say, with no toolkit in it
//! (spec 007 US1; contracts/focus-surface.md, "Rows").
//!
//! The row draws in one `snapshot()` in `postio-gtk`; the rules behind
//! what it draws are here, where a unit test proves them in milliseconds:
//! which day heading a row sits under, how many label pills fit, and when
//! the conversation's count is worth a badge.

use chrono::{Datelike, NaiveDate};
use postio_core::CommandId;
use postio_model::label::Label;
use postio_model::listing::ThreadSummary;
use postio_model::thread_view::{ThreadMember, ThreadView};
use postio_model::{EmailAddress, MessageId};

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

/// The date on an open message's header card (screen 04): relative for
/// today, "Today, 15:22", and the absolute date beyond it -- a
/// message once opened is not "yesterday", it is dated.
pub fn message_date(
    at: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Local>,
) -> String {
    let local = at.with_timezone(&now.timezone());
    if local.date_naive() == now.date_naive() {
        return format!("Today, {}", local.format("%H:%M"));
    }
    // The same shape as today's ("Today, 15:22"): a comma, no "at", and no
    // weekday -- the card's line is short, and the date is not what it is for.
    local.format("%-d %b %Y, %H:%M").to_string()
}

/// The badge a conversation's count draws, or none for a conversation of
/// one: the badge says how big the conversation is, and one is not big.
pub fn count_badge(message_count: u32) -> Option<String> {
    (message_count > 1).then(|| message_count.to_string())
}

/// A digest row's first column: "Weekly · digest", or "Digest" when its
/// rule's cadence is not known (screen 01, T136).
pub fn digest_title(cadence: Option<postio_model::listing::Cadence>) -> String {
    use postio_model::listing::Cadence;
    match cadence {
        Some(Cadence::Daily) => "Daily \u{b7} digest".to_owned(),
        Some(Cadence::Weekly) => "Weekly \u{b7} digest".to_owned(),
        Some(Cadence::Monthly) => "Monthly \u{b7} digest".to_owned(),
        None => "Digest".to_owned(),
    }
}

/// A digest row's subject: "Newsletters · 14 messages".
pub fn digest_subject(rule: &str, count: u32) -> String {
    let messages = if count == 1 { "message" } else { "messages" };
    format!("{rule} \u{b7} {count} {messages}")
}

/// A digest row's line: its summary's opening once one is written, and
/// until then who it is from -- "From Ledger, Forge and 4 others".
pub fn digest_line(summary: Option<&str>, senders: &[String]) -> String {
    if let Some(summary) = summary.map(str::trim).filter(|summary| !summary.is_empty()) {
        return summary.to_owned();
    }
    match senders {
        [] => String::new(),
        [one] => format!("From {one}"),
        [one, two] => format!("From {one} and {two}"),
        [one, two, rest @ ..] => {
            let others = if rest.len() == 1 { "other" } else { "others" };
            format!("From {one}, {two} and {} {others}", rest.len())
        }
    }
}

/// The words on the top bar's command field.
pub const COMMAND_PROMPT: &str = "Search mail, go to a folder, or run a command";

/// The main menu, top to bottom: each item's label and the command it runs
/// (`None` for About, which is the application's, not a command).
pub const MENU: &[(&str, Option<CommandId>)] = &[
    // The people (specs/011-focus-contacts S13): `g c`.
    ("Contacts", Some(CommandId::OpenContacts)),
    ("Settings", Some(CommandId::Settings)),
    // A check item (T232): checked while messages open beside the list.
    ("Read beside the list", Some(CommandId::ToggleReadingPane)),
    ("Keyboard shortcuts", Some(CommandId::CheatSheet)),
    ("About", None),
    ("Quit", Some(CommandId::Quit)),
];

/// The chrome's buttons, each with the command a click runs. The command
/// field runs search, and wears the palette's key beside search's.
pub const BUTTONS: &[CommandId] = &[
    CommandId::Compose,
    CommandId::Search,
    CommandId::CommandPalette,
    CommandId::GoToFolders,
    CommandId::ToggleHasAction,
    CommandId::Quit,
];

/// The compose button's tooltip: "Compose · c", or "Compose" with no key.
pub fn compose_tooltip(key: Option<&str>) -> String {
    match key {
        Some(key) => format!("Compose \u{b7} {key}"),
        None => "Compose".to_owned(),
    }
}

/// The strip's digest rules button: "1 digest rule", "4 digest rules".
pub fn digest_rules(count: usize) -> String {
    if count == 1 {
        "1 digest rule".to_owned()
    } else {
        format!("{count} digest rules")
    }
}

/// The longest a row's one line of words runs before it is cut with an
/// ellipsis. The row's width cuts it again; this keeps a whole preview from
/// riding every frontend's text layout.
const LINE_CHARS: usize = 90;

/// Whether `who` is one of the account's own identities.
fn is_me(who: &EmailAddress, me: &[EmailAddress]) -> bool {
    me.iter().any(|mine| mine.same_address(who))
}

/// The first name a row says: the display name's first word, else the local
/// part, else the address.
pub fn first_name(who: &EmailAddress) -> &str {
    let name = who.display();
    if who.name.as_deref().is_some_and(|n| !n.trim().is_empty()) {
        name.split_whitespace().next().unwrap_or(name)
    } else {
        who.local_part().unwrap_or(name)
    }
}

/// Who a thread row names (spec 013 US1): up to three first names, newest
/// first, then "you" when the account took part, then "+N" for the others
/// that did not fit ("Lena, Ben, Ana, you +2"). A thread of one message
/// names its sender in full, as any single message does.
///
/// `participants` is first-seen order, as `ThreadSummary` carries it; "you"
/// is decided by the account's identities in `me`, never by a name.
pub fn participants(
    participants: &[EmailAddress],
    message_count: u32,
    me: &[EmailAddress],
) -> String {
    if message_count <= 1 {
        return match participants.first() {
            Some(only) if is_me(only, me) => "You".to_owned(),
            Some(only) => only.display().to_owned(),
            None => String::new(),
        };
    }
    let mut took_part = false;
    let mut others: Vec<&str> = Vec::new();
    for who in participants.iter().rev() {
        if is_me(who, me) {
            took_part = true;
        } else {
            others.push(first_name(who));
        }
    }
    let hidden = others.len().saturating_sub(3);
    others.truncate(3);
    let mut said = others.join(", ");
    if took_part {
        if !said.is_empty() {
            said.push_str(", ");
        }
        said.push_str("you");
    }
    if hidden > 0 {
        said.push_str(&format!(" +{hidden}"));
    }
    said
}

/// The thread row's snippet: the latest message's sender and the start of
/// its words, "Ben: Looks good…". The ellipsis marks a cut, so a short line
/// is not given one. Empty words leave the sender alone out of it.
pub fn latest_line(from: Option<&EmailAddress>, me: &[EmailAddress], words: &str) -> String {
    let first = words.lines().map(str::trim).find(|line| !line.is_empty());
    let Some(first) = first else {
        return String::new();
    };
    let mut said = String::new();
    if let Some(from) = from {
        said.push_str(if is_me(from, me) {
            "You"
        } else {
            first_name(from)
        });
        said.push_str(": ");
    }
    if first.chars().count() > LINE_CHARS {
        // Cut at a word, not through one, unless the line is one long word.
        let end = first
            .char_indices()
            .nth(LINE_CHARS)
            .map_or(first.len(), |(at, _)| at);
        let cut = match first[..end].rsplit_once(char::is_whitespace) {
            Some((head, _)) if !first[end..].starts_with(char::is_whitespace) => head,
            _ => &first[..end],
        };
        said.push_str(cut.trim_end());
        said.push('\u{2026}');
    } else {
        said.push_str(first);
    }
    said
}

/// The "2 new" pill, or none when nothing is unread.
pub fn new_badge(unread: u32) -> Option<String> {
    (unread > 0).then(|| format!("{unread} new"))
}

/// The tag on a child that answers an older message than the one before it,
/// "↳ to Ana". `answers` indexes `members` (date order); a member that
/// cannot be found or has no sender leaves nothing to name.
pub fn child_tag(answers: Option<u32>, members: &[ThreadMember]) -> Option<String> {
    let answered = members.get(answers? as usize)?;
    let who = if answered.own {
        "you"
    } else {
        first_name(answered.from.as_ref()?)
    };
    Some(format!("\u{21b3} to {who}"))
}

/// Which message opening a thread row lands on (FR-013): the first unread,
/// else the first with an open action marker, else the latest. `None` only
/// for a thread with no members.
pub fn open_at(view: &ThreadView) -> Option<MessageId> {
    let members = &view.members;
    members
        .iter()
        .find(|member| !member.seen)
        .or_else(|| members.iter().find(|member| member.marker))
        .or_else(|| members.last())
        .map(|member| member.id)
}

/// The chevron in a thread row's gutter. A one-message thread has none, so
/// it is `Option<Chevron>` on the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chevron {
    /// Folded: the thread's children are not shown.
    Collapsed,
    /// Open: the children sit under the row.
    Expanded,
}

/// Everything a thread row says, composed once here so the Mac and GTK draw
/// the same words (contracts/frontend-threads.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadRowView {
    /// "Lena, Ben, Ana, you", or the sender's full name for a thread of one.
    pub participants: String,
    /// The message count, for threads of two or more only.
    pub count: Option<String>,
    /// The thread's subject.
    pub subject: String,
    /// Drawn bold: some message here is unread.
    pub bold: bool,
    /// Every label of the thread; the row draws [`MAX_PILLS`] of them.
    pub labels: Vec<Label>,
    /// "Ben: Looks good…".
    pub latest_line: String,
    /// "2 new".
    pub new: Option<String>,
    /// A paperclip: any message has an attachment.
    pub attachment: bool,
    /// Replies since the thread was muted, when it is.
    pub muted: Option<u32>,
    /// What the row says of it: "muted \u{b7} 1".
    pub muted_badge: Option<String>,
    /// The latest message's time, as every row says it.
    pub time: String,
    /// The gutter's chevron; `None` for a thread of one.
    pub chevron: Option<Chevron>,
}

impl ThreadRowView {
    /// The row for `thread`, with its `labels`, as seen by an account whose
    /// identities are `me`, `expanded` when its children are shown.
    pub fn new(
        thread: &ThreadSummary,
        labels: &[Label],
        me: &[EmailAddress],
        expanded: bool,
        now: chrono::DateTime<chrono::Local>,
    ) -> Self {
        let latest = &thread.representative;
        let words = latest.preview.as_deref().unwrap_or("");
        Self {
            participants: participants(&thread.participants, thread.message_count, me),
            count: count_badge(thread.message_count),
            subject: thread
                .subject
                .clone()
                .or_else(|| latest.subject.clone())
                .unwrap_or_default(),
            bold: thread.unread_count > 0,
            labels: labels.to_vec(),
            latest_line: latest_line(
                latest.from.as_ref(),
                me,
                postio_model::thread_view::before_attribution(words),
            ),
            new: new_badge(thread.unread_count),
            attachment: thread.has_attachments,
            muted: thread.muted,
            muted_badge: thread.muted.map(crate::thread_outline::muted_badge),
            time: crate::row::timestamp(latest.received_at, now),
            chevron: (thread.message_count > 1).then_some(if expanded {
                Chevron::Expanded
            } else {
                Chevron::Collapsed
            }),
        }
    }
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
    fn an_open_message_is_dated_relatively_today_and_absolutely_after() {
        use chrono::TimeZone;
        let now = chrono::Local
            .with_ymd_and_hms(2026, 9, 29, 16, 9, 0)
            .single()
            .expect("a real time");
        let at = |d: u32, h: u32, m: u32| {
            chrono::Local
                .with_ymd_and_hms(2026, 9, d, h, m, 0)
                .single()
                .expect("a real time")
                .with_timezone(&chrono::Utc)
        };
        assert_eq!(message_date(at(29, 15, 22), now), "Today, 15:22");
        assert_eq!(message_date(at(29, 0, 5), now), "Today, 00:05");
        assert_eq!(message_date(at(28, 15, 22), now), "28 Sep 2026, 15:22");
        assert_eq!(message_date(at(3, 9, 0), now), "3 Sep 2026, 09:00");
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
    /// What the marker is, which decides what else it may offer.
    pub kind: postio_model::listing::MarkerKind,
    /// The date: "Tue 29 Sep · 10:00–10:45", "Wed 30 Sep", "since Sat 26 Sep".
    pub date: Option<String>,
    /// The sentence the marker is about, verbatim, for a question or to-do.
    pub quote: Option<String>,
    /// What stands where the actions would: "Accepted", "Cancelled", "Past".
    pub status: Option<&'static str>,
    /// The commands that answer it, in order, each with its button's words.
    pub actions: Vec<(postio_core::CommandId, &'static str)>,
}

impl MarkerLine {
    /// This line in a Focus that can capture into a vault (`[focus.vault]`
    /// configured, milestone 3): a to-do then offers Task `t` before
    /// Snooze (spec C9, screens 01, 03 and 04). Nothing else changes.
    pub fn capturing(mut self, capture: bool) -> Self {
        if capture && self.kind == postio_model::listing::MarkerKind::Todo {
            self.actions
                .insert(0, (postio_core::CommandId::CaptureTask, "Task"));
        }
        self
    }
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
        kind: marker.kind,
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
    #[test]
    fn with_a_vault_a_to_do_offers_task_before_snooze_and_nothing_else_changes() {
        // Spec C9: Task joins Snooze on a to-do once Obsidian exists, which
        // is once `[focus.vault]` is configured (T158).
        let todo = MarkerSummary {
            kind: MarkerKind::Todo,
            when: Some(MarkerWhen::Due(at(30, 12, 0))),
            excerpt: Some("Please leave comments by Wednesday".into()),
            answer: None,
            cancelled: false,
        };
        let line = marker_line(&todo, at(26, 14, 9), &Utc).capturing(true);
        assert_eq!(
            line.actions,
            [
                (CommandId::CaptureTask, "Task"),
                (CommandId::Snooze, "Snooze")
            ]
        );
        assert_eq!(
            marker_line(&todo, at(26, 14, 9), &Utc)
                .capturing(false)
                .actions,
            [(CommandId::Snooze, "Snooze")],
            "no vault, no Task"
        );
        let question = MarkerSummary {
            kind: MarkerKind::Question,
            when: None,
            excerpt: Some("Can you approve these by Friday?".into()),
            answer: None,
            cancelled: false,
        };
        assert_eq!(
            marker_line(&question, at(26, 14, 9), &Utc)
                .capturing(true)
                .actions,
            [(CommandId::Reply, "Reply")],
            "a question is answered, not captured"
        );
    }
}

/// Where a list row's columns stand in a row `width` pixels wide
/// (focus-surface.md, Rows; T232 for the narrow list beside a reading
/// pane).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowColumns {
    /// Where the sender's column starts.
    pub sender_x: f32,
    /// How wide the sender's column is.
    pub sender_width: f32,
    /// Where the subject starts.
    pub subject_x: f32,
    /// Where a marked row's second line starts: under the subject, or,
    /// once the sender's column has had to narrow, under the sender, so the
    /// marker's chip, date and answers keep the row's width.
    pub marker_x: f32,
}

/// The sender's column at its widest, from screen 01.
pub const SENDER_WIDTH: f32 = 222.0;
/// The narrowest the sender's column gets: a name, ellipsized.
pub const SENDER_MIN: f32 = 120.0;
/// Where the sender's column starts, after the gutter.
pub const SENDER_X: f32 = 56.0;
/// The room the subject's column keeps before the sender's gives any up:
/// a subject, a pill and the trailing time and count.
pub const SUBJECT_ROOM: f32 = 260.0;
/// The gap between the two columns, and between things on a line.
pub const COLUMN_GAP: f32 = 12.0;
/// The room a row's right edge keeps.
pub const ROW_TRAILING: f32 = 24.0;

/// The columns of a row `width` pixels wide: the screen's at the inbox's
/// width, the sender's column narrowing (to [`SENDER_MIN`]) only when the
/// subject's would otherwise have less than [`SUBJECT_ROOM`] -- the list
/// beside a reading pane, 404 px at its narrowest.
pub fn row_columns(width: f32) -> RowColumns {
    let spare = width - SENDER_X - COLUMN_GAP - SUBJECT_ROOM - ROW_TRAILING;
    let sender_width = spare.clamp(SENDER_MIN, SENDER_WIDTH);
    let subject_x = SENDER_X + sender_width + COLUMN_GAP;
    RowColumns {
        sender_x: SENDER_X,
        sender_width,
        subject_x,
        marker_x: if sender_width < SENDER_WIDTH {
            SENDER_X
        } else {
            subject_x
        },
    }
}

/// The header strip's counts: `312 · 41 unread`, or the conversations alone
/// while nothing is unread.
pub fn strip_counts(conversations: u32, unread: u32) -> String {
    let conversations = crate::selection::count(conversations);
    if unread == 0 {
        return conversations;
    }
    format!(
        "{conversations} \u{b7} {} unread",
        crate::selection::count(unread)
    )
}

/// The header strip's count for a place that is not the inbox: how many
/// conversations it lists, and nothing about unread -- the unread count of
/// the inbox describes the inbox, and beside "Flagged" it described a place
/// the person had left.
pub fn place_counts(conversations: u32) -> String {
    crate::selection::count(conversations)
}

/// The has-action toggle's words: `Has action · 7`, or `Has action` before
/// the count is known.
pub fn has_action_label(count: Option<u32>) -> String {
    match count {
        Some(count) => format!("Has action \u{b7} {}", crate::selection::count(count)),
        None => "Has action".to_owned(),
    }
}

/// What the strip says while the has-action filter is on: `Showing 7 of
/// 312 · ! again to show all`, naming the key the keymap gives the toggle,
/// or `Showing 7 of 312` when nothing is bound to it.
pub fn showing(shown: u32, total: u32, key: Option<&str>) -> String {
    let said = format!(
        "Showing {} of {}",
        crate::selection::count(shown),
        crate::selection::count(total)
    );
    match key {
        Some(key) => format!("{said} \u{b7} {key} again to show all"),
        None => said,
    }
}

/// Who a row names in its sender column. Received mail names who it is
/// from; a draft or a queued message is from the person themselves, so in
/// Drafts and the Outbox the row names who it is to, as every mail client
/// does: `To Ada Lovelace`.
pub fn row_names(message: &postio_model::listing::MessageSummary) -> String {
    if message.send_state.is_none() {
        return message
            .from
            .as_ref()
            .map(|from| from.display().to_owned())
            .unwrap_or_default();
    }
    match crate::conversation::participants(&message.to).as_str() {
        "" => "No recipient".to_owned(),
        names => format!("To {names}"),
    }
}

#[cfg(test)]
mod strip_tests {
    use super::*;

    fn message(
        from: &str,
        to: &[(&str, &str)],
        send_state: Option<postio_model::DraftState>,
    ) -> postio_model::listing::MessageSummary {
        use postio_model::EmailAddress;
        postio_model::listing::MessageSummary {
            id: postio_model::MessageId::new(1),
            thread: None,
            from: Some(EmailAddress::new(Some(from), "me@example.com")),
            subject: None,
            preview: None,
            received_at: chrono::Utc::now(),
            seen: true,
            flagged: false,
            answered: false,
            send_state,
            send_at: None,
            has_attachments: false,
            thread_count: 1,
            to: to
                .iter()
                .map(|(name, address)| EmailAddress::new(Some(*name), *address))
                .collect(),
        }
    }

    #[test]
    fn received_mail_names_its_sender_and_a_draft_names_who_it_is_to() {
        let received = message("Ada Lovelace", &[("Grace Hopper", "g@example.com")], None);
        assert_eq!(row_names(&received), "Ada Lovelace");

        let draft = message(
            "Test User",
            &[("Ada Lovelace", "ada@example.com")],
            Some(postio_model::DraftState::Editing),
        );
        assert_eq!(row_names(&draft), "To Ada Lovelace");

        let queued = message(
            "Test User",
            &[
                ("Ada Lovelace", "ada@example.com"),
                ("Grace Hopper", "grace@example.com"),
            ],
            Some(postio_model::DraftState::Queued),
        );
        assert_eq!(row_names(&queued), "To Ada, Grace");

        let nobody = message("Test User", &[], Some(postio_model::DraftState::Editing));
        assert_eq!(row_names(&nobody), "No recipient");
    }

    #[test]
    fn a_place_counts_what_it_lists_and_never_the_inboxs_unread() {
        assert_eq!(place_counts(8), "8");
        assert_eq!(place_counts(1_200), "1,200");
    }

    #[test]
    fn the_strip_counts_conversations_and_unread() {
        assert_eq!(strip_counts(312, 41), "312 \u{b7} 41 unread");
        assert_eq!(
            strip_counts(12_408, 0),
            "12,408",
            "nothing unread says nothing"
        );
    }

    #[test]
    fn the_toggle_carries_its_count() {
        assert_eq!(has_action_label(Some(7)), "Has action \u{b7} 7");
        assert_eq!(has_action_label(None), "Has action");
    }

    #[test]
    fn the_filter_says_how_many_of_how_many_and_the_key_back() {
        assert_eq!(
            showing(7, 312, Some("!")),
            "Showing 7 of 312 \u{b7} ! again to show all"
        );
        assert_eq!(
            showing(7, 312, None),
            "Showing 7 of 312",
            "no key, no promise"
        );
    }

    #[test]
    fn a_digest_row_says_its_cadence_its_count_and_who_it_is_from() {
        use postio_model::listing::Cadence;
        assert_eq!(digest_title(Some(Cadence::Weekly)), "Weekly \u{b7} digest");
        assert_eq!(digest_title(None), "Digest");
        assert_eq!(
            digest_subject("Newsletters", 14),
            "Newsletters \u{b7} 14 messages"
        );
        assert_eq!(
            digest_subject("Newsletters", 1),
            "Newsletters \u{b7} 1 message"
        );
        let names = |names: &[&str]| {
            names
                .iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(digest_line(None, &names(&["Ledger"])), "From Ledger");
        assert_eq!(
            digest_line(
                None,
                &names(&["Ledger", "Forge", "Rates", "Tides", "Rail", "Soil"])
            ),
            "From Ledger, Forge and 4 others"
        );
        assert_eq!(
            digest_line(
                Some("Rail funding vote, CRDT libraries"),
                &names(&["Ledger"])
            ),
            "Rail funding vote, CRDT libraries"
        );
    }

    #[test]
    fn a_wide_row_keeps_the_screens_columns() {
        for width in [620.0, 1100.0, 1440.0] {
            let columns = row_columns(width);
            assert_eq!(columns.sender_width, 222.0, "{width}");
            assert_eq!(columns.subject_x, 290.0, "{width}");
            assert_eq!(columns.marker_x, 290.0, "{width}");
        }
    }

    #[test]
    fn a_narrow_row_gives_the_senders_column_up_first() {
        // 460: the list beside an 820 pane in a 1280 window.
        let columns = row_columns(460.0);
        assert_eq!(columns.sender_width, 120.0);
        assert_eq!(columns.subject_x, 188.0);
        // The subject keeps what is left, and the marker line the row.
        assert_eq!(columns.marker_x, SENDER_X);
        // Just short of the screen's columns, the sender gives up only what
        // the subject needs.
        let columns = row_columns(560.0);
        assert_eq!(columns.sender_width, 560.0 - 56.0 - 12.0 - 260.0 - 24.0);
        assert_eq!(columns.marker_x, SENDER_X);
    }

    #[test]
    fn the_sender_never_narrows_past_its_floor() {
        let columns = row_columns(404.0);
        assert_eq!(columns.sender_width, SENDER_MIN);
        assert_eq!(columns.subject_x, SENDER_X + SENDER_MIN + COLUMN_GAP);
    }

    #[test]
    fn the_strip_counts_digest_rules() {
        assert_eq!(digest_rules(1), "1 digest rule");
        assert_eq!(digest_rules(4), "4 digest rules");
        assert_eq!(compose_tooltip(Some("c")), "Compose \u{b7} c");
        assert_eq!(compose_tooltip(None), "Compose");
    }
}

#[cfg(test)]
mod thread_words_tests {
    use chrono::{Local, TimeZone, Utc};
    use postio_model::listing::{MessageSummary, ThreadSummary};
    use postio_model::thread_view::{Line, ThreadMember, ThreadView};
    use postio_model::{EmailAddress, MessageId, ThreadId};

    use super::*;

    fn who(name: &str) -> EmailAddress {
        let address = format!("{}@example.com", name.to_lowercase().replace(' ', "."));
        EmailAddress::new(Some(name), address)
    }

    fn me() -> Vec<EmailAddress> {
        vec![EmailAddress::new(Some("Test User"), "me@example.com")]
    }

    fn member(id: i64, from: &str, seen: bool, marker: bool) -> ThreadMember {
        ThreadMember {
            id: MessageId::new(id),
            from: Some(who(from)),
            own: false,
            at: Utc.with_ymd_and_hms(2026, 9, 21, 9, 0, 0).unwrap(),
            seen,
            has_attachments: false,
            line: Line::Own("hello".into()),
            answers: None,
            marker,
        }
    }

    fn view(members: Vec<ThreadMember>) -> ThreadView {
        ThreadView {
            thread: ThreadId::new(1),
            subject: Some("Plans".into()),
            members,
            draft: None,
            muted: None,
            labels: Vec::new(),
        }
    }

    #[test]
    fn participants_are_first_names_newest_first_with_you_last() {
        // First-seen order in; newest first out (SPEC §2).
        let seen = [
            who("Ana Ruiz"),
            who("Ben Ortiz"),
            me()[0].clone(),
            who("Lena Park"),
        ];
        assert_eq!(participants(&seen, 6, &me()), "Lena, Ben, Ana, you");
    }

    #[test]
    fn participants_past_three_say_how_many_more() {
        let seen = [
            who("Ana Ruiz"),
            who("Ben Ortiz"),
            who("Cleo Diaz"),
            who("Dan Wu"),
            who("Eve Lin"),
            me()[0].clone(),
        ];
        assert_eq!(participants(&seen, 9, &me()), "Eve, Dan, Cleo, you +2");
    }

    #[test]
    fn a_thread_the_account_did_not_join_has_no_you() {
        let seen = [who("Ana Ruiz"), who("Ben Ortiz")];
        assert_eq!(participants(&seen, 2, &me()), "Ben, Ana");
    }

    #[test]
    fn a_single_message_names_its_sender_in_full() {
        assert_eq!(participants(&[who("Ana Ruiz")], 1, &me()), "Ana Ruiz");
        assert_eq!(participants(&me(), 1, &me()), "You");
    }

    #[test]
    fn you_is_decided_by_address_not_by_name() {
        let alias = EmailAddress::new(Some("Somebody Else"), "ME@example.com");
        let impostor = EmailAddress::new(Some("Test User"), "other@example.com");
        assert_eq!(participants(&[alias, impostor], 2, &me()), "Test, you");
    }

    #[test]
    fn a_nameless_sender_is_said_by_the_local_part() {
        let bare = EmailAddress::new(None::<String>, "ops@example.com");
        assert_eq!(participants(&[bare, who("Ana Ruiz")], 2, &me()), "Ana, ops");
    }

    #[test]
    fn the_latest_line_leads_with_the_sender_and_marks_a_cut() {
        let ben = who("Ben Ortiz");
        assert_eq!(
            latest_line(Some(&ben), &me(), "Looks good"),
            "Ben: Looks good"
        );
        let long = format!("Looks good {}", "to me ".repeat(30));
        let said = latest_line(Some(&ben), &me(), &long);
        assert!(said.starts_with("Ben: Looks good to me"), "{said}");
        assert!(said.ends_with("me\u{2026}"), "{said}");
        assert_eq!(latest_line(Some(&me()[0]), &me(), "Done"), "You: Done");
        assert_eq!(
            latest_line(Some(&ben), &me(), "  \n "),
            "",
            "no words, no line"
        );
    }

    #[test]
    fn the_new_badge_counts_and_vanishes_at_zero() {
        assert_eq!(new_badge(2).as_deref(), Some("2 new"));
        assert_eq!(new_badge(0), None);
    }

    #[test]
    fn a_tag_names_the_first_name_of_who_was_answered() {
        let members = [
            member(1, "Ana Ruiz", true, false),
            member(2, "Ben Ortiz", true, false),
        ];
        assert_eq!(
            child_tag(Some(0), &members).as_deref(),
            Some("\u{21b3} to Ana")
        );
        assert_eq!(child_tag(None, &members), None);
        assert_eq!(child_tag(Some(7), &members), None, "an index off the end");
        let mut own = member(3, "Test User", true, false);
        own.own = true;
        assert_eq!(
            child_tag(Some(0), &[own]).as_deref(),
            Some("\u{21b3} to you")
        );
    }

    #[test]
    fn opening_a_thread_lands_on_unread_then_marker_then_latest() {
        let both = view(vec![
            member(1, "Ana", true, true),
            member(2, "Ben", false, false),
            member(3, "Cleo", false, false),
            member(4, "Dan", true, false),
        ]);
        assert_eq!(open_at(&both), Some(MessageId::new(2)), "first unread");

        let marked = view(vec![
            member(1, "Ana", true, false),
            member(2, "Ben", true, true),
            member(3, "Cleo", true, true),
            member(4, "Dan", true, false),
        ]);
        assert_eq!(
            open_at(&marked),
            Some(MessageId::new(2)),
            "first open marker"
        );

        let read = view(vec![
            member(1, "Ana", true, false),
            member(2, "Ben", true, false),
        ]);
        assert_eq!(open_at(&read), Some(MessageId::new(2)), "latest");
        assert_eq!(open_at(&view(Vec::new())), None);
    }

    fn summary(count: u32, unread: u32) -> ThreadSummary {
        let latest = Utc.with_ymd_and_hms(2026, 9, 21, 9, 14, 0).unwrap();
        ThreadSummary {
            id: Some(ThreadId::new(1)),
            representative: MessageSummary {
                id: MessageId::new(9),
                thread: Some(ThreadId::new(1)),
                from: Some(who("Ben Ortiz")),
                subject: Some("Q3 plan".into()),
                preview: Some("Looks good. On Mon, Ana wrote: > the plan".into()),
                received_at: latest,
                seen: unread == 0,
                flagged: false,
                answered: false,
                send_state: None,
                send_at: None,
                has_attachments: false,
                thread_count: count,
                to: Vec::new(),
            },
            subject: Some("Q3 plan".into()),
            participants: vec![who("Ana Ruiz"), who("Ben Ortiz")],
            message_count: count,
            unread_count: unread,
            flagged: false,
            has_attachments: true,
            last_at: latest,
            marker: None,
            copies: Vec::new(),
            muted: None,
        }
    }

    #[test]
    fn a_thread_row_says_every_field() {
        let now = Local.with_ymd_and_hms(2026, 9, 21, 12, 0, 0).unwrap();
        let labels = [Label {
            id: postio_model::LabelId::new(1),
            account_id: postio_model::AccountId::new(1),
            name: "Plans".into(),
            color: None,
        }];
        let row = ThreadRowView::new(&summary(6, 2), &labels, &me(), false, now);
        assert_eq!(row.participants, "Ben, Ana");
        assert_eq!(row.count.as_deref(), Some("6"));
        assert_eq!(row.subject, "Q3 plan");
        assert!(row.bold);
        assert_eq!(row.labels, labels);
        assert_eq!(
            row.latest_line, "Ben: Looks good.",
            "the attribution is cut"
        );
        assert_eq!(row.new.as_deref(), Some("2 new"));
        assert!(row.attachment);
        assert_eq!(row.muted, None);
        assert_eq!(row.chevron, Some(Chevron::Collapsed));
        assert_eq!(
            ThreadRowView::new(&summary(6, 2), &[], &me(), true, now).chevron,
            Some(Chevron::Expanded)
        );
    }

    #[test]
    fn a_muted_thread_row_reads_muted_and_the_count() {
        let now = Local.with_ymd_and_hms(2026, 9, 21, 12, 0, 0).unwrap();
        let mut muted = summary(4, 0);
        muted.muted = Some(1);
        let row = ThreadRowView::new(&muted, &[], &me(), false, now);
        assert_eq!(row.muted, Some(1));
        assert_eq!(row.muted_badge.as_deref(), Some("muted \u{b7} 1"));
        let plain = ThreadRowView::new(&summary(4, 0), &[], &me(), false, now);
        assert_eq!(plain.muted_badge, None);
    }

    #[test]
    fn a_thread_of_one_has_no_chevron_count_or_new() {
        let now = Local.with_ymd_and_hms(2026, 9, 21, 12, 0, 0).unwrap();
        let row = ThreadRowView::new(&summary(1, 0), &[], &me(), false, now);
        assert_eq!(row.chevron, None);
        assert_eq!(row.count, None);
        assert_eq!(row.new, None);
        assert!(!row.bold);
    }
}
