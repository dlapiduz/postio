//! The search dropdown: the bar's search half on a frontend that has a
//! results view (`Policy.caps.results_view`; spec 010 step 2, screens 01
//! and 03).
//!
//! Empty, it lists the searches run lately, the pinned saved searches with
//! their counts, and the cheat sheet with one plain-English example lowered
//! live. With words, it lists the top four conversations with their
//! passages, the "Narrow to" pills a facet offers, and "Show all N
//! results", focused. Step 8 adds the typing states (screens 02, 04, 05):
//! a short prefix's ghost and suggestions, an operator's people, labels or
//! folders, and the tiles plain English was understood as. The state is the bar's (`bar.rs`); this is what it is
//! drawn as, and the words are `postio_ui::search_view`'s.
//!
//! Tokens are given when the data arrives -- a recent search when the list
//! is read, a hit when its search lands -- and kept while it stays, so a
//! redraw (the passages landing) leaves the arrows where they were.

use chrono::{DateTime, Local};
use postio_client::protocol::RecentSearch;
use postio_core::{CommandId, Keymap};
use postio_model::{MessageId, ThreadId};
use postio_search::highlight;
use postio_search::query::{Clause, Filter};
use postio_search::results::{ConversationHit, ConversationKey, ConversationResults, Passage};
use postio_ui::hints::Hint;
use postio_ui::search_view as words;

/// Which of the design's states the dropdown is in (§2's table). `>`
/// keeps spec 009's lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DropdownState {
    /// Nothing typed: recent, saved, the cheat sheet (screen 01).
    Empty,
    /// Words: top hits, Narrow to, Show all (screen 03).
    Words,
    /// One to three letters: the ghost, suggestions, top hits so far,
    /// Show all (screen 02).
    Prefix,
    /// An operator's value being typed: `from:`/`to:` list people,
    /// `label:` labels, `in:` folders (screen 04).
    Operator,
    /// A sentence lowered into operators: "Understood as", the results,
    /// Show all (screen 05).
    PlainEnglish,
}

/// One tile of the "Understood as" bar: a term the sentence became, and
/// the words it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnderstoodTile {
    /// The operator with its colon, tertiary ("from:"), or empty for a word.
    pub op: String,
    /// What it is set to ("2026-08-01"), or the word.
    pub value: String,
    /// "from ‘last month’", the sentence's own words.
    pub origin: String,
}

/// How a run of a row's words is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RunStyle {
    /// The row's own face.
    #[default]
    Plain,
    /// Bold: the example sentence, the Show all count.
    Strong,
    /// Monospaced: a query, an operator.
    Mono,
}

/// A stretch of a row's words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    /// The words.
    pub text: String,
    /// The engine found the query's words here: the find highlight.
    pub highlighted: bool,
    /// How it is set.
    pub style: RunStyle,
}

impl Run {
    pub(crate) fn plain(text: impl Into<String>) -> Self {
        Run::styled(text, RunStyle::Plain)
    }

    fn styled(text: impl Into<String>, style: RunStyle) -> Self {
        Run {
            text: text.into(),
            highlighted: false,
            style,
        }
    }
}

/// What a dropdown row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DropdownRowKind {
    /// A search run lately: ↩ runs it again, ⌥⌫ forgets it.
    Recent,
    /// A conversation: ↩ opens its best message.
    Hit,
    /// "Show all N results".
    ShowAll,
    /// An operator and what it is for; runs nothing.
    CheatSheet,
    /// The plain-English example and what it becomes; runs nothing.
    Example,
    /// A completed word: ↩ (or Tab) puts it in the field.
    Word,
    /// A label: ↩ makes its chip, ⌥↩ in the operator state its exclusion.
    Label,
    /// A mailing list: ↩ makes its chip.
    List,
    /// Files whose names match: ↩ makes a `filename:` chip.
    File,
    /// A person: ↩ makes the `from:`/`to:` chip, ⌥↩ its exclusion.
    Person,
    /// A folder: ↩ makes the `in:` chip, ⌥↩ its exclusion.
    Folder,
}

