//! The search dropdown: the bar's search half on a frontend that has a
//! results view (`Policy.caps.results_view`; spec 010 step 2, screens 01
//! and 03).
//!
//! Empty, it lists the searches run lately, the pinned saved searches with
//! their counts, and the cheat sheet with one plain-English example lowered
//! live. With words, it lists the top four conversations with their
//! passages, the "Narrow to" pills a facet offers, and "Show all N
//! results", focused. The state is the bar's (`bar.rs`); this is what it is
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

/// Which of the design's states the dropdown is in (§2's table). Step 2
/// draws two of them; the short prefix, an operator, plain English and
/// `>` follow (009's bar draws `>` and `in:` still).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DropdownState {
    /// Nothing typed: recent, saved, the cheat sheet (screen 01).
    Empty,
    /// Words: top hits, Narrow to, Show all (screen 03).
    Words,
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
    fn plain(text: impl Into<String>) -> Self {
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
    /// The key that runs it (`alt+1`), as the keymap spells it.
    pub key: Option<String>,
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
}

/// The dropdown's top hits.
pub(crate) const TOP_HITS: u32 = 4;
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
fn marked(text: &str, ranges: &[std::ops::Range<usize>]) -> Vec<Run> {
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
fn passage(passage: &Passage) -> Vec<Run> {
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
    pub(crate) saved: &'a [(u64, String, Option<u64>)],
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
            .map(|(index, (token, name, count))| DropdownPill {
                token: *token,
                op: None,
                label: name.clone(),
                count: count.map(|count| count.to_string()),
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
                    title: marked(&title, &highlight::find(&title, terms)),
                    detail,
                    folder: landed
                        .folders
                        .iter()
                        .find(|(id, _)| *id == hit.mailbox_id)
                        .map(|(_, name)| words::in_folder(name)),
                    right: Some(words::hit_date(hit.newest_match.with_timezone(&Local), now)),
                    key: None,
                    selectable: true,
                }
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
                    key: None,
                })
                .collect(),
        });
    }
    if landed.total > 0 {
        sections.push(DropdownSection {
            title: String::new(),
            note: None,
            note_key: None,
            rows: vec![DropdownRow {
                token: landed.show_all,
                kind: DropdownRowKind::ShowAll,
                title: vec![Run::styled(
                    words::show_all(landed.total, landed.capped, None),
                    RunStyle::Strong,
                )],
                detail: vec![Run::plain(words::SHOW_ALL_DETAIL)],
                folder: None,
                right: None,
                key: postio_ui::hints::key(keymap, CommandId::ShowAllResults),
                selectable: true,
            }],
            pills: Vec::new(),
        });
    }
    DropdownView {
        state: DropdownState::Words,
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
