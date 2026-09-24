//! The first run, drawn: the desktop's three steps, in a terminal (US7).

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use crate::first_run::{Field, FirstRun};
use crate::theme::{Role, Theme};
use crate::view::fit;
use postio_ui::onboarding::{Status, SyncWindow};
use postio_ui::terminal::SafeText;

/// How wide the form is, at most.
const WIDTH: u16 = 72;

/// The label column.
const LABEL: usize = 12;

/// Draw `run` centred in `area`.
pub fn draw(frame: &mut Frame, area: Rect, run: &FirstRun, theme: &Theme) {
    let width = WIDTH.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let columns = usize::from(width);
    let mut lines: Vec<Line> = Vec::new();
    let mut cursor: Option<(u16, u16)> = None;
    let status = run.status();

    lines.push(Line::from(vec![
        Span::styled(
            "Add your first account",
            theme.style(Role::Text).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("   {}", postio_ui::onboarding::step_of(status)),
            theme.style(Role::Dim),
        ),
    ]));
    lines.push(Line::default());

    if *status == Status::SyncWindow {
        lines.push(Line::styled(
            "How far back should the first sync reach? A number picks.",
            theme.style(Role::Text),
        ));
        lines.push(Line::default());
        for (index, window) in SyncWindow::ALL.iter().enumerate() {
            lines.push(Line::from(vec![
                Span::styled(format!("{} ", index + 1), theme.style(Role::Accent)),
                Span::styled(format!("{:<16}", window.label()), theme.style(Role::Text)),
                Span::styled(window.estimate(), theme.style(Role::Dim)),
            ]));
        }
    } else {
        for field in run.fields() {
            let label = match field {
                Field::Address => "Address",
                Field::Name => "Name",
                Field::Password => "Password",
                Field::Incoming => "Incoming",
                Field::Outgoing => "Outgoing",
            };
            let here = run.field() == field && !status.is_busy();
            let style = if here {
                theme.style(Role::Accent).add_modifier(Modifier::BOLD)
            } else {
                theme.style(Role::Dim)
            };
            // Typed by the person, but a paste can carry anything.
            let value = SafeText::new(&run.value(field));
            if here {
                let row = u16::try_from(lines.len()).unwrap_or(u16::MAX);
                let column = LABEL + unicode_width::UnicodeWidthStr::width(value.as_str());
                cursor = Some((
                    u16::try_from(column)
                        .unwrap_or(u16::MAX)
                        .min(width.saturating_sub(1)),
                    row,
                ));
            }
            lines.push(Line::from(vec![
                Span::styled(format!("{label:<LABEL$}"), style),
                Span::styled(
                    fit(value.as_str(), columns.saturating_sub(LABEL)),
                    theme.style(Role::Text),
                ),
            ]));
        }
        if let Some(settings) = run.settings().filter(|_| !run.manual()) {
            lines.push(Line::default());
            // Where the settings came from: a provider's own name, or how
            // they were found.
            lines.push(Line::styled(
                fit(&format!("Found by {}", settings.source), columns),
                theme.style(Role::Dim),
            ));
            lines.push(Line::styled(
                fit(&format!("  in   {}", settings.imap.line()), columns),
                theme.style(Role::Text),
            ));
            lines.push(Line::styled(
                fit(&format!("  out  {}", settings.smtp.line()), columns),
                theme.style(Role::Text),
            ));
            if let Some(note) = &settings.note {
                lines.push(Line::styled(
                    fit(SafeText::new(note).as_str(), columns),
                    theme.style(Role::Warning),
                ));
            }
        }
        lines.push(Line::default());
        let (sentence, role) = match status {
            Status::Probing => ("Looking up the servers…".to_owned(), Role::Dim),
            Status::Connecting => ("Signing in…".to_owned(), Role::Dim),
            Status::Manual { .. } => (
                "Nothing was found for this address: type its servers as host:port.".to_owned(),
                Role::Warning,
            ),
            other => match other.message() {
                Some(message) => (message.to_owned(), Role::Error),
                None => (
                    "Enter moves on. Tab changes field. Ctrl+Q quits.".to_owned(),
                    Role::Dim,
                ),
            },
        };
        for part in sentence.split('\n') {
            lines.push(Line::styled(fit(part, columns), theme.style(role)));
        }
    }

    let height = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let y = area.y + area.height.saturating_sub(height) / 3;
    for (offset, line) in lines.into_iter().enumerate() {
        let row = y + u16::try_from(offset).unwrap_or(u16::MAX);
        if row >= area.y + area.height {
            break;
        }
        frame.render_widget(line, Rect::new(x, row, width, 1));
    }
    if let Some((column, row)) = cursor {
        frame.set_cursor_position(Position::new(x + column, y + row));
    }
}
