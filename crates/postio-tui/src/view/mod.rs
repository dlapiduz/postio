//! Drawing: the app's state onto a ratatui frame.
//!
//! Nothing here decides anything about mail; it draws what `App` holds.

pub mod bar;
pub mod bottom;
pub mod boxed;
pub mod composer;
pub mod filtered;
pub mod first_run;
pub mod folders;
pub mod hit;
pub mod list;
pub mod open;
pub mod palette;
pub mod pane;
pub mod picker;
pub mod settings;
pub mod sheet;
pub mod state;
pub mod strip;
pub mod topbar;
pub mod wrap;

use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;

use crate::app::{App, Focus, Front};
use crate::theme::{Role, Theme};

/// Draw the whole screen, and answer what is where on it, for the mouse.
pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, now: DateTime<Local>) -> hit::Hits {
    let mut hits = hit::Hits::default();
    let area = frame.area();
    if !app.fits() {
        let needs = crate::layout::MINIMUM;
        let sentence = format!("Terminal too small: needs {}×{}", needs.0, needs.1);
        let line = Line::styled(
            fit(&sentence, usize::from(area.width)),
            theme.style(Role::Warning),
        );
        frame.render_widget(line, Rect::new(area.x, area.y, area.width, 1));
    } else if app.settings().is_some() && app.first_run().is_none() {
        settings::draw(frame, area, app, theme);
        if let Some(open) = app.palette() {
            palette::draw(frame, area, &open, theme);
        }
        if let Some((bar, ctx)) = app.bar() {
            bar::draw(frame, area, bar, &ctx, now, theme, &mut hits);
        }
    } else if let Some(run) = app.first_run() {
        first_run::draw(frame, area, run, theme);
    } else {
        let window = app.window();
        topbar::draw(frame, window.top, app, theme, &mut hits);
        // A draft in a tab of its own has the whole screen between the top
        // bar and the bottom line while it is in front.
        let tab = app.composer_detached() && app.focus() == Focus::Composer;
        if !tab {
            if app.filtered().is_some() {
                filtered::strip(frame, window.strip, app, theme, &mut hits);
            } else {
                strip::draw(frame, window.strip, app, theme, &mut hits);
            }
        }
        let below = window.strip.y + u16::from(!tab);
        let body = Rect::new(
            area.x,
            below,
            area.width,
            window.bottom.y.saturating_sub(below),
        );
        if let (Some(row), Some(banner)) = (window.banner, app.banner()) {
            state::banner(frame, row, &banner, app, theme, &mut hits);
        }
        match app.front() {
            // Filtered takes the strip's place and the list's.
            _ if app.filtered().is_some() => {
                filtered::body(frame, body, app, theme, now, &mut hits);
            }
            // A detached draft has the whole body to itself.
            Front::Composer if app.composer().is_some() && tab => {
                composer::screen(frame, body, app, theme, now, &mut hits);
            }
            _ => {
                // Beside the reading pane the list keeps the left of the
                // rows, and the pane the rest.
                let beside = app
                    .pane()
                    .and_then(|_| crate::layout::split_pane(window.list));
                let region = beside.map_or(window.list, |(list, _)| list);
                if let Some((_, pane)) = beside {
                    pane::draw(frame, pane, app, theme, now, &mut hits);
                }
                let list = region;
                if let Some(said) = app.empty_inbox(now) {
                    state::empty(frame, list, &said, theme, &mut hits);
                } else {
                    list::draw(
                        frame,
                        list,
                        beside.is_some(),
                        &app.visible(),
                        app.top(),
                        theme,
                        now,
                        &|command| app.hint(command),
                        &mut hits,
                    );
                }
            }
        }
        if app.front() == Front::Reader && app.pane().is_none() {
            open::over_list(frame, area, app, theme, now, &mut hits);
        }
        // The draft being written, in the frame over the list.
        if app.front() == Front::Composer
            && !tab
            && app.composer().is_some()
            && let Some((_, inside)) = open::framed(frame, area, theme, &mut hits)
        {
            composer::screen(frame, inside, app, theme, now, &mut hits);
        }
        // The sweep's question, over the window.
        filtered::sweep(frame, body, app, theme, &mut hits);
        // Over everything: a click there lands on nothing underneath.
        if let Some((bar, ctx)) = app.bar() {
            bar::draw(frame, area, bar, &ctx, now, theme, &mut hits);
        }
        if let Some((open, keymap)) = app.picker() {
            let beside = app
                .pane()
                .and_then(|_| crate::layout::split_pane(window.list));
            let list = beside.map_or(window.list, |(list, _)| list);
            let place = picker::Place {
                area: Rect::new(
                    area.x,
                    window.list.y,
                    area.width,
                    window.bottom.y.saturating_sub(window.list.y),
                ),
                row: hits.area_of(hit::Target::Row(app.cursor())),
                subject_x: list.x + list::subject_x(list.width, beside.is_some()),
            };
            picker::draw(
                frame,
                &place,
                open,
                keymap,
                &|command| app.hint(command),
                theme,
                &mut hits,
            );
        }
        if let Some((open, reach)) = app.folders() {
            folders::draw(
                frame,
                area,
                open,
                &reach,
                &|command| app.hint(command),
                theme,
                &mut hits,
            );
        }
        if let Some(open) = app.palette() {
            palette::draw(frame, area, &open, theme);
            hits.add(area, hit::Target::Overlay);
        }
        if let Some((sheet, columns)) = app.key_map() {
            sheet::draw(
                frame,
                area,
                sheet,
                &columns,
                &|command| app.hint(command),
                theme,
                &mut hits,
            );
        }
        status_line(frame, window.bottom, app, tab, theme, &mut hits);
    }
    hits
}

