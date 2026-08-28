//! Reading a search query as chips, and what Backspace does to one.
//!
//! Canvas 2b: `/` opens a bar over the list, parsed operators become chips, and
//! Backspace pops a chip whole rather than nibbling a character off it. Search
//! is primary navigation, not a dialog — so there is no animation and nothing
//! to dismiss before typing.
//!
//! # Where the chips live
//!
//! The entry holds the *whole* query, and the chips are a parse of it drawn
//! alongside. They are a reading of what is typed, not a second store that could
//! disagree with it — which is why [`postio_search::ParsedQuery`] hands out
//! [`Span`](postio_search::query::Span)s into the input, and why
//! [`ParsedQuery::remove_token`] returns *the string to put back in the entry*.
//!
//! The alternative — lifting completed operators out of the entry into
//! standalone chips — is a nicer picture and a worse editor: the caret can no
//! longer move through the query, and every edit becomes a merge between two
//! representations. This way the entry is the truth and the chips follow it.
//!
//! # Where the widget went
//!
//! There is no `SearchBar` any more. `postio-cfd.1` folded the query bar and
//! the command palette into one box — [`crate::finder`] — which is where the
//! chips are now drawn. What stays here is the part that has nothing to do
//! with either surface: reading a parsed query as chips, and deciding what
//! Backspace means. Both are pure and tested without a display.
//!
//! # What else is here
//!
//! The rest of canvas 2b, in the order the eye meets it:
//!
//! | | |
//! |---|---|
//! | [`Live`] | the `14 hits · 11 ms` readout, and the pacing behind it |
//! | [`Facets`] | the Scope column and the Refine chips down the left |
//! | [`Preview`] | the focused result, with the match highlighted |
//! | [`View`] | the three of them, mounted into the shell as one surface |
//!
//! Each has a pure core that is tested without a display — [`readout`],
//! [`Pacer`], [`mark_html`] — and a widget that is only wiring.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{glib, pango};
use postio_model::MessageBody;
use postio_model::ids::MessageId;
use postio_search::ParsedQuery;
use postio_search::SearchHit;
use postio_search::facets::{Facets, Refinement, Scope};
use postio_search::query::{Field, TokenKind};

/// One chip: an operator the parser recognized in the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    /// Position of the token in [`ParsedQuery::tokens`], for popping it.
    pub index: usize,
    /// The exact source text, so what the chip says is what is in the entry.
    pub label: String,
    /// The operator it belongs to.
    pub field: Field,
    /// Whether it was negated with a leading `-`.
    pub negated: bool,
    /// Whether the operator has a value yet. A half-typed `from:` is still
    /// worth drawing — it tells the user the parser understood the keyword.
    pub complete: bool,
}

/// The chips to draw for a query, in the order they were typed.
///
/// Free text is not a chip: it stays plain, because it is the part the user is
/// usually still editing.
pub fn chips(parsed: &ParsedQuery) -> Vec<Chip> {
    parsed
        .tokens()
        .iter()
        .enumerate()
        .filter_map(|(index, token)| {
            let field = token.field()?;
            Some(Chip {
                index,
                label: token.raw.clone(),
                field,
                negated: token.negated(),
                complete: matches!(token.kind, TokenKind::Filter(_)),
            })
        })
        .collect()
}

/// What Backspace should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backspace {
    /// Delete one character, as usual.
    Ordinary,
    /// Take the whole chip out.
    PopChip {
        /// The token that went.
        index: usize,
        /// What the entry should now hold.
        query: String,
        /// Where the caret should now sit, in bytes.
        caret: usize,
    },
}

/// Decides what Backspace does with the caret at `caret` bytes into the query.
///
/// A chip pops when the caret is inside it or against its right edge — which is
/// where the caret is after typing one. Against its *left* edge the caret is
/// before the chip, not in it, so Backspace deletes what precedes as usual;
/// otherwise there would be no way to remove the space in front of a chip.
///
/// Free text is never popped whole. `subject:report` is one idea and deleting
/// it in one keystroke is a convenience; a word the user typed is a word, and
/// swallowing it would be a surprise.
pub fn backspace(parsed: &ParsedQuery, caret: usize) -> Backspace {
    let Some((index, token)) = parsed
        .tokens()
        .iter()
        .enumerate()
        .find(|(_, token)| token.span.contains(caret))
    else {
        return Backspace::Ordinary;
    };

    if !token.is_operator() || caret <= token.span.start {
        return Backspace::Ordinary;
    }

    // Where the join lands after `remove_token` trims the whitespace around the
    // hole it leaves.
    let caret = parsed.input()[..token.span.start].trim_end().len();
    Backspace::PopChip {
        index,
        query: parsed.remove_token(index),
        caret,
    }
}

/// How a chip reads to a screen reader.
pub fn spoken(chip: &Chip) -> String {
    let field = chip.field.keyword();
    let value = chip
        .label
        .split_once(':')
        .map(|(_, value)| value)
        .unwrap_or_default();
    match (chip.negated, chip.complete) {
        (false, true) => format!("{field} {value}"),
        (true, true) => format!("not {field} {value}"),
        (false, false) => format!("{field}, no value yet"),
        (true, false) => format!("not {field}, no value yet"),
    }
}

// ---------------------------------------------------------------------------
// The live readout — canvas 2b's `14 hits · 11 ms`
// ---------------------------------------------------------------------------

/// How long the box waits after a keystroke before it searches.
///
/// Sized to *typing*, not to the frame budget: people type at roughly
/// 150–250 ms a key, and the 60 ms this used to be fired between almost
/// every pair of keystrokes — typing `radon` searched `r`, `ra`, `rad`,
/// `rado`, `radon`, five queries for one question (#500). At 200 ms a word
/// typed at ordinary speed is one search, and the price is one beat between
/// the last keystroke and the answer. `Enter` does not wait: it flushes the
/// queued query immediately.
///
/// The keystroke itself never waits for a search — it only ever reschedules
/// one — which is what keeps typing inside the 16 ms interaction budget
/// regardless of this number.
pub const DEBOUNCE: Duration = Duration::from_millis(200);

/// The slot the readout reserves, in characters.
///
/// The slot the readout reserves, in characters.
///
/// Wide enough for the longest thing it can say — `10000+ hits · 100 ms` — so
/// the number never has to be truncated and the slot never has to resize
/// while somebody is typing.
const READOUT_CHARS: i32 = 20;

/// The slot while the corpus caveat is showing (#352).
///
/// Sized separately rather than making [`READOUT_CHARS`] wide enough for both.
/// The label is hidden at rest and takes its space only *during* a search —
/// canvas 2b draws the field wider while it is working — so a slot big enough
/// for the caveat would cost the field that width on every search, including
/// on the settled account that is the common case and the end state ADR 0016
/// guarantees. At the narrow breakpoint that is room the entry needs.
///
/// The reason a fixed slot exists at all is that the *number* changes per
/// keystroke and a field that breathed on every character would be the most
/// distracting thing on screen. The caveat does not do that: it changes when
/// backfill finishes, once. So the field resizes exactly once in an account's
/// life, which is not a twitch.
const READOUT_CHARS_SYNCING: i32 = 36;

/// What one search turned out to be.
///
/// The three numbers canvas 2b puts at the right-hand end of the field, and
/// the same three [`postio_search::SearchResults`] carries — this is that,
/// minus the hits themselves, because the readout does not need them and
/// copying a page of results to draw a number would be silly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// How many messages matched.
    pub hits: u64,
    /// Whether `hits` is a floor rather than the true count. See
    /// [`postio_search::SearchResults::total_hits_capped`].
    pub capped: bool,
    /// How long the search took.
    pub elapsed: Duration,
    /// Whether every message in the searched scope had a body to search.
    ///
    /// `false` adds the caveat: the hits come from a corpus that is still
    /// filling, so the count is a floor for a second reason (#352).
    pub corpus_complete: bool,
}

impl Outcome {
    /// Reads the outcome off a finished search.
    pub fn of(results: &postio_search::SearchResults) -> Self {
        Outcome {
            hits: results.total_hits,
            capped: results.total_hits_capped,
            elapsed: results.elapsed,
            corpus_complete: results.corpus_complete,
        }
    }
}

/// The readout, as the canvas writes it: `14 hits · 11 ms`.
///
/// No thousands separators, because the canvas' own scope counts are written
/// `4291` and two number formats in one column would read as two kinds of
/// number. A capped count is written `10000+ hits` rather than a bare figure,
/// so a floor never passes for a total.
/// A corpus still filling adds `· still syncing`, and nothing otherwise.
///
/// The wording is a *state that ends*, which is the whole of #352's design
/// call. ADR 0016 backfills every folder to completion by default, so "you do
/// not have this mail" would be false — the honest thing is that the answer is
/// not final yet. A count was rejected for the same reason: it would be a
/// draining queue reported as an alarm.
///
/// It says so once, here, rather than per result: a caveat repeated down a
/// list of hits stops being read by the third one.
pub fn readout(outcome: &Outcome) -> String {
    let base = format!("{} · {}", hits(outcome), elapsed(outcome.elapsed));
    match outcome.corpus_complete {
        true => base,
        false => format!("{base} · still syncing"),
    }
}