/// One row of the dropdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropdownRow {
    /// What [`Input::BarRun`](crate::Input::BarRun) hands back to run it.
    pub token: u64,
    /// What it is.
    pub kind: DropdownRowKind,
    /// Its title: a query, "Sender · Subject".
    pub title: Vec<Run>,
    /// After the title: a count, a passage, a hint.
    pub detail: Vec<Run>,
    /// The folder column: `in:Inbox`.
    pub folder: Option<String>,
    /// The right column: a date, "yesterday".
    pub right: Option<String>,
    /// The key that runs it, as the keymap spells it.
    pub key: Option<String>,
    /// Whether the arrows may rest on it.
    pub selectable: bool,
    /// A person's initials, for the round avatar.
    pub initials: Option<String>,
}

/// A pill: a saved search, or a filter to narrow to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropdownPill {
    /// What [`Input::BarRun`](crate::Input::BarRun) hands back to run it.
    pub token: u64,
    /// The operator, drawn tertiary and monospaced: `from:`.
    pub op: Option<String>,
    /// The name, or the value.
    pub label: String,
    /// How many it holds.
    pub count: Option<String>,
    /// A saved search that notifies: its quiet badge, "3 new", while mail
    /// has matched since it was last viewed (D15).
    pub fresh: Option<String>,
    /// The key that runs it (`alt+1`), as the keymap spells it.
    pub key: Option<String>,
}

/// A saved search's pill, as the empty dropdown is drawn from.
pub(crate) struct SavedPill {
    pub(crate) token: u64,
    pub(crate) name: String,
    pub(crate) count: Option<u64>,
    pub(crate) fresh: Option<u64>,
}

/// One section: its title, its note, and its rows or pills.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropdownSection {
    /// The bold secondary title; empty for the Show all row's.
    pub title: String,
    /// The tertiary note on the right.
    pub note: Option<String>,
    /// The key the note names first: `alt+BackSpace` before "forgets one".
    pub note_key: Option<String>,
    /// Its rows.
    pub rows: Vec<DropdownRow>,
    /// Its pills, drawn in a line after the title.
    pub pills: Vec<DropdownPill>,
}

/// The dropdown, whole: what [`Intent::Dropdown`](crate::Intent::Dropdown)
/// draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropdownView {
    /// Which state it is in.
    pub state: DropdownState,
    /// The rest of the best word, drawn tertiary after the caret ("las"
    /// after "at"): Tab takes it.
    pub ghost: Option<String>,
    /// The "Understood as" bar's tiles, in plain English.
    pub understood: Vec<UnderstoodTile>,
    /// Top to bottom.
    pub sections: Vec<DropdownSection>,
    /// The row focused by default: kept by the toolkit while the row it
    /// has highlighted is still drawn, taken when it is not.
    pub highlight: Option<u64>,
    /// A run moved the highlight here: taken whatever is highlighted.
    pub select: Option<u64>,
    /// The footer's keys.
    pub hints: Vec<Hint>,
    /// The footer's right side: "48 matches · 38 ms".
    pub count: Option<String>,
}

/// A search lane: requests of one lane supersede each other, so a frontend
/// may abort the one before when the next is asked (spec 010 D8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Lane {
    /// The conversation search the panel is drawn from.
    Conversations,
    /// The passages of the hits on screen.
    Passages,
    /// The matches of the conversation Quick Look shows: j supersedes them.
    Matches,
    /// The counts of a search that found nothing's ways out: slow, and
    /// waste once the query changes.
    Relaxations,
    /// What a prefix could become: each keystroke supersedes the last.
    Suggest,
    /// The latest from the person the arrows rest on: a move supersedes it.
    Latest,
}

/// The dropdown's top hits.
pub(crate) const TOP_HITS: u32 = 4;
/// The short-prefix state's top hits so far (screen 02).
pub(crate) const HITS_SO_FAR: u32 = 3;
/// The plain-English state's results (screen 05).
pub(crate) const PLAIN_RESULTS: u32 = 3;
/// "Latest from" the focused person (screen 04).
pub(crate) const LATEST: u32 = 2;
/// Recent searches the empty dropdown lists.
const RECENTS: usize = 3;
/// "Narrow to" pills, at most.
const PILLS: usize = 4;

/// What a dropdown token runs.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Action {
    /// Put this query in the field.
    Recent(String),
    /// Open this message.
    Hit(MessageId),
    /// Show every result.
    ShowAll,
    /// Add this clause to the query.
    Narrow(Clause),
    /// Run the saved search at this index.
    Saved(usize),
    /// Put this in place of what is being typed: a word, or a query.
    Replace(String),
    /// Make this operator's chip in place of what is being typed: ↩, or
    /// ⌥↩ for its exclusion.
    Pick(Filter),
}

