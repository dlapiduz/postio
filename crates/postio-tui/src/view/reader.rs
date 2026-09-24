//! The reading pane.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::Line;

use chrono::{DateTime, Local};

use crate::app::App;
use crate::row::Row;
use crate::theme::{Role, Theme};
use crate::view::fit;
use crate::view::hit::{Hits, Target};

/// Draw what `app` is reading into `area`: its row's header, then the
/// message from the line the reader is scrolled to.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    theme: &Theme,
    now: DateTime<Local>,
    hits: &mut Hits,
) {
    let Some(reading) = app.reading() else {
        return;
    };
    let row: Option<&Row> = app.row(reading.row);
    let top = app.reader_top();
    // A divider and a space between the list and the reader: without them the
    // list's date runs straight into the subject.
    for y in area.y..area.y + area.height {
        frame.render_widget(
            Line::styled("│", theme.style(Role::Dim)),
            Rect::new(area.x, y, 1, 1),
        );
    }
    let area = Rect::new(
        area.x + 2,
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    hits.add(
        Rect::new(area.x - 2, area.y, 1, area.height),
        Target::Divider,
    );
    let width = usize::from(area.width);
    let mut lines: Vec<Line> = Vec::new();
    if let Some(row) = row {
        lines.push(Line::styled(
            fit(row.subject.as_str(), width),
            theme.style(Role::Text).add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::styled(
            fit(row.from.as_str(), width),
            theme.style(Role::Dim),
        ));
        lines.push(Line::default());
    }
    let height = usize::from(area.height);
    let mut drawn: Vec<(Line, Option<usize>)> =
        lines.into_iter().map(|line| (line, None)).collect();
    // Only as many of the message's lines are wrapped as fill the pane.
    for (index, line) in reading.layout(now).0.into_iter().enumerate().skip(top) {
        if drawn.len() >= height {
            break;
        }
        for row in crate::view::wrap::wrap(&line, width) {
            drawn.push((row, Some(index)));
        }
    }
    for (offset, (line, target)) in drawn.into_iter().take(height).enumerate() {
        let y = area.y + u16::try_from(offset).unwrap_or(u16::MAX);
        let row = Rect::new(area.x, y, area.width, 1);
        frame.render_widget(line, row);
        hits.add(row, Target::Reader(target));
    }
}