/// The readout as a screen reader should hear it — the same facts, in words,
/// because "·" and "ms" are punctuation and an abbreviation rather than
/// something to read aloud.
pub fn spoken_readout(outcome: &Outcome) -> String {
    let elapsed = outcome.elapsed.as_millis();
    let counted = match elapsed {
        0 => format!("{}, in under a millisecond", hits(outcome)),
        1 => format!("{}, in 1 millisecond", hits(outcome)),
        _ => format!("{}, in {elapsed} milliseconds", hits(outcome)),
    };
    // The spoken form carries the sentence the visible one has no room for.
    // Three words are enough to *flag* a state beside a number; they are not
    // enough to explain one to somebody who cannot see the rest of the
    // window.
    match outcome.corpus_complete {
        true => counted,
        false => format!(
            "{counted}. This account is still syncing, so messages whose text \
             has not arrived yet could not be searched."
        ),
    }
}

fn hits(outcome: &Outcome) -> String {
    match (outcome.hits, outcome.capped) {
        (_, true) => format!("{}+ hits", outcome.hits),
        (0, _) => "no hits".to_string(),
        (1, _) => "1 hit".to_string(),
        (hits, _) => format!("{hits} hits"),
    }
}

/// A duration, in the unit that makes it readable.
///
/// Sub-millisecond is written `<1 ms` rather than `0 ms`: the search did
/// happen, and a zero would read as one that did not.
fn elapsed(elapsed: Duration) -> String {
    let millis = elapsed.as_millis();
    match millis {
        0 => "<1 ms".to_string(),
        1..=9_999 => format!("{millis} ms"),
        _ => format!("{:.1} s", elapsed.as_secs_f64()),
    }
}

/// Which question is outstanding, so an answer to an older one can be thrown
/// away instead of drawn.
///
/// Every run gets a sequence number, and only the newest one's answer is
/// accepted. This is the same generation rule [`crate::feed`] uses for message
/// pages and for the same reason: superseding a query is the *normal* case
/// when results follow every keystroke, and without it the readout flickers
/// backwards through the answers to queries nobody is asking any more.
///
/// Pure, and deliberately not a widget: the rule is worth testing on its own,
/// and it is the whole of what "cancelled, not awaited" means.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pacer {
    issued: u64,
}

impl Pacer {
    /// The sequence number of the outstanding run.
    pub fn issued(&self) -> u64 {
        self.issued
    }
}

impl Pacer {
    /// Hands out the sequence number for a new run, superseding whatever was
    /// in flight.
    pub fn issue(&mut self) -> u64 {
        self.issued += 1;
        self.issued
    }

    /// Whether `sequence`'s answer is still the answer to the current
    /// question.
    pub fn accepts(&self, sequence: u64) -> bool {
        sequence != 0 && sequence == self.issued
    }

    /// Gives up on whatever is in flight without asking anything new — the box
    /// closed, or emptied.
    pub fn abandon(&mut self) {
        self.issued += 1;
    }
}

/// What to run when a debounced query comes due.
type RunHandler = Box<dyn Fn(&ParsedQuery, u64)>;

/// The readout and the pacing behind it.
///
/// Drives one label. Every keystroke goes to [`Live::typed`], which
/// reschedules the pending run rather than starting another one, and every
/// answer comes back through [`Live::deliver`], which drops it if the question
/// has moved on.
///
/// # What the readout does while a search is running
///
/// Nothing: it goes on showing the last answer. A spinner over a local read is
/// the anti-pattern `/ux-architect` names outright, and a readout that blanked
/// between keystrokes would flash on every one. The number is either the
/// answer to what is typed or the answer to what was typed a few tens of
/// milliseconds ago, and at that distance the difference is not visible.
#[derive(Clone)]
pub struct Live {
    inner: Rc<LiveInner>,
}

struct LiveInner {
    label: gtk::Label,
    pacer: RefCell<Pacer>,
    /// The debounce timer, cancelled by the next keystroke.
    pending: RefCell<Option<glib::SourceId>>,
    /// The query that timer will run when it fires.
    queued: RefCell<Option<ParsedQuery>>,
    /// The last query [`Live::typed`] was told about, so a redraw that did
    /// not change the query does not read as a keystroke.
    asked: RefCell<Option<ParsedQuery>>,
    outcome: Cell<Option<Outcome>>,
    run: RefCell<Vec<RunHandler>>,
    /// The sequence number of the run whose answer has not come back yet.
    ///
    /// One search in flight at a time (#500): while this is set, [`flush`]
    /// leaves the queued query where it is, and [`deliver`] releases it. On
    /// a store answering in single-digit milliseconds nobody can tell; on a
    /// store gone slow — cold cache, a backfill on the same disk — this is
    /// what keeps a burst of typing from stacking a search per keystroke
    /// onto a pool that is already struggling.
    ///
    /// [`flush`]: Live::flush
    /// [`deliver`]: Live::deliver
    in_flight: Cell<Option<u64>>,
}

impl Live {
    /// Drives `label` — the readout at the right-hand end of the field.
    pub fn new(label: gtk::Label) -> Self {
        label.add_css_class("postio-readout");
        // A fixed slot, right-aligned. The count changes width on almost
        // every keystroke (`9 hits` to `1204 hits` to `no hits`), and a
        // field that breathed in the header on every character would be the
        // most distracting thing on the screen. Hidden at rest, so the
        // resting field is still the canvas' own width — 2b draws the field
        // wider while it is working, which is exactly what this is.
        label.set_width_chars(READOUT_CHARS);
        label.set_xalign(1.0);
        // The number is decoration for the field's own label until there is
        // one; `render` gives it a real description as soon as there is.
        label.set_accessible_role(gtk::AccessibleRole::Status);
        label.set_visible(false);
        let live = Live {
            inner: Rc::new(LiveInner {
                label,
                pacer: RefCell::new(Pacer::default()),
                pending: RefCell::new(None),
                queued: RefCell::new(None),
                asked: RefCell::new(None),
                outcome: Cell::new(None),
                run: RefCell::new(Vec::new()),
                in_flight: Cell::new(None),
            }),
        };
        live.render();
        live
    }

    /// Called when a debounced query comes due, with the sequence number its
    /// answer has to come back under.
    pub fn connect_run(&self, handler: impl Fn(&ParsedQuery, u64) + 'static) {
        self.inner.run.borrow_mut().push(Box::new(handler));
    }

    /// The query changed.
    ///
    /// Reschedules the pending run and supersedes any run already in flight,
    /// so the answer to the query being replaced is never drawn. Costs a timer
    /// reset and nothing else, which is what keeps this inside the interaction
    /// budget however fast the user types.
    ///
    /// A call that does not actually change the query does nothing at all.
    /// The box redraws for reasons that have nothing to do with typing — the
    /// keyboard context moved, the folder list arrived — and each of those
    /// would otherwise cancel a search that was about to answer the question
    /// still on screen.
    pub fn typed(&self, query: &ParsedQuery) {
        let inner = &self.inner;
        if inner.asked.borrow().as_ref() == Some(query) {
            return;
        }
        *inner.asked.borrow_mut() = Some(query.clone());
        self.cancel_pending();
        // Anything in flight is now answering a question nobody is asking.
        inner.pacer.borrow_mut().abandon();

        if query.is_empty() {
            // Nothing is being asked, so there is no number to show. Not
            // "0 hits": an empty box has not searched and must not claim to
            // have found nothing.
            inner.outcome.set(None);
            *inner.queued.borrow_mut() = None;
            self.render();
            return;
        }

        *inner.queued.borrow_mut() = Some(query.clone());
        let source = glib::timeout_add_local_once(
            DEBOUNCE,
            glib::clone!(
                #[strong(rename_to = live)]
                self,
                move || {
                    // Cleared before running, so `cancel_pending` never
                    // removes a source that has already fired.
                    *live.inner.pending.borrow_mut() = None;
                    live.flush();
                }
            ),
        );
        *inner.pending.borrow_mut() = Some(source);
    }

    /// Asks the last query again, now.
    ///
    /// Not typing, so there is no debounce to wait out: the scope changed, or
    /// something else the user did deliberately means the same query now has
    /// a different answer. Does nothing if there is no query.
    pub fn rerun(&self) {
        let asked = self.inner.asked.borrow().clone();
        let Some(query) = asked else { return };
        *self.inner.queued.borrow_mut() = Some(query);
        self.flush();
    }

    /// Runs the queued query now instead of waiting out the debounce.
    ///
    /// What `Enter` does, and what a test does instead of sleeping.
    pub fn flush(&self) {
        self.cancel_pending();
        if self.inner.in_flight.get().is_some() {
            // The store is still answering the last question. The queued
            // query waits — `deliver` (or `settled`) sends it the moment the
            // answer lands, and a newer keystroke replaces it while it
            // waits. This is what "one search in flight" means.
            return;
        }
        let Some(query) = self.inner.queued.borrow_mut().take() else {
            return;
        };
        let sequence = self.inner.pacer.borrow_mut().issue();
        self.inner.in_flight.set(Some(sequence));
        for handler in self.inner.run.borrow().iter() {
            handler(&query, sequence);
        }
    }