/// A conversation on screen, with its token.
#[derive(Debug, Clone)]
pub(crate) struct Shown {
    pub(crate) token: u64,
    pub(crate) hit: ConversationHit,
}

impl Shown {
    /// The thread the hit walks within, when it is one.
    pub(crate) fn thread(&self) -> Option<ThreadId> {
        match self.hit.key {
            ConversationKey::Thread(thread) => Some(thread),
            ConversationKey::Lone(_) => None,
        }
    }
}

/// What the words found, with the tokens it is drawn under.
#[derive(Debug, Clone)]
pub(crate) struct Landed {
    pub(crate) hits: Vec<Shown>,
    pub(crate) pills: Vec<(u64, words::NarrowPill, Clause)>,
    pub(crate) show_all: u64,
    pub(crate) total: u64,
    pub(crate) capped: bool,
    pub(crate) elapsed: std::time::Duration,
    pub(crate) folders: Vec<(postio_model::MailboxId, String)>,
}

/// The pills a search's facets offer: the top senders, attachments, the
/// top label, each only when it would keep fewer than all (it narrows),
/// four at most.
pub(crate) fn narrow_pills(results: &ConversationResults) -> Vec<(words::NarrowPill, Clause)> {
    let facets = &results.facets;
    let narrows = |count: u64| count > 0 && count < results.total;
    let positive = |filter| Clause {
        negated: false,
        filter,
    };
    let mut senders = facets
        .senders
        .iter()
        .filter(|count| narrows(count.conversations))
        .filter_map(|count| {
            let (_, person) = results
                .names
                .people
                .iter()
                .find(|(id, _)| *id == count.id)?;
            let said = person
                .name
                .clone()
                .unwrap_or_else(|| person.address.clone());
            Some((
                words::narrow_pill("from:", &said, count.conversations),
                positive(Filter::From(person.address.clone())),
            ))
        });
    let mut pills: Vec<_> = senders.by_ref().take(2).collect();
    if narrows(facets.attachment) {
        pills.push((
            words::narrow_pill("has:", "attachment", facets.attachment),
            positive(Filter::HasAttachment),
        ));
    }
    if let Some((count, name)) = facets
        .labels
        .iter()
        .filter(|count| narrows(count.conversations))
        .find_map(|count| {
            let (_, name) = results
                .names
                .labels
                .iter()
                .find(|(id, _)| *id == count.id)?;
            Some((count, name))
        })
    {
        pills.push((
            words::narrow_pill("label:", name, count.conversations),
            positive(Filter::Label(name.clone())),
        ));
    }
    pills.extend(senders);
    pills.truncate(PILLS);
    pills
}

/// `text` as runs, the stretches `ranges` names highlighted.
pub(crate) fn marked(text: &str, ranges: &[std::ops::Range<usize>]) -> Vec<Run> {
    let mut runs = Vec::new();
    let mut at = 0;
    for range in ranges {
        let (start, end) = (range.start.max(at), range.end.min(text.len()));
        if start >= end || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            continue;
        }
        if start > at {
            runs.push(Run::plain(&text[at..start]));
        }
        runs.push(Run {
            text: text[start..end].to_owned(),
            highlighted: true,
            style: RunStyle::Plain,
        });
        at = end;
    }
    if at < text.len() {
        runs.push(Run::plain(&text[at..]));
    }
    runs
}

/// A passage as runs, with its ellipses.
pub(crate) fn passage(passage: &Passage) -> Vec<Run> {
    let mut runs = Vec::new();
    if passage.elided_start {
        runs.push(Run::plain("\u{2026}"));
    }
    runs.extend(marked(&passage.text, &passage.ranges));
    if passage.elided_end {
        runs.push(Run::plain("\u{2026}"));
    }
    runs
}

/// A query as a recent row draws it: words as words, a query as one.
fn query_runs(query: &str) -> Vec<Run> {
    let operators = query
        .split_whitespace()
        .any(|word| word.contains(':') || word.starts_with('-'));
    let style = if operators {
        RunStyle::Mono
    } else {
        RunStyle::Plain
    };
    vec![Run::styled(query, style)]
}

/// What the empty dropdown is drawn from.
pub(crate) struct EmptyParts<'a> {
    pub(crate) recents: &'a [(u64, RecentSearch)],
    pub(crate) saved: &'a [SavedPill],
    pub(crate) sheet: &'a [u64],
    pub(crate) example: String,
    pub(crate) keymap: &'a Keymap,
    pub(crate) now: DateTime<Local>,
}

