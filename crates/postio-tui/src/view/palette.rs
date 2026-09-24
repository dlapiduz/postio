//! The command palette, and the finder's other modes, over the screen (US4).

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear};

use crate::app::PaletteView;
use crate::theme::{Role, Theme};
use crate::view::fit;

/// How wide the palette is, at most.
const WIDTH: u16 = 72;

/// Draw `palette` centred near the top of `area`.
pub fn draw(frame: &mut Frame, area: Rect, palette: &PaletteView, theme: &Theme) {
    let width = WIDTH.min(area.width.saturating_sub(4));
    let rows = u16::try_from(palette.rows.len()).unwrap_or(u16::MAX);
    let height = (rows + 3).min(area.height.saturating_sub(2));
    if width < 20 || height < 4 {
        return;
    }
    let outer = Rect::new(area.x + (area.width - width) / 2, area.y + 1, width, height);
    frame.render_widget(Clear, outer);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme.style(Role::Dim)),
        outer,
    );
    let inner = Rect::new(outer.x + 1, outer.y + 1, outer.width - 2, outer.height - 2);
    let prompt = format!("{} {}", palette.marker, palette.query);
    frame.render_widget(
        Line::styled(
            fit(&prompt, usize::from(inner.width)),
            theme.style(Role::Text),
        ),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    let caret = unicode_width::UnicodeWidthStr::width(prompt.as_str());
    frame.set_cursor_position(Position::new(
        inner.x
            + u16::try_from(caret)
                .unwrap_or(u16::MAX)
                .min(inner.width - 1),
        inner.y,
    ));

    let shown = usize::from(inner.height.saturating_sub(1));
    // Keep the chosen row in view.
    let first = palette.selected.saturating_sub(shown.saturating_sub(1));
    for (offset, (index, row)) in palette
        .rows
        .iter()
        .enumerate()
        .skip(first)
        .take(shown)
        .enumerate()
    {
        let y = inner.y + 1 + u16::try_from(offset).unwrap_or(u16::MAX);
        let chosen = index == palette.selected;
        let base = if chosen {
            theme.style(Role::Selection)
        } else {
            theme.style(Role::Text)
        };
        let chord = row.chord.as_deref().unwrap_or("");
        let room = usize::from(inner.width)
            .saturating_sub(unicode_width::UnicodeWidthStr::width(chord) + 3);
        let title = fit(&row.title, room);
        // The letters the query matched, marked.
        let mut spans = vec![Span::styled(if chosen { "› " } else { "  " }, base)];
        for (at, c) in title.char_indices() {
            let style = if row.positions.contains(&at) {
                base.add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                base
            };
            spans.push(Span::styled(c.to_string(), style));
        }
        let used = 2 + unicode_width::UnicodeWidthStr::width(title.as_str());
        let gap = usize::from(inner.width).saturating_sub(used + chord.len());
        spans.push(Span::styled(" ".repeat(gap), base));
        spans.push(Span::styled(chord.to_owned(), theme.style(Role::Dim)));
        frame.render_widget(Line::from(spans), Rect::new(inner.x, y, inner.width, 1));
    }
}
