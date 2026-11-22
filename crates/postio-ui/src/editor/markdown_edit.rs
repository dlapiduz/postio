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

/// `~~` around the selection.
pub fn strikethrough(text: &str, selection: Range<usize>) -> Edit {
    wrap(text, selection, "~~", "~~")
}

/// A backtick run around the selection, longer than any run inside it.
pub fn inline_code(text: &str, selection: Range<usize>) -> Edit {
    let selection = clamp(text, selection);
    let longest = text[selection.clone()]
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest + 1);
    wrap(text, selection, &fence, &fence)
}

/// The `#`s a line's heading has, and the length of its prefix.
fn heading_prefix(line: &str) -> Option<(usize, usize)> {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    ((1..=6).contains(&hashes) && line[hashes..].starts_with(' ')).then_some((hashes, hashes + 1))
}

/// Make the selected lines a heading of `level` (1 to 6), or, when the first
/// already is one of that level, normal text again.
pub fn heading(text: &str, selection: Range<usize>, level: u8) -> Edit {
    let selection = clamp(text, selection);
    let level = usize::from(level.clamp(1, 6));
    let first = &text[line_start(text, selection.start)..];
    let off = heading_prefix(first).is_some_and(|(hashes, _)| hashes == level);
    edit_lines(text, selection, |line| {
        let words = heading_prefix(line).map_or(line, |(_, prefix)| &line[prefix..]);
        if off {
            words.to_owned()
        } else {
            format!("{} {words}", "#".repeat(level))
        }
    })
}

/// Take any heading off the selected lines.
pub fn normal_text(text: &str, selection: Range<usize>) -> Edit {
    edit_lines(text, clamp(text, selection), |line| {
        heading_prefix(line)
            .map_or(line, |(_, prefix)| &line[prefix..])
            .to_owned()
    })
}

/// Fence the selected lines as a code block, or, inside a fenced block,
/// take its fences away. With nothing on the line, an empty block with the
/// caret inside.
pub fn code_block(text: &str, selection: Range<usize>) -> Edit {
    let selection = clamp(text, selection);
    let first = line_start(text, selection.start);
    let last = line_end(text, selection.end);
    let is_fence = |line: &str| line.starts_with("```") || line.starts_with("~~~");
    // Inside a fence: the nearest fence lines above and below.
    let above = first.checked_sub(1).map(|end| line_start(text, end)..end);
    let below = (last < text.len()).then(|| last + 1..line_end(text, last + 1));
    if let (Some(above), Some(below)) = (above, below)
        && is_fence(&text[above.clone()])
        && is_fence(&text[below.clone()])
    {
        let out = format!(
            "{}{}{}",
            &text[..above.start],
            &text[first..last],
            &text[(below.end + 1).min(text.len())..]
        );
        let shift = first - above.start;
        return Edit {
            text: out,
            selection: selection.start - shift..selection.end - shift,
        };
    }
    let out = format!(
        "{}```\n{}\n```{}",
        &text[..first],
        &text[first..last],
        &text[last..]
    );
    Edit {
        text: out,
        selection: selection.start + 4..selection.end + 4,
    }
}

/// A line's list marker: its indent, and the marker with its space.
fn list_marker(line: &str) -> Option<(usize, usize)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = &line[indent..];
    if rest.starts_with("- ") || rest.starts_with("* ") || rest.starts_with("+ ") {
        return Some((indent, 2));
    }
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    let after = &rest[digits..];
    ((1..=9).contains(&digits) && (after.starts_with(". ") || after.starts_with(") ")))
        .then_some((indent, digits + 2))
}

/// The task box after a line's marker, if it has one: its offset in the line.
fn task_box(line: &str) -> Option<usize> {
    let (indent, marker) = list_marker(line)?;
    let at = indent + marker;
    let rest = &line[at..];
    (rest.starts_with("[ ] ") || rest.starts_with("[x] ") || rest.starts_with("[X] ")).then_some(at)
}