/// The empty dropdown (screen 01).
pub(crate) fn empty(parts: EmptyParts<'_>) -> DropdownView {
    let mut sections = Vec::new();
    let recents: Vec<DropdownRow> = parts
        .recents
        .iter()
        .take(RECENTS)
        .map(|(token, recent)| DropdownRow {
            token: *token,
            kind: DropdownRowKind::Recent,
            title: query_runs(&recent.query),
            detail: vec![Run::plain(words::results_count(recent.hits))],
            folder: None,
            right: Some(words::recent_when(
                recent.last_run_at.with_timezone(&Local),
                parts.now,
            )),
            key: None,
            selectable: true,
            initials: None,
        })
        .collect();
    let highlight = recents.first().map(|row| row.token);
    if !recents.is_empty() {
        sections.push(DropdownSection {
            title: words::RECENT.to_owned(),
            note: Some(words::FORGETS_ONE.to_owned()),
            note_key: postio_ui::hints::key(parts.keymap, CommandId::ForgetRecent),
            rows: recents,
            pills: Vec::new(),
        });
    }
    if !parts.saved.is_empty() {
        let pills = parts
            .saved
            .iter()
            .enumerate()
            .map(|(index, pill)| DropdownPill {
                token: pill.token,
                op: None,
                label: pill.name.clone(),
                count: pill.count.map(|count| count.to_string()),
                fresh: pill.fresh.map(words::new_badge),
                key: postio_ui::command_bar::SAVED
                    .get(index)
                    .and_then(|id| postio_ui::hints::key(parts.keymap, *id)),
            })
            .collect();
        sections.push(DropdownSection {
            title: words::SAVED_SEARCHES.to_owned(),
            note: None,
            note_key: None,
            rows: Vec::new(),
            pills,
        });
    }
    let mut sheet: Vec<DropdownRow> = words::cheat_sheet()
        .into_iter()
        .zip(parts.sheet)
        .map(|((op, hint), token)| DropdownRow {
            token: *token,
            kind: DropdownRowKind::CheatSheet,
            title: vec![Run::styled(op, RunStyle::Mono)],
            detail: vec![Run::plain(hint)],
            folder: None,
            right: None,
            key: None,
            selectable: false,
            initials: None,
        })
        .collect();
    if let Some(token) = parts.sheet.get(words::cheat_sheet().len()) {
        sheet.push(DropdownRow {
            token: *token,
            kind: DropdownRowKind::Example,
            title: vec![
                Run::plain(format!("{} ", words::JUST_TYPE_IT)),
                Run::styled(words::EXAMPLE, RunStyle::Strong),
            ],
            detail: vec![
                Run::plain(format!("{} ", words::BECOMES)),
                Run::styled(parts.example, RunStyle::Mono),
            ],
            folder: None,
            right: None,
            key: None,
            selectable: false,
            initials: None,
        });
    }
    sections.push(DropdownSection {
        title: words::SEARCH_BY.to_owned(),
        note: None,
        note_key: None,
        rows: sheet,
        pills: Vec::new(),
    });
    DropdownView {
        state: DropdownState::Empty,
        ghost: None,
        understood: Vec::new(),
        sections,
        highlight,
        select: None,
        hints: words::empty_hints(parts.keymap),
        count: None,
    }
}

/// The words dropdown (screen 03), from what the words found, `terms`
/// being the query's words for the highlight.
pub(crate) fn words(
    landed: Option<&Landed>,
    terms: &[String],
    keymap: &Keymap,
    now: DateTime<Local>,
) -> DropdownView {
    let Some(landed) = landed else {
        return DropdownView {
            state: DropdownState::Words,
            ghost: None,
            understood: Vec::new(),
            sections: Vec::new(),
            highlight: None,
            select: None,
            hints: words::words_hints(keymap),
            count: None,
        };
    };
    let mut sections = Vec::new();
    if !landed.hits.is_empty() {
        let rows = landed
            .hits
            .iter()
            .map(|shown| {
                hit_row(
                    shown,
                    &landed.folders,
                    &|title| highlight::find(title, terms),
                    now,
                )
            })
            .collect();
        sections.push(DropdownSection {
            title: words::TOP_HITS.to_owned(),
            note: Some(words::TOP_HITS_NOTE.to_owned()),
            note_key: None,
            rows,
            pills: Vec::new(),
        });
    }
    if !landed.pills.is_empty() {
        sections.push(DropdownSection {
            title: words::NARROW_TO.to_owned(),
            note: None,
            note_key: None,
            rows: Vec::new(),
            pills: landed
                .pills
                .iter()
                .map(|(token, pill, _)| DropdownPill {
                    token: *token,
                    op: Some(pill.op.clone()),
                    label: pill.value.clone(),
                    count: Some(pill.count.clone()),
                    fresh: None,
                    key: None,
                })
                .collect(),
        });
    }
    if landed.total > 0 {
        sections.push(show_all_section(landed, None, keymap));
    }
    DropdownView {
        state: DropdownState::Words,
        ghost: None,
        understood: Vec::new(),
        sections,
        highlight: (landed.total > 0).then_some(landed.show_all),
        select: None,
        hints: words::words_hints(keymap),
        count: Some(words::footer_count(
            landed.total,
            landed.elapsed,
            landed.capped,
        )),
    }
}

