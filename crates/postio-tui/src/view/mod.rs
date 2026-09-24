//! Drawing: the app's state onto a ratatui frame.
//!
//! Nothing here decides anything about mail; it draws what `App` holds.

pub mod list;

use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;

use crate::app::App;
use crate::layout::Shown;
use crate::theme::{Role, Theme};

/// Draw the whole screen.
pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, now: DateTime<Local>) {
    let area = frame.area();
    match app.shown() {
        Shown::TooSmall { needs } => {
            let sentence = format!("Terminal too small: needs {}×{}", needs.0, needs.1);
            let line = Line::styled(
                fit(&sentence, usize::from(area.width)),
                theme.style(Role::Warning),
            );
            frame.render_widget(line, Rect::new(area.x, area.y, area.width, 1));
        }
        Shown::Panes(_) => {
            let [list, status] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
            list::draw(frame, list, &app.visible(), theme, now);
            let words = match (app.notice(), app.total()) {
                (Some(notice), _) => notice.to_owned(),
                (None, 1) => "1 conversation".to_owned(),
                (None, total) => format!("{total} conversations"),
            };
            let words = fit(&words, usize::from(status.width));
            frame.render_widget(Line::styled(words, theme.style(Role::Dim)), status);
        }
    }
}

/// `text`, cut to at most `width` terminal columns, ending in `…` when cut.
///
/// By display width, not characters or bytes: a CJK character is two
/// columns and a combining mark none, and a list that counted characters
/// would push its date column out of line on the first Japanese subject.
pub fn fit(text: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    let full: usize = text.chars().map(|c| c.width().unwrap_or(0)).sum();
    if full <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > width - 1 {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}
