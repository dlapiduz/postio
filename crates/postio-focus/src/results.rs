//! The results view: the main window as a search's results (spec 010 step
//! 3, US2, D17, design §3 and screens 06 and 07).
//!
//! A mode of the main window, not a surface over it: ⌘↩ in the dropdown
//! enters it, Esc's last rung and ⌘[ leave it for the inbox, which it never
//! touches (`history.rs`). It holds the query as the field has it -- the
//! one source of truth, edited only through `postio_search::edit` (D1) --
//! and a window over the conversations it matches.
//!
//! The rows are Top hits (Best match only: the first three the engine
//! ranks) then every conversation newest first, grouped by the month of
//! its newest match (D4). Every conversation is in its month, a top hit
//! too, which is what lets the groups' positions be known from the
//! timeline's counts before a row of them is read: a page of fifty is read
//! when its rows are wanted, and its passages after it lands. A month
//! group's count is the timeline's bar.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use chrono::{DateTime, Local, NaiveDate};
use postio_core::{CommandId, Keymap};
use postio_model::{MailboxId, MessageId, ThreadId};
use postio_search::ParsedQuery;
use postio_search::facets::SearchFacets;
use postio_search::query::{Clause, Filter, TokenKind};
use postio_search::relax::{Loosen, Relaxation};
use postio_search::results::{
    ConversationHit, ConversationKey, ConversationMatch, ConversationOrder, ConversationResults,
    FacetNames, Match, ResultsTab, Source,
};
use postio_ui::hints::Hint;
use postio_ui::search_view::{self as words, FilterKind};

use crate::dropdown::{self, Run};
use crate::feed::Step;
use crate::history::{Entry, Snapshot};
use crate::{Effect, FocusController, Input, Intent, Reply, Request};

/// Top hits, at most (design §3.4).
const TOP_HITS: u32 = 3;
/// Conversations a page reads.
pub(crate) const PAGE: u32 = 50;
/// Pages held at once: the rest are read again when they are wanted.
const RESIDENT: usize = 8;

/// One of the results' tabs, as the filter bar draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultsTabView {
    /// Which tab.
    pub tab: ResultsTab,
    /// "Conversations".
    pub label: String,
    /// "48".
    pub count: String,
    /// Whether it is the one shown.
    pub selected: bool,
    /// The key that picks it (`mod+1`), as the keymap spells it.
    pub key: Option<String>,
}

/// One bar of the timeline.
#[derive(Debug, Clone, PartialEq)]
pub struct MonthBar {
    /// The month's short name: "Sep".
    pub label: String,
    /// Conversations whose newest match fell in it.
    pub conversations: u64,
    /// Its height, 0 to 1 of the tallest.
    pub height: f64,
    /// Whether the query's dates take it in.
    pub selected: bool,
}

/// A group of rows: Top hits, or a month.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultGroup {
    /// "Top hits", "September 2026", "Earlier".
    pub title: String,
    /// Its count, grouped: "9"; empty for Top hits, which the design
    /// draws without one.
    pub count: String,
    /// The tertiary note after the count: "newest first".
    pub note: Option<String>,
    /// Its first row's position.
    pub first: u64,
    /// How many rows it holds.
    pub rows: u64,
    /// Whether it is Top hits, whose rows are taller.
    pub top_hits: bool,
    /// What a screen reader says for its header: "September 2026 · 9".
    pub accessible: String,
}

/// The results view's frame: everything but the rows, which are read one
/// at a time ([`FocusController::result_row`](crate::FocusController::result_row)).
#[derive(Debug, Clone, PartialEq)]
pub struct ResultsView {
    /// Conversations, Files, People.
    pub tabs: Vec<ResultsTabView>,
    /// The sort.
    pub order: ConversationOrder,
    /// "48 conversations".
    pub count_line: String,
    /// "12 files · 6 people · last 12 months".
    pub sub_line: String,
    /// The timeline, oldest first.
    pub months: Vec<MonthBar>,
    /// The timeline's hint on its right: "Matches by month · drag across
    /// months to narrow", or "Jul – Sep selected · drag to change".
    pub timeline_hint: String,
    /// "steps a month", with ⌥←/⌥→, while a range is selected.
    pub timeline_step: Option<Hint>,
    /// The groups, top to bottom.
    pub groups: Vec<ResultGroup>,
    /// How many rows there are.
    pub rows: u64,
    /// The row with the focus ring.
    pub cursor: Option<u64>,
    /// The footer's keys.
    pub hints: Vec<Hint>,
    /// "48 conversations · local index · 41 ms".
    pub footer: String,
    /// How many conversations are checked.
    pub selected: u64,
    /// The bulk bar's keys, while rows are checked.
    pub bulk: Vec<Hint>,
    /// The bulk bar's right: "⇧X select all 12", while some but not every
    /// conversation the query matches is checked.
    pub select_all: Option<Hint>,
}

/// One result row (design §3.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultRow {
    /// The message it shows and opens: the conversation's best match.
    pub message: MessageId,
    /// The conversation, when threading made one.
    pub thread: Option<ThreadId>,
    /// Which group it is in, an index into [`ResultsView::groups`].
    pub group: u32,
    /// Whether it is a top hit: taller, with its reason.
    pub top_hit: bool,
    /// The sender column.
    pub sender: String,
    /// Bold when something in it is unread.
    pub unread: bool,
    /// Why it ranked, on a top hit: "you replied · 3 matches".
    pub reason: Option<String>,
    /// The subject, its matched words marked.
    pub subject: Vec<Run>,
    /// Its labels, as pills.
    pub labels: Vec<LabelPill>,
    /// The paperclip.
    pub attachments: bool,
    /// The thread count, when more than one.
    pub count_badge: Option<String>,
    /// Where it matched: "body", "quoted text", a file's name.
    pub source_tag: String,
    /// Whether the tag is a file's name, drawn in italics.
    pub source_is_file: bool,
    /// The passage, its matched words marked; empty until it is read.
    pub passage: Vec<Run>,
    /// `in:Inbox`.
    pub folder: String,
    /// "26 Sep".
    pub date: String,
    /// Checked for a bulk verb.
    pub checked: bool,
    /// What a screen reader says for it (design §5).
    pub accessible: String,
}

/// A label on a result row: its name, and the colour it was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelPill {
    /// The label's name.
    pub name: String,
    /// Its colour as `#rrggbb`, when it was given one: the pill's dot.
    pub color: Option<String>,
}

