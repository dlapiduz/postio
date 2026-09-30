//! Wrapping a styled line to a width, for the reading pane.
//!
//! A message's lines are as wide as its paragraphs; the pane is as wide as
//! the terminal leaves it. Each line is broken at the last space that fits,
//! or mid-word when a word is wider than the pane, and a list item or a
//! quote keeps its indent on the lines it continues onto.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

/// `line` as the rows it takes at `width` columns; always at least one.
pub fn wrap(line: &Line<'static>, width: usize) -> Vec<Line<'static>> {
    let cells: Vec<(char, Style)> = line
        .spans
        .iter()
        .flat_map(|span| span.content.chars().map(move |c| (c, span.style)))
        .collect();
    let total: usize = cells.iter().map(|(c, _)| c.width().unwrap_or(0)).sum();
    if width == 0 || total <= width {
        return vec![line.clone()];
    }
    let indent = hang(&cells).min(width / 2);
    let mut rows: Vec<Vec<(char, Style)>> = Vec::new();
    let mut row: Vec<(char, Style)> = Vec::new();
    let mut used = 0;
    // Where in `row` the last space is, to break there.
    let mut space: Option<usize> = None;
    for &(c, style) in &cells {
        let w = c.width().unwrap_or(0);
        if used + w > width && !row.is_empty() {
            // Breaking at this very space carries nothing over.
            let break_at = if c == ' ' { Some(row.len()) } else { space };
            if c == ' ' {
                row.push((c, style));
            }
            let carried = match break_at {
                Some(at) => {
                    let rest = row.split_off(at + 1);
                    row.pop();
                    rest
                }
                None => Vec::new(),
            };
            rows.push(std::mem::take(&mut row));
            row = std::iter::repeat_n((' ', Style::default()), indent)
                .chain(carried)
                .collect();
            used = row.iter().map(|(c, _)| c.width().unwrap_or(0)).sum();
            space = None;
        }
        if c == ' ' && row.len() == indent && !rows.is_empty() {
            // A continuation does not start with the space it broke at.
            continue;
        }
        if c == ' ' {
            space = Some(row.len());
        }
        row.push((c, style));
        used += w;
    }
    rows.push(row);
    rows.into_iter()
        .map(|cells| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut text = String::new();
            let mut current: Option<Style> = None;
            for (c, style) in cells {
                if current.is_some_and(|now| now != style) {
                    spans.push(Span::styled(std::mem::take(&mut text), current.unwrap()));
                }
                current = Some(style);
                text.push(c);
            }
            if let Some(style) = current {
                spans.push(Span::styled(text, style));
            }
            Line::from(spans).style(line.style)
        })
        .collect()
}

/// How far a continuation is indented: past a list item's bullet or number,
/// or a quote's marker, so the text lines up under itself.
fn hang(cells: &[(char, Style)]) -> usize {
    let text: String = cells.iter().take(12).map(|(c, _)| *c).collect();
    let lead = text.len() - text.trim_start().len();
    let rest = &text[lead..];
    let marker = if rest.starts_with("- ") || rest.starts_with("* ") || rest.starts_with("> ") {
        2
    } else {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && rest[digits..].starts_with(". ") {
            digits + 2
        } else {
            0
        }
    };
    lead + marker
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line]) -> Vec<String> {
        lines.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn a_line_is_broken_at_the_last_space_that_fits() {
        let lines = wrap(&Line::raw("the quick brown fox jumps"), 11);
        assert_eq!(text(&lines), ["the quick", "brown fox", "jumps"]);
    }

    #[test]
    fn a_word_wider_than_the_pane_is_broken_inside_it() {
        let lines = wrap(&Line::raw("abcdefghij"), 4);
        assert_eq!(text(&lines), ["abcd", "efgh", "ij"]);
    }

    #[test]
    fn a_list_item_keeps_its_indent() {
        let lines = wrap(&Line::raw("- one two three four"), 10);
        assert_eq!(text(&lines), ["- one two", "  three", "  four"]);
    }

    #[test]
    fn styles_survive_the_break() {
        let line = Line::from(vec![
            Span::raw("plain "),
            Span::styled("bold words", Style::default().bold()),
        ]);
        let lines = wrap(&line, 8);
        assert_eq!(text(&lines), ["plain", "bold", "words"]);
        assert_eq!(lines[2].spans[0].style, Style::default().bold());
    }
}
