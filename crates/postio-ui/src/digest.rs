//! Digest rules, read in the one query language (spec 007 T132).
//!
//! `postio-config` keeps a rule's queries as text and does not parse them
//! (`docs/ARCHITECTURE.md`, the one query language), so it validates
//! everything about a rule but this: whether each query reads in full.
//! That check is made here, where the parser is. When a digest is due is
//! [`crate::schedule::next_due`].

use chrono::NaiveDate;
use postio_config::DigestRule;
use postio_search::query::TokenKind;

/// Each of `rule`'s queries that does not read in full, as a sentence
/// naming the rule and the query's place in it (contracts/config.md).
///
/// The language refuses nothing -- half-typed is an ordinary state in the
/// bar -- so here "does not read" is a blank query, or an operator it could
/// not read (`from:` with no address, `is:someday`, `after:2026-`). In a
/// search that constrains nothing; in a rule it would hold mail the rule
/// never meant, so a rule with one is not applied.
pub fn unreadable_queries(rule: &DigestRule, today: NaiveDate) -> Vec<String> {
    let name = rule.name.trim();
    rule.queries
        .iter()
        .enumerate()
        .filter_map(|(place, query)| {
            let place = place + 1;
            if query.trim().is_empty() {
                return Some(format!("query {place} of digest rule `{name}` is empty"));
            }
            let parsed = postio_search::parse(query, today);
            let unread: Vec<String> = parsed
                .tokens()
                .iter()
                .filter(|token| matches!(token.kind, TokenKind::Partial(_)))
                .map(|token| format!("`{}`", token.raw))
                .collect();
            (!unread.is_empty()).then(|| {
                format!(
                    "query {place} of digest rule `{name}` has what the query \
                     language cannot read: {}",
                    unread.join(", ")
                )
            })
        })
        .collect()
}

/// When a rule delivers, as the digest window's sub-row says it:
/// "Weekly, Sunday 09:00", "Daily, 08:00", "Monthly, day 1 09:00"; `None`
/// for a rule whose cadence, day or time names no time.
pub fn rule_when(rule: &DigestRule) -> Option<String> {
    use postio_config::Due;
    Some(match rule.due().ok()? {
        Due::Daily { at } => format!("Daily, {}", at.format("%H:%M")),
        Due::Weekly { day, at } => {
            let day = chrono::NaiveDate::from_isoywd_opt(2026, 1, day)?.format("%A");
            format!("Weekly, {day} {}", at.format("%H:%M"))
        }
        Due::Monthly { day, at } => format!("Monthly, day {day} {}", at.format("%H:%M")),
    })
}

/// The digest window's line under its title: "14 messages from 6 senders
/// · came due today 16:00".
pub fn window_subtitle(
    count: u32,
    senders: usize,
    at: chrono::DateTime<chrono::Local>,
    now: chrono::DateTime<chrono::Local>,
) -> String {
    let messages = if count == 1 { "message" } else { "messages" };
    let people = if senders == 1 { "sender" } else { "senders" };
    let when = if at.date_naive() == now.date_naive() {
        format!("today {}", at.format("%H:%M"))
    } else {
        at.format("%a %-d %b %H:%M").to_string()
    };
    format!("{count} {messages} from {senders} {people} \u{b7} came due {when}")
}

/// The rule dialog's preview heading: "Would have caught 9 messages in
/// the last 90 days" (screen 24).
pub fn preview_heading(count: u32) -> String {
    let messages = if count == 1 { "message" } else { "messages" };
    format!("Would have caught {count} {messages} in the last 90 days")
}

/// Under the preview's rows: "and 5 more", or nothing when they are all.
pub fn preview_more(count: u32, shown: usize) -> Option<String> {
    let rest = (count as usize).saturating_sub(shown);
    (rest > 0).then(|| format!("and {rest} more"))
}

/// The rule dialog's note: what still comes straight to the inbox.
pub fn rule_note(senders: usize) -> &'static str {
    if senders > 1 {
        "Mail from these senders with an invite, question or to-do still comes straight to the inbox."
    } else {
        "Mail from this sender with an invite, question or to-do still comes straight to the inbox."
    }
}