/// One operator term of the query, as a chip (design §1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    /// Its index among the query's tokens: what
    /// [`TermEdit::Remove`] names.
    pub token: u32,
    /// The operator with its colon, drawn tertiary: "from:".
    pub operator: String,
    /// Its value: "ada@example.com".
    pub value: String,
    /// A leading `-`: struck through.
    pub excluded: bool,
    /// Ringed: the term a focused relaxation loosens (step 7).
    pub focused: bool,
}

/// One filter button (design §3.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterButton {
    /// Which.
    pub kind: FilterKind,
    /// "From", or what it holds: "From: Ada Moreno".
    pub label: String,
    /// Solid: the query holds it.
    pub applied: bool,
    /// Ringed: its popover is open (step 4).
    pub open: bool,
}

/// The query as the field and the filter bar draw it: one query, two
/// controls (design §1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryView {
    /// The operator terms, in the query's order.
    pub chips: Vec<Chip>,
    /// The plain words, after the chips.
    pub words: String,
    /// The quiet hint on the right: "/ to edit", or "clears filters"
    /// after its key while nothing matches (D24).
    pub hint: String,
    /// The hint's key, as the registry spells it, drawn before it as a
    /// cap: `cmd+BackSpace`. `None` when the hint names its own key.
    pub hint_key: Option<String>,
    /// The filter bar's buttons.
    pub buttons: Vec<FilterButton>,
}

/// One looser search on the no-results page (§3.10, screen 13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelaxationView {
    /// Its number, from 1: the key that runs it.
    pub number: u32,
    /// That key, as the registry spells it (`1`); `None` when unbound.
    pub key: Option<String>,
    /// What it changes: "Remove “before March”".
    pub label: String,
    /// The query it runs, in the query language: drawn monospaced.
    pub query: String,
    /// "4 conversations".
    pub count: String,
    /// Ringed: Return runs it, and its chip is ringed in the field.
    pub focused: bool,
}

/// The page a search that found nothing shows in the rows' place (design
/// §3.10, screen 13): never a dead end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoResultsView {
    /// "Nothing matches all four filters".
    pub title: String,
    /// The sentence under it.
    pub body: String,
    /// The looser searches that find something, most first, at most four.
    pub relaxations: Vec<RelaxationView>,
    /// "Counting looser searches…" while their counts are on their way.
    pub counting: Option<String>,
    /// "Searched all 18,204 messages on this Mac."
    pub searched: String,
}

/// One row of a list popover (§3.6, screen 08): a person, a folder or a
/// label the results hold, with how many of them it holds.
#[derive(Debug, Clone, PartialEq)]
pub struct PopoverRow {
    /// What [`Input::PopoverToggle`] names it by: its place among the rows
    /// the popover opened with, whatever its own filter hides.
    pub token: u64,
    /// "Ada Moreno", "Inbox", "Atlas".
    pub title: String,
    /// The address under a person's name.
    pub detail: Option<String>,
    /// A person's avatar: "AM".
    pub initials: Option<String>,
    /// A label's colour as `#rrggbb`.
    pub color: Option<String>,
    /// Conversations among the results it opened on.
    pub count: u64,
    /// Its bar: the count, 0 to 1 of the largest.
    pub share: f64,
    /// The query holds it.
    pub checked: bool,
    /// The query excludes it (`-from:`).
    pub excluded: bool,
}

/// A filter popover, whole (design §3.6).
#[derive(Debug, Clone, PartialEq)]
pub struct PopoverView {
    /// Which button it hangs from.
    pub kind: FilterKind,
    /// Its search field's placeholder: "Filter people in these results".
    pub placeholder: String,
    /// What its search field holds.
    pub filter: String,
    /// The rows its field leaves, in the order it opened with.
    pub rows: Vec<PopoverRow>,
    /// Its footer's keys.
    pub hints: Vec<Hint>,
    /// The Date popover's presets, Custom… last.
    pub presets: Vec<DatePresetView>,
    /// The Date popover's plain words, as typed.
    pub words: String,
    /// What they became: "→ after:2026-07-01"; `None` when they are no
    /// date.
    pub parsed: Option<String>,
    /// The line under the words.
    pub words_hint: String,
    /// The Date popover's month chart: the timeline's bars.
    pub months: Vec<MonthBar>,
    /// "12 of 21": what its dates keep of what it opened on.
    pub result: Option<String>,
    /// "Jul – Sep 2026".
    pub range: Option<String>,
}

/// One of the Date popover's presets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatePresetView {
    /// What [`Input::DatePreset`](crate::Input::DatePreset) names it by.
    pub token: u64,
    /// "Last 30 days", "Custom…".
    pub label: String,
    /// How many conversations it keeps of those the popover opened on;
    /// none for Custom….
    pub count: Option<String>,
    /// The query's dates are its own: ringed.
    pub selected: bool,
}

/// One match card in Quick Look (design §3.7): where and when on the left,
/// the passage on the right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchCard {
    /// "Body", "Earlier reply", "Subject", a file's name.
    pub place: String,
    /// "Ada · 26 Sep"; where in a file; empty for the subject.
    pub when: String,
    /// The words around the match, marked; empty until they are cut.
    pub passage: Vec<Run>,
    /// The place is a file's name.
    pub file: bool,
}

/// Quick Look over the results, whole (design §3.7, screen 10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickLookView {
    /// "Quick Look".
    pub title: String,
    /// "1 of 12".
    pub position: String,
    /// "j/k moves through results while it stays open".
    pub walk: Option<Hint>,
    /// The header's buttons: Open, Archive, Close, each with its key.
    pub actions: Vec<Hint>,
    /// The result's subject, its matched words marked.
    pub subject: Vec<Run>,
    /// Its sender line: the name (strong), the address (mono), when, and
    /// the thread's size.
    pub sender: Vec<Run>,
    /// "4 matches in this conversation".
    pub matches_line: String,
    /// "]/[ jump between them".
    pub matches_hint: Option<Hint>,
    /// One card per match, oldest first, the subject last.
    pub cards: Vec<MatchCard>,
    /// The card with the accent ring.
    pub current: Option<u32>,
}

