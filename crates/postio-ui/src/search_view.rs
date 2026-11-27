//! The words Focus's search dropdown and results view say, composed once so every frontend agrees
//! (spec 010).

use std::time::Duration;

use chrono::{DateTime, Datelike, TimeZone};
use postio_core::{CommandId, Keymap};

use crate::hints::{self, Hint};

pub fn cheat_sheet() -> [(&'static str, &'static str); 8] {
    [
        ("from:", "person or address"),
        ("to:", "recipient"),
        ("subject:", "words in subject"),
        ("in:", "folder"),
        ("label:", "label name"),
        ("has:attachment", ""),
        ("after: before:", "dates"),
        ("-word", "exclude"),
    ]
}

/// Both times are read in their own zone, so pass them in the person's.
pub fn recent_when<Tz: TimeZone>(at: DateTime<Tz>, now: DateTime<Tz>) -> String {
    let (ran, today) = (at.date_naive(), now.date_naive());
    let days = (today - ran).num_days();
    match days {
        ..=0 => "today".to_owned(),
        1 => "yesterday".to_owned(),
        2..=6 => ran.format("%a").to_string(),
        _ if ran.year() == today.year() => ran.format("%-d %b").to_string(),
        _ => ran.format("%-d %b %Y").to_string(),
    }
}

fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn count(total: u64, capped: bool) -> String {
    format!("{}{}", grouped(total), if capped { "+" } else { "" })
}

pub fn facet_count(count: u64, capped: bool) -> String {
    self::count(count, capped)
}

pub fn suggestion_count(count: u64, capped: bool) -> String {
    self::count(count, capped)
}

pub fn footer_count(total: u64, elapsed: Duration, capped: bool) -> String {
    let noun = if total == 1 && !capped {
        "match"
    } else {
        "matches"
    };
    let took = match elapsed.as_millis() {
        0 => "<1 ms".to_owned(),
        ms => format!("{ms} ms"),
    };
    format!("{} {noun} \u{b7} {took}", count(total, capped))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NarrowPill {
    pub op: String,
    pub value: String,
    pub count: String,
}

impl NarrowPill {
    pub fn label(&self) -> String {
        format!("{} {} {}", self.op, self.value, self.count)
    }
}

pub fn narrow_pill(op: &str, value: &str, count: u64, capped: bool) -> NarrowPill {
    NarrowPill {
        op: op.to_owned(),
        value: value.to_owned(),
        count: self::count(count, capped),
    }
}

pub fn show_all(total: u64, capped: bool, typed: Option<&str>) -> String {
    let for_typed = typed
        .map(|typed| format!(" for \u{201c}{typed}\u{201d}"))
        .unwrap_or_default();
    if total == 1 && !capped {
        return format!("Show 1 result{for_typed}");
    }
    format!("Show all {} results{for_typed}", count(total, capped))
}

pub const PLACEHOLDER: &str = "Search mail, people and files, or type > for commands";
pub const RESTING_PLACEHOLDER: &str = "Search mail or run a command";
pub const RECENT: &str = "Recent";
pub const FORGETS_ONE: &str = "forgets one";
pub const SAVED_SEARCHES: &str = "Saved searches";
pub const SEARCH_BY: &str = "Search by";
pub const JUST_TYPE_IT: &str = "Or just type it:";
pub const BECOMES: &str = "becomes";
pub const EXAMPLE: &str = "invoices from ada last month";
pub const TOP_HITS: &str = "Top hits";
pub const TOP_HITS_NOTE: &str = "ranked by sender, recency and where the words matched";
pub const NARROW_TO: &str = "Narrow to";
pub const SHOW_ALL_DETAIL: &str = "in the main window, with filters, a timeline and Quick Look";
pub const NOTHING_DETAIL: &str = "Return offers looser searches";

pub fn results_count(n: u64, capped: bool) -> String {
    match (n, capped) {
        (1, false) => "1 result".to_owned(),
        (n, capped) => format!("{} results", count(n, capped)),
    }
}

pub fn saved_count(n: u64, capped: bool) -> String {
    count(n, capped)
}

/// A two-digit year for another year: the full one does not fit the results' 62-pt column.
pub fn hit_date<Tz: TimeZone>(at: DateTime<Tz>, now: DateTime<Tz>) -> String {
    let (day, today) = (at.date_naive(), now.date_naive());
    if day.year() == today.year() {
        day.format("%-d %b").to_string()
    } else {
        day.format("%-d %b %y").to_string()
    }
}

/// IMAP names the inbox INBOX in any case (RFC 3501 §5.1); search calls it "Inbox".
pub fn folder_name(name: &str) -> String {
    if name.eq_ignore_ascii_case("inbox") {
        "Inbox".to_owned()
    } else {
        name.to_owned()
    }
}

pub fn in_folder(name: &str) -> String {
    let name = folder_name(name);
    if name.contains(char::is_whitespace) {
        format!("in:\"{name}\"")
    } else {
        format!("in:{name}")
    }
}

/// `account` is added where there are several, so one message filed in two accounts does not read
/// as one row twice.
pub fn hit_place(
    held: Option<(&str, bool)>,
    folder: Option<&str>,
    account: Option<&str>,
) -> Option<String> {
    let place = match held {
        Some((rule, delivered)) => Some(crate::digest::held_place(rule, delivered)),
        None => folder.map(in_folder),
    };
    match (place, account) {
        (Some(place), Some(account)) => Some(format!("{place} \u{b7} {account}")),
        (place, account) => place.or_else(|| account.map(str::to_owned)),
    }
}

/// The panel's own walk, which the toolkit keeps; not a command.
fn arrows(label: &str) -> Hint {
    hints::fixed(
        "Up Down",
        label,
        "the arrows move the dropdown's highlight, which the toolkit keeps",
    )
}

pub fn empty_hints(keymap: &Keymap, recents: bool) -> Vec<Hint> {
    let mut hints = vec![arrows("move")];
    if recents {
        hints.push(hints::fixed(
            "Return",
            "run again",
            "Return runs the highlighted row: the field's own key, not a command",
        ));
    }
    hints.extend(hints::run(
        keymap,
        &[
            CommandId::SavedSearch1,
            CommandId::SavedSearch2,
            CommandId::SavedSearch3,
            CommandId::SavedSearch4,
        ],
        "saved",
    ));
    hints.push(hints::fixed(
        ">",
        "commands",
        "`>` typed first is the bar's commands-only prefix, not a key",
    ));
    hints
}

pub fn local_index(messages: u64) -> String {
    let noun = if messages == 1 { "message" } else { "messages" };
    format!("local index \u{b7} {} {noun}", grouped(messages))
}

pub fn words_hints(keymap: &Keymap) -> Vec<Hint> {
    let mut hints = vec![
        arrows("move"),
        hints::fixed(
            "Return",
            "open",
            "Return runs the highlighted row: the field's own key, not a command",
        ),
    ];
    hints.extend(hints::hint(
        keymap,
        CommandId::ShowAllResults,
        "all results",
    ));
    hints.push(hints::fixed(
        "Tab",
        "add first filter",
        "Tab in the field is the toolkit's key; the bar takes it only for a pill",
    ));
    hints
}

use postio_search::query::{Filter, State};
use postio_search::results::{Location, RankReason, Source};

/// A subject the source tag already names is not said again as a reason (#2085).
pub fn reason_beside(reasons: &[RankReason], sources: &[Source]) -> String {
    let tagged_subject = sources
        .iter()
        .any(|source| matches!(source, Source::Subject));
    let kept: Vec<RankReason> = reasons
        .iter()
        .filter(|reason| !(tagged_subject && matches!(reason, RankReason::InSubject)))
        .cloned()
        .collect();
    reason_line(&kept)
}

/// At most two parts: the strongest reason, then the match count if more than one. Says flagged,
/// never starred.
pub fn reason_line(reasons: &[RankReason]) -> String {
    let mut words = reasons.iter().filter_map(|reason| {
        Some(match reason {
            RankReason::Replied => "you replied",
            RankReason::Flagged => "you flagged",
            RankReason::FrequentSender => "frequent sender",
            RankReason::InSubject => "subject",
            RankReason::InSender => "sender",
            RankReason::InFileName => "file name",
            RankReason::InFileContent => "file contents",
            RankReason::Matches(_) => return None,
        })
    });
    let matched = reasons
        .iter()
        .find_map(|reason| match reason {
            RankReason::Matches(n) => Some(*n),
            _ => None,
        })
        .filter(|n| *n > 1)
        .map(|n| format!("{} matches", grouped(u64::from(n))));
    let first = words.next().map(str::to_owned);
    let second = matched.or_else(|| words.next().map(str::to_owned));
    [first, second]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" \u{b7} ")
}

pub fn source_tag(source: &Source) -> String {
    match source {
        Source::Subject => "subject".to_owned(),
        Source::Body => "body".to_owned(),
        Source::Quoted => "quoted text".to_owned(),
        Source::FileName { name, .. } | Source::FileContent { name, .. } => name.clone(),
    }
}

/// A subject is named only when it is the passage's source, i.e. nothing else matched; otherwise
/// the row's first line shows it.
pub fn sources_tag(sources: &[Source]) -> String {
    let is_file =
        |source: &Source| matches!(source, Source::FileName { .. } | Source::FileContent { .. });
    let Some(first) = sources.first() else {
        return String::new();
    };
    let tag = source_tag(first);
    if is_file(first) {
        return tag;
    }
    match sources[1..].iter().find(|source| is_file(source)) {
        Some(file) => format!("{tag} + {}", source_tag(file)),
        None => tag,
    }
}

/// The first match with a passage; else the first that is not the subject (the row's first line
/// draws it); else the subject.
pub fn shown_match(
    matches: &[postio_search::results::Match],
) -> Option<&postio_search::results::Match> {
    matches
        .iter()
        .find(|each| each.passage.is_some())
        .or_else(|| matches.iter().find(|each| each.source != Source::Subject))
        .or_else(|| matches.first())
}

pub fn location(location: &Location) -> String {
    match location {
        Location::Page(page) => format!("Page {page}"),
        Location::Sheet { name, row } => format!("Sheet \u{2018}{name}\u{2019}, row {row}"),
        Location::Slide(slide) => format!("Slide {slide}"),
        Location::Paragraph(paragraph) => format!("Paragraph {paragraph}"),
        Location::Line(line) => format!("Line {line}"),
        Location::Table { index, row } => format!("Table {index}, row {row}"),
        Location::ImageText => "Text in an image".to_owned(),
    }
}

pub const TOP_HITS_RESULTS_NOTE: &str = "why each one ranked is under the sender";
pub const NEWEST_FIRST: &str = "newest first";
pub const EARLIER: &str = "Earlier";
pub const TO_EDIT: &str = "/ to edit";

pub fn month_title(month: chrono::NaiveDate) -> String {
    month.format("%B %Y").to_string()
}

pub fn month_group(month: chrono::NaiveDate, n: u64) -> String {
    group_line(&month_title(month), n)
}

pub fn group_line(title: &str, n: u64) -> String {
    format!("{title} \u{b7} {}", grouped(n))
}

/// Postio says thread, never conversation (#2118).
fn threads(total: u64, capped: bool) -> String {
    let noun = if total == 1 && !capped {
        "thread"
    } else {
        "threads"
    };
    format!("{} {noun}", count(total, capped))
}

pub fn count_line(total: u64, capped: bool) -> String {
    threads(total, capped)
}

pub fn sub_line(files: u64, people: u64, capped: bool) -> String {
    let files = plural(files, capped, "file", "files");
    let people = plural(people, capped, "person", "people");
    format!("{files} \u{b7} {people} \u{b7} last 12 months")
}

/// With no filter it is `sub_line`. A person is said by their first name; `name_of` gives it for an
/// address.
pub fn narrowed_sub_line(
    filters: &[Filter],
    name_of: &dyn Fn(&str) -> Option<String>,
    today: chrono::NaiveDate,
    files: u64,
    people: u64,
    capped: bool,
) -> String {
    let held = |kind: FilterKind| -> Vec<&Filter> {
        filters
            .iter()
            .flat_map(Filter::alternatives)
            .filter(|filter| kind.holds(filter))
            .collect()
    };
    let first_name = |address: &str| {
        name_of(address)
            .and_then(|name| name.split_whitespace().next().map(str::to_owned))
            .unwrap_or_else(|| address.to_owned())
    };
    let values = |kind: FilterKind| -> Vec<String> {
        held(kind)
            .into_iter()
            .filter_map(|filter| match filter {
                Filter::From(who) | Filter::To(who) => Some(first_name(who)),
                Filter::In(name) | Filter::Label(name) => Some(name.clone()),
                _ => None,
            })
            .collect()
    };
    let mut said: Vec<String> = Vec::new();
    let order = [
        (FilterKind::Unread, ""),
        (FilterKind::From, "from"),
        (FilterKind::To, "to"),
        (FilterKind::Anywhere, "in"),
        (FilterKind::Label, "labelled"),
        (FilterKind::Attachment, ""),
        (FilterKind::HasAction, ""),
    ];
    for (kind, word) in order {
        if held(kind).is_empty() {
            continue;
        }
        said.push(match kind {
            FilterKind::Unread => "unread".to_owned(),
            FilterKind::Attachment => "with attachments".to_owned(),
            FilterKind::HasAction => "with an action".to_owned(),
            _ => format!("{word} {}", values(kind).join(" or ")),
        });
    }
    if !held(FilterKind::Date).is_empty() {
        let label = filter_button_label(FilterKind::Date, filters, name_of, today);
        said.push(match label.split_once(' ') {
            Some((first @ ("Since" | "Before"), rest)) => {
                format!("{} {rest}", first.to_lowercase())
            }
            _ => label,
        });
    }
    if said.is_empty() {
        return sub_line(files, people, capped);
    }
    let mut line = vec![said.join(" "), plural(files, capped, "file", "files")];
    if held(FilterKind::From).is_empty() && held(FilterKind::To).is_empty() {
        line.push(plural(people, capped, "person", "people"));
    }
    line.join(" \u{b7} ")
}

