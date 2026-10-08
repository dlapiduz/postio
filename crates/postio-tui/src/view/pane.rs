//! Reading beside the list (terminal.md, "Reading beside the list").
//!
//! From 128 columns, `[focus] reading = "pane"` puts the open message to the
//! right of the list: its header rows under the strip, a `│` between the
//! two, no border. With nothing open the pane says so, and how to open one.

use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;

use crate::app::App;
use crate::theme::{Role, Theme};
use crate::view::hit::{Hits, Target};

/// Draw the pane into `area`: the divider down its left column, then the
/// open message, or the page that says none is open.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    theme: &Theme,
    now: DateTime<Local>,
    hits: &mut Hits,
) {
    if area.width < 12 {
        return;
    }
    for y in area.y..area.y + area.height {
        frame.render_widget(
            Line::styled("│", theme.style(Role::Dim)),
            Rect::new(area.x, y, 1, 1),
        );
    }
    let inside = Rect::new(area.x + 1, area.y, area.width - 1, area.height);
    hits.add(inside, Target::Overlay);
    if app.reading().is_some() {
        crate::view::open::draw(frame, inside, area.width, app, theme, now, hits);
        if let Some(menu) = app.menu() {
            crate::view::open::draw_menu(frame, inside, menu, theme, hits);
        }
    } else {
        let said = postio_ui::focus_state::empty_pane(app.keymap());
        crate::view::state::empty(frame, inside, &said, theme, hits);
    }
}

#[cfg(test)]
mod tests {
    use postio_model::{MessageId, ThreadId};
    use postio_ui::focus_list::FocusRow;

    use crate::app::{App, Effect, Input, Pointer, update};
    use crate::test_support::{
        app, conversation, hits_of, key, local, places, press, screen, seed_places, show_focus,
        unread,
    };
    use crossterm::event::{KeyCode, KeyModifiers};

    /// Pane mode, as `[focus] reading = "pane"` puts it.
    fn pane_app(size: (u16, u16)) -> App {
        let mut app = app(size);
        let mut contents = places();
        contents.features.reading = postio_config::Reading::Pane;
        seed_places(&mut app, contents);
        let rows = (1..=4)
            .map(|id| {
                FocusRow::conversation(unread(conversation(
                    id,
                    "Lena Park",
                    &format!("Harbor note {id}"),
                    "a first line of some length that would run on and on and on",
                    local(23, 9, id as u32),
                )))
            })
            .collect();
        show_focus(&mut app, rows);
        app
    }

    fn body(message: i64, text: &str) -> Input {
        Input::Body {
            message: MessageId::new(message),
            answer: Ok(postio_client::protocol::Body::Ready {
                body: postio_model::MessageBody {
                    text: Some(text.into()),
                    html: None,
                },
                encoding_problems: false,
            }),
        }
    }

    fn enter() -> Input {
        key(KeyCode::Enter, KeyModifiers::NONE)
    }

    #[test]
    fn at_128_columns_the_message_opens_beside_the_list_with_a_divider_and_no_border() {
        let mut app = pane_app((128, 30));
        update(&mut app, enter());
        update(&mut app, body(1, "Hello from the pane"));
        let drawn = screen(128, 30, &app);
        let lines: Vec<&str> = drawn.lines().collect();
        assert!(!drawn.contains('╭'), "borderless:\n{drawn}");
        // The pane is 72 wide, the list keeps 56, the divider is the pane's
        // first column.
        for (y, line) in lines.iter().enumerate().take(29).skip(2) {
            assert_eq!(line.chars().nth(56), Some('│'), "row {y}:\n{drawn}");
        }
        assert!(
            lines[2]
                .chars()
                .skip(57)
                .collect::<String>()
                .contains("Harbor note 1"),
            "the header sits under the strip:\n{drawn}"
        );
        assert!(drawn.contains("Hello from the pane"), "{drawn}");
        assert!(lines[2].contains("↑ k  ↓ j"), "{drawn}");
        // The list is still there, narrow, with its trailing column.
        assert!(lines[4].contains("Lena Park"), "{drawn}");
    }