/// Box every selected list item as a task -- a line that is not an item
/// becomes one -- or, when the first already is a task, take the boxes off.
pub fn task_list(text: &str, selection: Range<usize>) -> Edit {
    let selection = clamp(text, selection);
    let off = task_box(&text[line_start(text, selection.start)..line_end(text, selection.start)])
        .is_some();
    edit_lines(text, selection, |line| {
        match (task_box(line), list_marker(line)) {
            (Some(at), _) if off => format!("{}{}", &line[..at], &line[at + 4..]),
            (Some(_), _) => line.to_owned(),
            (None, _) if off => line.to_owned(),
            (None, Some((indent, marker))) => {
                format!(
                    "{}[ ] {}",
                    &line[..indent + marker],
                    &line[indent + marker..]
                )
            }
            (None, None) => format!("- [ ] {line}"),
        }
    })
}

/// Tick or untick the box of every selected task.
pub fn toggle_task(text: &str, selection: Range<usize>) -> Edit {
    edit_lines(text, clamp(text, selection), |line| match task_box(line) {
        Some(at) => {
            let mark = if &line[at..at + 3] == "[ ]" {
                "[x]"
            } else {
                "[ ]"
            };
            format!("{}{mark}{}", &line[..at], &line[at + 3..])
        }
        None => line.to_owned(),
    })
}

/// Nest every selected list item under the item above it: its indent
/// becomes that item's content column.
pub fn indent(text: &str, selection: Range<usize>) -> Edit {
    let selection = clamp(text, selection);
    let first = line_start(text, selection.start);
    let parent = text[..first]
        .lines()
        .rev()
        .find_map(list_marker)
        .map(|(indent, marker)| indent + marker);
    edit_lines(text, selection, |line| match list_marker(line) {
        Some((current, _)) => {
            let to = parent
                .filter(|column| *column > current)
                .unwrap_or(current + 2);
            format!("{}{line}", " ".repeat(to - current))
        }
        None => line.to_owned(),
    })
}

/// Bring every selected list item out one level: to the indent of the item
/// it is nested under, or the margin.
pub fn outdent(text: &str, selection: Range<usize>) -> Edit {
    let selection = clamp(text, selection);
    let first = line_start(text, selection.start);
    let above: Vec<(usize, usize)> = text[..first].lines().filter_map(list_marker).collect();
    edit_lines(text, selection, |line| match list_marker(line) {
        Some((current, _)) if current > 0 => {
            let to = above
                .iter()
                .rev()
                .map(|(indent, _)| *indent)
                .find(|indent| *indent < current)
                .unwrap_or(0);
            line[current - to..].to_owned()
        }
        _ => line.to_owned(),
    })
}

/// A divider below the caret's line, as its own block, with the caret on
/// the line after it.
pub fn divider(text: &str, selection: Range<usize>) -> Edit {
    let end = line_end(text, clamp(text, selection).end);
    let rest = &text[end..];
    let head = format!("{}\n\n---\n", &text[..end]);
    let caret = head.len();
    let tail = if rest.is_empty() {
        String::new()
    } else {
        format!("\n{}", rest.trim_start_matches('\n'))
    };
    Edit::caret(format!("{head}{tail}"), caret)
}

/// The words of a new table's header cells (spec 014: a two-by-two
/// template to type over).
pub const TABLE_HEADERS: [&str; 2] = ["Column 1", "Column 2"];

/// A two-column table with one empty row at the caret, as its own block,
/// with the first header cell selected to type over.
pub fn table(text: &str, selection: Range<usize>) -> Edit {
    let at = clamp(text, selection).end;
    let start = line_start(text, at);
    let end = line_end(text, at);
    let line_empty = text[start..end].trim().is_empty();
    let (insert_at, lead) = if line_empty {
        (start, "")
    } else {
        (end, "\n\n")
    };
    let template = format!(
        "| {} | {} |\n|---|---|\n|  |  |",
        TABLE_HEADERS[0], TABLE_HEADERS[1]
    );
    let rest = &text[end..];
    let cell = insert_at + lead.len() + 2;
    Edit {
        text: format!("{}{lead}{template}{rest}", &text[..insert_at]),
        selection: cell..cell + TABLE_HEADERS[0].len(),
    }
}

