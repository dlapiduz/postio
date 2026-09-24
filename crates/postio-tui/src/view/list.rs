//! The message list.
//!
//! Only the rows in view are drawn, from the window's resident pages; a row
//! whose page is still on its way is blank, and asking for it is `App`'s
//! business, not the drawing's (Principle V: never a whole mailbox).
//!
//! A row is two lines, the compact form of the desktop's cards: marks, the
//! sender and the time over the subject and its preview. Every state has a
//! mark that is not a colour (`contracts/tui-surface.md` §Colour roles): `▌`
//! the cursor, `✓` selected, `●` unread, `⚑` flagged, `⎘` attachments.

use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::row::Row;
use crate::theme::{Role, Theme};
use crate::view::fit;
use crate::view::hit::{Hits, Target};

/// One visible row of the list.
pub struct Visible<'a> {
    /// The row, or `None` while its page is on its way.
    pub row: Option<&'a Row>,
    /// Whether the cursor is here.
    pub cursor: bool,
    /// Whether the row is selected.
    pub selected: bool,
}

/// Lines per row.
pub const LINES: u16 = crate::layout::LIST_ROW_LINES;
/// Width of the date column.
const DATE: usize = 9;
/// Where the sender and the subject start: after the cursor's bar, the
/// selection's mark, and the three state marks.
const TEXT: usize = 7;

/// Draw `rows` into `area`, two lines each.
/// `first` is the list position of the first row, for what a click on each
/// row means.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    rows: &[Visible],
    first: u32,
    theme: &Theme,
    now: DateTime<Local>,
    hits: &mut Hits,
) {
    let width = usize::from(area.width);
    let fits = usize::from(area.height / LINES);
    for (offset, visible) in rows.iter().take(fits).enumerate() {
        let y = area.y + u16::try_from(offset).unwrap_or(u16::MAX) * LINES;
        let rect = Rect::new(area.x, y, area.width, LINES);
        // The cursor is the tint across the row; a selection is a stripe
        // of its own colour in the gutter, so a cursor inside a selection
        // is still told apart from the rows around it.
        if visible.cursor && visible.row.is_some() {
            frame
                .buffer_mut()
                .set_style(rect, theme.style(Role::Surface));
        }
        let (top, bottom) = lines(visible, width, theme, now);
        frame.render_widget(top, Rect::new(area.x, y, area.width, 1));
        frame.render_widget(bottom, Rect::new(area.x, y + 1, area.width, 1));
        if visible.selected && area.width > 2 {
            frame.buffer_mut().set_style(
                Rect::new(area.x + 1, y, 2, LINES),
                theme.style(Role::Selection),
            );
        }
        let position = first.saturating_add(u32::try_from(offset).unwrap_or(u32::MAX));
        hits.add(rect, Target::Row(position));
    }
}

