//! The search dropdown: the bar's search half. The state is the bar's (`bar.rs`); this is what it
//! is drawn as. Tokens are given when data arrives and kept while it stays, so a redraw (passages
//! landing) leaves the arrows where they were.

use chrono::{DateTime, Local};
use postio_client::protocol::RecentSearch;
use postio_core::{CommandId, Keymap};
use postio_model::{MessageId, ThreadId};
use postio_search::highlight;
use postio_search::query::{Clause, Filter};
use postio_search::results::{
    ConversationHit, ConversationKey, ConversationResults, Counted, Passage,
};
use postio_search::suggest::Completion;
use postio_ui::hints::Hint;
use postio_ui::search_view as words;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DropdownState {
    Empty,
    Words,
    Prefix,
    Operator,
    PlainEnglish,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnderstoodTile {
    pub op: String,
    pub value: String,
    pub origin: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RunStyle {
    #[default]
    Plain,
    Strong,
    Mono,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub highlighted: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DropdownRowKind {
    Recent,
    Hit,
    ShowAll,
    CheatSheet,
    Example,
    Word,
    Label,
    List,
    File,
    Person,
    Folder,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropdownRow {
    pub token: u64,
    pub kind: DropdownRowKind,
    pub title: Vec<Run>,
    pub detail: Vec<Run>,
    pub folder: Option<String>,
    pub right: Option<String>,
    pub key: Option<String>,
    pub selectable: bool,
    pub initials: Option<String>,
    pub highlighted: Option<Hint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropdownPill {
    pub token: u64,
    pub op: Option<String>,
    pub label: String,
    pub count: Option<String>,
    /// A saved search that notifies: its "3 new" badge, while mail has matched since it was last
    /// viewed.
    pub fresh: Option<String>,
    pub key: Option<String>,
}

pub(crate) struct SavedPill {
    pub(crate) token: u64,
    pub(crate) name: String,
    pub(crate) count: Option<Counted>,
    pub(crate) fresh: Option<Counted>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropdownSection {
    pub title: String,
    pub note: Option<String>,
    pub note_key: Option<String>,
    pub rows: Vec<DropdownRow>,
    pub pills: Vec<DropdownPill>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropdownView {
    pub state: DropdownState,
    pub ghost: Option<String>,
    pub ghost_hint: Option<Hint>,
    pub operator: Option<String>,
    pub understood: Vec<UnderstoodTile>,
    pub sections: Vec<DropdownSection>,
    /// Kept by the toolkit while the row it has highlighted is still drawn, taken when it is not.
    pub highlight: Option<u64>,
    /// A run moved the highlight here: taken whatever is highlighted.
    pub select: Option<u64>,
    pub hints: Vec<Hint>,
    pub count: Option<String>,
}

/// Requests of one lane supersede each other, so a frontend may abort the one before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Lane {
    Conversations,
    Passages,
    Matches,
    Relaxations,
    Facets,
    Suggest,
    Latest,
    Files,
    People,
    Events,
}

pub(crate) const TOP_HITS: u32 = 4;
pub(crate) const HITS_SO_FAR: u32 = 3;
pub(crate) const PLAIN_RESULTS: u32 = 3;
pub(crate) const LATEST: u32 = 2;
const RECENTS: usize = 3;
const PILLS: usize = 4;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Action {
    Recent(String),
    Hit(MessageId),
    ShowAll,
    Narrow(Clause),
    Saved(usize),
    Replace(String),
    Pick(Filter),
}

#[derive(Debug, Clone)]
pub(crate) struct Shown {
    pub(crate) token: u64,
    pub(crate) hit: ConversationHit,
}

impl Shown {
    pub(crate) fn thread(&self) -> Option<ThreadId> {
        match self.hit.key {
            ConversationKey::Thread(thread) => Some(thread),
            ConversationKey::Lone(_) => None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Landed {
    pub(crate) hits: Vec<Shown>,
    pub(crate) pills: Vec<(u64, words::NarrowPill, Clause)>,
    pub(crate) show_all: u64,
    pub(crate) total: u64,
    pub(crate) capped: bool,
    pub(crate) elapsed: std::time::Duration,
    pub(crate) wheres: Wheres,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Wheres {
    pub(crate) folders: Vec<(postio_model::MailboxId, String)>,
    pub(crate) held: Vec<(postio_model::MessageId, String, bool)>,
    pub(crate) owners: Vec<(postio_model::MailboxId, String)>,
}

impl Wheres {
    pub(crate) fn place(
        &self,
        message: postio_model::MessageId,
        mailbox: postio_model::MailboxId,
    ) -> Option<String> {
        let held = self
            .held
            .iter()
            .find(|(held, _, _)| *held == message)
            .map(|(_, rule, delivered)| (rule.as_str(), *delivered));
        words::hit_place(
            held,
            named(&self.folders, mailbox),
            named(&self.owners, mailbox),
        )
    }
}

fn named(
    names: &[(postio_model::MailboxId, String)],
    mailbox: postio_model::MailboxId,
) -> Option<&str> {
    names
        .iter()
        .find(|(id, _)| *id == mailbox)
        .map(|(_, name)| name.as_str())
}

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
        .filter(|count| !results.names.own.contains(&count.id))
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
                words::narrow_pill("from:", &said, count.conversations, results.capped),
                positive(Filter::From(person.address.clone())),
            ))
        });
    let mut pills: Vec<_> = senders.by_ref().take(2).collect();
    if narrows(facets.attachment) {
        pills.push((
            words::narrow_pill("has:", "attachment", facets.attachment, results.capped),
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
            words::narrow_pill("label:", name, count.conversations, results.capped),
            positive(Filter::Label(name.clone())),
        ));
    }
    pills.extend(senders);
    pills.truncate(PILLS);
    pills
}

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

/// In characters, before the first match of a hit's passage.
pub(crate) const HIT_LEAD: usize = 16;

/// The row has room for about 45 characters after "Sender · Subject", where a window cut for the
/// results' row would draw only the words before the match.
pub(crate) fn led(cut: &Passage) -> Passage {
    let Some(first) = cut.ranges.first().map(|range| range.start) else {
        return cut.clone();
    };
    let text = &cut.text;
    if !text.is_char_boundary(first) || text[..first].chars().count() <= HIT_LEAD {
        return cut.clone();
    }
    let mut start = text[..first]
        .char_indices()
        .rev()
        .nth(HIT_LEAD - 1)
        .map_or(0, |(at, _)| at);
    if !text[..start].ends_with(char::is_whitespace) {
        start = text[start..first]
            .find(char::is_whitespace)
            .map_or(first, |space| start + space);
    }
    let start = start + (text[start..first].len() - text[start..first].trim_start().len());
    Passage {
        text: text[start..].to_owned(),
        ranges: cut
            .ranges
            .iter()
            .filter(|range| range.start >= start)
            .map(|range| range.start - start..range.end - start)
            .collect(),
        elided_start: true,
        elided_end: cut.elided_end,
    }
}

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

pub(crate) struct EmptyParts<'a> {
    pub(crate) recents: &'a [(u64, RecentSearch)],
    pub(crate) saved: &'a [SavedPill],
    pub(crate) sheet: &'a [u64],
    pub(crate) example: String,
    pub(crate) messages: Option<u64>,
    pub(crate) keymap: &'a Keymap,
    pub(crate) now: DateTime<Local>,
}

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
            detail: vec![Run::plain(words::results_count(recent.hits, recent.capped))],
            folder: None,
            right: Some(words::recent_when(
                recent.last_run_at.with_timezone(&Local),
                parts.now,
            )),
            key: None,
            selectable: true,
            initials: None,
            highlighted: None,
        })
        .collect();
    let highlight = recents.first().map(|row| row.token);
    let any_recent = !recents.is_empty();
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
                count: pill
                    .count
                    .map(|total| words::saved_count(total.conversations, total.capped)),
                fresh: pill
                    .fresh
                    .map(|new| words::new_badge(new.conversations, new.capped)),
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
            highlighted: None,
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
            highlighted: None,
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
        ghost_hint: None,
        operator: None,
        understood: Vec::new(),
        sections,
        highlight,
        select: None,
        hints: words::empty_hints(parts.keymap, any_recent),
        count: parts.messages.map(words::local_index),
    }
}

