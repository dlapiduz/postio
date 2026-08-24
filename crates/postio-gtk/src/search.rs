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

use gtk::glib;
use gtk::prelude::*;
use postio_search::ParsedQuery;
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
/// Short enough to read as instant — well under the ~100 ms at which a delay
/// stops feeling like part of the keystroke — and long enough that a burst of
/// typing costs one search rather than one per character. The debounce is
/// what keeps typing inside the 16 ms interaction budget: the keystroke never
/// waits for a search, it only ever reschedules one.
pub const DEBOUNCE: Duration = Duration::from_millis(60);

/// The slot the readout reserves, in characters.
///
/// Wide enough for the longest thing it can say — `10000+ hits · 100 ms` —
/// so the number never has to be truncated and the slot never has to resize.
const READOUT_CHARS: i32 = 20;

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
}

impl Outcome {
    /// Reads the outcome off a finished search.
    pub fn of(results: &postio_search::SearchResults) -> Self {
        Outcome {
            hits: results.total_hits,
            capped: results.total_hits_capped,
            elapsed: results.elapsed,
        }
    }
}

/// The readout, as the canvas writes it: `14 hits · 11 ms`.
///
/// No thousands separators, because the canvas' own scope counts are written
/// `4291` and two number formats in one column would read as two kinds of
/// number. A capped count is written `10000+ hits` rather than a bare figure,
/// so a floor never passes for a total.
pub fn readout(outcome: &Outcome) -> String {
    format!("{} · {}", hits(outcome), elapsed(outcome.elapsed))
}

/// The readout as a screen reader should hear it — the same facts, in words,
/// because "·" and "ms" are punctuation and an abbreviation rather than
/// something to read aloud.
pub fn spoken_readout(outcome: &Outcome) -> String {
    let elapsed = outcome.elapsed.as_millis();
    match elapsed {
        0 => format!("{}, in under a millisecond", hits(outcome)),
        1 => format!("{}, in 1 millisecond", hits(outcome)),
        _ => format!("{}, in {elapsed} milliseconds", hits(outcome)),
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

    /// Runs the queued query now instead of waiting out the debounce.
    ///
    /// What `Enter` does, and what a test does instead of sleeping.
    pub fn flush(&self) {
        self.cancel_pending();
        let Some(query) = self.inner.queued.borrow_mut().take() else {
            return;
        };
        let sequence = self.inner.pacer.borrow_mut().issue();
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
        if !self.inner.pacer.borrow().accepts(sequence) {
            return false;
        }
        self.inner.outcome.set(Some(outcome));
        self.render();
        true
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

    // -- the readout ------------------------------------------------------

    fn outcome(hits: u64, capped: bool, millis: u64) -> Outcome {
        Outcome {
            hits,
            capped,
            elapsed: Duration::from_millis(millis),
        }
    }

    #[test]
    fn the_readout_is_written_the_way_the_canvas_writes_it() {
        assert_eq!(readout(&outcome(14, false, 11)), "14 hits · 11 ms");
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
        };
        assert_eq!(Outcome::of(&results), outcome(14, false, 11));
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