/// A conversation's row: "Sender · Subject" with `marks` highlighted, its
/// passage, folder and date.
pub(crate) fn hit_row(
    shown: &Shown,
    folders: &[(postio_model::MailboxId, String)],
    marks: &dyn Fn(&str) -> Vec<std::ops::Range<usize>>,
    now: DateTime<Local>,
) -> DropdownRow {
    let hit = &shown.hit;
    let subject = hit.subject.clone().unwrap_or_default();
    let title = match &hit.from {
        Some(from) => {
            format!("{} \u{b7} {subject}", postio_ui::command_bar::said_of(from))
        }
        None => subject,
    };
    let detail = hit
        .matches
        .iter()
        .find_map(|each| each.passage.as_ref())
        .map(passage)
        .unwrap_or_default();
    DropdownRow {
        token: shown.token,
        kind: DropdownRowKind::Hit,
        title: marked(&title, &marks(&title)),
        detail,
        folder: folders
            .iter()
            .find(|(id, _)| *id == hit.mailbox_id)
            .map(|(_, name)| words::in_folder(name)),
        right: Some(words::hit_date(hit.newest_match.with_timezone(&Local), now)),
        key: None,
        selectable: true,
        initials: None,
    }
}

/// The "Show all N results" row's section, `typed` naming what while a
/// short prefix is typed ("for “at”").
fn show_all_section(landed: &Landed, typed: Option<&str>, keymap: &Keymap) -> DropdownSection {
    DropdownSection {
        title: String::new(),
        note: None,
        note_key: None,
        rows: vec![DropdownRow {
            token: landed.show_all,
            kind: DropdownRowKind::ShowAll,
            title: vec![Run::styled(
                words::show_all(landed.total, landed.capped, typed),
                RunStyle::Strong,
            )],
            detail: vec![Run::plain(words::SHOW_ALL_DETAIL)],
            folder: None,
            right: None,
            key: postio_ui::hints::key(keymap, CommandId::ShowAllResults),
            selectable: true,
            initials: None,
        }],
        pills: Vec::new(),
    }
}

/// Where in `text` a word begins with `prefix`, case aside: the find
/// highlight while a word is half typed ("[At]las").
pub(crate) fn prefix_marks(text: &str, prefix: &str) -> Vec<std::ops::Range<usize>> {
    let prefix: Vec<char> = prefix.chars().flat_map(char::to_lowercase).collect();
    if prefix.is_empty() {
        return Vec::new();
    }
    let mut marks = Vec::new();
    let mut previous: Option<char> = None;
    for (at, ch) in text.char_indices() {
        let starts = ch.is_alphanumeric() && !previous.is_some_and(char::is_alphanumeric);
        previous = Some(ch);
        if !starts {
            continue;
        }
        let mut end = at;
        let mut matched = 0;
        for (offset, each) in text[at..].char_indices() {
            if matched == prefix.len() {
                break;
            }
            let mut lowered = each.to_lowercase();
            if lowered.next() != Some(prefix[matched]) || lowered.next().is_some() {
                break;
            }
            matched += 1;
            end = at + offset + each.len_utf8();
        }
        if matched == prefix.len() {
            marks.push(at..end);
        }
    }
    marks
}

