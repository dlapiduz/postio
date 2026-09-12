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

/// The term to offer instead of `typed`, or `None` when nothing is close
/// enough to be worth offering.
///
/// Closest first; between two equally close terms, the one more of the
/// mailbox holds — a term in four hundred messages is likelier to be the word
/// than one in a single message that misspelled it the other way.
pub fn suggest<'a>(typed: &str, vocabulary: impl Iterator<Item = Term<'a>>) -> Option<Suggestion> {
    let typed = typed.to_lowercase();
    let limit = tolerance(&typed);
    if limit == 0 {
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
        let Some(distance) = distance_within(&typed, &term.text.to_lowercase(), limit) else {
            continue;
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
#[cfg(test)]
mod tests {
    use super::*;

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
    fn case_is_not_a_misspelling() {
        let offered = suggest("Hanah", vocabulary().into_iter()).expect("a suggestion");
        assert_eq!(offered.term, "hannah");
    }
}
