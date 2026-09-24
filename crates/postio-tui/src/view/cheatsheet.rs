//! The cheat sheet, over the screen (US4).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear};

use crate::app::SheetSection;
use crate::theme::{Role, Theme};
use crate::view::fit;

/// How wide one column of the sheet is.
const COLUMN: u16 = 44;

/// Draw `sections` over `area`, in as many columns as it takes.
pub fn draw(frame: &mut Frame, area: Rect, sections: &[SheetSection], theme: &Theme) {
    let outer = Rect::new(
        area.x + 1,
        area.y,
        area.width.saturating_sub(2),
        area.height.saturating_sub(1),
    );
    if outer.width < COLUMN || outer.height < 4 {
        return;
    }
    frame.render_widget(Clear, outer);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.style(Role::Dim))
            .title(Line::from(vec![
                Span::styled("─ ", theme.style(Role::Dim)),
                Span::styled("Keys", theme.style(Role::Accent)),
                Span::styled(" — any key closes ", theme.style(Role::Dim)),
            ])),
        outer,
    );
    let inner = Rect::new(outer.x + 2, outer.y + 1, outer.width - 4, outer.height - 2);
    let mut lines: Vec<Line> = Vec::new();
    for (title, rows) in sections {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        lines.push(Line::styled(
            *title,
            theme.style(Role::Accent).add_modifier(Modifier::BOLD),
        ));
        for (name, key) in rows {
            let room = usize::from(COLUMN - 2).saturating_sub(key.len() + 1);
            let name = fit(name, room);
            let gap = usize::from(COLUMN - 2)
                .saturating_sub(unicode_width::UnicodeWidthStr::width(name.as_str()) + key.len());
            lines.push(Line::from(vec![
                Span::styled(name, theme.style(Role::Text)),
                Span::raw(" ".repeat(gap)),
                Span::styled(key.clone(), theme.style(Role::Accent)),
            ]));
        }
    }
    let height = usize::from(inner.height);
    for (index, line) in lines.into_iter().enumerate() {
        let column = u16::try_from(index / height).unwrap_or(u16::MAX);
        let x = inner.x + column.saturating_mul(COLUMN);
        if x + COLUMN > inner.x + inner.width + 2 {
            break;
        }
        let y = inner.y + u16::try_from(index % height).unwrap_or(u16::MAX);
        frame.render_widget(line, Rect::new(x, y, COLUMN - 2, 1));
    }
}