    /// A run answered.
    ///
    /// Returns whether the answer was taken. `false` means the query was
    /// superseded while it ran, and the caller should drop the rest of the
    /// results too rather than filling the list with them.
    pub fn deliver(&self, sequence: u64, outcome: Outcome) -> bool {
        if self.inner.in_flight.get() == Some(sequence) {
            self.inner.in_flight.set(None);
        }
        let taken = if self.inner.pacer.borrow().accepts(sequence) {
            self.inner.outcome.set(Some(outcome));
            self.render();
            true
        } else {
            false
        };
        // Whether or not the answer was worth drawing, the store is free
        // now: if a query queued up behind this run, it goes out.
        if self.inner.in_flight.get().is_none() && self.inner.queued.borrow().is_some() {
            self.flush();
        }
        taken
    }

    /// The run for `sequence` ended without an answer — the store could not
    /// be read, or the search failed.
    ///
    /// The single-flight rule holds a queued query until the outstanding run
    /// resolves, so a run that dies silently would wedge the box: nothing in
    /// flight ever answers, and nothing queued ever runs. Whoever answers
    /// [`connect_run`] must call this on every path that will not reach
    /// [`deliver`].
    ///
    /// [`connect_run`]: Live::connect_run
    /// [`deliver`]: Live::deliver
    pub fn settled(&self, sequence: u64) {
        if self.inner.in_flight.get() == Some(sequence) {
            self.inner.in_flight.set(None);
        }
        if self.inner.in_flight.get().is_none() && self.inner.queued.borrow().is_some() {
            self.flush();
        }
    }

    /// Stop searching and take the readout down — the box closed.
    pub fn clear(&self) {
        self.cancel_pending();
        self.inner.pacer.borrow_mut().abandon();
        *self.inner.queued.borrow_mut() = None;
        *self.inner.asked.borrow_mut() = None;
        self.inner.outcome.set(None);
        self.render();
    }

    /// The sequence number an answer must come back under to be drawn.
    ///
    /// Only useful to something answering synchronously — the ordinary path
    /// takes it from [`Live::connect_run`], which is handed the number for
    /// the run it is being asked to make.
    pub fn outstanding(&self) -> u64 {
        self.inner.pacer.borrow().issued()
    }

    /// What the readout is currently saying, if anything.
    pub fn outcome(&self) -> Option<Outcome> {
        self.inner.outcome.get()
    }

    /// Whether a run is scheduled or in flight.
    pub fn is_running(&self) -> bool {
        self.inner.queued.borrow().is_some()
    }

    fn cancel_pending(&self) {
        if let Some(source) = self.inner.pending.borrow_mut().take() {
            source.remove();
        }
    }

