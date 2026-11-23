//! Where a query matched, as byte ranges into plain text. A snippet already carries FTS5 markers
//! ([`from_snippet`]); a preview has none, so [`find`] locates [`terms`] in it.
//! Matching is by token, FTS5's rule, not by substring: `mail` must not be painted inside
//! `maildir`, and a multi-word term (`from:ada@example.com`) matches consecutive tokens.

use std::ops::Range;

use crate::query::{Filter, ParsedQuery};

/// A control character rather than `<b>`: anything spellable is something a message could contain
/// and so forge.
pub const MATCH_START: char = '\u{1}';

pub const MATCH_END: char = '\u{2}';

pub const ELLIPSIS: &str = "…";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Highlighted {
    pub text: String,
    pub matches: Vec<Range<usize>>,
}

impl Highlighted {
    pub fn is_highlighted(&self) -> bool {
        !self.matches.is_empty()
    }

    /// Empty runs are never emitted, so a match at either end produces no blank.
    pub fn runs(&self) -> Vec<(&str, bool)> {
        let mut runs = Vec::new();
        let mut cursor = 0usize;
        for matched in &self.matches {
            if matched.start > cursor {
                runs.push((&self.text[cursor..matched.start], false));
            }
            if matched.end > matched.start {
                runs.push((&self.text[matched.start..matched.end], true));
            }
            cursor = matched.end;
        }
        if cursor < self.text.len() {
            runs.push((&self.text[cursor..], false));
        }
        runs
    }
}

/// A marker with no partner is dropped rather than trusted: a message can contain the control
/// characters, and a forged one must not paint a highlight the query did not earn.
pub fn from_snippet(snippet: &str) -> Highlighted {
    let mut text = String::with_capacity(snippet.len());
    let mut matches = Vec::new();
    let mut open: Option<usize> = None;

    for character in snippet.chars() {
        match character {
            MATCH_START => open = Some(text.len()),
            MATCH_END => {
                if let Some(start) = open.take()
                    && text.len() > start
                {
                    matches.push(start..text.len());
                }
            }
            _ => text.push(character),
        }
    }

    Highlighted { text, matches }
}

/// Positive terms only: a negated term is not in the message, a [`Partial`](crate::query::Partial)
/// constrains nothing, and non-text operators contribute nothing.
pub fn terms(query: &ParsedQuery) -> Vec<String> {
    let mut terms: Vec<String> = query
        .text_terms()
        .filter(|term| !term.negated)
        .map(|term| term.value.clone())
        .collect();

    for filter in query
        .filters()
        .filter(|clause| !clause.negated)
        .flat_map(|clause| clause.filter.alternatives())
    {
        let value = match filter {
            Filter::From(value)
            | Filter::To(value)
            | Filter::Subject(value)
            | Filter::Filename(value)
            | Filter::List(value) => value,
            _ => continue,
        };
        terms.push(value.clone());
    }

    terms.retain(|term| !term.is_empty());
    terms
}

pub fn find(text: &str, terms: &[String]) -> Vec<Range<usize>> {
    let tokens = tokenize(text);
    if tokens.is_empty() {
        return Vec::new();
    }

    let mut found = Vec::new();
    for term in terms {
        let wanted = words(term);
        if wanted.is_empty() {
            continue;
        }
        for start in 0..tokens.len() {
            if start + wanted.len() > tokens.len() {
                break;
            }
            let matched = wanted
                .iter()
                .zip(&tokens[start..])
                .all(|(word, (_, token))| word == token);
            if matched {
                let from = tokens[start].0.start;
                let to = tokens[start + wanted.len() - 1].0.end;
                found.push(from..to);
            }
        }
    }

    merge(found)
}

pub fn highlight(text: &str, terms: &[String]) -> Highlighted {
    Highlighted {
        matches: find(text, terms),
        text: text.to_owned(),
    }
}

/// The number FTS5's `snippet()` took, so a result row stays the length it has always been.
pub const SNIPPET_TOKENS: usize = 12;