/// The bottom line: the bulk bar while anything is selected; otherwise what
/// just happened -- a failure marked `✕` in the error colour, a success `✓`
/// in the success colour with its undo key in the accent -- and a draft left
/// open.
fn status_line(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    tab: bool,
    theme: &Theme,
    hits: &mut hit::Hits,
) {
    use ratatui::text::Span;
    use unicode_width::UnicodeWidthStr;
    if !tab && bottom::bulk_bar(frame, area, app, theme, hits) {
        return;
    }
    let mut spans: Vec<Span> = vec![Span::raw(" ")];
    let mut undo = None;
    if let Some(toast) = bottom::toast(app, theme) {
        if spans.len() > 1 {
            spans.push(Span::styled(" · ", theme.style(Role::Dim)));
        }
        let before: usize = spans.iter().map(|span| span.content.width()).sum();
        undo = toast
            .undo
            .map(|undo| before + undo.start..before + undo.end);
        spans.extend(toast.spans);
    }
    let width = usize::from(area.width);
    let left = Line::from(spans);
    let used = left.width();
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
    if let Some(undo) = undo {
        bottom::toast_target(hits, area.x, area.y, &undo);
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
    use crate::test_support::{
        app_with_keys, buffer, hits_of, places, saved_search, screen, seed_places,
    };

    fn with_places(size: (u16, u16)) -> App {
        with_places_and_keys(size, &Default::default())
    }

    /// Mail from one account with an Inbox of four unread and a saved search.
    fn with_places_and_keys(size: (u16, u16), bindings: &postio_config::KeyBindings) -> App {
        let mut app = app_with_keys(size, bindings);
        let mut contents = places();
        contents.folders.truncate(1);
        contents.folders[0].counts.unread = 4;
        contents.saved = vec![saved_search(
            "unread-from-ada",
            "Unread from Ada",
            "from:ada is:unread",
        )];
        seed_places(&mut app, contents);
        app
    }

    #[test]
    fn the_composer_draws_its_fields_and_body_in_the_reading_pane() {
        let mut app = with_places((160, 16));
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
        let mut app = with_places((160, 16));
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
        let mut app = with_places((160, 16));
        app.compose(postio_model::Draft::new(postio_model::AccountId::new(1)));
        let screen = screen(160, 16, &app);
        let foot = screen
            .lines()
            .find(|line| line.contains("Send"))
            .unwrap_or_else(|| panic!("no Send in the composer:\n{screen}"));
        assert!(foot.contains("alt+s"), "{foot}");
        for word in ["Send later", "Attach", "Remind"] {
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
        let mut app = with_places((160, 16));
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
        // The evening's words depend on the hour: "Later today" until 6pm,
        // "Tomorrow evening" after.
        let evening = format!(
            "2 {}",
            postio_ui::schedule::schedule_presets(chrono::Local::now())[1].0
        );
        for wanted in [
            "Send later",
            "1 In 1 hour",
            evening.as_str(),
            "3 Tomorrow morning",
            "4 Monday morning",
        ] {
            assert!(screen.contains(wanted), "{wanted} missing:\n{screen}");
        }
    }

    #[test]
    fn recipient_suggestions_are_listed_under_the_field_and_harmless() {
        let mut app = with_places((160, 16));
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
        let mut app = with_places((160, 16));
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
        let mut app = with_places((160, 16));
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

    fn writing_bold(app: &mut App) {
        let mut draft = postio_model::Draft::new(postio_model::AccountId::new(1));
        draft.body_markdown = Some("Some **bold** words".into());
        app.compose(draft);
    }

    #[test]
    fn the_preview_shows_bold_where_the_source_says_so() {
        // T057, toggle mode: one key swaps the text for the message as it
        // will arrive.
        let mut app = with_places((160, 16));
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
        let mut app = with_places((160, 16)).with_preview(postio_config::tui::Preview::Split);
        writing_bold(&mut app);
        let screen = screen(160, 16, &app);
        assert!(screen.contains("**bold**"), "{screen}");
        assert!(screen.contains("Some bold words"), "{screen}");
    }

    #[test]
    fn the_top_bar_offers_search_and_its_hints_come_from_the_keymap() {
        let app = with_places((160, 16));
        let drawn = screen(160, 16, &app);
        let top = drawn.lines().next().expect("a top row");
        assert!(top.contains("Search mail, go to a folder"), "{top}");
        assert!(top.contains("? keys"), "{top}");
        assert!(top.contains("Compose c"), "{top}");

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
        let rebound = with_places_and_keys((160, 16), &bindings);
        let drawn = screen(160, 16, &rebound);
        let top = drawn.lines().next().expect("a top row");
        assert!(top.contains(&format!("Compose {compose}")), "{top}");
        assert!(!top.contains("Compose c"), "{top}");
    }

    #[test]
    fn a_click_on_the_third_list_row_is_that_row() {
        // T070.
        use postio_ui::paging::Page;
        let mut app = with_places((160, 16));
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
        let rows = (0..5).map(crate::test_support::row).collect();
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
        // A plain row is one line, and a click on it is on it.
        let lines_of_row_2 = (0..16u16)
            .filter(|row| {
                hits.at(x, *row)
                    .is_some_and(|hit| hit.target == hit::Target::Row(2))
            })
            .count();
        assert_eq!(lines_of_row_2, 1, "a plain row is one line");
    }

    #[test]
    fn an_empty_store_opens_on_the_first_run() {
        // T084: the empty-store screen.
        use crossterm::event::{KeyCode, KeyEvent};
        let mut app = crate::test_support::app((120, 30));
        update(&mut app, Input::Places(crate::places::Places::default()));
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
        let mut app = with_places((160, 30));
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
    fn an_accounts_signatures_are_listed_with_what_each_key_does() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = with_places((160, 30));
        let mut account = app.accounts()[0].clone();
        let mut work = postio_model::Signature::new("Work", "Ada\nThe Engine Room");
        work.id = postio_model::SignatureId::new(5);
        account.signatures = vec![work];
        update(
            &mut app,
            Input::Places(crate::places::Places {
                accounts: vec![account],
                ..Default::default()
            }),
        );
        for key in [
            KeyEvent::new(KeyCode::Char(','), KeyModifiers::ALT),
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
        ] {
            update(&mut app, Input::Key(key));
        }
        let accounts = screen(160, 30, &app);
        assert!(accounts.contains("s signatures"), "offered:\n{accounts}");

        update(
            &mut app,
            Input::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)),
        );
        let listed = screen(160, 30, &app);
        for wanted in ["SIGNATURES", "Work", "Ada", "n new", "r rename", "d delete"] {
            assert!(listed.contains(wanted), "{wanted} missing:\n{listed}");
        }
    }

    #[test]
    fn the_privacy_section_shows_what_left_this_machine() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = with_places((160, 30));
        update(
            &mut app,
            Input::Key(KeyEvent::new(KeyCode::Char(','), KeyModifiers::ALT)),
        );
        while app.settings().expect("open").current() != postio_ui::settings::Section::Privacy {
            update(
                &mut app,
                Input::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            );
        }
        update(
            &mut app,
            Input::Privacy {
                log: postio_client::protocol::PrivacyLog {
                    activations: Vec::new(),
                    read_receipts: 2,
                },
                connections: vec![postio_model::egress::EgressEvent {
                    at: chrono::Utc::now(),
                    subsystem: postio_model::egress::EgressSubsystem::Imap,
                    account: None,
                    host: "imap.example.com".into(),
                    port: 993,
                    outcome: postio_model::egress::EgressOutcome::Connected,
                }],
            },
        );
        let screen = screen(160, 30, &app);
        let heading = |text: &str| text.to_uppercase();
        for wanted in [
            heading(postio_ui::privacy::ALLOWED),
            postio_ui::privacy::NO_ALLOWED.to_owned(),
            heading(postio_ui::privacy::LISTS_LEFT),
            postio_ui::privacy::NO_LISTS_LEFT.to_owned(),
            heading(postio_ui::privacy::READ_RECEIPTS),
            "2 messages have requested a read receipt".to_owned(),
            heading(postio_ui::privacy::CONNECTIONS),
            "imap · imap.example.com:993".to_owned(),
            "connected".to_owned(),
        ] {
            assert!(screen.contains(&wanted), "{wanted} missing:\n{screen}");
        }
        assert!(
            !screen.contains("config.toml"),
            "nothing of it is in the file:\n{screen}"
        );
    }

    #[test]
    fn a_browser_sign_in_shows_the_whole_address_and_what_it_allows() {
        use crossterm::event::{KeyCode, KeyEvent};
        let mut app = crate::test_support::app((100, 40));
        update(&mut app, Input::Places(crate::places::Places::default()));
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
        let mut app = with_places((160, 16));
        let mut draft = postio_model::Draft::new(postio_model::AccountId::new(1));
        // What a reply copies from the message it answers.
        draft.subject = "Re: \u{1b}]0;pwned\u{7}\u{1b}[2J".into();
        app.compose(draft);
        let screen = screen(160, 16, &app);
        assert!(!screen.contains('\u{1b}'), "{screen:?}");
        assert!(screen.contains("Re:"), "{screen}");
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
        let mut app = with_places((160, 16));
        update(
            &mut app,
            Input::Host(postio_core::Event::Error {
                message: "The server refused the move".into(),
                account: None,
            }),
        );
        let (text, cells, theme) = status_of(&app, Colour::TrueColor);
        assert!(text.starts_with(" ✕ The server refused the move"), "{text}");
        assert_eq!(cells[3].fg, theme.style(Role::Error).fg.unwrap(), "{text}");

        update(
            &mut app,
            Input::Host(postio_core::Event::ActionCompleted {
                description: "Archived 1 message".into(),
                undoable: true,
            }),
        );
        let (text, cells, theme) = status_of(&app, Colour::TrueColor);
        assert!(
            text.starts_with(" ✓ Archived 1 message · Undo ctrl+z"),
            "{text}"
        );
        assert_eq!(cells[1].fg, theme.style(Role::Success).fg.unwrap());
        let characters: Vec<char> = text.chars().collect();
        let key = characters
            .windows(6)
            .position(|window| window.iter().collect::<String>() == "ctrl+z")
            .expect("the key");
        for cell in &cells[key..key + 6] {
            assert_eq!(
                cell.fg,
                theme.style(Role::Accent).fg.unwrap(),
                "the undo key in the accent: {text}"
            );
        }
        // Without colour the marks still say it.
        let (text, _, _) = status_of(&app, Colour::None);
        assert!(text.starts_with(" ✓"), "{text}");
    }

    #[test]
    fn the_readers_lines_are_wrapped_to_its_pane() {
        // One paragraph far wider than the reading pane: every word of it
        // must be on screen, none cut at the pane's right edge.
        let words: Vec<String> = (0..60).map(|n| format!("w{n:02}")).collect();
        let mut app = with_places((160, 30));
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

    fn window_of(size: (u16, u16)) -> String {
        let mut app = with_places(size);
        let opening = crate::test_support::open_list(&mut app, 6);
        crate::test_support::serve(&mut app, opening);
        screen(size.0, size.1, &app)
    }

    #[test]
    fn the_window_is_a_top_bar_a_strip_and_rows_with_no_sidebar_at_120_by_36() {
        let drawn = window_of((120, 36));
        let lines: Vec<&str> = drawn.lines().collect();
        assert!(lines[0].contains("Search mail"), "the top bar:\n{drawn}");
        assert!(
            lines[1].trim_start().starts_with("Inbox"),
            "the strip is under it:\n{drawn}"
        );
        assert!(
            lines[2].contains("Sunday 20 September"),
            "the day's heading starts the list:\n{drawn}"
        );
        assert!(lines[3].contains("Ada"), "rows follow it:\n{drawn}");
        assert!(drawn.contains("Message 5"), "{drawn}");
        for gone in ["ada@example.com", "Archive", "Saved searches"] {
            assert!(!drawn.contains(gone), "no sidebar, but {gone}:\n{drawn}");
        }
        assert!(
            lines.iter().all(|line| !line.contains('│')),
            "no pane rule:\n{drawn}"
        );
    }

    #[test]
    fn the_window_keeps_its_rows_at_the_minimum_size() {
        let drawn = window_of((50, 12));
        let lines: Vec<&str> = drawn.lines().collect();
        assert_eq!(lines.len(), 12);
        assert!(lines[0].contains("Search"), "the top bar:\n{drawn}");
        assert!(
            lines[1].trim_start().starts_with("Inbox"),
            "the strip:\n{drawn}"
        );
        assert!(lines[2].contains("Sunday"), "the heading:\n{drawn}");
        assert!(lines[3].contains("Ada"), "a row:\n{drawn}");
        assert!(!drawn.contains("ada@example.com"), "no sidebar:\n{drawn}");
    }

    #[test]
    fn below_the_minimum_the_screen_says_so_and_draws_nothing_else() {
        let drawn = window_of((49, 12));
        assert!(drawn.contains("Terminal too small: needs 50×12"), "{drawn}");
        assert!(!drawn.contains("Message 0"), "{drawn}");
    }
}
