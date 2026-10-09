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
use postio_search::results::{
    ConversationHit, ConversationKey, ConversationOrder, ConversationResults, FacetNames, Match,
    ResultsTab, Source,
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
    /// Its count, grouped: "9".
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
    /// How many rows are checked.
    pub selected: u64,
    /// The bulk bar's keys, while rows are checked.
    pub bulk: Vec<Hint>,
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
    /// Its labels' names.
    pub labels: Vec<String>,
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
    /// The quiet hint on the right: "/ to edit".
    pub hint: String,
    /// The filter bar's buttons.
    pub buttons: Vec<FilterButton>,
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
}

impl Frame {
    fn of(results: &ConversationResults) -> Self {
        Frame {
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

/// The results mode.
#[derive(Debug)]
pub(crate) struct Results {
    /// The query, as the field holds it.
    pub(crate) query: String,
    parsed: ParsedQuery,
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
    /// Checked rows, by their best message.
    pub(crate) checked: Vec<MessageId>,
    /// The query has been kept among the recent searches.
    pub(crate) remembered: bool,
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
            remembered: false,
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
        self.asks()
    }

    /// Take `query`, lowered as `parsed`: the checked rows go with the
    /// rows they were, and the focus ring goes back to the top.
    pub(crate) fn set_query(&mut self, query: String, parsed: ParsedQuery) {
        self.terms = postio_search::highlight::terms(&parsed);
        self.query = query;
        self.parsed = parsed;
        self.checked.clear();
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

    /// `x` on the focused row: checked, or not.
    pub(crate) fn toggle_checked(&mut self) -> bool {
        let Some((message, _)) = self.cursor_message() else {
            return false;
        };
        match self.checked.iter().position(|at| *at == message) {
            Some(at) => {
                self.checked.remove(at);
            }
            None => self.checked.push(message),
        }
        true
    }

    /// The query's positive clauses.
    fn filters(&self) -> Vec<Filter> {
        self.parsed
            .filters()
            .filter(|clause| !clause.negated)
            .map(|clause| clause.filter.clone())
            .collect()
    }

    /// A person's name for an address, from the answer's people.
    fn name_of(&self, address: &str) -> Option<String> {
        let frame = self.frame.as_ref()?;
        frame
            .names
            .people
            .iter()
            .find(|(_, person)| person.address.eq_ignore_ascii_case(address))
            .and_then(|(_, person)| person.name.clone())
    }

    /// The field's chips and words, and the filter bar's buttons.
    pub(crate) fn query_view(&self, today: NaiveDate, names: &QueryNames<'_>) -> QueryView {
        let mut chips = Vec::new();
        let mut plain = Vec::new();
        for (index, token) in self.parsed.tokens().iter().enumerate() {
            match &token.kind {
                TokenKind::Filter(clause) => {
                    let raw = token.raw.trim_start_matches('-');
                    let (operator, value) = raw.split_once(':').unwrap_or((raw, ""));
                    chips.push(Chip {
                        token: index as u32,
                        operator: format!("{operator}:"),
                        value: value.trim_matches('"').to_owned(),
                        excluded: clause.negated,
                        focused: false,
                    });
                }
                TokenKind::Text(_) => plain.push(token.raw.clone()),
                TokenKind::Partial(_) => {}
            }
        }
        let filters = self.filters();
        let name_of = |address: &str| self.name_of(address).or_else(|| (names.lookup)(address));
        let buttons = FilterKind::ALL
            .iter()
            .map(|kind| FilterButton {
                kind: *kind,
                label: words::filter_button_label(*kind, &filters, &name_of, today),
                applied: filters.iter().any(|filter| kind.holds(filter)),
                open: false,
            })
            .collect();
        QueryView {
            chips,
            words: plain.join(" "),
            hint: words::TO_EDIT.to_owned(),
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
        let (after, before) = self.dates();
        let months = frame
            .map(|frame| {
                let tallest = frame
                    .facets
                    .months
                    .iter()
                    .map(|month| month.conversations)
                    .max()
                    .unwrap_or(0)
                    .max(1);
                frame
                    .facets
                    .months
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
            })
            .unwrap_or_default();
        let groups = self
            .spans()
            .into_iter()
            .map(|span| ResultGroup {
                title: span.title,
                count: words::tab_count(span.count, false),
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
                words::sub_line(frame.files, frame.people)
            }),
            months,
            groups,
            rows: self.rows(),
            cursor: self.cursor.filter(|_| self.ready()),
            hints: words::results_hints(with.keymap),
            footer: frame.map_or_else(String::new, |frame| {
                words::results_footer(total, capped, frame.elapsed)
            }),
            selected: self.checked.len() as u64,
            bulk: if self.checked.is_empty() {
                Vec::new()
            } else {
                words::bulk_hints(with.keymap)
            },
        }
    }

    /// The query's dates: `after:` and `before:`.
    fn dates(&self) -> (Option<NaiveDate>, Option<NaiveDate>) {
        let mut after = None;
        let mut before = None;
        for filter in self.filters() {
            match filter {
                Filter::After(date) => after = Some(date),
                Filter::Before(date) => before = Some(date),
                _ => {}
            }
        }
        (after, before)
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
                    names?
                        .labels
                        .iter()
                        .find(|(id, _)| id == label)
                        .map(|(_, name)| name.clone())
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
            checked: self.checked.contains(&hit.best),
        })
    }

    /// The results as history keeps them.
    pub(crate) fn snapshot(&self) -> crate::history::Snapshot {
        crate::history::Snapshot {
            query: self.query.clone(),
            tab: self.tab,
            order: self.order,
            cursor: self.cursor,
            checked: self.checked.clone(),
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
}

fn thread_of(hit: &ConversationHit) -> Option<ThreadId> {
    match hit.key {
        ConversationKey::Thread(thread) => Some(thread),
        ConversationKey::Lone(_) => None,
    }
}

/// Show `view` whole.
pub(crate) fn show(view: ResultsView) -> Step {
    Step::Show(Intent::ShowResults(Box::new(view)))
}

/// The commands the results answer while nothing is over them.
const RESULTS_KEYS: [CommandId; 16] = [
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
        self.results
            .as_ref()
            .map(|results| results.query_view(self.bar.now().date_naive(), &names))
            .unwrap_or(QueryView {
                chips: Vec::new(),
                words: String::new(),
                hint: String::new(),
                buttons: Vec::new(),
            })
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
        let parsed = self.bar.lower(&snapshot.query);
        let stamp = self.stamp();
        let mut results = Results::new(snapshot.query, parsed, snapshot.tab, snapshot.order, stamp);
        results.cursor = snapshot.cursor;
        results.checked = snapshot.checked;
        let asks = results.asks();
        self.results = Some(results);
        let mut steps = self.draw_results();
        steps.extend(asks);
        steps
    }

    /// Back to the inbox, on its own cursor and selection.
    fn show_inbox(&mut self) -> Vec<Step> {
        self.results = None;
        let mut steps = vec![Step::Show(Intent::LeaveResults)];
        steps.extend(self.cursor.redraw(self.feed.total()));
        steps
    }

    /// Show `entry`, from history.
    fn show_entry(&mut self, entry: Entry) -> Vec<Step> {
        match entry {
            Entry::Inbox => self.show_inbox(),
            Entry::Results(snapshot) => self.open_results(snapshot),
        }
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
        let results = self.results.as_mut()?;
        let steps = match id {
            CommandId::NextMessage | CommandId::PrevMessage => {
                let by = if id == CommandId::NextMessage { 1 } else { -1 };
                cursor_moved(results.step(by))
            }
            CommandId::FirstMessage => cursor_moved(results.step(i64::MIN / 2)),
            CommandId::LastMessage => cursor_moved(results.step(i64::MAX / 2)),
            CommandId::ToggleSelection => {
                if results.toggle_checked() {
                    let at = results.cursor.unwrap_or(0);
                    let mut steps = self.draw_results();
                    steps.push(Step::Show(Intent::ResultsPage {
                        first: at,
                        count: 1,
                    }));
                    steps
                } else {
                    Vec::new()
                }
            }
            // ⇧X selects every conversation the query matches, as a
            // predicate: step 6 (T094).
            CommandId::SelectAll => Vec::new(),
            CommandId::Back => {
                if results.checked.is_empty() {
                    let here = self.here();
                    self.history.home(here);
                    return Some(self.show_inbox());
                }
                results.checked.clear();
                let rows = results.rows();
                let mut steps = self.draw_results();
                steps.push(Step::Show(Intent::ResultsPage {
                    first: 0,
                    count: rows,
                }));
                steps
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
            CommandId::SaveSearch => {
                let query = results.query.clone();
                self.bar.set_saving(query.clone());
                vec![Step::Show(Intent::SaveSearch { query })]
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
            // A picker hangs from a row the results draw: step 6 (T100).
            _ if crate::pickers::opens_picker(id) => Vec::new(),
            _ if postio_ui::focus_target::dispatch(id).is_some() => {
                let Some((message, thread)) = results.cursor_message() else {
                    return Some(Vec::new());
                };
                let row = crate::RowFacts {
                    id: message,
                    digest: false,
                    threads: thread.into_iter().collect(),
                    writes: false,
                };
                self.verbs.command_on(id, &row).unwrap_or_default()
            }
            _ => return None,
        };
        Some(steps)
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
        self.draw_results()
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
        results.again(stamp)
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
        let mut steps = vec![Step::Show(Intent::Query(self.query_view()))];
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
            Input::ResultsOrder(order) => self.results_order(order),
            Input::ResultsPoint(position) => match self.results.as_mut() {
                Some(results) => cursor_moved(results.point(position)),
                None => Vec::new(),
            },
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
                let mut steps = Vec::new();
                if landed.frame {
                    steps.extend(self.draw_results());
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