/// Quick Look, open on one result.
#[derive(Debug)]
pub(crate) struct QuickLook {
    /// The row it shows.
    position: u64,
    /// That row's conversation, as it was when Quick Look came to it.
    hit: ConversationHit,
    /// Every match in the conversation, once read; until then the row's.
    matches: Option<Vec<ConversationMatch>>,
    /// The ringed card.
    current: usize,
    /// What its read was asked under.
    stamp: u64,
}

impl QuickLook {
    /// The cards' matches: the conversation's once read, else the row's
    /// own, each with its writer when the row knows it.
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
            })
            .collect();
        // The subject is one card, last, as the conversation's are.
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
            });
        }
        row
    }
}

/// A change to the query from a control rather than the keyboard, as
/// `postio-ffi` hands it over (contracts/ffi-search.md `TermEditFfi`): the
/// operator's keyword and value, which Rust spells (D13).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TermEdit {
    /// Add `field:value`, or `-field:value`.
    Add {
        /// The operator's keyword: "from".
        field: String,
        /// Its value: "ada@example.com".
        value: String,
        /// Excluded.
        negated: bool,
    },
    /// Take one chip out, by [`Chip::token`].
    Remove {
        /// The chip's token.
        token: u32,
    },
    /// A toggle button: `has:attachment`, `is:unread`, `has:action`.
    Toggle {
        /// "has", "is".
        field: String,
        /// "attachment", "unread", "action".
        value: String,
    },
    /// The timeline: the months `first..=last`, 0 the oldest bar.
    SetMonths {
        /// The first month.
        first: u32,
        /// The last month.
        last: u32,
    },
    /// Keep the words, drop every operator.
    ClearFilters,
}

/// `field:value` as a clause, the way the parser reads it; `None` when it
/// is no operator the query language has.
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

/// What the results answer was, kept for the frame.
#[derive(Debug, Clone)]
struct Frame {
    total: u64,
    capped: bool,
    facets: SearchFacets,
    files: u64,
    people: u64,
    names: FacetNames,
    elapsed: Duration,
    /// What the search looked through: the no-results page says so.
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

/// What a frame or a row is drawn with that the results do not hold.
pub(crate) struct Words<'a> {
    pub(crate) keymap: &'a Keymap,
    pub(crate) now: DateTime<Local>,
    /// Every folder the places know, for a row's `in:`.
    pub(crate) folders: &'a [(MailboxId, String)],
}

/// One group, as positions.
struct Span {
    title: String,
    accessible: String,
    count: u64,
    note: Option<String>,
    first: u64,
    rows: u64,
    top_hits: bool,
}

/// What a popover lists from: the answer for the query it opened on, so
/// its rows and counts hold still while its checks narrow the results.
#[derive(Debug, Clone)]
struct Base {
    facets: SearchFacets,
    names: FacetNames,
}

/// An open filter popover.
#[derive(Debug)]
pub(crate) struct Popover {
    kind: FilterKind,
    /// The query to restore on Esc (FR-027).
    before: String,
    /// What it lists, once the answer for `before` is known.
    base: Option<Base>,
    /// Its own search field.
    filter: String,
    /// The Date popover's plain words.
    words: String,
}

/// One thing a list popover offers: the filter it is, and how it is drawn.
struct Offer {
    filter: Filter,
    title: String,
    detail: Option<String>,
    initials: Option<String>,
    color: Option<String>,
    count: u64,
}

/// A search that found nothing: its ways out, once counted.
#[derive(Debug, Default)]
pub(crate) struct NoResults {
    /// Every looser search with its count, as the engine answered; `None`
    /// while they are counted.
    found: Option<Vec<(Relaxation, u64)>>,
    /// The one Return runs, among those shown.
    focus: usize,
}

/// The looser searches a page shows (FR-030).
const RELAXATIONS_SHOWN: usize = 4;

/// The results mode.
#[derive(Debug)]
pub(crate) struct Results {
    /// The query, as the field holds it.
    pub(crate) query: String,
    pub(crate) parsed: ParsedQuery,
    terms: Vec<String>,
    pub(crate) tab: ResultsTab,
    pub(crate) order: ConversationOrder,
    stamp: u64,
    frame: Option<Frame>,
    /// Top hits, once read (Best match).
    top: Option<Vec<ConversationHit>>,
    /// The month groups' pages read, by page.
    pages: BTreeMap<u32, Vec<ConversationHit>>,
    /// Pages asked for and not yet landed.
    asked: BTreeSet<u32>,
    /// The row with the focus ring; restored rows wait for the frame.
    pub(crate) cursor: Option<u64>,
    /// Checked rows, by their best message and its conversation -- or,
    /// with `all`, the rows taken back out of the whole.
    pub(crate) checked: Vec<(MessageId, Option<ThreadId>)>,
    /// ⇧X: every conversation the query matches is checked, but `checked`
    /// (a predicate, never the rows read).
    pub(crate) all: bool,
    /// The query has been kept among the recent searches.
    pub(crate) remembered: bool,
    /// The filter popover that is open.
    pub(crate) popover: Option<Popover>,
    /// The timeline of the query without its dates, and that query: what
    /// the bars outside a range are drawn from, so a range can be dragged
    /// wider than the one the results were narrowed to.
    undated: Option<(String, [postio_search::facets::MonthCount; 12])>,
    /// Quick Look, while it is open over the results.
    pub(crate) quick_look: Option<QuickLook>,
    /// Everyone an answer has named, kept across queries: a chip or a
    /// button names its person while the next answer is on its way,
    /// rather than falling back to the address and back again.
    people: Vec<postio_model::EmailAddress>,
    /// The search found nothing: its ways out (US6).
    none: Option<NoResults>,
}

