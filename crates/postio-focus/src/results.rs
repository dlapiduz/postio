//! The results view: the main window as a search's results, a mode of it and not a surface over
//! it. It holds the query as the field has it, edited only through `postio_search::edit`, and a
//! window over the conversations it matches: Top hits, then every conversation newest first by
//! the month of its newest match. A page of fifty is read when wanted, its passages after.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use chrono::{DateTime, Local, NaiveDate};
use postio_core::{CommandId, Keymap};
use postio_model::{MailboxId, MessageId, ThreadId, ThreadView};
use postio_search::ParsedQuery;
use postio_search::facets::SearchFacets;
use postio_search::query::{Clause, Filter, TokenKind};
use postio_search::relax::{Loosen, Relaxation};
use postio_search::results::{
    ConversationHit, ConversationKey, ConversationMatch, ConversationOrder, ConversationResults,
    Counted, FacetNames, FileHit, Location, Match, RankReason, ResultsTab, Source,
};
use postio_ui::hints::Hint;
use postio_ui::search_view::{self as words, FilterKind, ResultChild};

use crate::dropdown::{self, Run};
use crate::feed::Step;
use crate::history::{Entry, Snapshot};
use crate::{Effect, FocusController, Input, Intent, Reply, Request};

const TOP_HITS: u32 = 3;
pub(crate) const PAGE: u32 = 50;
const RESIDENT: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultsTabView {
    pub tab: ResultsTab,
    pub label: String,
    pub count: String,
    pub selected: bool,
    pub key: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MonthBar {
    pub label: String,
    pub conversations: u64,
    /// "9+" when the search was capped and every count is a floor.
    pub count: String,
    pub height: f64,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultGroup {
    pub title: String,
    pub count: String,
    pub note: Option<String>,
    pub first: u64,
    pub rows: u64,
    pub top_hits: bool,
    pub accessible: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResultsView {
    pub tabs: Vec<ResultsTabView>,
    pub order: ConversationOrder,
    pub sortable: bool,
    pub count_line: String,
    pub sub_line: String,
    pub months: Vec<MonthBar>,
    pub timeline_hint: String,
    pub timeline_step: Option<Hint>,
    pub groups: Vec<ResultGroup>,
    pub rows: u64,
    pub cursor: Option<u64>,
    pub hints: Vec<Hint>,
    pub footer: String,
    pub selected: u64,
    pub bulk: Vec<Hint>,
    pub select_all: Option<Hint>,
    pub files: Option<FilesHeader>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesHeader {
    pub title: String,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCard {
    pub attachment: postio_model::AttachmentId,
    pub message: MessageId,
    pub kind: String,
    pub preview: postio_ui::search_view::FilePreview,
    /// `None` when the file matched by its name alone.
    pub marked: Option<u32>,
    pub name: Vec<Run>,
    pub meta: String,
    /// Empty when only its name matched.
    pub line: Vec<Run>,
    pub subject: String,
    pub focused: bool,
    pub accessible: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonRow {
    pub address: String,
    pub name: String,
    pub initials: String,
    pub messages: String,
    pub last: String,
    pub focused: bool,
    pub accessible: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilePurpose {
    Preview,
    Save,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCopy {
    pub path: std::path::PathBuf,
    pub name: String,
    pub purpose: FilePurpose,
}

pub(crate) const FILES_READ: u32 = 1_000;

pub(crate) const PEOPLE_READ: u32 = 1_000;

/// A folder of this app's own under the user's temporary directory. Nothing else is ever removed
/// from or written to by the Files tab.
pub fn file_copies() -> std::path::PathBuf {
    std::env::temp_dir().join("postio-files")
}

pub(crate) fn remove_copy(path: &std::path::Path) {
    let root = file_copies();
    if !path.starts_with(&root) || path == root {
        tracing::debug!("a copy outside the app's folder was not removed");
        return;
    }
    let _ = std::fs::remove_file(path);
    if let Some(folder) = path.parent().filter(|folder| *folder != root) {
        let _ = std::fs::remove_dir(folder);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultSplice {
    pub anchor: u64,
    pub children: Vec<postio_ui::search_view::ResultChild>,
    pub focused: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultRow {
    pub message: MessageId,
    pub thread: Option<ThreadId>,
    pub group: u32,
    pub top_hit: bool,
    pub sender: String,
    pub from_address: Option<String>,
    pub unread: bool,
    pub reason: Option<String>,
    pub subject: Vec<Run>,
    pub labels: Vec<LabelPill>,
    pub attachments: bool,
    pub count_badge: Option<String>,
    pub summary: Option<String>,
    pub source_tag: String,
    pub source_is_file: bool,
    pub passage: Vec<Run>,
    pub folder: String,
    pub date: String,
    pub checked: bool,
    pub accessible: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelPill {
    pub name: String,
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    pub token: u32,
    pub operator: String,
    pub value: String,
    pub excluded: bool,
    pub focused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterButton {
    pub kind: FilterKind,
    pub label: String,
    pub applied: bool,
    pub open: bool,
    /// `None` for the toggles, which a click flips.
    pub key: Option<String>,
}

pub fn filter_command(kind: FilterKind) -> Option<CommandId> {
    match kind {
        FilterKind::From => Some(CommandId::FilterFrom),
        FilterKind::To => Some(CommandId::FilterTo),
        FilterKind::Date => Some(CommandId::FilterDate),
        FilterKind::Anywhere => Some(CommandId::FilterAnywhere),
        FilterKind::Label => Some(CommandId::FilterLabel),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryView {
    pub chips: Vec<Chip>,
    pub words: String,
    pub hint: String,
    /// `None` when the hint names its own key.
    pub hint_key: Option<String>,
    pub ringed: bool,
    pub buttons: Vec<FilterButton>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelaxationView {
    pub number: u32,
    pub key: Option<String>,
    pub label: String,
    pub query: String,
    pub count: String,
    pub focused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoResultsView {
    pub title: String,
    pub body: String,
    pub relaxations: Vec<RelaxationView>,
    pub counting: Option<String>,
    pub searched: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PopoverRow {
    /// Its place among the rows the popover opened with, whatever its own filter hides.
    pub token: u64,
    pub title: String,
    pub detail: Option<String>,
    pub initials: Option<String>,
    pub color: Option<String>,
    pub count: u64,
    /// "24+" when those results were capped and every count is a floor.
    pub count_label: String,
    pub share: f64,
    pub checked: bool,
    pub excluded: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PopoverView {
    pub kind: FilterKind,
    pub placeholder: String,
    pub filter: String,
    pub rows: Vec<PopoverRow>,
    pub hints: Vec<Hint>,
    pub presets: Vec<DatePresetView>,
    pub words: String,
    /// `None` when they are no date.
    pub parsed: Option<String>,
    pub words_hint: String,
    pub months: Vec<MonthBar>,
    pub result: Option<String>,
    pub range: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatePresetView {
    pub token: u64,
    pub label: String,
    pub count: Option<String>,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchCard {
    pub place: String,
    pub when: String,
    pub passage: Vec<Run>,
    pub file: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickLookView {
    pub title: String,
    pub position: String,
    pub walk: Option<Hint>,
    pub actions: Vec<Hint>,
    pub subject: Vec<Run>,
    pub sender: Vec<Run>,
    pub matches_line: String,
    pub matches_hint: Option<Hint>,
    pub cards: Vec<MatchCard>,
    pub current: Option<u32>,
}

#[derive(Debug)]
pub(crate) struct QuickLook {
    position: u64,
    hit: ConversationHit,
    matches: Option<Vec<ConversationMatch>>,
    current: usize,
    stepped: bool,
    stamp: u64,
}

impl QuickLook {
    fn matches(&self) -> Vec<ConversationMatch> {
        if let Some(matches) = &self.matches {
            return matches.clone();
        }
        let mut row: Vec<ConversationMatch> = self
            .hit
            .matches
            .iter()
            .filter(|found| found.source != Source::Subject)
            .map(|found| ConversationMatch {
                message: Some(self.hit.best),
                from: match found.source {
                    Source::Quoted => None,
                    _ => self.hit.from.clone(),
                },
                found: Match {
                    when: found.when.or(Some(self.hit.newest_match)),
                    ..found.clone()
                },
                to: Vec::new(),
            })
            .collect();
        if self
            .hit
            .matches
            .iter()
            .any(|found| found.source == Source::Subject)
        {
            row.push(ConversationMatch {
                message: None,
                from: None,
                found: Match {
                    source: Source::Subject,
                    passage: None,
                    when: None,
                },
                to: Vec::new(),
            });
        }
        row
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TermEdit {
    Add {
        field: String,
        value: String,
        negated: bool,
    },
    Remove {
        token: u32,
    },
    Toggle {
        field: String,
        value: String,
    },
    SetMonths {
        first: u32,
        last: u32,
    },
    ClearFilters,
}

/// `None` when it is no operator the query language has.
pub(crate) fn clause_of(
    field: &str,
    value: &str,
    negated: bool,
    today: NaiveDate,
) -> Option<Clause> {
    let value = if value.contains(char::is_whitespace) {
        format!("\"{}\"", value.replace('"', ""))
    } else {
        value.to_owned()
    };
    let raw = format!("{}{field}:{value}", if negated { "-" } else { "" });
    let parsed = postio_search::parse(&raw, today);
    match parsed.tokens() {
        [token] => match &token.kind {
            TokenKind::Filter(clause) => Some(clause.clone()),
            _ => None,
        },
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct Frame {
    total: u64,
    capped: bool,
    facets: SearchFacets,
    files: u64,
    people: u64,
    names: FacetNames,
    elapsed: Duration,
    messages_searched: u64,
    corpus_complete: bool,
    contents_complete: bool,
}

impl Frame {
    fn of(results: &ConversationResults) -> Self {
        Frame {
            messages_searched: results.messages_searched,
            corpus_complete: results.corpus_complete,
            contents_complete: results.contents_complete,
            total: results.total,
            capped: results.capped,
            facets: results.facets.clone(),
            files: results.files,
            people: results.people,
            names: results.names.clone(),
            elapsed: results.elapsed,
        }
    }
}

pub(crate) struct Words<'a> {
    pub(crate) keymap: &'a Keymap,
    pub(crate) now: DateTime<Local>,
    pub(crate) folders: &'a [(MailboxId, String)],
    pub(crate) owners: &'a [(MailboxId, String)],
}

struct Span {
    title: String,
    accessible: String,
    count: u64,
    note: Option<String>,
    first: u64,
    rows: u64,
    top_hits: bool,
}

/// Its rows and counts hold still while its checks narrow the results.
#[derive(Debug, Clone)]
struct Base {
    facets: SearchFacets,
    names: FacetNames,
}

#[derive(Debug)]
pub(crate) struct Popover {
    kind: FilterKind,
    /// The query to restore on Esc.
    before: String,
    base: Option<Base>,
    /// Set when the query holds the popover's own field and the results' facets would list only
    /// what it already names.
    own: Option<u64>,
    filter: String,
    words: String,
}

struct Offer {
    filter: Filter,
    title: String,
    detail: Option<String>,
    initials: Option<String>,
    color: Option<String>,
    count: u64,
}

#[derive(Debug, Default)]
pub(crate) struct NoResults {
    /// `None` while they are counted.
    found: Option<Vec<(Relaxation, Counted)>>,
    focus: usize,
}

const RELAXATIONS_SHOWN: usize = 4;

#[derive(Debug)]
pub(crate) struct Results {
    pub(crate) query: String,
    pub(crate) parsed: ParsedQuery,
    terms: Vec<String>,
    pub(crate) tab: ResultsTab,
    pub(crate) order: ConversationOrder,
    stamp: u64,
    frame: Option<Frame>,
    top: Option<Vec<ConversationHit>>,
    pages: BTreeMap<u32, Vec<ConversationHit>>,
    asked: BTreeSet<u32>,
    /// Restored rows wait for the frame.
    pub(crate) cursor: Option<u64>,
    pub(crate) checked: Vec<(MessageId, Option<ThreadId>)>,
    /// ⇧X: every conversation the query matches is checked but `checked` (a predicate, never the
    /// rows read).
    pub(crate) all: bool,
    pub(crate) remembered: bool,
    pub(crate) popover: Option<Popover>,
    /// What the bars outside a range are drawn from, so a range can be dragged wider than the one
    /// the results were narrowed to.
    undated: Option<(String, [postio_search::facets::MonthCount; 12], bool)>,
    pub(crate) quick_look: Option<QuickLook>,
    /// Kept across queries so a chip or button names its person while the next answer is on its
    /// way.
    people: Vec<postio_model::EmailAddress>,
    held: Vec<(MessageId, String, bool)>,
    none: Option<NoResults>,
    files: Option<Vec<FileHit>>,
    files_asked: bool,
    file_cursor: Option<u64>,
    pub(crate) copy: Option<PendingCopy>,
    listed: Option<Vec<postio_search::suggest::Person>>,
    listed_asked: bool,
    person_cursor: Option<u64>,
    pub(crate) expansion: Option<ResultExpansion>,
    default_done: bool,
    pub(crate) opening: Option<crate::walk::ThreadMatches>,
    /// Mail changed while the first page was being read: its answer may predate it, so they are
    /// read once more. A sync's burst costs two reads, not one each.
    stale: bool,
}

#[derive(Debug)]
pub(crate) struct PendingCopy {
    purpose: FilePurpose,
    stamp: u64,
    name: String,
    path: Option<std::path::PathBuf>,
}

impl Results {
    pub(crate) fn new(
        query: String,
        parsed: ParsedQuery,
        tab: ResultsTab,
        order: ConversationOrder,
        stamp: u64,
    ) -> Self {
        Results {
            terms: postio_search::highlight::terms(&parsed),
            query,
            parsed,
            tab,
            order,
            stamp,
            frame: None,
            top: None,
            pages: BTreeMap::new(),
            asked: BTreeSet::new(),
            cursor: None,
            checked: Vec::new(),
            all: false,
            remembered: false,
            popover: None,
            undated: None,
            quick_look: None,
            people: Vec::new(),
            held: Vec::new(),
            none: None,
            files: None,
            files_asked: false,
            file_cursor: None,
            copy: None,
            listed: None,
            listed_asked: false,
            person_cursor: None,
            stale: false,
            expansion: None,
            default_done: false,
            opening: None,
        }
    }

    pub(crate) fn default_order(parsed: &ParsedQuery) -> ConversationOrder {
        if parsed.text_terms().any(|term| !term.negated) {
            ConversationOrder::BestMatch
        } else {
            ConversationOrder::Newest
        }
    }

    pub(crate) fn asks(&mut self) -> Vec<Step> {
        let mut steps = Vec::new();
        if self.order == ConversationOrder::BestMatch {
            steps.push(Step::Ask(Request::ResultsPage {
                query: self.parsed.clone(),
                order: ConversationOrder::BestMatch,
                offset: 0,
                limit: TOP_HITS,
                stamp: self.stamp,
            }));
        }
        steps.extend(self.ask_page(0));
        steps.extend(self.ask_files());
        steps.extend(self.ask_people());
        steps
    }

    pub(crate) fn ask_people(&mut self) -> Option<Step> {
        if self.tab != ResultsTab::People || self.listed.is_some() || self.listed_asked {
            return None;
        }
        self.listed_asked = true;
        Some(Step::Ask(Request::People {
            query: self.parsed.clone(),
            stamp: self.stamp,
        }))
    }

    pub(crate) fn people_landed(
        &mut self,
        stamp: u64,
        answer: Result<Vec<postio_search::suggest::Person>, String>,
    ) -> bool {
        if stamp != self.stamp {
            return false;
        }
        self.listed_asked = false;
        let found = match answer {
            Ok(found) => found,
            Err(error) => {
                tracing::debug!(%error, "the people could not be read");
                Vec::new()
            }
        };
        self.person_cursor = match self.person_cursor {
            Some(at) if !found.is_empty() => Some(at.min(found.len() as u64 - 1)),
            _ if !found.is_empty() => Some(0),
            _ => None,
        };
        self.listed = Some(found);
        true
    }

    fn cursor_person(&self) -> Option<&postio_search::suggest::Person> {
        self.listed.as_ref()?.get(self.person_cursor? as usize)
    }

    pub(crate) fn person_row(&self, position: u64, with: &Words<'_>) -> Option<PersonRow> {
        let person = self.listed.as_ref()?.get(usize::try_from(position).ok()?)?;
        let address = postio_model::EmailAddress::new(person.name.as_deref(), &person.address);
        let name = person
            .name
            .clone()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| person.address.clone());
        let messages = words::person_messages(person.received + person.sent);
        let last = person
            .last
            .map(|last| words::hit_date(last.with_timezone(&Local), with.now))
            .unwrap_or_default();
        Some(PersonRow {
            accessible: words::person_accessible(&name, &person.address, &messages, &last),
            address: person.address.clone(),
            initials: postio_ui::row::initials(Some(&address)),
            name,
            messages,
            last,
            focused: self.person_cursor == Some(position),
        })
    }

    fn ask_files(&mut self) -> Option<Step> {
        if self.tab != ResultsTab::Files || self.files.is_some() || self.files_asked {
            return None;
        }
        self.files_asked = true;
        Some(Step::Ask(Request::Files {
            query: self.parsed.clone(),
            stamp: self.stamp,
        }))
    }

    pub(crate) fn files_landed(
        &mut self,
        stamp: u64,
        answer: Result<Vec<FileHit>, String>,
    ) -> bool {
        if stamp != self.stamp {
            return false;
        }
        self.files_asked = false;
        let found = match answer {
            Ok(found) => found,
            Err(error) => {
                tracing::debug!(%error, "the files could not be read");
                Vec::new()
            }
        };
        self.file_cursor = match self.file_cursor {
            Some(at) if !found.is_empty() => Some(at.min(found.len() as u64 - 1)),
            _ if !found.is_empty() => Some(0),
            _ => None,
        };
        self.files = Some(found);
        true
    }

    fn cursor_file(&self) -> Option<&FileHit> {
        self.files.as_ref()?.get(self.file_cursor? as usize)
    }

    pub(crate) fn file_card(&self, position: u64, with: &Words<'_>) -> Option<FileCard> {
        let hit = self.files.as_ref()?.get(usize::try_from(position).ok()?)?;
        let sender = hit
            .from
            .as_ref()
            .map(postio_ui::command_bar::said_of)
            .unwrap_or_default();
        let date = words::hit_date(hit.received_at.with_timezone(&Local), with.now);
        let meta = words::file_meta(&sender, &date, hit.size);
        let name_terms: Vec<String> = self.terms.clone();
        let name = dropdown::marked(
            &hit.name,
            &postio_search::highlight::find(&hit.name, &name_terms),
        );
        let (line, marked) = match &hit.matched {
            Some(Match {
                source: Source::FileContent { location, .. },
                passage,
                ..
            }) => {
                let mut line = vec![Run::plain(format!("{}: ", words::location(location)))];
                if let Some(cut) = passage {
                    line.extend(dropdown::passage(cut));
                }
                (line, Some(marked_line(location)))
            }
            _ => (Vec::new(), None),
        };
        let kind = words::file_kind(&hit.name, &hit.mime_type);
        let subject = words::file_subject(hit.subject.as_deref().unwrap_or_default());
        let line_text: String = line.iter().map(|run| run.text.as_str()).collect();
        Some(FileCard {
            attachment: hit.attachment,
            message: hit.message,
            preview: words::file_preview(&hit.name, &hit.mime_type),
            marked,
            accessible: words::file_accessible(&hit.name, &kind, &meta, &line_text, &subject),
            kind,
            name,
            meta,
            line,
            subject,
            focused: self.file_cursor == Some(position),
        })
    }

    fn ask_page(&mut self, page: u32) -> Option<Step> {
        if self.pages.contains_key(&page) || !self.asked.insert(page) {
            return None;
        }
        Some(Step::Ask(Request::ResultsPage {
            query: self.parsed.clone(),
            order: ConversationOrder::Newest,
            offset: page * PAGE,
            limit: PAGE,
            stamp: self.stamp,
        }))
    }

    /// The frame drawn stays on screen until the new one lands.
    pub(crate) fn again(&mut self, stamp: u64) -> Vec<Step> {
        self.stamp = stamp;
        self.frame = None;
        self.top = None;
        self.pages.clear();
        self.asked.clear();
        self.files = None;
        self.files_asked = false;
        self.listed = None;
        self.listed_asked = false;
        self.default_done = false;
        self.stale = false;
        let mut steps = Vec::new();
        if self.expansion.take().is_some() {
            steps.push(Step::Show(Intent::ResultSplice(None)));
        }
        if self.none.take().is_some() {
            steps.push(Step::Show(Intent::Relaxations(None)));
        }
        steps.extend(self.asks());
        steps
    }

    pub(crate) fn found_nothing(&self) -> bool {
        self.none.is_some()
    }

    fn ways_out(&self) -> Vec<&(Relaxation, Counted)> {
        let Some(found) = self.none.as_ref().and_then(|none| none.found.as_ref()) else {
            return Vec::new();
        };
        let mut shown: Vec<&(Relaxation, Counted)> =
            found.iter().filter(|(_, n)| n.conversations > 0).collect();
        shown.sort_by_key(|(_, n)| std::cmp::Reverse(n.conversations));
        shown.truncate(RELAXATIONS_SHOWN);
        shown
    }

    fn loosened_token(&self) -> Option<usize> {
        let focus = self.none.as_ref()?.focus;
        let (relaxation, _) = self.ways_out().get(focus).copied()?;
        let (Loosen::Drop { token }
        | Loosen::Anywhere { token }
        | Loosen::FolderNotLabel { token }) = relaxation.loosen;
        Some(token)
    }

    pub(crate) fn way_out(&self, at: usize) -> Option<String> {
        self.ways_out()
            .get(at)
            .map(|(relaxation, _)| relaxation.query.clone())
    }

    pub(crate) fn step_way_out(&mut self, by: i64) -> bool {
        let len = self.ways_out().len();
        let Some(none) = self.none.as_mut() else {
            return false;
        };
        if len == 0 {
            return false;
        }
        let next = (none.focus as i64)
            .saturating_add(by)
            .clamp(0, len as i64 - 1) as usize;
        let moved = next != none.focus;
        none.focus = next;
        moved
    }

    pub(crate) fn ways_out_landed(
        &mut self,
        stamp: u64,
        answer: Result<Vec<(Relaxation, Counted)>, String>,
    ) -> bool {
        if stamp != self.stamp {
            return false;
        }
        let Some(none) = self.none.as_mut() else {
            return false;
        };
        none.found = Some(match answer {
            Ok(found) => found,
            Err(error) => {
                tracing::debug!(%error, "the ways out of a search could not be counted");
                Vec::new()
            }
        });
        none.focus = 0;
        true
    }

    pub(crate) fn no_results_view(
        &self,
        keymap: &Keymap,
        today: NaiveDate,
        names: &QueryNames<'_>,
    ) -> Option<NoResultsView> {
        let none = self.none.as_ref()?;
        let frame = self.frame.as_ref()?;
        let name_of = |address: &str| self.name_of(address).or_else(|| (names.lookup)(address));
        let ways = self.ways_out();
        let relaxations: Vec<RelaxationView> = ways
            .iter()
            .enumerate()
            .map(|(at, (relaxation, n))| RelaxationView {
                number: at as u32 + 1,
                key: words::PICK_RELAXATION
                    .get(at)
                    .and_then(|id| postio_ui::hints::key(keymap, *id)),
                label: words::relaxation_line(relaxation, &self.parsed, &name_of, today),
                query: relaxation.query.clone(),
                count: words::relaxation_count(n.conversations, n.capped),
                focused: at == none.focus,
            })
            .collect();
        let counted = none.found.is_some();
        let terms = self.parsed.filters().count() + self.parsed.text_terms().count();
        Some(NoResultsView {
            title: words::nothing_matches(terms),
            body: if counted && relaxations.is_empty() {
                words::NO_RELAXATIONS_BODY.to_owned()
            } else {
                words::NO_RESULTS_BODY.to_owned()
            },
            relaxations,
            counting: (!counted).then(|| words::COUNTING_RELAXATIONS.to_owned()),
            searched: words::searched(
                keymap,
                frame.messages_searched,
                frame.corpus_complete,
                frame.contents_complete,
            ),
        })
    }

    pub(crate) fn set_query(&mut self, query: String, parsed: ParsedQuery) {
        self.terms = postio_search::highlight::terms(&parsed);
        self.query = query;
        self.parsed = parsed;
        self.clear_checked();
        self.cursor = None;
        self.file_cursor = None;
        self.remembered = false;
    }

    fn ready(&self) -> bool {
        self.frame.is_some() && (self.order == ConversationOrder::Newest || self.top.is_some())
    }

    fn top_len(&self) -> u64 {
        if self.tab != ResultsTab::Conversations || self.order != ConversationOrder::BestMatch {
            return 0;
        }
        self.top.as_ref().map_or(0, |top| top.len() as u64)
    }

    pub(crate) fn rows(&self) -> u64 {
        match (&self.frame, self.tab) {
            (Some(frame), ResultsTab::Conversations) if self.ready() => {
                self.top_len() + frame.total
            }
            (_, ResultsTab::Files) => self.files.as_ref().map_or(0, |files| files.len() as u64),
            (_, ResultsTab::People) => self.listed.as_ref().map_or(0, |people| people.len() as u64),
            _ => 0,
        }
    }

    fn spans(&self) -> Vec<Span> {
        let Some(frame) = self.frame.as_ref().filter(|_| self.ready()) else {
            return Vec::new();
        };
        if self.tab != ResultsTab::Conversations {
            return Vec::new();
        }
        let mut spans = Vec::new();
        let top = self.top_len();
        if top > 0 {
            spans.push(Span {
                accessible: words::group_line(words::TOP_HITS, top),
                title: words::TOP_HITS.to_owned(),
                count: top,
                note: Some(words::TOP_HITS_RESULTS_NOTE.to_owned()),
                first: 0,
                rows: top,
                top_hits: true,
            });
        }
        let mut first = top;
        let mut counted = 0;
        for month in frame.facets.months.iter().rev() {
            if month.conversations == 0 {
                continue;
            }
            // A capped total is a floor: no group runs past it.
            let rows = month.conversations.min(frame.total.saturating_sub(counted));
            if rows == 0 {
                break;
            }
            let newest = spans.iter().all(|span: &Span| span.top_hits);
            spans.push(Span {
                accessible: words::month_group(month.month, month.conversations),
                title: words::month_title(month.month),
                count: month.conversations,
                note: newest.then(|| words::NEWEST_FIRST.to_owned()),
                first,
                rows,
                top_hits: false,
            });
            first += rows;
            counted += rows;
        }
        let earlier = frame.total.saturating_sub(counted);
        if earlier > 0 {
            spans.push(Span {
                accessible: words::group_line(words::EARLIER, earlier),
                title: words::EARLIER.to_owned(),
                count: earlier,
                note: None,
                first,
                rows: earlier,
                top_hits: false,
            });
        }
        spans
    }

    fn hit_at(&self, position: u64) -> Option<(&ConversationHit, u32, bool)> {
        if self.tab != ResultsTab::Conversations || position >= self.rows() {
            return None;
        }
        let group = self
            .spans()
            .iter()
            .rposition(|span| span.first <= position)
            .unwrap_or(0) as u32;
        let top = self.top_len();
        if position < top {
            let hit = self.top.as_ref()?.get(position as usize)?;
            return Some((hit, group, true));
        }
        let at = position - top;
        let page = u32::try_from(at / u64::from(PAGE)).ok()?;
        let hit = self
            .pages
            .get(&page)?
            .get((at % u64::from(PAGE)) as usize)?;
        Some((hit, group, false))
    }

    pub(crate) fn position_of(&self, message: MessageId) -> Option<u64> {
        let top = self.top_len();
        if let Some(at) = self
            .top
            .iter()
            .flatten()
            .take(top as usize)
            .position(|hit| hit.best == message)
        {
            return Some(at as u64);
        }
        self.pages.iter().find_map(|(page, hits)| {
            hits.iter()
                .position(|hit| hit.best == message)
                .map(|at| top + u64::from(*page) * u64::from(PAGE) + at as u64)
        })
    }

    pub(crate) fn lone_subject(&self, message: MessageId) -> Option<String> {
        self.top
            .iter()
            .flatten()
            .chain(self.pages.values().flatten())
            .find(|hit| hit.best == message && hit.messages <= 1)
            .map(|hit| hit.subject.clone())
            .or_else(|| {
                self.files
                    .iter()
                    .flatten()
                    .find(|file| file.message == message)
                    .map(|file| file.subject.clone())
            })
            .map(Option::unwrap_or_default)
    }

    pub(crate) fn walk(&self) -> Vec<(MessageId, Option<ThreadId>)> {
        let top = self.top_len() as usize;
        // Top hits are drawn again under their months: walked once, where
        // they come first, so "Result n of m" counts conversations.
        let mut walked = std::collections::HashSet::new();
        self.top
            .iter()
            .flatten()
            .take(top)
            .chain(self.pages.values().flatten())
            .filter(|hit| walked.insert(hit.key))
            .map(|hit| (hit.best, thread_of(hit)))
            .collect()
    }

    /// "Result n of m", each top hit once. A row under the months comes after the top hits, less
    /// those drawn again above it (the ones read so far, since the pages between may not be).
    fn walked_number(&self, position: u64) -> (u64, u64) {
        let of = match (&self.frame, self.tab) {
            (Some(frame), ResultsTab::Conversations) => frame.total,
            _ => return (position, self.rows()),
        };
        let top = self.top_len();
        if position < top {
            return (position, of);
        }
        let at = position - top;
        let tops: Vec<_> = self
            .top
            .iter()
            .flatten()
            .take(top as usize)
            .map(|hit| hit.key)
            .collect();
        if let Some((hit, _, _)) = self.hit_at(position)
            && let Some(first) = tops.iter().position(|key| *key == hit.key)
        {
            return (first as u64, of);
        }
        let again = self
            .pages
            .iter()
            .flat_map(|(page, hits)| {
                let first = u64::from(*page) * u64::from(PAGE);
                hits.iter()
                    .enumerate()
                    .map(move |(i, hit)| (first + i as u64, hit))
            })
            .filter(|(row, hit)| *row <= at && tops.contains(&hit.key))
            .count() as u64;
        let number = (top + at).saturating_sub(again);
        (number.min(of.saturating_sub(1)), of)
    }

    pub(crate) fn cursor_sender(&self) -> Option<String> {
        let (hit, _, _) = self.hit_at(self.cursor?)?;
        hit.from.as_ref().map(|from| from.address.clone())
    }

    pub(crate) fn cursor_message(&self) -> Option<(MessageId, Option<ThreadId>)> {
        let (hit, _, _) = self.hit_at(self.cursor?)?;
        Some((hit.best, thread_of(hit)))
    }

    pub(crate) fn wanted(&mut self, position: u64) -> Vec<Step> {
        if !self.ready() || position >= self.rows() {
            return Vec::new();
        }
        let top = self.top_len();
        if position < top {
            return Vec::new();
        }
        let Ok(page) = u32::try_from((position - top) / u64::from(PAGE)) else {
            return Vec::new();
        };
        self.ask_page(page).into_iter().collect()
    }

    pub(crate) fn landed(
        &mut self,
        stamp: u64,
        order: ConversationOrder,
        offset: u32,
        answer: Result<Box<ConversationResults>, String>,
    ) -> Landed {
        if stamp != self.stamp {
            return Landed::default();
        }
        let results = match answer {
            Ok(results) => *results,
            Err(error) => {
                tracing::debug!(%error, "a page of search results could not be read");
                if order == ConversationOrder::Newest {
                    self.asked.remove(&(offset / PAGE));
                }
                return Landed::default();
            }
        };
        let was_ready = self.ready();
        if self.frame.is_none() {
            self.frame = Some(Frame::of(&results));
        }
        for held in &results.names.held {
            if !self.held.iter().any(|known| known.0 == held.0) {
                self.held.push(held.clone());
            }
        }
        for (_, person) in &results.names.people {
            if person.name.is_some()
                && !self
                    .people
                    .iter()
                    .any(|known| known.address.eq_ignore_ascii_case(&person.address))
            {
                self.people.push(person.clone());
            }
        }
        if self.dates() == (None, None) {
            self.undated = Some((
                self.query.clone(),
                results.facets.months,
                results.facets.capped,
            ));
        }
        if let Some(popover) = self.popover.as_mut()
            && popover.base.is_none()
            && popover.own.is_none()
            && popover.before == self.query
        {
            popover.base = Some(Base {
                facets: results.facets.clone(),
                names: results.names.clone(),
            });
        }
        let passages = self.passages_for(&results.hits);
        let mut landed = Landed {
            passages,
            ..Landed::default()
        };
        let span;
        if order == ConversationOrder::BestMatch {
            span = (0, results.hits.len() as u64);
            self.top = Some(results.hits);
        } else {
            let page = offset / PAGE;
            self.asked.remove(&page);
            let count = results.hits.len() as u64;
            self.pages.insert(page, results.hits);
            self.evict(page);
            span = (self.top_len() + u64::from(page) * u64::from(PAGE), count);
        }
        if !was_ready && self.ready() {
            landed.frame = true;
            if !self.remembered {
                self.remembered = true;
                landed.remember = self.frame.as_ref().map(|frame| (frame.total, frame.capped));
            }
            if self.frame.as_ref().is_some_and(|frame| frame.total == 0) {
                self.none = Some(NoResults::default());
                landed.nothing = true;
            }
            let rows = self.rows();
            self.cursor = match self.cursor {
                Some(cursor) if rows > 0 => Some(cursor.min(rows - 1)),
                _ if rows > 0 => Some(0),
                _ => None,
            };
        } else if self.ready() && span.1 > 0 {
            landed.rows = Some(span);
        }
        landed
    }

    pub(crate) fn facets_landed(
        &mut self,
        stamp: u64,
        answer: Result<Box<ConversationResults>, String>,
    ) -> bool {
        let Some(popover) = self.popover.as_mut() else {
            return false;
        };
        if popover.own != Some(stamp) || popover.base.is_some() {
            return false;
        }
        match answer {
            Ok(results) => {
                popover.base = Some(Base {
                    facets: results.facets,
                    names: results.names,
                });
                true
            }
            Err(error) => {
                tracing::debug!(%error, "a popover's facets could not be read");
                false
            }
        }
    }

    fn evict(&mut self, page: u32) {
        while self.pages.len() > RESIDENT {
            let far = self
                .pages
                .keys()
                .copied()
                .max_by_key(|held| held.abs_diff(page));
            match far {
                Some(far) => {
                    self.pages.remove(&far);
                }
                None => break,
            }
        }
    }

    fn passages_for(&self, hits: &[ConversationHit]) -> Option<Request> {
        if hits.is_empty() {
            return None;
        }
        Some(Request::ResultsPassages {
            query: self.parsed.clone(),
            hits: hits
                .iter()
                .map(|hit| {
                    (
                        hit.best,
                        hit.matches.iter().map(|each| each.source.clone()).collect(),
                    )
                })
                .collect(),
            stamp: self.stamp,
        })
    }

    pub(crate) fn passages(
        &mut self,
        stamp: u64,
        answer: Result<Vec<(MessageId, Vec<Match>)>, String>,
    ) -> Option<(u64, u64)> {
        if stamp != self.stamp {
            return None;
        }
        let found = match answer {
            Ok(found) => found,
            Err(error) => {
                tracing::debug!(%error, "search passages could not be read");
                return None;
            }
        };
        // Before the frame the rows have no positions yet, but what was read is theirs: kept on the
        // hits, and drawn when the frame lands (#2089).
        let ready = self.ready();
        let mut low = u64::MAX;
        let mut high = 0;
        for (message, matches) in found {
            let hits = self
                .top
                .iter_mut()
                .flatten()
                .chain(self.pages.values_mut().flatten())
                .filter(|hit| hit.best == message);
            for hit in hits {
                hit.matches.clone_from(&matches);
            }
            if !ready {
                continue;
            }
            if let Some(position) = self.position_of(message) {
                low = low.min(position);
                high = high.max(position);
            }
            if let Some(position) = self.page_position_of(message) {
                low = low.min(position);
                high = high.max(position);
            }
        }
        (low <= high).then(|| (low, high - low + 1))
    }

    fn page_position_of(&self, message: MessageId) -> Option<u64> {
        let top = self.top_len();
        self.pages.iter().find_map(|(page, hits)| {
            hits.iter()
                .position(|hit| hit.best == message)
                .map(|at| top + u64::from(*page) * u64::from(PAGE) + at as u64)
        })
    }

    pub(crate) fn step(&mut self, by: i64) -> Option<u64> {
        let rows = self.rows();
        if rows == 0 {
            return None;
        }
        let ring = self.ring();
        let at = ring.map_or(0, |at| at as i64);
        let next = (at + by).clamp(0, rows as i64 - 1) as u64;
        if Some(next) == *ring {
            return None;
        }
        *ring = Some(next);
        self.unfocus_child();
        Some(next)
    }

    fn ring(&mut self) -> &mut Option<u64> {
        match self.tab {
            ResultsTab::Files => &mut self.file_cursor,
            ResultsTab::People => &mut self.person_cursor,
            _ => &mut self.cursor,
        }
    }

    pub(crate) fn point(&mut self, position: u64) -> Option<u64> {
        let rows = self.rows();
        let ring = self.ring();
        let moved = (position < rows && *ring != Some(position)).then(|| {
            *ring = Some(position);
            position
        });
        if moved.is_some() {
            self.unfocus_child();
        }
        moved
    }

    pub(crate) fn toggle_checked(&mut self) -> bool {
        let Some((message, thread)) = self.cursor_message() else {
            return false;
        };
        match self.checked.iter().position(|(at, _)| *at == message) {
            Some(at) => {
                self.checked.remove(at);
            }
            None => self.checked.push((message, thread)),
        }
        true
    }

    pub(crate) fn check_all(&mut self) {
        self.all = true;
        self.checked.clear();
    }

    pub(crate) fn clear_checked(&mut self) {
        self.all = false;
        self.checked.clear();
    }

    pub(crate) fn any_checked(&self) -> bool {
        self.all || !self.checked.is_empty()
    }

    fn is_checked(&self, message: MessageId) -> bool {
        self.all != self.checked.iter().any(|(at, _)| *at == message)
    }

    pub(crate) fn selected(&self) -> u64 {
        if self.all {
            let total = self.frame.as_ref().map_or(0, |frame| frame.total);
            total.saturating_sub(self.checked.len() as u64)
        } else {
            self.checked.len() as u64
        }
    }

    pub(crate) fn aim(&self) -> Option<ResultsAim> {
        if self.all {
            return Some(ResultsAim::Matching {
                query: self.parsed.clone(),
                except: self.checked.iter().map(|(message, _)| *message).collect(),
            });
        }
        let picked: Vec<(MessageId, Option<ThreadId>)> = if self.checked.is_empty() {
            self.cursor_message().into_iter().collect()
        } else {
            self.checked.clone()
        };
        if picked.is_empty() {
            return None;
        }
        let selection = postio_core::state::Selection::These(
            picked.iter().map(|(message, _)| *message).collect(),
        );
        let reach = picked
            .iter()
            .filter_map(|(message, thread)| thread.map(|thread| (*message, vec![thread])))
            .collect();
        match postio_ui::focus_target::aim_by(
            &selection,
            &reach,
            None,
            postio_core::CommandId::Archive,
        ) {
            postio_ui::focus_target::Aim::Targets(aims) if !aims.is_empty() => {
                Some(ResultsAim::Targets {
                    aims,
                    conversations: picked.len(),
                })
            }
            _ => None,
        }
    }

    fn filters(&self) -> Vec<Filter> {
        self.parsed
            .filters()
            .filter(|clause| !clause.negated)
            .map(|clause| clause.filter.clone())
            .collect()
    }

    pub(crate) fn name_of(&self, address: &str) -> Option<String> {
        self.people
            .iter()
            .find(|person| person.address.eq_ignore_ascii_case(address))
            .and_then(|person| person.name.clone())
    }

    pub(crate) fn query_view(&self, today: NaiveDate, names: &QueryNames<'_>) -> QueryView {
        let name_of = |address: &str| self.name_of(address).or_else(|| (names.lookup)(address));
        let ringed = self.loosened_token();
        let mut chips = Vec::new();
        let mut plain = Vec::new();
        for (index, token) in self.parsed.tokens().iter().enumerate() {
            match &token.kind {
                TokenKind::Filter(clause) => {
                    let raw = token.raw.trim_start_matches('-');
                    let (operator, value) = raw.split_once(':').unwrap_or((raw, ""));
                    let value = value.trim_matches('"');
                    let person = |who: &str| {
                        name_of(who)
                            .filter(|name| !name.is_empty())
                            .unwrap_or_else(|| who.to_owned())
                    };
                    let value = match &clause.filter {
                        Filter::From(_) | Filter::To(_) => person(value),
                        Filter::AnyOf(set) => set
                            .members()
                            .iter()
                            .filter_map(|member| match member {
                                Filter::From(who) | Filter::To(who) => Some(person(who)),
                                Filter::In(name) => Some(words::folder_name(name)),
                                Filter::Subject(text)
                                | Filter::Filename(text)
                                | Filter::List(text)
                                | Filter::Account(text)
                                | Filter::Group(text)
                                | Filter::Label(text) => Some(text.clone()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join(", "),
                        Filter::In(_) => words::folder_name(value),
                        _ => value.to_owned(),
                    };
                    chips.push(Chip {
                        token: index as u32,
                        operator: format!("{operator}:"),
                        value,
                        excluded: clause.negated,
                        focused: ringed == Some(index),
                    });
                }
                TokenKind::Text(_) => plain.push(token.raw.clone()),
                TokenKind::Partial(_) => {}
            }
        }
        let filters = self.filters();
        let buttons = FilterKind::ALL
            .iter()
            .map(|kind| FilterButton {
                kind: *kind,
                label: words::filter_button_label(*kind, &filters, &name_of, today),
                applied: filters.iter().any(|filter| kind.holds(filter)),
                open: self
                    .popover
                    .as_ref()
                    .is_some_and(|popover| popover.kind == *kind),
                key: None,
            })
            .collect();
        QueryView {
            chips,
            words: plain.join(" "),
            hint: words::TO_EDIT.to_owned(),
            hint_key: None,
            ringed: false,
            buttons,
        }
    }

    pub(crate) fn view(&self, with: &Words<'_>) -> ResultsView {
        let frame = self.frame.as_ref().filter(|_| self.ready());
        let (total, capped) = frame.map_or((0, false), |frame| (frame.total, frame.capped));
        let tab = |tab, label: &str, count: u64, capped: bool, id| ResultsTabView {
            tab,
            label: label.to_owned(),
            count: words::tab_count(count, capped),
            selected: self.tab == tab,
            key: postio_ui::hints::key(with.keymap, id),
        };
        let files = self.files.as_ref().map_or_else(
            || frame.map_or(0, |frame| frame.files),
            |files| files.len() as u64,
        );
        let on_files = self.tab == ResultsTab::Files;
        let tabs = vec![
            tab(
                ResultsTab::Conversations,
                words::TABS[0],
                total,
                capped,
                CommandId::ResultsConversations,
            ),
            tab(
                ResultsTab::Files,
                words::TABS[1],
                files,
                capped,
                CommandId::ResultsFiles,
            ),
            tab(
                ResultsTab::People,
                words::TABS[2],
                self.listed.as_ref().map_or_else(
                    || frame.map_or(0, |frame| frame.people),
                    |people| people.len() as u64,
                ),
                capped,
                CommandId::ResultsPeople,
            ),
        ];
        let months = self.month_bars();
        let today = with.now.date_naive();
        let range = self.range(today, false);
        let groups = self
            .spans()
            .into_iter()
            .map(|span| ResultGroup {
                title: span.title,
                count: if span.top_hits {
                    String::new()
                } else {
                    words::tab_count(span.count, capped)
                },
                note: span.note,
                first: span.first,
                rows: span.rows,
                top_hits: span.top_hits,
                accessible: span.accessible,
            })
            .collect();
        ResultsView {
            tabs,
            order: if self.tab == ResultsTab::Files {
                ConversationOrder::Newest
            } else {
                self.order
            },
            sortable: self.tab != ResultsTab::Files,
            count_line: frame.map_or_else(String::new, |_| {
                if on_files {
                    words::files_count_line(files, capped)
                } else {
                    words::count_line(total, capped)
                }
            }),
            sub_line: frame.map_or_else(String::new, |frame| {
                let people = self
                    .listed
                    .as_ref()
                    .map_or(frame.people, |people| people.len() as u64);
                self.previewing(with).unwrap_or_else(|| {
                    let name_of = |address: &str| self.name_of(address);
                    words::narrowed_sub_line(
                        &self.filters(),
                        &name_of,
                        today,
                        files,
                        people,
                        frame.capped,
                    )
                })
            }),
            months,
            timeline_hint: range
                .as_deref()
                .map_or_else(|| words::TIMELINE_HINT.to_owned(), words::range_selected),
            timeline_step: range
                .as_ref()
                .and_then(|_| words::timeline_step(with.keymap)),
            groups,
            rows: self.rows(),
            cursor: match self.tab {
                ResultsTab::Files => self.file_cursor,
                ResultsTab::People => self.person_cursor,
                _ => self.cursor.filter(|_| self.ready()),
            },
            hints: if self.found_nothing() {
                words::no_results_hints(with.keymap, self.ways_out().len())
            } else if self.tab == ResultsTab::Files {
                words::files_hints(with.keymap)
            } else if self.tab == ResultsTab::People {
                words::people_hints(with.keymap)
            } else {
                words::results_hints(with.keymap)
            },
            footer: frame.map_or_else(String::new, |frame| {
                if self.found_nothing() {
                    words::searched_footer(frame.messages_searched, frame.elapsed)
                } else if on_files {
                    words::files_footer(files, capped, frame.elapsed)
                } else {
                    words::results_footer(total, capped, frame.elapsed)
                }
            }),
            selected: self.selected(),
            bulk: if self.any_checked() {
                words::bulk_hints(with.keymap)
            } else {
                Vec::new()
            },
            select_all: (self.any_checked() && self.selected() < total)
                .then(|| words::select_all_hint(with.keymap, total, capped))
                .flatten(),
            files: (self.tab == ResultsTab::Files).then(|| FilesHeader {
                title: words::FILES_TITLE.to_owned(),
                note: words::files_note(
                    with.keymap,
                    frame.is_none_or(|frame| frame.contents_complete),
                ),
            }),
        }
    }

    fn previewing(&self, with: &Words<'_>) -> Option<String> {
        let popover = self.popover.as_ref()?;
        if popover.before == self.query {
            return None;
        }
        if popover.kind == FilterKind::Date {
            let today = with.now.date_naive();
            let what = self
                .range(today, false)
                .unwrap_or_else(|| words::date_presets(today)[0].label.to_lowercase());
            return Some(words::previewing(&what));
        }
        let filters = self.filters();
        let name_of = |address: &str| self.name_of(address);
        let what =
            words::filter_button_label(popover.kind, &filters, &name_of, with.now.date_naive());
        // The value alone ("Ada Moreno"): the line is 210 points wide.
        let what = what
            .split_once(": ")
            .map_or(what.as_str(), |(_, value)| value);
        Some(words::previewing(what))
    }

    fn offers(&self, today: NaiveDate) -> Vec<Offer> {
        let Some(popover) = self.popover.as_ref() else {
            return Vec::new();
        };
        let Some(base) = popover.base.as_ref() else {
            return Vec::new();
        };
        let names = &base.names;
        let person = |count: &postio_search::facets::Count<postio_model::AddressId>,
                      field: &str| {
            let (_, who) = names.people.iter().find(|(id, _)| *id == count.id)?;
            let filter = clause_of(field, who.address.as_str(), false, today)?.filter;
            Some(Offer {
                filter,
                title: who
                    .name
                    .clone()
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| who.address.to_string()),
                detail: Some(who.address.to_string()),
                initials: Some(postio_ui::row::initials(Some(who))),
                color: None,
                count: count.conversations,
            })
        };
        match popover.kind {
            // You are in every message you sent: not someone to narrow
            // to (screen 08), as the People tab leaves you out, unless the
            // query already asks for you and there is a check to take off.
            // To keeps you, since mail to you is a real narrowing.
            FilterKind::From => base
                .facets
                .senders
                .iter()
                .filter_map(|count| {
                    let offer = person(count, "from")?;
                    let own = names.own.contains(&count.id);
                    (!own || self.holding(&offer.filter).is_some()).then_some(offer)
                })
                .collect(),
            FilterKind::To => base
                .facets
                .recipients
                .iter()
                .filter_map(|count| person(count, "to"))
                .collect(),
            FilterKind::Anywhere => base
                .facets
                .folders
                .iter()
                .filter_map(|count| {
                    let (_, name) = names.folders.iter().find(|(id, _)| *id == count.id)?;
                    Some(Offer {
                        filter: clause_of("in", name, false, today)?.filter,
                        title: words::folder_name(name),
                        detail: None,
                        initials: None,
                        color: None,
                        count: count.conversations,
                    })
                })
                .collect(),
            FilterKind::Label => base
                .facets
                .labels
                .iter()
                .filter_map(|count| {
                    let (_, name) = names.labels.iter().find(|(id, _)| *id == count.id)?;
                    Some(Offer {
                        filter: clause_of("label", name, false, today)?.filter,
                        title: name.clone(),
                        detail: None,
                        initials: None,
                        color: names
                            .label_colors
                            .iter()
                            .find(|(id, _)| *id == count.id)
                            .map(|(_, color)| color.clone()),
                        count: count.conversations,
                    })
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    fn holding(&self, filter: &Filter) -> Option<(usize, bool)> {
        self.parsed
            .tokens()
            .iter()
            .enumerate()
            .find_map(|(index, token)| match &token.kind {
                TokenKind::Filter(clause)
                    if clause
                        .filter
                        .alternatives()
                        .iter()
                        .any(|held| postio_search::edit::same(held, filter)) =>
                {
                    Some((index, clause.negated))
                }
                _ => None,
            })
    }

    pub(crate) fn popover_view(&self, today: NaiveDate) -> Option<PopoverView> {
        let popover = self.popover.as_ref()?;
        let offers = self.offers(today);
        let largest = offers
            .iter()
            .map(|offer| offer.count)
            .max()
            .unwrap_or(0)
            .max(1);
        let needle = popover.filter.trim().to_lowercase();
        let capped = popover.base.as_ref().is_some_and(|base| base.facets.capped);
        let rows = offers
            .into_iter()
            .enumerate()
            .filter(|(_, offer)| {
                needle.is_empty()
                    || offer.title.to_lowercase().contains(&needle)
                    || offer
                        .detail
                        .as_ref()
                        .is_some_and(|detail| detail.to_lowercase().contains(&needle))
            })
            .map(|(token, offer)| {
                let held = self.holding(&offer.filter);
                PopoverRow {
                    token: token as u64,
                    share: offer.count as f64 / largest as f64,
                    checked: held.is_some_and(|(_, negated)| !negated),
                    excluded: held.is_some_and(|(_, negated)| negated),
                    title: offer.title,
                    detail: offer.detail,
                    initials: offer.initials,
                    color: offer.color,
                    count_label: words::facet_count(offer.count, capped),
                    count: offer.count,
                }
            })
            .collect();
        let date = popover.kind == FilterKind::Date;
        let base = popover.base.as_ref();
        let (after, before) = self.dates();
        let presets = if date {
            let capped = base.is_some_and(|base| base.facets.capped);
            let mut presets: Vec<DatePresetView> = words::date_presets(today)
                .iter()
                .enumerate()
                .map(|(token, preset)| DatePresetView {
                    token: token as u64,
                    label: preset.label.to_owned(),
                    count: base
                        .map(|base| words::tab_count(base.facets.presets[preset.count_at], capped)),
                    selected: before.is_none() && after == preset.start,
                })
                .collect();
            presets.push(DatePresetView {
                token: presets.len() as u64,
                label: words::CUSTOM_DATE.to_owned(),
                count: None,
                selected: false,
            });
            presets
        } else {
            Vec::new()
        };
        let frame = self.frame.as_ref().filter(|_| self.ready());
        Some(PopoverView {
            kind: popover.kind,
            placeholder: words::popover_placeholder(popover.kind).to_owned(),
            filter: popover.filter.clone(),
            rows,
            hints: words::popover_hints(popover.kind),
            presets,
            words: popover.words.clone(),
            parsed: date
                .then(|| date_terms(&popover.words, today))
                .flatten()
                .map(|(_, _, terms)| words::date_parsed(&terms)),
            words_hint: if date {
                words::DATE_WORDS_HINT.to_owned()
            } else {
                String::new()
            },
            months: if date { self.month_bars() } else { Vec::new() },
            result: date
                .then(|| {
                    let frame = frame?;
                    let of = base?.facets.presets[4];
                    Some(words::date_result(frame.total, of, frame.capped))
                })
                .flatten(),
            range: date.then(|| self.range(today, true)).flatten(),
        })
    }

    fn popover_toggle(
        &self,
        token: u64,
        exclude: bool,
        today: NaiveDate,
    ) -> Option<postio_search::edit::Edit> {
        use postio_search::edit::Edit;
        let offer = self
            .offers(today)
            .into_iter()
            .nth(usize::try_from(token).ok()?)?;
        Some(if exclude {
            Edit::Exclude(offer.filter)
        } else {
            Edit::Toggle(offer.filter)
        })
    }

    fn month_bars(&self) -> Vec<MonthBar> {
        let Some(frame) = self.frame.as_ref() else {
            return Vec::new();
        };
        let (after, before) = self.dates();
        let undated = self
            .undated
            .as_ref()
            .filter(|(query, _, _)| *query == undated_query(&self.parsed));
        let (months, capped) = undated.map_or(
            (&frame.facets.months, frame.facets.capped),
            |(_, months, capped)| (months, *capped),
        );
        let tallest = months
            .iter()
            .map(|month| month.conversations)
            .max()
            .unwrap_or(0)
            .max(1);
        months
            .iter()
            .map(|month| MonthBar {
                label: month.month.format("%b").to_string(),
                conversations: month.conversations,
                count: words::facet_count(month.conversations, capped),
                height: month.conversations as f64 / tallest as f64,
                selected: (after.is_some() || before.is_some())
                    && after.is_none_or(|after| {
                        month
                            .month
                            .checked_add_months(chrono::Months::new(1))
                            .is_some_and(|end| end > after)
                    })
                    && before.is_none_or(|before| month.month < before),
            })
            .collect()
    }

    fn range(&self, today: NaiveDate, year: bool) -> Option<String> {
        let (first, last) = self.range_months(today)?;
        Some(words::month_range(first, last, year))
    }

    fn range_months(&self, today: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
        let (after, before) = self.dates();
        if after.is_none() && before.is_none() {
            return None;
        }
        let last = before.and_then(|before| before.pred_opt()).unwrap_or(today);
        let first = after.unwrap_or_else(|| {
            self.frame
                .as_ref()
                .map_or(last, |frame| frame.facets.months[0].month)
        });
        Some((first, last))
    }

    fn stepped(
        &self,
        back: bool,
        today: NaiveDate,
    ) -> Option<(Option<NaiveDate>, Option<NaiveDate>)> {
        use chrono::{Datelike, Months};
        let this = today.with_day(1)?;
        let next = this.checked_add_months(Months::new(1))?;
        let (after, before) = self.dates();
        if after.is_none() && before.is_none() {
            return back.then_some((Some(this), Some(next)));
        }
        let before = before.or(after.map(|_| next));
        let shift = |day: NaiveDate| {
            if back {
                day.checked_sub_months(Months::new(1))
            } else {
                day.checked_add_months(Months::new(1))
            }
        };
        let after = match after {
            Some(day) => Some(shift(day)?),
            None => None,
        };
        let before = match before {
            Some(day) => Some(shift(day)?),
            None => None,
        };
        if !back && (after.is_some_and(|day| day > this) || before.is_some_and(|day| day > next)) {
            return None;
        }
        Some((after, before))
    }

    fn dates(&self) -> (Option<NaiveDate>, Option<NaiveDate>) {
        dates_of(&self.parsed)
    }

    pub(crate) fn months_to_dates(&self, first: u32, last: u32) -> Option<(NaiveDate, NaiveDate)> {
        let months = &self.frame.as_ref()?.facets.months;
        let (first, last) = (first.min(last) as usize, first.max(last) as usize);
        let from = months.get(first)?.month;
        let to = months
            .get(last)?
            .month
            .checked_add_months(chrono::Months::new(1))?;
        Some((from, to))
    }

    pub(crate) fn row(&self, position: u64, with: &Words<'_>) -> Option<ResultRow> {
        let (hit, group, top_hit) = self.hit_at(position)?;
        let names = self.frame.as_ref().map(|frame| &frame.names);
        let sender = hit
            .from
            .as_ref()
            .map(postio_ui::command_bar::said_of)
            .unwrap_or_default();
        let from_address = hit.from.as_ref().map(|from| from.address.clone());
        let subject = hit.subject.clone().unwrap_or_default();
        let shown = words::shown_match(&hit.matches);
        let mut sources: Vec<Source> = shown.map(|each| each.source.clone()).into_iter().collect();
        sources.extend(hit.matches.iter().map(|each| each.source.clone()));
        let source_tag = words::sources_tag(&sources);
        let source_is_file = matches!(
            sources.first(),
            Some(Source::FileName { .. } | Source::FileContent { .. })
        );
        let mut passage = Vec::new();
        if let Some(each) = shown
            && let Some(cut) = &each.passage
        {
            if let Source::FileContent { location, .. } = &each.source {
                passage.push(Run::plain(format!("{}: ", words::location(location))));
            }
            passage.extend(dropdown::passage(cut));
        }
        let folder_name = names
            .and_then(|names| {
                names
                    .folders
                    .iter()
                    .find(|(id, _)| *id == hit.mailbox_id)
                    .map(|(_, name)| name.clone())
            })
            .or_else(|| {
                with.folders
                    .iter()
                    .find(|(id, _)| *id == hit.mailbox_id)
                    .map(|(_, name)| name.clone())
            });
        let held = self
            .held
            .iter()
            .find(|(message, _, _)| *message == hit.best)
            .map(|(_, rule, delivered)| (rule.as_str(), *delivered));
        let account = with
            .owners
            .iter()
            .find(|(id, _)| *id == hit.mailbox_id)
            .map(|(_, account)| account.as_str());
        let spoken_place = [
            held.map(|(rule, delivered)| postio_ui::digest::held_place(rule, delivered))
                .or_else(|| folder_name.as_deref().map(words::folder_name)),
            account.map(str::to_owned),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");
        let date = words::hit_date(hit.newest_match.with_timezone(&Local), with.now);
        let passage_text: String = passage.iter().map(|run| run.text.as_str()).collect();
        // A row that says "K of M messages match" has said its count: the
        // reason line's matches, which count the subject once too, would
        // be a second number beside it.
        let reasons: Vec<RankReason> = if matched_messages(hit).is_some() {
            hit.reasons
                .iter()
                .filter(|reason| !matches!(reason, RankReason::Matches(_)))
                .cloned()
                .collect()
        } else {
            hit.reasons.clone()
        };
        let reason = top_hit
            .then(|| words::reason_beside(&reasons, &sources))
            .filter(|line| !line.is_empty());
        Some(ResultRow {
            message: hit.best,
            thread: thread_of(hit),
            group,
            top_hit,
            from_address,
            accessible: words::accessible_row(
                &sender,
                &subject,
                &source_tag,
                &passage_text,
                &spoken_place,
                &date,
            ),
            unread: hit.unread,
            reason,
            subject: dropdown::marked(
                &subject,
                &postio_search::highlight::find(&subject, &self.terms),
            ),
            sender,
            labels: hit
                .labels
                .iter()
                .filter_map(|label| {
                    let names = names?;
                    let name = names.labels.iter().find(|(id, _)| id == label)?;
                    Some(LabelPill {
                        name: name.1.clone(),
                        color: names
                            .label_colors
                            .iter()
                            .find(|(id, _)| id == label)
                            .map(|(_, color)| color.clone()),
                    })
                })
                .collect(),
            attachments: hit.has_attachments,
            count_badge: (hit.messages > 1).then(|| hit.messages.to_string()),
            summary: match matched_messages(hit) {
                Some(matched) => Some(postio_ui::search_view::match_summary(
                    matched,
                    hit.messages as usize,
                )),
                None => single_match(hit).then(|| {
                    let who = hit
                        .from
                        .as_ref()
                        .map(postio_ui::focus_row::first_name)
                        .unwrap_or_default();
                    postio_ui::search_view::single_match_snippet(
                        hit.best_number as usize,
                        hit.messages as usize,
                        who,
                        &passage_text,
                    )
                }),
            },
            source_tag,
            source_is_file,
            passage,
            folder: words::hit_place(held, folder_name.as_deref(), account).unwrap_or_default(),
            date,
            checked: self.is_checked(hit.best),
        })
    }

    fn look(&mut self, stamp: u64) -> Option<Request> {
        let position = self.cursor?;
        let (hit, _, _) = self.hit_at(position)?;
        let hit = hit.clone();
        let key = hit.key;
        self.quick_look = Some(QuickLook {
            position,
            hit,
            matches: None,
            current: 0,
            stepped: false,
            stamp,
        });
        Some(Request::QuickLookMatches {
            query: self.parsed.clone(),
            key,
            stamp,
        })
    }

    fn matches_landed(
        &mut self,
        stamp: u64,
        answer: Result<Vec<ConversationMatch>, String>,
    ) -> bool {
        let Some(look) = self.quick_look.as_mut().filter(|look| look.stamp == stamp) else {
            return false;
        };
        let found = match answer {
            Ok(found) if !found.is_empty() => found,
            // Nothing read: the row's own matches stay, which are true.
            Ok(_) => return false,
            Err(error) => {
                tracing::debug!(%error, "a conversation's matches could not be read");
                return false;
            }
        };
        // The ring stays on the card the person moved it to, when the
        // conversation has it too: an answer landing after a ] must not
        // take the step back (#2132). Otherwise, the row's own message.
        let ringed = look
            .stepped
            .then(|| look.matches().get(look.current).cloned())
            .flatten();
        look.current = ringed
            .and_then(|ringed| {
                found.iter().position(|each| {
                    each.message == ringed.message && each.found.source == ringed.found.source
                })
            })
            .or_else(|| {
                found
                    .iter()
                    .position(|each| each.message == Some(look.hit.best))
            })
            .unwrap_or(0);
        look.matches = Some(found);
        true
    }

    fn step_match(&mut self, by: i64) -> bool {
        let Some(look) = self.quick_look.as_mut() else {
            return false;
        };
        let cards = look.matches().len();
        if cards == 0 {
            return false;
        }
        let next = (look.current as i64 + by).clamp(0, cards as i64 - 1) as usize;
        if next == look.current {
            return false;
        }
        look.current = next;
        look.stepped = true;
        true
    }

    fn quick_look_view(&self, with: &Words<'_>) -> Option<QuickLookView> {
        use crate::dropdown::RunStyle;
        let look = self.quick_look.as_ref()?;
        let hit = &look.hit;
        let matches = look.matches();
        let subject = hit.subject.clone().unwrap_or_default();
        let run = |text: String, style: RunStyle| Run {
            text,
            highlighted: false,
            style,
        };

        let mut sender = Vec::new();
        if let Some(from) = &hit.from {
            sender.push(run(postio_ui::command_bar::said_of(from), RunStyle::Strong));
            if from
                .name
                .as_deref()
                .is_some_and(|name| !name.trim().is_empty())
            {
                sender.push(run(" ".to_owned(), RunStyle::Plain));
                sender.push(run(from.address.clone(), RunStyle::Mono));
            }
        }
        let own = matches.iter().find(|each| each.message == Some(hit.best));
        let sent = own
            .and_then(|each| each.found.when)
            .unwrap_or(hit.newest_match);
        let tail: Vec<String> = own
            .and_then(|each| words::sent_to(&each.to))
            .into_iter()
            .chain(std::iter::once(words::sent_at(
                sent.with_timezone(&Local),
                with.now,
            )))
            .chain(words::thread_size(hit.messages))
            .collect();
        let tail = tail.join(" \u{b7} ");
        sender.push(run(
            if sender.is_empty() {
                tail
            } else {
                format!(" \u{b7} {tail}")
            },
            RunStyle::Plain,
        ));

        let cards: Vec<MatchCard> = matches
            .iter()
            .map(|each| {
                let passage = match (&each.found.passage, &each.found.source) {
                    (Some(cut), _) => dropdown::passage(cut),
                    (None, Source::Subject) => dropdown::marked(
                        &subject,
                        &postio_search::highlight::find(&subject, &self.terms),
                    ),
                    (None, _) => Vec::new(),
                };
                MatchCard {
                    place: words::match_place(&each.found.source),
                    when: words::match_when(
                        &each.found.source,
                        each.from.as_ref(),
                        each.found.when.map(|when| when.with_timezone(&Local)),
                        with.now,
                    ),
                    passage,
                    file: matches!(
                        each.found.source,
                        Source::FileName { .. } | Source::FileContent { .. }
                    ),
                }
            })
            .collect();
        Some(QuickLookView {
            title: words::QUICK_LOOK.to_owned(),
            position: {
                let (at, of) = self.walked_number(look.position);
                words::quick_look_position(at, of)
            },
            walk: words::quick_look_walk(with.keymap),
            actions: words::quick_look_actions(with.keymap),
            subject: dropdown::marked(
                &subject,
                &postio_search::highlight::find(&subject, &self.terms),
            ),
            sender,
            matches_line: words::matches_line(words::matches_of(matches.iter().map(|each| {
                (!matches!(each.found.source, Source::Subject)).then_some(each.message)
            }))),
            matches_hint: words::matches_hint(with.keymap),
            current: (!cards.is_empty()).then(|| look.current.min(cards.len() - 1) as u32),
            cards,
        })
    }

    pub(crate) fn snapshot(&self) -> crate::history::Snapshot {
        crate::history::Snapshot {
            // A popover's preview is not kept: what it opened on is.
            query: self
                .popover
                .as_ref()
                .map_or_else(|| self.query.clone(), |popover| popover.before.clone()),
            tab: self.tab,
            order: self.order,
            cursor: self.cursor,
            checked: self.checked.clone(),
            all: self.all,
        }
    }
}

pub(crate) struct QueryNames<'a> {
    pub(crate) lookup: &'a dyn Fn(&str) -> Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct Landed {
    pub(crate) frame: bool,
    pub(crate) rows: Option<(u64, u64)>,
    pub(crate) passages: Option<Request>,
    pub(crate) remember: Option<(u64, bool)>,
    pub(crate) nothing: bool,
}

fn matched_messages(hit: &ConversationHit) -> Option<usize> {
    // The messages, not the reason line's matches, which count the subject
    // once too: the row's K is its expansion's children (#2184).
    let matched = hit.matched_messages as usize;
    (hit.messages > 1 && matched > 1).then_some(matched)
}

fn single_match(hit: &ConversationHit) -> bool {
    matches!(hit.key, ConversationKey::Thread(_))
        && hit.messages > 1
        && matched_messages(hit).is_none()
}

fn thread_of(hit: &ConversationHit) -> Option<ThreadId> {
    match hit.key {
        ConversationKey::Thread(thread) => Some(thread),
        ConversationKey::Lone(_) => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveView {
    pub title: String,
    pub name_label: String,
    pub name: String,
    pub chips: Vec<String>,
    pub pin: bool,
    pub pin_label: String,
    pub pin_note: String,
    /// The key it will run on (`alt+3`), the next free; `None` past the
    /// fourth.
    pub pin_key: Option<String>,
    pub notify: bool,
    pub notify_label: String,
    pub notify_note: String,
    pub rolling: bool,
    pub rolling_label: String,
    /// "Off: always since 1 July. On: always the last 90 days"; `None`
    /// when the query has no date to keep, and the switch is not drawn.
    pub rolling_note: Option<String>,
    pub cancel: String,
    pub save: String,
    pub save_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ResultsAim {
    Targets {
        aims: Vec<postio_core::MessageTarget>,
        conversations: usize,
    },
    Matching {
        query: ParsedQuery,
        except: Vec<MessageId>,
    },
}

pub(crate) fn results_send(command: postio_core::Command, aim: ResultsAim) -> Request {
    match aim {
        ResultsAim::Targets { aims, .. } => Request::Send {
            command,
            aims,
            everything: None,
        },
        ResultsAim::Matching { query, except } => Request::Send {
            command,
            aims: Vec::new(),
            everything: Some(crate::Everything {
                accounts: Vec::new(),
                except,
                query: Some(query),
            }),
        },
    }
}

pub(crate) fn show(view: ResultsView) -> Step {
    Step::Show(Intent::ShowResults(Box::new(view)))
}

const RESULTS_KEYS: [CommandId; 33] = [
    CommandId::NextMessage,
    CommandId::PrevMessage,
    CommandId::FirstMessage,
    CommandId::LastMessage,
    CommandId::ToggleSelection,
    CommandId::SelectAll,
    CommandId::OpenMessage,
    CommandId::Back,
    CommandId::HistoryBack,
    CommandId::HistoryForward,
    CommandId::ResultsConversations,
    CommandId::ResultsFiles,
    CommandId::ResultsPeople,
    CommandId::FilterFrom,
    CommandId::FilterTo,
    CommandId::FilterDate,
    CommandId::FilterAnywhere,
    CommandId::FilterLabel,
    CommandId::ToggleResultOrder,
    CommandId::ToggleHasAction,
    CommandId::SaveSearch,
    CommandId::StepRangeBack,
    CommandId::StepRangeForward,
    CommandId::QuickLook,
    CommandId::NextMatch,
    CommandId::PrevMatch,
    CommandId::PickRelaxation1,
    CommandId::PickRelaxation2,
    CommandId::PickRelaxation3,
    CommandId::PickRelaxation4,
    CommandId::SaveFile,
    CommandId::ExpandThread,
    CommandId::CollapseThread,
];

pub(crate) fn results_key(id: CommandId) -> bool {
    RESULTS_KEYS.contains(&id)
        || id == CommandId::ContactPopover
        || matches!(
            id,
            CommandId::Reply | CommandId::ReplyAll | CommandId::Forward
        )
        || postio_ui::focus_target::dispatch(id).is_some()
        || crate::pickers::opens_picker(id)
}

impl FocusController {
    pub fn in_results(&self) -> bool {
        self.results.is_some()
    }

    pub(crate) fn results_query(&self) -> Option<String> {
        self.results.as_ref().map(|results| results.query.clone())
    }

    pub fn result_count(&self) -> u64 {
        self.results.as_ref().map_or(0, Results::rows)
    }

    pub fn result_row(&self, position: u64) -> Option<ResultRow> {
        let words = self.results_words();
        self.results.as_ref()?.row(position, &words)
    }

    pub fn result_file(&self, position: u64) -> Option<FileCard> {
        let words = self.results_words();
        self.results.as_ref()?.file_card(position, &words)
    }

    pub fn result_person(&self, position: u64) -> Option<PersonRow> {
        let words = self.results_words();
        self.results.as_ref()?.person_row(position, &words)
    }

    pub fn results_wanted(&mut self, position: u64) -> Vec<Effect> {
        let steps = match self.results.as_mut() {
            Some(results) => results.wanted(position),
            None => Vec::new(),
        };
        self.effects(steps)
    }

    fn results_words(&self) -> Words<'_> {
        Words {
            keymap: self.bar.keymap(),
            now: self.bar.now(),
            folders: self.bar.folders(),
            owners: self.bar.owners(),
        }
    }

    fn draw_results(&self) -> Vec<Step> {
        let Some(results) = &self.results else {
            return Vec::new();
        };
        let words = self.results_words();
        vec![
            show(results.view(&words)),
            Step::Show(Intent::Query(self.query_view())),
        ]
    }

    fn query_view(&self) -> QueryView {
        let lookup = |address: &str| self.bar.name_for(address);
        let names = QueryNames { lookup: &lookup };
        let Some(results) = self.results.as_ref() else {
            return QueryView {
                chips: Vec::new(),
                words: String::new(),
                hint: String::new(),
                hint_key: None,
                ringed: false,
                buttons: Vec::new(),
            };
        };
        let mut view = results.query_view(self.bar.now().date_naive(), &names);
        for button in &mut view.buttons {
            button.key = filter_command(button.kind)
                .and_then(|command| postio_ui::hints::key(self.bar.keymap(), command));
        }
        view.ringed = results.found_nothing();
        if results.found_nothing()
            && let Some(key) = postio_ui::hints::key(self.bar.keymap(), CommandId::BackToWords)
        {
            view.hint = words::CLEARS_FILTERS.to_owned();
            view.hint_key = Some(key);
        }
        view
    }

    fn draw_no_results(&self) -> Vec<Step> {
        let lookup = |address: &str| self.bar.name_for(address);
        let names = QueryNames { lookup: &lookup };
        self.results
            .as_ref()
            .and_then(|results| {
                results.no_results_view(self.bar.keymap(), self.bar.now().date_naive(), &names)
            })
            .map(|view| Step::Show(Intent::Relaxations(Some(Box::new(view)))))
            .into_iter()
            .collect()
    }

    fn pick_way_out(&mut self, at: usize) -> Vec<Step> {
        let Some(query) = self
            .results
            .as_ref()
            .and_then(|results| results.way_out(at))
        else {
            return Vec::new();
        };
        let stamp = self.stamp();
        let parsed = self.bar.lower(&query);
        let Some(results) = self.results.as_mut() else {
            return Vec::new();
        };
        results.set_query(query, parsed);
        let asks = results.again(stamp);
        let mut steps = self.close_quick_look();
        steps.push(Step::Show(Intent::Query(self.query_view())));
        steps.extend(asks);
        steps
    }

    fn way_out_moved(&mut self, by: i64) -> Vec<Step> {
        if !self
            .results
            .as_mut()
            .is_some_and(|results| results.step_way_out(by))
        {
            return Vec::new();
        }
        let mut steps = self.draw_no_results();
        steps.push(Step::Show(Intent::Query(self.query_view())));
        steps
    }

    fn here(&self) -> Entry {
        match &self.results {
            Some(results) => Entry::Results(results.snapshot()),
            None => Entry::Inbox,
        }
    }

    pub(crate) fn show_all_results(&mut self) -> Vec<Step> {
        let Some(typed) = self.bar.search_words().map(str::to_owned) else {
            return Vec::new();
        };
        let leaving = self.here();
        self.history.visit(leaving);
        let parsed = self.bar.lower(&typed);
        let query = spelled(&parsed);
        let order = Results::default_order(&parsed);
        // Kept among the recent searches now when the dropdown has counted
        // it, so leaving before the results land still keeps it; else once
        // they land.
        let remembered = self.bar.remember();
        let mut steps = self.dismiss_bar(true);
        steps.extend(self.open_results(Snapshot {
            query,
            tab: ResultsTab::Conversations,
            order,
            cursor: None,
            checked: Vec::new(),
            all: false,
        }));
        if let Some(step) = remembered {
            if let Some(results) = self.results.as_mut() {
                results.remembered = true;
            }
            steps.push(step);
        }
        steps
    }

    fn open_results(&mut self, snapshot: Snapshot) -> Vec<Step> {
        let mut steps = Vec::new();
        if self.results.as_ref().is_some_and(Results::found_nothing) {
            steps.push(Step::Show(Intent::Relaxations(None)));
        }
        let parsed = self.bar.lower(&snapshot.query);
        let stamp = self.stamp();
        let mut results = Results::new(snapshot.query, parsed, snapshot.tab, snapshot.order, stamp);
        results.cursor = snapshot.cursor;
        results.checked = snapshot.checked;
        results.all = snapshot.all;
        let asks = results.asks();
        self.results = Some(results);
        steps.extend(self.draw_results());
        steps.extend(asks);
        steps
    }

    fn show_inbox(&mut self) -> Vec<Step> {
        let mut steps = self.close_quick_look();
        self.results = None;
        steps.push(Step::Show(Intent::LeaveResults));
        steps.extend(self.cursor.redraw(self.feed.total()));
        steps
    }

    fn show_entry(&mut self, entry: Entry) -> Vec<Step> {
        let mut steps = self.close_quick_look();
        if self
            .results
            .as_ref()
            .is_some_and(|results| results.popover.is_some())
        {
            steps.push(Step::Show(Intent::Popover(None)));
        }
        steps.extend(match entry {
            Entry::Inbox => self.show_inbox(),
            Entry::Results(snapshot) => self.open_results(snapshot),
        });
        steps
    }

    pub(crate) fn history_command(&mut self, id: CommandId) -> Option<Vec<Step>> {
        let here = self.here();
        let to = match id {
            CommandId::HistoryBack => self.history.back(here),
            CommandId::HistoryForward => self.history.forward(here),
            _ => return None,
        };
        Some(to.map(|entry| self.show_entry(entry)).unwrap_or_default())
    }

    /// `None` hands it on to the list's table (`/`, `c`, `?`, the palette).
    pub(crate) fn results_command(&mut self, id: CommandId) -> Option<Vec<Step>> {
        if let Some(steps) = self.files_command(id) {
            return Some(steps);
        }
        if let Some(steps) = self.people_command(id) {
            return Some(steps);
        }
        if let Some(steps) = self.quick_look_command(id) {
            return Some(steps);
        }
        if id == CommandId::ContactPopover {
            let address = self.results.as_ref()?.cursor_sender();
            return Some(
                address
                    .map(|address| self.open_contact_popover(Some(address), &crate::cursor::NoRows))
                    .unwrap_or_default(),
            );
        }
        let results = self.results.as_mut()?;
        if results.found_nothing() {
            let steps = match id {
                CommandId::NextMessage | CommandId::PrevMessage => {
                    Some(self.way_out_moved(if id == CommandId::NextMessage { 1 } else { -1 }))
                }
                CommandId::FirstMessage => Some(self.way_out_moved(i64::MIN / 2)),
                CommandId::LastMessage => Some(self.way_out_moved(i64::MAX / 2)),
                CommandId::OpenMessage => {
                    let at = results.none.as_ref().map_or(0, |none| none.focus);
                    Some(self.pick_way_out(at))
                }
                CommandId::BackToWords => {
                    Some(self.results_edit(postio_search::edit::Edit::ClearFilters))
                }
                _ => None,
            };
            if steps.is_some() {
                return steps;
            }
        }
        let results = self.results.as_mut()?;
        let steps = match id {
            CommandId::PickRelaxation1 => self.pick_way_out(0),
            CommandId::PickRelaxation2 => self.pick_way_out(1),
            CommandId::PickRelaxation3 => self.pick_way_out(2),
            CommandId::PickRelaxation4 => self.pick_way_out(3),
            CommandId::NextMessage | CommandId::PrevMessage => {
                let by = if id == CommandId::NextMessage { 1 } else { -1 };
                self.results_step(by)
            }
            CommandId::FirstMessage => self.results_step(i64::MIN / 2),
            CommandId::LastMessage => self.results_step(i64::MAX / 2),
            CommandId::ExpandThread => self.expand_result(),
            CommandId::CollapseThread => self.collapse_result(),
            // A conversation can be drawn twice -- a Top hit and its
            // month's row -- so every row redraws, which the toolkit does
            // for the ones on screen.
            CommandId::ToggleSelection => {
                if results.toggle_checked() {
                    self.redraw_checks()
                } else {
                    Vec::new()
                }
            }
            CommandId::SelectAll => {
                results.check_all();
                self.redraw_checks()
            }
            CommandId::Back if results.popover.is_some() => self.popover_done(false),
            CommandId::Back => {
                if !results.any_checked() {
                    let here = self.here();
                    self.history.home(here);
                    return Some(self.show_inbox());
                }
                results.clear_checked();
                self.redraw_checks()
            }
            CommandId::HistoryBack | CommandId::HistoryForward => {
                return self.history_command(id);
            }
            CommandId::ResultsConversations => self.results_tab(ResultsTab::Conversations),
            CommandId::ResultsFiles => self.results_tab(ResultsTab::Files),
            CommandId::ResultsPeople => self.results_tab(ResultsTab::People),
            CommandId::FilterFrom
            | CommandId::FilterTo
            | CommandId::FilterDate
            | CommandId::FilterAnywhere
            | CommandId::FilterLabel => {
                match FilterKind::ALL
                    .into_iter()
                    .find(|kind| filter_command(*kind) == Some(id))
                {
                    Some(kind) => self.open_popover(kind),
                    None => Vec::new(),
                }
            }
            CommandId::ToggleResultOrder => {
                let order = match results.order {
                    ConversationOrder::BestMatch => ConversationOrder::Newest,
                    _ => ConversationOrder::BestMatch,
                };
                self.results_order(order)
            }
            CommandId::ToggleHasAction => {
                self.results_edit(postio_search::edit::Edit::Toggle(Filter::HasAction))
            }
            // Nothing is written until the Save popover's Save.
            CommandId::SaveSearch => self
                .save_view()
                .map(|view| vec![Step::Show(Intent::SavePopover(Some(Box::new(view))))])
                .unwrap_or_default(),
            CommandId::StepRangeBack | CommandId::StepRangeForward => {
                let today = self.bar.now().date_naive();
                let back = id == CommandId::StepRangeBack;
                match results.stepped(back, today) {
                    Some((after, before)) => {
                        self.results_edit(postio_search::edit::Edit::SetDates { after, before })
                    }
                    None => Vec::new(),
                }
            }
            CommandId::OpenMessage => self.open_result(),
            CommandId::Reply | CommandId::ReplyAll | CommandId::Forward => {
                let kind = match id {
                    CommandId::Reply => crate::ComposerKind::Reply,
                    CommandId::ReplyAll => crate::ComposerKind::ReplyAll,
                    _ => crate::ComposerKind::Forward {
                        form: postio_model::ForwardForm::This,
                    },
                };
                match results.cursor_message() {
                    Some((message, _)) => self.write(kind, Some(message)),
                    None => Vec::new(),
                }
            }
            _ if crate::pickers::opens_picker(id) => self.picker_on_results(id).unwrap_or_default(),
            // A verb on nothing (Undo, Refresh) goes on to the list's path, which sends it as it
            // is:
            // ⌘Z after archiving a result takes it back.
            _ if matches!(
                postio_ui::focus_target::dispatch(id),
                Some(postio_ui::focus_target::Dispatch::OnMail(_))
            ) =>
            {
                self.results_verb(id)
            }
            _ => return None,
        };
        Some(steps)
    }

    /// Search spans every folder, so an archived result still matches; the focus ring stays where
    /// it was.
    pub(crate) fn results_heard(&mut self, event: &postio_core::Event) -> Vec<Step> {
        if !matches!(
            event,
            postio_core::Event::ActionCompleted { .. }
                | postio_core::Event::UndoPerformed { .. }
                | postio_core::Event::MessagesChanged { .. }
        ) {
            return Vec::new();
        }
        let stamp = self.stamp();
        match self.results.as_mut() {
            // The first page is still out: one more read once it lands.
            Some(results) if results.frame.is_none() && results.asked.contains(&0) => {
                results.stale = true;
                Vec::new()
            }
            Some(results) => results.again(stamp),
            None => Vec::new(),
        }
    }

    pub(crate) fn clear_results_checks(&mut self) -> Vec<Step> {
        match self.results.as_mut() {
            Some(results) if results.any_checked() => {
                results.clear_checked();
                self.redraw_checks()
            }
            _ => Vec::new(),
        }
    }

    fn save_view(&self) -> Option<SaveView> {
        let results = self.results.as_ref()?;
        if results.query.trim().is_empty() {
            return None;
        }
        let today = self.bar.now().date_naive();
        let query = self.query_view();
        let mut chips: Vec<String> = query
            .chips
            .iter()
            .map(|chip| {
                let not = if chip.excluded { "-" } else { "" };
                format!("{not}{}{}", chip.operator, chip.value)
            })
            .collect();
        if !query.words.trim().is_empty() {
            chips.push(query.words.trim().to_owned());
        }
        let lookup = |address: &str| results.name_of(address);
        let pin_key = postio_ui::command_bar::SAVED
            .get(self.bar.saved_len())
            .and_then(|id| postio_ui::hints::key(self.bar.keymap(), *id));
        let after = results
            .parsed
            .filters()
            .find_map(|clause| match clause.filter {
                postio_search::query::Filter::After(date) if !clause.negated => Some(date),
                _ => None,
            });
        Some(SaveView {
            title: words::SAVE_TITLE.to_owned(),
            name_label: words::SAVE_NAME.to_owned(),
            name: words::save_name(&results.parsed, &lookup),
            chips,
            pin: true,
            pin_label: words::SAVE_PIN.to_owned(),
            pin_note: if pin_key.is_some() {
                words::SAVE_PIN_NOTE.to_owned()
            } else {
                words::SAVE_PIN_NOTE_NO_KEY.to_owned()
            },
            pin_key,
            notify: false,
            notify_label: words::SAVE_NOTIFY.to_owned(),
            notify_note: words::SAVE_NOTIFY_NOTE.to_owned(),
            rolling: false,
            rolling_label: words::SAVE_ROLLING.to_owned(),
            rolling_note: after.map(|after| words::rolling_note(after, today)),
            cancel: words::SAVE_CANCEL.to_owned(),
            save: words::SAVE.to_owned(),
            save_key: postio_ui::hints::key(self.bar.keymap(), CommandId::OpenMessage),
        })
    }

    fn save_as(&mut self, name: String, pin: bool, notify: bool, rolling: bool) -> Vec<Step> {
        let Some(results) = self.results.as_ref() else {
            return Vec::new();
        };
        let today = self.bar.now().date_naive();
        let name = name.trim();
        let save = crate::SaveSearch {
            query: results.query.clone(),
            name: (!name.is_empty()).then(|| name.to_owned()),
            pin,
            notify,
            dates: if rolling {
                postio_ui::saved_search::Dates::Rolling { today }
            } else {
                postio_ui::saved_search::Dates::Fixed { today }
            },
        };
        self.bar.set_saving(save.clone());
        vec![
            Step::Show(Intent::SavePopover(None)),
            Step::Show(Intent::SaveSearch(save)),
        ]
    }

    fn redraw_checks(&self) -> Vec<Step> {
        let rows = self.results.as_ref().map_or(0, Results::rows);
        let mut steps = self.draw_results();
        steps.push(Step::Show(Intent::ResultsPage {
            first: 0,
            count: rows,
        }));
        steps
    }

    /// What was checked has been acted on, so it goes.
    fn results_verb(&mut self, id: CommandId) -> Vec<Step> {
        let Some(postio_ui::focus_target::Dispatch::OnMail(command)) =
            postio_ui::focus_target::dispatch(id)
        else {
            return Vec::new();
        };
        let Some(results) = self.results.as_mut() else {
            return Vec::new();
        };
        if !results.any_checked() {
            return self.result_verb(id);
        }
        let Some(aim) = results.aim() else {
            return Vec::new();
        };
        results.clear_checked();
        let mut steps = vec![Step::Ask(results_send(command, aim))];
        steps.extend(self.redraw_checks());
        steps
    }

    fn result_verb(&mut self, id: CommandId) -> Vec<Step> {
        let Some((message, thread)) = self.results.as_ref().and_then(Results::cursor_message)
        else {
            return Vec::new();
        };
        let row = crate::RowFacts {
            id: message,
            digest: false,
            threads: thread.into_iter().collect(),
            writes: false,
        };
        self.verbs.command_on(id, &row).unwrap_or_default()
    }

    fn quick_look_command(&mut self, id: CommandId) -> Option<Vec<Step>> {
        let results = self.results.as_mut()?;
        let open = results.quick_look.is_some();
        let steps = match id {
            CommandId::QuickLook if open => self.close_quick_look(),
            CommandId::QuickLook => self.look(),
            CommandId::NextMatch | CommandId::PrevMatch => {
                let by = if id == CommandId::NextMatch { 1 } else { -1 };
                if results.step_match(by) {
                    self.draw_quick_look()
                } else {
                    Vec::new()
                }
            }
            _ if !open => return None,
            CommandId::NextMessage
            | CommandId::PrevMessage
            | CommandId::FirstMessage
            | CommandId::LastMessage => {
                let by = match id {
                    CommandId::NextMessage => 1,
                    CommandId::PrevMessage => -1,
                    CommandId::FirstMessage => i64::MIN / 2,
                    _ => i64::MAX / 2,
                };
                let moved = results.step(by);
                let mut steps = cursor_moved(moved);
                if moved.is_some() {
                    steps.extend(self.look());
                }
                steps
            }
            CommandId::OpenMessage => {
                let mut steps = self.close_quick_look();
                steps.extend(self.open_result());
                steps
            }
            CommandId::Back => self.close_quick_look(),
            CommandId::Archive | CommandId::Delete => {
                let mut steps = self.result_verb(id);
                let moved = self.results.as_mut().and_then(|results| results.step(1));
                steps.extend(cursor_moved(moved));
                if moved.is_some() {
                    steps.extend(self.look());
                } else {
                    steps.extend(self.close_quick_look());
                }
                steps
            }
            _ => return None,
        };
        Some(steps)
    }

    fn look(&mut self) -> Vec<Step> {
        let stamp = self.stamp();
        let Some(results) = self.results.as_mut() else {
            return Vec::new();
        };
        match results.look(stamp) {
            Some(request) => {
                let mut steps = self.draw_quick_look();
                steps.push(Step::Ask(request));
                steps
            }
            None => self.close_quick_look(),
        }
    }

    fn draw_quick_look(&self) -> Vec<Step> {
        let words = self.results_words();
        self.results
            .as_ref()
            .and_then(|results| results.quick_look_view(&words))
            .map(|view| Step::Show(Intent::QuickLook(Some(Box::new(view)))))
            .into_iter()
            .collect()
    }

    pub(crate) fn close_quick_look(&mut self) -> Vec<Step> {
        let mut steps = self.close_file_copy();
        if let Some(results) = self.results.as_mut()
            && results.quick_look.take().is_some()
        {
            steps.push(Step::Show(Intent::QuickLook(None)));
        }
        steps
    }

    fn open_result(&mut self) -> Vec<Step> {
        let Some(results) = self.results.as_mut() else {
            return Vec::new();
        };
        let Some((message, _)) = results.cursor_message() else {
            return Vec::new();
        };
        let mut walk = results.walk();
        let mut target = message;
        if let Some((child, matches, view)) = results.opened_child() {
            if let Some(entry) = walk.iter_mut().find(|(each, _)| *each == message) {
                entry.0 = child;
            }
            results.opening = Some(matches);
            self.walk_seed = Some(view);
            target = child;
        }
        self.bar.walk(walk);
        self.show_hit(target)
    }

    pub(crate) fn results_follow(&mut self, reading: Option<MessageId>) -> Vec<Step> {
        let Some(results) = self.results.as_mut() else {
            return Vec::new();
        };
        let Some(position) = reading.and_then(|message| results.position_of(message)) else {
            return Vec::new();
        };
        cursor_moved(results.point(position))
    }

    fn results_tab(&mut self, tab: ResultsTab) -> Vec<Step> {
        let Some(results) = self.results.as_mut() else {
            return Vec::new();
        };
        if results.tab == tab {
            return Vec::new();
        }
        results.tab = tab;
        let ask = results.ask_files().into_iter().chain(results.ask_people());
        let ask: Vec<Step> = ask.collect();
        let mut steps = self.close_quick_look();
        steps.extend(self.draw_results());
        steps.extend(ask);
        steps
    }

    /// A verb on mail has no mail here. `None` hands `id` on, to the results' own keys.
    fn people_command(&mut self, id: CommandId) -> Option<Vec<Step>> {
        let results = self.results.as_mut()?;
        if results.tab != ResultsTab::People {
            return None;
        }
        let steps = match id {
            CommandId::NextMessage | CommandId::PrevMessage => {
                cursor_moved(results.step(if id == CommandId::NextMessage { 1 } else { -1 }))
            }
            CommandId::FirstMessage => cursor_moved(results.step(i64::MIN / 2)),
            CommandId::LastMessage => cursor_moved(results.step(i64::MAX / 2)),
            CommandId::OpenMessage => {
                let Some(address) = results.cursor_person().map(|person| person.address.clone())
                else {
                    return Some(Vec::new());
                };
                let query = postio_search::query::spell(&Clause {
                    negated: false,
                    filter: Filter::From(address),
                });
                let leaving = self.here();
                self.history.visit(leaving);
                let order = Results::default_order(&self.bar.lower(&query));
                self.open_results(Snapshot {
                    query,
                    tab: ResultsTab::Conversations,
                    order,
                    cursor: None,
                    checked: Vec::new(),
                    all: false,
                })
            }
            CommandId::ToggleSelection
            | CommandId::SelectAll
            | CommandId::QuickLook
            | CommandId::SaveFile
            | CommandId::NextMatch
            | CommandId::PrevMatch => Vec::new(),
            _ if matches!(
                postio_ui::focus_target::dispatch(id),
                Some(postio_ui::focus_target::Dispatch::OnMail(_))
            ) || crate::pickers::opens_picker(id) =>
            {
                Vec::new()
            }
            _ => return None,
        };
        Some(steps)
    }

    /// A verb on mail has no mail here. `None` hands `id` on, to the results' own keys.
    fn files_command(&mut self, id: CommandId) -> Option<Vec<Step>> {
        let results = self.results.as_mut()?;
        if results.tab != ResultsTab::Files {
            return None;
        }
        let steps = match id {
            CommandId::NextMessage | CommandId::PrevMessage => {
                cursor_moved(results.step(if id == CommandId::NextMessage { 1 } else { -1 }))
            }
            CommandId::FirstMessage => cursor_moved(results.step(i64::MIN / 2)),
            CommandId::LastMessage => cursor_moved(results.step(i64::MAX / 2)),
            CommandId::QuickLook => {
                let previewing = results
                    .copy
                    .as_ref()
                    .is_some_and(|copy| copy.purpose == FilePurpose::Preview);
                if previewing {
                    self.close_file_copy()
                } else {
                    self.copy_file(FilePurpose::Preview)
                }
            }
            CommandId::SaveFile => self.copy_file(FilePurpose::Save),
            CommandId::OpenMessage => {
                let Some(message) = results.cursor_file().map(|file| file.message) else {
                    return Some(Vec::new());
                };
                let walk = results
                    .files
                    .iter()
                    .flatten()
                    .map(|file| (file.message, None))
                    .collect();
                let mut steps = self.close_file_copy();
                self.bar.walk(walk);
                steps.extend(self.show_hit(message));
                steps
            }
            CommandId::Back if results.copy.is_some() => self.close_file_copy(),
            CommandId::ToggleSelection
            | CommandId::SelectAll
            | CommandId::NextMatch
            | CommandId::PrevMatch => Vec::new(),
            _ if matches!(
                postio_ui::focus_target::dispatch(id),
                Some(postio_ui::focus_target::Dispatch::OnMail(_))
            ) || crate::pickers::opens_picker(id) =>
            {
                Vec::new()
            }
            _ => return None,
        };
        Some(steps)
    }

    fn copy_file(&mut self, purpose: FilePurpose) -> Vec<Step> {
        let mut steps = self.close_file_copy();
        let stamp = self.stamp();
        let Some(results) = self.results.as_mut() else {
            return steps;
        };
        let Some(file) = results.cursor_file() else {
            return steps;
        };
        let attachment = file.attachment;
        results.copy = Some(PendingCopy {
            purpose,
            stamp,
            name: file.name.clone(),
            path: None,
        });
        steps.push(Step::Ask(Request::AttachmentCopy {
            attachment,
            purpose,
            stamp,
        }));
        steps
    }

    pub(crate) fn close_file_copy(&mut self) -> Vec<Step> {
        let Some(copy) = self
            .results
            .as_mut()
            .and_then(|results| results.copy.take())
        else {
            return Vec::new();
        };
        let mut steps = Vec::new();
        if let Some(path) = copy.path {
            if copy.purpose == FilePurpose::Preview {
                steps.push(Step::Show(Intent::FileCopy(None)));
            }
            steps.push(Step::Ask(Request::RemoveCopy(path)));
        }
        steps
    }

    /// One that lands for a copy no longer wanted is removed at once.
    fn file_copied(
        &mut self,
        stamp: u64,
        purpose: FilePurpose,
        answer: Result<Option<std::path::PathBuf>, String>,
    ) -> Vec<Step> {
        let path = answer
            .map_err(|error| tracing::debug!(%error, "a file could not be copied"))
            .ok()
            .flatten();
        let wanted = self.results.as_mut().and_then(|results| {
            results.copy.as_mut().filter(|copy| {
                copy.stamp == stamp && copy.purpose == purpose && copy.path.is_none()
            })
        });
        match (wanted, path) {
            (Some(copy), Some(path)) => {
                copy.path = Some(path.clone());
                vec![Step::Show(Intent::FileCopy(Some(FileCopy {
                    path,
                    name: copy.name.clone(),
                    purpose,
                })))]
            }
            (Some(_), None) => {
                if let Some(results) = self.results.as_mut() {
                    results.copy = None;
                }
                vec![Step::Show(Intent::Toast {
                    text: words::file_not_here(self.bar.keymap()),
                    kind: crate::ToastKind::Notice,
                })]
            }
            (None, Some(path)) => vec![Step::Ask(Request::RemoveCopy(path))],
            (None, None) => Vec::new(),
        }
    }

    fn results_order(&mut self, order: ConversationOrder) -> Vec<Step> {
        let stamp = self.stamp();
        let Some(results) = self.results.as_mut() else {
            return Vec::new();
        };
        if results.order == order {
            return Vec::new();
        }
        results.order = order;
        results.cursor = None;
        let asks = results.again(stamp);
        let mut steps = self.close_quick_look();
        steps.extend(asks);
        steps
    }

    fn results_edit(&mut self, edit: postio_search::edit::Edit) -> Vec<Step> {
        let today = self.bar.now().date_naive();
        let stamp = self.stamp();
        let Some(results) = self.results.as_ref() else {
            return Vec::new();
        };
        let query = postio_search::edit::apply(&results.query, edit, today);
        if query == results.query {
            return Vec::new();
        }
        let parsed = self.bar.lower(&query);
        let Some(results) = self.results.as_mut() else {
            return Vec::new();
        };
        results.set_query(query, parsed);
        let asks = results.again(stamp);
        let mut steps = self.close_quick_look();
        steps.push(Step::Show(Intent::Query(self.query_view())));
        steps.extend(self.draw_popover());
        steps.extend(asks);
        steps
    }

    fn draw_popover(&self) -> Option<Step> {
        let today = self.bar.now().date_naive();
        let view = self.results.as_ref()?.popover_view(today)?;
        Some(Step::Show(Intent::Popover(Some(Box::new(view)))))
    }

    fn open_popover(&mut self, kind: FilterKind) -> Vec<Step> {
        if !kind.has_popover() {
            return Vec::new();
        }
        let mut steps = self.close_quick_look();
        if self
            .results
            .as_ref()
            .is_some_and(|results| results.popover.is_some())
        {
            steps.extend(self.popover_done(false));
        }
        let Some(results) = self.results.as_mut() else {
            return steps;
        };
        let field = own_field(kind).filter(|field| {
            results
                .parsed
                .filters()
                .any(|clause| clause.filter.field() == *field)
        });
        let base = match field {
            Some(_) => None,
            None => results.frame.as_ref().map(|frame| Base {
                facets: frame.facets.clone(),
                names: frame.names.clone(),
            }),
        };
        let before = results.query.clone();
        let rest = field.map(|field| without_field(&results.parsed, field));
        let own = rest.as_ref().map(|_| self.stamp());
        let Some(results) = self.results.as_mut() else {
            return steps;
        };
        results.popover = Some(Popover {
            kind,
            before,
            base,
            own,
            filter: String::new(),
            words: String::new(),
        });
        steps.push(Step::Show(Intent::Query(self.query_view())));
        steps.extend(self.draw_popover());
        if let (Some(query), Some(stamp)) = (rest, own) {
            steps.push(Step::Ask(Request::Facets {
                query: self.bar.lower(&query),
                stamp,
            }));
        }
        steps
    }

    /// No words put back the dates it opened on; words that are no date change nothing but what it
    /// says they became.
    fn date_words(&mut self, text: String) -> Vec<Step> {
        let today = self.bar.now().date_naive();
        let Some(results) = self.results.as_mut() else {
            return Vec::new();
        };
        let Some(popover) = results.popover.as_mut() else {
            return Vec::new();
        };
        popover.words = text;
        let dates = if popover.words.trim().is_empty() {
            Some(dates_of(&postio_search::parse(&popover.before, today)))
        } else {
            date_terms(&popover.words, today).map(|(after, before, _)| (after, before))
        };
        match dates {
            Some((after, before)) if (after, before) != results.dates() => {
                self.results_edit(postio_search::edit::Edit::SetDates { after, before })
            }
            _ => self.draw_popover().into_iter().collect(),
        }
    }

    fn date_preset(&mut self, token: u64) -> Vec<Step> {
        let today = self.bar.now().date_naive();
        let presets = words::date_presets(today);
        let Some(preset) = usize::try_from(token).ok().and_then(|at| presets.get(at)) else {
            return Vec::new();
        };
        let Some(popover) = self
            .results
            .as_mut()
            .and_then(|results| results.popover.as_mut())
        else {
            return Vec::new();
        };
        popover.words.clear();
        self.results_edit(postio_search::edit::Edit::SetDates {
            after: preset.start,
            before: None,
        })
    }

    fn popover_done(&mut self, apply: bool) -> Vec<Step> {
        let stamp = self.stamp();
        let Some(results) = self.results.as_mut() else {
            return Vec::new();
        };
        let Some(popover) = results.popover.take() else {
            return Vec::new();
        };
        let mut steps = vec![Step::Show(Intent::Popover(None))];
        if apply || popover.before == results.query {
            steps.extend(self.draw_results());
            return steps;
        }
        let parsed = self.bar.lower(&popover.before);
        let Some(results) = self.results.as_mut() else {
            return steps;
        };
        results.set_query(popover.before, parsed);
        let asks = results.again(stamp);
        steps.extend(self.close_quick_look());
        steps.push(Step::Show(Intent::Query(self.query_view())));
        steps.extend(asks);
        steps
    }

    fn term_edit(&self, edit: TermEdit) -> Option<postio_search::edit::Edit> {
        use postio_search::edit::Edit;
        let today = self.bar.now().date_naive();
        Some(match edit {
            TermEdit::Add {
                field,
                value,
                negated,
            } => Edit::Add(clause_of(&field, &value, negated, today)?),
            TermEdit::Remove { token } => Edit::Remove {
                token: token as usize,
            },
            TermEdit::Toggle { field, value } => {
                Edit::Toggle(clause_of(&field, &value, false, today)?.filter)
            }
            TermEdit::SetMonths { first, last } => {
                let (after, before) = self.results.as_ref()?.months_to_dates(first, last)?;
                Edit::SetDates {
                    after: Some(after),
                    before: Some(before),
                }
            }
            TermEdit::ClearFilters => Edit::ClearFilters,
        })
    }

    pub(crate) fn results_input(&mut self, input: Input) -> Vec<Step> {
        if self.results.is_none() {
            return Vec::new();
        }
        match input {
            Input::SearchEdit(edit) => match self.term_edit(edit) {
                Some(edit) => self.results_edit(edit),
                None => Vec::new(),
            },
            Input::ResultsTab(tab) => self.results_tab(tab),
            Input::SaveSearchAs {
                name,
                pin,
                notify,
                rolling,
            } => self.save_as(name, pin, notify, rolling),
            Input::ResultsOrder(order) => self.results_order(order),
            Input::ResultsPoint(position) => {
                let Some(results) = self.results.as_mut() else {
                    return Vec::new();
                };
                let moved = results.point(position);
                let looking = results.quick_look.is_some();
                let mut steps = cursor_moved(moved);
                if moved.is_some() && looking {
                    steps.extend(self.look());
                }
                steps
            }
            Input::SearchPopover(kind) => self.open_popover(kind),
            Input::PopoverToggle { token, exclude } => {
                let today = self.bar.now().date_naive();
                let edit = self
                    .results
                    .as_ref()
                    .and_then(|results| results.popover_toggle(token, exclude, today));
                match edit {
                    Some(edit) => self.results_edit(edit),
                    None => Vec::new(),
                }
            }
            Input::PopoverFilter(text) => {
                let Some(popover) = self
                    .results
                    .as_mut()
                    .and_then(|results| results.popover.as_mut())
                else {
                    return Vec::new();
                };
                popover.filter = text;
                self.draw_popover().into_iter().collect()
            }
            Input::PopoverDone { apply } => self.popover_done(apply),
            // The system's panel is gone already: only the copy goes.
            Input::FileCopyDone => match self
                .results
                .as_mut()
                .and_then(|results| results.copy.take())
                .and_then(|copy| copy.path)
            {
                Some(path) => vec![Step::Ask(Request::RemoveCopy(path))],
                None => Vec::new(),
            },
            Input::DateWords(text) => self.date_words(text),
            Input::DatePreset(token) => self.date_preset(token),
            _ => Vec::new(),
        }
    }

    pub(crate) fn results_reply(&mut self, reply: Reply) -> Vec<Step> {
        let now = self.pickers.clock.unwrap_or_else(Local::now);
        let Some(results) = self.results.as_mut() else {
            return Vec::new();
        };
        match reply {
            Reply::ResultsPage {
                stamp,
                order,
                offset,
                answer,
            } => {
                let landed = results.landed(stamp, order, offset, answer);
                let query = results.query.clone();
                let ways_out = landed.nothing.then(|| Request::Relaxations {
                    query: results.parsed.clone(),
                    today: self.bar.now().date_naive(),
                    stamp: results.stamp,
                });
                let mut steps = Vec::new();
                if landed.frame {
                    steps.extend(self.draw_results());
                    steps.extend(self.draw_popover());
                }
                // Nothing found: the page now, its ways out when they are
                // counted -- slowly, on a lane of their own, never in the
                // way of the next keystroke.
                if let Some(request) = ways_out {
                    steps.extend(self.draw_no_results());
                    steps.push(Step::Ask(request));
                }
                if let Some((first, count)) = landed.rows {
                    steps.push(Step::Show(Intent::ResultsPage { first, count }));
                }
                if let Some((hits, capped)) = landed.remember {
                    steps.push(Step::Ask(Request::RememberSearch {
                        query,
                        hits,
                        capped,
                    }));
                }
                steps.extend(landed.passages.map(Step::Ask));
                // The top result's matches ride with its page (spec 013
                // R11): asked beside the passages, so the default
                // expansion costs no trip of its own.
                steps.extend(self.default_expansion());
                let stamp = self.stamp();
                if let Some(results) = self.results.as_mut()
                    && results.stale
                    && results.frame.is_some()
                {
                    steps.extend(results.again(stamp));
                }
                steps
            }
            Reply::ResultsPassages { stamp, answer } => results
                .passages(stamp, answer)
                .map(|(first, count)| Step::Show(Intent::ResultsPage { first, count }))
                .into_iter()
                .collect(),
            Reply::Files { stamp, answer } => {
                if results.files_landed(stamp, answer) {
                    self.draw_results()
                } else {
                    Vec::new()
                }
            }
            Reply::People { stamp, answer } => {
                if results.people_landed(stamp, answer) {
                    self.draw_results()
                } else {
                    Vec::new()
                }
            }
            Reply::AttachmentCopy {
                stamp,
                purpose,
                answer,
            } => self.file_copied(stamp, purpose, answer),
            Reply::QuickLookMatches { stamp, answer } => {
                if let Some(changed) = results.expansion_matches(stamp, &answer, now) {
                    return if changed {
                        results.draw_splice()
                    } else {
                        Vec::new()
                    };
                }
                if results.matches_landed(stamp, answer) {
                    self.draw_quick_look()
                } else {
                    Vec::new()
                }
            }
            Reply::Facets { stamp, answer } => {
                if results.facets_landed(stamp, answer) {
                    self.draw_popover().into_iter().collect()
                } else {
                    Vec::new()
                }
            }
            Reply::Relaxations { stamp, answer } => {
                if !results.ways_out_landed(stamp, answer) {
                    return Vec::new();
                }
                let mut steps = self.draw_results();
                steps.extend(self.draw_no_results());
                steps
            }
            _ => Vec::new(),
        }
    }
}

fn cursor_moved(position: Option<u64>) -> Vec<Step> {
    position
        .map(|position| Step::Show(Intent::ResultsCursor(position)))
        .into_iter()
        .collect()
}

fn spelled(parsed: &ParsedQuery) -> String {
    parsed
        .tokens()
        .iter()
        .map(|token| token.raw.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

fn undated_query(parsed: &ParsedQuery) -> String {
    use postio_search::query::Field;
    parsed
        .tokens()
        .iter()
        .filter(|token| !matches!(token.field(), Some(Field::After | Field::Before)))
        .map(|token| token.raw.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

fn own_field(kind: FilterKind) -> Option<postio_search::query::Field> {
    use postio_search::query::Field;
    match kind {
        FilterKind::From => Some(Field::From),
        FilterKind::To => Some(Field::To),
        FilterKind::Anywhere => Some(Field::In),
        FilterKind::Label => Some(Field::Label),
        _ => None,
    }
}

fn without_field(parsed: &ParsedQuery, field: postio_search::query::Field) -> String {
    parsed
        .tokens()
        .iter()
        .filter(|token| token.field() != Some(field))
        .map(|token| token.raw.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `None` when they name no date.
fn date_terms(
    words: &str,
    today: NaiveDate,
) -> Option<(Option<NaiveDate>, Option<NaiveDate>, String)> {
    if words.trim().is_empty() {
        return None;
    }
    let lowered = postio_search::natural::lower(words, today, &|_| None);
    let mut after = None;
    let mut before = None;
    for clause in lowered.filters().filter(|clause| !clause.negated) {
        match clause.filter {
            Filter::After(day) => after = Some(day),
            Filter::Before(day) => before = Some(day),
            _ => {}
        }
    }
    if after.is_none() && before.is_none() {
        return None;
    }
    let spell = |filter| {
        postio_search::query::spell(&Clause {
            negated: false,
            filter,
        })
    };
    let terms = after
        .map(|day| spell(Filter::After(day)))
        .into_iter()
        .chain(before.map(|day| spell(Filter::Before(day))))
        .collect::<Vec<_>>()
        .join(" ");
    Some((after, before, terms))
}

fn dates_of(parsed: &ParsedQuery) -> (Option<NaiveDate>, Option<NaiveDate>) {
    let mut after = None;
    let mut before = None;
    for clause in parsed.filters().filter(|clause| !clause.negated) {
        match clause.filter {
            Filter::After(day) => after = Some(day),
            Filter::Before(day) => before = Some(day),
            _ => {}
        }
    }
    (after, before)
}

/// Which of a card's preview lines is the matching one: a sheet's row or a
/// text's line where it falls among the six drawn, a page's or a slide's
/// middle.
fn marked_line(location: &Location) -> u32 {
    const DRAWN: u32 = 6;
    match location {
        Location::Sheet { row, .. } | Location::Table { row, .. } => row.saturating_sub(1) % DRAWN,
        Location::Line(n) | Location::Paragraph(n) => n.saturating_sub(1) % DRAWN,
        Location::Page(_) | Location::Slide(_) | Location::ImageText => 3,
    }
}

#[derive(Debug)]
pub(crate) struct ResultExpansion {
    anchor: u64,
    thread: ThreadId,
    stamp: u64,
    matches: Option<Vec<ConversationMatch>>,
    view: Option<ThreadView>,
    children: Vec<ResultChild>,
    focused: Option<usize>,
}

impl Results {
    /// `None` when it is not a thread of several messages. Any other expansion is folded.
    fn expand_at(&mut self, position: u64, stamp: u64) -> Option<(Request, Request)> {
        if self.tab != ResultsTab::Conversations {
            return None;
        }
        let (hit, _, _) = self.hit_at(position)?;
        let thread = thread_of(hit).filter(|_| hit.messages > 1)?;
        let key = hit.key;
        self.expansion = Some(ResultExpansion {
            anchor: position,
            thread,
            stamp,
            matches: None,
            view: None,
            children: Vec::new(),
            focused: None,
        });
        Some((
            Request::QuickLookMatches {
                query: self.parsed.clone(),
                key,
                stamp,
            },
            Request::ReadThread(thread),
        ))
    }

    /// `None` when this answer is not the expansion's.
    fn expansion_matches(
        &mut self,
        stamp: u64,
        answer: &Result<Vec<ConversationMatch>, String>,
        now: DateTime<Local>,
    ) -> Option<bool> {
        let expansion = self.expansion.as_mut().filter(|each| each.stamp == stamp)?;
        match answer {
            Ok(matches) => expansion.matches = Some(matches.clone()),
            Err(error) => {
                tracing::debug!(%error, "the matches of an expanded result could not be read");
                self.expansion = None;
                return Some(false);
            }
        }
        Some(self.compose_children(now))
    }

    pub(crate) fn expansion_thread(
        &mut self,
        thread: ThreadId,
        view: Option<&ThreadView>,
        now: DateTime<Local>,
    ) -> bool {
        let Some(expansion) = self.expansion.as_mut().filter(|each| each.thread == thread) else {
            return false;
        };
        let Some(view) = view else {
            self.expansion = None;
            return false;
        };
        expansion.view = Some(view.clone());
        self.compose_children(now)
    }

    fn compose_children(&mut self, now: DateTime<Local>) -> bool {
        let Some(expansion) = self.expansion.as_mut() else {
            return false;
        };
        let (Some(matches), Some(view)) = (&expansion.matches, &expansion.view) else {
            return false;
        };
        let children = postio_ui::search_view::children(view, matches, now);
        if children == expansion.children {
            return false;
        }
        expansion.children = children;
        expansion.focused = None;
        true
    }

    pub(crate) fn splice(&self) -> Option<ResultSplice> {
        let expansion = self
            .expansion
            .as_ref()
            .filter(|each| !each.children.is_empty())?;
        Some(ResultSplice {
            anchor: expansion.anchor,
            children: expansion.children.clone(),
            focused: expansion.focused.map(|child| child as u32),
        })
    }

    pub(crate) fn draw_splice(&self) -> Vec<Step> {
        vec![Step::Show(Intent::ResultSplice(
            self.splice().map(Box::new),
        ))]
    }

    pub(crate) fn focused_child(&self) -> Option<u32> {
        self.expansion
            .as_ref()
            .and_then(|each| each.focused)
            .map(|child| child as u32)
    }

    fn unfocus_child(&mut self) {
        if let Some(expansion) = self.expansion.as_mut() {
            expansion.focused = None;
        }
    }

    fn step_children(&mut self, by: i64) -> ChildStep {
        let Some(cursor) = self.cursor else {
            return ChildStep::NotMine;
        };
        let rows = self.rows();
        let Some(expansion) = self.expansion.as_mut() else {
            return ChildStep::NotMine;
        };
        let marks: Vec<usize> = expansion
            .children
            .iter()
            .enumerate()
            .filter_map(|(at, child)| matches!(child, ResultChild::Match { .. }).then_some(at))
            .collect();
        let (Some(&first), Some(&last)) = (marks.first(), marks.last()) else {
            return ChildStep::NotMine;
        };
        let to = |focus: usize, cursor: Option<u64>| ChildStep::To {
            focus: Some(focus as u32),
            cursor,
        };
        match expansion.focused {
            Some(at) => {
                let place = marks.iter().position(|mark| *mark == at).unwrap_or(0);
                if by > 0 {
                    match marks.get(place + 1) {
                        Some(&next) => {
                            expansion.focused = Some(next);
                            to(next, None)
                        }
                        None if cursor + 1 >= rows => ChildStep::Stay,
                        None => ChildStep::NotMine,
                    }
                } else if place > 0 {
                    expansion.focused = Some(marks[place - 1]);
                    to(marks[place - 1], None)
                } else {
                    expansion.focused = None;
                    ChildStep::To {
                        focus: None,
                        cursor: None,
                    }
                }
            }
            None if by > 0 && cursor == expansion.anchor => {
                expansion.focused = Some(first);
                to(first, None)
            }
            None if by < 0 && cursor == expansion.anchor + 1 => {
                expansion.focused = Some(last);
                self.cursor = Some(expansion.anchor);
                to(last, Some(expansion.anchor))
            }
            None => ChildStep::NotMine,
        }
    }

    pub(crate) fn opened_child(
        &self,
    ) -> Option<(MessageId, crate::walk::ThreadMatches, ThreadView)> {
        let expansion = self.expansion.as_ref()?;
        let ResultChild::Match { message, .. } = expansion.children.get(expansion.focused?)? else {
            return None;
        };
        let messages: Vec<MessageId> = expansion
            .children
            .iter()
            .filter_map(|child| match child {
                ResultChild::Match { message, .. } => Some(*message),
                ResultChild::Gap { .. } => None,
            })
            .collect();
        let at = messages.iter().position(|each| each == message)?;
        Some((
            *message,
            crate::walk::ThreadMatches {
                query: self.parsed.clone(),
                messages,
                at,
            },
            expansion.view.clone()?,
        ))
    }

    fn wants_default(&mut self) -> bool {
        if self.default_done || self.expansion.is_some() {
            return false;
        }
        let Some(top) = self.top.as_ref().filter(|_| self.top_len() > 0) else {
            return false;
        };
        self.default_done = true;
        self.order == ConversationOrder::BestMatch
            && self.tab == ResultsTab::Conversations
            && top.first().and_then(matched_messages).is_some()
    }
}

enum ChildStep {
    NotMine,
    Stay,
    To {
        focus: Option<u32>,
        cursor: Option<u64>,
    },
}

impl FocusController {
    fn results_step(&mut self, by: i64) -> Vec<Step> {
        let Some(results) = self.results.as_mut() else {
            return Vec::new();
        };
        let before = results.focused_child();
        if by.abs() == 1 {
            match results.step_children(by) {
                ChildStep::Stay => return Vec::new(),
                ChildStep::To { focus, cursor } => {
                    let mut steps: Vec<Step> = cursor
                        .map(|position| Step::Show(Intent::ResultsCursor(position)))
                        .into_iter()
                        .collect();
                    steps.push(Step::Show(Intent::ResultChildFocus(focus)));
                    return steps;
                }
                ChildStep::NotMine => {}
            }
        }
        let moved = results.step(by);
        let mut steps = Vec::new();
        if moved.is_some() && before.is_some() {
            steps.push(Step::Show(Intent::ResultChildFocus(None)));
        }
        steps.extend(cursor_moved(moved));
        steps
    }

    fn default_expansion(&mut self) -> Vec<Step> {
        if !self.results.as_mut().is_some_and(Results::wants_default) {
            return Vec::new();
        }
        self.expand_row(0)
    }

    fn expand_result(&mut self) -> Vec<Step> {
        let Some(position) = self.results.as_ref().and_then(|results| results.cursor) else {
            return Vec::new();
        };
        if self
            .results
            .as_ref()
            .and_then(|results| results.expansion.as_ref())
            .is_some_and(|each| each.anchor == position)
        {
            return Vec::new();
        }
        self.expand_row(position)
    }

    fn expand_row(&mut self, position: u64) -> Vec<Step> {
        let stamp = self.stamp();
        let Some(results) = self.results.as_mut() else {
            return Vec::new();
        };
        let folded = results.expansion.take().is_some();
        let mut steps = Vec::new();
        if folded {
            steps.push(Step::Show(Intent::ResultChildFocus(None)));
            steps.push(Step::Show(Intent::ResultSplice(None)));
        }
        if let Some((matches, thread)) = results.expand_at(position, stamp) {
            steps.push(Step::Ask(matches));
            steps.push(Step::Ask(thread));
        }
        steps
    }

    fn collapse_result(&mut self) -> Vec<Step> {
        let Some(results) = self.results.as_mut() else {
            return Vec::new();
        };
        let Some(expansion) = results.expansion.take() else {
            return Vec::new();
        };
        let mut steps = Vec::new();
        if expansion.focused.is_some() {
            steps.push(Step::Show(Intent::ResultChildFocus(None)));
        }
        steps.push(Step::Show(Intent::ResultSplice(None)));
        steps
    }

    pub(crate) fn result_thread_read(
        &mut self,
        thread: ThreadId,
        view: Option<&ThreadView>,
    ) -> Vec<Step> {
        let now = self.pickers.clock.unwrap_or_else(Local::now);
        let Some(results) = self.results.as_mut() else {
            return Vec::new();
        };
        if results.expansion_thread(thread, view, now) {
            results.draw_splice()
        } else {
            Vec::new()
        }
    }

    pub(crate) fn results_returned(&mut self) -> Vec<Step> {
        let Some(results) = self.results.as_mut() else {
            return Vec::new();
        };
        results.opening = None;
        let Some(expansion) = results.expansion.as_ref() else {
            return Vec::new();
        };
        let (Some(child), anchor) = (expansion.focused, expansion.anchor) else {
            return Vec::new();
        };
        vec![
            Step::Show(Intent::ResultsCursor(anchor)),
            Step::Show(Intent::ResultChildFocus(Some(child as u32))),
        ]
    }
}
