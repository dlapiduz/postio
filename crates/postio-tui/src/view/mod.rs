//! Drawing: the app's state onto a ratatui frame.
//!
//! Nothing here decides anything about mail; it draws what `App` holds.

pub mod bottom;
pub mod cheatsheet;
pub mod composer;
pub mod first_run;
pub mod hit;
pub mod list;
pub mod palette;
pub mod reader;
pub mod search;
pub mod settings;
pub mod strip;
pub mod topbar;
pub mod wrap;

use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;

use crate::app::{App, Focus, Front};
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
    } else if let Some(run) = app.first_run() {
        first_run::draw(frame, area, run, theme);
    } else {
        let window = app.window();
        topbar::draw(frame, window.top, app, theme, &mut hits);
        // A draft in a tab of its own has the whole screen between the top
        // bar and the bottom line while it is in front.
        let tab = app.composer_detached() && app.focus() == Focus::Composer;
        if !tab {
            strip::draw(frame, window.strip, app, theme, &mut hits);
        }
        let below = window.strip.y + u16::from(!tab);
        let body = Rect::new(
            area.x,
            below,
            area.width,
            window.bottom.y.saturating_sub(below),
        );
        match app.front() {
            Front::Composer if app.composer().is_some() => {
                if let Some(writing) = app.composer() {
                    composer::draw(
                        frame,
                        body,
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
                        composer::draw_schedule(frame, body, times, theme, now);
                    }
                    if let Some(typed) = app.path_prompt() {
                        composer::draw_path_prompt(frame, body, typed, theme);
                    }
                }
            }
            // T315: the open message becomes the overlay frame; until then
            // it fills the body.
            Front::Reader => reader::draw(frame, body, app, theme, now, &mut hits),
            _ => {
                // A search's facets take the list's first line.
                let facets = app.facets();
                let list = if facets.is_empty() || window.list.height < 2 {
                    window.list
                } else {
                    search::draw_facets(
                        frame,
                        Rect::new(window.list.x, window.list.y, window.list.width, 1),
                        &facets,
                        app.facets_note(),
                        theme,
                        &mut hits,
                    );
                    Rect::new(
                        window.list.x,
                        window.list.y + 1,
                        window.list.width,
                        window.list.height - 1,
                    )
                };
                list::draw(
                    frame,
                    list,
                    &app.visible(),
                    app.top(),
                    theme,
                    now,
                    &|command| app.hint(command),
                    &mut hits,
                );
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
    if app.composer_detached() && !tab {
        spans.push(Span::styled(
            match app.hint(postio_core::CommandId::Compose) {
                Some(key) => format!("✎ A draft is open — {key} goes back to it"),
                None => "✎ A draft is open".to_owned(),
            },
            theme.style(Role::Text),
        ));
    }
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
    fn a_detached_draft_has_the_screen_and_the_mail_says_it_is_open() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = with_places((160, 16));
        let opening = crate::test_support::open_list(&mut app, 3);
        crate::test_support::serve(&mut app, opening);
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
        let mut app = with_places((160, 16));
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
        let mut app = with_places((160, 16));
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
            !bar.contains("go to a folder"),
            "the query replaces the placeholder: {bar}"
        );
    }

    #[test]
    fn a_searchs_facets_sit_over_its_results_and_take_a_click() {
        use crossterm::event::{KeyCode, KeyEvent};
        use postio_search::facets::{Facets, Refinement, Scope, ScopeCount};
        let mut app = with_places((160, 16));
        update(&mut app, Input::Key(KeyEvent::from(KeyCode::Char('/'))));
        let sequence = update(&mut app, Input::Key(KeyEvent::from(KeyCode::Char('t'))))
            .iter()
            .find_map(|effect| match effect {
                crate::app::Effect::Search { sequence, .. } => Some(*sequence),
                _ => None,
            })
            .expect("searched");
        update(
            &mut app,
            Input::Found {
                sequence,
                found: Ok(Some(postio_client::protocol::Found {
                    ids: vec![
                        postio_model::MessageId::new(4),
                        postio_model::MessageId::new(5),
                    ],
                    hits: 2,
                    capped: false,
                    corpus_complete: true,
                    elapsed: std::time::Duration::from_millis(7),
                })),
            },
        );
        update(
            &mut app,
            Input::Facets {
                sequence,
                facets: Some(Facets {
                    scopes: vec![
                        ScopeCount {
                            scope: Scope::AllMail,
                            hits: 2,
                        },
                        ScopeCount {
                            scope: Scope::Inbox,
                            hits: 1,
                        },
                    ],
                    refinements: vec![Refinement {
                        token: "is:unread".into(),
                        hits: 1,
                    }],
                }),
            },
        );
        let mut hits = hit::Hits::default();
        let drawn = {
            let backend = ratatui::backend::TestBackend::new(160, 16);
            let mut terminal = ratatui::Terminal::new(backend).unwrap();
            let theme = Theme::new(
                crate::caps::Colour::None,
                crate::caps::Background::Unknown,
                &Default::default(),
            )
            .0;
            terminal
                .draw(|frame| hits = draw(frame, &app, &theme, chrono::Local::now()))
                .unwrap();
            screen(160, 16, &app)
        };
        let facets = drawn
            .lines()
            .find(|line| line.contains("All mail"))
            .unwrap_or_else(|| panic!("no facet row:\n{drawn}"));
        for wanted in ["All mail 2", "Inbox only 1", "Lists 0", "is:unread 1"] {
            assert!(facets.contains(wanted), "{wanted} missing: {facets}");
        }
        let row = u16::try_from(
            drawn
                .lines()
                .position(|line| line.contains("All mail"))
                .unwrap(),
        )
        .unwrap();
        let at = facets.find("is:unread").unwrap();
        let column = u16::try_from(facets[..at].chars().count()).unwrap();
        let hit = hits.at(column, row).expect("the chip takes a click");
        assert_eq!(hit.target, hit::Target::Facet(3));

        // Nothing to narrow by: said, not left blank.
        update(
            &mut app,
            Input::Facets {
                sequence,
                facets: Some(Facets::default()),
            },
        );
        let drawn = screen(160, 16, &app);
        assert!(
            drawn.contains("Every match is alike"),
            "said, as far as the pane is wide:\n{drawn}"
        );
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
    fn the_palette_draws_its_rows_with_their_keys() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = with_places((160, 24));
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
        let mut app = with_places((200, 90));
        update(&mut app, Input::Key(KeyEvent::from(KeyCode::Char('?'))));
        let screen = screen(200, 90, &app);
        let keymap = postio_core::Keymap::resolve(&Default::default());
        let sections = postio_ui::cheatsheet::sections(
            &keymap,
            postio_core::Context::List,
            // No list is open here, so the view is unified, where a move has
            // no account to move within (#182). As this terminal asks.
            postio_core::Availability {
                frontend: postio_core::Frontend::Terminal,
                ..postio_core::Availability::open(postio_core::Scope::Unified)
            },
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

    #[test]
    fn an_opened_message_fills_the_body() {
        use chrono::Utc;
        use postio_ui::paging::Page;
        let mut app = with_places((160, 12));
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
            unread: true,
            address: None,
            ..crate::test_support::row_from(7, "Ada Lovelace", "Engine notes", "", Utc::now())
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
        app.open_reading(postio_model::MessageId::new(7));
        update(
            &mut app,
            crate::test_support::key(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE,
            ),
        );
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
            1,
            "the open message fills the body and its subject heads it:\n{screen}"
        );
    }

    #[test]
    fn the_reader_is_headed_by_who_wrote_to_whom_and_when_and_offers_its_keys() {
        use chrono::Utc;
        use postio_ui::paging::Page;
        let mut app = with_places((160, 20));
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
        let row = crate::test_support::row_from(
            7,
            "Ada Lovelace",
            "Engine notes",
            "",
            Utc.with_ymd_and_hms(2026, 9, 22, 9, 14, 0).unwrap(),
        );
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
        app.open_reading(postio_model::MessageId::new(7));
        update(
            &mut app,
            crate::test_support::key(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE,
            ),
        );
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
        let reader: Vec<String> = screen.lines().map(str::to_owned).collect();
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
