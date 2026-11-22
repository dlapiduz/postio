//! RFC 3676 `format=flowed`. A soft break is a space then a newline, so `unwrap` deletes only the
//! newline and is exact for prose.
//! The last line of a paragraph has no trailing space (a fixed line), which is how a hard `Break`
//! survives.
//! Lines starting with space, `>` or `From ` are space-stuffed; `unwrap` strips one space
//! unconditionally (§4.4).

use std::borrow::Cow;

pub const WIDTH: usize = 72;

/// Wraps one line of prose (no `\n`); `prefix` is the rendered quote markers and counts against
/// `width`.
/// Words split on single spaces, so this is not a whitespace-preserving wrapper. A word longer than
/// the budget is never broken.
pub fn wrap(text: &str, prefix: &str, width: usize) -> String {
    // Reserves a column for the soft-break space every line but the last carries; the last may end
    // one column short.
    let budget = width
        .saturating_sub(prefix.chars().count())
        .saturating_sub(1)
        .max(1);

    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split(' ') {
        let candidate_len = if current.is_empty() {
            word.chars().count()
        } else {
            current.chars().count() + 1 + word.chars().count()
        };
        if !current.is_empty() && candidate_len > budget {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    lines.push(current);

    let last = lines.len() - 1;
    let mut out = String::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(prefix);
        out.push_str(&stuff(line));
        if index != last {
            out.push(' ');
        }
    }
    out
}

/// The inverse of `wrap`; `prefix` must match what `wrap` was given.
pub fn unwrap(flowed: &str, prefix: &str) -> String {
    flowed
        .split('\n')
        .map(|line| {
            let content = line.strip_prefix(prefix).unwrap_or(line);
            content.strip_prefix(' ').unwrap_or(content)
        })
        .collect::<Vec<_>>()
        .concat()
}

/// Space-stuffs a line (RFC 3676 §4.3), for callers emitting lines `wrap` never saw, such as
/// `Block::Pre`.
pub fn stuff_for_display(line: &str) -> Cow<'_, str> {
    stuff(line)
}

fn stuff(line: &str) -> Cow<'_, str> {
    if line.starts_with(' ') || line.starts_with('>') || line.starts_with("From ") {
        Cow::Owned(format!(" {line}"))
    } else {
        Cow::Borrowed(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_is_one_unstuffed_line() {
        assert_eq!(wrap("hello there", "", WIDTH), "hello there");
    }

    #[test]
    fn wrapping_breaks_only_at_a_space_and_keeps_it() {
        let wrapped = wrap("hello there world", "", 8);
        assert_eq!(wrapped, "hello \nthere \nworld");
    }

    #[test]
    fn the_round_trip_reconstructs_the_original_paragraph() {
        for text in [
            "one",
            "a short sentence",
            "a considerably longer sentence that should wrap across several \
             physical lines once it is flowed at a narrow width",
            "supercalifragilisticexpialidocious is one word longer than the budget",
        ] {
            let wrapped = wrap(text, "", 24);
            assert_eq!(
                unwrap(&wrapped, ""),
                text,
                "round trip failed for {text:?}, wrapped as {wrapped:?}"
            );
        }
    }

    #[test]
    fn the_round_trip_holds_through_a_quote_prefix_too() {
        let text = "a quoted sentence long enough to need more than one line at this width";
        let wrapped = wrap(text, "> ", 24);
        assert_eq!(unwrap(&wrapped, "> "), text);
    }

    #[test]
    fn only_the_last_line_is_fixed() {
        let wrapped = wrap("one two three four five", "", 8);
        let lines: Vec<&str> = wrapped.split('\n').collect();
        for line in &lines[..lines.len() - 1] {
            assert!(line.ends_with(' '), "{line:?} should carry a soft break");
        }
        assert!(
            !lines.last().unwrap().ends_with(' '),
            "the last physical line must not look like it continues"
        );
    }

    #[test]
    fn a_line_that_would_start_with_a_quote_marker_is_stuffed() {
        let wrapped = wrap(">not actually a quote", "", WIDTH);
        assert_eq!(wrapped, " >not actually a quote");
        assert_eq!(unwrap(&wrapped, ""), ">not actually a quote");
    }

    #[test]
    fn a_line_starting_with_the_mbox_escape_is_stuffed() {
        let wrapped = wrap("From the desk of Ada Lovelace", "", WIDTH);
        assert_eq!(wrapped, " From the desk of Ada Lovelace");
        assert_eq!(unwrap(&wrapped, ""), "From the desk of Ada Lovelace");
    }

    #[test]
    fn a_single_word_longer_than_the_budget_is_not_broken_mid_word() {
        let word = "supercalifragilisticexpialidocious";
        assert_eq!(wrap(word, "", 8), word);
    }

    #[test]
    fn a_quote_prefix_counts_against_the_width() {
        let wrapped = wrap("eight nine", "> ", 10);
        assert_eq!(wrapped, "> eight \n> nine");
    }
}
