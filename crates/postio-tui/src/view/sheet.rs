//! The key map, drawn (terminal.md, "Key map"): a framed sheet over
//! everything but the top bar, its groups in columns 38 wide, as many as
//! fit, scrolling when they do not.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line as Text, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear};
use unicode_width::UnicodeWidthStr;

use crate::sheet::{COLUMN, GAP, Line, Sheet};
use crate::theme::{Role, Theme};
use crate::view::fit;
use crate::view::hit::{Hits, Target};
use postio_core::CommandId;
use postio_ui::keymap_sheet;

/// The sheet's frame: the whole screen but the top bar.
pub fn frame_of(area: Rect) -> Rect {
    Rect::new(
        area.x,
        area.y + 1,
        area.width,
        area.height.saturating_sub(1),
    )
}

/// Where the columns go: inside the frame and its padding, under the header
/// and over the footer.
pub fn body(area: Rect) -> Rect {
    let frame = frame_of(area);
    Rect::new(
        frame.x + 2,
        frame.y + 4,
        frame.width.saturating_sub(4),
        frame.height.saturating_sub(6),
    )
}

/// Draw the sheet.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    sheet: &Sheet,
    columns: &[Vec<Line>],
    hint: &dyn Fn(CommandId) -> Option<String>,
    theme: &Theme,
    hits: &mut Hits,
) {
    let outer = frame_of(area);
    if outer.width < COLUMN + 4 || outer.height < 8 {
        return;
    }
    frame.render_widget(Clear, outer);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.style(Role::Dim)),
        outer,
    );
    hits.add(outer, Target::Overlay);
    let dim = theme.style(Role::Dim);
    let text = theme.style(Role::Text);
    let accent = theme.style(Role::Accent);
    let inner_x = outer.x + 2;
    let inner_width = outer.width - 4;
    frame.render_widget(
        Text::styled(keymap_sheet::TITLE, text.add_modifier(Modifier::BOLD)),
        Rect::new(inner_x, outer.y + 1, inner_width, 1),
    );
    // The keys that close it, then the close mark.
    let mut spans = Vec::new();
    for (index, command) in keymap_sheet::CLOSE_COMMANDS.into_iter().enumerate() {
        let Some(key) = hint(command) else { continue };
        if index > 0 && !spans.is_empty() {
            spans.push(Span::styled(format!(" {} ", keymap_sheet::CLOSE_OR), dim));
        }
        spans.push(Span::styled(key.replace("Escape", "Esc"), accent));
    }
    spans.push(Span::styled(
        format!(" {}  ", keymap_sheet::CLOSE_WORD),
        dim,
    ));
    let width = u16::try_from(spans.iter().map(|span| span.content.width()).sum::<usize>())
        .unwrap_or(0)
        + 1;
    let at = outer.x + outer.width - 2 - width;
    frame.render_widget(Text::from(spans), Rect::new(at, outer.y + 1, width, 1));
    let mark = Rect::new(outer.x + outer.width - 3, outer.y + 1, 1, 1);
    frame.render_widget(Text::styled("✕", text), mark);
    hits.add(mark, Target::Command("cheat_sheet"));
    frame.render_widget(
        Text::styled(fit(keymap_sheet::SUBTITLE, usize::from(inner_width)), dim),
        Rect::new(inner_x, outer.y + 2, inner_width, 1),
    );

    let body = body(area);
    let shown = usize::from(body.height);
    let scroll = sheet.scroll();
    for (index, column) in columns.iter().enumerate() {
        let x = body.x + u16::try_from(index).unwrap_or(0) * (COLUMN + GAP);
        for (offset, line) in column.iter().skip(scroll).take(shown).enumerate() {
            let row = Rect::new(x, body.y + u16::try_from(offset).unwrap_or(0), COLUMN, 1);
            match line {
                Line::Blank => {}
                Line::Heading(title) => frame
                    .render_widget(Text::styled(*title, text.add_modifier(Modifier::BOLD)), row),
                Line::Row { title, keys } => {
                    let keys = fit(keys, usize::from(COLUMN) / 2);
                    let room = usize::from(COLUMN).saturating_sub(keys.width() + 1);
                    let title = fit(title, room);
                    let gap = usize::from(COLUMN).saturating_sub(title.width() + keys.width());
                    frame.render_widget(
                        Text::from(vec![
                            Span::styled(title, text),
                            Span::raw(" ".repeat(gap)),
                            Span::styled(keys, accent),
                        ]),
                        row,
                    );
                }
            }
        }
    }
    let footer = Rect::new(inner_x, outer.y + outer.height - 2, inner_width, 1);
    frame.render_widget(Text::styled(keymap_sheet::REBIND_FOOTER, dim), footer);
    let tallest = crate::sheet::height(columns);
    if tallest > shown {
        let hint = "↑↓ scroll";
        let w = u16::try_from(hint.width()).unwrap_or(0);
        if w + 2 < inner_width {
            frame.render_widget(
                Text::styled(hint, dim),
                Rect::new(footer.x + footer.width - w, footer.y, w, 1),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};
    use postio_ui::keymap_sheet;

    use crate::app::{App, Focus, update};
    use crate::test_support::{
        app, click, hits_of, key, open_list, places, press, screen, seed_places, serve, wheel,
    };
    use crate::view::hit::Target;

    fn opened(size: (u16, u16)) -> App {
        let mut app = app(size);
        seed_places(&mut app, places());
        let opening = open_list(&mut app, 3);
        serve(&mut app, opening);
        app
    }

    fn map() -> Vec<(keymap_sheet::Group, Vec<keymap_sheet::KeyMapRow>)> {
        keymap_sheet::key_map(
            &postio_core::Keymap::for_terminal(&Default::default()),
            postio_core::Frontend::Terminal,
        )
    }

    #[test]
    fn question_mark_opens_every_group_under_the_top_bar_with_the_shared_words() {
        let mut app = opened((200, 90));
        update(&mut app, press('?'));
        let drawn = screen(200, 90, &app);
        assert!(
            drawn.lines().next().unwrap().contains("Compose"),
            "the top bar stays:\n{drawn}"
        );
        assert!(drawn.contains(keymap_sheet::TITLE), "{drawn}");
        assert!(drawn.contains(&keymap_sheet::SUBTITLE[..40]), "{drawn}");
        assert!(drawn.contains(keymap_sheet::REBIND_FOOTER), "{drawn}");
        for (group, rows) in map() {
            assert!(
                drawn.contains(group.title()),
                "{} missing:\n{drawn}",
                group.title()
            );
            for row in rows.iter().take(3) {
                assert!(
                    drawn.lines().any(|line| line.contains(row.title)),
                    "{} missing:\n{drawn}",
                    row.title
                );
            }
        }
        let archive = drawn
            .lines()
            .find(|line| line.contains("Archive thread"))
            .unwrap();
        assert!(archive.contains('A'), "{archive}");
    }

    #[test]
    fn it_has_as_many_columns_38_wide_as_fit() {
        let mut app = opened((200, 90));
        update(&mut app, press('?'));
        let drawn = screen(200, 90, &app);
        let columns: std::collections::BTreeSet<usize> = map()
            .iter()
            .filter_map(|(group, _)| {
                drawn
                    .lines()
                    .find_map(|line| line.find(group.title()))
                    .map(|byte| {
                        drawn.lines().find(|l| l.contains(group.title())).unwrap()[..byte]
                            .chars()
                            .count()
                    })
            })
            .collect();
        assert!(columns.len() > 1, "groups sit side by side: {columns:?}");
        let starts: Vec<usize> = columns.into_iter().collect();
        for pair in starts.windows(2) {
            assert_eq!(pair[1] - pair[0], 40, "38 wide and a gap of 2: {starts:?}");
        }
        let narrow = {
            let mut app = opened((60, 30));
            update(&mut app, press('?'));
            screen(60, 30, &app)
        };
        let starts: std::collections::BTreeSet<usize> = narrow
            .lines()
            .filter_map(|line| line.find("Move and select").or_else(|| line.find("Open")))
            .collect();
        assert_eq!(starts.len(), 1, "one column at 60:\n{narrow}");
    }

    #[test]
    fn what_does_not_fit_scrolls_and_the_wheel_and_arrows_move_it() {
        let mut app = opened((60, 17));
        update(&mut app, press('?'));
        let before = screen(60, 17, &app);
        assert!(before.contains("Move and select"), "{before}");
        assert!(!before.contains("Digests and filtering"), "{before}");
        for _ in 0..80 {
            update(&mut app, key(KeyCode::Down, KeyModifiers::NONE));
        }
        let after = screen(60, 17, &app);
        assert!(!after.contains("Move and select"), "{after}");
        assert!(
            after.contains("Digests and filtering") || after.contains("In search"),
            "{after}"
        );
        for _ in 0..80 {
            update(&mut app, wheel(Target::Overlay, false));
        }
        assert!(screen(60, 17, &app).contains("Move and select"));
    }

    #[test]
    fn question_mark_and_escape_close_and_other_keys_do_nothing() {
        let mut app = opened((120, 40));
        update(&mut app, press('?'));
        assert_eq!(app.focus(), Focus::Keys);
        let effects = update(&mut app, press('a'));
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, crate::app::Effect::Send(_))),
            "{effects:?}"
        );
        assert!(screen(120, 40, &app).contains(keymap_sheet::TITLE));
        update(&mut app, press('?'));
        assert_eq!(app.focus(), Focus::List);
        update(&mut app, press('?'));
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::List);
        assert!(!screen(120, 40, &app).contains(keymap_sheet::REBIND_FOOTER));
    }

    #[test]
    fn the_close_mark_is_a_click_and_a_click_elsewhere_does_nothing() {
        let mut app = opened((120, 40));
        update(&mut app, press('?'));
        let hits = hits_of(120, 40, &app);
        let drawn = screen(120, 40, &app);
        let (row, line) = drawn
            .lines()
            .enumerate()
            .find(|(_, l)| l.contains('✕'))
            .expect("a close mark");
        let column = line.chars().position(|c| c == '✕').unwrap();
        let at = hits
            .at(u16::try_from(column).unwrap(), u16::try_from(row).unwrap())
            .unwrap();
        let elsewhere = hits.at(5, 20).unwrap();
        let effects = update(&mut app, click(elsewhere.target, false, false));
        assert!(effects.is_empty());
        assert_eq!(app.focus(), Focus::Keys);
        update(&mut app, click(at.target, false, false));
        assert_eq!(app.focus(), Focus::List);
    }
}
