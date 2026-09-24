//! The composer, in the reading pane (US3).

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use postio_ui::terminal::SafeText;

use crate::composer::{Composer, Field};
use crate::theme::{Role, Theme};
use crate::view::fit;
use crate::view::hit::{Hits, Target};

/// The width of the field labels, so the values line up.
const LABEL: u16 = 9;

/// Draw `composer` into `area`; with `focused`, the terminal's cursor goes
/// where the next letter will land.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    composer: &Composer,
    preview: Option<postio_config::Preview>,
    focused: bool,
    actions: &[Action],
    theme: &Theme,
    hits: &mut Hits,
) {
    hits.add(Rect::new(area.x, area.y, 1, area.height), Target::Divider);
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
        hits.add(
            Rect::new(area.x, y, area.width, 1),
            Target::ComposerField(field),
        );
        if focused && here {
            let column = u16::try_from(composer.cursor_in(field)).unwrap_or(u16::MAX);
            frame.set_cursor_position(Position::new(area.x + LABEL + column.min(value_width), y));
        }
        y += 1;
        if here {
            y = draw_suggestions(frame, area, y, composer, theme);
        }
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
    // The buttons along the foot, as the desktop's composer has them: what
    // each does, and the key this terminal can send for it.
    if !actions.is_empty() && height > 2 {
        height -= 1;
        draw_actions(frame, area.x, y + height, area.width, actions, theme, hits);
    }
    // The draft's files, under the body: each by name and size, the way the
    // desktop lists them. A forwarded file's name is the sender's.
    let files = composer.attachments();
    let shown = files.len().min(ATTACHMENTS);
    let mut listed: Vec<Line> = files[..shown]
        .iter()
        .map(|file| {
            let name = SafeText::new(file.display_name());
            Line::from(vec![
                Span::styled("📎 ", theme.style(Role::Dim)),
                Span::styled(name.as_str().to_owned(), theme.style(Role::Text)),
                Span::styled(
                    format!("  {}", postio_ui::format::human_size(file.size)),
                    theme.style(Role::Dim),
                ),
            ])
        })
        .collect();
    if files.len() > shown {
        listed.push(Line::styled(
            format!("   and {} more", files.len() - shown),
            theme.style(Role::Dim),
        ));
    }
    let needed = u16::try_from(listed.len()).unwrap_or(u16::MAX);
    if needed > 0 && height > needed {
        height -= needed;
        for (offset, line) in listed.into_iter().enumerate() {
            let row = y + height + u16::try_from(offset).unwrap_or(u16::MAX);
            frame.render_widget(line, Rect::new(area.x, row, area.width, 1));
        }
    }
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
    let body = Rect::new(area.x, y, area.width, height);
    hits.add(body, Target::ComposerBody);
    // Where the textarea will have scrolled to, for where a click lands.
    composer.body_top(height);
    match preview {
        None => frame.render_widget(composer.body(), body),
        Some(postio_config::Preview::Toggle) => draw_preview(frame, body, composer),
        Some(postio_config::Preview::Split) => {
            let half = body.width / 2;
            frame.render_widget(
                composer.body(),
                Rect::new(body.x, body.y, half, body.height),
            );
            for row in body.y..body.y + body.height {
                frame.render_widget(
                    Line::styled("│", theme.style(Role::Dim)),
                    Rect::new(body.x + half, row, 1, 1),
                );
            }
            let right = Rect::new(
                body.x + half + 2,
                body.y,
                body.width.saturating_sub(half + 2),
                body.height,
            );
            draw_preview(frame, right, composer);
        }
    }
}

/// The message as it will arrive, from its top, in `area`.
fn draw_preview(frame: &mut Frame, area: Rect, composer: &Composer) {
    let lines = composer.preview().lines();
    for (offset, line) in lines.into_iter().take(usize::from(area.height)).enumerate() {
        let row = area.y + u16::try_from(offset).unwrap_or(u16::MAX);
        frame.render_widget(line, Rect::new(area.x, row, area.width, 1));
    }
}

