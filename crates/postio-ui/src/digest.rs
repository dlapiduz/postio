//! Digest rules read in the one query language: `postio-config` stores queries as text, so
//! whether each reads in full is checked here.

use chrono::NaiveDate;
use postio_config::DigestRule;
use postio_search::query::TokenKind;

/// Each query that does not read in full (blank, or an operator it could not read), as a
/// sentence naming the rule. A rule with one is not applied: it would hold mail it never meant.
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

/// "Weekly, Sunday 09:00"; `None` for a rule whose cadence, day or time names no time.
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

pub fn preview_heading(count: u32) -> String {
    let messages = if count == 1 { "message" } else { "messages" };
    format!("Would have caught {count} {messages} in the last 90 days")
}

pub fn preview_more(count: u32, shown: usize) -> Option<String> {
    let rest = (count as usize).saturating_sub(shown);
    (rest > 0).then(|| format!("and {rest} more"))
}

pub fn rule_note(senders: usize) -> &'static str {
    if senders > 1 {
        "Mail from these senders with an invite, question or to-do still comes straight to the inbox."
    } else {
        "Mail from this sender with an invite, question or to-do still comes straight to the inbox."
    }
}

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

pub fn held_place(rule: &str, delivered: bool) -> String {
    if delivered {
        format!("digest \u{b7} {rule}")
    } else {
        format!("held \u{b7} {rule}")
    }
}

pub const RULES_TITLE: &str = "Digest rules";

pub const RULES_SUBTITLE: &str =
    "Held mail skips the inbox until its digest comes \u{b7} search still finds it";

pub const RULES_EMPTY: &str = "No digest rules yet";

pub fn rules_empty_hint(keymap: &postio_core::Keymap) -> String {
    match crate::hints::key(keymap, postio_core::CommandId::DigestRule) {
        Some(key) => format!("Press {key} on a message to digest its sender"),
        None => "Digest a sender from a message".to_owned(),
    }
}

pub fn holds(count: u32) -> String {
    format!("holds {count}")
}

pub fn next_delivery(at: chrono::DateTime<chrono::Local>) -> String {
    format!("next {}", at.format("%a %-d %b %H:%M"))
}

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

