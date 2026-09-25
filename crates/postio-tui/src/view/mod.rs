//! Drawing: the app's state onto a ratatui frame.
//!
//! Nothing here decides anything about mail; it draws what `App` holds.

pub mod cheatsheet;
pub mod composer;
pub mod first_run;
pub mod hit;
pub mod list;
pub mod palette;
pub mod reader;
pub mod search;
pub mod settings;
pub mod sidebar;
pub mod topbar;
pub mod wrap;

use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;

use crate::app::{App, Focus};
use crate::layout::{Pane, Shown};

/// The sidebar's width in columns.
const SIDEBAR: u16 = 26;
use crate::theme::{Role, Theme};

/// The composer's buttons, each with the key this terminal can send for
/// it, from the keymap in force; one with no key it can send is left out,
/// and the last go first when the pane is narrow.
fn composer_actions(app: &App) -> Vec<composer::Action> {
    use postio_core::CommandId;
    [
        (CommandId::Send, "Send", "send"),
        (CommandId::ScheduleSend, "Schedule", "schedule_send"),
        (CommandId::AttachFile, "Attach", "attach_file"),
        (CommandId::DiscardDraft, "Discard", "discard_draft"),
        (CommandId::TogglePreview, "Preview", "toggle_preview"),
    ]
    .into_iter()
    .filter_map(|(command, word, id)| Some((app.hint(command)?, word, id)))
    .collect()
}

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
        Shown::Panes(_) if app.settings().is_some() && app.first_run().is_none() => {
            settings::draw(frame, area, app, theme);
            if let Some(open) = app.palette() {
                palette::draw(frame, area, &open, theme);
            }
        }
        Shown::Panes(_) if app.first_run().is_some() => {
            if let Some(run) = app.first_run() {
                first_run::draw(frame, area, run, theme);
            }
        }
        Shown::Panes(panes) => {
            let [top, body, status] = Layout::vertical([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .areas(area);
            topbar::draw(frame, top, app, theme);
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
                    &composer_actions(app),
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
                            app.sync_lines(),
                            theme,
                            &mut hits,
                        );
                    }
                    Pane::List => {
                        list::draw(
                            frame,
                            *area,
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
                                &composer_actions(app),
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
            // Over everything: a click there lands on nothing underneath.
            if let Some(open) = app.palette() {
                palette::draw(frame, area, &open, theme);
                hits.add(area, hit::Target::Overlay);
            }
            if let Some(sections) = app.cheat_sheet() {
                cheatsheet::draw(frame, area, &sections, theme);
                hits.add(area, hit::Target::Overlay);
            }
            status_line(
                frame,
                status,
                app,
                tab,
                !drawn.contains(&Pane::Sidebar),
                theme,
            );
        }
    }
    hits
}