    fn render(&self) {
        let inner = &self.inner;
        match inner.outcome.get() {
            Some(outcome) => {
                // Only as wide as this outcome needs. See
                // `READOUT_CHARS_SYNCING`.
                inner.label.set_width_chars(match outcome.corpus_complete {
                    true => READOUT_CHARS,
                    false => READOUT_CHARS_SYNCING,
                });
                inner.label.set_text(&readout(&outcome));
                inner
                    .label
                    .update_property(&[gtk::accessible::Property::Label(&spoken_readout(
                        &outcome,
                    ))]);
                inner.label.set_visible(true);
            }
            None => {
                inner.label.set_text("");
                inner.label.set_visible(false);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Scope and refine — canvas 2b's left column
// ---------------------------------------------------------------------------

/// What the refine column says when it has nothing to offer.
///
/// Never a blank space and never a shrug: the two reasons a shortlist can be
/// empty are different, and which one it is decides what the next keystroke
/// should be.
const NOTHING_MATCHED: &str = "Nothing matched, so there is nothing to narrow.";
const NOTHING_TO_NARROW: &str = "Every match is alike — nothing left to narrow by.";

/// The keys the column offers, drawn at its foot.
///
/// Canvas 2b's third line, `C-s save as folder`: `CommandId::SaveSearch`
/// wires it (issue #10), so the hint can finally say something true.
const PANEL_KEYS: &str = "Ret open · Tab refine · C-s save as folder";

type ScopeHandler = Box<dyn Fn(Scope)>;
type RefineHandler = Box<dyn Fn(&str)>;

mod panel_imp {
    use super::*;

    pub struct Panel {
        pub(super) scopes: gtk::ListBox,
        pub(super) chips: gtk::FlowBox,
        pub(super) nothing: gtk::Label,
        /// The tokens currently drawn, in the order they are drawn.
        pub(super) offered: RefCell<Vec<String>>,
        pub(super) scope: Cell<Scope>,
        /// Set while the panel is moving its own selection, so putting the
        /// scope back does not read as the user picking it.
        pub(super) echoing: Cell<bool>,
        pub(super) on_scope: RefCell<Vec<ScopeHandler>>,
        pub(super) on_refine: RefCell<Vec<RefineHandler>>,
    }

    impl Default for Panel {
        fn default() -> Self {
            Panel {
                scopes: gtk::ListBox::new(),
                chips: gtk::FlowBox::new(),
                nothing: gtk::Label::new(None),
                offered: RefCell::new(Vec::new()),
                scope: Cell::new(Scope::default()),
                echoing: Cell::new(false),
                on_scope: RefCell::new(Vec::new()),
                on_refine: RefCell::new(Vec::new()),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Panel {
        const NAME: &'static str = "PostioSearchPanel";
        type Type = super::Panel;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for Panel {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for Panel {}
    impl BinImpl for Panel {}
}

glib::wrapper! {
    /// Canvas 2b's left column: which slice of the mailbox to search, and
    /// what else is true of what it found.
    ///
    /// Wears the sidebar's own classes, because it *is* the sidebar while a
    /// search is on screen — one list idiom, one selected-row treatment, one
    /// place the eye looks for "where am I".
    pub struct Panel(ObjectSubclass<panel_imp::Panel>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Panel {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl Panel {
    /// A column scoped to all mail, with nothing measured yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Which scope is active.
    pub fn scope(&self) -> Scope {
        self.imp().scope.get()
    }

    /// Put the column on `scope` without calling it a choice the user made.
    pub fn set_scope(&self, scope: Scope) {
        let imp = self.imp();
        if imp.scope.replace(scope) == scope {
            return;
        }
        self.select_scope_row(scope);
    }

    /// Draw what the query's result set turned out to be.
    ///
    /// `total` is the size of that set — what the readout is showing — and is
    /// what decides which refinements are worth offering. See
    /// [`postio_search::Facets::suggested`].
    pub fn set_facets(&self, facets: &Facets, total: u64) {
        let imp = self.imp();

        for (index, scope) in Scope::ALL.iter().enumerate() {
            let Some(row) = imp.scopes.row_at_index(index as i32) else {
                continue;
            };
            let hits = facets.hits(*scope);
            set_scope_count(&row, *scope, hits);
        }

        while let Some(child) = imp.chips.first_child() {
            imp.chips.remove(&child);
        }
        let offered = facets.suggested(total);
        for refinement in &offered {
            imp.chips.append(&self.refine_chip(refinement));
        }
        *imp.offered.borrow_mut() = offered
            .iter()
            .map(|refinement| refinement.token.clone())
            .collect();

        let empty = offered.is_empty();
        imp.chips.set_visible(!empty);
        imp.nothing.set_visible(empty);
        if empty {
            imp.nothing.set_text(if total == 0 {
                NOTHING_MATCHED
            } else {
                NOTHING_TO_NARROW
            });
        }
    }

    /// Called when the user picks a scope.
    pub fn connect_scope(&self, handler: impl Fn(Scope) + 'static) {
        self.imp().on_scope.borrow_mut().push(Box::new(handler));
    }

    /// Called when a refine chip is activated, with the token to append.
    pub fn connect_refine(&self, handler: impl Fn(&str) + 'static) {
        self.imp().on_refine.borrow_mut().push(Box::new(handler));
    }

    /// Put the keyboard on the first refine chip. What `Tab` does from the
    /// query box, per the canvas' own footer.
    ///
    /// Answers whether there was anything to move to, so the caller can let
    /// `Tab` mean what it usually means when there is not.
    pub fn focus_refine(&self) -> bool {
        // The chip itself, not the `GtkFlowBoxChild` wrapping it: the button
        // is what `Enter` and `Space` activate, and focusing the wrapper
        // would leave the keyboard one step short of doing anything.
        let Some(chip) = self
            .imp()
            .chips
            .child_at_index(0)
            .and_then(|child| child.child())
        else {
            return false;
        };
        chip.grab_focus()
    }

    /// The tokens currently offered, in the order they are drawn.
    ///
    /// What the column is offering is a fact about the result set, not a
    /// private detail of the widget — a test asserts on it, and so could a
    /// screen reader summary.
    pub fn offered(&self) -> Vec<String> {
        self.imp().offered.borrow().clone()
    }

    fn build(&self) {
        let imp = self.imp();
        // Not `.postio-sidebar`: the pane it sits in already wears that, so
        // the ground and the `.postio-kicker` / `.postio-rule` insets come
        // for free, and adding it here would be the same class on a widget
        // and its own ancestor.
        self.add_css_class("postio-search-panel");
        self.set_hexpand(false);

        imp.scopes.set_selection_mode(gtk::SelectionMode::Single);
        imp.scopes.add_css_class("postio-folders");
        imp.scopes
            .update_property(&[gtk::accessible::Property::Label("Search scope")]);
        for scope in Scope::ALL {
            imp.scopes.append(&scope_row(scope));
        }
        self.select_scope_row(Scope::default());

        imp.scopes.connect_row_selected(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_, row| {
                let Some(row) = row else { return };
                let index = row.index().max(0) as usize;
                let Some(scope) = Scope::ALL.get(index).copied() else {
                    return;
                };
                panel.imp().scope.set(scope);
                if panel.imp().echoing.get() {
                    return;
                }
                for handler in panel.imp().on_scope.borrow().iter() {
                    handler(scope);
                }
            }
        ));

        imp.chips.set_selection_mode(gtk::SelectionMode::None);
        imp.chips.set_max_children_per_line(3);
        imp.chips.set_row_spacing(6);
        imp.chips.set_column_spacing(6);
        imp.chips.set_homogeneous(false);
        imp.chips.add_css_class("postio-refine");
        imp.chips
            .update_property(&[gtk::accessible::Property::Label("Refine the search")]);

        imp.nothing.add_css_class("postio-refine-empty");
        imp.nothing.set_xalign(0.0);
        imp.nothing.set_wrap(true);
        imp.nothing.set_visible(false);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&kicker("Scope"));
        column.append(&imp.scopes);

        let rule = gtk::Separator::new(gtk::Orientation::Horizontal);
        rule.add_css_class("postio-rule");
        column.append(&rule);
        column.append(&kicker("Refine"));
        column.append(&imp.chips);
        column.append(&imp.nothing);

        let filler = gtk::Box::new(gtk::Orientation::Vertical, 0);
        filler.set_vexpand(true);
        column.append(&filler);

        // The keys this column offers, where the canvas puts them. Mono, and
        // the same shape the focused message row uses for its own hints.
        let keys = gtk::Label::new(Some(PANEL_KEYS));
        keys.add_css_class("postio-panel-keys");
        keys.set_xalign(0.0);
        keys.set_wrap(true);
        keys.set_accessible_role(gtk::AccessibleRole::Presentation);
        column.append(&keys);

        self.set_child(Some(&column));
    }

    fn select_scope_row(&self, scope: Scope) {
        let imp = self.imp();
        let index = Scope::ALL
            .iter()
            .position(|candidate| *candidate == scope)
            .unwrap_or(0);
        let Some(row) = imp.scopes.row_at_index(index as i32) else {
            return;
        };
        imp.echoing.set(true);
        imp.scopes.select_row(Some(&row));
        imp.echoing.set(false);
    }

    fn refine_chip(&self, refinement: &Refinement) -> gtk::Button {
        let button = gtk::Button::with_label(&refinement.token);
        button.add_css_class("postio-refine-chip");
        // A button, not a label with a click handler: the keyboard reaches it,
        // `Enter` and `Space` activate it, and a screen reader calls it what
        // it is. The count rides in the description rather than on the face —
        // the column is 212px wide and a scannable shortlist beats a wide one.
        button.set_tooltip_text(Some(&spoken_refinement(refinement)));
        button.update_property(&[gtk::accessible::Property::Label(&spoken_refinement(
            refinement,
        ))]);
        let token = refinement.token.clone();
        button.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| {
                for handler in panel.imp().on_refine.borrow().iter() {
                    handler(&token);
                }
            }
        ));
        button
    }
}

/// How a refine chip reads to a screen reader, and in its tooltip: the token
/// plus what taking it would leave.
pub fn spoken_refinement(refinement: &Refinement) -> String {
    match refinement.hits {
        1 => format!("{}, 1 match", refinement.token),
        hits => format!("{}, {hits} matches", refinement.token),
    }
}

/// One scope row: the name, and how many of the matches are in it.
fn scope_row(scope: Scope) -> gtk::ListBoxRow {
    let name = gtk::Label::new(Some(scope.label()));
    name.add_css_class("postio-folder-name");
    name.set_xalign(0.0);
    name.set_hexpand(true);
    name.set_ellipsize(pango::EllipsizeMode::End);

    let count = gtk::Label::new(None);
    count.add_css_class("postio-folder-count");

    let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    line.append(&name);
    line.append(&count);

    let row = gtk::ListBoxRow::new();
    row.add_css_class("postio-folder");
    row.set_child(Some(&line));
    set_scope_count(&row, scope, 0);
    row
}

/// Writes a scope row's count, and the sentence a screen reader hears.
///
/// A zero is drawn, unlike the sidebar's unread counts which hide at zero: an
/// empty scope is a fact worth knowing before switching to it, where an inbox
/// with nothing unread is just an ordinary inbox.
fn set_scope_count(row: &gtk::ListBoxRow, scope: Scope, hits: u64) {
    let Some(count) = row
        .child()
        .and_then(|line| line.last_child())
        .and_then(|label| label.downcast::<gtk::Label>().ok())
    else {
        return;
    };
    count.set_text(&hits.to_string());
    row.update_property(&[gtk::accessible::Property::Label(&match hits {
        1 => format!("{}, 1 match", scope.label()),
        hits => format!("{}, {hits} matches", scope.label()),
    })]);
}

/// A section heading, in the sidebar's own kicker type.
fn kicker(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("postio-kicker");
    label.set_xalign(0.0);
    label
}

// ---------------------------------------------------------------------------
// The preview — canvas 2b's right-hand pane
// ---------------------------------------------------------------------------

/// The reading measure the canvas sets on the preview's prose, in characters.
const PROSE_MEASURE: i32 = 58;

/// The kicker over the preview, from the artboard.
const PREVIEW_KICKER: &str = "preview · match highlighted";

/// What the pane says with no result focused.
const NOTHING_FOCUSED: &str = "Arrow through the results to preview one.";

/// What it says while a result's body is still only a snippet.
///
/// The commonest state in this application and not an error: headers sync
/// long before bodies do, so a search can find a message the store has never
/// fetched. The snippet FTS5 cut is genuinely all there is to show, and
/// saying so beats an empty pane that looks broken.
const BODY_NOT_HERE: &str = "The full message has not been fetched yet.";

type OpenHandler = Box<dyn Fn(MessageId)>;

mod preview_imp {
    use super::*;

    #[derive(Default)]
    pub struct Preview {
        pub(super) subject: gtk::Label,
        pub(super) snippet: gtk::Label,
        pub(super) note: gtk::Label,
        pub(super) body: gtk::Box,
        /// Holds the footer down while there is no body to do it. Hidden
        /// once there is, or the two would split the pane between them and
        /// the message would stop half way down it.
        pub(super) filler: gtk::Box,
        pub(super) open: gtk::Button,
        /// Built on first use, not at startup: a `WebKitWebView` is the most
        /// expensive widget in the application and the `<500ms` budget is
        /// measured from launch, where nothing has been searched yet.
        pub(super) reader: RefCell<Option<crate::reader::Reader>>,
        /// Where `cid:` parts come from, once something supplies them.
        /// Filled through a slot rather than at construction because
        /// `WebKitWebContext` registers its scheme handler once and for good.
        pub(super) blobs: RefCell<Option<Rc<dyn crate::reader::BlobSource>>>,
        pub(super) focused: RefCell<Option<MessageId>>,
        pub(super) terms: RefCell<Vec<String>>,
        pub(super) on_open: RefCell<Vec<OpenHandler>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Preview {
        const NAME: &'static str = "PostioSearchPreview";
        type Type = super::Preview;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for Preview {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for Preview {}
    impl BinImpl for Preview {}
}

glib::wrapper! {
    /// The focused result, with the match highlighted.
    ///
    /// Canvas 2b's right-hand pane: a kicker, the subject, the message, and
    /// `Open Ret`. The message is rendered by [`crate::reader::Reader`] —
    /// the same hardened `WebView` the reading pane uses, with JavaScript and
    /// network access off — because a preview is still someone else's HTML
    /// and a second, softer renderer would be a second way to be attacked.
    pub struct Preview(ObjectSubclass<preview_imp::Preview>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Preview {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl Preview {
    /// An empty preview.
    pub fn new() -> Self {
        Self::default()
    }

    /// Where inline (`cid:`) images resolve from.
    ///
    /// Without one they simply do not load, which is the safe default and the
    /// right one for a preview of a message whose parts have never been
    /// fetched.
    pub fn set_blob_source(&self, source: Rc<dyn crate::reader::BlobSource>) {
        *self.imp().blobs.borrow_mut() = Some(source);
    }

    /// The message currently previewed.
    pub fn focused(&self) -> Option<MessageId> {
        *self.imp().focused.borrow()
    }

    /// Preview `hit`, highlighting `terms`.
    ///
    /// Shows what the search already knows — the subject and the snippet FTS5
    /// cut — immediately. There is nothing to wait for: those came back with
    /// the hit, so arrowing through results never shows an empty frame while
    /// something loads.
    pub fn show(&self, hit: &SearchHit, terms: &[String]) {
        let imp = self.imp();
        let same = *imp.focused.borrow() == Some(hit.message_id);
        *imp.focused.borrow_mut() = Some(hit.message_id);
        *imp.terms.borrow_mut() = terms.to_vec();

        // The subject is not highlighted, and the canvas does not highlight
        // it either. It is already set in the heading face at heading weight,
        // so bold — the only treatment Pango markup can apply without a
        // hard-coded colour — is invisible on it. The snippet below carries
        // the highlighting, where regular-weight prose makes it obvious.
        let subject = hit.subject.as_deref().unwrap_or("(no subject)");
        imp.subject.set_text(subject);
        imp.subject.set_tooltip_text(Some(subject));

        let snippet = postio_search::highlight::from_snippet(&hit.snippet);
        let has_snippet = !snippet.text.trim().is_empty();
        imp.snippet.set_markup(&markup(&snippet));

        // Moving the focus to a different message means whatever body is on
        // screen belongs to the wrong one. Moving it to the *same* message —
        // a re-render after a refine, say — must not throw away a body that
        // has already arrived.
        if !same {
            imp.body.set_visible(false);
            imp.filler.set_visible(true);
            if let Some(reader) = imp.reader.borrow().as_ref() {
                reader.clear();
            }
        }
        imp.snippet
            .set_visible(!imp.body.is_visible() && has_snippet);
        imp.note.set_visible(!imp.body.is_visible());
        imp.note.set_text(BODY_NOT_HERE);
        imp.open.set_sensitive(true);
        self.set_accessible_label(&format!("Preview of {subject}"));
    }

    /// The body arrived for `message`.
    ///
    /// Ignored if the focus has moved on — the same rule the readout follows,
    /// for the same reason: arrowing down the results is faster than fetching
    /// a body, and a body that landed in the wrong preview would be worse
    /// than one that never landed.
    pub fn set_body(&self, message: MessageId, body: &MessageBody, sender: Option<&str>) {
        let imp = self.imp();
        if *imp.focused.borrow() != Some(message) {
            return;
        }
        let reader = self.reader();
        reader.set_highlight(imp.terms.borrow().clone());
        reader.render(body, sender);
        imp.body.set_visible(true);
        imp.filler.set_visible(false);
        imp.snippet.set_visible(false);
        imp.note.set_visible(false);
    }

    /// Nothing is focused.
    pub fn clear(&self) {
        let imp = self.imp();
        *imp.focused.borrow_mut() = None;
        imp.terms.borrow_mut().clear();
        imp.subject.set_text("");
        imp.snippet.set_text("");
        imp.snippet.set_visible(false);
        imp.body.set_visible(false);
        imp.filler.set_visible(true);
        imp.note.set_visible(true);
        imp.note.set_text(NOTHING_FOCUSED);
        imp.open.set_sensitive(false);
        if let Some(reader) = imp.reader.borrow().as_ref() {
            reader.clear();
        }
        self.set_accessible_label("Preview");
    }

    /// Called when the previewed message is opened.
    pub fn connect_open(&self, handler: impl Fn(MessageId) + 'static) {
        self.imp().on_open.borrow_mut().push(Box::new(handler));
    }

    /// Open what is previewed — what `Ret` and the `Open` button both do.
    ///
    /// One verb: this only *emits*, and whoever wired it dispatches the
    /// registry's own open command. A pane that opened a message itself would
    /// be a second implementation of a verb the registry already owns.
    pub fn open(&self) {
        let imp = self.imp();
        let Some(message) = *imp.focused.borrow() else {
            return;
        };
        for handler in imp.on_open.borrow().iter() {
            handler(message);
        }
    }

    /// The hardened reader, built the first time a body actually arrives.
    fn reader(&self) -> crate::reader::Reader {
        let imp = self.imp();
        if let Some(reader) = imp.reader.borrow().as_ref() {
            return reader.clone();
        }
        // The blob source is read through a slot on every request, so the
        // reader can be built before anything has supplied one and start
        // resolving `cid:` parts the moment something does.
        let blobs = self.clone();
        let source = move |content_id: &str| {
            let blobs = blobs.imp().blobs.borrow();
            blobs.as_ref().and_then(|source| source.resolve(content_id))
        };
        let reader = crate::reader::Reader::new(Rc::new(source));
        imp.body.append(&reader.widget());
        *imp.reader.borrow_mut() = Some(reader.clone());
        reader
    }

    fn set_accessible_label(&self, label: &str) {
        self.update_property(&[gtk::accessible::Property::Label(label)]);
    }

    fn build(&self) {
        let imp = self.imp();
        self.add_css_class("postio-preview");
        self.set_accessible_role(gtk::AccessibleRole::Group);

        let kicker = gtk::Label::new(Some(PREVIEW_KICKER));
        kicker.add_css_class("postio-preview-kicker");
        kicker.set_xalign(0.0);
        kicker.set_accessible_role(gtk::AccessibleRole::Presentation);

        imp.subject.add_css_class("postio-preview-subject");
        imp.subject.set_xalign(0.0);
        imp.subject.set_wrap(true);
        imp.subject.set_wrap_mode(pango::WrapMode::WordChar);
        imp.subject
            .set_accessible_role(gtk::AccessibleRole::Caption);

        imp.snippet.add_css_class("postio-preview-snippet");
        imp.snippet.set_xalign(0.0);
        imp.snippet.set_wrap(true);
        // The canvas' 58ch measure, not the pane's width: a line of prose
        // the full width of a wide window is one the eye loses its place in.
        // `halign` as well as `max-width-chars`, or the label is allocated
        // the whole pane and wraps at *that* instead of at its natural width.
        imp.snippet.set_max_width_chars(PROSE_MEASURE);
        imp.snippet.set_halign(gtk::Align::Start);
        imp.snippet.set_visible(false);

        imp.note.add_css_class("postio-preview-note");
        imp.note.set_xalign(0.0);
        imp.note.set_wrap(true);

        imp.body.set_orientation(gtk::Orientation::Vertical);
        imp.body.set_vexpand(true);
        imp.body.set_visible(false);

        imp.open
            .set_child(Some(&crate::header::labelled("Open", "Ret")));
        imp.open.add_css_class("suggested-action");
        imp.open.set_halign(gtk::Align::Start);
        imp.open
            .update_property(&[gtk::accessible::Property::Label("Open this message")]);
        imp.open.connect_clicked(glib::clone!(
            #[weak(rename_to = preview)]
            self,
            move |_| preview.open()
        ));

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        footer.add_css_class("postio-preview-footer");
        footer.append(&imp.open);

        imp.filler.set_orientation(gtk::Orientation::Vertical);
        imp.filler.set_vexpand(true);

        // The head and the footer carry the canvas' 28px inset; the body does
        // not. `reader.css` gives the message its own margin, and a message
        // indented twice — once by this pane and once by its own stylesheet —
        // would sit visibly further in than the subject above it.
        let head = gtk::Box::new(gtk::Orientation::Vertical, 0);
        head.add_css_class("postio-preview-head");
        head.append(&kicker);
        head.append(&imp.subject);
        head.append(&imp.snippet);
        head.append(&imp.note);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&head);
        column.append(&imp.body);
        column.append(&imp.filler);
        column.append(&footer);
        self.set_child(Some(&column));

        self.clear();
    }
}

// ---------------------------------------------------------------------------
// The search view — the three panes, while a search is on screen
// ---------------------------------------------------------------------------

/// Hides everything in `pane` except `mine`, recording what to put back.
///
/// The `visible` *property* rather than `is_visible()`, which in gtk4-rs
/// answers the effective question — visible, and every ancestor visible too —
/// and is therefore `false` for every child of a window that has not been
/// presented yet. Mounting before `present` is exactly what the shot does,
/// and reading the effective answer there left the folder list on screen
/// underneath the panel.
fn displace(pane: &gtk::Box, mine: &gtk::Widget, displaced: &mut Vec<(gtk::Widget, bool)>) {
    let mut child = pane.first_child();
    while let Some(current) = child {
        let next = current.next_sibling();
        if &current != mine {
            displaced.push((current.clone(), current.property::<bool>("visible")));
            current.set_visible(false);
        }
        child = next;
    }
}

/// Canvas 2b, mounted.
///
/// The artboard is not a new window or a new overlay: it is the three panes
/// the application already has, with the sidebar showing [`Panel`] instead of
/// the folder list. That is the whole of it, and it is deliberate — search is
/// primary navigation here, so it has to happen *in* the application rather
/// than on top of it, and `Esc` has to put everything back exactly as it was.
///
/// # One call to mount it
///
/// [`View::attach`] takes the shell and the box and wires the rest itself:
/// the panel appears when there is a query and goes away when there is not,
/// because both of those are things the box already announces. Whoever owns
/// the store answers [`Live::connect_run`] and hands the facets back through
/// [`View::set_facets`]; nothing else has to know this surface exists.
#[derive(Clone)]
pub struct View {
    inner: Rc<ViewInner>,
}

struct ViewInner {
    panel: Panel,
    preview: Preview,
    sidebar: gtk::Box,
    /// The reading pane's arbiter. The preview never touches its own
    /// visibility (#502): it registers as an occupant and the shell decides.
    shell: crate::shell::Shell,
    /// The sidebar children the panel displaced, each with the `visible` it
    /// had before, so leaving search puts back exactly what was there. Only
    /// the sidebar: the reading pane has one owner now, the shell.
    displaced: RefCell<Vec<(gtk::Widget, bool)>>,
    /// The terms the query is currently asking about, for the highlighting.
    terms: RefCell<Vec<String>>,
    active: Cell<bool>,
}

impl View {
    /// Mount the search surfaces into `shell` and wire them to `finder`.
    ///
    /// Called once, by whoever builds the window.
    pub fn attach(shell: &crate::shell::Shell, finder: &crate::finder::Finder) -> View {
        let panel = Panel::new();
        panel.set_vexpand(true);
        panel.set_visible(false);
        let sidebar = shell.sidebar();
        sidebar.append(&panel);

        let preview = Preview::new();
        preview.set_vexpand(true);
        shell.reader().append(&preview);
        shell.register_reader_occupant(
            crate::shell::ReaderOccupant::SearchPreview,
            preview.upcast_ref(),
        );

        let view = View {
            inner: Rc::new(ViewInner {
                panel,
                preview,
                sidebar,
                shell: shell.clone(),
                displaced: RefCell::new(Vec::new()),
                terms: RefCell::new(Vec::new()),
                active: Cell::new(false),
            }),
        };

        // The box already says when there is a query and when there is not,
        // so the surface follows it rather than being told twice.
        finder.connect_changed({
            let view = view.clone();
            move |parsed| {
                // The terms follow the query rather than the results: they
                // are what the *user asked for*, and the preview has to paint
                // them the moment a hit arrives rather than a round trip
                // later.
                *view.inner.terms.borrow_mut() = postio_search::highlight::terms(parsed);
                view.set_searching(!parsed.is_empty());
            }
        });
        finder.connect_dismissed({
            let view = view.clone();
            move || view.set_searching(false)
        });

        // Seed from whatever the box is already holding. `attach` runs at
        // window build, where that is nothing — but a surface that only ever
        // learns the query from the *next* keystroke is one that starts wrong
        // whenever it does not, and the shot mounts it after typing.
        {
            let parsed = finder.parsed();
            *view.inner.terms.borrow_mut() = postio_search::highlight::terms(&parsed);
            view.set_searching(!parsed.is_empty());
        }

        // Canvas 2b's footer: `Tab refine`.
        finder.connect_tab({
            let view = view.clone();
            move || view.panel().focus_refine()
        });

        // A refine chip appends a token the user could have typed, so it
        // lands in the box as an ordinary chip and Backspace pops it like any
        // other. The alternative — a filter held beside the query — would be
        // a second place the search is written down, and the two would
        // disagree the first time anyone edited either.
        view.panel().connect_refine({
            let finder = finder.clone();
            move |token| {
                let query = finder.query();
                finder.set_query(crate::finder::Query {
                    mode: crate::finder::Mode::Search,
                    text: postio_search::facets::append(&query.text, token),
                });
            }
        });

        // The scope is *not* written into the box — switching it must not
        // mean editing what was typed. So the same query is simply asked
        // again, against the new scope, which whoever answers reads off the
        // panel.
        view.panel().connect_scope({
            let finder = finder.clone();
            move |_| {
                if let Some(live) = finder.live() {
                    live.rerun();
                }
            }
        });

        view
    }

    /// Which slice of the mailbox the search is looking at.
    ///
    /// Whoever answers [`Live::connect_run`] reads this to build its request:
    /// the scope is state the panel owns, not something the query carries.
    pub fn scope(&self) -> Scope {
        self.inner.panel.scope()
    }

    /// The scope and refine column.
    pub fn panel(&self) -> Panel {
        self.inner.panel.clone()
    }

    /// The focused result, with the match highlighted.
    pub fn preview(&self) -> Preview {
        self.inner.preview.clone()
    }

    /// Preview `hit`, or nothing.
    ///
    /// Called as the focus moves through the results. The terms come from the
    /// query the box is holding, so the highlighting always answers what was
    /// actually asked rather than trailing it.
    pub fn set_focused(&self, hit: Option<&SearchHit>) {
        match hit {
            Some(hit) => {
                self.inner
                    .preview
                    .show(hit, &self.inner.terms.borrow().clone());
                // Browsing means previewing: if `Enter` had handed the pane
                // to the real reader, moving the focus takes it back.
                self.inner.shell.preview_focused();
            }
            None => self.inner.preview.clear(),
        }
    }

    /// The terms the preview is painting.
    pub fn terms(&self) -> Vec<String> {
        self.inner.terms.borrow().clone()
    }

    /// Whether the search surface is the one on screen.
    pub fn is_active(&self) -> bool {
        self.inner.active.get()
    }

    /// Draw what the query's result set turned out to be. See
    /// [`Panel::set_facets`].
    pub fn set_facets(&self, facets: &Facets, total: u64) {
        self.inner.panel.set_facets(facets, total);
    }

    /// Show or hide the search surface.
    ///
    /// Instant, with no transition: the motion budget is explicit that pane
    /// switches do not animate, and a column that slid in would be a column
    /// you wait for before you can read the counts on it.
    pub fn set_searching(&self, searching: bool) {
        let inner = &self.inner;
        if inner.active.replace(searching) == searching {
            return;
        }

        if searching {
            // The folder list steps aside rather than being unparented: it
            // keeps its selection, its scroll position and its
            // subscriptions, so `Esc` costs nothing. Only the sidebar is
            // snapshotted — the reading pane's occupants answer to the
            // shell's arbiter (#502), which needs no snapshot because what
            // shows after search leaves is computed from what is then
            // active.
            let mut displaced = inner.displaced.borrow_mut();
            displaced.clear();
            displace(
                &inner.sidebar,
                &inner.panel.clone().upcast(),
                &mut displaced,
            );
            inner.panel.set_visible(true);
            inner.shell.set_searching(true);
        } else {
            inner.panel.set_visible(false);
            inner.preview.clear();
            inner.shell.set_searching(false);
            for (widget, visible) in inner.displaced.borrow_mut().drain(..) {
                widget.set_visible(visible);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Painting the match — canvas 2b's "preview · match highlighted"
// ---------------------------------------------------------------------------

/// The class the reader stylesheet tints. See `reader.css`.
const MARK_CLASS: &str = "postio-match";

/// Tags whose contents are not prose and must not be marked.
///
/// `script` and `style` never survive [`crate::reader::sanitize`], and
/// `title` never appears in a body fragment — they are here because
/// "the sanitizer removes it" is a fact about another module, and a
/// highlighter that would corrupt a stylesheet if one ever reached it is one
/// bad refactor away from doing so.
const OPAQUE_TAGS: [&str; 3] = ["script", "style", "title"];

/// Wraps every place `terms` match in `html` with a `<mark>` the reader
/// stylesheet tints.
///
/// Applied *after* sanitizing, not before: ammonia would strip the `<mark>`
/// as an unknown tag, and marking first would mean running a matcher over
/// markup that has not been cleaned yet. What goes in is already-safe HTML
/// and what comes out adds one fixed literal tag to it — no attacker-shaped
/// string is ever interpolated.
///
/// Matches never cross a tag boundary. `<b>mail</b>dir` is two text runs and
/// FTS5 would not have matched `maildir` across them either, so the
/// highlighting agrees with why the message was a hit.
pub fn mark_html(html: &str, terms: &[String]) -> String {
    if terms.is_empty() {
        return html.to_owned();
    }

    let mut out = String::with_capacity(html.len());
    let mut run = String::new();
    let mut rest = html;
    // `Some(tag)` while inside an element whose contents are not prose.
    let mut opaque: Option<&str> = None;

    while !rest.is_empty() {
        let Some(next) = rest.find(['<', '&']) else {
            run.push_str(rest);
            break;
        };
        run.push_str(&rest[..next]);
        rest = &rest[next..];

        if rest.starts_with('&') {
            // An entity is one indivisible character as far as the reader is
            // concerned, and splitting one would corrupt it. It also ends the
            // token run, which is right: `&amp;` is punctuation.
            let end = rest
                .find(';')
                .filter(|end| *end <= 12)
                .map(|end| end + 1)
                .unwrap_or(1);
            flush(&mut out, &mut run, terms, opaque.is_none());
            out.push_str(&rest[..end]);
            rest = &rest[end..];
            continue;
        }

        // A tag. Copy it through untouched, and note whether it opens or
        // closes something whose contents must be left alone.
        let end = rest.find('>').map(|end| end + 1).unwrap_or(rest.len());
        let tag = &rest[..end];
        flush(&mut out, &mut run, terms, opaque.is_none());
        out.push_str(tag);
        rest = &rest[end..];

        let name = tag_name(tag);
        match opaque {
            Some(open) if tag.starts_with("</") && name == Some(open) => opaque = None,
            None if !tag.starts_with("</") => {
                if let Some(name) = name.filter(|name| OPAQUE_TAGS.contains(name)) {
                    opaque = Some(name);
                }
            }
            _ => {}
        }
    }
    flush(&mut out, &mut run, terms, opaque.is_none());
    out
}

/// Empties `run` into `out`, marking the matches if this run is prose.
fn flush(out: &mut String, run: &mut String, terms: &[String], prose: bool) {
    if run.is_empty() {
        return;
    }
    if !prose {
        out.push_str(run);
        run.clear();
        return;
    }
    let highlighted = postio_search::highlight::highlight(run, terms);
    for (piece, matched) in highlighted.runs() {
        if matched {
            out.push_str("<mark class=\"");
            out.push_str(MARK_CLASS);
            out.push_str("\">");
            out.push_str(piece);
            out.push_str("</mark>");
        } else {
            out.push_str(piece);
        }
    }
    run.clear();
}

/// The lower-cased element name of a tag, opening or closing.
fn tag_name(tag: &str) -> Option<&str> {
    let body = tag
        .trim_start_matches('<')
        .trim_start_matches('/')
        .trim_end_matches('>')
        .trim_end_matches('/');
    let name = body.split([' ', '\t', '\n', '\r']).next()?;
    (!name.is_empty() && name.chars().all(|ch| ch.is_ascii_alphanumeric())).then_some(name)
}

/// `Highlighted` as Pango markup for a label, with the matches in bold.
///
/// Weight rather than the accent tint the body gets. A `GtkLabel` can only be
/// given a *literal* colour through Pango markup, and a literal colour is a
/// hard-coded value that would not follow the theme — the one thing
/// `/gtk-design` says never to do. Bold is also already this application's
/// label idiom for "this is why it matched": the finder marks its fuzzy hits
/// the same way, in the same face, one row above.
pub fn markup(highlighted: &postio_search::Highlighted) -> String {
    let mut out = String::with_capacity(highlighted.text.len());
    for (piece, matched) in highlighted.runs() {
        let escaped = glib::markup_escape_text(piece);
        if matched {
            out.push_str("<b>");
            out.push_str(&escaped);
            out.push_str("</b>");
        } else {
            out.push_str(&escaped);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed day, so a test that mentions a relative date is not a test of
    /// what day it is.
    fn parse(query: &str) -> ParsedQuery {
        postio_search::parse(
            query,
            chrono::NaiveDate::from_ymd_opt(2026, 3, 1).expect("a real date"),
        )
    }

    // -- chips ------------------------------------------------------------

    #[test]
    fn an_empty_query_has_no_chips() {
        assert!(chips(&parse("")).is_empty());
    }

    #[test]
    fn free_text_is_not_a_chip() {
        assert!(
            chips(&parse("quarterly report")).is_empty(),
            "the part the user is still editing stays plain"
        );
    }

    #[test]
    fn an_operator_becomes_a_chip_labelled_as_typed() {
        let drawn = chips(&parse("from:ada@example.com report"));

        assert_eq!(drawn.len(), 1);
        assert_eq!(drawn[0].label, "from:ada@example.com");
        assert_eq!(drawn[0].field, Field::From);
        assert!(drawn[0].complete);
        assert!(!drawn[0].negated);
    }

    #[test]
    fn a_half_typed_operator_is_still_drawn() {
        let drawn = chips(&parse("from:"));

        assert_eq!(drawn.len(), 1);
        assert!(
            !drawn[0].complete,
            "it tells the user the keyword was understood"
        );
    }

    #[test]
    fn a_negated_operator_says_so() {
        let drawn = chips(&parse("-is:unread"));

        assert_eq!(drawn.len(), 1);
        assert!(drawn[0].negated);
        assert_eq!(drawn[0].label, "-is:unread");
    }

    #[test]
    fn several_operators_keep_the_order_they_were_typed_in() {
        let drawn = chips(&parse("from:ada@example.com is:flagged has:attach"));
        let fields: Vec<Field> = drawn.iter().map(|chip| chip.field).collect();

        assert_eq!(fields, vec![Field::From, Field::Is, Field::Has]);
    }

    #[test]
    fn a_word_that_merely_contains_a_colon_is_not_an_operator() {
        assert!(
            chips(&parse("note:this")).is_empty(),
            "`note` is not an operator this build knows"
        );
    }

    // -- backspace --------------------------------------------------------

    #[test]
    fn backspace_in_free_text_is_ordinary() {
        let parsed = parse("report");

        assert_eq!(backspace(&parsed, 6), Backspace::Ordinary);
    }

    #[test]
    fn backspace_at_the_right_edge_of_a_chip_pops_it_whole() {
        let query = "from:ada@example.com report";
        let parsed = parse(query);
        let end = "from:ada@example.com".len();

        assert_eq!(
            backspace(&parsed, end),
            Backspace::PopChip {
                index: 0,
                query: "report".to_owned(),
                caret: 0
            }
        );
    }

    #[test]
    fn backspace_inside_a_chip_pops_it_whole_too() {
        let parsed = parse("is:flagged report");

        let Backspace::PopChip { query, .. } = backspace(&parsed, 4) else {
            panic!("expected a pop");
        };
        assert_eq!(query, "report", "not `is:lagged`");
    }

    #[test]
    fn backspace_at_the_left_edge_of_a_chip_is_ordinary() {
        let parsed = parse("report is:flagged");
        let start = "report ".len();

        assert_eq!(
            backspace(&parsed, start),
            Backspace::Ordinary,
            "the caret is before the chip, so there is a space to delete"
        );
    }

    #[test]
    fn popping_a_chip_from_the_middle_tidies_the_gap() {
        let query = "report is:flagged more";
        let parsed = parse(query);
        let end = "report is:flagged".len();

        assert_eq!(
            backspace(&parsed, end),
            Backspace::PopChip {
                index: 1,
                query: "report more".to_owned(),
                caret: "report".len()
            },
            "one space between the halves, and the caret where the chip was"
        );
    }

    #[test]
    fn popping_the_last_chip_leaves_what_came_before() {
        let query = "report is:flagged";
        let parsed = parse(query);

        assert_eq!(
            backspace(&parsed, query.len()),
            Backspace::PopChip {
                index: 1,
                query: "report".to_owned(),
                caret: "report".len()
            }
        );
    }

    #[test]
    fn free_text_is_never_popped_whole() {
        let parsed = parse("quarterly report");

        assert_eq!(
            backspace(&parsed, "quarterly".len()),
            Backspace::Ordinary,
            "a word the user typed is a word"
        );
    }

    #[test]
    fn a_half_typed_chip_pops_whole_as_well() {
        let parsed = parse("report from:");

        assert!(matches!(
            backspace(&parsed, "report from:".len()),
            Backspace::PopChip { .. }
        ));
    }

    #[test]
    fn backspace_past_the_end_of_the_query_is_ordinary() {
        let parsed = parse("from:ada@example.com");

        assert_eq!(backspace(&parsed, 999), Backspace::Ordinary);
    }

    #[test]
    fn chips_can_be_popped_one_after_another() {
        let mut query = "from:ada@example.com is:flagged report".to_owned();

        // The caret rests at the right edge of the last chip each time, which
        // is where it is just after typing one.
        loop {
            let parsed = parse(&query);
            let Some(last) = chips(&parsed).last().cloned() else {
                break;
            };
            let caret = parsed.tokens()[last.index].span.end;
            match backspace(&parsed, caret) {
                Backspace::PopChip { query: next, .. } => query = next,
                Backspace::Ordinary => panic!("a chip at its right edge must pop"),
            }
        }

        assert_eq!(query, "report", "the free text is what survives");
    }

    #[test]
    fn backspace_at_the_end_of_trailing_free_text_is_ordinary() {
        let query = "from:ada@example.com report";
        let parsed = parse(query);

        assert_eq!(
            backspace(&parsed, query.len()),
            Backspace::Ordinary,
            "the caret is in `report`, not in the chip before it"
        );
    }

    // -- painting the match -----------------------------------------------

    fn terms(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_string()).collect()
    }

    #[test]
    fn a_matched_word_is_wrapped_where_it_stands() {
        assert_eq!(
            mark_html("<p>the maildir index</p>", &terms(&["maildir"])),
            "<p>the <mark class=\"postio-match\">maildir</mark> index</p>"
        );
    }

    #[test]
    fn a_query_with_no_terms_leaves_the_markup_alone() {
        let html = "<p>the maildir index</p>";
        assert_eq!(mark_html(html, &[]), html);
    }

    #[test]
    fn a_term_inside_a_tag_is_not_a_word_on_the_page() {
        // `title` is an attribute here, and `p` an element name. Marking
        // either would produce markup, not a highlight.
        let html = r#"<p title="maildir">nothing</p>"#;
        assert_eq!(mark_html(html, &terms(&["maildir", "p"])), html);
    }

    #[test]
    fn a_match_never_crosses_a_tag_boundary() {
        let html = "<b>mail</b>dir";
        assert_eq!(
            mark_html(html, &terms(&["maildir"])),
            html,
            "FTS5 did not match across the tag either, so nothing here may"
        );
    }

    #[test]
    fn an_entity_survives_being_marked_around() {
        assert_eq!(
            mark_html("a &amp; maildir", &terms(&["maildir"])),
            "a &amp; <mark class=\"postio-match\">maildir</mark>"
        );
        assert_eq!(
            mark_html("a &amp; b", &terms(&["amp"])),
            "a &amp; b",
            "`&amp;` is one character, not the word `amp`"
        );
    }

    #[test]
    fn a_bare_ampersand_does_not_swallow_the_rest_of_the_body() {
        assert_eq!(
            mark_html("Tom & Jerry maildir", &terms(&["maildir"])),
            "Tom & Jerry <mark class=\"postio-match\">maildir</mark>"
        );
    }

    #[test]
    fn a_stylesheet_is_not_prose() {
        let html = "<style>.maildir { color: red }</style><p>maildir</p>";
        assert_eq!(
            mark_html(html, &terms(&["maildir"])),
            "<style>.maildir { color: red }</style><p><mark class=\"postio-match\">maildir</mark></p>",
            "marking inside a stylesheet would corrupt it"
        );
    }

    #[test]
    fn several_matches_across_several_elements_are_all_painted() {
        assert_eq!(
            mark_html("<p>maildir one</p><p>two maildir</p>", &terms(&["maildir"])),
            "<p><mark class=\"postio-match\">maildir</mark> one</p>\
             <p>two <mark class=\"postio-match\">maildir</mark></p>"
        );
    }

    #[test]
    fn markup_escapes_what_it_did_not_write() {
        let highlighted =
            postio_search::highlight::highlight("a <b> & maildir", &terms(&["maildir"]));
        assert_eq!(
            markup(&highlighted),
            "a &lt;b&gt; &amp; <b>maildir</b>",
            "the message's own angle brackets are text, not markup"
        );
    }

    #[test]
    fn markup_of_a_plain_string_is_the_plain_string() {
        let highlighted = postio_search::highlight::highlight("nothing here", &terms(&["maildir"]));
        assert_eq!(markup(&highlighted), "nothing here");
    }

    // -- the readout ------------------------------------------------------

    fn outcome(hits: u64, capped: bool, millis: u64) -> Outcome {
        Outcome {
            hits,
            capped,
            elapsed: Duration::from_millis(millis),
            // The end state, and the one every other test here is about:
            // ADR 0016 backfills to completion, so a settled account carries
            // no caveat.
            corpus_complete: true,
        }
    }

    #[test]
    fn the_readout_is_written_the_way_the_canvas_writes_it() {
        assert_eq!(readout(&outcome(14, false, 11)), "14 hits · 11 ms");
    }

    // -- the corpus caveat (#352) ----------------------------------------

    #[test]
    fn a_corpus_still_filling_says_so_once_beside_the_count() {
        let filling = Outcome {
            corpus_complete: false,
            ..outcome(14, false, 11)
        };
        assert_eq!(readout(&filling), "14 hits · 11 ms · still syncing");
    }

    #[test]
    fn the_settled_slot_still_fits_the_longest_thing_without_the_caveat() {
        // The common case keeps the width it has always had: a settled
        // account must not pay for a caveat it does not carry.
        let longest = outcome(10_000, true, 90);
        assert_eq!(readout(&longest), "10000+ hits · 90 ms");
        assert!(readout(&longest).chars().count() <= READOUT_CHARS as usize);
    }

    #[test]
    fn a_complete_corpus_carries_no_caveat_at_all() {
        // The acceptance criterion that stops this becoming permanent
        // furniture: under ADR 0016 every account converges here, so this is
        // the state the readout spends most of its life in.
        assert_eq!(readout(&outcome(14, false, 11)), "14 hits · 11 ms");
        assert!(!readout(&outcome(0, false, 3)).contains("syncing"));
    }

    #[test]
    fn the_caveat_composes_with_a_capped_count() {
        // Two different reasons the number is a floor, and they are allowed to
        // be true at once: a term common enough to cap the count, asked of a
        // corpus that is not all here.
        let both = Outcome {
            corpus_complete: false,
            ..outcome(10_000, true, 90)
        };
        assert_eq!(readout(&both), "10000+ hits · 90 ms · still syncing");
        assert!(
            readout(&both).chars().count() <= READOUT_CHARS_SYNCING as usize,
            "the longest thing the readout can say must fit the slot it \
             reserves, or the field twitches when the caveat goes away: {:?}",
            readout(&both)
        );
    }

    #[test]
    fn the_spoken_form_explains_what_three_words_only_flag() {
        let filling = Outcome {
            corpus_complete: false,
            ..outcome(14, false, 11)
        };
        let spoken = spoken_readout(&filling);
        assert!(spoken.starts_with("14 hits, in 11 milliseconds"));
        assert!(
            spoken.contains("still syncing"),
            "somebody who cannot see the window gets the flag and no way to \
             find out what it means: {spoken:?}"
        );
        assert!(
            !spoken_readout(&outcome(14, false, 11)).contains("syncing"),
            "and a settled account is not told about a state it is not in"
        );
    }

    #[test]
    fn one_hit_is_not_one_hits() {
        assert_eq!(readout(&outcome(1, false, 3)), "1 hit · 3 ms");
    }

    #[test]
    fn nothing_found_says_so_rather_than_showing_a_zero() {
        assert_eq!(readout(&outcome(0, false, 4)), "no hits · 4 ms");
    }

    #[test]
    fn a_capped_count_never_passes_for_a_total() {
        assert_eq!(readout(&outcome(10_000, true, 91)), "10000+ hits · 91 ms");
    }

    #[test]
    fn a_search_too_fast_to_measure_still_took_some_time() {
        assert_eq!(
            readout(&outcome(3, false, 0)),
            "3 hits · <1 ms",
            "`0 ms` would read as a search that did not run"
        );
    }

    #[test]
    fn a_search_slow_enough_to_notice_changes_unit() {
        let slow = Outcome {
            hits: 2,
            capped: false,
            elapsed: Duration::from_millis(12_400),
            corpus_complete: true,
        };
        assert_eq!(readout(&slow), "2 hits · 12.4 s");
    }

    #[test]
    fn the_readout_reads_aloud_as_words() {
        assert_eq!(
            spoken_readout(&outcome(14, false, 11)),
            "14 hits, in 11 milliseconds"
        );
        assert_eq!(
            spoken_readout(&outcome(1, false, 1)),
            "1 hit, in 1 millisecond"
        );
        assert_eq!(
            spoken_readout(&outcome(0, false, 0)),
            "no hits, in under a millisecond"
        );
    }

    #[test]
    fn an_outcome_is_read_straight_off_a_finished_search() {
        let results = postio_search::SearchResults {
            hits: Vec::new(),
            total_hits: 14,
            total_hits_capped: false,
            elapsed: Duration::from_millis(11),
            corpus_complete: true,
        };
        assert_eq!(Outcome::of(&results), outcome(14, false, 11));

        // And the caveat is carried across rather than re-derived: the
        // executor is what knows the scope it searched (#352).
        let filling = postio_search::SearchResults {
            corpus_complete: false,
            ..results
        };
        assert!(!Outcome::of(&filling).corpus_complete);
    }

    // -- pacing -----------------------------------------------------------

    #[test]
    fn nothing_is_accepted_before_anything_is_asked() {
        let pacer = Pacer::default();
        assert!(!pacer.accepts(0));
        assert!(!pacer.accepts(1));
    }

    #[test]
    fn the_answer_to_the_current_question_is_accepted() {
        let mut pacer = Pacer::default();
        let first = pacer.issue();
        assert!(pacer.accepts(first));
    }

    #[test]
    fn a_superseded_query_is_dropped_rather_than_drawn() {
        let mut pacer = Pacer::default();
        let first = pacer.issue();
        let second = pacer.issue();

        assert!(
            !pacer.accepts(first),
            "the answer to what was typed two keystrokes ago is not an answer"
        );
        assert!(pacer.accepts(second));
    }

    #[test]
    fn answers_that_come_back_out_of_order_do_not_move_the_readout_backwards() {
        let mut pacer = Pacer::default();
        let first = pacer.issue();
        let second = pacer.issue();

        // The newest answers first, then the older one straggles in.
        assert!(pacer.accepts(second));
        assert!(!pacer.accepts(first));
    }

    #[test]
    fn abandoning_leaves_nothing_that_could_still_be_accepted() {
        let mut pacer = Pacer::default();
        let asked = pacer.issue();
        pacer.abandon();

        assert!(!pacer.accepts(asked));
    }

    // -- spoken -----------------------------------------------------------

    #[test]
    fn a_chip_reads_as_what_it_does() {
        let drawn = chips(&parse("from:ada@example.com"));
        assert_eq!(spoken(&drawn[0]), "from ada@example.com");

        let drawn = chips(&parse("-is:unread"));
        assert_eq!(spoken(&drawn[0]), "not is unread");

        let drawn = chips(&parse("subject:"));
        assert_eq!(spoken(&drawn[0]), "subject, no value yet");
    }
}
