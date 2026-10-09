//! The words Focus's search dropdown and results view say (spec 010).
//!
//! The copy comes from the Mac design (`Design/focus-macos-search`, SPEC
//! section 2 and screens 01 and 03) and is composed here, once, so the Mac
//! and any later frontend say the same thing and a test can hold it.

use std::time::Duration;

use chrono::{DateTime, Datelike, TimeZone};
use postio_core::{CommandId, Keymap};

use crate::hints::{self, Hint};

/// The operators the empty dropdown lists under "Search by", each with the
/// hint after it (screen 01), in the design's order.
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

/// When a recent search ran, as the right-hand column of its row says it:
/// "today", "yesterday", the weekday within the last week ("Mon"), then the
/// day and month ("21 Sep"), with the year for another year. Both times are
/// read in their own zone, so pass them in the person's.
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

/// `n` with thousands separators: 18204 is "18,204".
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

/// The count `capped` makes a floor of: "10,000+".
fn count(total: u64, capped: bool) -> String {
    format!("{}{}", grouped(total), if capped { "+" } else { "" })
}

/// The dropdown footer's right-hand side: "48 matches · 38 ms", or
/// "10,000+ matches · 41 ms" when the count stopped at the cap.
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

/// One "Narrow to" pill: the operator in monospace, the value, and how
/// many of the results it would keep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NarrowPill {
    /// The operator with its colon: "from:", "has:", "label:".
    pub op: String,
    /// What it is set to: a person, "attachment", a label.
    pub value: String,
    /// The count, grouped: "9", "1,204".
    pub count: String,
}

impl NarrowPill {
    /// The pill as one line, for a screen reader and for tests.
    pub fn label(&self) -> String {
        format!("{} {} {}", self.op, self.value, self.count)
    }
}

/// The "Narrow to" pill for `op` `value`, which keeps `count` results.
pub fn narrow_pill(op: &str, value: &str, count: u64) -> NarrowPill {
    NarrowPill {
        op: op.to_owned(),
        value: value.to_owned(),
        count: grouped(count),
    }
}

/// The dropdown's last row: "Show all 48 results", or, while a short prefix
/// is being typed, "Show all 214 results for \u{201c}at\u{201d}". One result is
/// "Show 1 result"; a capped count is a floor ("10,000+").
pub fn show_all(total: u64, capped: bool, typed: Option<&str>) -> String {
    let for_typed = typed
        .map(|typed| format!(" for \u{201c}{typed}\u{201d}"))
        .unwrap_or_default();
    if total == 1 && !capped {
        return format!("Show 1 result{for_typed}");
    }
    format!("Show all {} results{for_typed}", count(total, capped))
}

/// The field's placeholder while it is empty (screen 01).
pub const PLACEHOLDER: &str = "Search mail, people and files, or type > for commands";
/// The empty dropdown's first section: the searches run lately.
pub const RECENT: &str = "Recent";
/// What the Recent section's note says after its key: "⌥⌫ forgets one".
pub const FORGETS_ONE: &str = "forgets one";
/// The pinned saved searches' section.
pub const SAVED_SEARCHES: &str = "Saved searches";
/// The cheat sheet's section.
pub const SEARCH_BY: &str = "Search by";
/// The cheat sheet's last line, before the example sentence.
pub const JUST_TYPE_IT: &str = "Or just type it:";
/// Between the example sentence and the query it becomes.
pub const BECOMES: &str = "becomes";
/// The plain English the cheat sheet lowers live, as screen 01 has it.
pub const EXAMPLE: &str = "invoices from ada last month";
/// The words state's hits.
pub const TOP_HITS: &str = "Top hits";
/// The Top hits section's note.
pub const TOP_HITS_NOTE: &str = "ranked by sender, recency and where the words matched";
/// The words state's filter pills.
pub const NARROW_TO: &str = "Narrow to";
/// What the Show all row says after its count.
pub const SHOW_ALL_DETAIL: &str = "in the main window, with filters, a timeline and Quick Look";

/// How many conversations a recent search found: "48 results".
pub fn results_count(n: u64) -> String {
    match n {
        1 => "1 result".to_owned(),
        n => format!("{} results", grouped(n)),
    }
}