/// A new rule's name, from its senders' names: "Ledger", "Ledger and
/// Forge", "Ledger, Forge and 2 others" (contracts/config.md: the name
/// defaults to the sender's).
pub fn rule_name(names: &[String]) -> String {
    match names {
        [] => "Digest".to_owned(),
        [one] => one.clone(),
        [one, two] => format!("{one} and {two}"),
        [one, two, rest @ ..] => {
            let others = if rest.len() == 1 { "other" } else { "others" };
            format!("{one}, {two} and {} {others}", rest.len())
        }
    }
}

/// When a previewed message came, as the preview's rows say it: "Today",
/// a weekday within the week, then the date.
pub fn preview_day(
    at: chrono::DateTime<chrono::Local>,
    now: chrono::DateTime<chrono::Local>,
) -> String {
    let (day, today) = (at.date_naive(), now.date_naive());
    if day == today {
        "Today".to_owned()
    } else if today - day < chrono::Duration::days(7) && day < today {
        at.format("%a").to_string()
    } else {
        at.format("%-d %b").to_string()
    }
}

/// Where held mail waits, as a search result says it in place of its
/// folder: "held · Newsletters" until its digest comes, "digest ·
/// Newsletters" once it has (US10 scenario 7).
pub fn held_place(rule: &str, delivered: bool) -> String {
    if delivered {
        format!("digest \u{b7} {rule}")
    } else {
        format!("held \u{b7} {rule}")
    }
}

/// The `g d` view's title (spec C15, T139).
pub const RULES_TITLE: &str = "Digest rules";

/// The line under it: what a rule does, and what it never does.
pub const RULES_SUBTITLE: &str =
    "Held mail skips the inbox until its digest comes \u{b7} search still finds it";

/// What the view says with no rule yet.
pub const RULES_EMPTY: &str = "No digest rules yet";

/// How to make one, with the key that does it.
pub fn rules_empty_hint(keymap: &postio_core::Keymap) -> String {
    match crate::hints::key(keymap, postio_core::CommandId::DigestRule) {
        Some(key) => format!("Press {key} on a message to digest its sender"),
        None => "Digest a sender from a message".to_owned(),
    }
}

/// "holds 3": what a rule holds now.
pub fn holds(count: u32) -> String {
    format!("holds {count}")
}

/// When a rule next delivers: "next Sun 4 Oct 09:00".
pub fn next_delivery(at: chrono::DateTime<chrono::Local>) -> String {
    format!("next {}", at.format("%a %-d %b %H:%M"))
}

/// The `g d` view's footer.
pub fn rules_footer(keymap: &postio_core::Keymap) -> Vec<crate::hints::Hint> {
    use postio_core::CommandId;
    let mut said = Vec::new();
    said.extend(crate::hints::hint(keymap, CommandId::OpenMessage, "edit"));
    said.extend(crate::hints::hint(
        keymap,
        CommandId::Delete,
        "remove and release",
    ));
    said.extend(crate::hints::hint(keymap, CommandId::Back, "inbox"));
    said
}

/// What removing a rule asks: its name, and what comes back.
pub fn remove_body(holds: u32) -> String {
    let messages = if holds == 1 { "message" } else { "messages" };
    format!(
        "What it holds now \u{2014} {holds} {messages} \u{2014} comes to the inbox, and its \
         senders\u{2019} mail is no longer held."
    )
}

/// What the digest window shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// The summary (screen 22).
    Summary,
    /// The plain list of its messages.
    List,
    /// The email from a reference (screen 23).
    Email,
}

impl Page {
    /// The page a digest opens on: its summary once one is written, the
    /// plain list until then.
    pub fn opening(has_summary: bool) -> Page {
        if has_summary {
            Page::Summary
        } else {
            Page::List
        }
    }

    /// `Tab`: the other of summary and list -- only while there is a
    /// summary (FR-175), and never over an email.
    pub fn toggled(self, has_summary: bool) -> Option<Page> {
        match (self, has_summary) {
            (_, false) | (Page::Email, _) => None,
            (Page::Summary, true) => Some(Page::List),
            (Page::List, true) => Some(Page::Summary),
        }
    }

    /// `Esc`: from an email, back to the summary; nowhere from the others,
    /// so a plain `Esc` closes the window.
    pub fn back(self) -> Option<Page> {
        (self == Page::Email).then_some(Page::Summary)
    }

    /// Whether the header's subtitle line and Archive all show.
    pub fn shows_header(self) -> bool {
        self != Page::Email
    }
}