impl Results {
    /// Results for `query` (lowered as `parsed`), asked under `stamp`.
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
            none: None,
        }
    }

    /// The sort a query starts on: Best match when it has words, else
    /// Newest (FR-022).
    pub(crate) fn default_order(parsed: &ParsedQuery) -> ConversationOrder {
        if parsed.text_terms().any(|term| !term.negated) {
            ConversationOrder::BestMatch
        } else {
            ConversationOrder::Newest
        }
    }

    /// The reads the view starts with: Top hits under Best match, and the
    /// month groups' first page, together.
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
        steps
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

    /// Ask everything again under `stamp`: the query, sort or tab changed.
    /// What was read goes; the frame drawn stays on screen until the new
    /// one lands.
    pub(crate) fn again(&mut self, stamp: u64) -> Vec<Step> {
        self.stamp = stamp;
        self.frame = None;
        self.top = None;
        self.pages.clear();
        self.asked.clear();
        let mut steps = Vec::new();
        if self.none.take().is_some() {
            steps.push(Step::Show(Intent::Relaxations(None)));
        }
        steps.extend(self.asks());
        steps
    }

    /// Whether the search found nothing, and the no-results page is up.
    pub(crate) fn found_nothing(&self) -> bool {
        self.none.is_some()
    }

    /// The looser searches shown: those that find something, most first,
    /// at most four. Empty while they are counted.
    fn ways_out(&self) -> Vec<&(Relaxation, u64)> {
        let Some(found) = self.none.as_ref().and_then(|none| none.found.as_ref()) else {
            return Vec::new();
        };
        let mut shown: Vec<&(Relaxation, u64)> = found.iter().filter(|(_, n)| *n > 0).collect();
        shown.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        shown.truncate(RELAXATIONS_SHOWN);
        shown
    }

    /// The token the focused way out loosens: its chip is ringed.
    fn loosened_token(&self) -> Option<usize> {
        let focus = self.none.as_ref()?.focus;
        let (relaxation, _) = self.ways_out().get(focus).copied()?;
        let (Loosen::Drop { token }
        | Loosen::Anywhere { token }
        | Loosen::FolderNotLabel { token }) = relaxation.loosen;
        Some(token)
    }

    /// The query way out `at` (from 0) runs, if there is one.
    pub(crate) fn way_out(&self, at: usize) -> Option<String> {
        self.ways_out()
            .get(at)
            .map(|(relaxation, _)| relaxation.query.clone())
    }

    /// Move the focus among the ways out by `by`; whether it moved.
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

    /// The ways out landed, asked under `stamp`: whether the page changes.
    pub(crate) fn ways_out_landed(
        &mut self,
        stamp: u64,
        answer: Result<Vec<(Relaxation, u64)>, String>,
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

    /// The no-results page (§3.10), while the search found nothing.
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
                count: words::relaxation_count(*n),
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
                frame.messages_searched,
                frame.corpus_complete,
                frame.contents_complete,
            ),
        })
    }

    /// Take `query`, lowered as `parsed`: the checked rows go with the
    /// rows they were, and the focus ring goes back to the top.
    pub(crate) fn set_query(&mut self, query: String, parsed: ParsedQuery) {
        self.terms = postio_search::highlight::terms(&parsed);
        self.query = query;
        self.parsed = parsed;
        self.clear_checked();
        self.cursor = None;
        self.remembered = false;
    }

    /// Whether the frame is known: the totals, and under Best match how
    /// many Top hits there are.
    fn ready(&self) -> bool {
        self.frame.is_some() && (self.order == ConversationOrder::Newest || self.top.is_some())
    }

    fn top_len(&self) -> u64 {
        if self.tab != ResultsTab::Conversations || self.order != ConversationOrder::BestMatch {
            return 0;
        }
        self.top.as_ref().map_or(0, |top| top.len() as u64)
    }

    /// How many rows the view draws.
    pub(crate) fn rows(&self) -> u64 {
        match (&self.frame, self.tab) {
            (Some(frame), ResultsTab::Conversations) if self.ready() => {
                self.top_len() + frame.total
            }
            _ => 0,
        }
    }

    /// The groups, as positions: Top hits, then each month with a match,
    /// newest first, then what is older than the timeline.
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

    /// The hit at `position`, its group, and whether it is a top hit.
    fn hit_at(&self, position: u64) -> Option<(&ConversationHit, u32, bool)> {
        if position >= self.rows() {
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

    /// Where `message`'s row is, among the rows read.
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

    /// The rows read, in order, each with its conversation: what `j` and
    /// `k` walk in a message a result opened.
    pub(crate) fn walk(&self) -> Vec<(MessageId, Option<ThreadId>)> {
        let top = self.top_len() as usize;
        self.top
            .iter()
            .flatten()
            .take(top)
            .chain(self.pages.values().flatten())
            .map(|hit| (hit.best, thread_of(hit)))
            .collect()
    }

    /// The message under the focus ring, once its row is read.
    pub(crate) fn cursor_message(&self) -> Option<(MessageId, Option<ThreadId>)> {
        let (hit, _, _) = self.hit_at(self.cursor?)?;
        Some((hit.best, thread_of(hit)))
    }

    /// Rows `position` wants read: its page, and the one after when it is
    /// near the end of its own.
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

    /// A page landed. What it changes: the frame, once it is known, or
    /// the rows it fills; then its passages are asked for.
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
            self.undated = Some((self.query.clone(), results.facets.months));
        }
        if let Some(popover) = self.popover.as_mut()
            && popover.base.is_none()
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
                landed.remember = self.frame.as_ref().map(|frame| frame.total);
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

    /// The pages furthest from `page` go when more than [`RESIDENT`] are
    /// held.
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

    /// The passages request for `hits`, when there are any.
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

    /// Passages landed: put in the rows they belong to; the span of rows
    /// changed.
    pub(crate) fn passages(
        &mut self,
        stamp: u64,
        answer: Result<Vec<(MessageId, Vec<Match>)>, String>,
    ) -> Option<(u64, u64)> {
        // Before the frame, the rows' positions are not known yet: the
        // frame, when it lands, has every row read again.
        if stamp != self.stamp || !self.ready() {
            return None;
        }
        let found = match answer {
            Ok(found) => found,
            Err(error) => {
                tracing::debug!(%error, "search passages could not be read");
                return None;
            }
        };
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
            if let Some(position) = self.position_of(message) {
                low = low.min(position);
                high = high.max(position);
            }
            // A top hit is in its month too.
            if let Some(position) = self.page_position_of(message) {
                low = low.min(position);
                high = high.max(position);
            }
        }
        (low <= high).then(|| (low, high - low + 1))
    }

    /// Where `message` is among the month groups' rows read.
    fn page_position_of(&self, message: MessageId) -> Option<u64> {
        let top = self.top_len();
        self.pages.iter().find_map(|(page, hits)| {
            hits.iter()
                .position(|hit| hit.best == message)
                .map(|at| top + u64::from(*page) * u64::from(PAGE) + at as u64)
        })
    }

    /// Move the focus ring by `by` rows, or to an end; the row it is on.
    pub(crate) fn step(&mut self, by: i64) -> Option<u64> {
        let rows = self.rows();
        if rows == 0 {
            return None;
        }
        let at = self.cursor.map_or(0, |at| at as i64);
        let next = (at + by).clamp(0, rows as i64 - 1) as u64;
        if Some(next) == self.cursor {
            return None;
        }
        self.cursor = Some(next);
        Some(next)
    }

    /// Put the focus ring on `position`.
    pub(crate) fn point(&mut self, position: u64) -> Option<u64> {
        (position < self.rows() && self.cursor != Some(position)).then(|| {
            self.cursor = Some(position);
            position
        })
    }

    /// `x` on the focused row: checked, or not -- under ⇧X, taken back
    /// out of the whole, or put back in it.
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

    /// ⇧X: every conversation the query matches.
    pub(crate) fn check_all(&mut self) {
        self.all = true;
        self.checked.clear();
    }

    /// Nothing checked.
    pub(crate) fn clear_checked(&mut self) {
        self.all = false;
        self.checked.clear();
    }

    /// Whether anything is checked.
    pub(crate) fn any_checked(&self) -> bool {
        self.all || !self.checked.is_empty()
    }

    /// Whether the row whose best message is `message` is checked.
    fn is_checked(&self, message: MessageId) -> bool {
        self.all != self.checked.iter().any(|(at, _)| *at == message)
    }

    /// How many conversations are checked: under ⇧X, the whole match but
    /// those taken back out.
    pub(crate) fn selected(&self) -> u64 {
        if self.all {
            let total = self.frame.as_ref().map_or(0, |frame| frame.total);
            total.saturating_sub(self.checked.len() as u64)
        } else {
            self.checked.len() as u64
        }
    }

    /// Where a verb on the results goes: the checked conversations, the
    /// whole match as a predicate under ⇧X, or the focused one. `None`
    /// when there is nothing to aim at.
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
        match postio_ui::focus_target::aim_by(&selection, &reach, None) {
            postio_ui::focus_target::Aim::Targets(aims) if !aims.is_empty() => {
                Some(ResultsAim::Targets {
                    aims,
                    conversations: picked.len(),
                })
            }
            _ => None,
        }
    }

    /// The query's positive clauses.
    fn filters(&self) -> Vec<Filter> {
        self.parsed
            .filters()
            .filter(|clause| !clause.negated)
            .map(|clause| clause.filter.clone())
            .collect()
    }

    /// A person's name for an address, from the people the answers named.
    pub(crate) fn name_of(&self, address: &str) -> Option<String> {
        self.people
            .iter()
            .find(|person| person.address.eq_ignore_ascii_case(address))
            .and_then(|person| person.name.clone())
    }

    /// The field's chips and words, and the filter bar's buttons.
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
                    // A person reads as who they are ("from: Ada Moreno",
                    // screen 10); the query keeps the address, and a value
                    // nobody is named by is drawn as typed.
                    let person = |who: &str| {
                        name_of(who)
                            .filter(|name| !name.is_empty())
                            .unwrap_or_else(|| who.to_owned())
                    };
                    let value = match &clause.filter {
                        Filter::From(_) | Filter::To(_) => person(value),
                        // Either of several (D26): every value, by name
                        // when it is a person -- "Ada Moreno, Tomás Reyes".
                        Filter::AnyOf(set) => set
                            .members()
                            .iter()
                            .filter_map(|member| match member {
                                Filter::From(who) | Filter::To(who) => Some(person(who)),
                                Filter::Subject(text)
                                | Filter::In(text)
                                | Filter::Filename(text)
                                | Filter::List(text)
                                | Filter::Account(text)
                                | Filter::Group(text)
                                | Filter::Label(text) => Some(text.clone()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join(", "),
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
            })
            .collect();
        QueryView {
            chips,
            words: plain.join(" "),
            hint: words::TO_EDIT.to_owned(),
            hint_key: None,
            buttons,
        }
    }

    /// The frame, drawn.
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
                frame.map_or(0, |frame| frame.files),
                false,
                CommandId::ResultsFiles,
            ),
            tab(
                ResultsTab::People,
                words::TABS[2],
                frame.map_or(0, |frame| frame.people),
                false,
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
                // Top hits is at most three and says why instead: screen
                // 06 draws no count beside it.
                count: if span.top_hits {
                    String::new()
                } else {
                    words::tab_count(span.count, false)
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
            order: self.order,
            count_line: frame.map_or_else(String::new, |_| words::count_line(total, capped)),
            sub_line: frame.map_or_else(String::new, |frame| {
                self.previewing(with)
                    .unwrap_or_else(|| words::sub_line(frame.files, frame.people))
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
            cursor: self.cursor.filter(|_| self.ready()),
            hints: if self.found_nothing() {
                words::no_results_hints(with.keymap, self.ways_out().len())
            } else {
                words::results_hints(with.keymap)
            },
            footer: frame.map_or_else(String::new, |frame| {
                if self.found_nothing() {
                    words::searched_footer(frame.messages_searched, frame.elapsed)
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
        }
    }

    /// "previewing From: Ada Moreno · ↩ applies", while a popover's
    /// changes are on screen and not yet applied.
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

    /// What the open list popover offers, in its base's order.
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
            FilterKind::From => base
                .facets
                .senders
                .iter()
                .filter_map(|count| person(count, "from"))
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
                        title: name.clone(),
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

    /// Where the query holds `filter`, as a clause of its own or one value
    /// of a set (D26): its token, and whether it is excluded there.
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

    /// The open popover, drawn.
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

    /// The edit a check on the popover's row `token` makes: Space toggles
    /// it, ⌥-click excludes it, or takes its exclusion out. A second value
    /// of the field joins the first as either of them (D26); the rules are
    /// `postio_search::edit`'s.
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

    /// The timeline's bars: the months of the query without its dates,
    /// when they are known, each marked when the query's dates take it in.
    fn month_bars(&self) -> Vec<MonthBar> {
        let Some(frame) = self.frame.as_ref() else {
            return Vec::new();
        };
        let (after, before) = self.dates();
        let undated = self
            .undated
            .as_ref()
            .filter(|(query, _)| *query == undated_query(&self.parsed));
        let months = undated.map_or(&frame.facets.months, |(_, months)| months);
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

    /// The months the query's dates take in, as the timeline says them:
    /// "Jul – Sep"; an open end runs to this month.
    fn range(&self, today: NaiveDate, year: bool) -> Option<String> {
        let (first, last) = self.range_months(today)?;
        Some(words::month_range(first, last, year))
    }

    /// The first and last month the dates take in (any day of each).
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

    /// The dates ⌥← (`back`) or ⌥→ step the range to: both bounds a month,
    /// an open end closed at this month first; no range is this month,
    /// and nothing steps past it.
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

    /// The query's dates: `after:` and `before:`.
    fn dates(&self) -> (Option<NaiveDate>, Option<NaiveDate>) {
        dates_of(&self.parsed)
    }

    /// The months `first..=last` of the timeline, as `after:` and
    /// `before:`.
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

    /// The row at `position`, drawn; `None` until its page is read.
    pub(crate) fn row(&self, position: u64, with: &Words<'_>) -> Option<ResultRow> {
        let (hit, group, top_hit) = self.hit_at(position)?;
        let names = self.frame.as_ref().map(|frame| &frame.names);
        let sender = hit
            .from
            .as_ref()
            .map(postio_ui::command_bar::said_of)
            .unwrap_or_default();
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
            })
            .unwrap_or_default();
        let date = words::hit_date(hit.newest_match.with_timezone(&Local), with.now);
        let passage_text: String = passage.iter().map(|run| run.text.as_str()).collect();
        let reason = top_hit
            .then(|| words::reason_line(&hit.reasons))
            .filter(|line| !line.is_empty());
        Some(ResultRow {
            message: hit.best,
            thread: thread_of(hit),
            group,
            top_hit,
            accessible: words::accessible_row(
                &sender,
                &subject,
                &source_tag,
                &passage_text,
                &folder_name,
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
            source_tag,
            source_is_file,
            passage,
            folder: if folder_name.is_empty() {
                String::new()
            } else {
                words::in_folder(&folder_name)
            },
            date,
            checked: self.is_checked(hit.best),
        })
    }

    /// Quick Look on the focused row: what it reads, asked under `stamp`;
    /// `None` while the row is not read yet.
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
            stamp,
        });
        Some(Request::QuickLookMatches {
            query: self.parsed.clone(),
            key,
            stamp,
        })
    }

    /// The conversation's matches landed: the cards are theirs, and the
    /// ring on the first that is the row's own message. Whether Quick Look
    /// changed.
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
        look.current = found
            .iter()
            .position(|each| each.message == Some(look.hit.best))
            .unwrap_or(0);
        look.matches = Some(found);
        true
    }

    /// ] or [: the ring a card on, or back. Whether it moved.
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
        true
    }

    /// Quick Look, drawn.
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
        // When the row's own message was sent, once its card says; until
        // then when the conversation last matched.
        let sent = matches
            .iter()
            .find(|each| each.message == Some(hit.best))
            .and_then(|each| each.found.when)
            .unwrap_or(hit.newest_match);
        let tail: Vec<String> = std::iter::once(words::sent_at(sent.with_timezone(&Local)))
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
                    // The subject before the conversation is read: the
                    // row's own, marked as the header marks it.
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
            position: words::quick_look_position(look.position, self.rows()),
            walk: words::quick_look_walk(with.keymap),
            actions: words::quick_look_actions(with.keymap),
            subject: dropdown::marked(
                &subject,
                &postio_search::highlight::find(&subject, &self.terms),
            ),
            sender,
            matches_line: words::matches_line(cards.len()),
            matches_hint: words::matches_hint(with.keymap),
            current: (!cards.is_empty()).then(|| look.current.min(cards.len() - 1) as u32),
            cards,
        })
    }

    /// The results as history keeps them.
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

