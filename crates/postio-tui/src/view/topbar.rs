//! The bar across the top: the search field, and the keys worth knowing.
//!
//! The desktop's header in a terminal row (PLATE 1b): a field on the left
//! that is the search once it is open, and on the right the keys for the
//! cheat sheet and for writing, read from the keymap in force -- a rebound
//! key is hinted as rebound.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

use crate::app::{App, Focus};
use crate::theme::{Role, Theme};
use crate::view::search;

/// The field's width when there is room: about the desktop's share.
const FIELD: u16 = 60;

/// Draw the bar into `area`, one row.
pub fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    use postio_core::CommandId;
    let mut hints: Vec<Span> = Vec::new();
    for (command, word) in [
        (CommandId::CheatSheet, "keys"),
        (CommandId::Compose, "compose"),
    ] {
        let Some(key) = app.hint(command) else {
            continue;
        };
        if !hints.is_empty() {
            hints.push(Span::raw("  "));
        }
        hints.push(Span::styled(key, theme.style(Role::Accent)));
        hints.push(Span::styled(format!(" {word}"), theme.style(Role::Dim)));
    }
    let hinted = u16::try_from(Line::from(hints.clone()).width()).unwrap_or(u16::MAX);
    // The hints give way to the field on a narrow terminal, never the
    // other way round.
    let room = area.width.saturating_sub(2);
    let show_hints = hinted + 4 + 24 <= room;
    let beside = if show_hints { hinted + 3 } else { 0 };
    let field_width = room
        .saturating_sub(beside)
        .min(FIELD.max(area.width * 2 / 5));
    let field = Rect::new(area.x + 1, area.y, field_width, 1);
    frame
        .buffer_mut()
        .set_style(field, theme.style(Role::Surface));
    let inner = Rect::new(field.x + 1, field.y, field.width.saturating_sub(2), 1);
    match app.search_query() {
        Some(query) => search::draw(
            frame,
            inner,
            query,
            &app.search_chips(),
            app.search_caret(),
            app.search_readout().as_deref(),
            app.focus() == Focus::Search,
            theme,
        ),
        None => {
            frame.render_widget(
                Line::styled("Search all mail", theme.style(Role::Dim)),
                inner,
            );
            if let Some(key) = app.hint(CommandId::Search) {
                let width = u16::try_from(unicode_width::UnicodeWidthStr::width(key.as_str()))
                    .unwrap_or(u16::MAX);
                if width + 17 <= inner.width {
                    frame.render_widget(
                        Line::styled(key, theme.style(Role::Accent)),
                        Rect::new(inner.x + inner.width - width, inner.y, width, 1),
                    );
                }
            }
        }
    }
    if show_hints {
        let x = area.x + area.width - hinted - 1;
        frame.render_widget(Line::from(hints), Rect::new(x, area.y, hinted, 1));
    }
}