/// A top hit's date column: the day and month ("26 Sep"), with the year's
/// last two digits for another year ("22 Oct 25"), the list's short form
/// (`row::timestamp`): the design never draws a year, and the full one
/// does not fit the results' 62-pt column. Both times in the person's zone.
pub fn hit_date<Tz: TimeZone>(at: DateTime<Tz>, now: DateTime<Tz>) -> String {
    let (day, today) = (at.date_naive(), now.date_naive());
    if day.year() == today.year() {
        day.format("%-d %b").to_string()
    } else {
        day.format("%-d %b %y").to_string()
    }
}

/// A hit's folder column, as the operator that would find it there:
/// `in:Inbox`, quoted when the name has a space.
pub fn in_folder(name: &str) -> String {
    if name.contains(char::is_whitespace) {
        format!("in:\"{name}\"")
    } else {
        format!("in:{name}")
    }
}

/// The arrows, as one hint: the panel's own walk, which the toolkit keeps
/// (009 FR-004), not a command.
fn arrows(label: &str) -> Hint {
    hints::fixed(
        "Up Down",
        label,
        "the arrows move the dropdown's highlight, which the toolkit keeps",
    )
}

/// The empty dropdown's footer (screen 01): move, run a recent search
/// again, the saved searches' keys, and `>` for commands.
pub fn empty_hints(keymap: &Keymap) -> Vec<Hint> {
    let mut hints = vec![
        arrows("move"),
        hints::fixed(
            "Return",
            "run again",
            "Return runs the highlighted row: the field's own key, not a command",
        ),
    ];
    // ⌥1 to ⌥4 as one cap, when the four keys are one modifier and the digits.
    let saved = [
        CommandId::SavedSearch1,
        CommandId::SavedSearch2,
        CommandId::SavedSearch3,
        CommandId::SavedSearch4,
    ];
    let keys: Vec<Option<&str>> = saved.iter().map(|id| keymap.binding(*id)).collect();
    if let [Some(first), .., Some(last)] = keys.as_slice() {
        let run = first
            .strip_suffix('1')
            .filter(|modifier| last.strip_suffix('4') == Some(*modifier))
            .map(|modifier| format!("{modifier}1\u{2013}4"));
        hints.extend(match run {
            Some(key) => Some(Hint {
                key,
                label: "saved".to_owned(),
            }),
            None => hints::pair(keymap, saved[0], saved[3], "saved"),
        });
    }
    hints.push(hints::fixed(
        ">",
        "commands",
        "`>` typed first is the bar's commands-only prefix, not a key",
    ));
    hints
}