/// Names a query view reads that the answer may not have yet.
pub(crate) struct QueryNames<'a> {
    pub(crate) lookup: &'a dyn Fn(&str) -> Option<String>,
}

/// What a landed page changed.
#[derive(Debug, Default)]
pub(crate) struct Landed {
    /// The frame is known now: draw it whole.
    pub(crate) frame: bool,
    /// Rows `first, count` changed.
    pub(crate) rows: Option<(u64, u64)>,
    /// The passages to ask for.
    pub(crate) passages: Option<Request>,
    /// Keep the query among the recent searches, with this many found.
    pub(crate) remember: Option<u64>,
    /// It found nothing: draw the no-results page and count its ways out.
    pub(crate) nothing: bool,
}

fn thread_of(hit: &ConversationHit) -> Option<ThreadId> {
    match hit.key {
        ConversationKey::Thread(thread) => Some(thread),
        ConversationKey::Lone(_) => None,
    }
}

/// The Save popover (design §3.9, screen 12): every word it draws, and
/// the switches as they start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveView {
    /// "Save as a saved search".
    pub title: String,
    /// "Name", over the field.
    pub name_label: String,
    /// The name offered: "Atlas budget from Ada".
    pub name: String,
    /// The terms, read-only: "from:Ada Moreno", "after:2026-07-01", the
    /// words.
    pub chips: Vec<String>,
    /// Pin to saved searches, as it starts: on.
    pub pin: bool,
    /// "Pin to saved searches".
    pub pin_label: String,
    /// "Appears at the top of search as", before its key.
    pub pin_note: String,
    /// The key it will run on (`alt+3`), the next free; `None` past the
    /// fourth.
    pub pin_key: Option<String>,
    /// Notify when new mail matches, as it starts: off.
    pub notify: bool,
    /// "Notify when new mail matches".
    pub notify_label: String,
    /// "A quiet badge, not a banner".
    pub notify_note: String,
    /// Keep the date rolling, as it starts: off.
    pub rolling: bool,
    /// "Keep the date rolling".
    pub rolling_label: String,
    /// "Off: always since 1 July. On: always the last 90 days"; `None`
    /// when the query has no date to keep, and the switch is not drawn.
    pub rolling_note: Option<String>,
    /// "Cancel".
    pub cancel: String,
    /// "Save".
    pub save: String,
    /// Save's key (`Return`).
    pub save_key: Option<String>,
}