/// One row as its two lines of `width` columns: marks, sender and date, then
/// subject and preview under the sender.
fn lines<'a>(
    visible: &Visible,
    width: usize,
    theme: &Theme,
    now: DateTime<Local>,
) -> (Line<'a>, Line<'a>) {
    let Some(row) = visible.row else {
        return (Line::default(), Line::default());
    };
    let bar = Span::styled(
        if visible.cursor { "▌" } else { " " },
        theme.style(Role::Focus),
    );
    let chosen = Span::raw(if visible.selected { "✓" } else { " " });
    let mark = |on: bool, glyph: &'static str, role: Role| {
        Span::styled(if on { glyph } else { " " }, theme.style(role))
    };
    let emphasis = if row.unread {
        theme.style(Role::Unread)
    } else {
        theme.style(Role::Text)
    };

    // Line one: the sender, and the date at the right edge.
    let date = format!("{:>DATE$}", postio_ui::row::timestamp(row.when, now));
    let room = width.saturating_sub(TEXT + DATE + 2);
    let count = if row.count > 1 {
        format!(" ({})", row.count)
    } else {
        String::new()
    };
    let from = fit(row.from.as_str(), room.saturating_sub(count.width()));
    let count = fit(&count, room.saturating_sub(from.width()));
    let pad = room.saturating_sub(from.width() + count.width());
    let top = Line::from(vec![
        bar.clone(),
        chosen,
        Span::raw(" "),
        mark(row.unread, "●", Role::Accent),
        mark(row.flagged, "⚑", Role::Flagged),
        mark(row.attachment, "⎘", Role::Dim),
        Span::raw(" "),
        Span::styled(from, emphasis),
        Span::styled(count, theme.style(Role::Dim)),
        Span::raw(" ".repeat(pad + 1)),
        Span::styled(date, theme.style(Role::Dim)),
    ]);

    // Line two: the subject, and as much of the preview as is left.
    let room = width.saturating_sub(TEXT + 1);
    let subject = fit(row.subject.as_str(), room);
    let preview = if row.preview.as_str().is_empty() {
        String::new()
    } else {
        fit(
            &format!(" — {}", row.preview.as_str()),
            room.saturating_sub(subject.width()),
        )
    };
    let bottom = Line::from(vec![
        bar,
        Span::raw(" ".repeat(TEXT - 1)),
        Span::styled(subject, emphasis),
        Span::styled(preview, theme.style(Role::Dim)),
    ]);
    (top, bottom)
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use postio_model::MessageId;
    use postio_ui::terminal::SafeText;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use ratatui::style::Modifier;

    use super::*;
    use crate::caps::{Background, Colour};

    fn row(id: i64, from: &str, subject: &str, unread: bool, flagged: bool) -> Row {
        Row {
            id: MessageId::new(id),
            thread: None,
            is_thread: false,
            from: SafeText::new(from),
            address: None,
            subject: SafeText::new(subject),
            preview: SafeText::new("and a preview"),
            when: Utc.with_ymd_and_hms(2026, 9, 20, 9, 14, 0).unwrap(),
            unread,
            flagged,
            attachment: false,
            count: 1,
        }
    }

    fn render(rows: &[Visible]) -> Vec<String> {
        render_cells(rows)
            .into_iter()
            .map(|line| line.concat())
            .collect()
    }

    fn buffer(rows: &[Visible], colour: Colour) -> (ratatui::buffer::Buffer, Theme) {
        let theme = Theme::new(colour, Background::Dark, &Default::default()).0;
        let now = Local.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap();
        let height = u16::try_from(rows.len() * 2).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(70, height)).unwrap();
        terminal
            .draw(|frame| {
                draw(
                    frame,
                    frame.area(),
                    rows,
                    0,
                    &theme,
                    now,
                    &mut Hits::default(),
                );
            })
            .unwrap();
        (terminal.backend().buffer().clone(), theme)
    }

    /// Each row's two lines, as cells.
    fn render_cells(rows: &[Visible]) -> Vec<Vec<String>> {
        let (buffer, _) = buffer(rows, Colour::None);
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_owned())
                    .collect::<Vec<String>>()
            })
            .collect()
    }

    #[test]
    fn wide_characters_do_not_push_the_date_column_out_of_line() {
        let rows = [
            row(1, "Ada Lovelace", "Plain subject", false, false),
            row(2, "山田太郎", "会議の議事録について確認", true, false),
            row(3, "🎉 Party Planner", "Saturday 🎂🎈", false, true),
        ];
        let visible: Vec<Visible> = rows
            .iter()
            .map(|row| Visible {
                row: Some(row),
                cursor: false,
                selected: false,
            })
            .collect();
        let cells: Vec<Vec<String>> = render_cells(&visible).into_iter().step_by(2).collect();
        // The date is the same text on each row; it must start at the same
        // cell on each. By cell, not by string index: a wide character is
        // its symbol plus an empty continuation cell.
        let starts: Vec<usize> = cells
            .iter()
            .map(|line| {
                (0..line.len() - 2)
                    .find(|&x| line[x] == "S" && line[x + 1] == "u" && line[x + 2] == "n")
                    .unwrap_or_else(|| panic!("the date is drawn: {:?}", line.concat()))
            })
            .collect();
        assert!(
            starts.windows(2).all(|pair| pair[0] == pair[1]),
            "{starts:?}\n{:#?}",
            cells.iter().map(|line| line.concat()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn every_state_has_a_mark_that_is_not_a_colour() {
        let mut marked = row(1, "Ada", "Unread and flagged", true, true);
        marked.attachment = true;
        let rows = [marked, row(2, "Bea", "Read", false, false)];
        let lines = render(&[
            Visible {
                row: Some(&rows[0]),
                cursor: true,
                selected: true,
            },
            Visible {
                row: Some(&rows[1]),
                cursor: false,
                selected: false,
            },
        ]);
        let first = format!("{}{}", lines[0], lines[1]);
        let plain = format!("{}{}", lines[2], lines[3]);
        // The cursor's bar on both of its lines.
        assert!(lines[0].starts_with('▌'), "{:?}", lines[0]);
        assert!(lines[1].starts_with('▌'), "{:?}", lines[1]);
        for mark in ['✓', '●', '⚑', '⎘'] {
            assert!(first.contains(mark), "{mark} missing: {first:?}");
        }
        for mark in ['▌', '✓', '●', '⚑', '⎘'] {
            assert!(!plain.contains(mark), "{mark} on a plain row: {plain:?}");
        }
    }

    #[test]
    fn the_cursor_row_is_tinted_on_both_its_lines() {
        let rows = [
            row(1, "Ada", "Here", false, false),
            row(2, "Bea", "Not here", false, false),
        ];
        let visible = [
            Visible {
                row: Some(&rows[0]),
                cursor: true,
                selected: false,
            },
            Visible {
                row: Some(&rows[1]),
                cursor: false,
                selected: false,
            },
        ];
        // With colour, the raised surface, every cell of both lines.
        let (drawn, theme) = buffer(&visible, Colour::TrueColor);
        let surface = theme.style(Role::Surface).bg.expect("a tint");
        for y in 0..2 {
            for x in 0..drawn.area.width {
                assert_eq!(drawn[(x, y)].bg, surface, "({x}, {y}) is not tinted");
            }
        }
        for y in 2..4 {
            assert_ne!(drawn[(10, y)].bg, surface, "the next row is not");
        }
        // Without, reverse video, the same way.
        let (drawn, _) = buffer(&visible, Colour::None);
        for y in 0..2 {
            for x in 0..drawn.area.width {
                assert!(
                    drawn[(x, y)].modifier.contains(Modifier::REVERSED),
                    "({x}, {y}) is not reversed"
                );
            }
        }
        assert!(!drawn[(10, 2)].modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn the_cursor_stands_out_from_a_selection_it_is_inside() {
        // Rows 2-4 marked, the cursor on row 3: walking through a selection
        // must still say which row the keyboard is on.
        let rows: Vec<Row> = (1..=5)
            .map(|id| row(id, "Ada", "Walked through", false, false))
            .collect();
        let visible: Vec<Visible> = rows
            .iter()
            .enumerate()
            .map(|(at, row)| Visible {
                row: Some(row),
                cursor: at == 2,
                selected: (1..=3).contains(&at),
            })
            .collect();
        let (drawn, theme) = buffer(&visible, Colour::TrueColor);
        let surface = theme.style(Role::Surface).bg.expect("a tint");
        let selection = theme.style(Role::Selection).bg.expect("a selection colour");
        let focus = theme.style(Role::Focus).fg.expect("the cursor's accent");
        assert_ne!(selection, surface, "a selection is not the cursor's tint");
        assert_ne!(selection, focus, "nor the cursor bar's colour");
        let line = |at: u16| at * 2;
        // The cursor row: tinted across, on both lines, with its bar.
        for y in [line(2), line(2) + 1] {
            assert_eq!(drawn[(10, y)].bg, surface, "the cursor row, line {y}");
            assert_eq!(drawn[(0, y)].symbol(), "▌");
        }
        // Its neighbours in the selection: not tinted, no bar.
        for at in [1, 3] {
            for y in [line(at), line(at) + 1] {
                assert_ne!(drawn[(10, y)].bg, surface, "row {at} looks like the cursor");
                assert_eq!(
                    drawn[(10, y)].bg,
                    drawn[(10, line(0))].bg,
                    "row {at}'s text sits on the ground"
                );
                assert_ne!(drawn[(0, y)].symbol(), "▌", "row {at} has the cursor's bar");
            }
        }
        // All three still marked, the cursor's row included, in the
        // selection's colour.
        for at in 1..=3 {
            assert_eq!(drawn[(1, line(at))].symbol(), "✓", "row {at}");
            assert_eq!(drawn[(1, line(at))].bg, selection, "row {at}");
        }
        for at in [0, 4] {
            assert_ne!(drawn[(1, line(at))].symbol(), "✓", "row {at} is not marked");
        }

        // Without colour: the cursor reversed, the selection its checks.
        let (drawn, _) = buffer(&visible, Colour::None);
        assert!(drawn[(10, line(2))].modifier.contains(Modifier::REVERSED));
        for at in [1, 3] {
            assert!(
                !drawn[(10, line(at))].modifier.contains(Modifier::REVERSED),
                "row {at}"
            );
            assert_eq!(drawn[(1, line(at))].symbol(), "✓", "row {at}");
        }
        assert_eq!(drawn[(1, line(2))].symbol(), "✓");
    }

    #[test]
    fn a_row_is_its_sender_and_time_over_its_subject_and_preview() {
        let rows = [row(1, "Ada Lovelace", "Engine notes", true, false)];
        let lines = render(&[Visible {
            row: Some(&rows[0]),
            cursor: false,
            selected: false,
        }]);
        assert!(
            lines[0].contains("Ada Lovelace") && lines[0].trim_end().ends_with("Sun"),
            "{:?}",
            lines[0]
        );
        assert!(
            lines[1].contains("Engine notes — and a preview"),
            "{:?}",
            lines[1]
        );
    }

    #[test]
    fn a_row_still_on_its_way_is_a_blank_line() {
        let lines = render(&[Visible {
            row: None,
            cursor: false,
            selected: false,
        }]);
        assert_eq!(lines[0].trim(), "");
        assert_eq!(lines[1].trim(), "");
    }
}