/// `]` (`by` 1) and `[` (`by` -1): the focused reference after a step,
/// clamped to the summary's ends; `None` for a summary of no statements.
pub fn step_reference(current: Option<usize>, by: i32, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let next = (current.unwrap_or(0) as i32 + by).clamp(0, len as i32 - 1);
    Some(next as usize)
}

/// The window's title: the cadence and the rule's name -- "Weekly ·
/// Newsletters" -- or the name alone when the rule's cadence is not known.
pub fn window_title(cadence: Option<postio_model::listing::Cadence>, rule: &str) -> String {
    match crate::focus_row::digest_title(cadence).split_once(" \u{b7} ") {
        Some((cadence, _)) => format!("{cadence} \u{b7} {rule}"),
        None => rule.to_owned(),
    }
}

/// The Archive all button's words: "Archive all 14".
pub fn archive_all(count: u32) -> String {
    format!("Archive all {count}")
}

/// The line under the title that edits the rule: "Weekly, Sunday 09:00 ·
/// Edit rule and cadence".
pub fn rule_line(when: Option<&str>) -> String {
    match when {
        Some(when) => format!("{when} \u{b7} Edit rule and cadence"),
        None => "Edit rule and cadence".to_owned(),
    }
}

/// A run of a summary's statements under one topic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Topic<'a> {
    /// The topic's heading.
    pub name: &'a str,
    /// Where its first statement is among all of them.
    pub start: usize,
    /// Its statements, in reading order.
    pub statements: &'a [postio_model::summary::SummaryStatement],
}

/// The summary's statements grouped by topic: consecutive statements of one
/// topic share one heading, and a topic that comes back gets a heading again.
pub fn topics(statements: &[postio_model::summary::SummaryStatement]) -> Vec<Topic<'_>> {
    let mut runs: Vec<Topic<'_>> = Vec::new();
    for (index, statement) in statements.iter().enumerate() {
        match runs.last_mut() {
            Some(run) if run.name == statement.topic => {
                run.statements = &statements[run.start..=index];
            }
            _ => runs.push(Topic {
                name: &statement.topic,
                start: index,
                statements: &statements[index..=index],
            }),
        }
    }
    runs
}

/// A topic's heading with its count and its sources: `Rates · 3 statements
/// from 2 messages`. Counted from the run itself, so the heading says what
/// the group holds before a statement is read.
pub fn topic_heading(topic: &Topic<'_>) -> String {
    let statements = topic.statements.len();
    let messages = topic
        .statements
        .iter()
        .map(|statement| statement.reference.message)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    format!(
        "{} \u{b7} {statements} {} from {messages} {}",
        topic.name,
        if statements == 1 {
            "statement"
        } else {
            "statements"
        },
        if messages == 1 { "message" } else { "messages" },
    )
}

/// What the focused reference's card says it is: its number, who wrote the
/// message and its subject -- `Reference 3 · Ada Moreno · The rate decision`.
/// A card that only repeated a subject read as a third heading under the
/// last group.
pub fn reference_title(number: u32, sender: Option<&str>, subject: Option<&str>) -> String {
    let mut said = format!("Reference {number}");
    for part in [sender, subject].into_iter().flatten() {
        if !part.is_empty() {
            said.push_str(" \u{b7} ");
            said.push_str(part);
        }
    }
    said
}

/// The line under a summary: where it was written and what it cites.
pub fn summary_footer(messages: u32) -> String {
    format!(
        "Written on this computer by the local model from these {messages} messages only. \
         Every statement links to the email it came from."
    )
}

/// What the focused reference's card offers beside its subject.
pub const OPEN_FULL_EMAIL: &str = "open the full email";

/// The banner over an email opened from a reference.
pub fn cited_banner(number: u32) -> String {
    format!(
        "Cited as {number} in the summary; the passage is highlighted. \
         Esc goes back to the summary in this same window."
    )
}

/// The email page's subtitle: "Source 2 of 14", `at` being the message's
/// place among the digest's (`None` when it is not among them).
pub fn source_line(at: Option<usize>, total: usize) -> String {
    format!("Source {} of {total}", at.map_or(0, |index| index + 1))
}

/// How far back the rule dialog's preview looks.
pub const PREVIEW_DAYS: i64 = 90;

/// The cadences, in the rule dialog's menu order.
pub const CADENCES: [(postio_model::listing::Cadence, &str); 3] = [
    (postio_model::listing::Cadence::Daily, "Daily"),
    (postio_model::listing::Cadence::Weekly, "Weekly"),
    (postio_model::listing::Cadence::Monthly, "Monthly"),
];

