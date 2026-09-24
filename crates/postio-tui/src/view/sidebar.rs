//! The sidebar pane.
//!
//! PLATE 1b's sidebar in a terminal: each account headed by its address,
//! small and letter-spaced; its folders indented under it; the open one on
//! the raised surface with a bar at its left edge; unread counts in the
//! accent at the right; and the sync state at the foot. A rule at the right
//! edge keeps it apart from the list.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use crate::sidebar;
use crate::theme::{Role, Theme};
use crate::view::fit;
use crate::view::hit::{Hits, Target};

/// Columns a folder's name is indented by.
const INDENT: usize = 3;

/// Draw `lines` into `area`, marking `cursor`; `focused` when the keyboard is
/// in the sidebar. `sync` is the account's two sync lines, for the foot.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    lines: &[sidebar::Line],
    cursor: usize,
    focused: bool,
    sync: Option<(String, String)>,
    theme: &Theme,
    hits: &mut Hits,
) {
    if area.width < 2 {
        return;
    }
    // The rule, then everything else inside it.
    for y in area.y..area.y + area.height {
        frame.render_widget(
            Line::styled("│", theme.style(Role::Dim)),
            Rect::new(area.x + area.width - 1, y, 1, 1),
        );
    }
    let area = Rect::new(area.x, area.y, area.width - 1, area.height);
    let width = usize::from(area.width);

    // The foot takes two rows and a gap, when there is room for the folders
    // as well.
    let foot = if sync.is_some() && area.height >= 8 {
        3
    } else {
        0
    };
    if let Some((state, detail)) = sync.filter(|_| foot > 0) {
        for (offset, text) in [state, detail].iter().enumerate() {
            let y = area.y + area.height - 2 + u16::try_from(offset).unwrap_or(0);
            frame.render_widget(
                Line::styled(fit(text, width.saturating_sub(2)), theme.style(Role::Dim)),
                Rect::new(area.x + 2, y, area.width.saturating_sub(2), 1),
            );
        }
    }

    // Rows as drawn: a blank before every heading, the first included, so
    // the pane does not start hard under the top bar.
    let mut rows: Vec<Option<usize>> = Vec::with_capacity(lines.len() + 4);
    for (index, line) in lines.iter().enumerate() {
        if line.heading {
            rows.push(None);
        }
        rows.push(Some(index));
    }
    // Keep the cursor in view in a long sidebar.
    let height = usize::from(area.height.saturating_sub(foot));
    let at = rows
        .iter()
        .position(|row| *row == Some(cursor))
        .unwrap_or(0);
    let top = at.saturating_sub(height.saturating_sub(1));
    for (offset, row) in rows.iter().skip(top).take(height).enumerate() {
        let Some(index) = *row else {
            continue;
        };
        let line = &lines[index];
        let y = area.y + u16::try_from(offset).unwrap_or(u16::MAX);
        let rect = Rect::new(area.x, y, area.width, 1);
        let here = index == cursor;
        if here {
            frame
                .buffer_mut()
                .set_style(rect, theme.style(Role::Surface));
        }
        frame.render_widget(one(line, here, focused, width, theme), rect);
        if !line.heading {
            hits.add(rect, Target::Sidebar(index));
        }
    }
}

fn one<'a>(
    line: &sidebar::Line,
    here: bool,
    focused: bool,
    width: usize,
    theme: &Theme,
) -> Line<'a> {
    if line.heading {
        return Line::from(vec![
            Span::raw("  "),
            Span::styled(
                heading(line.label.as_str(), width.saturating_sub(3)),
                theme.style(Role::Dim),
            ),
        ]);
    }
    let bar = if here { "▌" } else { " " };
    let count = line
        .count
        .map(|count| count.to_string())
        .unwrap_or_default();
    let room = width.saturating_sub(INDENT + count.len() + 2);
    let label = fit(line.label.as_str(), room);
    let pad = room.saturating_sub(unicode_width::UnicodeWidthStr::width(label.as_str()));
    let style = match (here, focused) {
        (true, true) => theme.style(Role::Text).add_modifier(Modifier::BOLD),
        _ => theme.style(Role::Text),
    };
    Line::from(vec![
        Span::styled(bar, theme.style(Role::Focus)),
        Span::raw(" ".repeat(INDENT - 1)),
        Span::styled(label, style),
        Span::raw(" ".repeat(pad + 1)),
        Span::styled(count, theme.style(Role::Accent)),
    ])
}

/// A heading as the canvas sets it: capitals, a space between letters when
/// that fits in `width`, and just capitals when it does not.
fn heading(text: &str, width: usize) -> String {
    let capitals = text.to_uppercase();
    let spaced: String = capitals
        .chars()
        .flat_map(|c| [c, ' '])
        .collect::<String>()
        .trim_end()
        .to_owned();
    if unicode_width::UnicodeWidthStr::width(spaced.as_str()) <= width {
        spaced
    } else {
        fit(&capitals, width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_heading_is_spaced_out_only_where_it_fits() {
        assert_eq!(
            heading("ada@example.com", 40),
            "A D A @ E X A M P L E . C O M"
        );
        assert_eq!(heading("ada@example.com", 20), "ADA@EXAMPLE.COM");
    }
}