/// Where a verb on the results goes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ResultsAim {
    /// These conversations, named.
    Targets {
        /// The targets, in the order to send them.
        aims: Vec<postio_core::MessageTarget>,
        /// How many conversations they are: what a picker says it acts on.
        conversations: usize,
    },
    /// Every conversation `query` matches but the rows `except` stands for.
    Matching {
        /// The query.
        query: ParsedQuery,
        /// Each a conversation's best message.
        except: Vec<MessageId>,
    },
}

/// `command`, sent where `aim` says.
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

/// Show `view` whole.
pub(crate) fn show(view: ResultsView) -> Step {
    Step::Show(Intent::ShowResults(Box::new(view)))
}

/// The commands the results answer while nothing is over them.
const RESULTS_KEYS: [CommandId; 25] = [
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
];

/// Whether the results, up, answer `id` themselves: their own keys, and
/// the verbs on a row, which act on the focused result.
pub(crate) fn results_key(id: CommandId) -> bool {
    RESULTS_KEYS.contains(&id)
        || matches!(
            id,
            CommandId::Reply | CommandId::ReplyAll | CommandId::Forward
        )
        || postio_ui::focus_target::dispatch(id).is_some()
        || crate::pickers::opens_picker(id)
}

impl FocusController {
    /// Whether the main window is showing a search's results (spec 010
    /// D17).
    pub fn in_results(&self) -> bool {
        self.results.is_some()
    }