/// The weekdays, in the rule dialog's menu order.
pub const WEEKDAYS: [chrono::Weekday; 7] = [
    chrono::Weekday::Mon,
    chrono::Weekday::Tue,
    chrono::Weekday::Wed,
    chrono::Weekday::Thu,
    chrono::Weekday::Fri,
    chrono::Weekday::Sat,
    chrono::Weekday::Sun,
];

/// What a new rule starts as: weekly, on Sunday, at 09:00 -- as indices into
/// [`CADENCES`] and [`WEEKDAYS`], and the first day of the month.
pub const DEFAULT_CADENCE: usize = 1;
/// See [`DEFAULT_CADENCE`].
pub const DEFAULT_WEEKDAY: usize = 6;
/// See [`DEFAULT_CADENCE`].
pub const DEFAULT_MONTH_DAY: usize = 0;
/// See [`DEFAULT_CADENCE`].
pub const DEFAULT_TIME: &str = "09:00";

/// Said when the time is not 24-hour HH:MM.
pub const TIME_ERROR: &str = "Give the time as 24-hour HH:MM, such as 09:00";

/// Said when "Digest mail like this" is asked for and no model is there to
/// ask.
pub const LIKE_THIS_NEEDS_A_MODEL: &str =
    "Digest mail like this needs a model: turn like_this on under [focus.model] in config.toml";

/// Said when the model found nothing alike to digest.
pub const NOTHING_ALIKE: &str = "The model found nothing alike to digest";

/// The placeholder in the query entry.
pub const QUERY_PLACEHOLDER: &str = "list:weekly.example.org or a search";

/// The rule dialog's heading for a new rule from `senders` senders.
pub fn new_rule_heading(senders: usize) -> &'static str {
    if senders > 1 {
        "Digest these senders"
    } else {
        "Digest this sender"
    }
}

/// The rule dialog's heading for editing `name`.
pub fn edit_rule_heading(name: &str) -> String {
    format!("Digest rule \u{b7} {name}")
}

/// The queries a new rule from `senders` holds: one `from:` each.
pub fn sender_queries(senders: &[postio_model::EmailAddress]) -> Vec<String> {
    senders
        .iter()
        .map(|sender| format!("from:{}", sender.address.to_lowercase()))
        .collect()
}

/// The plain-senders "From" line: their addresses, comma-separated.
pub fn sender_line(senders: &[postio_model::EmailAddress]) -> String {
    senders
        .iter()
        .map(|sender| sender.address.to_lowercase())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whether a rule reads as senders -- every query is a `from:` -- and so is
/// edited through the plain line; anything else opens in the query entry.
pub fn is_sender_rule(queries: &[String]) -> bool {
    queries
        .iter()
        .all(|query| query.trim().to_lowercase().starts_with("from:"))
}

/// The queries typed into the query entry: one per comma-separated piece.
pub fn split_queries(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The create button's words: Save while editing, Create for a new rule.
pub fn create_words(editing: bool) -> &'static str {
    if editing { "Save" } else { "Create" }
}

/// The rule dialog's schedule controls, as the menus hold them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    /// Index into [`CADENCES`].
    pub cadence: usize,
    /// Index into [`WEEKDAYS`].
    pub weekday: usize,
    /// Index of the day of the month, from 0.
    pub month_day: usize,
    /// The time, "HH:MM".
    pub at: String,
}

impl Schedule {
    /// What a new rule starts as.
    pub fn new_rule() -> Self {
        Schedule {
            cadence: DEFAULT_CADENCE,
            weekday: DEFAULT_WEEKDAY,
            month_day: DEFAULT_MONTH_DAY,
            at: DEFAULT_TIME.to_owned(),
        }
    }

    /// What editing a rule that is due `due` shows.
    pub fn of(due: &postio_config::Due) -> Self {
        use postio_config::Due;
        let mut shown = Schedule::new_rule();
        match due {
            Due::Daily { at } => {
                shown.cadence = 0;
                shown.at = at.format("%H:%M").to_string();
            }
            Due::Weekly { day, at } => {
                shown.cadence = 1;
                shown.weekday = WEEKDAYS.iter().position(|each| each == day).unwrap_or(6);
                shown.at = at.format("%H:%M").to_string();
            }
            Due::Monthly { day, at } => {
                shown.cadence = 2;
                shown.month_day = day.saturating_sub(1) as usize;
                shown.at = at.format("%H:%M").to_string();
            }
        }
        shown
    }

