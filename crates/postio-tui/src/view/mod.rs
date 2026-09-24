//! Drawing: the app's state onto a ratatui frame.
//!
//! Nothing here decides anything about mail; it draws what `App` holds.

pub mod list;
pub mod sidebar;

use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;

use crate::app::{App, Focus};
use crate::layout::{Pane, Shown};

/// The sidebar's width in columns.
const SIDEBAR: u16 = 26;
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
        Shown::Panes(panes) => {
            let [body, status] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
            let widths: Vec<Constraint> = panes
                .iter()
                .map(|pane| match pane {
                    Pane::Sidebar => Constraint::Length(SIDEBAR),
                    Pane::List => Constraint::Min(40),
                    Pane::Reader => Constraint::Percentage(45),
                })
                .collect();
            let areas = Layout::horizontal(widths).split(body);
            for (pane, area) in panes.iter().zip(areas.iter()) {
                match pane {
                    Pane::Sidebar => {
                        let (lines, cursor) = app.sidebar();
                        sidebar::draw(
                            frame,
                            *area,
                            lines,
                            cursor,
                            app.focus() == Focus::Sidebar,
                            theme,
                        );
                    }
                    Pane::List => list::draw(frame, *area, &app.visible(), theme, now),
                    Pane::Reader => {
                        // The reader arrives with User Story 2.
                        let quiet = Line::styled("", theme.style(Role::Dim));
                        frame.render_widget(quiet, *area);
                    }
                }
            }
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

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::app::{Input, update};
    use crate::caps::{Background, Colour};
    use crate::input::Keys;

    fn screen(width: u16, height: u16, app: &App) -> String {
        let theme = Theme::new(Colour::None, Background::Unknown, &Default::default()).0;
        let now = chrono::Local
            .with_ymd_and_hms(2026, 9, 23, 12, 0, 0)
            .unwrap();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| draw(frame, app, &theme, now))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn with_sidebar(size: (u16, u16)) -> App {
        use postio_model::mailbox::{Mailbox, MailboxRole};
        let keys = Keys::new(&postio_core::Keymap::resolve(&Default::default())).0;
        let mut app = App::new(size, keys);
        let mut account = postio_model::Account::new(
            "ada",
            postio_model::EmailAddress::new(None::<String>, "ada@example.com"),
        );
        account.id = postio_model::AccountId::new(1);
        account.enabled = true;
        let mut inbox = Mailbox::new(account.id, "INBOX", None);
        inbox.id = postio_model::MailboxId::new(1);
        inbox.role = MailboxRole::Inbox;
        inbox.selectable = true;
        inbox.counts.unread = 4;
        update(
            &mut app,
            Input::Sidebar(crate::sidebar::Contents {
                accounts: vec![account],
                folders: vec![inbox],
                counts: Vec::new(),
                saved: vec!["Unread from Ada".into()],
            }),
        );
        app
    }

    #[test]
    fn a_wide_terminal_draws_the_sidebar_beside_the_list() {
        let app = with_sidebar((160, 12));
        let screen = screen(160, 12, &app);
        for wanted in [
            "Inbox",
            "Flagged",
            "Snoozed",
            "Saved searches",
            "Unread from Ada",
        ] {
            assert!(screen.contains(wanted), "{wanted} missing:\n{screen}");
        }
        assert!(
            screen
                .lines()
                .any(|line| line.contains("Inbox") && line.contains('4')),
            "{screen}"
        );
    }

    #[test]
    fn a_narrower_terminal_leaves_the_sidebar_out() {
        let app = with_sidebar((100, 12));
        let screen = screen(100, 12, &app);
        assert!(!screen.contains("Snoozed"), "{screen}");
    }
}
