//! RFC 3676 `format=flowed`: soft-wrapped plain text a flowed-aware reader
//! reflows to the window, and any other reader still reads correctly.
//!
//! Three rules, in order of how often they matter:
//!
//! * **A soft break is a space, then a newline.** Not a marker character —
//!   the space that already separated two words becomes the break, so
//!   [`unwrap`] undoes it by deleting exactly the newline and keeping the
//!   space. That is what makes wrapping and unwrapping exact inverses for
//!   ordinary prose, rather than an approximation that loses whatever
//!   whitespace the writer had.
//! * **The last physical line of a paragraph carries no trailing space.**
//!   That is a *fixed* line — RFC 3676's word for "this newline is real,
//!   stop joining here" — and it is how a hard [`Inline::Break`] survives
//!   the trip.
//! * **Space-stuffing.** A line whose content would start with a space, a
//!   `>`, or the five literal characters `From ` is ambiguous — the first
//!   two collide with a soft-break continuation and a quote marker, the
//!   third is the decades-old mbox escape. One space is added; every reader,
//!   flowed-aware or not, strips exactly one on the way back in (RFC 3676
//!   §4.4), which is why [`unwrap`] does the same unconditionally rather
//!   than trying to guess whether stuffing actually happened.
//!
//! [`Inline::Break`]: crate::document::Inline::Break

use std::borrow::Cow;

/// The wrap width the mockup (`04-format-bar-states.png`) shows, inside RFC
/// 3676 §4.2's conventional 66–78 range.
pub const WIDTH: usize = 72;

/// Wrap one line of prose — no embedded `\n` — into `format=flowed` text.
///
/// `prefix` is quote depth already rendered as `>` markers (empty for
/// unquoted text); every physical line gets it, and it counts against
/// `width`, so a deeply quoted line still fits the terminal it names.
///
/// Words are split on a single space and rejoined the same way: this is
/// exact for the prose [`Document`](crate::document::Document) actually
/// renders, which never emits doubled spaces, but is not a general-purpose
/// whitespace-preserving wrapper — see the module docs' first rule for why
/// that restriction is what keeps [`unwrap`] exact rather than lossy.
///
/// A single word longer than the budget is never broken mid-word — it is
/// the one case this cannot keep the terminal's own promise, and the
/// alternative (finding a shorter word to butcher instead) is worse.
pub fn wrap(text: &str, prefix: &str, width: usize) -> String {
    // Every line but the last carries a trailing soft-break space once
    // rendered, which is a column the reader actually sees -- reserved here
    // unconditionally, since which line will turn out to be the last is not
    // known until the whole paragraph has been filled. The true last line
    // ends up, at most, one column shorter than it strictly had to be.
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

/// The inverse of [`wrap`]: `format=flowed` text back to the one line it
/// came from.
///
/// `prefix` must be the same quote-depth string `wrap` was given — it is
/// stripped from every physical line before the rest runs. Exact for
/// anything `wrap` produced; see the module docs for the two rules that
/// make it so.
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

/// Adds the one space RFC 3676 §4.3 asks for when `line`'s first character
/// would otherwise be read as a soft-break continuation, a quote marker, or
/// the mbox `From ` escape.
///
/// Public under a name that says what it is *for* rather than what [`wrap`]
/// uses it for: a caller emitting a line this module never wrapped —
/// [`crate::document`]'s `Block::Pre`, which shares the wire with flowed
/// text but must never be reflowed — still needs the exact same escaping,
/// since the ambiguity is about what the byte stream looks like, not about
/// how any particular line got there.
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
        // At width 10 with a two-character prefix, the content budget is 8:
        // "eight" (5) plus " nine" (5) is 10, over budget, so it wraps.
        let wrapped = wrap("eight nine", "> ", 10);
        assert_eq!(wrapped, "> eight \n> nine");
    }
}