/// The words dropdown's footer (screen 03): move, open the highlighted
/// message, all results, and Tab for the first filter.
pub fn words_hints(keymap: &Keymap) -> Vec<Hint> {
    let mut hints = vec![
        arrows("move"),
        hints::fixed(
            "Return",
            "open message",
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

// ---------------------------------------------------------------------------
// The results view (spec 010 step 3, design §3, screens 06 and 07)
// ---------------------------------------------------------------------------

use postio_search::query::{Filter, State};
use postio_search::results::{Location, RankReason, Source};

/// Why a top hit ranked, as the line under its sender says it: at most two
/// reasons, then how many of its messages matched when that is more than
/// one ("you replied · 3 matches"). The app says flagged, never starred
/// (D19).
pub fn reason_line(reasons: &[RankReason]) -> String {
    let mut words: Vec<String> = reasons
        .iter()
        .filter_map(|reason| {
            Some(
                match reason {
                    RankReason::Replied => "you replied",
                    RankReason::Flagged => "you flagged",
                    RankReason::FrequentSender => "frequent sender",
                    RankReason::InSubject => "subject",
                    RankReason::InFileName => "file name",
                    RankReason::Matches(_) => return None,
                }
                .to_owned(),
            )
        })
        .take(2)
        .collect();
    let matched = reasons.iter().find_map(|reason| match reason {
        RankReason::Matches(n) => Some(*n),
        _ => None,
    });
    if let Some(n) = matched.filter(|n| *n > 1) {
        words.push(format!("{} matches", grouped(u64::from(n))));
    }
    words.join(" \u{b7} ")
}

/// Where the words matched, as a row's source tag says it: "body", "quoted
/// text", "subject", or the file's name.
pub fn source_tag(source: &Source) -> String {
    match source {
        Source::Subject => "subject".to_owned(),
        Source::Body => "body".to_owned(),
        Source::Quoted => "quoted text".to_owned(),
        Source::FileName { name, .. } | Source::FileContent { name, .. } => name.clone(),
    }
}

/// A row's tag, `sources` the passage's first and then every place it
/// matched: where the passage came from, and a file that also matched
/// beside a passage that is not the file's ("body +
/// Atlas-Q3-budget.xlsx", screen 06). A subject is named only when it is
/// the passage's source, which is when nothing else matched: otherwise the
/// row's first line shows it, marked.
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

/// The match a row's second line shows: the first with a passage; until
/// one is cut (or when none can be), the first that is not the subject,
/// which the row's first line already draws; the subject only when nothing
/// else matched.
pub fn shown_match(
    matches: &[postio_search::results::Match],
) -> Option<&postio_search::results::Match> {
    matches
        .iter()
        .find(|each| each.passage.is_some())
        .or_else(|| matches.iter().find(|each| each.source != Source::Subject))
        .or_else(|| matches.first())
}

/// Where in a file a passage sits, before the passage: "Page 2",
/// "Sheet ‘Summary’, row 14" (FR-025).
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

/// The Top hits group's note in the results (screen 06).
pub const TOP_HITS_RESULTS_NOTE: &str = "why each one ranked is under the sender";
/// The first month group's note: the groups run newest first.
pub const NEWEST_FIRST: &str = "newest first";
/// The group of conversations older than the timeline's twelve months.
pub const EARLIER: &str = "Earlier";
/// The quiet hint at the right of the results' field.
pub const TO_EDIT: &str = "/ to edit";

/// A month group's title: "September 2026".
pub fn month_title(month: chrono::NaiveDate) -> String {
    month.format("%B %Y").to_string()
}

/// A month group's header as one line: "September 2026 · 9", what a
/// screen reader says for it.
pub fn month_group(month: chrono::NaiveDate, n: u64) -> String {
    group_line(&month_title(month), n)
}

/// Any group's header as one line: "Top hits · 3", "Earlier · 120".
pub fn group_line(title: &str, n: u64) -> String {
    format!("{title} \u{b7} {}", grouped(n))
}

/// A count of conversations: "48 conversations", "1 conversation",
/// "10,000+ conversations".
fn conversations(total: u64, capped: bool) -> String {
    let noun = if total == 1 && !capped {
        "conversation"
    } else {
        "conversations"
    };
    format!("{} {noun}", count(total, capped))
}

/// The timeline's count line (§3.3): "48 conversations".
pub fn count_line(total: u64, capped: bool) -> String {
    conversations(total, capped)
}

/// The timeline's sub-line (§3.3): "12 files · 6 people · last 12 months".
pub fn sub_line(files: u64, people: u64) -> String {
    let files = match files {
        1 => "1 file".to_owned(),
        n => format!("{} files", grouped(n)),
    };
    let people = match people {
        1 => "1 person".to_owned(),
        n => format!("{} people", grouped(n)),
    };
    format!("{files} \u{b7} {people} \u{b7} last 12 months")
}

/// The results footer's right-hand side (§3.5): "48 conversations · local
/// index · 41 ms".
pub fn results_footer(total: u64, capped: bool, elapsed: Duration) -> String {
    let took = match elapsed.as_millis() {
        0 => "<1 ms".to_owned(),
        ms => format!("{ms} ms"),
    };
    format!(
        "{} \u{b7} local index \u{b7} {took}",
        conversations(total, capped)
    )
}

/// One of the filter bar's buttons (§3.2), in its order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FilterKind {
    /// From ▾: a person.
    From,
    /// To ▾: a person.
    To,
    /// Date ▾: `after:` and `before:`.
    Date,
    /// Anywhere ▾: a folder.
    Anywhere,
    /// Label ▾.
    Label,
    /// A toggle: `has:attachment`.
    Attachment,
    /// A toggle: `has:action`.
    HasAction,
    /// A toggle: `is:unread`.
    Unread,
}

impl FilterKind {
    /// Every button, left to right.
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

    /// What the button says with nothing applied.
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

    /// Whether it opens a popover (▾), rather than toggling.
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

    /// Whether `filter` is one this button applies.
    pub fn holds(self, filter: &Filter) -> bool {
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

/// The month `date` falls in, as a date label says it: "July", or "July
/// 2025" in another year than `today`'s.
fn month_word(date: chrono::NaiveDate, today: chrono::NaiveDate) -> String {
    if date.year() == today.year() {
        date.format("%B").to_string()
    } else {
        date.format("%B %Y").to_string()
    }
}

/// A day as a date label says it: the month on its first day, else "14
/// Jul", with the year in another year.
fn day_word(date: chrono::NaiveDate, today: chrono::NaiveDate) -> String {
    match (date.day(), date.year() == today.year()) {
        (1, _) => month_word(date, today),
        (_, true) => date.format("%-d %b").to_string(),
        (_, false) => date.format("%-d %b %Y").to_string(),
    }
}

/// What a filter button says (§3.2): its title with nothing applied, its
/// value once one is ("From: Ada Moreno", "Since July", "Label: Atlas"),
/// "+1" for each more. `filters` are the query's positive clauses this
/// button holds; `name_of` gives a person's name for an address.
pub fn filter_button_label(
    kind: FilterKind,
    filters: &[Filter],
    name_of: &dyn Fn(&str) -> Option<String>,
    today: chrono::NaiveDate,
) -> String {
    let held: Vec<&Filter> = filters.iter().filter(|filter| kind.holds(filter)).collect();
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
        Filter::In(name) | Filter::Label(name) => name.clone(),
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

/// What a screen reader says for a result (design §5): "Ada Moreno, Re:
/// Atlas Q3 budget, matched in body: …, Inbox, 26 Sep". A row with no
/// passage yet leaves the match out.
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

/// The results footer's keys (§3.5): move, open, select, edit the query,
/// and Esc back to the inbox. Quick Look's Space joins with step 5.
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

/// The bulk bar's verbs while results are checked (§3.5): Archive, Label,
/// Move, Mark read, Snooze, each with its key.
pub fn bulk_hints(keymap: &Keymap) -> Vec<Hint> {
    [
        (CommandId::Archive, "Archive"),
        (CommandId::AddLabel, "Label"),
        (CommandId::Move, "Move"),
        (CommandId::ToggleRead, "Mark read"),
        (CommandId::Snooze, "Snooze"),
    ]
    .into_iter()
    .filter_map(|(id, label)| hints::hint(keymap, id, label))
    .collect()
}

/// The bulk bar's right while some results are checked (§3.5): "⇧X select
/// all 12", every conversation the query matches. `None` when nothing is
/// bound to Select all.
pub fn select_all_hint(keymap: &Keymap, total: u64, capped: bool) -> Option<Hint> {
    hints::hint(
        keymap,
        CommandId::SelectAll,
        &format!("select all {}", count(total, capped)),
    )
}

/// The name the Save popover offers for `query` (§3.9): its words,
/// capitalised, then "from" and "to" with each person's first name --
/// "Atlas budget from Ada". `name_of` finds an address's name. A query
/// with no words and no people is its own name.
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
        match &clause.filter {
            Filter::From(who) => said.push(format!("from {}", first_name(who))),
            Filter::To(who) => said.push(format!("to {}", first_name(who))),
            _ => {}
        }
    }
    let name = said.join(" ");
    let mut letters = name.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => query.input().trim().to_owned(),
    }
}

/// The results tabs' words, in their order (§3.2).
pub const TABS: [&str; 3] = ["Conversations", "Files", "People"];

/// A count as a tab says it: "48", "10,000+".
pub fn tab_count(total: u64, capped: bool) -> String {
    count(total, capped)
}

/// "‹ Inbox", the results toolbar's way back (§3.1).
pub const BACK_TO_INBOX: &str = "Inbox";
/// The results toolbar's Save search button (§3.1).
pub const SAVE_SEARCH: &str = "Save search";
/// The quiet word before the Sort menu (§3.2).
pub const SORT: &str = "Sort";
/// The Sort menu's ranked order (§3.2).
pub const BEST_MATCH: &str = "Best match";
/// The Sort menu's date order (§3.2).
pub const NEWEST: &str = "Newest";
/// The timeline's hint on its right (§3.3).
pub const TIMELINE_HINT: &str = "Matches by month \u{b7} drag across months to narrow";

/// A list popover's own search field (§3.6): what it says while empty.
/// The Date popover has none.
pub fn popover_placeholder(kind: FilterKind) -> &'static str {
    match kind {
        FilterKind::From | FilterKind::To => "Filter people in these results",
        FilterKind::Anywhere => "Filter folders",
        FilterKind::Label => "Filter labels",
        _ => "",
    }
}

/// A popover's footer (§3.6, screen 08): Space toggles, ⌥-click
/// excludes, ↩ applies; the Date popover only applies. The keys are the
/// popover's own, not commands, spelled as the registry spells keys.
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

/// The timeline's sub-line while a popover previews a change (screen
/// 09): "previewing Jul – Sep · ↩ applies".
pub fn previewing(what: &str) -> String {
    format!("previewing {what} \u{b7} \u{21a9} applies")
}

/// One of the Date popover's presets (§3.6, screen 09): what it says, and
/// the `after:` it writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatePreset {
    /// "Last 30 days".
    pub label: &'static str,
    /// The `after:` it writes; `None` for Any time, which takes the dates
    /// out.
    pub start: Option<chrono::NaiveDate>,
    /// Where its count is in `SearchFacets::presets`.
    pub count_at: usize,
}