    /// The results' query, while they are up.
    pub(crate) fn results_query(&self) -> Option<String> {
        self.results.as_ref().map(|results| results.query.clone())
    }

    /// How many rows the results draw.
    pub fn result_count(&self) -> u64 {
        self.results.as_ref().map_or(0, Results::rows)
    }

    /// The result at `position`, drawn; `None` while its page is on its
    /// way ([`results_wanted`](Self::results_wanted) asks for it).
    pub fn result_row(&self, position: u64) -> Option<ResultRow> {
        let words = self.results_words();
        self.results.as_ref()?.row(position, &words)
    }

    /// A row the table wants and [`result_row`](Self::result_row) could
    /// not give: its page is read, and `Intent::ResultsPage` says when.
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
        }
    }

    /// The results' frame and the field's query, drawn whole.
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
                buttons: Vec::new(),
            };
        };
        let mut view = results.query_view(self.bar.now().date_naive(), &names);
        // D24: with nothing found, ⌘⌫ clears the filters, and the field
        // says so where it says "/ to edit".
        if results.found_nothing()
            && let Some(key) = postio_ui::hints::key(self.bar.keymap(), CommandId::BackToWords)
        {
            view.hint = words::CLEARS_FILTERS.to_owned();
            view.hint_key = Some(key);
        }
        view
    }

    /// The no-results page, drawn; `None` while the search found
    /// something.
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

    /// Run the way out at `at` (from 0), when there is one: the query is
    /// edited in place, as a chip's ✕ edits it.
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

    /// The focus moved among the ways out: the page and the field's ring.
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

    /// The current place, as history keeps it.
    fn here(&self) -> Entry {
        match &self.results {
            Some(results) => Entry::Results(results.snapshot()),
            None => Entry::Inbox,
        }
    }

    /// ⌘↩, or Show all: the main window turns into the results for what
    /// is typed, and what it showed goes behind them (D17).
    pub(crate) fn show_all_results(&mut self) -> Vec<Step> {
        let Some(typed) = self.bar.search_words().map(str::to_owned) else {
            return Vec::new();
        };
        let leaving = self.here();
        self.history.visit(leaving);
        // The field holds the query the words were lowered to, so a chip's
        // token is the token the edits count (D1).
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

    /// Show `snapshot`'s results, asking for them again.
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

    /// Back to the inbox, on its own cursor and selection.
    fn show_inbox(&mut self) -> Vec<Step> {
        let mut steps = self.close_quick_look();
        self.results = None;
        steps.push(Step::Show(Intent::LeaveResults));
        steps.extend(self.cursor.redraw(self.feed.total()));
        steps
    }

    /// Show `entry`, from history. A popover open over what it leaves
    /// goes with it, unapplied: history keeps the query it opened on.
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

    /// ⌘[ and ⌘]: from the inbox or the results, nothing closed on the way
    /// (D18). `None` when `id` is neither.
    pub(crate) fn history_command(&mut self, id: CommandId) -> Option<Vec<Step>> {
        if !self.policy.caps.results_view {
            return None;
        }
        let here = self.here();
        let to = match id {
            CommandId::HistoryBack => self.history.back(here),
            CommandId::HistoryForward => self.history.forward(here),
            _ => return None,
        };
        Some(to.map(|entry| self.show_entry(entry)).unwrap_or_default())
    }

    /// A command while the results are up and nothing is over them;
    /// `None` hands it on to the list's table (`/`, `c`, `?`, the palette).
    pub(crate) fn results_command(&mut self, id: CommandId) -> Option<Vec<Step>> {
        if let Some(steps) = self.quick_look_command(id) {
            return Some(steps);
        }
        let results = self.results.as_mut()?;
        // Nothing found: the keys walk and run its ways out (US6, D24).
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
                cursor_moved(results.step(by))
            }
            CommandId::FirstMessage => cursor_moved(results.step(i64::MIN / 2)),
            CommandId::LastMessage => cursor_moved(results.step(i64::MAX / 2)),
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
            // ⇧X selects every conversation the query matches, as a
            // predicate (FR-026).
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
            CommandId::ToggleResultOrder => {
                let order = match results.order {
                    ConversationOrder::BestMatch => ConversationOrder::Newest,
                    _ => ConversationOrder::BestMatch,
                };
                self.results_order(order)
            }
            // `!` in the results is the Has action filter, as its button.
            CommandId::ToggleHasAction => {
                self.results_edit(postio_search::edit::Edit::Toggle(Filter::HasAction))
            }
            // ⌘S: the Save popover, from the toolbar's button (§3.9).
            // Nothing is written until its Save.
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
                    _ => crate::ComposerKind::Forward,
                };
                match results.cursor_message() {
                    Some((message, _)) => self.write(kind, Some(message)),
                    None => Vec::new(),
                }
            }
            // A picker hangs from the focused result, over what a verb
            // would aim at.
            _ if crate::pickers::opens_picker(id) => self.picker_on_results(id).unwrap_or_default(),
            // A verb on mail acts on what is checked, or the focused
            // result. One on nothing -- Undo, Refresh -- goes on to the
            // list's path, which sends it as it is: ⌘Z after archiving a
            // result takes it back.
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

    /// A verb this client sent has landed, or was taken back: the results
    /// are asked again, so a row says where its mail is now ("in:Archive")
    /// and one deleted is gone. Search spans every folder, so an archived
    /// result still matches; the focus ring stays where it was.
    pub(crate) fn results_heard(&mut self, event: &postio_core::Event) -> Vec<Step> {
        if !matches!(
            event,
            postio_core::Event::ActionCompleted { .. } | postio_core::Event::UndoPerformed { .. }
        ) {
            return Vec::new();
        }
        let stamp = self.stamp();
        match self.results.as_mut() {
            Some(results) => results.again(stamp),
            None => Vec::new(),
        }
    }

    /// The results' checks go, after a picker acted on them.
    pub(crate) fn clear_results_checks(&mut self) -> Vec<Step> {
        match self.results.as_mut() {
            Some(results) if results.any_checked() => {
                results.clear_checked();
                self.redraw_checks()
            }
            _ => Vec::new(),
        }
    }

    /// The Save popover for the results' query (§3.9, screen 12): a name
    /// made from it, its terms as read-only chips, and the three switches
    /// as they start. `None` with no query.
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

    /// Save ↩ in the Save popover: it goes, and the frontend writes the
    /// save; how it went comes back as `Input::SearchSaved`.
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

    /// The checks changed: the frame's count and bar, and every row.
    fn redraw_checks(&self) -> Vec<Step> {
        let rows = self.results.as_ref().map_or(0, Results::rows);
        let mut steps = self.draw_results();
        steps.push(Step::Show(Intent::ResultsPage {
            first: 0,
            count: rows,
        }));
        steps
    }

    /// A verb from the results: at what is checked -- the whole match as a
    /// predicate under ⇧X -- or the focused result. What was checked has
    /// been acted on, so it goes.
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

    /// A verb on the focused result.
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

    /// Quick Look's keys (spec 010 US4, FR-028, design §3.7): Space opens
    /// and closes it; while it is open, ]/[ ring the next and previous
    /// match, j/k move the results and it follows in place, ↩ opens the
    /// message in its place, `a` archives and it moves on, Esc closes it
    /// first. `None` when `id` is not one of its keys now.
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
            // The verb, then the next result -- or, after the last, nothing
            // left to look at (spec edge case: "the panel moves to the next
            // result, or closes if none is left").
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

    /// Quick Look on the focused result: drawn from the row now, its
    /// conversation's matches asked for. Closed when the row is not read.
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

    /// Quick Look, drawn whole.
    fn draw_quick_look(&self) -> Vec<Step> {
        let words = self.results_words();
        self.results
            .as_ref()
            .and_then(|results| results.quick_look_view(&words))
            .map(|view| Step::Show(Intent::QuickLook(Some(Box::new(view)))))
            .into_iter()
            .collect()
    }

    /// Close Quick Look, when it is open.
    pub(crate) fn close_quick_look(&mut self) -> Vec<Step> {
        match self
            .results
            .as_mut()
            .and_then(|results| results.quick_look.take())
        {
            Some(_) => vec![Step::Show(Intent::QuickLook(None))],
            None => Vec::new(),
        }
    }

    /// Open the focused result, among the others its `j`/`k` walk.
    fn open_result(&mut self) -> Vec<Step> {
        let Some(results) = &self.results else {
            return Vec::new();
        };
        let Some((message, _)) = results.cursor_message() else {
            return Vec::new();
        };
        self.bar.walk(results.walk());
        self.show_hit(message)
    }

    /// The message a result opened is now `reading`: the focus ring
    /// follows it.
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
        // The Files and People tabs list their own (steps 9 and 10); the
        // frame says which is shown.
        let mut steps = self.close_quick_look();
        steps.extend(self.draw_results());
        steps
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

    /// Change the query by `edit` and ask again: the field shows the new
    /// query now, the results when they land.
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

    /// The open popover, drawn; nothing when none is.
    fn draw_popover(&self) -> Option<Step> {
        let today = self.bar.now().date_naive();
        let view = self.results.as_ref()?.popover_view(today)?;
        Some(Step::Show(Intent::Popover(Some(Box::new(view)))))
    }

    /// A filter button with a popover was pressed: open it over the query
    /// it will restore. Another one open closes first, as Esc would.
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
        let base = results.frame.as_ref().map(|frame| Base {
            facets: frame.facets.clone(),
            names: frame.names.clone(),
        });
        results.popover = Some(Popover {
            kind,
            before: results.query.clone(),
            base,
            filter: String::new(),
            words: String::new(),
        });
        steps.push(Step::Show(Intent::Query(self.query_view())));
        steps.extend(self.draw_popover());
        steps
    }

    /// The Date popover's plain words: the dates they name, previewed; no
    /// words put back the dates it opened on, and words that are no date
    /// change nothing but what it says they became.
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

    /// A Date preset: its `after:`, or no dates for Any time; Custom…
    /// writes nothing (the words are where a custom date goes).
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

    /// ↩ keeps what the popover previewed; Esc, or a click away, puts back
    /// the query it opened on, exactly (FR-027).
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

    /// A [`TermEdit`] from a control, in the query language's terms.
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

    /// The results' own inputs.
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
            Input::DateWords(text) => self.date_words(text),
            Input::DatePreset(token) => self.date_preset(token),
            _ => Vec::new(),
        }
    }

    /// An answer the results asked for.
    pub(crate) fn results_reply(&mut self, reply: Reply) -> Vec<Step> {
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
                if let Some(hits) = landed.remember {
                    steps.push(Step::Ask(Request::RememberSearch { query, hits }));
                }
                steps.extend(landed.passages.map(Step::Ask));
                steps
            }
            Reply::ResultsPassages { stamp, answer } => results
                .passages(stamp, answer)
                .map(|(first, count)| Step::Show(Intent::ResultsPage { first, count }))
                .into_iter()
                .collect(),
            Reply::QuickLookMatches { stamp, answer } => {
                if results.matches_landed(stamp, answer) {
                    self.draw_quick_look()
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

/// The focus ring moved to `position`, or nothing did.
fn cursor_moved(position: Option<u64>) -> Vec<Step> {
    position
        .map(|position| Step::Show(Intent::ResultsCursor(position)))
        .into_iter()
        .collect()
}

/// A lowered query as the field writes it: its tokens, one space apart.
fn spelled(parsed: &ParsedQuery) -> String {
    parsed
        .tokens()
        .iter()
        .map(|token| token.raw.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `parsed` without its `after:` and `before:`: what the timeline's bars
/// outside a range count.
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

/// The dates plain `words` lower to, and the terms they are written as:
/// "since july" is `after:2026-07-01`. `None` when they name no date.
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

/// A query's dates: its `after:` and `before:`.
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
