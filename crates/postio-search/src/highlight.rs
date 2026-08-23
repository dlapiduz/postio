//! Where a query matched, so a surface can draw it.
//!
//! Canvas 2b highlights the matched terms twice: once in the result row's
//! snippet, and again in the preview beside it. Those are two different
//! problems wearing one face.
//!
//! * The **snippet** already knows. FTS5 cut it out of the indexed text and
//!   wrapped each match in [`MATCH_START`]/[`MATCH_END`] as it went, so
//!   [`from_snippet`] only has to take the markers back out and remember
//!   where they were.
//! * The **preview** does not. It is the message body as the reader renders
//!   it, and nothing has marked it up — so [`find`] has to locate the query's
//!   own terms in it, using [`terms`] to say what those are.
//!
//! Both answer in byte ranges into plain text, because that is what every
//! consumer can turn into what it needs: Pango attributes, a CSS span, or a
//! test assertion.
//!
//! # Why matching is by token and not by substring
//!
//! FTS5's default tokenizer splits on non-alphanumerics, so `mail` does not
//! match `maildir` — and a highlighter that used `str::find` would paint
//! `mail` inside `maildir` for a query that never matched that message on
//! that word. The acceptance criterion is that the highlighting *matches the
//! executed query*, so the rule here is the tokenizer's rule: a term matches
//! a whole token, and a multi-word term matches consecutive tokens. That
//! also makes `from:ada@example.com` behave: it is three tokens in a row,
//! which highlights the address and not every `example` on the page.

use std::ops::Range;

use crate::query::{Filter, ParsedQuery};

/// Marks the start of a match inside a snippet.
///
/// A control character rather than `<b>`: the snippet is plain text on its
/// way to a widget, and anything spellable is something a message could
/// contain and thereby forge.
pub const MATCH_START: char = '\u{1}';

/// Marks the end of a match inside a snippet. See [`MATCH_START`].
pub const MATCH_END: char = '\u{2}';

/// What [`snippet`](https://sqlite.org/fts5.html#the_snippet_function) puts
/// where it cut.
pub const ELLIPSIS: &str = "…";

/// Text, plus where in it the query matched.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Highlighted {
    /// The text with no markers in it, ready to display.
    pub text: String,
    /// Byte ranges into `text`, in order and never overlapping.
    pub matches: Vec<Range<usize>>,
}

impl Highlighted {
    /// Whether the query matched anywhere in this text.
    pub fn is_highlighted(&self) -> bool {
        !self.matches.is_empty()
    }

    /// The text split into runs, each flagged with whether it matched.
    ///
    /// The shape a renderer actually wants: walk it and emit a plain span or
    /// a highlighted one. Empty runs are never emitted, so a match at either
    /// end does not produce a leading or trailing blank.
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

/// Reads a snippet FTS5 produced, taking the markers back out.
///
/// Unbalanced markers cannot arise from `snippet()`, but they can arise from
/// a message that contains the control characters itself, so a marker with no
/// partner is simply dropped rather than trusted — a forged `\u{1}` must not
/// be able to paint a highlight the query did not earn.
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

/// The terms a query asked for, as the text a reader would see them in.
///
/// Positive terms only. A negated term is not in this message by definition,
/// and a [`Partial`](crate::query::Partial) constrains nothing yet — neither
/// has anything to point at. Operators whose value is not text (`is:`,
/// `has:`, dates, sizes) contribute nothing either.
///
/// `from:`/`to:` values are included: they are terms the user asked about,
/// and the token rule above keeps an address from spraying highlights over
/// every word it happens to share with the body.
pub fn terms(query: &ParsedQuery) -> Vec<String> {
    let mut terms: Vec<String> = query
        .text_terms()
        .filter(|term| !term.negated)
        .map(|term| term.value.clone())
        .collect();

    for clause in query.filters().filter(|clause| !clause.negated) {
        let value = match &clause.filter {
            Filter::From(value)
            | Filter::To(value)
            | Filter::Subject(value)
            | Filter::Filename(value)
            | Filter::List(value) => value,
            // `in:` names a mailbox, which is not part of what is being read.
            _ => continue,
        };
        terms.push(value.clone());
    }

    terms.retain(|term| !term.is_empty());
    terms
}

/// Finds every place `terms` matches in `text`, merged and in order.
///
/// See the module docs for why this is token matching rather than substring
/// matching. Case is folded, so `Maildir` matches `maildir`.
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

/// [`find`], returning the text alongside — the same shape [`from_snippet`]
/// answers in, so a surface can draw a snippet and a preview the one way.
pub fn highlight(text: &str, terms: &[String]) -> Highlighted {
    Highlighted {
        matches: find(text, terms),
        text: text.to_owned(),
    }
}

/// Splits text into `(byte range, folded token)` the way FTS5's default
/// tokenizer does: runs of alphanumerics, everything else a separator.
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

/// A term's tokens, in order. `"ada@example.com"` is three of them.
fn words(term: &str) -> Vec<String> {
    tokenize(term).into_iter().map(|(_, word)| word).collect()
}

/// Sorts ranges and folds overlapping or touching ones together, so two terms
/// that matched the same word do not draw the highlight twice.
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

    // -- snippets ---------------------------------------------------------

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
        // An opening marker with no partner, which `snippet()` never emits —
        // so it came from the message, and a message does not get to decide
        // what looks like a hit.
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

    // -- what a query asks for --------------------------------------------

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
    fn a_mailbox_is_not_part_of_what_is_being_read() {
        assert!(terms(&parse("in:archive")).is_empty());
    }

    // -- finding terms in text --------------------------------------------

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
        // The ranges have to land on char boundaries or slicing them panics.
        for matched in &found.matches {
            assert!(found.text.is_char_boundary(matched.start));
            assert!(found.text.is_char_boundary(matched.end));
        }
    }
}
