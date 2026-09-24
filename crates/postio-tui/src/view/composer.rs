//! The composer, in the reading pane (US3).

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use crate::composer::{Composer, Field};
use crate::theme::{Role, Theme};
use crate::view::fit;

/// The width of the field labels, so the values line up.
const LABEL: u16 = 9;

/// Draw `composer` into `area`; with `focused`, the terminal's cursor goes
/// where the next letter will land.
pub fn draw(frame: &mut Frame, area: Rect, composer: &Composer, focused: bool, theme: &Theme) {
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
    let mut fields = Vec::new();
    if composer.shows_identities() {
        fields.push((Field::From, "From"));
    }
    fields.push((Field::To, "To"));
    if composer.shows_extra_recipients() {
        fields.push((Field::Cc, "Cc"));
        fields.push((Field::Bcc, "Bcc"));
    }
    fields.push((Field::Subject, "Subject"));

    let value_width = area.width.saturating_sub(LABEL);
    let mut y = area.y;
    for (field, label) in fields {
        if y >= area.y + area.height {
            return;
        }
        let here = composer.field() == field;
        let label_style = if here {
            theme.style(Role::Accent).add_modifier(Modifier::BOLD)
        } else {
            theme.style(Role::Dim)
        };
        // The fields hold only what `Composer::new` sanitized and what was
        // typed; typing cannot produce a control character.
        let value = fit(composer.value(field), usize::from(value_width));
        let line = Line::from(vec![
            Span::styled(
                format!("{label:<width$}", width = usize::from(LABEL)),
                label_style,
            ),
            Span::styled(value, theme.style(Role::Text)),
        ]);
        frame.render_widget(line, Rect::new(area.x, y, area.width, 1));
        if focused && here {
            let column = u16::try_from(composer.cursor_in(field)).unwrap_or(u16::MAX);
            frame.set_cursor_position(Position::new(area.x + LABEL + column.min(value_width), y));
        }
        y += 1;
    }
    // A rule between the headers and the body, as a sent message has.
    if y < area.y + area.height {
        frame.render_widget(
            Line::styled("─".repeat(usize::from(area.width)), theme.style(Role::Dim)),
            Rect::new(area.x, y, area.width, 1),
        );
        y += 1;
    }
    let mut height = (area.y + area.height).saturating_sub(y);
    // A reply's quote, folded to one line under the body: it is sent, it is
    // not edited here, and it would otherwise push what is being written off
    // the screen.
    let quoted = composer.quote_lines();
    if quoted > 0 && height > 1 {
        height -= 1;
        let summary = match quoted {
            1 => "▸ Quoted message, 1 line".to_owned(),
            lines => format!("▸ Quoted message, {lines} lines"),
        };
        frame.render_widget(
            Line::styled(
                fit(&summary, usize::from(area.width)),
                theme.style(Role::Quote),
            ),
            Rect::new(area.x, y + height, area.width, 1),
        );
    }
    if height == 0 {
        return;
    }
    frame.render_widget(composer.body(), Rect::new(area.x, y, area.width, height));
}

/// The schedule-send picker, over the bottom of the composer: the four
/// times, each with the number that picks it and when that is.
pub fn draw_schedule(
    frame: &mut Frame,
    area: Rect,
    times: &[(&'static str, chrono::DateTime<chrono::Local>)],
    theme: &Theme,
    now: chrono::DateTime<chrono::Local>,
) {
    let area = Rect::new(
        area.x + 2,
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    let wanted = u16::try_from(times.len() + 1).unwrap_or(u16::MAX);
    if area.height < wanted {
        return;
    }
    let top = area.y + area.height - wanted;
    let width = usize::from(area.width);
    let mut lines = vec![Line::styled(
        fit("Send later — a number picks, Esc goes back", width),
        theme.style(Role::Accent).add_modifier(Modifier::BOLD),
    )];
    for (index, (label, when)) in times.iter().enumerate() {
        // The day only when it is not today: "18:00" this evening, "Tue
        // 08:00" otherwise.
        let at = if when.date_naive() == now.date_naive() {
            when.format("%H:%M").to_string()
        } else {
            when.format("%a %H:%M").to_string()
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", index + 1), theme.style(Role::Accent)),
            Span::styled(format!("{label:<18}"), theme.style(Role::Text)),
            Span::styled(at, theme.style(Role::Dim)),
        ]));
    }
    for (offset, line) in lines.into_iter().enumerate() {
        let y = top + u16::try_from(offset).unwrap_or(u16::MAX);
        let row = Rect::new(area.x, y, area.width, 1);
        frame.render_widget(ratatui::widgets::Clear, row);
        frame.render_widget(line, row);
    }
}