fn plural(n: u64, capped: bool, one: &str, many: &str) -> String {
    match (n, capped) {
        (1, false) => format!("1 {one}"),
        (n, capped) => format!("{} {many}", count(n, capped)),
    }
}

pub fn results_footer(total: u64, capped: bool, elapsed: Duration) -> String {
    footer_of(&threads(total, capped), elapsed)
}

/// Counts the cards under the Files tab, not the search's threads (#2183).
pub fn files_count_line(files: u64, capped: bool) -> String {
    plural(files, capped, "file", "files")
}

pub fn files_footer(files: u64, capped: bool, elapsed: Duration) -> String {
    footer_of(&files_count_line(files, capped), elapsed)
}

fn footer_of(counted: &str, elapsed: Duration) -> String {
    let took = match elapsed.as_millis() {
        0 => "<1 ms".to_owned(),
        ms => format!("{ms} ms"),
    };
    format!("{counted} \u{b7} local index \u{b7} {took}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FilterKind {
    From,
    To,
    Date,
    Anywhere,
    Label,
    Attachment,
    HasAction,
    Unread,
}

impl FilterKind {
    pub const ALL: [FilterKind; 8] = [
        FilterKind::From,
        FilterKind::To,
        FilterKind::Date,
        FilterKind::Anywhere,
        FilterKind::Label,
        FilterKind::Attachment,
        FilterKind::HasAction,
        FilterKind::Unread,
    ];

    pub fn title(self) -> &'static str {
        match self {
            FilterKind::From => "From",
            FilterKind::To => "To",
            FilterKind::Date => "Date",
            FilterKind::Anywhere => "Anywhere",
            FilterKind::Label => "Label",
            FilterKind::Attachment => "Attachment",
            FilterKind::HasAction => "Has action",
            FilterKind::Unread => "Unread",
        }
    }

    /// `None` for a button with a popover.
    pub fn toggle_term(self) -> Option<(&'static str, &'static str)> {
        match self {
            FilterKind::Attachment => Some(("has", "attachment")),
            FilterKind::HasAction => Some(("has", "action")),
            FilterKind::Unread => Some(("is", "unread")),
            FilterKind::From
            | FilterKind::To
            | FilterKind::Date
            | FilterKind::Anywhere
            | FilterKind::Label => None,
        }
    }

    pub fn has_popover(self) -> bool {
        matches!(
            self,
            FilterKind::From
                | FilterKind::To
                | FilterKind::Date
                | FilterKind::Anywhere
                | FilterKind::Label
        )
    }

    pub fn holds(self, filter: &Filter) -> bool {
        let Some(filter) = filter.alternatives().first() else {
            return false;
        };
        match self {
            FilterKind::From => matches!(filter, Filter::From(_)),
            FilterKind::To => matches!(filter, Filter::To(_)),
            FilterKind::Date => matches!(filter, Filter::After(_) | Filter::Before(_)),
            FilterKind::Anywhere => matches!(filter, Filter::In(_)),
            FilterKind::Label => matches!(filter, Filter::Label(_)),
            FilterKind::Attachment => matches!(filter, Filter::HasAttachment),
            FilterKind::HasAction => matches!(filter, Filter::HasAction),
            FilterKind::Unread => matches!(filter, Filter::Is(State::Unread)),
        }
    }
}

fn month_word(date: chrono::NaiveDate, today: chrono::NaiveDate) -> String {
    if date.year() == today.year() {
        date.format("%B").to_string()
    } else {
        date.format("%B %Y").to_string()
    }
}

fn day_word(date: chrono::NaiveDate, today: chrono::NaiveDate) -> String {
    match (date.day(), date.year() == today.year()) {
        (1, _) => month_word(date, today),
        (_, true) => date.format("%-d %b").to_string(),
        (_, false) => date.format("%-d %b %Y").to_string(),
    }
}

pub fn filter_button_label(
    kind: FilterKind,
    filters: &[Filter],
    name_of: &dyn Fn(&str) -> Option<String>,
    today: chrono::NaiveDate,
) -> String {
    let held: Vec<&Filter> = filters
        .iter()
        .flat_map(Filter::alternatives)
        .filter(|filter| kind.holds(filter))
        .collect();
    if held.is_empty() || !kind.has_popover() {
        return kind.title().to_owned();
    }
    if kind == FilterKind::Date {
        let after = held.iter().find_map(|filter| match filter {
            Filter::After(date) => Some(*date),
            _ => None,
        });
        let before = held.iter().find_map(|filter| match filter {
            Filter::Before(date) => Some(*date),
            _ => None,
        });
        return match (after, before) {
            (Some(after), Some(before)) if after.day() == 1 && before.day() == 1 => {
                let last = before.pred_opt().unwrap_or(before);
                if (last.year(), last.month()) == (after.year(), after.month()) {
                    month_word(after, today)
                } else {
                    format!(
                        "{} \u{2013} {}",
                        month_word(after, today),
                        month_word(last, today)
                    )
                }
            }
            (Some(after), Some(before)) => {
                format!(
                    "{} \u{2013} {}",
                    day_word(after, today),
                    day_word(before, today)
                )
            }
            (Some(after), None) => format!("Since {}", day_word(after, today)),
            (None, Some(before)) => format!("Before {}", day_word(before, today)),
            (None, None) => kind.title().to_owned(),
        };
    }
    let value = |filter: &Filter| match filter {
        Filter::From(who) | Filter::To(who) => name_of(who).unwrap_or_else(|| who.clone()),
        Filter::In(name) => folder_name(name),
        Filter::Label(name) => name.clone(),
        _ => String::new(),
    };
    let more = match held.len() {
        1 => String::new(),
        n => format!(" +{}", n - 1),
    };
    let title = match kind {
        FilterKind::Anywhere => "In",
        other => other.title(),
    };
    format!("{title}: {}{more}", value(held[0]))
}

pub const A11Y_QUERY: &str = "Search query";
pub const A11Y_SUGGESTIONS: &str = "Search suggestions";
pub const A11Y_RESULTS: &str = "Search results";
pub const A11Y_FILES: &str = "Files";
pub const A11Y_PEOPLE: &str = "People";

pub fn dropdown_row_accessible(
    title: &str,
    detail: &str,
    folder: Option<&str>,
    right: Option<&str>,
) -> String {
    [Some(title), Some(detail), folder, right]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn accessible_row(
    sender: &str,
    subject: &str,
    tag: &str,
    passage: &str,
    folder: &str,
    date: &str,
) -> String {
    let matched = if tag.is_empty() || passage.is_empty() {
        String::new()
    } else {
        format!(", matched in {tag}: {passage}")
    };
    format!("{sender}, {subject}{matched}, {folder}, {date}")
}

pub fn results_hints(keymap: &Keymap) -> Vec<Hint> {
    let mut out = Vec::new();
    out.extend(hints::pair(
        keymap,
        CommandId::NextMessage,
        CommandId::PrevMessage,
        "move",
    ));
    out.extend(hints::hint(keymap, CommandId::QuickLook, "Quick Look"));
    out.extend(hints::hint(keymap, CommandId::OpenMessage, "open"));
    out.extend(hints::hint(keymap, CommandId::ToggleSelection, "select"));
    out.extend(hints::hint(keymap, CommandId::Search, "edit query"));
    out.extend(hints::hint(keymap, CommandId::Back, "back to inbox"));
    out
}

pub fn bulk_hints(keymap: &Keymap) -> Vec<Hint> {
    crate::focus_dialog::RESULTS_BULK
        .iter()
        .filter_map(|verb| hints::hint(keymap, verb.command, verb.label))
        .collect()
}

/// `None` when nothing is bound to Select all.
pub fn select_all_hint(keymap: &Keymap, total: u64, capped: bool) -> Option<Hint> {
    hints::hint(
        keymap,
        CommandId::SelectAll,
        &format!("select all {}", count(total, capped)),
    )
}

pub fn save_name(
    query: &postio_search::ParsedQuery,
    name_of: &dyn Fn(&str) -> Option<String>,
) -> String {
    use postio_search::query::Filter;
    let mut said: Vec<String> = query
        .text_terms()
        .filter(|term| !term.negated)
        .map(|term| term.value.clone())
        .collect();
    let first_name = |address: &str| {
        name_of(address)
            .and_then(|name| name.split_whitespace().next().map(str::to_owned))
            .unwrap_or_else(|| address.to_owned())
    };
    for clause in query.filters().filter(|clause| !clause.negated) {
        let people = |pick: fn(&Filter) -> Option<&String>| {
            let who: Vec<String> = clause
                .filter
                .alternatives()
                .iter()
                .filter_map(|filter| pick(filter).map(|who| first_name(who)))
                .collect();
            (!who.is_empty()).then(|| who.join(" or "))
        };
        if let Some(who) = people(|filter| match filter {
            Filter::From(who) => Some(who),
            _ => None,
        }) {
            said.push(format!("from {who}"));
        } else if let Some(who) = people(|filter| match filter {
            Filter::To(who) => Some(who),
            _ => None,
        }) {
            said.push(format!("to {who}"));
        }
    }
    let name = said.join(" ");
    let mut letters = name.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => query.input().trim().to_owned(),
    }
}

pub const SAVE_TITLE: &str = "Save as a saved search";
pub const SAVE_NAME: &str = "Name";
pub const SAVE_PIN: &str = "Pin to saved searches";
pub const SAVE_PIN_NOTE: &str = "Appears at the top of search as";

pub fn save_pin_note_no_key(taken: Option<&str>) -> String {
    match taken {
        Some(keys) => format!("Listed after {keys} in search, with no key"),
        None => "Listed in search, with no key".to_owned(),
    }
}
pub const SAVE_NOTIFY: &str = "Notify when new mail matches";
pub const SAVE_NOTIFY_NOTE: &str = "A quiet badge, not a banner";
pub const SAVE_ROLLING: &str = "Keep the date rolling";
pub const SAVE_CANCEL: &str = "Cancel";
pub const SAVE: &str = "Save";

pub fn rolling_note(after: chrono::NaiveDate, today: chrono::NaiveDate) -> String {
    let days = (today - after).num_days().max(0);
    format!(
        "Off: always since {}. On: always the last {} days",
        after.format("%-d %B"),
        grouped(days as u64)
    )
}

/// A quiet badge, never a banner.
pub fn new_badge(n: u64, capped: bool) -> String {
    format!("{} new", count(n, capped))
}

pub const TABS: [&str; 3] = ["Threads", "Files", "People"];

pub fn tab_count(total: u64, capped: bool) -> String {
    count(total, capped)
}

pub const BACK_TO_INBOX: &str = "Inbox";
pub const SAVE_SEARCH: &str = "Save search";
pub const SORT: &str = "Sort";
pub const BEST_MATCH: &str = "Best match";
pub const NEWEST: &str = "Newest";
pub const TIMELINE_HINT: &str = "Matches by month \u{b7} drag across months to narrow";

pub fn popover_placeholder(kind: FilterKind) -> &'static str {
    match kind {
        FilterKind::From | FilterKind::To => "Filter people in these results",
        FilterKind::Anywhere => "Filter folders",
        FilterKind::Label => "Filter labels",
        _ => "",
    }
}

pub fn popover_hints(kind: FilterKind) -> Vec<Hint> {
    let hint = |key: &str, label: &str| Hint {
        key: key.to_owned(),
        label: label.to_owned(),
    };
    let mut out = Vec::new();
    if kind != FilterKind::Date {
        out.push(hint("space", "toggle"));
        out.push(hint("alt", "-click excludes"));
    }
    out.push(hint("Return", "apply"));
    out
}

pub fn previewing(what: &str) -> String {
    format!("previewing {what} \u{b7} \u{21a9} applies")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatePreset {
    pub label: &'static str,
    /// `None` for Any time, which takes the dates out.
    pub start: Option<chrono::NaiveDate>,
    pub count_at: usize,
}

/// Custom… (`CUSTOM_DATE`) follows them and writes nothing itself.
pub fn date_presets(today: chrono::NaiveDate) -> [DatePreset; 5] {
    let starts = postio_search::facets::preset_starts(today);
    let preset = |label, count_at: usize| DatePreset {
        label,
        start: starts[count_at],
        count_at,
    };
    [
        preset("Any time", 4),
        preset("Last 7 days", 0),
        preset("Last 30 days", 1),
        preset("This quarter", 2),
        preset("This year", 3),
    ]
}

pub const CUSTOM_DATE: &str = "Custom\u{2026}";

pub const DATE_WORDS_HINT: &str = "Type a date in plain words, or drag across the months.";

pub fn date_parsed(terms: &str) -> String {
    format!("\u{2192} {terms}")
}

pub fn month_range(first: chrono::NaiveDate, last: chrono::NaiveDate, year: bool) -> String {
    let (first, last) = (first.min(last), first.max(last));
    let same_month = (first.year(), first.month()) == (last.year(), last.month());
    if first.year() != last.year() {
        return format!(
            "{} \u{2013} {}",
            first.format("%b %Y"),
            last.format("%b %Y")
        );
    }
    let tail = if year {
        format!(" {}", last.year())
    } else {
        String::new()
    };
    if same_month {
        format!("{}{tail}", first.format("%b"))
    } else {
        format!(
            "{} \u{2013} {}{tail}",
            first.format("%b"),
            last.format("%b")
        )
    }
}

pub fn range_selected(range: &str) -> String {
    format!("{range} selected \u{b7} drag to change")
}

pub fn timeline_step(keymap: &Keymap) -> Option<Hint> {
    hints::pair(
        keymap,
        CommandId::StepRangeBack,
        CommandId::StepRangeForward,
        "steps a month",
    )
}

pub fn date_result(kept: u64, of: u64, capped: bool) -> String {
    format!("{} of {}", count(kept, capped), count(of, capped))
}

pub fn checked_line(n: u64) -> String {
    format!("{} selected", grouped(n))
}

pub const QUICK_LOOK: &str = "Quick Look";

pub fn quick_look_position(at: u64, of: u64) -> String {
    format!("{} of {}", grouped(at.saturating_add(1)), grouped(of))
}

pub fn quick_look_walk(keymap: &Keymap) -> Option<Hint> {
    hints::pair(
        keymap,
        CommandId::NextMessage,
        CommandId::PrevMessage,
        "moves through results while it stays open",
    )
}

pub fn quick_look_actions(keymap: &Keymap) -> Vec<Hint> {
    QUICK_LOOK_ACTIONS
        .into_iter()
        .filter_map(|(id, label)| hints::hint(keymap, id, label))
        .collect()
}

/// A button left out for want of a key leaves its command out too, so a frontend never runs one
/// button's command from another.
pub fn quick_look_commands(keymap: &Keymap) -> Vec<CommandId> {
    QUICK_LOOK_ACTIONS
        .into_iter()
        .filter(|(id, _)| keymap.binding(*id).is_some())
        .map(|(id, _)| id)
        .collect()
}

const QUICK_LOOK_ACTIONS: [(CommandId, &str); 3] = [
    (CommandId::OpenMessage, "Open"),
    (CommandId::Archive, "Archive"),
    (CommandId::QuickLook, "Close"),
];

/// Counted as a result's reason line counts them (#2085): each message once however many cards, and
/// the subject once. `None` is the subject's card.
pub fn matches_of<T: PartialEq>(cards: impl IntoIterator<Item = Option<T>>) -> usize {
    let mut messages: Vec<T> = Vec::new();
    let mut subject = false;
    for card in cards {
        match card {
            Some(message) if !messages.contains(&message) => messages.push(message),
            Some(_) => {}
            None => subject = true,
        }
    }
    messages.len() + usize::from(subject)
}

/// Byte ranges into the typed text (#2082).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldChip {
    pub start: usize,
    pub end: usize,
    pub operator_end: usize,
    pub excluded: bool,
}