pub(crate) fn words(
    landed: Option<&Landed>,
    terms: &[String],
    query: Option<&postio_search::ParsedQuery>,
    keymap: &Keymap,
    now: DateTime<Local>,
) -> DropdownView {
    let Some(landed) = landed else {
        return DropdownView {
            state: DropdownState::Words,
            ghost: None,
            ghost_hint: None,
            operator: None,
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
                    &landed.wheres,
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
    } else {
        let terms = query.map_or(1, |query| {
            query.filters().count() + query.text_terms().count()
        });
        let mut section = show_all_section(landed, None, keymap);
        let row = &mut section.rows[0];
        row.title = vec![Run::styled(words::nothing_matches(terms), RunStyle::Strong)];
        row.detail = vec![Run::plain(words::NOTHING_DETAIL)];
        sections.push(section);
    }
    DropdownView {
        state: DropdownState::Words,
        ghost: None,
        ghost_hint: None,
        operator: None,
        understood: Vec::new(),
        sections,
        highlight: Some(landed.show_all),
        select: None,
        hints: words::words_hints(keymap),
        count: Some(words::footer_count(
            landed.total,
            landed.elapsed,
            landed.capped,
        )),
    }
}

pub(crate) fn hit_row(
    shown: &Shown,
    wheres: &Wheres,
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
        .map(|cut| passage(&led(cut)))
        .unwrap_or_default();
    DropdownRow {
        token: shown.token,
        kind: DropdownRowKind::Hit,
        title: marked(&title, &marks(&title)),
        detail,
        folder: wheres.place(hit.best, hit.mailbox_id),
        right: Some(words::hit_date(hit.newest_match.with_timezone(&Local), now)),
        key: None,
        selectable: true,
        initials: None,
        highlighted: None,
    }
}

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
            highlighted: None,
        }],
        pills: Vec::new(),
    }
}

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

