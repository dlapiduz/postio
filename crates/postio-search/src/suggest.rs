//! What to search for instead, when what was typed found nothing.
//!
//! # Why this is not a looser match
//!
//! ADR 0037. A query string here is not only a way to draw a list: a saved
//! search is a query with a name, and *a rule is a saved search plus actions*
//! (`docs/PRODUCT.md`). Widening the match so `hanah` finds `hannah` would
//! therefore make rules archive, label and move mail on terms that are only
//! approximately present, silently. A search that finds nothing is visible in
//! the same second; a rule that fired on a near-miss is not.
//!
//! So the query keeps meaning exactly what it says, and a near-miss is
//! answered by offering *a different query* — one the user could have typed,
//! and accepts deliberately.
//!
//! # Why it lives in this crate
//!
//! Ranking a candidate is arithmetic over two strings and a count. It needs
//! no database, so it is testable without one, and this crate is the leaf
//! that may not have `rusqlite` anyway. Reading the vocabulary is
//! `postio-index`'s half.

/// A term the index holds, and how many documents hold it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Term<'a> {
    /// The term as the index tokenized it.
    pub text: &'a str,
    /// How many documents contain it. A tie-breaker, and a guard against
    /// offering a term that one message once misspelled.
    pub documents: u64,
}

/// What to offer instead of a term that matched nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// The term to search for instead.
    pub term: String,
    /// How many documents hold it, so the offer can say what it would find.
    pub documents: u64,
}

/// The most a term may be mistyped and still be recognisable.
///
/// One edit for a short word and two for a long one, rather than a flat
/// number: at distance two, `cat` reaches `dog`'s neighbourhood and every
/// three-letter word is a candidate for every other. Length is what makes an
/// edit meaningful.
fn tolerance(typed: &str) -> usize {
    match typed.chars().count() {
        0..=3 => 0,
        4..=7 => 1,
        _ => 2,
    }
}

/// Damerau-Levenshtein distance, capped: returns `None` once it is certain
/// the distance exceeds `limit`.
///
/// Transpositions count as one edit rather than two, because swapping two
/// letters is among the commonest ways to mistype a name and `hnanah` is one
/// slip rather than two.
fn distance_within(a: &str, b: &str, limit: usize) -> Option<usize> {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    // A length gap wider than the limit cannot be closed by edits.
    if a.len().abs_diff(b.len()) > limit {
        return None;
    }

    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut before_previous: Vec<usize> = vec![0; b.len() + 1];
    let mut current: Vec<usize> = vec![0; b.len() + 1];

    for i in 1..=a.len() {
        current[0] = i;
        let mut best_in_row = current[0];
        for j in 1..=b.len() {
            let substitution = usize::from(a[i - 1] != b[j - 1]);
            let mut value = (previous[j] + 1)
                .min(current[j - 1] + 1)
                .min(previous[j - 1] + substitution);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                value = value.min(before_previous[j - 2] + 1);
            }
            current[j] = value;
            best_in_row = best_in_row.min(value);
        }
        // Every distance from here on is at least this row's best.
        if best_in_row > limit {
            return None;
        }
        std::mem::swap(&mut before_previous, &mut previous);
        std::mem::swap(&mut previous, &mut current);
    }

    let distance = previous[b.len()];
    (distance <= limit).then_some(distance)
}

/// The fewest letters that are a beginning worth completing.
///
/// Three, the same floor below which [`tolerance`] corrects nothing: two
/// letters begin a quarter of the vocabulary, and the offer would be noise.
const SHORTEST_BEGINNING: usize = 3;