pub fn field_chips(typed: &str, today: chrono::NaiveDate) -> Vec<FieldChip> {
    use postio_search::query::TokenKind;
    postio_search::parse(typed, today)
        .tokens()
        .iter()
        .filter_map(|token| {
            let excluded = match &token.kind {
                TokenKind::Filter(clause) => clause.negated,
                TokenKind::Partial(partial) => partial.negated,
                TokenKind::Text(_) => return None,
            };
            let colon = token.raw.find(':')?;
            Some(FieldChip {
                start: token.span.start,
                end: token.span.end,
                operator_end: token.span.start + colon + 1,
                excluded,
            })
        })
        .collect()
}

pub fn matches_line(n: usize) -> String {
    match n {
        1 => "1 match in this thread".to_owned(),
        n => format!("{} matches in this thread", grouped(n as u64)),
    }
}

pub fn matches_hint(keymap: &Keymap) -> Option<Hint> {
    hints::pair(
        keymap,
        CommandId::NextMatch,
        CommandId::PrevMatch,
        "jump between them",
    )
}

pub fn match_place(source: &Source) -> String {
    match source {
        Source::Subject => "Subject".to_owned(),
        Source::Body => "Body".to_owned(),
        Source::Quoted => "Earlier reply".to_owned(),
        Source::FileName { name, .. } | Source::FileContent { name, .. } => name.clone(),
    }
}

/// Nothing for the subject or a quote, whose writer is not known.
pub fn match_when<Tz: TimeZone>(
    source: &Source,
    from: Option<&postio_model::EmailAddress>,
    when: Option<DateTime<Tz>>,
    now: DateTime<Tz>,
) -> String {
    if let Source::FileContent { location: at, .. } = source {
        return location(at);
    }
    let who = from.map(|from| match from.name.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() => name.split_whitespace().next().unwrap_or(name).to_owned(),
        _ => from.address.clone(),
    });
    who.into_iter()
        .chain(when.map(|when| hit_date(when, now)))
        .collect::<Vec<_>>()
        .join(" \u{b7} ")
}

/// With its year when that is not this one's (#2082).
pub fn sent_at<Tz: TimeZone>(at: DateTime<Tz>, now: DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    use chrono::Datelike;
    if at.year() == now.year() {
        at.format("%a %-d %b, %H:%M").to_string()
    } else {
        at.format("%a %-d %b %Y, %H:%M").to_string()
    }
}

/// You first, then each by the header's name (else the address), three at most and a count. `None`
/// when nobody is known.
pub fn sent_to(to: &[postio_search::results::Addressee]) -> Option<String> {
    const NAMED: usize = 3;
    let you = to.iter().any(|each| each.own);
    let said: Vec<String> = you
        .then(|| "you".to_owned())
        .into_iter()
        .chain(to.iter().filter(|each| !each.own).map(|each| {
            each.address
                .name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or(&each.address.address)
                .to_owned()
        }))
        .collect();
    if said.is_empty() {
        return None;
    }
    let named = said[..said.len().min(NAMED)].join(", ");
    Some(match said.len().saturating_sub(NAMED) {
        0 => format!("to {named}"),
        more => format!("to {named} and {more} more"),
    })
}

pub fn thread_size(messages: u32) -> Option<String> {
    (messages > 1).then(|| format!("thread of {messages}"))
}

pub const NO_RESULTS_BODY: &str = "Each line below loosens one filter and shows how many \
                                   threads you would get. Pick one, or press its number.";

pub const NO_RELAXATIONS_BODY: &str =
    "Loosening any one of them still finds nothing. Clear the filters to start from the words.";

pub const COUNTING_RELAXATIONS: &str = "Counting looser searches\u{2026}";

pub const CLEARS_FILTERS: &str = "clears filters";

pub fn nothing_matches(terms: usize) -> String {
    match terms {
        0 | 1 => "Nothing matches this search".to_owned(),
        2 => "Nothing matches both filters".to_owned(),
        n => format!("Nothing matches all {} filters", small_number(n)),
    }
}

fn small_number(n: usize) -> String {
    const WORDS: [&str; 11] = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    ];
    WORDS
        .get(n)
        .map_or_else(|| n.to_string(), |word| (*word).to_owned())
}

/// `query` is the search that found nothing, whose token the relaxation names.
pub fn relaxation_line(
    relaxation: &postio_search::relax::Relaxation,
    query: &postio_search::ParsedQuery,
    name_of: &dyn Fn(&str) -> Option<String>,
    today: chrono::NaiveDate,
) -> String {
    use postio_search::query::TokenKind;
    use postio_search::relax::Loosen;
    let quoted = |what: &str| format!("\u{201c}{what}\u{201d}");
    let remove = |what: &str| format!("Remove {}", quoted(what));
    let (Loosen::Drop { token } | Loosen::Anywhere { token } | Loosen::FolderNotLabel { token }) =
        relaxation.loosen;
    let Some(token) = query.tokens().get(token) else {
        return relaxation.query.clone();
    };
    let clause = match &token.kind {
        TokenKind::Filter(clause) => clause,
        _ => return remove(&token.raw),
    };
    let values = clause.filter.alternatives();
    let Some(filter) = values.first() else {
        return remove(&token.raw);
    };
    let person = |who: &String| {
        name_of(who)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| who.clone())
    };
    let either = |say: &dyn Fn(&Filter) -> Option<String>| {
        values
            .iter()
            .filter_map(say)
            .collect::<Vec<_>>()
            .join(" or ")
    };
    let people = either(&|filter| match filter {
        Filter::From(who) | Filter::To(who) => Some(person(who)),
        _ => None,
    });
    let named = either(&|filter| match filter {
        Filter::Subject(value) | Filter::Label(value) | Filter::In(value) => Some(quoted(value)),
        _ => None,
    });
    match relaxation.loosen {
        Loosen::Anywhere { .. } => match filter {
            Filter::Subject(_) => {
                format!("Look for {named} anywhere, not just the subject")
            }
            _ => remove(&token.raw),
        },
        Loosen::FolderNotLabel { .. } => match filter {
            Filter::Label(_) => format!("Look for a folder called {named}, not a label"),
            _ => remove(&token.raw),
        },
        Loosen::Drop { .. } if clause.negated => remove(&token.raw),
        Loosen::Drop { .. } => match filter {
            Filter::From(_) => format!("Anyone, not just {people}"),
            Filter::To(_) => format!("To anyone, not just {people}"),
            Filter::Subject(_) => format!("Any subject, not just {named}"),
            Filter::Label(_) => format!("Any label, not just {named}"),
            Filter::In(_) => format!("Any folder, not just {named}"),
            Filter::After(_) | Filter::Before(_) => {
                let label = filter_button_label(
                    FilterKind::Date,
                    std::slice::from_ref(filter),
                    name_of,
                    today,
                );
                let mut chars = label.chars();
                let label = match chars.next() {
                    Some(first) => first.to_lowercase().chain(chars).collect(),
                    None => label,
                };
                remove(&label)
            }
            Filter::HasAttachment => remove("has attachment"),
            Filter::HasAction => remove("has action"),
            Filter::Is(state) => remove(match state {
                State::Unread => "unread",
                State::Read => "read",
                State::Flagged => "flagged",
                State::Bulk => "bulk",
                State::Automated => "automated",
            }),
            _ => remove(&token.raw),
        },
    }
}

pub const PICK_RELAXATION: [CommandId; 4] = [
    CommandId::PickRelaxation1,
    CommandId::PickRelaxation2,
    CommandId::PickRelaxation3,
    CommandId::PickRelaxation4,
];

pub fn no_results_hints(keymap: &Keymap, offered: usize) -> Vec<Hint> {
    let mut out = Vec::new();
    let last = offered.min(PICK_RELAXATION.len());
    if last > 0 {
        let first = hints::key(keymap, PICK_RELAXATION[0]);
        let to = hints::key(keymap, PICK_RELAXATION[last - 1]);
        let key = match (first, to) {
            (Some(first), Some(to)) if last > 1 => Some(format!("{first}\u{2013}{to}")),
            (Some(one), _) | (None, Some(one)) => Some(one),
            (None, None) => None,
        };
        out.extend(key.map(|key| Hint {
            key,
            label: "loosen a filter".to_owned(),
        }));
    }
    out.extend(hints::hint(keymap, CommandId::BackToWords, "clear filters"));
    out
}

pub fn relaxation_count(n: u64, capped: bool) -> String {
    threads(n, capped)
}

/// "all" only when every body is indexed, "including attachment contents" only when every
/// attachment has been read: never a claim the index cannot back.
pub fn searched(
    keymap: &Keymap,
    messages: u64,
    corpus_complete: bool,
    contents_complete: bool,
) -> String {
    let what = match (messages, corpus_complete) {
        (1, true) => "the 1 message".to_owned(),
        (1, false) => "1 message".to_owned(),
        (n, true) => format!("all {} messages", grouped(n)),
        (n, false) => format!("{} messages", grouped(n)),
    };
    let contents = if contents_complete {
        ", including attachment contents"
    } else {
        ""
    };
    let rest = if corpus_complete {
        ""
    } else {
        " Some are still being indexed."
    };
    let here = crate::contacts_view::this_computer(keymap);
    format!("Searched {what} on {here}{contents}.{rest}")
}

pub fn searched_footer(messages: u64, elapsed: Duration) -> String {
    let noun = if messages == 1 { "message" } else { "messages" };
    let took = match elapsed.as_millis() {
        0 => "<1 ms".to_owned(),
        ms => format!("{ms} ms"),
    };
    format!("Searched {} {noun} \u{b7} {took}", grouped(messages))
}

pub const SUGGESTIONS: &str = "Suggestions";
pub const SUGGESTIONS_NOTE: &str = "complete the word, or jump to a filter";
pub const AS_A_WORD: &str = "as a word";
pub const LABEL: &str = "label";
pub const TOP_HITS_SO_FAR: &str = "Top hits so far";
pub const TOP_HITS_SO_FAR_NOTE: &str = "update on every keystroke";
pub const PEOPLE_NOTE: &str = "by how often you write to each other";
pub const LATEST_NOTE: &str = "preview of the focused person";
pub const UNDERSTOOD_AS: &str = "Understood as";
pub const UNDERSTOOD_NOTE: &str = "each part is a chip you can edit";
pub const RESULTS: &str = "Results";

pub fn list_detail(id: &str) -> String {
    format!("mailing list \u{b7} {id}")
}

pub fn files_named(typed: &str) -> String {
    format!("Files named \u{201c}{typed}\u{2026}\u{201d}")
}

pub fn files_detail(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [one, two] => format!("{one}, {two}"),
        [one, two, rest @ ..] => format!("{one}, {two} and {} more", grouped(rest.len() as u64)),
    }
}

pub const FILES_TITLE: &str = "Files whose name or contents match";

/// Says when the indexer is still reading, so a file not found yet is not taken for one that does
/// not exist. iWork documents and text in images are not read.
pub fn files_note(keymap: &Keymap, contents_complete: bool) -> String {
    let here = crate::contacts_view::this_computer(keymap);
    if contents_complete {
        format!("contents are indexed on {here} for PDF, Office documents and text")
    } else {
        format!("still reading the contents of files on {here}: some may not be found yet")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilePreview {
    Sheet,
    Page,
    Slides,
    Image,
    Text,
}

pub fn file_kind(name: &str, mime_type: &str) -> String {
    let extension = std::path::Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| !extension.is_empty() && extension.len() <= 7);
    if let Some(extension) = extension {
        return match extension.to_ascii_lowercase().as_str() {
            "numbers" => "NUM".to_owned(),
            "jpeg" => "JPG".to_owned(),
            other => other.to_ascii_uppercase(),
        };
    }
    let subtype = mime_type
        .split(';')
        .next()
        .unwrap_or_default()
        .rsplit('/')
        .next()
        .unwrap_or_default();
    match subtype {
        "pdf" => "PDF",
        "plain" => "TXT",
        "csv" => "CSV",
        "jpeg" => "JPG",
        "png" => "PNG",
        _ => "FILE",
    }
    .to_owned()
}

pub fn file_preview(name: &str, mime_type: &str) -> FilePreview {
    match file_kind(name, mime_type).as_str() {
        "XLSX" | "XLS" | "CSV" | "NUM" | "ODS" => FilePreview::Sheet,
        "PPTX" | "PPT" | "KEY" | "ODP" => FilePreview::Slides,
        "JPG" | "PNG" | "GIF" | "HEIC" | "WEBP" | "TIFF" => FilePreview::Image,
        "TXT" | "MD" | "LOG" => FilePreview::Text,
        _ => FilePreview::Page,
    }
}

pub fn file_meta(sender: &str, date: &str, size: u64) -> String {
    [sender, date, &crate::format::human_size(size)]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" \u{b7} ")
}