/// `snippet()` needs indexed content and `message_bodies_fts` is `content = ''` (#407), so the
/// excerpt is cut here from the same text that was indexed.
/// [`find`]'s token rule is FTS5's own. A query that hit only the subject leaves the body unmatched
/// and this answers a leading excerpt rather than nothing.
/// Whitespace is collapsed to one line; it is a separator to the tokenizer, so matching is
/// unchanged.
pub fn snippet(text: &str, terms: &[String]) -> String {
    let flat = collapse_whitespace(text);
    let tokens = tokenize(&flat);
    if tokens.is_empty() {
        return String::new();
    }
    let found = find(&flat, terms);

    let first = found.first().and_then(|range| {
        tokens
            .iter()
            .position(|(span, _)| span.start >= range.start)
    });
    let (from_token, to_token) = match first {
        Some(index) => (
            index.saturating_sub(SNIPPET_TOKENS / 2),
            (index + SNIPPET_TOKENS / 2 + 1).min(tokens.len()),
        ),
        None => (0, SNIPPET_TOKENS.min(tokens.len())),
    };

    // Cut at token boundaries, except at the ends of the text itself: an
    // untrimmed window is the whole string, and stopping at the last token
    // would drop the full stop after it.
    let from = if from_token == 0 {
        0
    } else {
        tokens[from_token].0.start
    };
    let to = if to_token == tokens.len() {
        flat.len()
    } else {
        tokens[to_token - 1].0.end
    };
    let mut out = String::with_capacity(to - from + 8);
    if from_token > 0 {
        out.push_str(ELLIPSIS);
    }

    let mut cursor = from;
    for range in found {
        if range.end <= from {
            continue;
        }
        if range.start >= to {
            break;
        }
        let start = range.start.max(from);
        let end = range.end.min(to);
        out.push_str(&flat[cursor..start]);
        out.push(MATCH_START);
        out.push_str(&flat[start..end]);
        out.push(MATCH_END);
        cursor = end;
    }
    out.push_str(&flat[cursor..to]);
    if to_token < tokens.len() {
        out.push_str(ELLIPSIS);
    }
    out
}

pub(crate) fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for character in text.chars() {
        if character.is_whitespace() {
            in_space = !out.is_empty();
        } else {
            if in_space {
                out.push(' ');
                in_space = false;
            }
            out.push(character);
        }
    }
    out
}

fn tokenize(text: &str) -> Vec<(Range<usize>, String)> {
    let mut tokens = Vec::new();
    let mut start: Option<usize> = None;

    for (offset, character) in text.char_indices() {
        if character.is_alphanumeric() {
            start.get_or_insert(offset);
        } else if let Some(from) = start.take() {
            tokens.push((from..offset, text[from..offset].to_lowercase()));
        }
    }
    if let Some(from) = start {
        tokens.push((from..text.len(), text[from..].to_lowercase()));
    }
    tokens
}

fn words(term: &str) -> Vec<String> {
    tokenize(term).into_iter().map(|(_, word)| word).collect()
}

