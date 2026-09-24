//! The search field, in the top bar (US4).

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use crate::theme::{Role, Theme};
use crate::view::fit;

/// Draw the field holding `query` in the top row of `area`, with `readout`
/// at its right.
///
/// The operators in the query are drawn as `chips` -- marked, in place -- so
/// the query language is read back as it is typed, as the desktop's bar does
/// (`postio_ui::search`); one still waiting for its value is marked without
/// weight, in progress rather than wrong. With `focused`, the terminal's
/// cursor is at `caret`.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    query: &str,
    chips: &[postio_ui::search::Chip],
    caret: usize,
    readout: Option<&str>,
    focused: bool,
    theme: &Theme,
) {
    if area.height == 0 {
        return;
    }
    let row = Rect::new(area.x, area.y, area.width, 1);
    let parsed = postio_search::parse(query, chrono::Local::now().date_naive());
    let mut spans = vec![Span::styled("/ ", theme.style(Role::Accent))];
    let mut at = 0;
    for (index, token) in parsed.tokens().iter().enumerate() {
        let (start, end) = (token.span.start, token.span.end);
        if start > at {
            spans.push(Span::styled(
                query[at..start].to_owned(),
                theme.style(Role::Text),
            ));
        }
        let style = match chips.iter().find(|chip| chip.index == index) {
            Some(chip) if chip.complete => theme.style(Role::Accent).add_modifier(Modifier::BOLD),
            Some(_) => theme.style(Role::Accent),
            None => theme.style(Role::Text),
        };
        spans.push(Span::styled(query[start..end].to_owned(), style));
        at = end;
    }
    if at < query.len() {
        spans.push(Span::styled(
            query[at..].to_owned(),
            theme.style(Role::Text),
        ));
    }
    frame.render_widget(Line::from(spans), row);
    if let Some(readout) = readout {
        let width = unicode_width::UnicodeWidthStr::width(readout);
        let typed = unicode_width::UnicodeWidthStr::width(query) + 4;
        if width + typed <= usize::from(area.width) {
            let x = area.x + area.width - u16::try_from(width).unwrap_or(u16::MAX);
            frame.render_widget(
                Line::styled(fit(readout, width), theme.style(Role::Dim)),
                Rect::new(x, area.y, u16::try_from(width).unwrap_or(0), 1),
            );
        }
    }
    if focused {
        let column = 2 + unicode_width::UnicodeWidthStr::width(
            query
                .char_indices()
                .nth(caret)
                .map_or(query, |(at, _)| &query[..at]),
        );
        frame.set_cursor_position(Position::new(
            area.x
                + u16::try_from(column)
                    .unwrap_or(u16::MAX)
                    .min(area.width.saturating_sub(1)),
            area.y,
        ));
    }
}