    #[test]
    fn beside_the_pane_the_senders_column_is_twelve() {
        let app = pane_app((128, 30));
        let drawn = screen(128, 30, &app);
        let row = drawn.lines().find(|l| l.contains("Harbor note 1")).unwrap();
        let at = row[..row.find("Harbor note 1").unwrap()].chars().count();
        // Cursor, mark and a space, then twelve.
        assert_eq!(at, 3 + 12, "{drawn}");
    }

    #[test]
    fn with_nothing_open_the_pane_says_how_to_open_one() {
        let app = pane_app((140, 30));
        let drawn = screen(140, 30, &app);
        assert!(drawn.contains("No message open"), "{drawn}");
        assert!(drawn.contains("Read over the list"), "{drawn}");
    }

    #[test]
    fn narrower_than_128_the_message_opens_in_the_overlay_and_the_setting_is_kept() {
        let mut app = pane_app((127, 30));
        update(&mut app, enter());
        let drawn = screen(127, 30, &app);
        assert!(drawn.contains('╭'), "{drawn}");
        assert!(!drawn.contains("No message open"), "{drawn}");
    }

    #[test]
    fn crossing_the_line_moves_an_open_message_still_open() {
        let mut app = pane_app((120, 30));
        update(&mut app, enter());
        update(&mut app, body(1, "Still here"));
        assert!(screen(120, 30, &app).contains('╭'));
        update(&mut app, Input::Resize(140, 30));
        let drawn = screen(140, 30, &app);
        assert!(
            !drawn.contains('╭') && drawn.contains("Still here"),
            "{drawn}"
        );
        assert_eq!(app.front(), crate::app::Front::Reader);
        update(&mut app, Input::Resize(100, 30));
        let drawn = screen(100, 30, &app);
        assert!(
            drawn.contains('╭') && drawn.contains("Still here"),
            "{drawn}"
        );
    }

    #[test]
    fn the_message_follows_the_cursor_and_escape_leaves_the_pane_empty() {
        let mut app = pane_app((140, 30));
        update(&mut app, enter());
        let effects = update(&mut app, press('j'));
        assert!(
            effects.contains(&Effect::ReadBody(MessageId::new(2))),
            "{effects:?}"
        );
        update(&mut app, body(2, "Second"));
        assert!(screen(140, 30, &app).contains("Second"));
        // A click on a row is the pointer's j.
        let hit = hits_of(140, 30, &app).at(5, 5).expect("a row");
        let effects = update(
            &mut app,
            Input::Pointer(Pointer::Click {
                hit,
                ctrl: false,
                shift: false,
            }),
        );
        assert_eq!(app.cursor(), 2, "{effects:?}");
        assert!(
            effects.contains(&Effect::ReadBody(MessageId::new(3))),
            "{effects:?}"
        );
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(screen(140, 30, &app).contains("No message open"));
        assert_eq!(app.cursor(), 2);
    }

    #[test]
    fn f8_chooses_where_messages_open_writes_it_and_says_so() {
        let mut app = app((140, 30));
        seed_places(&mut app, places());
        let effects = update(&mut app, key(KeyCode::F(8), KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::SetReading(postio_config::Reading::Pane)),
            "{effects:?}"
        );
        assert_eq!(app.notice(), Some("Messages open beside the list"));
        let effects = update(&mut app, key(KeyCode::F(8), KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::SetReading(postio_config::Reading::Dialog)),
            "{effects:?}"
        );
        assert_eq!(app.notice(), Some("Messages open over the list"));
        // Too narrow for a pane: the choice is made and the words say when.
        let mut narrow = crate::test_support::app((100, 30));
        seed_places(&mut narrow, places());
        update(&mut narrow, key(KeyCode::F(8), KeyModifiers::NONE));
        assert_eq!(
            narrow.notice(),
            Some("Messages open beside the list once the window is wider")
        );
    }

    #[test]
    fn the_wheel_scrolls_the_pane_and_the_list_keeps_its_place() {
        let mut app = pane_app((140, 30));
        update(&mut app, enter());
        let long: Vec<String> = (0..90).map(|n| format!("paragraph {n}")).collect();
        update(&mut app, body(1, &long.join("\n")));
        let hit = hits_of(140, 30, &app).at(100, 15).expect("the pane");
        update(&mut app, Input::Pointer(Pointer::Wheel { hit, down: true }));
        assert!(app.reader_top() > 0);
        assert_eq!(app.top(), 0);
        let _ = ThreadId::new(1);
    }
}