fn merge(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(query: &str) -> ParsedQuery {
        crate::parse(
            query,
            chrono::NaiveDate::from_ymd_opt(2026, 8, 22).expect("a real date"),
        )
    }

    fn marked(text: &str) -> String {
        text.replace('[', &MATCH_START.to_string())
            .replace(']', &MATCH_END.to_string())
    }

    fn painted(highlighted: &Highlighted) -> String {
        highlighted
            .runs()
            .iter()
            .map(|(run, matched)| {
                if *matched {
                    format!("[{run}]")
                } else {
                    (*run).to_string()
                }
            })
            .collect()
    }

    #[test]
    fn a_snippet_loses_its_markers_and_keeps_where_they_were() {
        let highlighted = from_snippet(&marked("a 40k-message [maildir] takes nine minutes"));

        assert_eq!(highlighted.text, "a 40k-message maildir takes nine minutes");
        assert_eq!(highlighted.matches, vec![14..21]);
        assert_eq!(&highlighted.text[14..21], "maildir");
    }

    #[test]
    fn a_snippet_with_no_match_is_plain_text() {
        let highlighted = from_snippet("nothing marked here");

        assert_eq!(highlighted.text, "nothing marked here");
        assert!(!highlighted.is_highlighted());
        assert_eq!(painted(&highlighted), "nothing marked here");
    }

    #[test]
    fn several_matches_in_one_snippet_all_survive() {
        let highlighted = from_snippet(&marked("[one] and [two] and [three]"));

        assert_eq!(painted(&highlighted), "[one] and [two] and [three]");
        assert_eq!(highlighted.matches.len(), 3);
    }

    #[test]
    fn a_marker_a_message_forged_cannot_paint_a_highlight() {
        let highlighted = from_snippet(&format!("plain {MATCH_START}text"));

        assert_eq!(highlighted.text, "plain text");
        assert!(!highlighted.is_highlighted());

        let stray = from_snippet(&format!("plain {MATCH_END}text"));
        assert_eq!(stray.text, "plain text");
        assert!(!stray.is_highlighted());
    }

    #[test]
    fn an_empty_match_is_not_a_match() {
        let highlighted = from_snippet(&marked("nothing [] here"));

        assert_eq!(highlighted.text, "nothing  here");
        assert!(!highlighted.is_highlighted());
    }

    #[test]
    fn runs_do_not_emit_empty_pieces_at_the_edges() {
        let highlighted = from_snippet(&marked("[whole]"));

        assert_eq!(highlighted.runs(), vec![("whole", true)]);
    }

    #[test]
    fn free_text_is_what_a_plain_query_asks_for() {
        assert_eq!(terms(&parse("maildir rebuild")), ["maildir", "rebuild"]);
    }

    #[test]
    fn a_negated_term_has_nothing_to_point_at() {
        assert_eq!(terms(&parse("maildir -notmuch")), ["maildir"]);
    }

    #[test]
    fn a_half_typed_operator_asks_for_nothing_yet() {
        assert!(terms(&parse("subject:")).is_empty());
    }

    #[test]
    fn text_valued_operators_are_terms_too() {
        let asked = terms(&parse(
            "subject:invoice from:ada is:unread larger:1M after:aug1",
        ));

        assert_eq!(
            asked,
            ["invoice", "ada"],
            "`is:`, `larger:` and `after:` have no text in the message to point at"
        );
    }

    #[test]
    fn every_value_of_a_positive_set_is_a_term() {
        assert_eq!(
            terms(&parse("from:{ada tomas} subject:{budget plan} -to:{bo cy}")),
            ["ada", "tomas", "budget", "plan"]
        );
        assert!(terms(&parse("in:{archive inbox}")).is_empty());
    }

    #[test]
    fn a_mailbox_is_not_part_of_what_is_being_read() {
        assert!(terms(&parse("in:archive")).is_empty());
    }

    #[test]
    fn a_term_matches_a_whole_token_and_not_a_prefix_of_one() {
        let found = highlight("the maildir index", &["mail".to_owned()]);
        assert!(
            !found.is_highlighted(),
            "FTS5 did not match `mail` against `maildir`, so nothing here may"
        );

        let found = highlight("the maildir index", &["maildir".to_owned()]);
        assert_eq!(painted(&found), "the [maildir] index");
    }

    #[test]
    fn case_is_folded_both_ways() {
        let found = highlight("Maildir and MAILDIR", &["maildir".to_owned()]);
        assert_eq!(painted(&found), "[Maildir] and [MAILDIR]");

        let found = highlight("maildir", &["MAILDIR".to_owned()]);
        assert_eq!(painted(&found), "[maildir]");
    }

    #[test]
    fn a_phrase_matches_consecutive_tokens_only() {
        let terms = ["quarterly report".to_owned()];

        assert_eq!(
            painted(&highlight("the quarterly report is late", &terms)),
            "the [quarterly report] is late"
        );
        assert_eq!(
            painted(&highlight("quarterly and the report", &terms)),
            "quarterly and the report",
            "the words are there but the phrase is not"
        );
    }

    #[test]
    fn an_address_highlights_itself_and_not_the_words_it_is_made_of() {
        let found = highlight(
            "write to ada@example.com, not to the example above",
            &["ada@example.com".to_owned()],
        );

        assert_eq!(
            painted(&found),
            "write to [ada@example.com], not to the example above"
        );
    }

    #[test]
    fn two_terms_over_the_same_word_draw_one_highlight() {
        let found = highlight(
            "the maildir index",
            &["maildir".to_owned(), "maildir".to_owned()],
        );

        assert_eq!(found.matches.len(), 1);
        assert_eq!(painted(&found), "the [maildir] index");
    }

    #[test]
    fn matches_come_back_in_the_order_they_appear() {
        let found = highlight(
            "rebuild the maildir, then rebuild the index",
            &["index".to_owned(), "rebuild".to_owned()],
        );

        assert_eq!(
            painted(&found),
            "[rebuild] the maildir, then [rebuild] the [index]"
        );
        assert!(
            found
                .matches
                .windows(2)
                .all(|pair| pair[0].end <= pair[1].start)
        );
    }

    #[test]
    fn a_term_with_no_word_characters_matches_nothing() {
        assert!(find("anything at all", &["---".to_owned()]).is_empty());
        assert!(find("anything at all", &[String::new()]).is_empty());
    }

    #[test]
    fn text_with_no_words_has_nothing_to_match() {
        assert!(find("--- ---", &["maildir".to_owned()]).is_empty());
    }

    #[test]
    fn matching_is_safe_across_multi_byte_characters() {
        let found = highlight("the café is naïve", &["café".to_owned()]);

        assert_eq!(painted(&found), "the [café] is naïve");
        for matched in &found.matches {
            assert!(found.text.is_char_boundary(matched.start));
            assert!(found.text.is_char_boundary(matched.end));
        }
    }
}