/// The Date popover's presets, top to bottom: Any time, Last 7 days, Last
/// 30 days, This quarter, This year. Custom… ([`CUSTOM_DATE`]) follows
/// them and writes nothing itself.
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

/// The Date popover's last preset: its field is where a custom date goes.
pub const CUSTOM_DATE: &str = "Custom\u{2026}";

/// The Date popover's line under its field (screen 09).
pub const DATE_WORDS_HINT: &str = "Type a date in plain words, or drag across the months.";

/// What the Date popover's plain words became (screen 09): "→
/// after:2026-07-01".
pub fn date_parsed(terms: &str) -> String {
    format!("\u{2192} {terms}")
}

/// The months `first` to `last` (any day in each), as the timeline and
/// the Date popover say a range: "Jul – Sep", "Sep", or with `year`
/// "Jul – Sep 2026", and both years when they differ: "Dec 2025 – Feb
/// 2026".
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

/// The timeline's hint while a range is selected (screen 09): "Jul – Sep
/// selected · drag to change". The keys that step it follow as a
/// [`timeline_step`] hint.
pub fn range_selected(range: &str) -> String {
    format!("{range} selected \u{b7} drag to change")
}

/// ⌥←/⌥→ "steps a month", while a range is selected.
pub fn timeline_step(keymap: &Keymap) -> Option<Hint> {
    hints::pair(
        keymap,
        CommandId::StepRangeBack,
        CommandId::StepRangeForward,
        "steps a month",
    )
}

