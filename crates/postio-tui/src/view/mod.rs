//! Drawing: the app's state onto a ratatui frame.
//!
//! Nothing here decides anything about mail; it draws what `App` holds.

pub mod cheatsheet;
pub mod composer;
pub mod hit;
pub mod list;
pub mod palette;
pub mod reader;
pub mod search;
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

/// Draw the whole screen, and answer what is where on it, for the mouse.
pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, now: DateTime<Local>) -> hit::Hits {
    let mut hits = hit::Hits::default();
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
                    Pane::Reader => match app.reader_columns() {
                        Some(columns) => Constraint::Length(columns),
                        None => Constraint::Percentage(45),
                    },
                })
                .collect();
            let areas = Layout::horizontal(widths).split(body);
            // A draft in a tab of its own has the whole screen while it is
            // in front, as the desktop's composer window would.
            let tab = app.composer_detached() && app.focus() == Focus::Composer;
            let drawn: &[Pane] = if tab { &[] } else { &panes };
            if tab && let Some(writing) = app.composer() {
                composer::draw(
                    frame,
                    body,
                    writing,
                    app.preview_shown(),
                    app.scheduling().is_none() && app.path_prompt().is_none(),
                    theme,
                    &mut hits,
                );
                if let Some(times) = app.scheduling() {
                    composer::draw_schedule(frame, body, times, theme, now);
                }
                if let Some(typed) = app.path_prompt() {
                    composer::draw_path_prompt(frame, body, typed, theme);
                }
            }
            for (pane, area) in drawn.iter().zip(areas.iter()) {
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
                            &mut hits,
                        );
                    }
                    Pane::List => {
                        let list_area = match app.search_query() {
                            Some(query) => search::draw(
                                frame,
                                *area,
                                query,
                                app.search_caret(),
                                app.search_readout().as_deref(),
                                app.focus() == Focus::Search,
                                theme,
                            ),
                            None => *area,
                        };
                        list::draw(
                            frame,
                            list_area,
                            &app.visible(),
                            app.top(),
                            theme,
                            now,
                            &mut hits,
                        );
                    }
                    Pane::Reader => {
                        if let Some(writing) = app.composer().filter(|_| !app.showing_reader()) {
                            composer::draw(
                                frame,
                                *area,
                                writing,
                                app.preview_shown(),
                                app.focus() == Focus::Composer
                                    && app.scheduling().is_none()
                                    && app.path_prompt().is_none(),
                                theme,
                                &mut hits,
                            );
                            if let Some(times) = app.scheduling() {
                                composer::draw_schedule(frame, *area, times, theme, now);
                            }
                            if let Some(typed) = app.path_prompt() {
                                composer::draw_path_prompt(frame, *area, typed, theme);
                            }
                        } else {
                            reader::draw(frame, *area, app, theme, now, &mut hits);
                        }
                    }
                }
            }
            let count = match app.total() {
                1 => "1 conversation".to_owned(),
                total => format!("{total} conversations"),
            };
            let mut words = match (app.notice(), app.sync_line()) {
                (Some(notice), _) => notice.to_owned(),
                (None, Some(sync)) => format!("{sync} · {count}"),
                (None, None) => count,
            };
            if app.composer_detached() && !tab {
                words = format!("✎ A draft is open — c goes back to it · {words}");
            }
            // Over everything: a click there lands on nothing underneath.
            if let Some(open) = app.palette() {
                palette::draw(frame, area, &open, theme);
                hits.add(area, hit::Target::Overlay);
            }
            if let Some(sections) = app.cheat_sheet() {
                cheatsheet::draw(frame, area, &sections, theme);
                hits.add(area, hit::Target::Overlay);
            }
            let words = fit(&words, usize::from(status.width));
            frame.render_widget(Line::styled(words, theme.style(Role::Dim)), status);
        }
    }
    hits
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
            .draw(|frame| {
                draw(frame, app, &theme, now);
            })
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
                saved: vec![crate::sidebar::Saved {
                    name: "Unread from Ada".into(),
                    query: "from:ada is:unread".into(),
                }],
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

    fn buffer(width: u16, height: u16, app: &App) -> ratatui::buffer::Buffer {
        let theme = Theme::new(Colour::None, Background::Unknown, &Default::default()).0;
        let now = chrono::Local
            .with_ymd_and_hms(2026, 9, 23, 12, 0, 0)
            .unwrap();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                draw(frame, app, &theme, now);
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn writing_bold(app: &mut App) {
        let mut draft = postio_model::Draft::new(postio_model::AccountId::new(1));
        draft.body_markdown = Some("Some **bold** words".into());
        app.compose(draft);
    }

    #[test]
    fn the_preview_shows_bold_where_the_source_says_so() {
        // T057, toggle mode: one key swaps the text for the message as it
        // will arrive.
        let mut app = with_sidebar((160, 16));
        writing_bold(&mut app);
        update(
            &mut app,
            Input::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('p'),
                crossterm::event::KeyModifiers::ALT,
            )),
        );
        let drawn = buffer(160, 16, &app);
        let screen = screen(160, 16, &app);
        assert!(!screen.contains("**bold**"), "{screen}");
        let (x, y) = (0..drawn.area.height)
            .find_map(|y| {
                let line: String = (0..drawn.area.width)
                    .map(|x| drawn[(x, y)].symbol().to_owned())
                    .collect();
                line.find("Some bold words").map(|at| {
                    let column = line[..at].chars().count() + "Some ".len();
                    (u16::try_from(column).unwrap(), y)
                })
            })
            .unwrap_or_else(|| panic!("no preview:\n{screen}"));
        assert!(
            drawn[(x, y)]
                .modifier
                .contains(ratatui::style::Modifier::BOLD),
            "`bold` is not drawn bold"
        );

        update(
            &mut app,
            Input::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('p'),
                crossterm::event::KeyModifiers::ALT,
            )),
        );
        assert!(
            screen_of(&app).contains("**bold**"),
            "the same key goes back"
        );
    }

    fn screen_of(app: &App) -> String {
        screen(160, 16, app)
    }

    #[test]
    fn split_mode_shows_the_text_and_the_preview_side_by_side() {
        let mut app = with_sidebar((160, 16)).with_preview(postio_config::tui::Preview::Split);
        writing_bold(&mut app);
        let screen = screen(160, 16, &app);
        assert!(screen.contains("**bold**"), "{screen}");
        assert!(screen.contains("Some bold words"), "{screen}");
    }

    #[test]
    fn a_detached_draft_has_the_screen_and_the_mail_says_it_is_open() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = with_sidebar((160, 16));
        let mut draft = postio_model::Draft::new(postio_model::AccountId::new(1));
        draft.subject = "Tide gate".into();
        app.compose(draft);
        update(
            &mut app,
            Input::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::ALT)),
        );
        let tab = screen(160, 16, &app);
        assert!(
            !tab.contains("Inbox"),
            "the draft's tab has the whole screen:\n{tab}"
        );
        assert!(tab.contains("Tide gate"), "{tab}");

        update(&mut app, Input::Key(KeyEvent::from(KeyCode::Esc)));
        let mail = screen(160, 16, &app);
        assert!(mail.contains("Inbox"), "{mail}");
        assert!(
            !mail.contains("Subject"),
            "the reading pane is the reader's:\n{mail}"
        );
        assert!(mail.contains("A draft is open"), "{mail}");
    }

    #[test]
    fn the_search_bar_sits_over_the_list_with_its_readout() {
        let mut app = with_sidebar((160, 16));
        update(
            &mut app,
            Input::Key(crossterm::event::KeyEvent::from(
                crossterm::event::KeyCode::Char('/'),
            )),
        );
        for c in "from:ada tide".chars() {
            update(
                &mut app,
                Input::Key(crossterm::event::KeyEvent::from(
                    crossterm::event::KeyCode::Char(c),
                )),
            );
        }
        update(
            &mut app,
            Input::Found {
                sequence: 13,
                found: Ok(Some(postio_client::protocol::Found {
                    ids: vec![postio_model::MessageId::new(4)],
                    hits: 1,
                    capped: false,
                    corpus_complete: false,
                    elapsed: std::time::Duration::from_millis(7),
                })),
            },
        );
        let screen = screen(160, 16, &app);
        let bar = screen
            .lines()
            .find(|line| line.contains("/ from:ada tide"))
            .unwrap_or_else(|| panic!("no bar:\n{screen}"));
        assert!(bar.contains("1 hit · 7 ms · still syncing"), "{bar}");
    }

    #[test]
    fn the_palette_draws_its_rows_with_their_keys() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = with_sidebar((160, 24));
        update(
            &mut app,
            Input::Key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL)),
        );
        for c in "archive".chars() {
            update(&mut app, Input::Key(KeyEvent::from(KeyCode::Char(c))));
        }
        let screen = screen(160, 24, &app);
        assert!(screen.contains("> archive"), "{screen}");
        let row = screen
            .lines()
            .find(|line| line.contains("Archive") && !line.contains("thread"))
            .unwrap_or_else(|| panic!("no Archive row:\n{screen}"));
        assert!(row.contains(" a│"), "the key, at the right edge: {row}");
    }

    #[test]
    fn the_cheat_sheet_shows_every_section_and_binding() {
        // T067.
        use crossterm::event::{KeyCode, KeyEvent};
        let mut app = with_sidebar((200, 90));
        update(&mut app, Input::Key(KeyEvent::from(KeyCode::Char('?'))));
        let screen = screen(200, 90, &app);
        let keymap = postio_core::Keymap::resolve(&Default::default());
        let sections = postio_ui::cheatsheet::sections(
            &keymap,
            postio_core::Context::List,
            // No list is open here, so the view is unified, where a move has
            // no account to move within (#182).
            postio_core::Availability::open(postio_core::Scope::Unified),
        );
        assert!(!sections.is_empty());
        for section in &sections {
            assert!(
                screen.contains(section.title),
                "{} missing:\n{screen}",
                section.title
            );
            for row in &section.rows {
                let key = row
                    .id
                    .and_then(|id| postio_ui::terminal::deliverable_binding(&keymap, id, false))
                    .unwrap_or_default();
                assert!(
                    screen
                        .lines()
                        .any(|line| line.contains(row.title) && line.contains(&key)),
                    "{} ({key}) missing:\n{screen}",
                    row.title
                );
            }
        }
        update(&mut app, Input::Key(KeyEvent::from(KeyCode::Esc)));
        assert!(!screen_of_size(&app, 200, 90).contains(sections[0].title));
    }

    fn screen_of_size(app: &App, width: u16, height: u16) -> String {
        screen(width, height, app)
    }

    fn hits_of(width: u16, height: u16, app: &App) -> hit::Hits {
        let theme = Theme::new(Colour::None, Background::Unknown, &Default::default()).0;
        let now = chrono::Local
            .with_ymd_and_hms(2026, 9, 23, 12, 0, 0)
            .unwrap();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut hits = hit::Hits::default();
        terminal
            .draw(|frame| hits = draw(frame, app, &theme, now))
            .unwrap();
        hits
    }

    #[test]
    fn a_click_on_the_third_list_row_is_that_row() {
        // T070.
        use postio_ui::paging::Page;
        let mut app = with_sidebar((160, 16));
        let effects = update(
            &mut app,
            Input::Opened {
                scope: postio_model::ListScope::Mailbox(postio_model::MailboxId::new(1)),
                total: 5,
            },
        );
        let generation = effects
            .iter()
            .find_map(|effect| match effect {
                crate::app::Effect::Fetch { generation, .. } => Some(*generation),
                _ => None,
            })
            .expect("a page asked for");
        let rows = (0..5).map(crate::app::tests::row).collect();
        update(
            &mut app,
            Input::Page {
                generation,
                page: 0,
                rows: Ok(Page { rows, total: 5 }),
            },
        );
        let screen = screen(160, 16, &app);
        let (y, line) = screen
            .lines()
            .enumerate()
            .find(|(_, line)| line.contains("Message 2"))
            .unwrap_or_else(|| panic!("no third row:\n{screen}"));
        let x = u16::try_from(line.chars().position(|c| c == 'M').unwrap()).unwrap();
        let hit = hits_of(160, 16, &app)
            .at(x, u16::try_from(y).unwrap())
            .expect("something is there");
        assert_eq!(hit.target, hit::Target::Row(2));
    }

    fn divider(hits: &hit::Hits, width: u16, height: u16) -> u16 {
        (0..width)
            .find(|x| {
                (0..height).any(|y| {
                    hits.at(*x, y)
                        .is_some_and(|hit| hit.target == hit::Target::Divider)
                })
            })
            .expect("a divider")
    }

    fn reading_something(app: &mut App) {
        app.set_reading_for_tests(crate::conversation::Reading {
            row: postio_model::MessageId::new(1),
            members: vec![crate::conversation::tests::member_with_lines(1, 3)],
            current: 0,
        });
    }

    #[test]
    fn dragging_the_divider_widens_the_list_and_the_width_is_remembered() {
        // T074.
        use crate::app::Pointer;
        let mut app = with_sidebar((160, 16));
        reading_something(&mut app);
        let hits = hits_of(160, 16, &app);
        let before = divider(&hits, 160, 16);
        let press = hits.at(before, 3).expect("the divider is there");
        update(
            &mut app,
            Input::Pointer(Pointer::Click {
                hit: press,
                ctrl: false,
                shift: false,
            }),
        );
        update(
            &mut app,
            Input::Pointer(Pointer::Drag { column: before + 5 }),
        );
        update(
            &mut app,
            Input::Pointer(Pointer::Drag {
                column: before + 10,
            }),
        );
        let effects = update(&mut app, Input::Pointer(Pointer::Release));
        let after = divider(&hits_of(160, 16, &app), 160, 16);
        assert_eq!(after, before + 10, "the list is ten columns wider");
        let saved = effects
            .iter()
            .find_map(|effect| match effect {
                crate::app::Effect::SaveLayout(state) => Some(*state),
                _ => None,
            })
            .expect("the width is saved when the drag ends");

        // A restart with what was saved draws the same.
        let mut again = with_sidebar((160, 16)).with_layout(saved);
        reading_something(&mut again);
        assert_eq!(divider(&hits_of(160, 16, &again), 160, 16), after);
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