#[cfg(test)]
mod snippet_tests {

    use super::*;

    fn marked(text: &str, terms: &[&str]) -> String {
        let terms: Vec<String> = terms.iter().map(|term| (*term).to_owned()).collect();
        snippet(text, &terms)
            .replace(MATCH_START, "[")
            .replace(MATCH_END, "]")
    }

    #[test]
    fn the_match_is_wrapped_where_it_was_found() {
        assert_eq!(
            marked("The quarterly report is attached.", &["report"]),
            "The quarterly [report] is attached."
        );
    }

    #[test]
    fn a_long_body_is_cut_around_the_match_and_says_so() {
        let text = format!("{} needle {}", "alpha ".repeat(40), "omega ".repeat(40));

        let out = marked(&text, &["needle"]);

        assert!(out.starts_with('…'), "{out}");
        assert!(out.ends_with('…'), "{out}");
        assert!(out.contains("[needle]"), "{out}");
        assert!(
            out.matches("alpha").count() <= SNIPPET_TOKENS,
            "the window is a window: {out}"
        );
    }

    #[test]
    fn text_with_no_match_still_gives_the_row_its_opening() {
        let text = "alpha ".repeat(40);

        let out = marked(&text, &["needle"]);

        assert!(out.starts_with("alpha"), "{out}");
        assert!(out.ends_with('…'), "{out}");
        assert!(!out.contains('['), "nothing matched, so nothing is marked");
    }

    #[test]
    fn a_phrase_marks_the_whole_phrase_and_not_its_words_apart() {
        let out = marked(
            "A quarterly summary, then the quarterly report itself.",
            &["quarterly report"],
        );

        assert!(out.contains("[quarterly report]"), "{out}");
        assert_eq!(out.matches('[').count(), 1, "the loose word is not a hit");
    }

    #[test]
    fn every_match_in_the_window_is_marked_not_only_the_first() {
        let out = marked("report about the report", &["report"]);

        assert_eq!(out, "[report] about the [report]");
    }

    #[test]
    fn a_match_at_either_end_does_not_leave_a_stray_marker() {
        assert_eq!(marked("needle", &["needle"]), "[needle]");
        assert_eq!(marked("a needle", &["needle"]), "a [needle]");
        assert_eq!(marked("needle a", &["needle"]), "[needle] a");
    }

    #[test]
    fn newlines_become_spaces_without_moving_what_matched() {
        assert_eq!(
            marked("Dear Ada,\n\n   The report\tis ready.\n", &["report"]),
            "Dear Ada, The [report] is ready."
        );
    }

    #[test]
    fn a_star_is_a_character_and_not_a_prefix_search() {
        // Postio's grammar has no prefix operator: `fts_literal` quotes every term and `unicode61`
        // drops `*` as a separator, so `report*` is the term `report`.
        let out = marked("The report and the reporting", &["report*"]);

        assert!(out.contains("[report]"), "{out}");
        assert!(
            !out.contains("[reporting]"),
            "a prefix search would have matched this, and there is no prefix \
             search: {out}"
        );
    }

    #[test]
    fn near_is_a_word_and_not_an_operator() {
        let out = marked("the mill is near the store", &["near"]);

        assert_eq!(out, "the mill is [near] the store");
    }

    #[test]
    fn text_with_no_tokens_at_all_produces_nothing() {
        assert_eq!(snippet("", &["report".to_owned()]), "");
        assert_eq!(snippet("   \n\t ", &["report".to_owned()]), "");
    }

    #[test]
    fn what_is_marked_is_what_from_snippet_reads_back() {
        let terms = vec!["report".to_owned()];
        let text = "The quarterly report is attached.";

        let read_back = from_snippet(&snippet(text, &terms));

        assert_eq!(read_back.text, text);
        assert_eq!(
            read_back
                .matches
                .iter()
                .map(|range| &read_back.text[range.clone()])
                .collect::<Vec<_>>(),
            vec!["report"]
        );
    }
}
