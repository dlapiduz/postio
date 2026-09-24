//! The reading pane.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::Line;

use crate::reader::Rendered;
use crate::row::Row;
use crate::theme::{Role, Theme};
use crate::view::fit;

/// Draw `row`'s header and `rendered` into `area`, from line `top`.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    row: Option<&Row>,
    rendered: &Rendered,
    top: usize,
    theme: &Theme,
) {
    // A divider and a space between the list and the reader: without them the
    // list's date runs straight into the subject.
    for y in area.y..area.y + area.height {
        frame.render_widget(
            Line::styled("│", theme.style(Role::Dim)),
            Rect::new(area.x, y, 1, 1),
        );
    }
    let area = Rect::new(
        area.x + 2,
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    let width = usize::from(area.width);
    let mut lines: Vec<Line> = Vec::new();
    if let Some(row) = row {
        lines.push(Line::styled(
            fit(row.subject.as_str(), width),
            theme.style(Role::Text).add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::styled(
            fit(row.from.as_str(), width),
            theme.style(Role::Dim),
        ));
        lines.push(Line::default());
    }
    lines.extend(rendered.lines().into_iter().skip(top));
    for (offset, line) in lines.into_iter().take(usize::from(area.height)).enumerate() {
        let y = area.y + u16::try_from(offset).unwrap_or(u16::MAX);
        frame.render_widget(line, Rect::new(area.x, y, area.width, 1));
    }
}