#[derive(Debug, Clone)]
pub(crate) enum Offer {
    Word(Completion),
    Label(postio_search::suggest::Completion),
    List(postio_search::suggest::Completion),
    Files(Vec<postio_search::suggest::Completion>),
    Person(postio_search::suggest::Person),
    Folder(postio_search::suggest::Completion),
}

impl Offer {
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

    fn row<Tz: chrono::TimeZone>(
        &self,
        token: u64,
        typed: &str,
        now: DateTime<Tz>,
        first: bool,
        operator: bool,
    ) -> DropdownRow {
        let count = |found: &Completion| Some(words::suggestion_count(found.count, found.capped));
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
            highlighted: None,
        };
        match self {
            Offer::Word(word) => DropdownRow {
                title: marked(&word.text, &prefix_marks(&word.text, typed)),
                detail: vec![Run::plain(words::AS_A_WORD)],
                right: count(word),
                key: first.then(|| "Tab".to_owned()),
                ..base
            },
            Offer::Label(label) if operator => DropdownRow {
                kind: DropdownRowKind::Label,
                title: marked(&label.text, &prefix_marks(&label.text, typed)),
                right: count(label),
                ..base
            },
            Offer::Label(label) => DropdownRow {
                kind: DropdownRowKind::Label,
                title: mono_marked("label:", &label.text, typed),
                detail: vec![Run::plain(words::LABEL)],
                right: count(label),
                ..base
            },
            Offer::List(list) => DropdownRow {
                kind: DropdownRowKind::List,
                title: marked(&list.text, &prefix_marks(&list.text, typed)),
                detail: vec![Run::plain(words::list_detail(&list.text))],
                right: count(list),
                ..base
            },
            Offer::Files(files) => {
                let title = words::files_named(typed);
                let names: Vec<&str> = files.iter().map(|file| file.text.as_str()).collect();
                DropdownRow {
                    kind: DropdownRowKind::File,
                    title: marked(&title, &prefix_marks(&title, typed)),
                    detail: vec![Run::plain(words::files_detail(&names))],
                    right: Some(words::suggestion_count(
                        files.iter().map(|file| file.count).sum(),
                        false,
                    )),
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
            Offer::Folder(folder) => {
                // The inbox reads Inbox, as every search surface says it.
                let name = words::folder_name(&folder.text);
                let query = if name == folder.text {
                    folder.query.clone()
                } else {
                    words::in_folder(&folder.text)
                };
                DropdownRow {
                    kind: DropdownRowKind::Folder,
                    title: marked(&name, &prefix_marks(&name, typed)),
                    detail: vec![Run::styled(query, RunStyle::Mono)],
                    right: count(folder),
                    key: None,
                    ..base
                }
            }
        }
    }
}

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

pub(crate) struct PrefixParts<'a> {
    pub(crate) typed: &'a str,
    pub(crate) ghost: Option<String>,
    pub(crate) offers: &'a [(u64, Offer)],
    pub(crate) landed: Option<&'a Landed>,
    pub(crate) keymap: &'a Keymap,
    pub(crate) now: DateTime<Local>,
}

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
                        &landed.wheres,
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
        ghost_hint: parts.ghost.as_ref().map(|_| words::ghost_hint()),
        operator: None,
        ghost: parts.ghost,
        understood: Vec::new(),
        sections,
        highlight,
        select: None,
        hints: words::prefix_hints(parts.keymap),
        count,
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Latest {
    pub(crate) name: String,
    pub(crate) hits: Vec<Shown>,
    pub(crate) wheres: Wheres,
}

