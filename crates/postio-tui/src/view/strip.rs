//! The strip under the top bar: where the list is, how much is in it, and
//! what narrows it (terminal.md, "The screen").
//!
//! From the left: the place's name with `▾` and the key that opens the
//! places, the counts, the has-action toggle. From the right: the filtered
//! count and the digest rules, each only while its feature is in use (C10).
//! Narrowing drops the right side first, then the counts; the place and the
//! toggle stay. Each item is a click target for the command its key runs.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, Strip};
use crate::theme::{Role, Theme};
use crate::view::fit;
use crate::view::hit::{Hits, Target};
use postio_core::CommandId;

/// The gap between two items.
const GAP: usize = 3;

/// One item of the strip: its pieces, and the command a click runs.
struct Item<'a> {
    pieces: Vec<Span<'a>>,
    command: Option<&'static str>,
}

impl Item<'_> {
    fn width(&self) -> usize {
        self.pieces.iter().map(|piece| piece.content.width()).sum()
    }
}

/// A word and the key that runs it, as the strip words it: the key in the
/// accent after the word.
fn keyed<'a>(
    word: String,
    word_style: ratatui::style::Style,
    key: Option<String>,
    theme: &Theme,
) -> Vec<Span<'a>> {
    let mut pieces = vec![Span::styled(word, word_style)];
    if let Some(key) = key {
        pieces.push(Span::styled(format!(" {key}"), theme.style(Role::Accent)));
    }
    pieces
}

/// The strip's items, left side then right side.
fn items<'a>(app: &App, strip: &Strip, theme: &Theme) -> (Vec<Item<'a>>, Vec<Item<'a>>) {
    let mut place = vec![Span::styled(
        strip.place.as_str().to_owned(),
        theme.style(Role::Text).add_modifier(Modifier::BOLD),
    )];
    place.push(Span::styled(" ▾", theme.style(Role::Dim)));
    if let Some(key) = app.hint(CommandId::GoToFolders) {
        place.push(Span::styled(format!(" {key}"), theme.style(Role::Accent)));
    }
    let mut left = vec![Item {
        pieces: place,
        command: Some("go_to_folders"),
    }];
    left.push(Item {
        pieces: vec![Span::styled(strip.counts.clone(), theme.style(Role::Dim))],
        command: None,
    });
    if let Some(toggle) = &strip.toggle {
        // The accent while it is on, reversed where there is no colour.
        let style = if toggle.on {
            theme
                .style(Role::Accent)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            theme.style(Role::Text)
        };
        let mut pieces = vec![Span::styled(format!("⚑ {}", toggle.label), style)];
        if let Some(key) = app.hint(CommandId::ToggleHasAction) {
            pieces.push(Span::styled(format!(" {key}"), theme.style(Role::Accent)));
        }
        left.push(Item {
            pieces,
            command: Some("toggle_has_action"),
        });
        if let Some(showing) = &strip.showing {
            left.push(Item {
                pieces: vec![Span::styled(showing.clone(), theme.style(Role::Dim))],
                command: None,
            });
        }
    }
    let mut right = Vec::new();
    if let Some(filtered) = &strip.filtered {
        right.push(Item {
            pieces: keyed(
                filtered.clone(),
                theme.style(Role::Dim),
                app.hint(CommandId::GoToFiltered),
                theme,
            ),
            command: Some("go_to_filtered"),
        });
    }
    if let Some(rules) = &strip.rules {
        right.push(Item {
            pieces: keyed(
                rules.clone(),
                theme.style(Role::Dim),
                app.hint(CommandId::GoToDigestRules),
                theme,
            ),
            command: Some("go_to_digest_rules"),
        });
    }
    (left, right)
}

/// The width `items` take with a gap between each.
fn span_of(items: &[Item]) -> usize {
    items.iter().map(Item::width).sum::<usize>() + GAP * items.len().saturating_sub(1)
}

