//! The reading pane.
//!
//! PLATE 1b's reader in a terminal: the subject as the heading, a dim line
//! of who wrote to whom and when, then the message wrapped to the pane with
//! room on either side, and the keys for the two verbs a message is read
//! for at the foot.

use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use crate::app::App;
use crate::row::Row;
use crate::theme::{Role, Theme};
use crate::view::fit;
use crate::view::hit::{Hits, Target};
use crate::view::wrap::wrap;

/// Columns of room on each side of the text.
const PAD: u16 = 2;

/// Draw what `app` is reading into `area`: its header, the message from the
/// line the reader is scrolled to, and the keys at the foot.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    theme: &Theme,
    now: DateTime<Local>,
    hits: &mut Hits,
) {
    // A rule between the list and the reader, whether or not anything is
    // being read, so the panes keep their shape.
    for y in area.y..area.y + area.height {
        frame.render_widget(
            Line::styled("│", theme.style(Role::Dim)),
            Rect::new(area.x, y, 1, 1),
        );
    }
    let Some(reading) = app.reading() else {
        return;
    };
    hits.add(Rect::new(area.x, area.y, 1, area.height), Target::Divider);
    let inner = Rect::new(
        area.x + 1 + PAD,
        area.y + 1,
        area.width.saturating_sub(1 + 2 * PAD),
        area.height.saturating_sub(1),
    );
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let width = usize::from(inner.width);
    let row: Option<&Row> = app.row(reading.row);
    let member = reading.members.get(reading.current);

    let mut header: Vec<Line> = Vec::new();
    if let Some(row) = row {
        let subject = Line::styled(
            row.subject.as_str().to_owned(),
            theme.style(Role::Text).add_modifier(Modifier::BOLD),
        );
        // A long subject takes a second line, and no more.
        header.extend(wrap(&subject, width).into_iter().take(2));
    }
    if let Some((who, when)) = meta(row, member, now) {
        // The date is kept whole; whom it was sent to gives way first.
        let when = format!(" · {when}");
        let room = width.saturating_sub(unicode_width::UnicodeWidthStr::width(when.as_str()));
        let line = if room >= 12 {
            format!("{}{when}", fit(&who, room))
        } else {
            fit(&format!("{who}{when}"), width)
        };
        header.push(Line::styled(line, theme.style(Role::Dim)));
    }
    if !header.is_empty() {
        header.push(Line::default());
    }

    // The keys, at the foot, with a blank line above them, when the pane is
    // tall enough to spare two lines.
    let foot = footer(app, theme);
    let spare = foot.is_some() && inner.height >= 10;
    let height = usize::from(inner.height) - if spare { 2 } else { 0 };

    let top = app.reader_top();
    let mut drawn: Vec<(Line, Option<usize>)> =
        header.into_iter().map(|line| (line, None)).collect();
    // Only as many of the message's lines are wrapped as fill the pane.
    for (index, line) in reading.layout(now).0.into_iter().enumerate().skip(top) {
        if drawn.len() >= height {
            break;
        }
        for row in wrap(&line, width) {
            drawn.push((row, Some(index)));
        }
    }
    for (offset, (line, target)) in drawn.into_iter().take(height).enumerate() {
        let y = inner.y + u16::try_from(offset).unwrap_or(u16::MAX);
        let rect = Rect::new(inner.x, y, inner.width, 1);
        frame.render_widget(line, rect);
        hits.add(rect, Target::Reader(target));
    }
    if let Some(foot) = foot.filter(|_| spare) {
        let y = inner.y + inner.height - 1;
        frame.render_widget(foot, Rect::new(inner.x, y, inner.width, 1));
    }
}

/// `from → to, cc` and the date, as far as they are known.
fn meta(
    row: Option<&Row>,
    member: Option<&crate::conversation::Member>,
    now: DateTime<Local>,
) -> Option<(String, String)> {
    let from = member
        .and_then(|member| member.address.clone())
        .or_else(|| row.and_then(|row| row.address.clone()))
        .or_else(|| member.map(|member| member.from.as_str().to_owned()))
        .or_else(|| row.map(|row| row.from.as_str().to_owned()))?;
    let when = member
        .map(|member| member.when)
        .or(row.map(|row| row.when))?;
    let recipients: Vec<&str> = member
        .map(|member| member.recipients.iter().map(|to| to.as_str()).collect())
        .unwrap_or_default();
    let date = when
        .with_timezone(&now.timezone())
        .format("%a %-d %b, %H:%M")
        .to_string();
    let who = if recipients.is_empty() {
        from
    } else {
        format!("{from} → {}", recipients.join(", "))
    };
    Some((who, date))
}

/// The reader's two verbs and their keys, from the keymap in force.
fn footer<'a>(app: &App, theme: &Theme) -> Option<Line<'a>> {
    use postio_core::CommandId;
    let mut spans = Vec::new();
    for (command, word) in [(CommandId::Reply, "reply"), (CommandId::Archive, "archive")] {
        let Some(key) = app.hint(command) else {
            continue;
        };
        if !spans.is_empty() {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(key, theme.style(Role::Accent)));
        spans.push(Span::styled(format!(" {word}"), theme.style(Role::Dim)));
    }
    (!spans.is_empty()).then(|| Line::from(spans))
}
