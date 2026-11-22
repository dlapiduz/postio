//! Formatting as edits to Markdown text: what a formatting command does
//! where the body is Markdown -- the terminal composer, and the desktop's
//! Markdown source view (spec 014 US6, US7).
//!
//! Pure functions over the text and a selection, so the two surfaces that
//! edit Markdown make the same edit for the same command, and each rule is
//! red in a second. The surfaces only move their caret and buffer to match
//! the [`Edit`] returned.
//!
//! Offsets are bytes into the text. A selection is a range; an empty one is
//! the caret.

use std::ops::Range;

/// The text after a formatting command, and where the selection goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    /// The whole text, edited.
    pub text: String,
    /// The selection after the edit; empty is a caret.
    pub selection: Range<usize>,
}

impl Edit {
    fn caret(text: String, at: usize) -> Self {
        Edit {
            text,
            selection: at..at,
        }
    }
}

/// `open` and `close` around the selection, or, with nothing selected, a
/// pair with the caret between them to type into.
pub fn wrap(text: &str, selection: Range<usize>, open: &str, close: &str) -> Edit {
    let Range { start, end } = clamp(text, selection);
    let mut out = String::with_capacity(text.len() + open.len() + close.len());
    out.push_str(&text[..start]);
    out.push_str(open);
    out.push_str(&text[start..end]);
    out.push_str(close);
    out.push_str(&text[end..]);
    let inner = start + open.len();
    if start == end {
        Edit::caret(out, inner)
    } else {
        Edit::caret(out, end + open.len() + close.len())
    }
}

/// A link: the selected words as its label and the caret where the address
/// goes, or an empty `[]()` with the caret in the label.
pub fn link(text: &str, selection: Range<usize>) -> Edit {
    let Range { start, end } = clamp(text, selection);
    let words = &text[start..end];
    let out = format!("{}[{words}](){}", &text[..start], &text[end..]);
    if words.is_empty() {
        Edit::caret(out, start + 1)
    } else {
        Edit::caret(out, start + words.len() + 3)
    }
}

/// `prefix` at the start of every line the selection touches, or taken off
/// again when the first of them already has it. The caret keeps its place
/// in the words.
pub fn toggle_prefix(text: &str, selection: Range<usize>, prefix: &str) -> Edit {
    let selection = clamp(text, selection);
    let off = text[line_start(text, selection.start)..].starts_with(prefix);
    edit_lines(text, selection, |line| {
        if off {
            line.strip_prefix(prefix).unwrap_or(line).to_owned()
        } else {
            format!("{prefix}{line}")
        }
    })
}

// ---------------------------------------------------------------------------
// Lines
// ---------------------------------------------------------------------------

/// Rewrite every line the selection touches with `rewrite`, moving the
/// selection by what changed before it on its own lines.
fn edit_lines(text: &str, selection: Range<usize>, rewrite: impl Fn(&str) -> String) -> Edit {
    let first = line_start(text, selection.start);
    let last = line_end(text, selection.end.max(selection.start));
    let mut out = String::with_capacity(text.len() + 16);
    out.push_str(&text[..first]);
    let mut start = selection.start;
    let mut end = selection.end;
    let mut at = first;
    for (index, line) in text[first..last].split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let rewritten = rewrite(line);
        let shift = rewritten.len() as isize - line.len() as isize;
        let line_end = at + line.len();
        let moved = |offset: usize| {
            if (at..=line_end).contains(&offset) {
                // Within the line: keep the place in the words, never inside
                // what was added or taken off at its start.
                let column = offset - at;
                (out.len() as isize + (column as isize + shift).max(0)) as usize
            } else {
                offset
            }
        };
        if (at..=line_end).contains(&selection.start) {
            start = moved(selection.start).min(out.len() + rewritten.len());
        }
        if (at..=line_end).contains(&selection.end) {
            end = moved(selection.end).min(out.len() + rewritten.len());
        }
        out.push_str(&rewritten);
        at = line_end + 1;
    }
    let growth = out.len() as isize - last as isize;
    if selection.end > last {
        end = (selection.end as isize + growth) as usize;
    }
    out.push_str(&text[last..]);
    Edit {
        text: out,
        selection: start..end.max(start),
    }
}

/// The byte offset of the start of the line holding `at`.
pub fn line_start(text: &str, at: usize) -> usize {
    text[..at.min(text.len())].rfind('\n').map_or(0, |n| n + 1)
}

/// The byte offset of the end of the line holding `at`, before its `\n`.
pub fn line_end(text: &str, at: usize) -> usize {
    let at = at.min(text.len());
    text[at..].find('\n').map_or(text.len(), |n| at + n)
}

/// A selection inside the text and on character boundaries.
fn clamp(text: &str, selection: Range<usize>) -> Range<usize> {
    let fit = |mut at: usize| {
        at = at.min(text.len());
        while !text.is_char_boundary(at) {
            at -= 1;
        }
        at
    };
    let (start, end) = (fit(selection.start), fit(selection.end));
    start.min(end)..start.max(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text` with `|` marking the caret, or `[` and `]` a selection.
    fn at(marked: &str) -> (String, Range<usize>) {
        if let Some(caret) = marked.find('|') {
            return (marked.replacen('|', "", 1), caret..caret);
        }
        let start = marked.find('[').expect("a selection");
        let end = marked.find(']').expect("its end") - 1;
        (marked.replacen('[', "", 1).replacen(']', "", 1), start..end)
    }

    fn shown(edit: &Edit) -> String {
        let mut text = edit.text.clone();
        if edit.selection.is_empty() {
            text.insert(edit.selection.start, '|');
        } else {
            text.insert(edit.selection.end, ']');
            text.insert(edit.selection.start, '[');
        }
        text
    }

    #[test]
    fn wrap_surrounds_the_selection_or_opens_a_pair_to_type_into() {
        let (text, selection) = at("[tide gate]");
        assert_eq!(shown(&wrap(&text, selection, "**", "**")), "**tide gate**|");
        let (text, selection) = at("a | b");
        assert_eq!(shown(&wrap(&text, selection, "*", "*")), "a *|* b");
    }

    #[test]
    fn a_link_wraps_its_words_and_waits_for_the_address() {
        let (text, selection) = at("[the docs]");
        assert_eq!(shown(&link(&text, selection)), "[the docs](|)");
        let (text, selection) = at("|");
        assert_eq!(shown(&link(&text, selection)), "[|]()");
    }

    #[test]
    fn a_prefix_goes_on_the_line_and_a_second_press_takes_it_off() {
        for prefix in ["# ", "- ", "1. ", "> "] {
            let (text, selection) = at("o|ne");
            let on = toggle_prefix(&text, selection, prefix);
            assert_eq!(shown(&on), format!("{prefix}o|ne"));
            let off = toggle_prefix(&on.text, on.selection, prefix);
            assert_eq!(shown(&off), "o|ne", "{prefix:?} again takes it off");
        }
    }

    #[test]
    fn a_prefix_goes_on_every_line_the_selection_touches() {
        let (text, selection) = at("intro\n[one\ntwo]\nend");
        assert_eq!(
            shown(&toggle_prefix(&text, selection, "- ")),
            "intro\n- [one\n- two]\nend"
        );
    }
}