/// The status line: what just happened on the left -- a failure marked `✕`
/// in the error colour, a success `✓` in the success colour with its undo
/// key in the accent -- and how many conversations are listed on the right,
/// with the sync state when no sidebar is there to carry it.
fn status_line(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    tab: bool,
    no_sidebar: bool,
    theme: &Theme,
) {
    use crate::app::Tone;
    use ratatui::text::Span;
    let count = match app.total() {
        1 => "1 conversation".to_owned(),
        total => format!("{total} conversations"),
    };
    let right = match app.sync_line().filter(|_| no_sidebar) {
        Some(sync) => format!("{sync} · {count}"),
        None => count,
    };
    let mut spans: Vec<Span> = Vec::new();
    if app.composer_detached() && !tab {
        spans.push(Span::styled(
            match app.hint(postio_core::CommandId::Compose) {
                Some(key) => format!("✎ A draft is open — {key} goes back to it"),
                None => "✎ A draft is open".to_owned(),
            },
            theme.style(Role::Text),
        ));
    }
    if let Some(notice) = app.notice() {
        if !spans.is_empty() {
            spans.push(Span::styled(" · ", theme.style(Role::Dim)));
        }
        match app.notice_tone() {
            Tone::Failed => spans.push(Span::styled("✕ ", theme.style(Role::Error))),
            Tone::Worked => spans.push(Span::styled("✓ ", theme.style(Role::Success))),
            Tone::Plain => {}
        }
        let text = match app.notice_tone() {
            Tone::Failed => theme.style(Role::Error),
            _ => theme.style(Role::Text),
        };
        let offer = app
            .notice_undo()
            .map(|key| format!(" — {key} to undo"))
            .filter(|offer| notice.ends_with(offer.as_str()));
        match (offer, app.notice_undo()) {
            (Some(offer), Some(key)) => {
                spans.push(Span::styled(
                    notice[..notice.len() - offer.len()].to_owned(),
                    text,
                ));
                spans.push(Span::styled(" — ", theme.style(Role::Dim)));
                spans.push(Span::styled(key.to_owned(), theme.style(Role::Accent)));
                spans.push(Span::styled(" to undo", theme.style(Role::Dim)));
            }
            _ => spans.push(Span::styled(notice.to_owned(), text)),
        }
    }
    let width = usize::from(area.width);
    let left = Line::from(spans);
    let used = left.width();
    let right_width = unicode_width::UnicodeWidthStr::width(right.as_str());
    if used > width {
        // Too long for the line: the words, cut, without their colours.
        let words: String = left
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        frame.render_widget(
            Line::styled(fit(&words, width), theme.style(Role::Text)),
            area,
        );
        return;
    }
    frame.render_widget(left, area);
    if used + right_width + 3 <= width {
        let x = area.x + u16::try_from(width - right_width).unwrap_or(0);
        frame.render_widget(
            Line::styled(right, theme.style(Role::Dim)),
            Rect::new(x, area.y, u16::try_from(right_width).unwrap_or(0), 1),
        );
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
        with_sidebar_and_keys(size, &Default::default())
    }

    fn with_sidebar_and_keys(size: (u16, u16), bindings: &postio_config::KeyBindings) -> App {
        use postio_model::mailbox::{Mailbox, MailboxRole};
        let keys = Keys::new(&postio_core::Keymap::resolve(bindings)).0;
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
                    key: "unread-from-ada".into(),
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
    fn the_composer_shows_how_to_send_with_a_key_this_terminal_delivers() {
        // Nothing on screen said how to send, and the desktop's Ctrl+Return
        // is, in many terminals, the terminal's own fullscreen.
        let mut app = with_sidebar((160, 16));
        app.compose(postio_model::Draft::new(postio_model::AccountId::new(1)));
        let screen = screen(160, 16, &app);
        let foot = screen
            .lines()
            .find(|line| line.contains("Send"))
            .unwrap_or_else(|| panic!("no Send in the composer:\n{screen}"));
        assert!(foot.contains("alt+s"), "{foot}");
        for word in ["Schedule", "Attach", "Discard"] {
            assert!(foot.contains(word), "{word} in {foot}");
        }
        // And it is something to click.
        let (y, line) = screen
            .lines()
            .enumerate()
            .find(|(_, line)| line.contains("Send"))
            .unwrap();
        let x = u16::try_from(line[..line.find("Send").unwrap()].chars().count()).unwrap();
        let hits = hits_of(160, 16, &app);
        assert_eq!(
            hits.at(x, u16::try_from(y).unwrap()).map(|hit| hit.target),
            Some(hit::Target::ComposerAction("send"))
        );
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
    fn a_whole_operator_is_marked_bold_and_a_half_typed_one_is_not() {
        let mut app = with_sidebar((160, 16));
        for c in "/from:ada is:".chars() {
            update(
                &mut app,
                Input::Key(crossterm::event::KeyEvent::from(
                    crossterm::event::KeyCode::Char(c),
                )),
            );
        }
        let drawn = buffer(160, 16, &app);
        let screen = screen(160, 16, &app);
        let (row, line) = screen
            .lines()
            .enumerate()
            .find(|(_, line)| line.contains("/ from:ada is:"))
            .unwrap_or_else(|| panic!("no bar:\n{screen}"));
        let bold = |needle: &str| {
            let column = line[..line.find(needle).expect("drawn")].chars().count();
            drawn[(u16::try_from(column).unwrap(), u16::try_from(row).unwrap())]
                .modifier
                .contains(ratatui::style::Modifier::BOLD)
        };
        assert!(bold("from:ada"), "a whole operator reads as one");
        assert!(
            !bold("is:"),
            "one still waiting for its value is in progress"
        );
    }

    #[test]
    fn the_search_field_in_the_top_bar_holds_the_query_and_its_readout() {
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
        let bar = screen.lines().next().expect("a top row");
        assert!(
            bar.contains("/ from:ada tide"),
            "the search field is the top bar:\n{screen}"
        );
        assert!(
            !bar.contains("Inbox"),
            "across the top, over no pane:\n{screen}"
        );
        assert!(bar.contains("1 hit · 7 ms · still syncing"), "{bar}");
        assert!(
            !bar.contains("Search all mail"),
            "the query replaces the placeholder: {bar}"
        );
    }

    #[test]
    fn the_top_bar_offers_search_and_its_hints_come_from_the_keymap() {
        let app = with_sidebar((160, 16));
        let drawn = screen(160, 16, &app);
        let top = drawn.lines().next().expect("a top row");
        assert!(top.contains("Search all mail"), "{top}");
        assert!(top.contains("? keys"), "{top}");
        assert!(top.contains("c compose"), "{top}");

        let mut bindings = postio_config::KeyBindings::default();
        bindings
            .overrides_mut()
            .insert("compose".into(), "N".into());
        let keymap = postio_core::Keymap::resolve(&bindings);
        let compose = postio_ui::terminal::deliverable_binding(
            &keymap,
            postio_core::CommandId::Compose,
            false,
        )
        .expect("still bound");
        let rebound = with_sidebar_and_keys((160, 16), &bindings);
        let drawn = screen(160, 16, &rebound);
        let top = drawn.lines().next().expect("a top row");
        assert!(top.contains(&format!("{compose} compose")), "{top}");
        assert!(!top.contains("c compose"), "{top}");
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
        assert!(
            screen.contains("╭─ Commands ─"),
            "a rounded frame, titled:\n{screen}"
        );
        assert!(screen.contains('╯'), "{screen}");
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
        assert!(
            screen.contains("╭─ Keys "),
            "a rounded frame, titled:\n{screen}"
        );
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
        let hits = hits_of(160, 16, &app);
        let hit = hits
            .at(x, u16::try_from(y).unwrap())
            .expect("something is there");
        assert_eq!(hit.target, hit::Target::Row(2));
        // A row is two lines and the rule under them, and a click on any of
        // the three is on it.
        let lines_of_row_2 = (0..16u16)
            .filter(|row| {
                hits.at(x, *row)
                    .is_some_and(|hit| hit.target == hit::Target::Row(2))
            })
            .count();
        assert_eq!(lines_of_row_2, 3, "all of the row's lines are the row");
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
                crate::app::Effect::SaveLayout(state) => Some(state.clone()),
                _ => None,
            })
            .expect("the width is saved when the drag ends");

        // A restart with what was saved draws the same.
        let mut again = with_sidebar((160, 16)).with_layout(saved);
        reading_something(&mut again);
        assert_eq!(divider(&hits_of(160, 16, &again), 160, 16), after);
    }

    #[test]
    fn an_empty_store_opens_on_the_first_run() {
        // T084: the empty-store screen.
        use crossterm::event::{KeyCode, KeyEvent};
        let keys = Keys::new(&postio_core::Keymap::resolve(&Default::default())).0;
        let mut app = App::new((120, 30), keys);
        update(
            &mut app,
            Input::Sidebar(crate::sidebar::Contents::default()),
        );
        let first = screen(120, 30, &app);
        for wanted in ["Add your first account", "1 / 3", "Address"] {
            assert!(first.contains(wanted), "{wanted} missing:\n{first}");
        }

        for c in "ada@example.test".chars() {
            update(&mut app, Input::Key(KeyEvent::from(KeyCode::Char(c))));
        }
        update(&mut app, Input::Key(KeyEvent::from(KeyCode::Enter)));
        update(
            &mut app,
            Input::Discovered(Ok(postio_ui::onboarding::Status::Found(
                postio_ui::onboarding::Settings {
                    imap: postio_ui::onboarding::Server {
                        host: "imap.example.test".into(),
                        port: 993,
                        security: postio_model::TransportSecurity::Tls,
                    },
                    smtp: postio_ui::onboarding::Server {
                        host: "smtp.example.test".into(),
                        port: 465,
                        security: postio_model::TransportSecurity::Tls,
                    },
                    source: "Fastmail".into(),
                    ..Default::default()
                },
            ))),
        );
        for c in "secret".chars() {
            update(&mut app, Input::Key(KeyEvent::from(KeyCode::Char(c))));
        }
        let found = screen(120, 30, &app);
        for wanted in [
            "imap.example.test:993 · TLS",
            "Fastmail",
            "Password",
            "••••••",
        ] {
            assert!(found.contains(wanted), "{wanted} missing:\n{found}");
        }
        assert!(
            !found.contains("secret"),
            "the password is never drawn:\n{found}"
        );

        update(&mut app, Input::Key(KeyEvent::from(KeyCode::Enter)));
        update(
            &mut app,
            Input::AccountAdded(Err("The server rejected that address and password.".into())),
        );
        let refused = screen(120, 30, &app);
        assert!(
            refused.contains("The server rejected that address and password."),
            "{refused}"
        );

        update(&mut app, Input::Key(KeyEvent::from(KeyCode::Enter)));
        update(&mut app, Input::AccountAdded(Ok(())));
        let window = screen(120, 30, &app);
        for choice in postio_ui::onboarding::SyncWindow::ALL {
            assert!(
                window.contains(choice.label()),
                "{} missing:\n{window}",
                choice.label()
            );
        }
        assert!(window.contains("3 / 3"), "{window}");
    }

    #[test]
    fn the_settings_show_every_section_and_the_accounts() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = with_sidebar((160, 30));
        update(
            &mut app,
            Input::Key(KeyEvent::new(KeyCode::Char(','), KeyModifiers::ALT)),
        );
        let screen = screen(160, 30, &app);
        for group in postio_ui::settings::Group::ALL {
            assert!(
                screen.contains(group.label()),
                "{} missing:\n{screen}",
                group.label()
            );
        }
        for section in postio_ui::settings::Section::ALL {
            assert!(
                screen.contains(section.label()),
                "{} missing:\n{screen}",
                section.label()
            );
        }
        assert!(
            screen.contains(postio_ui::settings::Section::Accounts.description()),
            "{screen}"
        );
        assert!(
            screen.contains("ada@example.com"),
            "the accounts are listed:\n{screen}"
        );
    }

    #[test]
    fn a_browser_sign_in_shows_the_whole_address_and_what_it_allows() {
        use crossterm::event::{KeyCode, KeyEvent};
        let keys = Keys::new(&postio_core::Keymap::resolve(&Default::default())).0;
        let mut app = App::new((100, 40), keys);
        update(
            &mut app,
            Input::Sidebar(crate::sidebar::Contents::default()),
        );
        for c in "ada@example.test".chars() {
            update(&mut app, Input::Key(KeyEvent::from(KeyCode::Char(c))));
        }
        update(&mut app, Input::Key(KeyEvent::from(KeyCode::Enter)));
        update(
            &mut app,
            Input::Discovered(Ok(postio_ui::onboarding::Status::Found(
                postio_ui::onboarding::Settings {
                    oauth_sign_in: true,
                    ..Default::default()
                },
            ))),
        );
        for c in "postio-test".chars() {
            update(&mut app, Input::Key(KeyEvent::from(KeyCode::Char(c))));
        }
        update(&mut app, Input::Key(KeyEvent::from(KeyCode::Enter)));
        let url = format!(
            "https://login.example.test/authorize?client_id=postio-test&state={}&end=here",
            "x".repeat(120)
        );
        update(
            &mut app,
            Input::Consent(Ok(postio_ui::onboarding::BrowserSignIn {
                provider: "Example".into(),
                scopes: vec!["offline_access".into()],
                redirect_uri: "http://127.0.0.1:41337/".into(),
                authorize_url: url.clone(),
            })),
        );
        let screen = screen(100, 40, &app);
        let joined: String = screen.lines().map(str::trim).collect();
        assert!(
            joined.contains(&url),
            "the whole address, wrapped:\n{screen}"
        );
        assert!(
            screen.contains("Stay signed in without asking again"),
            "{screen}"
        );
        assert!(screen.contains("Enter opens it"), "{screen}");
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
        for wanted in ["Inbox", "Flagged", "Snoozed", "Unread from Ada"] {
            assert!(screen.contains(wanted), "{wanted} missing:\n{screen}");
        }
        // A heading is set in capitals, as the canvas sets it.
        assert!(
            screen.contains("SAVED SEARCHES"),
            "the saved searches' heading is missing:\n{screen}"
        );
        assert!(
            screen
                .lines()
                .any(|line| line.contains("Inbox") && line.contains('4')),
            "{screen}"
        );
    }

    #[test]
    fn the_sidebar_is_headed_by_the_account_and_marks_the_open_folder() {
        let app = with_sidebar((160, 16));
        let screen = screen(160, 16, &app);
        let sidebar: Vec<String> = screen
            .lines()
            .map(|line| line.chars().take(26).collect())
            .collect();
        assert!(
            sidebar
                .iter()
                .any(|line| line.replace(' ', "").contains("ADA@EXAMPLE.COM")),
            "the account's address heads its folders:\n{screen}"
        );
        let inbox = sidebar
            .iter()
            .find(|line| line.contains("Inbox"))
            .unwrap_or_else(|| panic!("no Inbox:\n{screen}"));
        assert!(
            inbox.starts_with('▌'),
            "the open folder has a bar: {inbox:?}"
        );
        assert!(
            inbox.trim_end_matches('│').trim_end().ends_with('4'),
            "its count at the right: {inbox:?}"
        );
        // Every row between the top bar and the status line.
        assert!(
            sidebar[1..sidebar.len() - 1]
                .iter()
                .all(|line| line.ends_with('│')),
            "a rule keeps the sidebar apart:\n{screen}"
        );
    }

    #[test]
    fn the_sync_state_sits_at_the_foot_of_the_sidebar() {
        let inbox = postio_model::ListScope::Mailbox(postio_model::MailboxId::new(1));
        let mut app = with_sidebar((160, 16));
        update(
            &mut app,
            Input::Opened {
                scope: inbox,
                total: 0,
            },
        );
        let (state, detail) = app.sync_lines().expect("an account is shown");
        let screen = screen(160, 16, &app);
        let lines: Vec<&str> = screen.lines().collect();
        let foot = |line: &str| line.chars().take(26).collect::<String>();
        assert!(foot(lines[13]).contains(&state), "{screen}");
        assert!(foot(lines[14]).contains(&detail), "{screen}");
        assert!(
            !lines[15].contains(&state),
            "not on the status line as well:\n{screen}"
        );

        // Without the sidebar, the status line still says it.
        let mut narrow = with_sidebar((100, 16));
        update(
            &mut narrow,
            Input::Opened {
                scope: inbox,
                total: 0,
            },
        );
        let screen = screen_of_size(&narrow, 100, 16);
        assert!(screen.lines().last().unwrap().contains(&state), "{screen}");
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
            screen.contains("│  Engine notes"),
            "a divider keeps the panes apart:\n{screen}"
        );
    }

    #[test]
    fn the_reader_is_headed_by_who_wrote_to_whom_and_when_and_offers_its_keys() {
        use chrono::Utc;
        use postio_ui::paging::Page;
        use postio_ui::terminal::SafeText;
        let mut app = with_sidebar((160, 20));
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
            address: Some("ada@example.com".into()),
            subject: SafeText::new("Engine notes"),
            preview: SafeText::new(""),
            when: Utc.with_ymd_and_hms(2026, 9, 22, 9, 14, 0).unwrap(),
            unread: false,
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
            Input::Addressed {
                message: postio_model::MessageId::new(7),
                to: vec![
                    postio_model::EmailAddress::new(None::<String>, "grace@example.net"),
                    postio_model::EmailAddress::new(Some("Bea"), "bea@example.org"),
                ],
            },
        );
        let screen = screen(160, 20, &app);
        let reader: Vec<String> = screen
            .lines()
            // What is right of the list's rule.
            .map(|line| line.rsplit('│').next().unwrap_or_default().to_owned())
            .collect();
        let at = |needle: &str| {
            reader
                .iter()
                .position(|line| line.contains(needle))
                .unwrap_or_else(|| panic!("{needle} missing:\n{screen}"))
        };
        let subject = at("Engine notes");
        let meta = at("ada@example.com → grace@example.net, Bea");
        assert_eq!(meta, subject + 1, "who under what:\n{screen}");
        assert!(
            reader[meta].contains("Tue 22 Sep"),
            "and when: {}",
            reader[meta]
        );
        let keys = reader.len() - 2;
        assert!(
            reader[keys].contains("e reply") && reader[keys].contains("a archive"),
            "the keys at the foot:\n{screen}"
        );
    }

    fn status_of(app: &App, colour: Colour) -> (String, Vec<ratatui::buffer::Cell>, Theme) {
        let theme = Theme::new(colour, Background::Dark, &Default::default()).0;
        let now = chrono::Local
            .with_ymd_and_hms(2026, 9, 23, 12, 0, 0)
            .unwrap();
        let mut terminal = Terminal::new(TestBackend::new(160, 16)).unwrap();
        terminal
            .draw(|frame| {
                draw(frame, app, &theme, now);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let cells: Vec<ratatui::buffer::Cell> = (0..160).map(|x| buffer[(x, 15)].clone()).collect();
        let text = cells.iter().map(|cell| cell.symbol().to_owned()).collect();
        (text, cells, theme)
    }

    #[test]
    fn the_status_line_marks_what_failed_and_what_worked_and_the_undo_key() {
        let mut app = with_sidebar((160, 16));
        update(
            &mut app,
            Input::Host(postio_core::Event::Error {
                message: "The server refused the move".into(),
            }),
        );
        let (text, cells, theme) = status_of(&app, Colour::TrueColor);
        assert!(text.starts_with("✕ The server refused the move"), "{text}");
        assert_eq!(cells[2].fg, theme.style(Role::Error).fg.unwrap(), "{text}");

        update(
            &mut app,
            Input::Host(postio_core::Event::ActionCompleted {
                description: "Archived 1 message".into(),
                undoable: true,
            }),
        );
        let (text, cells, theme) = status_of(&app, Colour::TrueColor);
        assert!(
            text.starts_with("✓ Archived 1 message — u to undo"),
            "{text}"
        );
        assert_eq!(cells[0].fg, theme.style(Role::Success).fg.unwrap());
        let key = text.chars().position(|c| c == 'u').expect("the key");
        assert_eq!(
            cells[key].fg,
            theme.style(Role::Accent).fg.unwrap(),
            "the undo key in the accent: {text}"
        );
        // Without colour the marks still say it.
        let (text, _, _) = status_of(&app, Colour::None);
        assert!(text.starts_with('✓'), "{text}");
    }

    #[test]
    fn the_readers_lines_are_wrapped_to_its_pane() {
        // One paragraph far wider than the reading pane: every word of it
        // must be on screen, none cut at the pane's right edge.
        let words: Vec<String> = (0..60).map(|n| format!("w{n:02}")).collect();
        let mut app = with_sidebar((160, 30));
        app.set_reading_for_tests(crate::conversation::Reading {
            row: postio_model::MessageId::new(1),
            members: vec![crate::conversation::tests::member_saying(
                1,
                &words.join(" "),
            )],
            current: 0,
        });
        let screen = screen(160, 30, &app);
        for word in &words {
            assert!(screen.contains(word.as_str()), "{word} was cut:\n{screen}");
        }
    }

    #[test]
    fn a_narrower_terminal_leaves_the_sidebar_out() {
        let app = with_sidebar((100, 12));
        let screen = screen(100, 12, &app);
        assert!(!screen.contains("Snoozed"), "{screen}");
    }
}
