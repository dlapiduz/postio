//! Reading a search query as chips, and saying what a search turned out to be.
//!
//! Canvas 2b: `/` opens a bar, parsed operators become chips, and the
//! right-hand end of the field says how many hits and how long they took.
//! None of that is a toolkit's business — it is a reading of a
//! [`postio_search::ParsedQuery`] and a sentence about a result set.
//!
//! It lived in `postio-gtk::search` until #1157, where the macOS bar could
//! not reach any of it: the chips, the Backspace rule, the readout wording,
//! its screen-reader form, and the debounce pacing. A second frontend
//! re-deriving those would be a second query vocabulary on screen, a second
//! answer to what "still syncing" means, and a second debounce — and the
//! chips in particular are how a user *learns* Postio's query language, so
//! two of them is two languages.
//!
//! # Where the chips live
//!
//! The entry holds the *whole* query, and the chips are a parse of it drawn
//! alongside. They are a reading of what is typed, not a second store that
//! could disagree with it — which is why [`postio_search::ParsedQuery`] hands
//! out spans into the input, and why `remove_token` returns *the string to
//! put back in the entry*.
//!
//! The alternative — lifting completed operators out of the entry into
//! standalone chips — is a nicer picture and a worse editor: the caret can no
//! longer move through the query, and every edit becomes a merge between two
//! representations. This way the entry is the truth and the chips follow it.

use std::time::Duration;

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

/// What one search turned out to be.
///
/// The three numbers canvas 2b puts at the right-hand end of the field, and
/// the same three [`postio_search::SearchResults`] carries — this is that,
/// minus the hits themselves, because the readout does not need them and
/// copying a page of results to draw a number would be silly.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
    /// filling, so the count is a floor for a second reason (#352). Also
    /// `false` while an account in scope is rebuilding its local search
    /// index (#981, `postio_session::reindex_account`) — a message can drop
    /// out of results mid-rebuild the same way one that has not backfilled
    /// yet does, and it is the same honest caveat either way.
    pub corpus_complete: bool,
    /// Accounts a unified search could not reach, by the name the sidebar
    /// shows, in the sidebar's order.
    ///
    /// ADR 0005 Q10: *a view that cannot include an account says so, names
    /// the account, and stays usable.* Empty for a single-account search,
    /// which leaves nothing out, and empty for a unified search whose
    /// accounts all answered — the ordinary case, and the one this must not
    /// become furniture in.
    ///
    /// Names rather than ids because the only thing that ever reads it is a
    /// sentence a person reads, and a `Vec` rather than a `bool` because Q10
    /// asks for the account to be named: "some accounts are missing" is the
    /// disclosure people learn to ignore, since it never says which one to go
    /// and fix.
    ///
    /// **Not from [`postio_search::SearchResults`].** Which accounts answered
    /// is a fact about connections, and the executor only ever sees the
    /// store — a search of a store whose account is offline reads exactly
    /// like one whose account is fine. The composition root joins the two.
    pub unreachable: Vec<String>,
}

impl Outcome {
    /// Reads the outcome off a finished search.
    pub fn of(results: &postio_search::SearchResults) -> Self {
        Outcome {
            hits: results.total_hits,
            capped: results.total_hits_capped,
            elapsed: results.elapsed,
            corpus_complete: results.corpus_complete,
            // Filled by the caller: see the field.
            unreachable: Vec::new(),
        }
    }

    /// The same outcome, carrying the accounts a search could not reach.
    pub fn with_unreachable(mut self, unreachable: Vec<String>) -> Self {
        self.unreachable = unreachable;
        self
    }

    /// The same outcome, with the corpus caveat also raised when an account
    /// in scope is mid-rebuild (#981).
    ///
    /// Only ever turns `corpus_complete` off, never back on: the executor's
    /// own answer already accounts for backfill, and a rebuild finishing is
    /// not proof a backfill did too.
    pub fn with_reindexing(mut self, reindexing: bool) -> Self {
        self.corpus_complete &= !reindexing;
        self
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
    let mut line = format!("{} · {}", hits(outcome), elapsed(outcome.elapsed));
    if !outcome.corpus_complete {
        line.push_str(" · still syncing");
    }
    // Both, when both are true. They are different facts with different
    // fixes -- one ends on its own under ADR 0016, the other needs the
    // account to come back -- so neither may hide the other.
    match outcome.unreachable.as_slice() {
        [] => {}
        // One name fits and is worth more than a count: it says which
        // account to go and look at.
        [only] => line.push_str(&format!(" · {only} unreachable")),
        // Past one it does not fit, and a fixed slot is what keeps the field
        // from breathing per keystroke. The count still says there is more
        // than one to fix; `spoken_readout` carries the names.
        many => line.push_str(&format!(" · {} unreachable", many.len())),
    }
    line
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
    let mut spoken = counted;
    if !outcome.corpus_complete {
        spoken.push_str(
            ". This account is still syncing, so messages whose text has not \
             arrived yet could not be searched.",
        );
    }
    // Every name, which is the whole reason the spoken form exists: the
    // visible caveat has room to flag the state and, past one account, not to
    // say which ones.
    if !outcome.unreachable.is_empty() {
        spoken.push_str(&format!(
            ". {} could not be searched, so this answer may be short.",
            and_list(&outcome.unreachable)
        ));
    }
    spoken
}

/// `a`, `a and b`, `a, b and c` — a list as somebody reads it aloud.
fn and_list(items: &[String]) -> String {
    match items.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, head)) => format!("{} and {last}", head.join(", ")),
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