/// Draw the strip into `area`, one row, on the surface background.
pub fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, hits: &mut Hits) {
    // A ground of its own where there is colour to draw one; reversed video
    // is kept for the toggle, which would cancel against it.
    if theme.style(Role::Surface).bg.is_some() {
        frame
            .buffer_mut()
            .set_style(area, theme.style(Role::Surface));
    }
    let strip = app.strip();
    let (mut left, mut right) = items(app, &strip, theme);
    let width = usize::from(area.width).saturating_sub(2);
    // Narrowing: the right side goes first, then the counts, and then what
    // follows the toggle.
    if span_of(&left) + GAP + span_of(&right) > width {
        right.clear();
    }
    if span_of(&left) > width {
        left.retain(|item| item.command.is_some());
    }
    if span_of(&left) > width
        && let Some(place) = left.first_mut()
    {
        // The place's name gives way last, cut.
        let room = width.saturating_sub(4);
        if let Some(name) = place.pieces.first_mut() {
            *name = Span::styled(fit(&name.content, room), name.style);
        }
    }
    let mut x = area.x + 1;
    let mut place = |item: Item, x: &mut u16| {
        let at = *x;
        let w = u16::try_from(item.width()).unwrap_or(u16::MAX);
        let command = item.command;
        frame.render_widget(Line::from(item.pieces), Rect::new(at, area.y, w, 1));
        if let Some(command) = command {
            hits.add(Rect::new(at, area.y, w, 1), Target::Command(command));
        }
        *x = at + w + GAP as u16;
    };
    for item in left {
        place(item, &mut x);
    }
    let total = u16::try_from(span_of(&right)).unwrap_or(0);
    let mut x = (area.x + area.width).saturating_sub(total + 1);
    for item in right {
        place(item, &mut x);
    }
}

#[cfg(test)]
mod tests {
    use postio_model::FocusScope;
    use postio_ui::focus_list::FocusRow;

    use crate::app::{App, Input, update};
    use crate::test_support::{
        app, conversation, drawing_counts, hits_of, local, marked, places_with_features, press,
        screen, seed_places, show_focus, show_scope,
    };
    use crate::view::hit::Target;

    /// The inbox of the drawing, counted.
    fn counted(size: (u16, u16)) -> App {
        let mut app = app(size);
        seed_places(&mut app, places_with_features());
        let rows = (1..=3)
            .map(|id| {
                FocusRow::conversation(conversation(
                    id,
                    "Ada",
                    &format!("Subject {id}"),
                    "",
                    local(23, 9, 0),
                ))
            })
            .collect();
        show_focus(&mut app, rows);
        update(&mut app, Input::FocusCounts(drawing_counts()));
        app
    }

    fn strip_of(app: &App, size: (u16, u16)) -> String {
        screen(size.0, size.1, app)
            .lines()
            .nth(1)
            .expect("a strip")
            .to_owned()
    }

    #[test]
    fn the_strip_names_the_place_its_counts_the_toggle_and_the_features_in_use() {
        let strip = strip_of(&counted((120, 30)), (120, 30));
        assert!(
            strip.contains("Inbox ▾ g o   312 · 41 unread   ⚑ Has action · 7 !"),
            "{strip}"
        );
        assert!(
            strip
                .trim_end()
                .ends_with("186 filtered today g f   4 digest rules g d"),
            "{strip}"
        );
    }

    #[test]
    fn a_count_shows_only_while_its_feature_is_in_use() {
        // Filtering off and no rules: neither says anything.
        let mut app = app((120, 30));
        seed_places(&mut app, crate::test_support::places());
        show_focus(&mut app, Vec::new());
        update(&mut app, Input::FocusCounts(drawing_counts()));
        let strip = strip_of(&app, (120, 30));
        assert!(!strip.contains("filtered"), "{strip}");
        assert!(!strip.contains("digest rule"), "{strip}");
        // Filtering on, but nothing filed today.
        let mut quiet = counted((120, 30));
        update(
            &mut quiet,
            Input::FocusCounts(postio_client::protocol::FocusCounts {
                filtered_today: 0,
                ..drawing_counts()
            }),
        );
        let strip = strip_of(&quiet, (120, 30));
        assert!(!strip.contains("filtered"), "{strip}");
        assert!(strip.contains("4 digest rules g d"), "{strip}");
    }

    #[test]
    fn narrowing_drops_the_right_side_first_and_then_the_counts() {
        let mut app = counted((120, 30));
        update(&mut app, Input::Resize(80, 30));
        let strip = strip_of(&app, (80, 30));
        assert!(strip.contains("312 · 41 unread"), "{strip}");
        assert!(strip.contains("Has action · 7"), "{strip}");
        assert!(!strip.contains("filtered"), "the right side went: {strip}");
        update(&mut app, Input::Resize(50, 30));
        let strip = strip_of(&app, (50, 30));
        assert!(strip.contains("Inbox ▾ g o"), "{strip}");
        assert!(
            strip.contains("Has action · 7 !"),
            "the toggle stays: {strip}"
        );
        assert!(!strip.contains("unread"), "the counts went: {strip}");
    }