/// The Date popover's result line (screen 09): "12 of 21", the
/// conversations its dates keep of those it opened on.
pub fn date_result(kept: u64, of: u64, capped: bool) -> String {
    format!("{} of {}", count(kept, capped), count(of, capped))
}

/// The bulk bar's count while results are checked (§3.5): "5 selected".
pub fn checked_line(n: u64) -> String {
    format!("{} selected", grouped(n))
}

/// Quick Look's title (§3.7, screen 10).
pub const QUICK_LOOK: &str = "Quick Look";

/// Which result Quick Look shows, `at` from 0: "1 of 12".
pub fn quick_look_position(at: u64, of: u64) -> String {
    format!("{} of {}", grouped(at.saturating_add(1)), grouped(of))
}

/// The header's hint that the results still walk: "j/k moves through
/// results while it stays open".
pub fn quick_look_walk(keymap: &Keymap) -> Option<Hint> {
    hints::pair(
        keymap,
        CommandId::NextMessage,
        CommandId::PrevMessage,
        "moves through results while it stays open",
    )
}

/// The header's buttons, each with its key: Open, Archive, Close.
pub fn quick_look_actions(keymap: &Keymap) -> Vec<Hint> {
    [
        (CommandId::OpenMessage, "Open"),
        (CommandId::Archive, "Archive"),
        (CommandId::QuickLook, "Close"),
    ]
    .into_iter()
    .filter_map(|(id, label)| hints::hint(keymap, id, label))
    .collect()
}