pub fn file_subject(subject: &str) -> String {
    format!("in \u{2018}{subject}\u{2019}")
}

pub fn file_accessible(name: &str, kind: &str, meta: &str, line: &str, subject: &str) -> String {
    [name, kind, meta, line, subject]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Nothing is fetched to preview or save a file whose bytes are not here.
pub fn file_not_here(keymap: &Keymap) -> String {
    let here = crate::contacts_view::this_computer(keymap);
    format!("That file isn\u{2019}t on {here} yet: open its message to download it")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridArrow {
    Left,
    Right,
    Up,
    Down,
}

/// `None` when it goes nowhere; with no ring yet, the first card. Down onto a shorter row lands on
/// its last card, as Finder does.
pub fn grid_step(ring: Option<u64>, arrow: GridArrow, count: u64, columns: u64) -> Option<u64> {
    if count == 0 {
        return None;
    }
    let columns = columns.max(1);
    let Some(ring) = ring else {
        return Some(0);
    };
    match arrow {
        GridArrow::Left => ring.checked_sub(1),
        GridArrow::Right => (ring + 1 < count).then_some(ring + 1),
        GridArrow::Up => ring.checked_sub(columns),
        GridArrow::Down => {
            if ring + columns < count {
                return Some(ring + columns);
            }
            let last = count - 1;
            (last / columns > ring / columns).then_some(last)
        }
    }
}

const RESULTS_TABS: [CommandId; 3] = [
    CommandId::ResultsConversations,
    CommandId::ResultsFiles,
    CommandId::ResultsPeople,
];

/// Space hands a copy of the file to the system's viewer: Quick Look on the Mac, the desktop's app
/// elsewhere (#2183).
pub fn files_hints(keymap: &Keymap) -> Vec<Hint> {
    let mut out = vec![hints::fixed(
        "Up/Down/Left/Right",
        "move",
        "the arrows move the ring across the grid's columns and rows, which \
         the collection view lays out; j and k step it as the list's do",
    )];
    let look = if keymap.platform() == Some(postio_config::paths::Platform::Apple) {
        QUICK_LOOK
    } else {
        "open the file"
    };
    out.extend(hints::hint(keymap, CommandId::QuickLook, look));
    out.extend(hints::hint(
        keymap,
        CommandId::OpenMessage,
        "open the message",
    ));
    out.extend(hints::hint(keymap, CommandId::SaveFile, "save file"));
    out.extend(hints::run(keymap, &RESULTS_TABS, "switch tab"));
    out
}

pub fn person_messages(messages: u64) -> String {
    format!(
        "{} {}",
        grouped(messages),
        if messages == 1 { "message" } else { "messages" }
    )
}

pub fn person_accessible(name: &str, address: &str, messages: &str, last: &str) -> String {
    let mut said: Vec<String> = [name, address, messages]
        .into_iter()
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect();
    if !last.is_empty() {
        said.push(format!("last {last}"));
    }
    said.join(", ")
}

pub fn people_hints(keymap: &Keymap) -> Vec<Hint> {
    let mut out = Vec::new();
    out.extend(hints::pair(
        keymap,
        CommandId::NextMessage,
        CommandId::PrevMessage,
        "move",
    ));
    out.extend(hints::hint(
        keymap,
        CommandId::OpenMessage,
        "search their mail",
    ));
    out.extend(hints::run(keymap, &RESULTS_TABS, "switch tab"));
    out
}

pub fn matching(noun: &str, typed: &str) -> String {
    if typed.is_empty() {
        noun.to_owned()
    } else {
        format!("{noun} matching \u{201c}{typed}\u{201d}")
    }
}

pub fn last_when<Tz: TimeZone>(at: DateTime<Tz>, now: DateTime<Tz>) -> String {
    let (day, today) = (at.date_naive(), now.date_naive());
    match (today - day).num_days() {
        ..=0 => "today".to_owned(),
        1 => "yesterday".to_owned(),
        2..=60 => day.format("%-d %b").to_string(),
        _ if day.year() == today.year() => day.format("%B").to_string(),
        _ => day.format("%B %Y").to_string(),
    }
}

pub fn person_detail<Tz: TimeZone>(
    address: &str,
    messages: u64,
    last: Option<DateTime<Tz>>,
    now: DateTime<Tz>,
) -> String {
    let mut line = format!(
        "{address} \u{b7} {} {}",
        grouped(messages),
        if messages == 1 { "message" } else { "messages" }
    );
    if let Some(last) = last {
        line.push_str(&format!(" \u{b7} last {}", last_when(last, now)));
    }
    line
}

pub fn latest_from(name: &str) -> String {
    format!("Latest from {name}")
}

pub fn operator_source(keyword: &str) -> &'static str {
    match keyword {
        "label" => "Labels, by how many threads carry them",
        "in" => "Folders mail is filed in",
        _ => "Contacts and everyone you have mail with",
    }
}

pub fn origin_line(words: &str) -> String {
    format!("from \u{2018}{words}\u{2019}")
}

pub fn parsed_footer(keymap: &Keymap, total: u64, elapsed: Duration, capped: bool) -> String {
    format!(
        "parsed on {} \u{b7} {}",
        crate::contacts_view::this_computer(keymap),
        footer_count(total, elapsed, capped)
    )
}

pub fn plain_results_note(
    after: Option<chrono::NaiveDate>,
    before: Option<chrono::NaiveDate>,
) -> String {
    let month = after.zip(before).and_then(|(after, before)| {
        let last = before.pred_opt()?;
        ((after.year(), after.month()) == (last.year(), last.month()) && after.day() == 1)
            .then(|| after.format("%B %Y").to_string())
    });
    match month {
        Some(month) => format!("{month} \u{b7} {NEWEST_FIRST}"),
        None => NEWEST_FIRST.to_owned(),
    }
}

pub fn prefix_hints(keymap: &Keymap) -> Vec<Hint> {
    let mut hints = vec![
        hints::fixed(
            "Tab",
            "complete",
            "Tab in the field is the toolkit's key; the bar takes it for the ghost",
        ),
        arrows("move"),
        hints::fixed(
            "Return",
            "open",
            "Return runs the highlighted row: the field's own key, not a command",
        ),
    ];
    hints.extend(hints::hint(
        keymap,
        CommandId::ShowAllResults,
        "all results",
    ));
    hints
}

pub fn ghost_hint() -> Hint {
    hints::fixed(
        "Tab",
        "completes",
        "Tab in the field is the toolkit's key; the bar takes it for the ghost",
    )
}

pub fn adds_hint() -> Hint {
    hints::fixed(
        "Return",
        "adds",
        "Return runs the highlighted row: the field's own key, not a command",
    )
}

pub fn operator_hints(keymap: &Keymap, keyword: &str) -> Vec<Hint> {
    let mut hints = vec![hints::fixed(
        "Return",
        "add as chip",
        "Return runs the highlighted row: the field's own key, not a command",
    )];
    hints.extend(hints::hint(
        keymap,
        CommandId::ExcludeSuggestion,
        &format!("exclude (-{keyword}:)"),
    ));
    hints.push(arrows("choose"));
    hints.push(hints::fixed(
        "BackSpace",
        "back to words",
        "Backspace on an empty value is the field's own key; the bar reads the text",
    ));
    hints
}

pub fn nobody_matches(noun: &str) -> &'static str {
    match noun {
        "Labels" => "No label matches",
        "Folders" => "No folder matches",
        _ => "No one matches",
    }
}

pub fn operator_hints_for_nobody() -> Vec<Hint> {
    vec![hints::fixed(
        "BackSpace",
        "back to words",
        "Backspace on an empty value is the field's own key; the bar reads the text",
    )]
}