/// What a suggestion row is, as the answer gave it.
#[derive(Debug, Clone)]
pub(crate) enum Offer {
    /// A word that completes the prefix.
    Word(postio_search::suggest::Completion),
    /// A label.
    Label(postio_search::suggest::Completion),
    /// A mailing list.
    List(postio_search::suggest::Completion),
    /// The files whose names begin a word so, as one row.
    Files(Vec<postio_search::suggest::Completion>),
    /// A person, for `from:` or `to:`.
    Person(postio_search::suggest::Person),
    /// A folder, for `in:`.
    Folder(postio_search::suggest::Completion),
}

impl Offer {
    /// What ↩ on it does, `field` being the operator typed.
    pub(crate) fn action(&self, field: Option<postio_search::query::Field>) -> Option<Action> {
        use postio_search::query::Field;
        Some(match (self, field) {
            (Offer::Word(word), _) => Action::Replace(word.text.clone()),
            (Offer::Label(label), Some(Field::Label)) => {
                Action::Pick(Filter::Label(label.text.clone()))
            }
            (Offer::Label(found) | Offer::List(found), _) => {
                Action::Replace(format!("{} ", found.query))
            }
            (Offer::Files(files), _) => Action::Replace(format!("{} ", files.first()?.query)),
            (Offer::Person(person), Some(Field::To)) => {
                Action::Pick(Filter::To(person.address.clone()))
            }
            (Offer::Person(person), _) => Action::Pick(Filter::From(person.address.clone())),
            (Offer::Folder(folder), _) => Action::Pick(Filter::In(folder.text.clone())),
        })
    }

    /// The filter ⌥↩ excludes: a person's, a label's or a folder's.
    pub(crate) fn excluded(&self, field: Option<postio_search::query::Field>) -> Option<Filter> {
        use postio_search::query::Field;
        match (self, field) {
            (Offer::Person(person), Some(Field::To)) => Some(Filter::To(person.address.clone())),
            (Offer::Person(person), _) => Some(Filter::From(person.address.clone())),
            (Offer::Label(label), _) => Some(Filter::Label(label.text.clone())),
            (Offer::Folder(folder), _) => Some(Filter::In(folder.text.clone())),
            _ => None,
        }
    }

    /// Its row, `typed` being what is highlighted in it.
    fn row<Tz: chrono::TimeZone>(
        &self,
        token: u64,
        typed: &str,
        now: DateTime<Tz>,
        first: bool,
        operator: bool,
    ) -> DropdownRow {
        let count = |n: u64| Some(n.to_string());
        let base = DropdownRow {
            token,
            kind: DropdownRowKind::Word,
            title: Vec::new(),
            detail: Vec::new(),
            folder: None,
            right: None,
            key: None,
            selectable: true,
            initials: None,
        };
        match self {
            Offer::Word(word) => DropdownRow {
                title: marked(&word.text, &prefix_marks(&word.text, typed)),
                detail: vec![Run::plain(words::AS_A_WORD)],
                right: count(word.count),
                // Tab takes the first word: the ghost's.
                key: first.then(|| "Tab".to_owned()),
                ..base
            },
            // `label:` typed: the names alone, the operator being in the field.
            Offer::Label(label) if operator => DropdownRow {
                kind: DropdownRowKind::Label,
                title: marked(&label.text, &prefix_marks(&label.text, typed)),
                right: count(label.count),
                ..base
            },
            Offer::Label(label) => DropdownRow {
                kind: DropdownRowKind::Label,
                title: mono_marked("label:", &label.text, typed),
                detail: vec![Run::plain(words::LABEL)],
                right: count(label.count),
                ..base
            },
            Offer::List(list) => DropdownRow {
                kind: DropdownRowKind::List,
                title: marked(&list.text, &prefix_marks(&list.text, typed)),
                detail: vec![Run::plain(words::list_detail(&list.text))],
                right: count(list.count),
                ..base
            },
            Offer::Files(files) => {
                let title = words::files_named(typed);
                let names: Vec<&str> = files.iter().map(|file| file.text.as_str()).collect();
                DropdownRow {
                    kind: DropdownRowKind::File,
                    title: marked(&title, &prefix_marks(&title, typed)),
                    detail: vec![Run::plain(words::files_detail(&names))],
                    right: count(files.iter().map(|file| file.count).sum()),
                    ..base
                }
            }
            Offer::Person(person) => {
                let address =
                    postio_model::EmailAddress::new(person.name.as_deref(), &person.address);
                let said = person
                    .name
                    .clone()
                    .unwrap_or_else(|| person.address.clone());
                DropdownRow {
                    kind: DropdownRowKind::Person,
                    title: vec![Run::styled(said, RunStyle::Strong)],
                    detail: vec![Run::styled(
                        words::person_detail(
                            &person.address,
                            person.received + person.sent,
                            person.last.map(|last| last.with_timezone(&now.timezone())),
                            now.clone(),
                        ),
                        RunStyle::Mono,
                    )],
                    initials: Some(postio_ui::row::initials(Some(&address))),
                    ..base
                }
            }
            Offer::Folder(folder) => DropdownRow {
                kind: DropdownRowKind::Folder,
                title: marked(&folder.text, &prefix_marks(&folder.text, typed)),
                detail: vec![Run::styled(folder.query.clone(), RunStyle::Mono)],
                right: count(folder.count),
                key: None,
                ..base
            },
        }
    }
}