    /// When the controls say the rule is due, or the sentence saying why
    /// they do not say it.
    pub fn due(&self) -> Result<postio_config::Due, String> {
        use postio_config::Due;
        let at = chrono::NaiveTime::parse_from_str(self.at.trim(), "%H:%M")
            .map_err(|_| TIME_ERROR.to_owned())?;
        Ok(match CADENCES[self.cadence.min(2)].0 {
            postio_model::listing::Cadence::Daily => Due::Daily { at },
            postio_model::listing::Cadence::Weekly => Due::Weekly {
                day: WEEKDAYS[self.weekday.min(6)],
                at,
            },
            postio_model::listing::Cadence::Monthly => Due::Monthly {
                day: self.month_day.min(27) as u32 + 1,
                at,
            },
        })
    }
}

/// The rules list's count: "1 rule", "4 rules".
pub fn rule_count(count: usize) -> String {
    if count == 1 {
        "1 rule".to_owned()
    } else {
        format!("{count} rules")
    }
}

/// What a rule row's buttons say, with the command each runs.
pub const RULE_ROW_BUTTONS: [(postio_core::CommandId, &str); 2] = [
    (postio_core::CommandId::OpenMessage, "Edit"),
    (postio_core::CommandId::Delete, "Remove"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_topic_heading_carries_its_count_and_its_sources() {
        let mut statements = vec![
            statement("Rates", 1),
            statement("Rates", 2),
            statement("Rates", 3),
            statement("Engineering reading", 4),
        ];
        // Two of the three cite one message.
        statements[1].reference.message = statements[0].reference.message;
        let runs = topics(&statements);
        assert_eq!(
            topic_heading(&runs[0]),
            "Rates \u{b7} 3 statements from 2 messages"
        );
        assert_eq!(
            topic_heading(&runs[1]),
            "Engineering reading \u{b7} 1 statement from 1 message"
        );
    }

    #[test]
    fn a_reference_card_names_its_number_sender_and_subject() {
        assert_eq!(
            reference_title(3, Some("Ada Moreno"), Some("The rate decision")),
            "Reference 3 \u{b7} Ada Moreno \u{b7} The rate decision"
        );
        assert_eq!(
            reference_title(3, None, Some("Plans")),
            "Reference 3 \u{b7} Plans"
        );
        assert_eq!(reference_title(3, None, None), "Reference 3");
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 26).expect("a real date")
    }

    fn rule(name: &str, queries: &[&str]) -> DigestRule {
        DigestRule {
            name: name.to_owned(),
            queries: queries.iter().map(|query| (*query).to_owned()).collect(),
            ..DigestRule::default()
        }
    }

    /// Contracts/config.md: a query that does not parse is reported by the
    /// rule's name and the query's place. The language never refuses a
    /// query -- half-typed is a normal state in the bar -- so here "does
    /// not parse" is an operator it could not read: in a rule, that
    /// constrains nothing, and a rule that holds everything is the worst
    /// thing a digest can do.
    #[test]
    fn an_operator_the_language_cannot_read_is_named_by_rule_and_place() {
        let found = unreadable_queries(
            &rule(
                "Newsletters",
                &["from:news@localfirst.example", "from:", "after:2026-"],
            ),
            today(),
        );
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(
            found[0].contains("Newsletters")
                && found[0].contains('2')
                && found[0].contains("from:"),
            "{}",
            found[0]
        );
        assert!(
            found[1].contains('3') && found[1].contains("after:2026-"),
            "{}",
            found[1]
        );
    }

    #[test]
    fn a_blank_query_reads_as_nothing() {
        let found = unreadable_queries(&rule("School", &["   "]), today());
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("School"), "{}", found[0]);
    }

    #[test]
    fn queries_the_language_reads_in_full_are_not_reported() {
        let found = unreadable_queries(
            &rule(
                "Lists",
                &[
                    "from:news@localfirst.example",
                    "list:harbour-dev.lists.example.org",
                    "is:unread invoice after:yesterday",
                ],
            ),
            today(),
        );
        assert!(found.is_empty(), "{found:?}");
        assert_eq!(
            unreadable_queries(&rule("Lists", &["is:someday"]), today()).len(),
            1,
            "and one it cannot read is reported"
        );
    }

    #[test]
    fn a_rule_says_when_it_delivers() {
        let toml = |text: &str| {
            postio_config::Config::from_toml_str(text)
                .expect("a config")
                .focus
                .digests
                .remove(0)
        };
        let weekly = toml(
            "[[focus.digests]]\nname = \"N\"\nmatch = [\"from:a@b.test\"]\ncadence = \"weekly\"\nday = \"sunday\"\nat = \"09:00\"\n",
        );
        assert_eq!(rule_when(&weekly).as_deref(), Some("Weekly, Sunday 09:00"));
        let daily = toml(
            "[[focus.digests]]\nname = \"N\"\nmatch = [\"from:a@b.test\"]\ncadence = \"daily\"\nat = \"08:00\"\n",
        );
        assert_eq!(rule_when(&daily).as_deref(), Some("Daily, 08:00"));
        let monthly = toml(
            "[[focus.digests]]\nname = \"N\"\nmatch = [\"from:a@b.test\"]\ncadence = \"monthly\"\nday = 1\nat = \"09:00\"\n",
        );
        assert_eq!(rule_when(&monthly).as_deref(), Some("Monthly, day 1 09:00"));
    }

    #[test]
    fn the_window_says_how_much_from_how_many_and_when() {
        use chrono::TimeZone as _;
        let now = chrono::Local
            .with_ymd_and_hms(2026, 9, 26, 16, 9, 0)
            .single()
            .expect("a time");
        let at = chrono::Local
            .with_ymd_and_hms(2026, 9, 26, 16, 0, 0)
            .single()
            .expect("a time");
        assert_eq!(
            window_subtitle(14, 6, at, now),
            "14 messages from 6 senders \u{b7} came due today 16:00"
        );
        assert_eq!(
            window_subtitle(1, 1, at - chrono::Duration::days(1), now),
            "1 message from 1 sender \u{b7} came due Fri 25 Sep 16:00"
        );
    }

    #[test]
    fn the_rule_dialog_says_what_it_would_catch_and_what_it_is_called() {
        assert_eq!(
            preview_heading(9),
            "Would have caught 9 messages in the last 90 days"
        );
        assert_eq!(
            preview_heading(1),
            "Would have caught 1 message in the last 90 days"
        );
        assert_eq!(preview_more(9, 4).as_deref(), Some("and 5 more"));
        assert_eq!(preview_more(3, 3), None);
        let names = |names: &[&str]| {
            names
                .iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(rule_name(&names(&["Ledger"])), "Ledger");
        assert_eq!(
            rule_name(&names(&["Ledger", "Forge", "Rates", "Tides"])),
            "Ledger, Forge and 2 others"
        );
        use chrono::TimeZone as _;
        let now = chrono::Local
            .with_ymd_and_hms(2026, 9, 26, 16, 9, 0)
            .single()
            .expect("a time");
        assert_eq!(preview_day(now, now), "Today");
        assert_eq!(preview_day(now - chrono::Duration::days(3), now), "Wed");
        assert_eq!(preview_day(now - chrono::Duration::days(14), now), "12 Sep");
    }

    #[test]
    fn the_rules_list_says_what_each_holds_and_how_to_make_one() {
        assert_eq!(holds(3), "holds 3");
        assert_eq!(
            rules_empty_hint(postio_core::Keymap::defaults()),
            "Press d on a message to digest its sender"
        );
        // Linux's spelling; on a Mac `Delete` is the BackSpace key.
        let linux = postio_core::Keymap::resolve_on(
            &Default::default(),
            postio_config::paths::Platform::Freedesktop,
        );
        assert_eq!(
            crate::hints::line(&rules_footer(&linux)),
            "Return edit \u{b7} Delete remove and release \u{b7} Escape inbox"
        );
        assert!(remove_body(1).contains("1 message "));
    }

    fn statement(topic: &str, number: u32) -> postio_model::summary::SummaryStatement {
        postio_model::summary::SummaryStatement {
            topic: topic.to_owned(),
            text: format!("s{number}"),
            reference: postio_model::summary::SummaryReference {
                number,
                message: postio_model::MessageId::new(i64::from(number)),
                excerpt: String::new(),
            },
        }
    }

    #[test]
    fn pages_toggle_only_with_a_summary_and_back_leaves_only_an_email() {
        assert_eq!(Page::opening(true), Page::Summary);
        assert_eq!(Page::opening(false), Page::List);
        assert_eq!(Page::Summary.toggled(true), Some(Page::List));
        assert_eq!(Page::List.toggled(true), Some(Page::Summary));
        assert_eq!(Page::List.toggled(false), None);
        assert_eq!(Page::Email.toggled(true), None);
        assert_eq!(Page::Email.back(), Some(Page::Summary));
        assert_eq!(Page::List.back(), None);
        assert!(!Page::Email.shows_header());
    }

    #[test]
    fn stepping_a_reference_clamps_to_the_summarys_ends() {
        assert_eq!(step_reference(Some(0), -1, 3), Some(0));
        assert_eq!(step_reference(Some(1), 1, 3), Some(2));
        assert_eq!(step_reference(Some(2), 1, 3), Some(2));
        assert_eq!(step_reference(None, 1, 3), Some(1));
        assert_eq!(step_reference(None, 1, 0), None);
    }

    #[test]
    fn statements_group_by_consecutive_topic() {
        let all = vec![
            statement("Sync", 1),
            statement("Sync", 2),
            statement("Town", 3),
            statement("Sync", 4),
        ];
        let runs = topics(&all);
        let shape: Vec<(&str, usize, usize)> = runs
            .iter()
            .map(|run| (run.name, run.start, run.statements.len()))
            .collect();
        assert_eq!(shape, vec![("Sync", 0, 2), ("Town", 2, 1), ("Sync", 3, 1)]);
    }

    #[test]
    fn the_digest_windows_words() {
        use postio_model::listing::Cadence;
        assert_eq!(
            window_title(Some(Cadence::Weekly), "Newsletters"),
            "Weekly \u{b7} Newsletters"
        );
        assert_eq!(window_title(None, "Newsletters"), "Newsletters");
        assert_eq!(archive_all(14), "Archive all 14");
        assert_eq!(rule_line(None), "Edit rule and cadence");
        assert_eq!(
            rule_line(Some("Weekly, Sunday 09:00")),
            "Weekly, Sunday 09:00 \u{b7} Edit rule and cadence"
        );
        assert_eq!(source_line(Some(1), 14), "Source 2 of 14");
        assert_eq!(source_line(None, 14), "Source 0 of 14");
        assert!(cited_banner(3).starts_with("Cited as 3 in the summary"));
        assert_eq!(rule_count(1), "1 rule");
        assert_eq!(rule_count(4), "4 rules");
    }

    #[test]
    fn like_this_without_a_model_names_the_table_to_fill_in() {
        assert!(LIKE_THIS_NEEDS_A_MODEL.contains("[focus.model]"));
        assert!(LIKE_THIS_NEEDS_A_MODEL.contains("like_this"));
    }

    #[test]
    fn a_new_rule_is_weekly_on_sunday_at_nine() {
        let due = Schedule::new_rule().due().expect("a rule");
        assert_eq!(
            due,
            postio_config::Due::Weekly {
                day: chrono::Weekday::Sun,
                at: chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            }
        );
    }

    #[test]
    fn a_schedule_round_trips_and_a_bad_time_says_so() {
        let monthly = postio_config::Due::Monthly {
            day: 15,
            at: chrono::NaiveTime::from_hms_opt(18, 30, 0).unwrap(),
        };
        let shown = Schedule::of(&monthly);
        assert_eq!(
            (shown.cadence, shown.month_day, shown.at.as_str()),
            (2, 14, "18:30")
        );
        assert_eq!(shown.due(), Ok(monthly));
        let mut bad = Schedule::new_rule();
        bad.at = "nine".into();
        assert_eq!(bad.due(), Err(TIME_ERROR.to_owned()));
    }

    #[test]
    fn senders_make_from_queries_and_anything_else_opens_as_a_query() {
        let senders = vec![postio_model::EmailAddress::new(
            Some("Ada"),
            "Ada@Example.com",
        )];
        assert_eq!(sender_queries(&senders), vec!["from:ada@example.com"]);
        assert_eq!(sender_line(&senders), "ada@example.com");
        assert!(is_sender_rule(&sender_queries(&senders)));
        assert!(!is_sender_rule(&["list:weekly.example.org".to_owned()]));
        assert_eq!(split_queries(" a , ,b "), vec!["a", "b"]);
        assert_eq!(new_rule_heading(2), "Digest these senders");
        assert_eq!(create_words(true), "Save");
    }
}
