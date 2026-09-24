//! The sidebar pane.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use crate::sidebar;
use crate::theme::{Role, Theme};
use crate::view::fit;

/// Draw `lines` into `area`, marking `cursor`; `focused` when the keyboard is
/// in the sidebar.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    lines: &[sidebar::Line],
    cursor: usize,
    focused: bool,
    theme: &Theme,
) {
    let width = usize::from(area.width);
    // Keep the cursor in view in a long sidebar.
    let height = usize::from(area.height);
    let top = cursor.saturating_sub(height.saturating_sub(1));
    for (offset, (index, line)) in lines.iter().enumerate().skip(top).take(height).enumerate() {
        let y = area.y + u16::try_from(offset).unwrap_or(u16::MAX);
        let row = Rect::new(area.x, y, area.width, 1);
        frame.render_widget(one(line, index == cursor, focused, width, theme), row);
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
        return Line::styled(
            fit(line.label.as_str(), width),
            theme.style(Role::Dim).add_modifier(Modifier::BOLD),
        );
    }
    let mark = if here { "› " } else { "  " };
    let count = line
        .count
        .map(|count| count.to_string())
        .unwrap_or_default();
    let room = width.saturating_sub(2 + count.len() + 1);
    let label = fit(line.label.as_str(), room);
    let pad = room.saturating_sub(unicode_width::UnicodeWidthStr::width(label.as_str()));
    let style = if here && focused {
        theme.style(Role::Selection)
    } else {
        theme.style(Role::Text)
    };
    Line::from(vec![
        Span::styled(mark, theme.style(Role::Focus)),
        Span::styled(label, style),
        Span::raw(" ".repeat(pad + 1)),
        Span::styled(count, theme.style(Role::Dim)),
    ])
}