/// The query that asks the index for every term [`suggest`] could offer for
/// `typed`, or `None` when nothing could be offered.
///
/// The index expands it against its own term dictionary, so the offer is
/// drawn from every word the mailbox holds rather than a sample of them.
/// `typed~N` reaches terms that *begin* within `N` edits of the word -- the
/// word mistyped, left unfinished, or both -- and `typed*` only the ones
/// that begin with it, for a word too short to correct. `N` is
/// `tolerance`, so the index widens exactly as far as the ranking will
/// accept and no further.
///
/// Unquoted, which is the point: the index reads a quoted term exactly and
/// an unquoted one as this. Only ever a single word of letters and digits,
/// so nothing typed reaches the index as syntax.
pub fn widened(typed: &str) -> Option<String> {
    let typed = typed.to_lowercase();
    if typed.chars().count() < SHORTEST_BEGINNING || !typed.chars().all(char::is_alphanumeric) {
        return None;
    }
    Some(match tolerance(&typed) {
        0 => format!("{typed}*"),
        edits => format!("{typed}~{edits}"),
    })
}

/// The term to offer instead of `typed`, or `None` when nothing is close
/// enough to be worth offering.
///
/// Two ways to be close. A term that **begins** with what was typed is the
/// word left unfinished -- whole-word matching finds nothing for half a word
/// -- and it outranks every correction, because nothing was mistyped. Past
/// that, a term within `tolerance` edits is the word mistyped, closest
/// first.
///
/// Between two equally close terms, the one more of the mailbox holds — a
/// term in four hundred messages is likelier to be the word than one in a
/// single message that misspelled it the other way.
pub fn suggest<'a>(typed: &str, vocabulary: impl Iterator<Item = Term<'a>>) -> Option<Suggestion> {
    let typed = typed.to_lowercase();
    let limit = tolerance(&typed);
    let completes = typed.chars().count() >= SHORTEST_BEGINNING;
    if limit == 0 && !completes {
        return None;
    }

    let mut best: Option<(usize, Term<'a>)> = None;
    for term in vocabulary {
        // A term that *is* what was typed is not a suggestion. The query
        // found nothing for some other reason -- a second term, a scope --
        // and offering the same words back is a shrug with extra steps.
        if term.text.eq_ignore_ascii_case(&typed) {
            return None;
        }
        let text = term.text.to_lowercase();
        // A completion ranks as distance zero: ahead of any correction.
        let distance = if completes && text.starts_with(&typed) {
            0
        } else if limit == 0 {
            continue;
        } else {
            match distance_within(&typed, &text, limit) {
                Some(distance) => distance,
                None => continue,
            }
        };
        let better = match &best {
            None => true,
            // Closer wins; between two equally close, the commoner one.
            // Not a tuple comparison: that orders `documents` *ascending* on a
            // tie, which is precisely backwards and offered `hanna` over
            // `hannah` because two messages hold it and sixty-six hold the
            // other.
            Some((best_distance, best_term)) => {
                distance < *best_distance
                    || (distance == *best_distance && term.documents > best_term.documents)
            }
        };
        if better {
            best = Some((distance, term));
        }
    }

    best.map(|(_, term)| Suggestion {
        term: term.text.to_owned(),
        documents: term.documents,
    })
}
/// The words a forgiving search reads `typed` as: the word itself when the
/// vocabulary holds it, then the words it begins, then the words it is a
/// misspelling of -- the same two kinds of closeness [`suggest`] ranks, and
/// in the same order -- at most `most` of them.
///
/// This is the one place a query is widened rather than answered with an
/// offer, and only for the command bar's interactive search: ADR 0037, as
/// amended. A rule's query never comes here.
///
/// A word the vocabulary holds is not corrected: spelled right, `tour` meant
/// tour, and `your`, one edit away and in every other message, would drown
/// it. Only the words it begins -- `tours` -- are near it then.
pub fn near<'a>(
    typed: &str,
    vocabulary: impl Iterator<Item = Term<'a>>,
    most: usize,
) -> Vec<String> {
    let typed = typed.to_lowercase();
    let vocabulary: Vec<Term<'a>> = vocabulary.collect();
    let held = vocabulary
        .iter()
        .any(|term| term.text.to_lowercase() == typed);
    let limit = if held { 0 } else { tolerance(&typed) };
    let completes = typed.chars().count() >= SHORTEST_BEGINNING;

    // Ranked: the word itself, then a completion, then a correction by how
    // many edits it is away; between equals, the commoner word.
    let mut found: Vec<(usize, u64, String)> = Vec::new();
    for term in vocabulary {
        let text = term.text.to_lowercase();
        let rank = if text == typed {
            0
        } else if completes && text.starts_with(&typed) {
            1
        } else if limit == 0 {
            continue;
        } else {
            match distance_within(&typed, &text, limit) {
                Some(distance) => 1 + distance,
                None => continue,
            }
        };
        found.push((rank, term.documents, text));
    }
    found.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
    found.dedup_by(|a, b| a.2 == b.2);
    found
        .into_iter()
        .take(most)
        .map(|(_, _, text)| text)
        .collect()
}