/// `op` and `value` in mono, `typed` highlighted where a word of the value
/// begins with it: `label:[At]las`.
fn mono_marked(op: &str, value: &str, typed: &str) -> Vec<Run> {
    let mut runs = vec![Run::styled(op, RunStyle::Mono)];
    runs.extend(
        marked(value, &prefix_marks(value, typed))
            .into_iter()
            .map(|run| Run {
                style: RunStyle::Mono,
                ..run
            }),
    );
    runs
}

/// The offers a suggestion answer is drawn as, in the design's order: the
/// word, the labels, the lists, the files (as one row), the folders, the
/// people.
pub(crate) fn offers(found: &postio_search::suggest::Suggestions) -> Vec<Offer> {
    let mut offers: Vec<Offer> = found.words.iter().cloned().map(Offer::Word).collect();
    offers.extend(found.labels.iter().cloned().map(Offer::Label));
    offers.extend(found.lists.iter().cloned().map(Offer::List));
    if !found.files.is_empty() {
        offers.push(Offer::Files(found.files.clone()));
    }
    offers.extend(found.folders.iter().cloned().map(Offer::Folder));
    offers.extend(found.people.iter().cloned().map(Offer::Person));
    offers
}

/// What the short-prefix dropdown is drawn from.
pub(crate) struct PrefixParts<'a> {
    pub(crate) typed: &'a str,
    pub(crate) ghost: Option<String>,
    pub(crate) offers: &'a [(u64, Offer)],
    pub(crate) landed: Option<&'a Landed>,
    pub(crate) keymap: &'a Keymap,
    pub(crate) now: DateTime<Local>,
}

/// The short-prefix dropdown (screen 02).
pub(crate) fn prefix(parts: PrefixParts<'_>) -> DropdownView {
    let mut sections = Vec::new();
    let rows: Vec<DropdownRow> = parts
        .offers
        .iter()
        .enumerate()
        .map(|(at, (token, offer))| offer.row(*token, parts.typed, parts.now, at == 0, false))
        .collect();
    let mut highlight = rows.first().map(|row| row.token);
    if !rows.is_empty() {
        sections.push(DropdownSection {
            title: words::SUGGESTIONS.to_owned(),
            note: Some(words::SUGGESTIONS_NOTE.to_owned()),
            note_key: None,
            rows,
            pills: Vec::new(),
        });
    }
    let mut count = None;
    if let Some(landed) = parts.landed {
        if !landed.hits.is_empty() {
            let rows: Vec<DropdownRow> = landed
                .hits
                .iter()
                .map(|shown| {
                    hit_row(
                        shown,
                        &landed.folders,
                        &|title| prefix_marks(title, parts.typed),
                        parts.now,
                    )
                })
                .collect();
            highlight = highlight.or_else(|| rows.first().map(|row| row.token));
            sections.push(DropdownSection {
                title: words::TOP_HITS_SO_FAR.to_owned(),
                note: Some(words::TOP_HITS_SO_FAR_NOTE.to_owned()),
                note_key: None,
                rows,
                pills: Vec::new(),
            });
        }
        if landed.total > 0 {
            sections.push(show_all_section(landed, Some(parts.typed), parts.keymap));
            highlight = highlight.or(Some(landed.show_all));
        }
        count = Some(words::footer_count(
            landed.total,
            landed.elapsed,
            landed.capped,
        ));
    }
    DropdownView {
        state: DropdownState::Prefix,
        ghost: parts.ghost,
        understood: Vec::new(),
        sections,
        highlight,
        select: None,
        hints: words::prefix_hints(parts.keymap),
        count,
    }
}