pub(crate) struct OperatorParts<'a> {
    pub(crate) keyword: &'a str,
    pub(crate) value: &'a str,
    pub(crate) offers: &'a [(u64, Offer)],
    pub(crate) latest: Option<&'a Latest>,
    pub(crate) answered: bool,
    pub(crate) keymap: &'a Keymap,
    pub(crate) now: DateTime<Local>,
}

pub(crate) fn operator(parts: OperatorParts<'_>) -> DropdownView {
    let (noun, note) = match parts.keyword {
        "label" => ("Labels", None),
        "in" => ("Folders", None),
        _ => ("People", Some(words::PEOPLE_NOTE.to_owned())),
    };
    let rows: Vec<DropdownRow> = parts
        .offers
        .iter()
        .map(|(token, offer)| DropdownRow {
            highlighted: Some(words::adds_hint()),
            ..offer.row(*token, parts.value, parts.now, false, true)
        })
        .collect();
    let highlight = rows.first().map(|row| row.token);
    let nobody = parts.answered && rows.is_empty();
    let note = if nobody {
        Some(words::nobody_matches(noun).to_owned())
    } else {
        note
    };
    let mut sections = vec![DropdownSection {
        title: words::matching(noun, parts.value),
        note,
        note_key: None,
        rows,
        pills: Vec::new(),
    }];
    if let Some(latest) = parts
        .latest
        .filter(|latest| !nobody && !latest.hits.is_empty())
    {
        sections.push(DropdownSection {
            title: words::latest_from(&latest.name),
            note: Some(words::LATEST_NOTE.to_owned()),
            note_key: None,
            rows: latest
                .hits
                .iter()
                .map(|shown| hit_row(shown, &latest.wheres, &|_| Vec::new(), parts.now))
                .collect(),
            pills: Vec::new(),
        });
    }
    DropdownView {
        state: DropdownState::Operator,
        ghost: None,
        ghost_hint: None,
        operator: Some(format!("{}:", parts.keyword)),
        understood: Vec::new(),
        sections,
        highlight,
        select: None,
        hints: if nobody {
            words::operator_hints_for_nobody()
        } else {
            words::operator_hints(parts.keymap, parts.keyword)
        },
        count: Some(words::operator_source(parts.keyword).to_owned()),
    }
}

pub(crate) struct PlainParts<'a> {
    pub(crate) tiles: Vec<UnderstoodTile>,
    pub(crate) landed: Option<&'a Landed>,
    pub(crate) terms: &'a [String],
    pub(crate) note: String,
    pub(crate) keymap: &'a Keymap,
    pub(crate) now: DateTime<Local>,
}

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
                        &landed.wheres,
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
            parts.keymap,
            landed.total,
            landed.elapsed,
            landed.capped,
        ));
    }
    DropdownView {
        state: DropdownState::PlainEnglish,
        ghost: None,
        ghost_hint: None,
        operator: None,
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