/// What the dropdown offers for a prefix (spec 010, step 8).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Suggestions {
    /// The rest of the best word, drawn after the caret: "las" after "at".
    pub ghost: Option<String>,
    /// Vocabulary words.
    pub words: Vec<Completion>,
    /// Labels.
    pub labels: Vec<Completion>,
    /// Mailing lists.
    pub lists: Vec<Completion>,
    /// File names.
    pub files: Vec<Completion>,
    /// Folders, for `in:`.
    pub folders: Vec<Completion>,
    /// People.
    pub people: Vec<Person>,
}

/// One completion, and how much it would find.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// What the row says.
    pub text: String,
    /// The query it runs.
    pub query: String,
    /// What that query counts.
    pub count: u64,
}

/// A person `from:` and `to:` complete to, ranked two-way (D21).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    /// Their name, when their mail gives one.
    pub name: Option<String>,
    /// Their address.
    pub address: String,
    /// Messages from them.
    pub received: u64,
    /// Messages to them.
    pub sent: u64,
    /// The newest message either way.
    pub last: Option<chrono::DateTime<chrono::Utc>>,
}

/// The words `vocabulary` holds that complete `prefix`, best first: the
/// ones more of the mailbox holds, then the shorter, then alphabetically.
/// The prefix itself is not a completion -- it adds nothing to what was
/// typed -- and neither is a word that only matches it ignoring case in a
/// way that changes its length. Each comes back as its own query, counted
/// in documents; the executor counts the query itself before it is shown.
pub fn rank_words<'a>(prefix: &str, vocabulary: impl Iterator<Item = Term<'a>>) -> Vec<Completion> {
    let prefix = prefix.to_lowercase();
    if prefix.is_empty() {
        return Vec::new();
    }
    let mut found: Vec<(u64, String)> = Vec::new();
    for term in vocabulary {
        let text = term.text.to_lowercase();
        if text.len() <= prefix.len() || !text.starts_with(&prefix) {
            continue;
        }
        match found.iter_mut().find(|(_, held)| *held == text) {
            Some(held) => held.0 = held.0.max(term.documents),
            None => found.push((term.documents, text)),
        }
    }
    found.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(a.1.chars().count().cmp(&b.1.chars().count()))
            .then(a.1.cmp(&b.1))
    });
    found
        .into_iter()
        .map(|(documents, text)| Completion {
            query: text.clone(),
            text,
            count: documents,
        })
        .collect()
}

