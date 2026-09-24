//! The message list.
//!
//! Only the rows in view are drawn, from the window's resident pages; a row
//! whose page is still on its way is a blank line, and asking for it is
//! `App`'s business, not the drawing's (Principle V: never a whole mailbox).
//!
//! Every state has a mark that is not a colour (`contracts/tui-surface.md`
//! §Colour roles): `›` the cursor, `▌` selected, `●` unread, `⚑` flagged.

use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

use crate::row::Row;
use crate::theme::{Role, Theme};
use crate::view::fit;

/// One visible line of the list.
pub struct Visible<'a> {
    /// The row, or `None` while its page is on its way.
    pub row: Option<&'a Row>,
    /// Whether the cursor is here.
    pub cursor: bool,
    /// Whether the row is selected.
    pub selected: bool,
}

/// Width of the date column.
const DATE: usize = 9;
/// Width of the sender column.
const FROM: usize = 20;

/// Draw `rows` into `area`, one per line.
pub fn draw(frame: &mut Frame, area: Rect, rows: &[Visible], theme: &Theme, now: DateTime<Local>) {
    let width = usize::from(area.width);
    for (offset, visible) in rows.iter().take(usize::from(area.height)).enumerate() {
        let y = area.y + u16::try_from(offset).unwrap_or(u16::MAX);
        let line_area = Rect::new(area.x, y, area.width, 1);
        frame.render_widget(line(visible, width, theme, now), line_area);
    }
}

/// One row as a line of `width` columns: gutter, marks, sender, subject and
/// preview, date at the right edge.
fn line<'a>(visible: &Visible, width: usize, theme: &Theme, now: DateTime<Local>) -> Line<'a> {
    let Some(row) = visible.row else {
        return Line::default();
    };
    let gutter = match (visible.cursor, visible.selected) {
        (true, true) => "›▌",
        (true, false) => "› ",
        (false, true) => " ▌",
        (false, false) => "  ",
    };
    let unread = if row.unread { "●" } else { " " };
    let flagged = if row.flagged { "⚑" } else { " " };
    let date = postio_ui::row::timestamp(row.when, now);

    // gutter 2, marks 2, a space, sender, a space, …, a space, date.
    let fixed = 2 + 2 + 1 + FROM + 1 + 1 + DATE;
    let middle = width.saturating_sub(fixed);
    let from = fit(row.from.as_str(), FROM);
    let from_pad = FROM.saturating_sub(unicode_width::UnicodeWidthStr::width(from.as_str()));
    let count = if row.count > 1 {
        format!(" ({})", row.count)
    } else {
        String::new()
    };
    let subject = format!("{}{count}", row.subject.as_str());
    let subject = fit(&subject, middle);
    let subject_width = unicode_width::UnicodeWidthStr::width(subject.as_str());
    let preview = fit(
        &format!(" — {}", row.preview.as_str()),
        middle.saturating_sub(subject_width),
    );
    let used = subject_width + unicode_width::UnicodeWidthStr::width(preview.as_str());
    let pad = middle.saturating_sub(used);
    let date = format!("{date:>DATE$}");

    let emphasis = if row.unread {
        theme.style(Role::Unread)
    } else {
        theme.style(Role::Text)
    };
    let base = if visible.selected {
        theme.style(Role::Selection)
    } else {
        ratatui::style::Style::default()
    };
    Line::from(vec![
        Span::styled(gutter, theme.style(Role::Focus)),
        Span::styled(unread, theme.style(Role::Unread)),
        Span::styled(flagged, theme.style(Role::Flagged)),
        Span::raw(" "),
        Span::styled(from, emphasis),
        Span::raw(" ".repeat(from_pad + 1)),
        Span::styled(subject, emphasis),
        Span::styled(preview, theme.style(Role::Dim)),
        Span::raw(" ".repeat(pad + 1)),
        Span::styled(date, theme.style(Role::Dim)),
    ])
    .style(base)
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use postio_model::MessageId;
    use postio_ui::terminal::SafeText;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

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

    fn render_cells(rows: &[Visible]) -> Vec<Vec<String>> {
        let theme = Theme::new(Colour::None, Background::Unknown, &Default::default()).0;
        let now = Local.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(70, rows.len() as u16)).unwrap();
        terminal
            .draw(|frame| draw(frame, frame.area(), rows, &theme, now))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
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
        let cells = render_cells(&visible);
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
        let rows = [
            row(1, "Ada", "Unread and flagged", true, true),
            row(2, "Bea", "Read", false, false),
        ];
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
        assert!(
            lines[0].starts_with('›') || lines[0].contains('›'),
            "{:?}",
            lines[0]
        );
        assert!(lines[0].contains('▌'), "{:?}", lines[0]);
        assert!(lines[0].contains('●'), "{:?}", lines[0]);
        assert!(lines[0].contains('⚑'), "{:?}", lines[0]);
        for mark in ['›', '▌', '●', '⚑'] {
            assert!(
                !lines[1].contains(mark),
                "{mark} on a plain row: {:?}",
                lines[1]
            );
        }
    }

    #[test]
    fn a_row_still_on_its_way_is_a_blank_line() {
        let lines = render(&[Visible {
            row: None,
            cursor: false,
            selected: false,
        }]);
        assert_eq!(lines[0].trim(), "");
    }
}
