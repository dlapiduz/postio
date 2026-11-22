//! What to search for instead when what was typed found nothing (ADR 0037). The query keeps meaning
//! exactly what it says, because a saved search and a rule are queries too and a widened match
//! would act on mail only approximately present; a near-miss is answered by offering a different
//! query the user accepts deliberately.
//! Pure arithmetic over strings and a count, so it needs no database; reading the vocabulary is
//! `postio-index`'s half.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Term<'a> {
    pub text: &'a str,
    pub documents: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub term: String,
    pub documents: u64,
}

/// One edit for a short word and two for a long one: at distance two `cat` reaches `dog`'s
/// neighbourhood and every three-letter word is a candidate for every other.
fn tolerance(typed: &str) -> usize {
    match typed.chars().count() {
        0..=3 => 0,
        4..=7 => 1,
        _ => 2,
    }
}

/// Transpositions count as one edit: swapping two letters is among the commonest ways to mistype a
/// name.
fn distance_within(a: &str, b: &str, limit: usize) -> Option<usize> {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
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
        if best_in_row > limit {
            return None;
        }
        std::mem::swap(&mut before_previous, &mut previous);
        std::mem::swap(&mut previous, &mut current);
    }

    let distance = previous[b.len()];
    (distance <= limit).then_some(distance)
}

/// The same floor below which [`tolerance`] corrects nothing: two letters begin a quarter of the
/// vocabulary.
const SHORTEST_BEGINNING: usize = 3;

/// `typed~N` reaches terms that begin within `N` edits of the word (mistyped, unfinished, or both)
/// and `typed*` only
/// those that begin with it, for a word too short to correct. Unquoted, which is the point: the
/// index reads a quoted
/// term exactly. Only ever a single word of letters and digits, so nothing typed reaches the index
/// as syntax.
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

/// A term that begins with what was typed is the word left unfinished and outranks every
/// correction. Past that, a
/// term within `tolerance` edits, closest first; between equally close terms the one more of the
/// mailbox holds.
pub fn suggest<'a>(typed: &str, vocabulary: impl Iterator<Item = Term<'a>>) -> Option<Suggestion> {
    let typed = typed.to_lowercase();
    let limit = tolerance(&typed);
    let completes = typed.chars().count() >= SHORTEST_BEGINNING;
    if limit == 0 && !completes {
        return None;
    }

    let mut best: Option<(usize, Term<'a>)> = None;
    for term in vocabulary {
        // A term that is what was typed is not a suggestion: the query found nothing for another
        // reason.
        if term.text.eq_ignore_ascii_case(&typed) {
            return None;
        }
        let text = term.text.to_lowercase();
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
            // Closer wins; between equals the commoner. Not a tuple comparison: that orders
            // `documents` ascending on a tie and offered `hanna` over `hannah`.
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
/// The word itself when the vocabulary holds it, then the words it begins, then the words it is a
/// misspelling of, at
/// most `most`. The one place a query is widened rather than answered with an offer, only for the
/// command bar's
/// interactive search (ADR 0037). A word the vocabulary holds is not corrected: `your`, one edit
/// from `tour`, would drown it.
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Suggestions {
    pub ghost: Option<String>,
    pub words: Vec<Completion>,
    pub labels: Vec<Completion>,
    pub lists: Vec<Completion>,
    pub files: Vec<Completion>,
    pub folders: Vec<Completion>,
    pub people: Vec<Person>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub text: String,
    pub query: String,
    pub count: u64,
    /// A floor ("1,000+"): a suggestion as common as `as` is not counted to the end of the mailbox
    /// while a key is being typed (D29).
    pub capped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub name: Option<String>,
    pub address: String,
    pub received: u64,
    pub sent: u64,
    pub last: Option<chrono::DateTime<chrono::Utc>>,
}

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
            capped: false,
        })
        .collect()
}

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
        let offered = suggest("hnanah", vocabulary().into_iter()).expect("a suggestion");
        assert_eq!(offered.term, "hannah");
    }

    #[test]
    fn the_closer_term_wins_and_then_the_commoner_one() {
        let offered = suggest("hannh", vocabulary().into_iter()).expect("a suggestion");
        assert_eq!(
            offered.term, "hannah",
            "between two equally close terms, the one in more messages"
        );
    }

    #[test]
    fn a_short_word_is_never_corrected() {
        assert_eq!(suggest("cot", vocabulary().into_iter()), None);
        assert_eq!(suggest("dot", vocabulary().into_iter()), None);
    }

    #[test]
    fn a_term_the_index_holds_is_never_offered_back() {
        assert_eq!(suggest("hannah", vocabulary().into_iter()), None);
    }

    #[test]
    fn a_word_nothing_resembles_is_left_alone() {
        assert_eq!(suggest("xylophone", vocabulary().into_iter()), None);
    }

    #[test]
    fn an_unfinished_word_is_offered_the_word_it_begins() {
        let offered = suggest("quart", vocabulary().into_iter()).expect("a completion");
        assert_eq!(offered.term, "quarterly");
    }

    #[test]
    fn a_short_start_is_still_completed_though_never_corrected() {
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
        assert_eq!(widened("Ban").as_deref(), Some("ban*"));
    }

    #[test]
    fn nothing_widens_two_letters_or_anything_that_is_not_one_word() {
        assert_eq!(widened("ha"), None);
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
        let vocabulary: Vec<Term<'static>> = [("tour", 3), ("tours", 2), ("your", 900)]
            .into_iter()
            .map(|(text, documents)| Term { text, documents })
            .collect();
        assert_eq!(near("tour", vocabulary.into_iter(), 8), ["tour", "tours"]);
    }
}