/// What to draw after the caret for `typed`, when `word` completes it: the
/// rest of the word, in its own letters. `None` when it does not begin so.
pub fn ghost(typed: &str, word: &str) -> Option<String> {
    let typed_chars = typed.chars().count();
    let lowered: String = word.chars().take(typed_chars).collect();
    (lowered.to_lowercase() == typed.to_lowercase() && word.chars().count() > typed_chars)
        .then(|| word.chars().skip(typed_chars).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(texts: &[(&'static str, u64)]) -> Vec<Term<'static>> {
        texts
            .iter()
            .map(|(text, documents)| Term {
                text,
                documents: *documents,
            })
            .collect()
    }

    #[test]
    fn rank_words_offers_the_commonest_completion_first() {
        let ranked = rank_words(
            "at",
            words(&[
                ("attic", 2),
                ("atlas", 9),
                ("at", 40),
                ("budget", 30),
                ("Atlantic", 2),
                ("atlas", 3),
            ])
            .into_iter(),
        );
        let texts: Vec<(&str, u64)> = ranked
            .iter()
            .map(|word| (word.text.as_str(), word.count))
            .collect();
        assert_eq!(
            texts,
            [("atlas", 9), ("attic", 2), ("atlantic", 2)],
            "most documents first, then shorter; the prefix itself and \
             words it does not begin are not completions; a word seen twice \
             keeps its larger count"
        );
        assert_eq!(ranked[0].query, "atlas", "a word runs as itself");
        assert!(rank_words("", words(&[("atlas", 1)]).into_iter()).is_empty());
    }

    #[test]
    fn the_ghost_is_the_rest_of_the_word_whatever_the_case_typed() {
        assert_eq!(ghost("at", "atlas").as_deref(), Some("las"));
        assert_eq!(ghost("At", "atlas").as_deref(), Some("las"));
        assert_eq!(ghost("atlas", "atlas"), None, "nothing left to add");
        assert_eq!(ghost("bu", "atlas"), None);
        assert_eq!(ghost("caf", "caf\u{e9}s").as_deref(), Some("\u{e9}s"));
    }

    fn vocabulary() -> Vec<Term<'static>> {
        vec![
            Term {
                text: "hannah",
                documents: 66,
            },
            Term {
                text: "hanna",
                documents: 2,
            },
            Term {
                text: "banana",
                documents: 11,
            },
            Term {
                text: "quarterly",
                documents: 400,
            },
            Term {
                text: "cat",
                documents: 9,
            },
            Term {
                text: "dog",
                documents: 9,
            },
        ]
    }

    #[test]
    fn a_dropped_letter_is_offered_the_word_it_dropped_it_from() {
        let offered = suggest("hanah", vocabulary().into_iter()).expect("a suggestion");
        assert_eq!(offered.term, "hannah");
        assert_eq!(
            offered.documents, 66,
            "the offer says what it would find, so it is worth accepting"
        );
    }

    #[test]
    fn a_transposition_counts_as_one_slip_not_two() {
        // Swapping two letters is among the commonest ways to mistype a name.
        // At two edits `hnanah` would still be reachable, but only because the
        // word is long; the point is that it costs one.
        let offered = suggest("hnanah", vocabulary().into_iter()).expect("a suggestion");
        assert_eq!(offered.term, "hannah");
    }

    #[test]
    fn the_closer_term_wins_and_then_the_commoner_one() {
        // `hanna` is one edit from `hannah` and one from `hanah`, so both are
        // candidates for `hannh`; the one more of the mailbox holds is the
        // likelier word.
        let offered = suggest("hannh", vocabulary().into_iter()).expect("a suggestion");
        assert_eq!(
            offered.term, "hannah",
            "between two equally close terms, the one in more messages"
        );
    }

    #[test]
    fn a_short_word_is_never_corrected() {
        // At one edit `cat` reaches `dog`'s neighbourhood and every
        // three-letter word is a candidate for every other.
        assert_eq!(suggest("cot", vocabulary().into_iter()), None);
        assert_eq!(suggest("dot", vocabulary().into_iter()), None);
    }

    #[test]
    fn a_term_the_index_holds_is_never_offered_back() {
        // The query found nothing for some other reason -- a second term, a
        // scope -- and offering the same words back is a shrug with steps.
        assert_eq!(suggest("hannah", vocabulary().into_iter()), None);
    }

    #[test]
    fn a_word_nothing_resembles_is_left_alone() {
        assert_eq!(suggest("xylophone", vocabulary().into_iter()), None);
    }

    #[test]
    fn an_unfinished_word_is_offered_the_word_it_begins() {
        // Nobody misspelled anything: the word was simply not finished, and
        // whole-word matching finds nothing for half a word.
        let offered = suggest("quart", vocabulary().into_iter()).expect("a completion");
        assert_eq!(offered.term, "quarterly");
    }

    #[test]
    fn a_short_start_is_still_completed_though_never_corrected() {
        // Three letters are too few to correct -- every such word is one edit
        // from another -- but they are a real beginning.
        let offered = suggest("ban", vocabulary().into_iter()).expect("a completion");
        assert_eq!(offered.term, "banana");
    }

    #[test]
    fn two_letters_are_not_a_beginning_worth_completing() {
        assert_eq!(suggest("ha", vocabulary().into_iter()), None);
    }

    #[test]
    fn among_completions_the_commoner_word_wins() {
        let offered = suggest("han", vocabulary().into_iter()).expect("a completion");
        assert_eq!(offered.term, "hannah", "66 messages over 2");
    }

    #[test]
    fn a_long_word_is_widened_as_far_as_it_may_be_mistyped() {
        assert_eq!(widened("reimbursment").as_deref(), Some("reimbursment~2"));
        assert_eq!(widened("hanah").as_deref(), Some("hanah~1"));
    }

    #[test]
    fn a_short_word_is_widened_only_to_what_begins_with_it() {
        // Three letters are too few to correct, but a real beginning.
        assert_eq!(widened("Ban").as_deref(), Some("ban*"));
    }

    #[test]
    fn nothing_widens_two_letters_or_anything_that_is_not_one_word() {
        assert_eq!(widened("ha"), None);
        // Punctuation would be read as query syntax, or split into two
        // words the offer could not name as one.
        assert_eq!(widened("ada@example"), None);
        assert_eq!(widened("hanah~"), None);
        assert_eq!(widened("\"hanah\""), None);
    }

    #[test]
    fn case_is_not_a_misspelling() {
        let offered = suggest("Hanah", vocabulary().into_iter()).expect("a suggestion");
        assert_eq!(offered.term, "hannah");
    }

    #[test]
    fn near_words_are_the_word_itself_then_its_completions_then_its_misspellings() {
        let words = near("hanna", vocabulary().into_iter(), 8);
        assert_eq!(
            words,
            ["hanna", "hannah"],
            "the word as typed first, then what it begins, the commoner first"
        );
        let words = near("hannh", vocabulary().into_iter(), 8);
        assert_eq!(
            words,
            ["hannah", "hanna"],
            "equally close: the commoner first"
        );
    }

    #[test]
    fn near_words_are_capped_at_the_most_asked_for() {
        let vocabulary: Vec<Term<'static>> = ["ticket", "tickets", "ticketed", "ticketing"]
            .into_iter()
            .map(|text| Term { text, documents: 1 })
            .collect();
        assert_eq!(near("ticket", vocabulary.into_iter(), 2).len(), 2);
    }

    #[test]
    fn a_short_word_is_near_only_what_begins_with_it() {
        assert_eq!(
            near("cot", vocabulary().into_iter(), 8),
            Vec::<String>::new()
        );
        assert_eq!(near("ban", vocabulary().into_iter(), 8), ["banana"]);
    }

    #[test]
    fn a_word_the_vocabulary_holds_is_near_only_what_it_begins() {
        // "tour" is spelled right, so "your" -- one edit away and in every
        // other message -- is not what was meant; "tours" may be.
        let vocabulary: Vec<Term<'static>> = [("tour", 3), ("tours", 2), ("your", 900)]
            .into_iter()
            .map(|(text, documents)| Term { text, documents })
            .collect();
        assert_eq!(near("tour", vocabulary.into_iter(), 8), ["tour", "tours"]);
    }
}