pub fn remove_body(holds: u32) -> String {
    let messages = if holds == 1 { "message" } else { "messages" };
    format!(
        "What it holds now \u{2014} {holds} {messages} \u{2014} comes to the inbox, and its \
         senders\u{2019} mail is no longer held."
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Summary,
    List,
    Email,
}

impl Page {
    pub fn opening(has_summary: bool) -> Page {
        if has_summary {
            Page::Summary
        } else {
            Page::List
        }
    }

    pub fn toggled(self, has_summary: bool) -> Option<Page> {
        match (self, has_summary) {
            (_, false) | (Page::Email, _) => None,
            (Page::Summary, true) => Some(Page::List),
            (Page::List, true) => Some(Page::Summary),
        }
    }

    pub fn back(self) -> Option<Page> {
        (self == Page::Email).then_some(Page::Summary)
    }

    pub fn shows_header(self) -> bool {
        self != Page::Email
    }
}

pub fn step_reference(current: Option<usize>, by: i32, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let next = (current.unwrap_or(0) as i32 + by).clamp(0, len as i32 - 1);
    Some(next as usize)
}

pub fn window_title(cadence: Option<postio_model::listing::Cadence>, rule: &str) -> String {
    match crate::focus_row::digest_title(cadence).split_once(" \u{b7} ") {
        Some((cadence, _)) => format!("{cadence} \u{b7} {rule}"),
        None => rule.to_owned(),
    }
}

pub fn archive_all(count: u32) -> String {
    format!("Archive all {count}")
}

pub fn rule_line(when: Option<&str>) -> String {
    match when {
        Some(when) => format!("{when} \u{b7} Edit rule and cadence"),
        None => "Edit rule and cadence".to_owned(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Topic<'a> {
    pub name: &'a str,
    pub start: usize,
    pub statements: &'a [postio_model::summary::SummaryStatement],
}

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

/// `Reference 3 · Ada Moreno · The rate decision`; a subject alone read as a third heading under
/// the last group.
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

pub fn summary_footer(messages: u32) -> String {
    format!(
        "Written on this computer by the local model from these {messages} messages only. \
         Every statement links to the email it came from."
    )
}

pub const OPEN_FULL_EMAIL: &str = "open the full email";

pub fn cited_banner(number: u32) -> String {
    format!(
        "Cited as {number} in the summary; the passage is highlighted. \
         Esc goes back to the summary in this same window."
    )
}

pub fn source_line(at: Option<usize>, total: usize) -> String {
    format!("Source {} of {total}", at.map_or(0, |index| index + 1))
}

pub const PREVIEW_DAYS: i64 = 90;

pub const CADENCES: [(postio_model::listing::Cadence, &str); 3] = [
    (postio_model::listing::Cadence::Daily, "Daily"),
    (postio_model::listing::Cadence::Weekly, "Weekly"),
    (postio_model::listing::Cadence::Monthly, "Monthly"),
];

pub const WEEKDAYS: [chrono::Weekday; 7] = [
    chrono::Weekday::Mon,
    chrono::Weekday::Tue,
    chrono::Weekday::Wed,
    chrono::Weekday::Thu,
    chrono::Weekday::Fri,
    chrono::Weekday::Sat,
    chrono::Weekday::Sun,
];

/// A new rule starts weekly, Sunday, 09:00 (indices into `CADENCES` and `WEEKDAYS`).
pub const DEFAULT_CADENCE: usize = 1;
pub const DEFAULT_WEEKDAY: usize = 6;
pub const DEFAULT_MONTH_DAY: usize = 0;
pub const DEFAULT_TIME: &str = "09:00";

pub const TIME_ERROR: &str = "Give the time as 24-hour HH:MM, such as 09:00";

pub const LIKE_THIS_NEEDS_A_MODEL: &str =
    "Digest mail like this needs a model: turn like_this on under [focus.model] in config.toml";

pub const NOTHING_ALIKE: &str = "The model found nothing alike to digest";

pub const QUERY_PLACEHOLDER: &str = "list:weekly.example.org or a search";

pub fn new_rule_heading(senders: usize) -> &'static str {
    if senders > 1 {
        "Digest these senders"
    } else {
        "Digest this sender"
    }
}

pub fn edit_rule_heading(name: &str) -> String {
    format!("Digest rule \u{b7} {name}")
}

pub fn sender_queries(senders: &[postio_model::EmailAddress]) -> Vec<String> {
    senders
        .iter()
        .map(|sender| format!("from:{}", sender.address.to_lowercase()))
        .collect()
}

pub fn sender_line(senders: &[postio_model::EmailAddress]) -> String {
    senders
        .iter()
        .map(|sender| sender.address.to_lowercase())
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn is_sender_rule(queries: &[String]) -> bool {
    queries
        .iter()
        .all(|query| query.trim().to_lowercase().starts_with("from:"))
}

pub fn split_queries(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .map(str::to_owned)
        .collect()
}

pub fn create_words(editing: bool) -> &'static str {
    if editing { "Save" } else { "Create" }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    pub cadence: usize,
    pub weekday: usize,
    pub month_day: usize,
    pub at: String,
}

impl Schedule {
    pub fn new_rule() -> Self {
        Schedule {
            cadence: DEFAULT_CADENCE,
            weekday: DEFAULT_WEEKDAY,
            month_day: DEFAULT_MONTH_DAY,
            at: DEFAULT_TIME.to_owned(),
        }
    }

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

    pub fn of_rule(rule: &DigestRule) -> Self {
        rule.due()
            .map_or_else(|_| Schedule::new_rule(), |due| Schedule::of(&due))
    }

    pub fn cadence_kind(&self) -> postio_model::listing::Cadence {
        CADENCES[self.cadence.min(2)].0
    }

    pub fn due(&self) -> Result<postio_config::Due, String> {
        use postio_config::Due;
        let at = chrono::NaiveTime::parse_from_str(self.at.trim(), "%H:%M")
            .map_err(|_| TIME_ERROR.to_owned())?;
        Ok(match self.cadence_kind() {
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

pub fn rule_count(count: usize) -> String {
    if count == 1 {
        "1 rule".to_owned()
    } else {
        format!("{count} rules")
    }
}

pub const RULE_ROW_BUTTONS: [(postio_core::CommandId, &str); 2] = [
    (postio_core::CommandId::OpenMessage, "Edit"),
    (postio_core::CommandId::Delete, "Remove"),
];

pub fn list_tab(count: usize) -> String {
    if count == 1 {
        "1 message".to_owned()
    } else {
        format!("{count} messages")
    }
}

pub const FROM: &str = "From";

pub const DELIVER: &str = "Deliver";

pub const MATCH_INSTEAD: &str = "Match a list or a search instead\u{2026}";

pub const LIKE_THIS: &str = "Digest mail like this";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_tab_counts_the_messages() {
        assert_eq!(list_tab(1), "1 message");
        assert_eq!(list_tab(14), "14 messages");
    }

    #[test]
    fn a_topic_heading_carries_its_count_and_its_sources() {
        let mut statements = vec![
            statement("Rates", 1),
            statement("Rates", 2),
            statement("Rates", 3),
            statement("Engineering reading", 4),
        ];
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
        let linux = postio_core::Keymap::resolve_on(
            &Default::default(),
            postio_config::paths::Platform::Freedesktop,
        );
        assert_eq!(
            crate::hints::line(&rules_footer(&linux)),
            "Return edit \u{b7} # remove and release \u{b7} Escape inbox"
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