/// How many matches the conversation holds: "4 matches in this
/// conversation".
pub fn matches_line(n: usize) -> String {
    match n {
        1 => "1 match in this conversation".to_owned(),
        n => format!("{} matches in this conversation", grouped(n as u64)),
    }
}

/// The hint after it: "]/[ jump between them".
pub fn matches_hint(keymap: &Keymap) -> Option<Hint> {
    hints::pair(
        keymap,
        CommandId::NextMatch,
        CommandId::PrevMatch,
        "jump between them",
    )
}

/// Where a match card's words are, its left column's first line: "Body",
/// "Earlier reply" for quoted history, "Subject", or the file's name.
pub fn match_place(source: &Source) -> String {
    match source {
        Source::Subject => "Subject".to_owned(),
        Source::Body => "Body".to_owned(),
        Source::Quoted => "Earlier reply".to_owned(),
        Source::FileName { name, .. } | Source::FileContent { name, .. } => name.clone(),
    }
}

/// A match card's second line: who said it and when, by first name
/// ("Ada · 26 Sep"); where in a file a file's match is; nothing for the
/// subject or a quote, whose writer is not known.
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

/// When a message was sent, as Quick Look's sender line says it: "Sat 26
/// Sep, 15:51".
pub fn sent_at<Tz: TimeZone>(at: DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    at.format("%a %-d %b, %H:%M").to_string()
}