    #[test]
    fn the_strips_items_are_click_targets() {
        let app = counted((120, 30));
        let strip = strip_of(&app, (120, 30));
        let hits = hits_of(120, 30, &app);
        let at = |needle: &str| {
            let x = strip.find(needle).expect(needle);
            let x = u16::try_from(strip[..x].chars().count()).unwrap();
            hits.at(x, 1).map(|hit| hit.target)
        };
        assert_eq!(at("Inbox"), Some(Target::Command("go_to_folders")));
        assert_eq!(at("Has action"), Some(Target::Command("toggle_has_action")));
        assert_eq!(at("filtered"), Some(Target::Command("go_to_filtered")));
        assert_eq!(
            at("digest rules"),
            Some(Target::Command("go_to_digest_rules"))
        );
    }

    #[test]
    fn bang_narrows_to_the_rows_with_a_marker_and_back_keeping_the_cursor_and_dropping_the_selection()
     {
        let mut app = counted((120, 30));
        update(&mut app, press('j'));
        update(&mut app, press('x'));
        assert!(!app.selection().selection().is_empty());
        let effects = update(&mut app, press('!'));
        assert!(
            effects.contains(&crate::app::Effect::Open(postio_model::ListScope::Focus(
                FocusScope::HasAction
            ))),
            "{effects:?}"
        );
        assert!(app.selection().selection().is_empty(), "the selection goes");
        // The narrowed list holds the second and third message.
        let marker = postio_model::listing::MarkerSummary {
            kind: postio_model::listing::MarkerKind::Question,
            when: None,
            excerpt: Some("Can you?".into()),
            answer: None,
            cancelled: false,
        };
        let rows = (2..=3)
            .map(|id| {
                FocusRow::conversation(marked(
                    conversation(id, "Ada", &format!("Subject {id}"), "", local(23, 9, 0)),
                    marker.clone(),
                ))
            })
            .collect();
        show_scope(&mut app, FocusScope::HasAction, rows);
        assert!(app.has_action());
        assert_eq!(app.cursor(), 0, "the cursor is on the message it was on");
        let drawn = screen(120, 30, &app);
        let lines: Vec<&str> = drawn.lines().collect();
        assert!(
            lines[1].contains("Showing 7 of 312 · ! again to show all"),
            "{drawn}"
        );
        assert!(lines[2].contains("Has action · 7"), "one heading: {drawn}");
        assert!(!lines[2].contains("Today"), "not a day's: {drawn}");
        assert_eq!(drawn.matches("Has action · 7").count(), 2, "{drawn}");
        // Back again.
        let effects = update(&mut app, press('!'));
        assert!(
            effects.contains(&crate::app::Effect::Open(postio_model::ListScope::Focus(
                FocusScope::Inbox
            ))),
            "{effects:?}"
        );
    }

    #[test]
    fn the_toggle_is_the_accent_while_on_and_reversed_without_colour() {
        use crate::caps::{Background, Colour};
        use crate::theme::{Role, Theme};
        let mut app = counted((120, 30));
        let marked_row = FocusRow::conversation(marked(
            conversation(2, "Ada", "Subject 2", "", local(23, 9, 0)),
            postio_model::listing::MarkerSummary {
                kind: postio_model::listing::MarkerKind::Question,
                when: None,
                excerpt: None,
                answer: None,
                cancelled: false,
            },
        ));
        show_scope(&mut app, FocusScope::HasAction, vec![marked_row]);
        for colour in [Colour::TrueColor, Colour::None] {
            let theme = Theme::new(colour, Background::Dark, &Default::default()).0;
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 30)).unwrap();
            terminal
                .draw(|frame| {
                    crate::view::draw(frame, &app, &theme, crate::test_support::now());
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            let strip: String = (0..120)
                .map(|x| buffer[(x, 1)].symbol().to_owned())
                .collect();
            let x = u16::try_from(strip[..strip.find('⚑').unwrap()].chars().count()).unwrap();
            let cell = &buffer[(x, 1)];
            assert!(
                cell.modifier.contains(ratatui::style::Modifier::REVERSED),
                "{colour:?}: on is reversed"
            );
            if let Some(accent) = theme.style(Role::Accent).fg {
                assert_eq!(cell.fg, accent, "{colour:?}: in the accent");
            }
            assert!(
                !buffer[(2, 1)]
                    .modifier
                    .contains(ratatui::style::Modifier::REVERSED)
                    || colour != Colour::None,
                "the rest of the strip is not reversed without colour"
            );
        }
        // Off, it is plain.
        let off = counted((120, 30));
        let strip = strip_of(&off, (120, 30));
        assert!(strip.contains("⚑ Has action"));
    }
}
