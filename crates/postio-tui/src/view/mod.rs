//! Drawing: the app's state onto a ratatui frame.
//!
//! Nothing here decides anything about mail; it draws what `App` holds.

pub mod composer;
pub mod list;
pub mod reader;
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
                        if let Some(writing) = app.composer() {
                            composer::draw(
                                frame,
                                *area,
                                writing,
                                app.focus() == Focus::Composer
                                    && app.scheduling().is_none()
                                    && app.path_prompt().is_none(),
                                theme,
                            );
                            if let Some(times) = app.scheduling() {
                                composer::draw_schedule(frame, *area, times, theme, now);
                            }
                            if let Some(typed) = app.path_prompt() {
                                composer::draw_path_prompt(frame, *area, typed, theme);
                            }
                        } else if let Some(reading) = app.reading() {
                            reader::draw(
                                frame,
                                *area,
                                app.row(reading.row),
                                reading,
                                app.reader_top(),
                                theme,
                                now,
                            );
                        }
                    }
                }
            }
            let count = match app.total() {
                1 => "1 conversation".to_owned(),
                total => format!("{total} conversations"),
            };
            let words = match (app.notice(), app.sync_line()) {
                (Some(notice), _) => notice.to_owned(),
                (None, Some(sync)) => format!("{sync} · {count}"),
                (None, None) => count,
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
    fn the_composer_draws_its_fields_and_body_in_the_reading_pane() {
        let mut app = with_sidebar((160, 16));
        let mut draft = postio_model::Draft::new(postio_model::AccountId::new(1));
        draft.to = vec![postio_model::EmailAddress::new(
            None::<String>,
            "grace@example.net",
        )];
        draft.subject = "Tide gate".into();
        draft.body_markdown = Some("Some **bold** words".into());
        app.compose(draft);

        let screen = screen(160, 16, &app);
        for wanted in [
            "To",
            "grace@example.net",
            "Subject",
            "Tide gate",
            "Some **bold** words",
        ] {
            assert!(screen.contains(wanted), "{wanted} missing:\n{screen}");
        }
        assert!(!screen.contains("Cc"), "Cc is on demand:\n{screen}");
    }

    #[test]
    fn a_reply_shows_its_quote_folded_under_the_body() {
        let mut app = with_sidebar((160, 16));
        let found = crate::composer::tests::a_message_and_its_account();
        app.compose(postio_body::replying::reply_draft(
            postio_body::replying::ReplyKind::Reply,
            &found.0,
            &found.1,
        ));
        let screen = screen(160, 16, &app);
        assert!(screen.contains("▸ Quoted message"), "{screen}");
        assert!(!screen.contains("Hello there"), "folded:\n{screen}");
    }

    #[test]
    fn the_schedule_picker_lists_its_times_by_number() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        let mut app = with_sidebar((160, 16));
        let mut draft = postio_model::Draft::new(postio_model::AccountId::new(1));
        draft.to = vec![postio_model::EmailAddress::new(
            None::<String>,
            "grace@example.net",
        )];
        app.compose(draft);
        update(
            &mut app,
            Input::Key(KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }),
        );
        let screen = screen(160, 16, &app);
        for wanted in [
            "Send later",
            "1 In 1 hour",
            "2 This evening",
            "3 Tomorrow morning",
            "4 Monday morning",
        ] {
            assert!(screen.contains(wanted), "{wanted} missing:\n{screen}");
        }
    }

    #[test]
    fn recipient_suggestions_are_listed_under_the_field_and_harmless() {
        let mut app = with_sidebar((160, 16));
        app.compose(postio_model::Draft::new(postio_model::AccountId::new(1)));
        for c in "ada@".chars() {
            update(
                &mut app,
                Input::Key(crossterm::event::KeyEvent::from(
                    crossterm::event::KeyCode::Char(c),
                )),
            );
        }
        // Contact names are harvested from received mail's headers.
        let hostile = postio_model::contact_group::RecipientCandidate::Contact(
            postio_model::EmailAddress::new(Some("Ada\u{1b}[2J"), "ada@example.com"),
        );
        update(
            &mut app,
            Input::Recipients {
                prefix: "ada@".into(),
                found: vec![hostile],
            },
        );
        let screen = screen(160, 16, &app);
        assert!(
            screen
                .lines()
                .any(|line| line.contains("› Ada") && line.contains("<ada@example.com>")),
            "{screen}"
        );
        assert!(!screen.contains('\u{1b}'), "{screen:?}");
    }

    #[test]
    fn a_drafts_files_are_listed_with_their_sizes() {
        let mut app = with_sidebar((160, 16));
        let mut draft = postio_model::Draft::new(postio_model::AccountId::new(1));
        let mut pdf = postio_model::Attachment::new(
            postio_model::MessageId::UNASSIGNED,
            "application/pdf",
            12_288,
        );
        // A forward carries the sender's file names.
        pdf.filename = Some("fixture\u{1b}]0;x\u{7}.pdf".into());
        draft.attachments = vec![pdf];
        app.compose(draft);
        let screen = screen(160, 16, &app);
        let listed = screen
            .lines()
            .find(|line| line.contains("fixture"))
            .unwrap_or_else(|| panic!("not listed:\n{screen}"));
        assert!(
            listed.contains(&postio_ui::format::human_size(12_288)),
            "{listed}"
        );
        assert!(!screen.contains('\u{1b}'), "{screen:?}");
    }

    #[test]
    fn the_path_prompt_is_drawn_while_it_is_open() {
        let mut app = with_sidebar((160, 16));
        app.compose(postio_model::Draft::new(postio_model::AccountId::new(1)));
        update(
            &mut app,
            Input::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('a'),
                crossterm::event::KeyModifiers::ALT,
            )),
        );
        update(&mut app, Input::Paste("~/fixture.pdf".into()));
        let screen = screen(160, 16, &app);
        assert!(screen.contains("Attach: ~/fixture.pdf"), "{screen}");
    }

    #[test]
    fn a_hostile_subject_in_the_composer_reaches_the_screen_harmless() {
        let mut app = with_sidebar((160, 16));
        let mut draft = postio_model::Draft::new(postio_model::AccountId::new(1));
        // What a reply copies from the message it answers.
        draft.subject = "Re: \u{1b}]0;pwned\u{7}\u{1b}[2J".into();
        app.compose(draft);
        let screen = screen(160, 16, &app);
        assert!(!screen.contains('\u{1b}'), "{screen:?}");
        assert!(screen.contains("Re:"), "{screen}");
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
    fn the_message_under_the_cursor_is_read_beside_the_list() {
        use chrono::Utc;
        use postio_ui::paging::Page;
        use postio_ui::terminal::SafeText;
        let mut app = with_sidebar((160, 12));
        let scope = postio_model::ListScope::Mailbox(postio_model::MailboxId::new(1));
        let effects = update(&mut app, Input::Opened { scope, total: 1 });
        let (generation, page) = effects
            .iter()
            .find_map(|effect| match effect {
                crate::app::Effect::Fetch {
                    generation, page, ..
                } => Some((*generation, *page)),
                _ => None,
            })
            .expect("the first page is asked for");
        let row = crate::row::Row {
            id: postio_model::MessageId::new(7),
            thread: None,
            is_thread: false,
            from: SafeText::new("Ada Lovelace"),
            address: None,
            subject: SafeText::new("Engine notes"),
            preview: SafeText::new(""),
            when: Utc::now(),
            unread: true,
            flagged: false,
            attachment: false,
            count: 1,
        };
        update(
            &mut app,
            Input::Page {
                generation,
                page,
                rows: Ok(Page {
                    total: 1,
                    rows: vec![row],
                }),
            },
        );
        update(&mut app, Input::Rested(postio_model::MessageId::new(7)));
        update(
            &mut app,
            Input::Body {
                message: postio_model::MessageId::new(7),
                answer: Ok(postio_client::protocol::Body::Ready {
                    body: postio_model::MessageBody {
                        text: None,
                        html: Some("<p>The <b>analytical</b> engine</p>".into()),
                    },
                    encoding_problems: false,
                }),
            },
        );
        let screen = screen(160, 12, &app);
        assert!(screen.contains("The analytical engine"), "{screen}");
        assert_eq!(
            screen.matches("Engine notes").count(),
            2,
            "the subject is in the list and heads the reader:\n{screen}"
        );
        assert!(
            screen.contains("│ Engine notes"),
            "a divider keeps the panes apart:\n{screen}"
        );
    }

    #[test]
    fn a_narrower_terminal_leaves_the_sidebar_out() {
        let app = with_sidebar((100, 12));
        let screen = screen(100, 12, &app);
        assert!(!screen.contains("Snoozed"), "{screen}");
    }
}