/// A conversation's size on the sender line, when it is more than one
/// message: "thread of 3".
pub fn thread_size(messages: u32) -> Option<String> {
    (messages > 1).then(|| format!("thread of {messages}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeDelta, Utc};

    fn at(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 15, 30, 0).unwrap()
    }

    // The design's "Search by" grid (screen 01), in its order.
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
        // Wednesday 30 September 2026.
        let now = at(2026, 9, 30);
        assert_eq!(recent_when(now, now), "today");
        assert_eq!(recent_when(now - TimeDelta::days(1), now), "yesterday");
        // Within the week: the weekday, three letters.
        assert_eq!(recent_when(at(2026, 9, 28), now), "Mon");
        assert_eq!(recent_when(now - TimeDelta::days(6), now), "Thu");
        // A week or more: the day and month.
        assert_eq!(recent_when(at(2026, 9, 21), now), "21 Sep");
        assert_eq!(recent_when(at(2026, 9, 23), now), "23 Sep");
        // Another year says so.
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
        let pill = narrow_pill("from:", "Tomás Reyes", 9);
        assert_eq!(
            (pill.op.as_str(), pill.value.as_str(), pill.count.as_str()),
            ("from:", "Tomás Reyes", "9")
        );
        assert_eq!(pill.label(), "from: Tomás Reyes 9");

        let attachment = narrow_pill("has:", "attachment", 12);
        assert_eq!(attachment.label(), "has: attachment 12");

        let big = narrow_pill("label:", "Atlas", 1_204);
        assert_eq!(big.count, "1,204");
    }

    #[test]
    fn a_recent_search_counts_its_results() {
        assert_eq!(results_count(48), "48 results");
        assert_eq!(results_count(1), "1 result");
        assert_eq!(results_count(0), "0 results");
        assert_eq!(results_count(18_204), "18,204 results");
    }

    #[test]
    fn a_top_hit_says_its_day_and_month_and_another_years() {
        let now = at(2026, 9, 26);
        assert_eq!(hit_date(now, now), "26 Sep");
        assert_eq!(hit_date(at(2026, 8, 14), now), "14 Aug");
        // Another year: the list's short form (`row::timestamp`), which
        // fits the 62-pt date column.
        assert_eq!(hit_date(at(2025, 12, 3), now), "3 Dec 25");
    }

    #[test]
    fn a_hits_folder_is_written_as_its_operator() {
        assert_eq!(in_folder("Inbox"), "in:Inbox");
        assert_eq!(in_folder("Q3 close"), "in:\"Q3 close\"");
    }

    #[test]
    fn the_empty_dropdowns_footer_names_its_keys_from_the_keymap() {
        let keymap = postio_core::Keymap::resolve_on(
            &postio_config::KeyBindings::default(),
            postio_config::paths::Platform::Apple,
        );
        let hints = empty_hints(&keymap);
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
                ("Return".to_owned(), "open message".to_owned()),
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

    // -- The results view (spec 010 step 3, design §3, screens 06, 07) --

    use postio_search::results::{Location, RankReason, Source};

    #[test]
    fn a_top_hit_says_why_it_ranked_in_at_most_two_reasons_and_its_matches() {
        use RankReason::*;
        assert_eq!(
            reason_line(&[Replied, Matches(3)]),
            "you replied · 3 matches"
        );
        // D19: the app says flagged, never starred.
        assert_eq!(
            reason_line(&[Flagged, Matches(2)]),
            "you flagged · 2 matches"
        );
        assert_eq!(
            reason_line(&[FrequentSender, InFileName, Matches(1)]),
            "frequent sender · file name"
        );
        assert_eq!(
            reason_line(&[Replied, FrequentSender, InSubject, Matches(4)]),
            "you replied · frequent sender · 4 matches",
            "two reasons at most, and the matches"
        );
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
        // Screen 06's first row: two places, one tag.
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
        // The tag is the passage's source: a subject that also matched is
        // drawn by the row's first line, not named again (screen 06).
        assert_eq!(
            sources_tag(&[Source::Body, Source::Subject, Source::Body]),
            "body"
        );
        assert_eq!(
            sources_tag(&[Source::Quoted, Source::Subject, Source::Quoted]),
            "quoted text"
        );
        // A file's passage is the file alone, whatever else matched.
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
        // No passage yet, or none to cut: still where the body matched, not
        // the subject the row already shows.
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
    fn the_timeline_counts_conversations_and_says_what_else_matched() {
        assert_eq!(count_line(48, false), "48 conversations");
        assert_eq!(count_line(1, false), "1 conversation");
        assert_eq!(count_line(10_000, true), "10,000+ conversations");
        assert_eq!(sub_line(12, 6), "12 files · 6 people · last 12 months");
        assert_eq!(sub_line(1, 1), "1 file · 1 person · last 12 months");
        assert_eq!(sub_line(0, 2), "0 files · 2 people · last 12 months");
    }

    #[test]
    fn the_results_footer_says_how_many_where_and_how_long() {
        let took = |ms| Duration::from_millis(ms);
        assert_eq!(
            results_footer(48, false, took(41)),
            "48 conversations · local index · 41 ms"
        );
        assert_eq!(
            results_footer(10_000, true, took(41)),
            "10,000+ conversations · local index · 41 ms"
        );
        assert_eq!(
            results_footer(1, false, Duration::from_micros(300)),
            "1 conversation · local index · <1 ms"
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
    fn a_result_reads_aloud_as_who_what_where_it_matched_and_when() {
        // Design §5.
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
                ("a", "Archive"),
                ("l", "Label"),
                ("m", "Move"),
                ("r", "Mark read"),
                ("s", "Snooze"),
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

    // The results view's chrome (§3.1-3.5, screen 06): the words the Mac
    // draws around the rows, each the design's.
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

    // Quick Look's words (§3.7, screen 10).
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
                ("Archive".to_owned(), "a".to_owned()),
                ("Close".to_owned(), "space".to_owned()),
            ]
        );
        assert_eq!(matches_line(4), "4 matches in this conversation");
        assert_eq!(matches_line(1), "1 match in this conversation");
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
        assert_eq!(sent_at(now), "Sat 26 Sep, 15:30");
        assert_eq!(thread_size(3), Some("thread of 3".to_owned()));
        assert_eq!(thread_size(1), None);
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
            named("invoices to:ben@example.org"),
            "Invoices to ben@example.org"
        );
        assert_eq!(
            named("has:attachment"),
            "has:attachment",
            "nothing to say it better"
        );
    }
}