/// The latest from the focused person, as it landed.
#[derive(Debug, Clone)]
pub(crate) struct Latest {
    pub(crate) name: String,
    pub(crate) hits: Vec<Shown>,
    pub(crate) folders: Vec<(postio_model::MailboxId, String)>,
}

/// What the operator dropdown is drawn from.
pub(crate) struct OperatorParts<'a> {
    pub(crate) keyword: &'a str,
    pub(crate) value: &'a str,
    pub(crate) offers: &'a [(u64, Offer)],
    pub(crate) latest: Option<&'a Latest>,
    pub(crate) keymap: &'a Keymap,
    pub(crate) now: DateTime<Local>,
}

/// The operator dropdown (screen 04): people, labels or folders, and the
/// latest from the focused person.
pub(crate) fn operator(parts: OperatorParts<'_>) -> DropdownView {
    let (noun, note) = match parts.keyword {
        "label" => ("Labels", None),
        "in" => ("Folders", None),
        _ => ("People", Some(words::PEOPLE_NOTE.to_owned())),
    };
    let rows: Vec<DropdownRow> = parts
        .offers
        .iter()
        .map(|(token, offer)| offer.row(*token, parts.value, parts.now, false, true))
        .collect();
    let highlight = rows.first().map(|row| row.token);
    let mut sections = vec![DropdownSection {
        title: words::matching(noun, parts.value),
        note,
        note_key: None,
        rows,
        pills: Vec::new(),
    }];
    if let Some(latest) = parts.latest.filter(|latest| !latest.hits.is_empty()) {
        sections.push(DropdownSection {
            title: words::latest_from(&latest.name),
            note: Some(words::LATEST_NOTE.to_owned()),
            note_key: None,
            rows: latest
                .hits
                .iter()
                .map(|shown| hit_row(shown, &latest.folders, &|_| Vec::new(), parts.now))
                .collect(),
            pills: Vec::new(),
        });
    }
    DropdownView {
        state: DropdownState::Operator,
        ghost: None,
        understood: Vec::new(),
        sections,
        highlight,
        select: None,
        hints: words::operator_hints(parts.keymap, parts.keyword),
        count: Some(words::operator_source(parts.keyword).to_owned()),
    }
}

/// What the plain-English dropdown is drawn from.
pub(crate) struct PlainParts<'a> {
    pub(crate) tiles: Vec<UnderstoodTile>,
    pub(crate) landed: Option<&'a Landed>,
    pub(crate) terms: &'a [String],
    pub(crate) note: String,
    pub(crate) keymap: &'a Keymap,
    pub(crate) now: DateTime<Local>,
}

/// The plain-English dropdown (screen 05): what it understood, then the
/// results and Show all.
pub(crate) fn plain(parts: PlainParts<'_>) -> DropdownView {
    let mut sections = Vec::new();
    let mut highlight = None;
    let mut count = None;
    if let Some(landed) = parts.landed {
        if !landed.hits.is_empty() {
            let rows: Vec<DropdownRow> = landed
                .hits
                .iter()
                .map(|shown| {
                    hit_row(
                        shown,
                        &landed.folders,
                        &|title| highlight::find(title, parts.terms),
                        parts.now,
                    )
                })
                .collect();
            highlight = rows.first().map(|row| row.token);
            sections.push(DropdownSection {
                title: words::RESULTS.to_owned(),
                note: Some(parts.note),
                note_key: None,
                rows,
                pills: Vec::new(),
            });
        }
        if landed.total > 0 {
            sections.push(show_all_section(landed, None, parts.keymap));
            highlight = highlight.or(Some(landed.show_all));
        }
        count = Some(words::parsed_footer(
            landed.total,
            landed.elapsed,
            landed.capped,
        ));
    }
    DropdownView {
        state: DropdownState::PlainEnglish,
        ghost: None,
        understood: parts.tiles,
        sections,
        highlight,
        select: None,
        hints: words::plain_hints(parts.keymap),
        count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marked_runs_cover_the_text_and_skip_ranges_off_a_char_boundary() {
        let runs = marked("caf\u{e9} budget", &[0..2, 3..4, 6..12]);
        let text: String = runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(text, "caf\u{e9} budget");
        let lit: Vec<&str> = runs
            .iter()
            .filter(|run| run.highlighted)
            .map(|run| run.text.as_str())
            .collect();
        assert_eq!(lit, ["ca", "budget"], "3..4 splits the é and is skipped");
    }
}