/// The inline marks taken off the selection: emphasis, strikethrough and
/// code markers, and a link down to its words. An escaped character stays
/// as it was typed.
pub fn clear_formatting(text: &str, selection: Range<usize>) -> Edit {
    let selection = clamp(text, selection);
    let selection = if selection.is_empty() {
        line_start(text, selection.start)..line_end(text, selection.start)
    } else {
        selection
    };
    let inner = &text[selection.clone()];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        match c {
            '\\' => {
                out.push(c);
                if let Some((_, next)) = chars.next() {
                    out.push(next);
                }
            }
            '*' | '_' | '~' | '`' => {}
            ']' if inner[at + 1..].starts_with('(') => {
                // `](address)`: the address goes with the brackets.
                let close = inner[at..].find(')').map_or(inner.len(), |n| at + n);
                while chars.peek().is_some_and(|(next, _)| *next <= close) {
                    chars.next();
                }
            }
            '[' => {}
            other => out.push(other),
        }
    }
    let end = selection.start + out.len();
    Edit {
        text: format!(
            "{}{out}{}",
            &text[..selection.start],
            &text[selection.end..]
        ),
        selection: selection.start..end,
    }
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
        // Where the line first changed: an offset before it stays, one after
        // it moves with the words, and none lands inside what was added.
        let same = line
            .bytes()
            .zip(rewritten.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        let moved = |offset: usize| {
            let column = offset - at;
            let column = if column <= same {
                column
            } else {
                (column as isize + shift).max(same as isize) as usize
            };
            out.len() + column
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

    /// `text` with `|` marking the caret, or `«` and `»` a selection.
    fn at(marked: &str) -> (String, Range<usize>) {
        if let Some(caret) = marked.find('|') {
            return (marked.replacen('|', "", 1), caret..caret);
        }
        let start = marked.find('«').expect("a selection");
        let end = marked.find('»').expect("its end") - '«'.len_utf8();
        (marked.replacen('«', "", 1).replacen('»', "", 1), start..end)
    }

    fn shown(edit: &Edit) -> String {
        let mut text = edit.text.clone();
        if edit.selection.is_empty() {
            text.insert(edit.selection.start, '|');
        } else {
            text.insert(edit.selection.end, '»');
            text.insert(edit.selection.start, '«');
        }
        text
    }

    #[test]
    fn wrap_surrounds_the_selection_or_opens_a_pair_to_type_into() {
        let (text, selection) = at("«tide gate»");
        assert_eq!(shown(&wrap(&text, selection, "**", "**")), "**tide gate**|");
        let (text, selection) = at("a | b");
        assert_eq!(shown(&wrap(&text, selection, "*", "*")), "a *|* b");
    }

    #[test]
    fn a_link_wraps_its_words_and_waits_for_the_address() {
        let (text, selection) = at("«the docs»");
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
        let (text, selection) = at("intro\n«one\ntwo»\nend");
        assert_eq!(
            shown(&toggle_prefix(&text, selection, "- ")),
            "intro\n«- one\n- two»\nend"
        );
    }

    // -- spec 014's now-phase commands (T027) -----------------------------

    fn run(f: impl Fn(&str, Range<usize>) -> Edit, marked: &str) -> String {
        let (text, selection) = at(marked);
        shown(&f(&text, selection))
    }

    #[test]
    fn strikethrough_and_inline_code_wrap() {
        assert_eq!(run(strikethrough, "a «b» c"), "a ~~b~~| c");
        assert_eq!(run(inline_code, "a «b» c"), "a `b`| c");
        assert_eq!(
            run(inline_code, "«a`b»"),
            "``a`b``|",
            "a longer run than any inside"
        );
        assert_eq!(run(inline_code, "|"), "`|`");
    }

    #[test]
    fn each_heading_level_sets_its_hashes_and_again_takes_them_off() {
        for level in 1..=6u8 {
            let hashes = "#".repeat(usize::from(level));
            assert_eq!(
                run(|t, s| heading(t, s, level), "Pl|an"),
                format!("{hashes} Pl|an")
            );
            let set = format!("{hashes} Pl|an");
            assert_eq!(run(|t, s| heading(t, s, level), &set), "Pl|an");
        }
        assert_eq!(
            run(|t, s| heading(t, s, 2), "# Pl|an"),
            "## Pl|an",
            "a level replaces another"
        );
        assert_eq!(run(normal_text, "### Pl|an"), "Pl|an");
        assert_eq!(run(normal_text, "Pl|an"), "Pl|an");
    }

    #[test]
    fn a_code_block_fences_the_lines_or_opens_an_empty_one() {
        assert_eq!(
            run(code_block, "«let x = 1;\nlet y = 2;»"),
            "```\n«let x = 1;\nlet y = 2;»\n```"
        );
        assert_eq!(run(code_block, "|"), "```\n|\n```");
        assert_eq!(
            run(code_block, "```\nlet |x\n```"),
            "let |x",
            "inside a fence, the fence goes"
        );
    }

    #[test]
    fn a_task_list_boxes_its_items_and_again_takes_the_boxes_off() {
        assert_eq!(run(task_list, "«- one\n- two»"), "«- [ ] one\n- [ ] two»");
        assert_eq!(run(task_list, "plain|"), "- [ ] plain|");
        assert_eq!(run(task_list, "- [x] do|ne"), "- do|ne");
        assert_eq!(run(task_list, "  1. nu|m"), "  1. [ ] nu|m");
    }

    #[test]
    fn toggle_task_ticks_and_unticks_the_box() {
        assert_eq!(run(toggle_task, "- [ ] o|pen"), "- [x] o|pen");
        assert_eq!(run(toggle_task, "- [X] d|one"), "- [ ] d|one");
        assert_eq!(run(toggle_task, "- not a ta|sk"), "- not a ta|sk");
    }

    #[test]
    fn indent_nests_an_item_under_the_one_above_and_outdent_undoes_it() {
        assert_eq!(run(indent, "- a\n- b|"), "- a\n  - b|");
        assert_eq!(run(indent, "1. a\n2. b|"), "1. a\n   2. b|");
        assert_eq!(run(outdent, "- a\n  - b|"), "- a\n- b|");
        assert_eq!(run(outdent, "- a|"), "- a|", "nothing to come out of");
        assert_eq!(run(indent, "not a l|ist"), "not a l|ist");
    }

    #[test]
    fn a_divider_goes_below_the_current_block() {
        assert_eq!(run(divider, "abo|ve"), "above\n\n---\n|");
        assert_eq!(run(divider, "abo|ve\n\nbelow"), "above\n\n---\n|\nbelow");
    }

    #[test]
    fn a_table_is_a_two_by_two_template_with_its_first_cell_selected() {
        assert_eq!(
            run(table, "|"),
            "| «Column 1» | Column 2 |\n|---|---|\n|  |  |"
        );
        assert_eq!(
            run(table, "words|"),
            "words\n\n| «Column 1» | Column 2 |\n|---|---|\n|  |  |"
        );
    }

    #[test]
    fn clear_formatting_takes_the_marks_off_the_selection() {
        assert_eq!(
            run(
                clear_formatting,
                "«**bold** and *it* ~~s~~ `c` [link](https://example.com)»"
            ),
            "«bold and it s c link»"
        );
        assert_eq!(
            run(clear_formatting, "«a \\*literal\\*»"),
            "«a \\*literal\\*»",
            "an escape is kept"
        );
    }
}
