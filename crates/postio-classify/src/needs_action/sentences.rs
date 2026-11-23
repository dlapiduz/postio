//! The own text as the detector reads it: clauses with character offsets, so a marker's quote is a
//! span of the text and nothing else.

use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Clause {
    pub(super) span: Range<usize>,
    pub(super) text: String,
    /// The words a dash cut off in front, when few enough to say who it is put to: the "Mateo" of
    /// "Mateo — please share the link".
    pub(super) before_dash: Option<String>,
}

const VOCATIVE_WORDS: usize = 3;

/// Paragraphs split at blank lines (one indented as code is not prose); a `>` line is quoted and
/// never read; a sentence ends at `.`, `?`, `!` or `…` before whitespace, not at a URL query, a
/// decimal or a common abbreviation; clauses split at `;` and `—`.
pub(super) fn clauses(own: &str) -> Vec<Clause> {
    let chars: Vec<char> = own.chars().collect();
    let mut out = Vec::new();
    for block in blocks(&chars) {
        for sentence in sentences(&chars, block) {
            split(&chars, sentence, &mut out);
        }
    }
    out
}

fn blocks(chars: &[char]) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (at, character) in chars.iter().enumerate() {
        if *character == '\n' {
            lines.push(start..at);
            start = at + 1;
        }
    }
    lines.push(start..chars.len());

    let mut blocks = Vec::new();
    let mut run: Vec<Range<usize>> = Vec::new();
    let mut paragraph_is_code = true;
    let mut paragraph: Vec<Range<usize>> = Vec::new();
    let mut flush_paragraph = |paragraph: &mut Vec<Range<usize>>, code: bool| {
        if !code {
            blocks.append(paragraph);
        }
        paragraph.clear();
    };
    for line in lines {
        let text = &chars[line.clone()];
        let first = text.iter().position(|character| !character.is_whitespace());
        let blank = first.is_none();
        let quoted = first.is_some_and(|first| text[first] == '>');
        if blank || quoted {
            if let (Some(first), Some(last)) = (run.first(), run.last()) {
                paragraph.push(first.start..last.end);
            }
            run.clear();
        } else {
            paragraph_is_code &= indented(text);
            run.push(line);
        }
        if blank {
            flush_paragraph(&mut paragraph, paragraph_is_code);
            paragraph_is_code = true;
        }
    }
    if let (Some(first), Some(last)) = (run.first(), run.last()) {
        paragraph.push(first.start..last.end);
    }
    flush_paragraph(&mut paragraph, paragraph_is_code);
    blocks
}

fn indented(line: &[char]) -> bool {
    line.first() == Some(&'\t') || line.iter().take(4).filter(|c| **c == ' ').count() == 4
}

fn sentences(chars: &[char], block: Range<usize>) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = block.start;
    for at in block.clone() {
        let character = chars[at];
        if !matches!(character, '.' | '?' | '!' | '…') {
            continue;
        }
        let ends = at + 1 == block.end || chars[at + 1].is_whitespace();
        if !ends || (character == '.' && abbreviation(chars, start, at)) {
            continue;
        }
        out.push(start..at + 1);
        start = at + 1;
    }
    out.push(start..block.end);
    out
}

fn abbreviation(chars: &[char], start: usize, dot: usize) -> bool {
    let word_start = chars[start..dot]
        .iter()
        .rposition(|character| character.is_whitespace() || *character == '(')
        .map_or(start, |at| start + at + 1);
    let word: String = chars[word_start..dot].iter().collect();
    super::lexicon::ABBREVIATIONS.contains(&word.to_lowercase().as_str())
}

fn split(chars: &[char], sentence: Range<usize>, out: &mut Vec<Clause>) {
    let mut start = sentence.start;
    let mut before_dash = None;
    for at in sentence.clone() {
        let character = chars[at];
        if character != ';' && character != '—' {
            continue;
        }
        let text = push(chars, start..at, before_dash.take(), out);
        if character == '—' {
            before_dash = text.filter(|text| text.split_whitespace().count() <= VOCATIVE_WORDS);
        }
        start = at + 1;
    }
    push(chars, start..sentence.end, before_dash, out);
}