/// How many of a draft's files are listed by name before the rest are
/// counted.
const ATTACHMENTS: usize = 4;

/// How many suggestions are shown at once.
const SUGGESTIONS: usize = 5;

/// Recipient suggestions under the field being typed in, from row `y`;
/// answers the row after them.
fn draw_suggestions(
    frame: &mut Frame,
    area: Rect,
    mut y: u16,
    composer: &Composer,
    theme: &Theme,
) -> u16 {
    let width = usize::from(area.width.saturating_sub(LABEL));
    for (index, candidate) in composer.suggestions().iter().take(SUGGESTIONS).enumerate() {
        if y >= area.y + area.height {
            break;
        }
        let chosen = index == composer.suggestion();
        // A contact's name is whatever a sender's header said.
        let label = SafeText::new(&postio_ui::recipients::candidate_label(candidate));
        let (mark, role) = if chosen {
            ("› ", Role::Surface)
        } else {
            ("  ", Role::Dim)
        };
        frame.render_widget(
            Line::styled(fit(&format!("{mark}{label}"), width), theme.style(role)),
            Rect::new(area.x + LABEL, y, area.width.saturating_sub(LABEL), 1),
        );
        y += 1;
    }
    y
}

/// The schedule-send picker, over the bottom of the composer: the four
/// One of the composer's buttons: its key as this terminal sends it, what it
/// says, and the command it runs.
pub type Action = (String, &'static str, &'static str);

/// The buttons, left to right: the first is the one a message is for, and
/// is filled; the rest are words with their keys.
fn draw_actions(
    frame: &mut Frame,
    x: u16,
    y: u16,
    width: u16,
    actions: &[Action],
    theme: &Theme,
    hits: &mut Hits,
) {
    let mut at = x;
    let end = x + width;
    for (index, (key, word, id)) in actions.iter().enumerate() {
        let label = format!(" {word} {key} ");
        let wide = u16::try_from(unicode_width::UnicodeWidthStr::width(label.as_str()))
            .unwrap_or(u16::MAX);
        if at + wide > end {
            break;
        }
        let line = if index == 0 {
            let filled = theme.style(Role::Surface).add_modifier(Modifier::BOLD);
            Line::from(vec![
                Span::styled(format!(" {word} "), filled.patch(theme.style(Role::Accent))),
                Span::styled(format!("{key} "), filled),
            ])
        } else {
            Line::from(vec![
                Span::styled(format!(" {word} "), theme.style(Role::Text)),
                Span::styled(format!("{key} "), theme.style(Role::Dim)),
            ])
        };
        let rect = Rect::new(at, y, wide, 1);
        frame.render_widget(line, rect);
        hits.add(rect, Target::ComposerAction(id));
        at += wide + 1;
    }
}

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

/// The path prompt (FR-027), over the last row of the composer, with the
/// terminal's cursor at its end.
pub fn draw_path_prompt(frame: &mut Frame, area: Rect, typed: &str, theme: &Theme) {
    let area = Rect::new(
        area.x + 2,
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    if area.height == 0 {
        return;
    }
    let row = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
    // What was typed or pasted here is the person's own, but a paste can
    // carry anything.
    let typed = SafeText::new(typed);
    let label = "Attach: ";
    let line = Line::from(vec![
        Span::styled(
            label,
            theme.style(Role::Accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            fit(
                typed.as_str(),
                usize::from(area.width).saturating_sub(label.len()),
            ),
            theme.style(Role::Text),
        ),
    ]);
    frame.render_widget(ratatui::widgets::Clear, row);
    frame.render_widget(line, row);
    let column = unicode_width::UnicodeWidthStr::width(typed.as_str()) + label.len();
    frame.set_cursor_position(Position::new(
        row.x
            + u16::try_from(column)
                .unwrap_or(u16::MAX)
                .min(row.width.saturating_sub(1)),
        row.y,
    ));
}