pub fn plain_hints(keymap: &Keymap) -> Vec<Hint> {
    let mut hints = vec![
        arrows("move"),
        hints::fixed(
            "Return",
            "open",
            "Return runs the highlighted row: the field's own key, not a command",
        ),
        hints::fixed(
            "Tab",
            "edit as chips",
            "Tab in the field is the toolkit's key; the bar takes it for the chips",
        ),
    ];
    hints.extend(hints::hint(keymap, CommandId::BackToWords, "keep as words"));
    hints
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultChild {
    Match {
        message: postio_model::MessageId,
        number: u32,
        tag: String,
        /// `None` while no passage is cut.
        passage: Option<postio_search::results::Passage>,
        sender: String,
        initials: String,
        time: String,
    },
    Gap {
        from: u32,
        to: u32,
        senders: String,
        day: String,
    },
}

/// One Gap for each run of non-matching messages (before the first, between, after the last), so
/// every message is said once (#2178).
pub fn children(
    view: &postio_model::ThreadView,
    matches: &[postio_search::results::ConversationMatch],
    now: DateTime<chrono::Local>,
) -> Vec<ResultChild> {
    // A subject match is the thread's; a message the view does not hold is not shown.
    let mut found: Vec<(usize, &postio_search::results::ConversationMatch)> = Vec::new();
    for each in matches {
        let Some(message) = each.message else {
            continue;
        };
        let Some(at) = view.members.iter().position(|member| member.id == message) else {
            continue;
        };
        if !found.iter().any(|(seen, _)| *seen == at) {
            found.push((at, each));
        }
    }
    found.sort_by_key(|(at, _)| *at);
    if found.is_empty() {
        return Vec::new();
    }
    let gap = |start: usize, end: usize| {
        let run = &view.members[start..end];
        let mut senders: Vec<String> = Vec::new();
        for member in run {
            let who = match (&member.from, member.own) {
                (_, true) => "You".to_owned(),
                (Some(from), false) => crate::focus_row::first_name(from).to_owned(),
                (None, false) => "?".to_owned(),
            };
            if !senders.contains(&who) {
                senders.push(who);
            }
        }
        ResultChild::Gap {
            from: (start + 1) as u32,
            to: end as u32,
            senders: senders.join(", "),
            day: crate::row::timestamp(run[0].at, now),
        }
    };
    let mut out = Vec::new();
    let mut next = 0;
    for (at, each) in found {
        if at > next {
            out.push(gap(next, at));
        }
        next = at + 1;
        let member = &view.members[at];
        out.push(ResultChild::Match {
            message: member.id,
            number: (at + 1) as u32,
            tag: match_tag(at + 1, &each.found.source),
            passage: child_passage(&each.found),
            sender: match (&member.from, member.own) {
                (_, true) => "You".to_owned(),
                (Some(from), false) => from.display().to_owned(),
                (None, false) => "Unknown sender".to_owned(),
            },
            initials: crate::row::initials(member.from.as_ref()),
            time: crate::row::timestamp(member.at, now),
        });
    }
    if view.members.len() > next {
        out.push(gap(next, view.members.len()));
    }
    out
}

fn match_tag(number: usize, source: &Source) -> String {
    match source {
        Source::FileName { .. } | Source::FileContent { .. } => {
            format!("message {number} \u{b7} file")
        }
        Source::Quoted => format!("message {number} \u{b7} quoted"),
        Source::Subject | Source::Body => format!("message {number}"),
    }
}

fn child_passage(found: &postio_search::results::Match) -> Option<postio_search::results::Passage> {
    let Source::FileContent {
        name, location: at, ..
    } = &found.source
    else {
        return found.passage.clone().or_else(|| match &found.source {
            Source::FileName { name, .. } => Some(postio_search::results::Passage {
                text: name.clone(),
                ranges: Vec::new(),
                elided_start: false,
                elided_end: false,
            }),
            _ => None,
        });
    };
    let lead = format!("{name} \u{b7} {}: ", location(at));
    let cut = found.passage.clone()?;
    let shift = lead.len();
    Some(postio_search::results::Passage {
        text: format!("{lead}{}", cut.text),
        ranges: cut
            .ranges
            .iter()
            .map(|range| range.start + shift..range.end + shift)
            .collect(),
        elided_start: false,
        elided_end: cut.elided_end,
    })
}

pub fn gap_line(child: &ResultChild) -> String {
    let ResultChild::Gap {
        from,
        to,
        senders,
        day,
    } = child
    else {
        return String::new();
    };
    if from == to {
        format!("Message {from} doesn't match \u{b7} {senders}, {day}")
    } else {
        format!("Messages {from}\u{2013}{to} don't match \u{b7} {senders}, {day}")
    }
}

pub fn match_summary(matched: usize, total: usize) -> String {
    format!("{matched} of {total} messages match")
}

pub fn single_match_snippet(number: usize, total: usize, who: &str, words: &str) -> String {
    if words.trim().is_empty() {
        return format!("Message {number} of {total} \u{b7} {who}");
    }
    format!("Message {number} of {total} \u{b7} {who}: {words}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeDelta, Utc};

    fn at(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 15, 30, 0).unwrap()
    }

    #[test]
    fn quick_look_s_buttons_run_what_they_say_with_a_key_unbound() {
        // Space given to another command leaves Quick Look no key: its button is dropped and later
        // ones must not shift onto the wrong command.
        let mut keys = postio_config::KeyBindings::default();
        keys.overrides_mut()
            .insert("next_match".to_owned(), "space".to_owned());
        let keymap =
            postio_core::Keymap::resolve_on(&keys, postio_config::paths::Platform::Freedesktop);
        assert_eq!(keymap.binding(CommandId::QuickLook), None, "space is taken");
        let labels: Vec<String> = quick_look_actions(&keymap)
            .into_iter()
            .map(|hint| hint.label)
            .collect();
        assert_eq!(labels, ["Open", "Archive"]);
        assert_eq!(
            quick_look_commands(&keymap),
            [CommandId::OpenMessage, CommandId::Archive]
        );
    }

    #[test]
    fn a_toggle_button_names_its_term_and_a_popover_button_none() {
        let terms: Vec<_> = FilterKind::ALL
            .into_iter()
            .map(|kind| (kind, kind.toggle_term()))
            .collect();
        assert_eq!(
            terms,
            [
                (FilterKind::From, None),
                (FilterKind::To, None),
                (FilterKind::Date, None),
                (FilterKind::Anywhere, None),
                (FilterKind::Label, None),
                (FilterKind::Attachment, Some(("has", "attachment"))),
                (FilterKind::HasAction, Some(("has", "action"))),
                (FilterKind::Unread, Some(("is", "unread"))),
            ]
        );
    }

    #[test]
    fn a_people_row_says_its_messages_and_the_tab_its_keys() {
        assert_eq!(person_messages(1), "1 message");
        assert_eq!(person_messages(4_512), "4,512 messages");
        assert_eq!(
            person_accessible("Ada Moreno", "ada@example.com", "3 messages", "26 Sep"),
            "Ada Moreno, ada@example.com, 3 messages, last 26 Sep"
        );
        let keymap = postio_core::Keymap::resolve_on(
            &postio_config::KeyBindings::default(),
            postio_config::paths::Platform::Apple,
        );
        let hints = people_hints(&keymap);
        let said: Vec<&str> = hints.iter().map(|hint| hint.label.as_str()).collect();
        assert_eq!(said, ["move", "search their mail", "switch tab"]);
        assert_eq!(hints[2].key, "cmd+1\u{2013}3");
        assert_eq!(
            files_hints(&keymap).last().map(|hint| hint.key.as_str()),
            Some("cmd+1\u{2013}3")
        );
    }

    #[test]
    fn a_suggestion_count_is_grouped_and_a_capped_one_is_a_floor() {
        assert_eq!(suggestion_count(1_000, true), "1,000+");
        assert_eq!(suggestion_count(12, false), "12");
        assert_eq!(suggestion_count(4_512, false), "4,512");
        assert_eq!(suggestion_count(0, false), "0");
    }

    #[test]
    fn the_cheat_sheet_is_the_designs_eight_entries() {
        assert_eq!(
            cheat_sheet(),
            [
                ("from:", "person or address"),
                ("to:", "recipient"),
                ("subject:", "words in subject"),
                ("in:", "folder"),
                ("label:", "label name"),
                ("has:attachment", ""),
                ("after: before:", "dates"),
                ("-word", "exclude"),
            ]
        );
    }

    #[test]
    fn a_range_of_months_reads_as_the_timeline_and_the_date_popover_say_it() {
        let day = |y, m, d| chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert_eq!(
            month_range(day(2026, 7, 1), day(2026, 9, 30), false),
            "Jul \u{2013} Sep"
        );
        assert_eq!(
            month_range(day(2026, 9, 30), day(2026, 7, 1), true),
            "Jul \u{2013} Sep 2026"
        );
        assert_eq!(
            month_range(day(2026, 9, 1), day(2026, 9, 26), true),
            "Sep 2026"
        );
        assert_eq!(
            month_range(day(2025, 12, 1), day(2026, 2, 28), false),
            "Dec 2025 \u{2013} Feb 2026"
        );
        assert_eq!(date_result(12, 21, false), "12 of 21");
        assert_eq!(date_parsed("after:2026-07-01"), "\u{2192} after:2026-07-01");
    }

    #[test]
    fn the_date_presets_are_the_designs_with_the_dates_their_words_lower_to() {
        let day = |y, m, d| chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap();
        let presets = date_presets(day(2026, 9, 26));
        assert_eq!(
            presets.map(|preset| (preset.label, preset.start)),
            [
                ("Any time", None),
                ("Last 7 days", Some(day(2026, 9, 19))),
                ("Last 30 days", Some(day(2026, 8, 27))),
                ("This quarter", Some(day(2026, 7, 1))),
                ("This year", Some(day(2026, 1, 1))),
            ]
        );
        assert_eq!(presets.map(|preset| preset.count_at), [4, 0, 1, 2, 3]);
    }

    #[test]
    fn a_recent_search_says_when_it_ran_the_way_the_design_does() {
        let now = at(2026, 9, 30);
        assert_eq!(recent_when(now, now), "today");
        assert_eq!(recent_when(now - TimeDelta::days(1), now), "yesterday");
        assert_eq!(recent_when(at(2026, 9, 28), now), "Mon");
        assert_eq!(recent_when(now - TimeDelta::days(6), now), "Thu");
        assert_eq!(recent_when(at(2026, 9, 21), now), "21 Sep");
        assert_eq!(recent_when(at(2026, 9, 23), now), "23 Sep");
        assert_eq!(recent_when(at(2025, 12, 3), now), "3 Dec 2025");
    }

    #[test]
    fn a_recent_search_that_ran_in_the_future_is_today() {
        let now = at(2026, 9, 30);
        assert_eq!(recent_when(now + TimeDelta::hours(30), now), "today");
    }

    #[test]
    fn the_footer_counts_matches_and_says_how_long_it_took() {
        let took = |ms| Duration::from_millis(ms);
        assert_eq!(footer_count(48, took(38), false), "48 matches · 38 ms");
        assert_eq!(footer_count(1, took(4), false), "1 match · 4 ms");
        assert_eq!(footer_count(0, took(2), false), "0 matches · 2 ms");
        assert_eq!(footer_count(214, took(12), false), "214 matches · 12 ms");
        assert_eq!(
            footer_count(10_000, took(41), true),
            "10,000+ matches · 41 ms"
        );
        assert_eq!(
            footer_count(18_204, took(9), false),
            "18,204 matches · 9 ms"
        );
    }

    #[test]
    fn a_search_that_took_under_a_millisecond_says_so_without_a_zero() {
        assert_eq!(
            footer_count(3, Duration::from_micros(400), false),
            "3 matches · <1 ms"
        );
    }

    #[test]
    fn a_narrow_to_pill_says_its_operator_its_value_and_its_count() {
        let pill = narrow_pill("from:", "Tomás Reyes", 9, false);
        assert_eq!(
            (pill.op.as_str(), pill.value.as_str(), pill.count.as_str()),
            ("from:", "Tomás Reyes", "9")
        );
        assert_eq!(pill.label(), "from: Tomás Reyes 9");

        let attachment = narrow_pill("has:", "attachment", 12, false);
        assert_eq!(attachment.label(), "has: attachment 12");

        let big = narrow_pill("label:", "Atlas", 1_204, false);
        assert_eq!(big.count, "1,204");

        let floor = narrow_pill("label:", "Atlas", 1_204, true);
        assert_eq!(floor.count, "1,204+");
    }

    #[test]
    fn a_recent_search_counts_its_results() {
        assert_eq!(results_count(48, false), "48 results");
        assert_eq!(results_count(1, false), "1 result");
        assert_eq!(results_count(0, false), "0 results");
        assert_eq!(results_count(18_204, false), "18,204 results");
        assert_eq!(results_count(5_000, true), "5,000+ results");
        assert_eq!(results_count(1, true), "1+ results");
    }

    #[test]
    fn a_saved_search_pill_and_its_badge_say_a_capped_count_as_a_floor() {
        assert_eq!(saved_count(39, false), "39");
        assert_eq!(saved_count(1_204, false), "1,204");
        assert_eq!(saved_count(5_000, true), "5,000+");
        assert_eq!(new_badge(3, false), "3 new");
        assert_eq!(new_badge(99, true), "99+ new");
    }

    #[test]
    fn a_top_hit_says_its_day_and_month_and_another_years() {
        let now = at(2026, 9, 26);
        assert_eq!(hit_date(now, now), "26 Sep");
        assert_eq!(hit_date(at(2026, 8, 14), now), "14 Aug");
        assert_eq!(hit_date(at(2025, 12, 3), now), "3 Dec 25");
    }

    #[test]
    fn a_hit_says_where_it_waits_and_in_which_account() {
        assert_eq!(
            hit_place(None, Some("INBOX"), None).as_deref(),
            Some("in:Inbox")
        );
        assert_eq!(
            hit_place(Some(("Newsletters", false)), Some("Inbox"), None).as_deref(),
            Some("held \u{b7} Newsletters"),
            "where it waits, in place of its folder"
        );
        assert_eq!(
            hit_place(Some(("Newsletters", true)), Some("Inbox"), None).as_deref(),
            Some("digest \u{b7} Newsletters")
        );
        assert_eq!(
            hit_place(None, Some("Inbox"), Some("second@example.org")).as_deref(),
            Some("in:Inbox \u{b7} second@example.org")
        );
        assert_eq!(
            hit_place(None, None, Some("second@example.org")).as_deref(),
            Some("second@example.org")
        );
        assert_eq!(hit_place(None, None, None), None);
    }

    #[test]
    fn a_hits_folder_is_written_as_its_operator() {
        assert_eq!(in_folder("Inbox"), "in:Inbox");
        assert_eq!(in_folder("Q3 close"), "in:\"Q3 close\"");
        assert_eq!(in_folder("INBOX"), "in:Inbox");
        assert_eq!(folder_name("inbox"), "Inbox");
        assert_eq!(folder_name("Receipts"), "Receipts");
    }

    #[test]
    fn the_empty_dropdowns_footer_names_its_keys_from_the_keymap() {
        let keymap = postio_core::Keymap::resolve_on(
            &postio_config::KeyBindings::default(),
            postio_config::paths::Platform::Apple,
        );
        let hints = empty_hints(&keymap, true);
        let said: Vec<(&str, &str)> = hints
            .iter()
            .map(|hint| (hint.key.as_str(), hint.label.as_str()))
            .collect();
        assert_eq!(
            said,
            [
                ("Up Down", "move"),
                ("Return", "run again"),
                ("alt+1\u{2013}4", "saved"),
                (">", "commands"),
            ]
        );
        assert!(
            !empty_hints(&keymap, false)
                .iter()
                .any(|hint| hint.label == "run again")
        );
    }

    #[test]
    fn the_empty_dropdowns_footer_says_how_many_messages_the_index_holds() {
        assert_eq!(local_index(18_204), "local index \u{b7} 18,204 messages");
        assert_eq!(local_index(1), "local index \u{b7} 1 message");
        assert_eq!(local_index(0), "local index \u{b7} 0 messages");
    }

    #[test]
    fn the_field_teaches_tab_beside_the_ghost_and_a_chosen_row_says_return_adds() {
        let tab = ghost_hint();
        assert_eq!((tab.key.as_str(), tab.label.as_str()), ("Tab", "completes"));
        let adds = adds_hint();
        assert_eq!((adds.key.as_str(), adds.label.as_str()), ("Return", "adds"));
    }

    #[test]
    fn the_field_at_rest_says_less_than_the_field_opened() {
        assert_eq!(RESTING_PLACEHOLDER, "Search mail or run a command");
        assert_eq!(
            PLACEHOLDER,
            "Search mail, people and files, or type > for commands"
        );
    }

    #[test]
    fn the_words_dropdowns_footer_names_its_keys_from_the_keymap() {
        let keymap = postio_core::Keymap::resolve_on(
            &postio_config::KeyBindings::default(),
            postio_config::paths::Platform::Apple,
        );
        let said: Vec<(String, String)> = words_hints(&keymap)
            .into_iter()
            .map(|hint| (hint.key, hint.label))
            .collect();
        let show_all = keymap
            .binding(postio_core::CommandId::ShowAllResults)
            .expect("bound on the Mac")
            .to_owned();
        assert_eq!(
            said,
            [
                ("Up Down".to_owned(), "move".to_owned()),
                ("Return".to_owned(), "open".to_owned()),
                (show_all, "all results".to_owned()),
                ("Tab".to_owned(), "add first filter".to_owned()),
            ]
        );
    }

    #[test]
    fn the_show_all_row_names_how_many_results_it_opens() {
        assert_eq!(show_all(48, false, None), "Show all 48 results");
        assert_eq!(
            show_all(214, false, Some("at")),
            "Show all 214 results for \u{201c}at\u{201d}"
        );
        assert_eq!(show_all(1, false, None), "Show 1 result");
        assert_eq!(show_all(10_000, true, None), "Show all 10,000+ results");
    }

    use postio_search::results::{Location, RankReason, Source};

    #[test]
    fn a_top_hit_says_why_it_ranked_in_at_most_two_parts() {
        use RankReason::*;
        assert_eq!(
            reason_line(&[Replied, Matches(3)]),
            "you replied · 3 matches"
        );
        assert_eq!(
            reason_line(&[Flagged, Matches(2)]),
            "you flagged · 2 matches"
        );
        assert_eq!(
            reason_line(&[FrequentSender, InFileName, Matches(1)]),
            "frequent sender · file name"
        );
        assert_eq!(
            reason_line(&[FrequentSender, InFileContent, Matches(1)]),
            "frequent sender · file contents"
        );
        assert_eq!(
            reason_line(&[Replied, FrequentSender, InSubject, Matches(4)]),
            "you replied · 4 matches",
            "two parts at most: the strongest reason, then the matches"
        );
        assert_eq!(
            reason_line(&[Replied, FrequentSender, InSubject, Matches(1)]),
            "you replied · frequent sender",
            "one match is not worth saying, so the next reason has the place"
        );
        assert_eq!(reason_line(&[Matches(5)]), "5 matches");
        assert_eq!(reason_line(&[InSubject]), "subject");
        assert_eq!(reason_line(&[Matches(1)]), "");
        assert_eq!(reason_line(&[]), "");
    }

    #[test]
    fn a_source_tag_names_where_the_words_matched() {
        assert_eq!(source_tag(&Source::Body), "body");
        assert_eq!(source_tag(&Source::Quoted), "quoted text");
        assert_eq!(source_tag(&Source::Subject), "subject");
        let attachment = postio_model::AttachmentId::new(7);
        assert_eq!(
            source_tag(&Source::FileName {
                attachment,
                name: "Atlas-budget-template.xlsx".into()
            }),
            "Atlas-budget-template.xlsx"
        );
        assert_eq!(
            source_tag(&Source::FileContent {
                attachment,
                name: "Atlas-Sep-actuals.pdf".into(),
                location: Location::Page(2),
            }),
            "Atlas-Sep-actuals.pdf"
        );
        assert_eq!(
            sources_tag(&[
                Source::Body,
                Source::FileName {
                    attachment,
                    name: "Atlas-Q3-budget.xlsx".into()
                },
                Source::Subject,
            ]),
            "body + Atlas-Q3-budget.xlsx"
        );
        assert_eq!(sources_tag(&[Source::Body, Source::Body]), "body");
        assert_eq!(sources_tag(&[]), "");
        assert_eq!(
            sources_tag(&[Source::Body, Source::Subject, Source::Body]),
            "body"
        );
        assert_eq!(
            sources_tag(&[Source::Quoted, Source::Subject, Source::Quoted]),
            "quoted text"
        );
        assert_eq!(
            sources_tag(&[
                Source::FileName {
                    attachment,
                    name: "Atlas-budget-template.xlsx".into()
                },
                Source::Body,
            ]),
            "Atlas-budget-template.xlsx"
        );
        assert_eq!(sources_tag(&[Source::Subject]), "subject");
    }

    #[test]
    fn a_row_shows_the_first_match_with_a_passage_and_never_tags_the_subject_over_one() {
        use postio_search::results::Match;
        let found = |source: Source, cut: bool| Match {
            source,
            passage: cut.then(postio_search::passage::Passage::default),
            when: None,
        };
        let shown = |matches: &[Match]| shown_match(matches).map(|each| each.source.clone());
        assert_eq!(
            shown(&[found(Source::Subject, false), found(Source::Body, true)]),
            Some(Source::Body)
        );
        assert_eq!(
            shown(&[found(Source::Subject, false), found(Source::Body, false)]),
            Some(Source::Body)
        );
        assert_eq!(
            shown(&[found(Source::Subject, false)]),
            Some(Source::Subject)
        );
        assert_eq!(shown(&[]), None);
    }

    #[test]
    fn a_passage_inside_a_file_says_where() {
        assert_eq!(location(&Location::Page(2)), "Page 2");
        assert_eq!(
            location(&Location::Sheet {
                name: "Summary".into(),
                row: 14
            }),
            "Sheet \u{2018}Summary\u{2019}, row 14"
        );
        assert_eq!(location(&Location::Slide(3)), "Slide 3");
    }

    #[test]
    fn a_month_group_is_titled_with_its_month_and_counted() {
        let september = chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        assert_eq!(month_title(september), "September 2026");
        assert_eq!(group_line("Top hits", 3), "Top hits · 3");
        assert_eq!(month_group(september, 9), "September 2026 · 9");
        assert_eq!(
            month_group(chrono::NaiveDate::from_ymd_opt(2025, 12, 1).unwrap(), 1_204),
            "December 2025 · 1,204"
        );
    }

    #[test]
    fn search_says_thread_never_conversation() {
        assert_eq!(TABS, ["Threads", "Files", "People"]);
        assert_eq!(
            operator_source("label"),
            "Labels, by how many threads carry them"
        );
    }

    #[test]
    fn the_timeline_counts_threads_and_says_what_else_matched() {
        assert_eq!(count_line(48, false), "48 threads");
        assert_eq!(count_line(1, false), "1 thread");
        assert_eq!(count_line(10_000, true), "10,000+ threads");
        assert_eq!(
            sub_line(12, 6, false),
            "12 files · 6 people · last 12 months"
        );
        assert_eq!(sub_line(1, 1, false), "1 file · 1 person · last 12 months");
        assert_eq!(sub_line(0, 2, false), "0 files · 2 people · last 12 months");
    }

    #[test]
    fn a_capped_answer_draws_every_count_as_a_floor() {
        assert_eq!(count_line(5_000, true), "5,000+ threads");
        assert_eq!(tab_count(5_000, true), "5,000+");
        assert_eq!(facet_count(1_204, true), "1,204+");
        assert_eq!(facet_count(24, false), "24");
        assert_eq!(
            sub_line(1, 1, true),
            "1+ files · 1+ people · last 12 months"
        );
        assert_eq!(
            sub_line(1_204, 6, true),
            "1,204+ files · 6+ people · last 12 months"
        );
    }

    #[test]
    fn the_results_footer_says_how_many_where_and_how_long() {
        let took = |ms| Duration::from_millis(ms);
        assert_eq!(
            results_footer(48, false, took(41)),
            "48 threads · local index · 41 ms"
        );
        assert_eq!(
            results_footer(10_000, true, took(41)),
            "10,000+ threads · local index · 41 ms"
        );
        assert_eq!(
            results_footer(1, false, Duration::from_micros(300)),
            "1 thread · local index · <1 ms"
        );
    }

    #[test]
    fn a_narrowed_sub_line_says_the_filters_then_the_files() {
        use postio_search::query::{Filter, State};
        let names = |address: &str| (address == "ada@example.com").then(|| "Ada Moreno".to_owned());
        let date = |y, m, d| chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap();
        let today = date(2026, 9, 30);
        let line = |filters: &[Filter], files, people| {
            narrowed_sub_line(filters, &names, today, files, people, false)
        };
        assert_eq!(
            narrowed_sub_line(
                &[Filter::Label("Atlas".into())],
                &names,
                today,
                1,
                1_204,
                true
            ),
            "labelled Atlas · 1+ files · 1,204+ people",
            "past the cap, a floor (D30)"
        );

        assert_eq!(line(&[], 12, 6), "12 files · 6 people · last 12 months");
        assert_eq!(
            line(
                &[
                    Filter::From("ada@example.com".into()),
                    Filter::After(date(2026, 7, 1))
                ],
                9,
                1
            ),
            "from Ada since July · 9 files"
        );
        assert_eq!(
            line(
                &[Filter::AnyOf(
                    postio_search::query::AnyOf::new(vec![
                        Filter::From("ada@example.com".into()),
                        Filter::From("tomas@example.com".into()),
                    ])
                    .unwrap()
                )],
                3,
                2
            ),
            "from Ada or tomas@example.com · 3 files"
        );
        assert_eq!(
            line(&[Filter::HasAttachment, Filter::Is(State::Unread)], 1, 4),
            "unread with attachments · 1 file · 4 people"
        );
        assert_eq!(
            line(
                &[
                    Filter::In("Archive".into()),
                    Filter::Label("Atlas".into()),
                    Filter::Before(date(2026, 3, 1))
                ],
                0,
                1
            ),
            "in Archive labelled Atlas before March · 0 files · 1 person"
        );
    }

    #[test]
    fn a_filter_button_reads_as_its_value_when_applied() {
        use postio_search::query::{Filter, State};
        let names = |address: &str| (address == "ada@example.com").then(|| "Ada Moreno".to_owned());
        let date = |y, m, d| chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap();
        let today = date(2026, 9, 30);
        let label = |kind, filters: &[Filter]| filter_button_label(kind, filters, &names, today);

        assert_eq!(label(FilterKind::From, &[]), "From");
        assert_eq!(
            label(FilterKind::From, &[Filter::From("ada@example.com".into())]),
            "From: Ada Moreno"
        );
        assert_eq!(
            label(FilterKind::From, &[Filter::From("ada".into())]),
            "From: ada",
            "a name nobody is known by reads as typed"
        );
        assert_eq!(
            label(
                FilterKind::From,
                &[
                    Filter::From("ada@example.com".into()),
                    Filter::From("tomas@example.com".into())
                ]
            ),
            "From: Ada Moreno +1"
        );
        assert_eq!(
            label(FilterKind::To, &[Filter::To("ada@example.com".into())]),
            "To: Ada Moreno"
        );
        assert_eq!(label(FilterKind::Date, &[]), "Date");
        assert_eq!(
            label(FilterKind::Date, &[Filter::After(date(2026, 7, 1))]),
            "Since July"
        );
        assert_eq!(
            label(FilterKind::Date, &[Filter::After(date(2026, 7, 14))]),
            "Since 14 Jul"
        );
        assert_eq!(
            label(FilterKind::Date, &[Filter::After(date(2025, 7, 1))]),
            "Since July 2025"
        );
        assert_eq!(
            label(FilterKind::Date, &[Filter::Before(date(2026, 3, 1))]),
            "Before March"
        );
        assert_eq!(
            label(
                FilterKind::Date,
                &[
                    Filter::After(date(2026, 7, 1)),
                    Filter::Before(date(2026, 10, 1))
                ]
            ),
            "July \u{2013} September"
        );
        assert_eq!(
            label(
                FilterKind::Date,
                &[
                    Filter::After(date(2026, 8, 1)),
                    Filter::Before(date(2026, 9, 1))
                ]
            ),
            "August"
        );
        assert_eq!(label(FilterKind::Anywhere, &[]), "Anywhere");
        assert_eq!(
            label(FilterKind::Anywhere, &[Filter::In("Archive".into())]),
            "In: Archive"
        );
        assert_eq!(
            label(FilterKind::Anywhere, &[Filter::In("INBOX".into())]),
            "In: Inbox",
            "IMAP's INBOX is the inbox, as every search surface calls it"
        );
        assert_eq!(
            label(FilterKind::Label, &[Filter::Label("Atlas".into())]),
            "Label: Atlas"
        );
        assert_eq!(
            label(FilterKind::Attachment, &[Filter::HasAttachment]),
            "Attachment"
        );
        assert_eq!(label(FilterKind::HasAction, &[]), "Has action");
        assert_eq!(
            label(FilterKind::Unread, &[Filter::Is(State::Unread)]),
            "Unread"
        );
    }

    #[test]
    fn a_set_is_held_by_its_fields_button_and_reads_as_its_first_value() {
        use postio_search::query::Filter;
        let names = |address: &str| (address == "ada@example.com").then(|| "Ada Moreno".to_owned());
        let today = chrono::NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        let set = |members| Filter::any_of(members).expect("a set");
        let people = set(vec![
            Filter::From("ada@example.com".into()),
            Filter::From("tomas@example.com".into()),
        ]);
        assert!(FilterKind::From.holds(&people));
        assert!(!FilterKind::To.holds(&people));
        assert_eq!(
            filter_button_label(
                FilterKind::From,
                std::slice::from_ref(&people),
                &names,
                today
            ),
            "From: Ada Moreno +1"
        );
        let folders = set(vec![
            Filter::In("Inbox".into()),
            Filter::In("Archive".into()),
            Filter::In("Receipts".into()),
        ]);
        assert!(FilterKind::Anywhere.holds(&folders));
        assert_eq!(
            filter_button_label(FilterKind::Anywhere, &[folders], &names, today),
            "In: Inbox +2"
        );
    }

    #[test]
    fn the_filter_bar_has_the_designs_buttons_in_its_order() {
        let labels: Vec<&str> = FilterKind::ALL.iter().map(|kind| kind.title()).collect();
        assert_eq!(
            labels,
            [
                "From",
                "To",
                "Date",
                "Anywhere",
                "Label",
                "Attachment",
                "Has action",
                "Unread"
            ]
        );
        assert!(FilterKind::From.has_popover() && !FilterKind::Unread.has_popover());
    }

    #[test]
    fn the_dropdown_and_its_neighbours_are_named_for_a_screen_reader() {
        assert_eq!(A11Y_QUERY, "Search query");
        assert_eq!(A11Y_SUGGESTIONS, "Search suggestions");
        assert_eq!(A11Y_RESULTS, "Search results");
        assert_eq!(A11Y_FILES, "Files");
        assert_eq!(A11Y_PEOPLE, "People");
        assert_eq!(
            dropdown_row_accessible(
                "Atlas Q3 budget",
                "3 results",
                Some("in:Inbox"),
                Some("yesterday")
            ),
            "Atlas Q3 budget, 3 results, in:Inbox, yesterday"
        );
        assert_eq!(
            dropdown_row_accessible("Show all results", "", None, None),
            "Show all results"
        );
    }

    #[test]
    fn a_result_reads_aloud_as_who_what_where_it_matched_and_when() {
        assert_eq!(
            accessible_row(
                "Ada Moreno",
                "Re: Atlas Q3 budget, final numbers",
                "body",
                "…the final Q3 numbers for the Atlas budget…",
                "Inbox",
                "26 Sep"
            ),
            "Ada Moreno, Re: Atlas Q3 budget, final numbers, matched in body: \
             …the final Q3 numbers for the Atlas budget…, Inbox, 26 Sep"
        );
        assert_eq!(
            accessible_row(
                "Priya Nair",
                "Contractor invoices",
                "",
                "",
                "Inbox",
                "25 Sep"
            ),
            "Priya Nair, Contractor invoices, Inbox, 25 Sep"
        );
    }

    #[test]
    fn the_bulk_bar_names_the_lists_verbs_and_their_keys() {
        let keymap = postio_core::Keymap::resolve_on(
            &postio_config::KeyBindings::default(),
            postio_config::paths::Platform::Apple,
        );
        let said: Vec<(String, String)> = bulk_hints(&keymap)
            .into_iter()
            .map(|hint| (hint.key, hint.label))
            .collect();
        let said: Vec<(&str, &str)> = said
            .iter()
            .map(|(key, label)| (key.as_str(), label.as_str()))
            .collect();
        assert_eq!(
            said,
            [
                ("e", "Archive"),
                ("b", "Snooze"),
                ("I", "Mark read"),
                ("l", "Label"),
                ("v", "Move"),
                ("#", "Delete"),
            ]
        );
    }

    #[test]
    fn the_results_footer_names_its_keys_from_the_keymap() {
        let keymap = postio_core::Keymap::resolve_on(
            &postio_config::KeyBindings::default(),
            postio_config::paths::Platform::Apple,
        );
        let said: Vec<(String, String)> = results_hints(&keymap)
            .into_iter()
            .map(|hint| (hint.key, hint.label))
            .collect();
        let labels: Vec<&str> = said.iter().map(|(_, label)| label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "move",
                "Quick Look",
                "open",
                "select",
                "edit query",
                "back to inbox"
            ]
        );
        assert_eq!(said[1].0, "space", "screen 06's footer: Space Quick Look");
        assert_eq!(said[0].0, "j/k");
        assert_eq!(said.last().map(|(key, _)| key.as_str()), Some("Escape"));
    }

    #[test]
    fn the_results_chrome_says_the_designs_words() {
        assert_eq!(BACK_TO_INBOX, "Inbox");
        assert_eq!(SAVE_SEARCH, "Save search");
        assert_eq!(SORT, "Sort");
        assert_eq!(BEST_MATCH, "Best match");
        assert_eq!(NEWEST, "Newest");
        assert_eq!(
            TIMELINE_HINT,
            "Matches by month \u{b7} drag across months to narrow"
        );
        assert_eq!(checked_line(5), "5 selected");
        assert_eq!(checked_line(1_204), "1,204 selected");
    }

    #[test]
    fn quick_look_says_where_each_match_is_and_when() {
        let keymap = postio_core::Keymap::resolve_on(
            &postio_config::KeyBindings::default(),
            postio_config::paths::Platform::Apple,
        );
        assert_eq!(QUICK_LOOK, "Quick Look");
        assert_eq!(quick_look_position(0, 12), "1 of 12");
        assert_eq!(quick_look_position(1_203, 1_204), "1,204 of 1,204");
        assert_eq!(
            quick_look_walk(&keymap).map(|hint| (hint.key, hint.label)),
            Some((
                "j/k".to_owned(),
                "moves through results while it stays open".to_owned()
            ))
        );
        assert_eq!(
            quick_look_actions(&keymap)
                .into_iter()
                .map(|hint| (hint.label, hint.key))
                .collect::<Vec<_>>(),
            [
                ("Open".to_owned(), "Return".to_owned()),
                ("Archive".to_owned(), "e".to_owned()),
                ("Close".to_owned(), "space".to_owned()),
            ]
        );
        assert_eq!(matches_line(4), "4 matches in this thread");
        assert_eq!(matches_line(1), "1 match in this thread");
        assert_eq!(
            matches_hint(&keymap).map(|hint| (hint.key, hint.label)),
            Some(("]/[".to_owned(), "jump between them".to_owned()))
        );

        assert_eq!(match_place(&Source::Body), "Body");
        assert_eq!(match_place(&Source::Quoted), "Earlier reply");
        assert_eq!(match_place(&Source::Subject), "Subject");
        let file = Source::FileContent {
            attachment: postio_model::AttachmentId::new(1),
            name: "Atlas-Q3-budget.xlsx".to_owned(),
            location: Location::Sheet {
                name: "Q3".to_owned(),
                row: 3,
            },
        };
        assert_eq!(match_place(&file), "Atlas-Q3-budget.xlsx");

        let now = at(2026, 9, 26);
        let ada = postio_model::EmailAddress::new(Some("Ada Moreno"), "ada@example.com");
        let bare = postio_model::EmailAddress::new(None::<&str>, "ravi@example.com");
        assert_eq!(
            match_when(&Source::Body, Some(&ada), Some(now), now),
            "Ada \u{b7} 26 Sep"
        );
        assert_eq!(
            match_when(&Source::Body, Some(&bare), Some(now), now),
            "ravi@example.com \u{b7} 26 Sep",
            "no name, the address"
        );
        assert_eq!(match_when(&Source::Quoted, None, None, now), "");
        assert_eq!(
            match_when(&file, Some(&ada), Some(now), now),
            "Sheet \u{2018}Q3\u{2019}, row 3",
            "a file's match says where in it"
        );
        assert_eq!(sent_at(now, now), "Sat 26 Sep, 15:30");
        let last_year = now - chrono::Duration::days(365);
        assert_eq!(sent_at(last_year, now), "Fri 26 Sep 2025, 15:30");
        assert_eq!(thread_size(3), Some("thread of 3".to_owned()));
        assert_eq!(thread_size(1), None);
    }

    #[test]
    fn quick_looks_sender_line_says_who_it_went_to() {
        use postio_search::results::Addressee;
        let to = |name: Option<&str>, address: &str, own: bool| Addressee {
            address: postio_model::EmailAddress::new(name, address),
            own,
        };
        let you = to(Some("Sam Doe"), "sam@example.com", true);
        let finance = to(Some("Finance team"), "finance@example.com", false);
        assert_eq!(
            sent_to(&[finance.clone(), you.clone()]).as_deref(),
            Some("to you, Finance team"),
            "you first, as one says it"
        );
        assert_eq!(
            sent_to(&[to(None, "tomas@example.com", false)]).as_deref(),
            Some("to tomas@example.com"),
            "no name, the address"
        );
        let many = [
            you,
            finance,
            to(Some("Ben Adeyemi"), "ben@example.com", false),
            to(Some("Lena Park"), "lena@example.com", false),
            to(Some("Priya Nair"), "priya@example.com", false),
        ];
        assert_eq!(
            sent_to(&many).as_deref(),
            Some("to you, Finance team, Ben Adeyemi and 2 more")
        );
        assert_eq!(sent_to(&[]), None);
    }

    #[test]
    fn a_saved_search_is_named_from_its_words_and_its_people() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 9, 26).unwrap();
        let ada = |address: &str| (address == "ada@example.com").then(|| "Ada Moreno".to_owned());
        let named = |query: &str| save_name(&postio_search::parse(query, today), &ada);
        assert_eq!(
            named("from:ada@example.com after:2026-07-01 atlas budget"),
            "Atlas budget from Ada",
            "screen 12"
        );
        assert_eq!(named("from:ada@example.com"), "From Ada");
        assert_eq!(
            named("budget from:{ada@example.com tomas@example.com}"),
            "Budget from Ada or tomas@example.com",
            "either of them (D26)"
        );
        assert_eq!(
            named("invoices to:ben@example.org"),
            "Invoices to ben@example.org"
        );
        assert_eq!(
            named("has:attachment"),
            "has:attachment",
            "nothing to say it better"
        );
    }

    #[test]
    fn operator_terms_are_chips_in_the_field_and_an_exclusion_is_struck() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 6, 2).unwrap();
        let typed = "atlas from:ada@example.com -to:ben@example.net budget";
        let chips = field_chips(typed, today);
        let said: Vec<(&str, &str, bool)> = chips
            .iter()
            .map(|chip| {
                (
                    &typed[chip.start..chip.operator_end],
                    &typed[chip.operator_end..chip.end],
                    chip.excluded,
                )
            })
            .collect();
        assert_eq!(
            said,
            [
                ("from:", "ada@example.com", false),
                ("-to:", "ben@example.net", true),
            ]
        );
        assert!(field_chips("atlas budget", today).is_empty());
    }

    #[test]
    fn a_reason_beside_a_subject_tag_does_not_say_subject_again() {
        let reasons = [
            RankReason::InSubject,
            RankReason::InSender,
            RankReason::Matches(1),
        ];
        assert_eq!(reason_beside(&reasons, &[Source::Subject]), "sender");
        assert_eq!(reason_beside(&reasons, &[Source::Body]), "subject · sender");
    }

    #[test]
    fn a_message_with_two_cards_is_one_match_and_the_subject_one_more() {
        assert_eq!(matches_of([Some(1), Some(1), Some(2), None]), 3);
        assert_eq!(matches_of([None::<u8>]), 1);
        assert_eq!(matches_of(Vec::<Option<u8>>::new()), 0);
    }

    #[test]
    fn nothing_matches_says_how_many_filters_found_nothing_together() {
        assert_eq!(nothing_matches(4), "Nothing matches all four filters");
        assert_eq!(nothing_matches(2), "Nothing matches both filters");
        assert_eq!(nothing_matches(12), "Nothing matches all 12 filters");
        assert_eq!(nothing_matches(1), "Nothing matches this search");
        assert_eq!(nothing_matches(0), "Nothing matches this search");
    }

    #[test]
    fn a_relaxation_of_a_set_names_every_value() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 9, 26).unwrap();
        let names = |address: &str| match address {
            "ada@example.com" => Some("Ada Moreno".to_owned()),
            "tomas@example.com" => Some("Tom\u{e1}s Reyes".to_owned()),
            _ => None,
        };
        let query = postio_search::parse(
            "from:{ada@example.com tomas@example.com} to:{ada@example.com bo} \
             label:{Atlas Harbor} in:{Inbox Archive} subject:{budget plan}",
            today,
        );
        let lines: Vec<String> = postio_search::relax::relax(&query)
            .iter()
            .map(|relaxation| relaxation_line(relaxation, &query, &names, today))
            .collect();
        assert_eq!(
            lines,
            [
                "Anyone, not just Ada Moreno or Tom\u{e1}s Reyes",
                "To anyone, not just Ada Moreno or bo",
                "Any label, not just \u{201c}Atlas\u{201d} or \u{201c}Harbor\u{201d}",
                "Look for a folder called \u{201c}Atlas\u{201d} or \u{201c}Harbor\u{201d}, not a label",
                "Any folder, not just \u{201c}Inbox\u{201d} or \u{201c}Archive\u{201d}",
                "Any subject, not just \u{201c}budget\u{201d} or \u{201c}plan\u{201d}",
            ]
        );
    }

    #[test]
    fn a_relaxation_says_the_one_thing_it_changes() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 9, 26).unwrap();
        let ada = |address: &str| address.starts_with("ada").then(|| "Ada Moreno".to_owned());
        let query = postio_search::parse(
            "from:ada has:attachment before:2026-03-01 subject:\"budget v4\"",
            today,
        );
        let lines: Vec<String> = postio_search::relax::relax(&query)
            .iter()
            .map(|relaxation| relaxation_line(relaxation, &query, &ada, today))
            .collect();
        assert_eq!(
            lines,
            [
                "Anyone, not just Ada Moreno",
                "Remove \u{201c}has attachment\u{201d}",
                "Remove \u{201c}before March\u{201d}",
                "Any subject, not just \u{201c}budget v4\u{201d}",
                "Look for \u{201c}budget v4\u{201d} anywhere, not just the subject",
            ],
            "screen 13's, in the query's order"
        );

        let other = postio_search::parse(
            "label:Atlas after:2026-07-01 -from:ben@example.org is:unread atlas budget",
            today,
        );
        let lines: Vec<String> = postio_search::relax::relax(&other)
            .iter()
            .map(|relaxation| relaxation_line(relaxation, &other, &ada, today))
            .collect();
        assert_eq!(
            lines,
            [
                "Any label, not just \u{201c}Atlas\u{201d}",
                "Look for a folder called \u{201c}Atlas\u{201d}, not a label",
                "Remove \u{201c}since July\u{201d}",
                "Remove \u{201c}-from:ben@example.org\u{201d}",
                "Remove \u{201c}unread\u{201d}",
                "Remove \u{201c}atlas\u{201d}",
                "Remove \u{201c}budget\u{201d}",
            ]
        );
        assert_eq!(relaxation_count(4, false), "4 threads");
        assert_eq!(relaxation_count(1, false), "1 thread");
        assert_eq!(relaxation_count(18_204, false), "18,204 threads");
        assert_eq!(relaxation_count(10_000, true), "10,000+ threads");
    }

    #[test]
    fn a_file_card_says_its_type_who_sent_it_and_where_it_came() {
        assert_eq!(
            file_kind("Atlas-Q3-budget.xlsx", "application/octet-stream"),
            "XLSX"
        );
        assert_eq!(file_kind("variance-by-team.numbers", ""), "NUM");
        assert_eq!(file_kind("scan", "application/pdf"), "PDF");
        assert_eq!(file_kind("notes", "text/plain; charset=utf-8"), "TXT");
        assert_eq!(file_preview("Atlas-Q3-budget.xlsx", ""), FilePreview::Sheet);
        assert_eq!(file_preview("deck.pptx", ""), FilePreview::Slides);
        assert_eq!(file_preview("whiteboard.jpg", ""), FilePreview::Image);
        assert_eq!(file_preview("Atlas-Sep-actuals.pdf", ""), FilePreview::Page);
        assert_eq!(
            file_meta("Ada Moreno", "26 Sep", 48 * 1024),
            "Ada Moreno \u{b7} 26 Sep \u{b7} 48 KB"
        );
        assert_eq!(
            file_subject("Re: Atlas Q3 budget"),
            "in \u{2018}Re: Atlas Q3 budget\u{2019}"
        );
        assert_eq!(FILES_TITLE, "Files whose name or contents match");
        let mac = postio_core::Keymap::resolve_on(
            &postio_config::KeyBindings::default(),
            postio_config::paths::Platform::Apple,
        );
        assert!(files_note(&mac, true).starts_with("contents are indexed on this Mac"));
        assert!(files_note(&mac, false).contains("still reading"));
    }

    #[test]
    fn the_arrows_walk_the_files_grid_as_finder_does() {
        use GridArrow::{Down, Left, Right, Up};
        assert_eq!(grid_step(None, Right, 6, 4), Some(0), "no ring: the first");
        assert_eq!(grid_step(Some(0), Right, 6, 4), Some(1));
        assert_eq!(grid_step(Some(0), Left, 6, 4), None);
        assert_eq!(grid_step(Some(5), Right, 6, 4), None);
        assert_eq!(grid_step(Some(1), Down, 6, 4), Some(5));
        assert_eq!(
            grid_step(Some(3), Down, 6, 4),
            Some(5),
            "down onto a shorter row lands on its last card"
        );
        assert_eq!(grid_step(Some(5), Down, 6, 4), None, "the last row");
        assert_eq!(grid_step(Some(5), Up, 6, 4), Some(1));
        assert_eq!(grid_step(Some(2), Up, 6, 4), None);
        assert_eq!(grid_step(None, Down, 0, 4), None, "no cards");
    }

    #[test]
    fn the_words_name_the_machine_they_run_on() {
        let on = |platform| {
            postio_core::Keymap::resolve_on(&postio_config::KeyBindings::default(), platform)
        };
        let mac = on(postio_config::paths::Platform::Apple);
        let linux = on(postio_config::paths::Platform::Freedesktop);
        assert_eq!(
            searched(&linux, 3, true, false),
            "Searched all 3 messages on this computer."
        );
        assert_eq!(
            searched(&mac, 3, true, false),
            "Searched all 3 messages on this Mac."
        );
        assert!(files_note(&linux, true).contains("indexed on this computer"));
        assert!(files_note(&linux, false).contains("files on this computer"));
        assert!(file_not_here(&linux).contains("isn\u{2019}t on this computer"));
        assert!(file_not_here(&mac).contains("isn\u{2019}t on this Mac"));
        assert!(
            parsed_footer(&linux, 3, Duration::from_millis(21), false)
                .starts_with("parsed on this computer")
        );
    }

    #[test]
    fn searched_claims_attachment_contents_only_once_they_are_read() {
        let mac = postio_core::Keymap::resolve_on(
            &postio_config::KeyBindings::default(),
            postio_config::paths::Platform::Apple,
        );
        assert_eq!(
            searched(&mac, 18_204, true, true),
            "Searched all 18,204 messages on this Mac, including attachment contents."
        );
        assert_eq!(
            searched(&mac, 18_204, true, false),
            "Searched all 18,204 messages on this Mac.",
            "US6 scenario 2: contents not yet extracted"
        );
        assert_eq!(
            searched(&mac, 18_204, false, false),
            "Searched 18,204 messages on this Mac. Some are still being indexed.",
            "bodies still backfilling: not \u{201c}all\u{201d}"
        );
        assert_eq!(
            searched(&mac, 1, true, false),
            "Searched the 1 message on this Mac."
        );
        for each in [
            searched(&mac, 18_204, true, true),
            searched(&mac, 18_204, false, true),
            searched_footer(18_204, Duration::from_millis(41)),
        ] {
            assert!(
                !each.contains("server") && !each.contains('\u{2325}'),
                "never the server line (S4): {each}"
            );
        }
        assert_eq!(
            searched_footer(18_204, Duration::from_millis(41)),
            "Searched 18,204 messages \u{b7} 41 ms"
        );
        assert_eq!(
            NO_RESULTS_BODY,
            "Each line below loosens one filter and shows how many threads you would \
             get. Pick one, or press its number."
        );
    }

    #[test]
    fn the_typing_states_words_are_the_designs() {
        use chrono::{Local, TimeZone};
        let now = Local.with_ymd_and_hms(2026, 9, 26, 15, 0, 0).unwrap();
        let at = |m, d| Local.with_ymd_and_hms(2026, m, d, 9, 0, 0).unwrap();
        assert_eq!(
            person_detail("ada@example.org", 412, Some(at(9, 26)), now),
            "ada@example.org \u{b7} 412 messages \u{b7} last today"
        );
        assert_eq!(last_when(at(9, 19), now), "19 Sep");
        assert_eq!(last_when(at(6, 2), now), "June");
        assert_eq!(
            last_when(Local.with_ymd_and_hms(2025, 6, 2, 9, 0, 0).unwrap(), now),
            "June 2025"
        );
        assert_eq!(
            person_detail::<Local>("admin@example.org", 9, None, now),
            "admin@example.org \u{b7} 9 messages"
        );
        assert_eq!(
            matching("People", "ad"),
            "People matching \u{201c}ad\u{201d}"
        );
        assert_eq!(matching("People", ""), "People");
        assert_eq!(latest_from("Ada Moreno"), "Latest from Ada Moreno");
        assert_eq!(origin_line("last month"), "from \u{2018}last month\u{2019}");
        assert_eq!(files_named("at"), "Files named \u{201c}at\u{2026}\u{201d}");
        assert_eq!(
            files_detail(&["Atlas-Q3-budget.xlsx", "Atlas-Sep-actuals.pdf", "a", "b"]),
            "Atlas-Q3-budget.xlsx, Atlas-Sep-actuals.pdf and 2 more"
        );
        assert_eq!(files_detail(&["one.pdf"]), "one.pdf");
        assert_eq!(
            list_detail("atlas-planning.example.org"),
            "mailing list \u{b7} atlas-planning.example.org"
        );
        assert_eq!(
            parsed_footer(
                &postio_core::Keymap::resolve_on(
                    &postio_config::KeyBindings::default(),
                    postio_config::paths::Platform::Apple,
                ),
                3,
                Duration::from_millis(21),
                false
            ),
            "parsed on this Mac \u{b7} 3 matches \u{b7} 21 ms"
        );
        let day = |m, d| chrono::NaiveDate::from_ymd_opt(2026, m, d).unwrap();
        assert_eq!(
            plain_results_note(Some(day(8, 1)), Some(day(9, 1))),
            "August 2026 \u{b7} newest first"
        );
        assert_eq!(plain_results_note(Some(day(8, 1)), None), "newest first");
        let mac = Keymap::resolve_on(
            &postio_config::KeyBindings::default(),
            postio_config::paths::Platform::Apple,
        );
        let said = |hints: Vec<Hint>| {
            hints
                .into_iter()
                .map(|hint| format!("{} {}", hint.key, hint.label))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            said(operator_hints(&mac, "from")),
            [
                "Return add as chip",
                "alt+Return exclude (-from:)",
                "Up Down choose",
                "BackSpace back to words"
            ]
        );
        assert_eq!(
            said(plain_hints(&mac)),
            [
                "Up Down move",
                "Return open",
                "Tab edit as chips",
                "cmd+BackSpace keep as words"
            ]
        );
        assert_eq!(
            said(prefix_hints(&mac)),
            [
                "Tab complete",
                "Up Down move",
                "Return open",
                "cmd+Return all results"
            ]
        );
    }

    mod in_threads {
        use super::super::*;
        use chrono::TimeZone;
        use postio_model::thread_view::{Line, ThreadMember, ThreadView};
        use postio_model::{EmailAddress, MessageId, ThreadId};
        use postio_search::results::{ConversationMatch, Match, Passage, Source};

        fn harbor() -> ThreadView {
            let members = (1..=8u32)
                .map(|number| ThreadMember {
                    id: MessageId::new(i64::from(number)),
                    from: Some(if number % 2 == 0 {
                        EmailAddress::new(Some("Ben Adeyemi"), "ben@example.com")
                    } else {
                        EmailAddress::new(Some("Tom\u{e1}s Reyes"), "tomas@example.com")
                    }),
                    own: false,
                    at: chrono::Utc
                        .with_ymd_and_hms(2026, 9, 12 + number, 12, 0, 0)
                        .unwrap(),
                    seen: true,
                    has_attachments: false,
                    line: Line::Own(format!("message {number}")),
                    answers: None,
                    marker: false,
                })
                .collect();
            ThreadView {
                thread: ThreadId::new(7),
                subject: Some("Harbor API draft v3".to_owned()),
                members,
                draft: None,
                muted: None,
                labels: Vec::new(),
            }
        }

        fn now() -> DateTime<chrono::Local> {
            chrono::Local
                .with_ymd_and_hms(2026, 9, 24, 15, 0, 0)
                .unwrap()
        }

        #[test]
        fn a_match_child_says_who_wrote_it_and_when_as_every_row_does() {
            let mut view = harbor();
            view.members[4].own = true;
            let kids = children(
                &view,
                &[
                    found(3, Source::Body, "rate limit"),
                    found(5, Source::Body, "rate limit"),
                ],
                now(),
            );
            let people: Vec<_> = kids
                .iter()
                .filter_map(|kid| match kid {
                    ResultChild::Match {
                        sender,
                        initials,
                        time,
                        ..
                    } => Some((sender.as_str(), initials.as_str(), time.as_str())),
                    ResultChild::Gap { .. } => None,
                })
                .collect();
            let said = |number: usize| crate::row::timestamp(view.members[number - 1].at, now());
            assert_eq!(
                people,
                [
                    ("Tom\u{e1}s Reyes", "TR", said(3).as_str()),
                    ("You", "TR", said(5).as_str()),
                ]
            );
            assert_ne!(said(3), "", "the time is said");
        }

        fn found(message: i64, source: Source, text: &str) -> ConversationMatch {
            ConversationMatch {
                message: Some(MessageId::new(message)),
                from: None,
                found: Match {
                    source,
                    passage: Some(Passage {
                        text: text.to_owned(),
                        ranges: std::iter::once(0..4).collect(),
                        elided_start: false,
                        elided_end: false,
                    }),
                    when: None,
                },
                to: Vec::new(),
            }
        }

        fn matches_3_5_6() -> Vec<ConversationMatch> {
            vec![
                found(3, Source::Body, "rate limit headers on every reply"),
                found(5, Source::Body, "rate limit headers again"),
                found(6, Source::Quoted, "rate limit headers, quoted"),
            ]
        }

        #[test]
        fn matches_one_child_each_and_a_gap_where_a_message_did_not() {
            let kids = children(&harbor(), &matches_3_5_6(), now());
            assert_eq!(kids.len(), 6, "{kids:?}");
            let numbers: Vec<_> = kids
                .iter()
                .map(|kid| match kid {
                    ResultChild::Match { number, .. } => Some(*number),
                    ResultChild::Gap { .. } => None,
                })
                .collect();
            assert_eq!(numbers, [None, Some(3), None, Some(5), Some(6), None]);
            assert_eq!(
                gap_line(&kids[2]),
                "Message 4 doesn't match \u{b7} Ben, 16 Sep"
            );
            let ResultChild::Match { message, tag, .. } = &kids[1] else {
                panic!("the second is a match");
            };
            assert_eq!((*message, tag.as_str()), (MessageId::new(3), "message 3"));
        }

        /// #2178: every message the expansion does not show is in a gap line, including before the
        /// first match and after the last.
        #[test]
        fn the_gaps_account_for_every_message_not_shown() {
            let kids = children(&harbor(), &matches_3_5_6(), now());
            let gaps: Vec<String> = kids
                .iter()
                .filter(|kid| matches!(kid, ResultChild::Gap { .. }))
                .map(gap_line)
                .collect();
            assert_eq!(
                gaps,
                [
                    "Messages 1\u{2013}2 don't match \u{b7} Tom\u{e1}s, Ben, 13 Sep",
                    "Message 4 doesn't match \u{b7} Ben, 16 Sep",
                    "Messages 7\u{2013}8 don't match \u{b7} Tom\u{e1}s, Ben, Sat",
                ]
            );
            let said: u32 = kids
                .iter()
                .map(|kid| match kid {
                    ResultChild::Match { .. } => 1,
                    ResultChild::Gap { from, to, .. } => to - from + 1,
                })
                .sum();
            assert_eq!(said, 8, "every message of the eight, once: {kids:?}");
        }

        /// #2178: a gap says its first message's day as every row says a time, so "Mon" beside "16
        /// Mar" does not read as yesterday.
        #[test]
        fn a_gap_dates_its_first_message_as_the_rows_do() {
            let mut view = harbor();
            for (at, member) in view.members.iter_mut().enumerate() {
                member.at = chrono::Utc
                    .with_ymd_and_hms(2026, 3, 16 + at as u32, 12, 0, 0)
                    .unwrap();
            }
            let kids = children(&view, &[found(4, Source::Body, "rate limit")], now());
            let gaps: Vec<String> = kids
                .iter()
                .filter(|kid| matches!(kid, ResultChild::Gap { .. }))
                .map(gap_line)
                .collect();
            assert_eq!(
                gaps,
                [
                    "Messages 1\u{2013}3 don't match \u{b7} Tom\u{e1}s, Ben, 16 Mar",
                    "Messages 5\u{2013}8 don't match \u{b7} Tom\u{e1}s, Ben, 20 Mar",
                ]
            );
        }

        #[test]
        fn a_match_on_the_first_and_the_last_message_needs_no_gap_at_either_end() {
            let kids = children(
                &harbor(),
                &[
                    found(1, Source::Body, "rate limit"),
                    found(8, Source::Body, "rate limit"),
                ],
                now(),
            );
            let lines: Vec<String> = kids
                .iter()
                .map(|kid| match kid {
                    ResultChild::Match { number, .. } => format!("{number}"),
                    gap @ ResultChild::Gap { .. } => gap_line(gap),
                })
                .collect();
            assert_eq!(
                lines,
                [
                    "1",
                    "Messages 2\u{2013}7 don't match \u{b7} Ben, Tom\u{e1}s, 14 Sep",
                    "8"
                ]
            );
        }

        #[test]
        fn a_file_match_is_tagged_and_its_passage_names_the_file() {
            let file = found(
                5,
                Source::FileContent {
                    attachment: postio_model::AttachmentId::new(1),
                    name: "limits.pdf".to_owned(),
                    location: Location::Page(2),
                },
                "rate limit headers table",
            );
            let kids = children(&harbor(), &[file], now());
            let [_, ResultChild::Match { tag, passage, .. }, _] = kids.as_slice() else {
                panic!("one match between two gaps: {kids:?}");
            };
            assert_eq!(tag, "message 5 \u{b7} file");
            let passage = passage.as_ref().expect("a passage");
            assert!(passage.text.contains("limits.pdf"), "{passage:?}");
            assert!(passage.text.ends_with("rate limit headers table"));
            let marked = &passage.text[passage.ranges[0].clone()];
            assert_eq!(marked, "rate", "the marks moved with the words");
        }

        #[test]
        fn a_run_of_non_matches_is_one_gap() {
            let kids = children(
                &harbor(),
                &[
                    found(2, Source::Body, "rate limit"),
                    found(6, Source::Body, "rate limit"),
                ],
                now(),
            );
            assert_eq!(kids.len(), 5, "{kids:?}");
            assert_eq!(
                gap_line(&kids[2]),
                "Messages 3\u{2013}5 don't match \u{b7} Tom\u{e1}s, Ben, 15 Sep"
            );
        }

        #[test]
        fn the_subject_and_strangers_are_no_children() {
            let subject = ConversationMatch {
                message: None,
                from: None,
                found: Match {
                    source: Source::Subject,
                    passage: None,
                    when: None,
                },
                to: Vec::new(),
            };
            let kids = children(
                &harbor(),
                &[
                    subject,
                    found(99, Source::Body, "rate"),
                    found(4, Source::Body, "rate"),
                ],
                now(),
            );
            let shown: Vec<u32> = kids
                .iter()
                .filter_map(|kid| match kid {
                    ResultChild::Match { number, .. } => Some(*number),
                    ResultChild::Gap { .. } => None,
                })
                .collect();
            assert_eq!(shown, [4], "{kids:?}");
        }

        #[test]
        fn the_summary_and_the_single_match_snippet() {
            assert_eq!(match_summary(3, 6), "3 of 6 messages match");
            assert_eq!(
                single_match_snippet(5, 8, "Tom\u{e1}s", "the rate limit"),
                "Message 5 of 8 \u{b7} Tom\u{e1}s: the rate limit"
            );
            assert_eq!(
                single_match_snippet(5, 8, "Tom\u{e1}s", " "),
                "Message 5 of 8 \u{b7} Tom\u{e1}s",
                "no words yet, no colon"
            );
        }
    }
}