fn push(
    chars: &[char],
    range: Range<usize>,
    before_dash: Option<String>,
    out: &mut Vec<Clause>,
) -> Option<String> {
    let slice = &chars[range.clone()];
    let first = slice
        .iter()
        .position(|character| !character.is_whitespace())?;
    let last = slice
        .iter()
        .rposition(|character| !character.is_whitespace())?;
    let span = range.start + first..range.start + last + 1;
    let text = chars[span.clone()]
        .iter()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    out.push(Clause {
        span,
        text: text.clone(),
        before_dash,
    });
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(own: &str) -> Vec<String> {
        clauses(own).into_iter().map(|clause| clause.text).collect()
    }

    fn assert_spans_are_the_text(own: &str) {
        for clause in clauses(own) {
            let quoted: String = own
                .chars()
                .skip(clause.span.start)
                .take(clause.span.len())
                .collect();
            assert_eq!(
                quoted.split_whitespace().collect::<Vec<_>>().join(" "),
                clause.text,
                "{clause:?}"
            );
            assert_eq!(quoted.trim(), quoted, "a span has no whitespace around it");
        }
    }

    #[test]
    fn a_sentence_is_a_span_of_the_text_itself() {
        let own =
            "Hi Ada,\n\nI've attached the reports. Can you approve these by Friday?\n\nIne\u{301}s";

        assert_eq!(
            texts(own),
            [
                "Hi Ada,",
                "I've attached the reports.",
                "Can you approve these by Friday?",
                "Ine\u{301}s",
            ]
        );
        assert_spans_are_the_text(own);
    }

    #[test]
    fn a_sentence_wrapped_across_lines_is_one_span() {
        let own =
            "Could you have a look at section 12 and tell me\nwhether you think it's reasonable?";

        let clauses = clauses(own);

        assert_eq!(clauses.len(), 1);
        assert_eq!(clauses[0].span, 0..own.chars().count());
        assert_eq!(
            clauses[0].text,
            "Could you have a look at section 12 and tell me whether you think it's reasonable?"
        );
    }

    #[test]
    fn offsets_count_characters_not_bytes() {
        let own = "Olá — can you check the café menu? Merci.";

        let clauses = clauses(own);

        assert_eq!(clauses[1].text, "can you check the café menu?");
        assert_eq!(clauses[1].span, 6..34);
        assert_spans_are_the_text(own);
    }

    #[test]
    fn clauses_split_at_semicolons_and_dashes() {
        assert_eq!(
            texts("Please leave comments by Wednesday; I'd like to freeze it Thursday."),
            [
                "Please leave comments by Wednesday",
                "I'd like to freeze it Thursday."
            ]
        );
        assert_eq!(
            texts("Quick question — are you around on Monday?"),
            ["Quick question", "are you around on Monday?"]
        );
    }

    #[test]
    fn a_dash_keeps_who_the_clause_after_it_is_put_to() {
        let clauses = clauses("Mateo — please share the prototype link with Ada.");

        assert_eq!(clauses[1].text, "please share the prototype link with Ada.");
        assert_eq!(clauses[1].before_dash.as_deref(), Some("Mateo"));
        assert_eq!(clauses[0].before_dash, None);
    }

    #[test]
    fn a_long_run_before_a_dash_names_nobody() {
        let clauses = clauses("The invoices from last month are all paid — thanks for chasing.");

        assert_eq!(clauses[1].before_dash, None);
    }

    #[test]
    fn neither_a_url_s_query_nor_a_decimal_ends_a_sentence() {
        assert_eq!(
            texts(
                "The board is at https://metrics.example.com/board?id=42&range=7d if you want a look. \
                 Clause 7.2 is fine."
            ),
            [
                "The board is at https://metrics.example.com/board?id=42&range=7d if you want a look.",
                "Clause 7.2 is fine.",
            ]
        );
    }

    #[test]
    fn an_abbreviation_does_not_end_a_sentence() {
        assert_eq!(
            texts("Bring something to read, e.g. a book. Thanks."),
            ["Bring something to read, e.g. a book.", "Thanks."]
        );
    }

    #[test]
    fn code_and_quoted_lines_are_never_read() {
        let own =
            "The regex was:\n\n    ^(a+)+$?\n\n> Can you do Tuesday instead?\nYes, Tuesday works.";

        assert_eq!(texts(own), ["The regex was:", "Yes, Tuesday works."]);
        assert_spans_are_the_text(own);
    }

    #[test]
    fn a_blank_line_ends_a_sentence_that_had_no_full_stop() {
        assert_eq!(
            texts("Hi Ada\n\nSend me the numbers\n\nTove"),
            ["Hi Ada", "Send me the numbers", "Tove"]
        );
    }

    #[test]
    fn nothing_comes_of_nothing() {
        assert!(clauses("").is_empty());
        assert!(clauses(" \n\n \t\n").is_empty());
    }
}
