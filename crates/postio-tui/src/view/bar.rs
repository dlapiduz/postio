//! The command bar, drawn (terminal.md, "The command bar").
//!
//! A rounded box `min(100, W - 4)` wide, centred, under the top bar and as
//! tall as its lines: the saved searches, the input, an echo of what was
//! typed with its chips, the lines, and the footer. Whatever does not fit
//! scrolls with the chosen line.

use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line as Text, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear};
use unicode_width::UnicodeWidthStr;

use crate::bar::{Bar, Ctx, Entry, Line};
use crate::theme::{Role, Theme};
use crate::view::fit;
use crate::view::hit::{Hits, Target};
use postio_core::CommandId;

/// The widest the box is.
const WIDEST: u16 = 100;

fn cells(text: &str) -> u16 {
    u16::try_from(text.width()).unwrap_or(u16::MAX)
}

/// The box the bar takes in `area`.
pub fn outer(area: Rect, height: u16) -> Rect {
    let width = WIDEST.min(area.width.saturating_sub(4));
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + 1,
        width,
        height.min(area.height.saturating_sub(2)),
    )
}

/// The top bar's field while the bar is open: the input, in the field.
pub fn field(frame: &mut Frame, area: Rect, typed: &str, theme: &Theme) {
    frame.render_widget(
        Text::from(vec![
            Span::styled("⌕ ", theme.style(Role::Accent)),
            Span::styled(
                fit(typed, usize::from(area.width).saturating_sub(2)),
                theme.style(Role::Text),
            ),
        ]),
        area,
    );
}

/// Draw `bar` over `area`.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    bar: &Bar,
    ctx: &Ctx<'_>,
    now: DateTime<Local>,
    theme: &Theme,
    hits: &mut Hits,
) {
    let entries = bar.entries(ctx);
    let saved = bar.saved();
    let typed = bar.typed();
    let echo = !typed.trim().is_empty();
    // Borders, saved, input, echo, a rule, the footer; and a rule above the
    // footer when there are lines.
    let chips_row = !bar.chips().is_empty();
    let fixed =
        2 + u16::from(!saved.is_empty()) + 1 + u16::from(chips_row) + u16::from(echo) + 1 + 1;
    let room = area.height.saturating_sub(2).saturating_sub(fixed);
    let body = u16::try_from(entries.len()).unwrap_or(u16::MAX).min(room);
    let rule_over_footer = u16::from(body > 0);
    let height = fixed + body + rule_over_footer;
    let box_ = outer(area, height);
    if box_.width < 30 || box_.height < 5 {
        return;
    }
    frame.render_widget(Clear, box_);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.style(Role::Dim)),
        box_,
    );
    hits.add(box_, Target::Overlay);
    let inner = Rect::new(box_.x + 1, box_.y + 1, box_.width - 2, box_.height - 2);
    let dim = theme.style(Role::Dim);
    let accent = theme.style(Role::Accent);
    let text = theme.style(Role::Text);
    let rule = |frame: &mut Frame, y: u16| {
        frame.render_widget(
            Text::styled("─".repeat(usize::from(inner.width)), dim),
            Rect::new(inner.x, y, inner.width, 1),
        );
    };
    let mut y = inner.y;

    if !saved.is_empty() {
        let mut spans = vec![Span::styled("Saved", dim)];
        let mut x = inner.x + 5;
        let end = inner.x + inner.width;
        let save_hint = ctx
            .key(CommandId::SaveSearch)
            .filter(|_| echo)
            .map(|key| format!("{key} saves"));
        let reserved = save_hint.as_deref().map_or(0, |hint| cells(hint) + 2);
        for (index, (name, _)) in saved.iter().enumerate() {
            let key = ctx
                .key(postio_ui::command_bar::SAVED[index.min(3)])
                .filter(|_| index < 4);
            let label = format!(
                "  {name}{}",
                key.as_deref().map(|k| format!(" {k}")).unwrap_or_default()
            );
            let width = cells(&label);
            if x + width + reserved > end {
                break;
            }
            spans.push(Span::styled(format!("  {name}"), text));
            if let Some(key) = &key {
                spans.push(Span::styled(format!(" {key}"), accent));
            }
            hits.add(Rect::new(x, y, width, 1), Target::BarSaved(index));
            x += width;
        }
        frame.render_widget(Text::from(spans), Rect::new(inner.x, y, inner.width, 1));
        if let Some(hint) = save_hint {
            let w = cells(&hint);
            frame.render_widget(Text::styled(hint, dim), Rect::new(end - w, y, w, 1));
        }
        y += 1;
    }

    // The input, with Esc at its right.
    let esc = ctx
        .key(CommandId::Back)
        .unwrap_or_default()
        .replace("Escape", "Esc");
    let mut spans = vec![Span::styled("⌕ ", accent)];
    if typed.is_empty() {
        spans.push(Span::styled(postio_ui::focus_row::COMMAND_PROMPT, dim));
    } else {
        spans.push(Span::styled(
            fit(
                typed,
                usize::from(inner.width).saturating_sub(4 + usize::from(cells(&esc))),
            ),
            text,
        ));
    }
    frame.render_widget(Text::from(spans), Rect::new(inner.x, y, inner.width, 1));
    if !esc.is_empty() {
        let w = cells(&esc);
        frame.render_widget(
            Text::styled(esc, dim),
            Rect::new(inner.x + inner.width - w - 1, y, w, 1),
        );
    }
    let before: String = typed.chars().take(bar.caret()).collect();
    frame.set_cursor_position(Position::new(
        (inner.x + 2 + cells(&before)).min(inner.x + inner.width - 1),
        y,
    ));
    y += 1;

    if chips_row {
        let mut spans = vec![Span::raw(" ")];
        let mut x = inner.x + 1;
        for (index, chip) in bar.chips().iter().enumerate() {
            let style = if bar.editing() == Some(index) {
                accent.add_modifier(Modifier::BOLD | Modifier::REVERSED)
            } else {
                accent.add_modifier(Modifier::BOLD)
            };
            let w = cells(chip);
            hits.add(Rect::new(x, y, w, 1), Target::BarChip(index));
            spans.push(Span::styled(chip.clone(), style));
            spans.push(Span::raw("  "));
            x += w + 2;
        }
        frame.render_widget(Text::from(spans), Rect::new(inner.x, y, inner.width, 1));
        y += 1;
    }
    if echo {
        let said = match (bar.editing(), bar.words()) {
            (Some(at), Some(words)) => {
                let chip = bar.chips().get(at).map(String::as_str).unwrap_or_default();
                let editing = chip
                    .split_once(':')
                    .map_or(chip.to_owned(), |(op, _)| format!("{op}:"));
                let mut said = format!(
                    "You typed \u{201c}{words}\u{201d} \u{b7} editing {editing} \u{b7} {}",
                    postio_ui::hints::line([&postio_ui::hints::fixed(
                        "Tab",
                        "next chip",
                        "Tab moves between the bar's chips: the toolkit's focus order, not a command",
                    )])
                );
                if let Some(key) = ctx.key(CommandId::BackToWords) {
                    said.push_str(&format!(" \u{b7} {key} back to plain words"));
                }
                said
            }
            _ => postio_ui::command_bar::echo(typed.trim()),
        };
        frame.render_widget(
            Text::styled(fit(&format!(" {said}"), usize::from(inner.width)), dim),
            Rect::new(inner.x, y, inner.width, 1),
        );
        y += 1;
    }
    rule(frame, y);
    y += 1;

    let chosen = bar.chosen(&entries);
    let shown = usize::from(body);
    let first = if shown == 0 {
        0
    } else {
        (chosen + 1).saturating_sub(shown)
    };
    for (offset, (index, entry)) in entries
        .iter()
        .enumerate()
        .skip(first)
        .take(shown)
        .enumerate()
    {
        let row = Rect::new(
            inner.x,
            y + u16::try_from(offset).unwrap_or(0),
            inner.width,
            1,
        );
        draw_entry(frame, row, entry, index == chosen, theme);
        if entry.row.is_selectable() {
            hits.add(row, Target::BarRow(index));
        }
    }
    y += body;
    if body > 0 {
        rule(frame, y);
        y += 1;
    }
    let mut footer = String::from("\u{2191}\u{2193} move  \u{21b5} run");
    footer.push_str(&format!(
        "  {} commands only",
        postio_ui::finder::COMMANDS_ONLY
    ));
    let right = "Local index";
    frame.render_widget(
        Text::styled(
            fit(
                &footer,
                usize::from(inner.width).saturating_sub(usize::from(cells(right)) + 2),
            ),
            dim,
        ),
        Rect::new(inner.x + 1, y, inner.width.saturating_sub(1), 1),
    );
    let w = cells(right);
    frame.render_widget(
        Text::styled(right, dim),
        Rect::new(inner.x + inner.width - w - 1, y, w, 1),
    );
    let _ = now;
}

fn draw_entry(frame: &mut Frame, row: Rect, entry: &Entry, chosen: bool, theme: &Theme) {
    let dim = theme.style(Role::Dim);
    let text = theme.style(Role::Text);
    let width = usize::from(row.width);
    match &entry.line {
        Line::Heading(title) => {
            frame.render_widget(
                Text::styled(
                    fit(title, width.saturating_sub(1)),
                    dim.add_modifier(Modifier::BOLD),
                ),
                Rect::new(row.x + 1, row.y, row.width.saturating_sub(1), 1),
            );
        }
        Line::Row { title, detail, key } => {
            if chosen {
                frame
                    .buffer_mut()
                    .set_style(row, theme.style(Role::Surface));
            }
            let key = key.clone().unwrap_or_default();
            let room = width.saturating_sub(usize::from(cells(&key)) + 3);
            let mut spans = vec![mark(chosen, theme)];
            let title = fit(title, room);
            let mut used = 1 + title.width();
            spans.push(Span::styled(
                title,
                if chosen {
                    text.add_modifier(Modifier::BOLD)
                } else {
                    text
                },
            ));
            if let Some(detail) = detail {
                let left = room.saturating_sub(used + 2);
                if left > 3 {
                    let detail = fit(detail, left);
                    used += 2 + detail.width();
                    spans.push(Span::styled(format!("  {detail}"), dim));
                }
            }
            spans.push(Span::raw(
                " ".repeat(width.saturating_sub(used + key.width() + 1)),
            ));
            spans.push(Span::styled(key, theme.style(Role::Accent)));
            frame.render_widget(Text::from(spans), row);
        }
        Line::Message(found) => {
            if chosen {
                frame
                    .buffer_mut()
                    .set_style(row, theme.style(Role::Surface));
            }
            let now = Local::now();
            let time = postio_ui::row::timestamp(found.at, now);
            let place = found
                .place
                .as_ref()
                .map(|p| p.as_str().to_owned())
                .unwrap_or_default();
            let tail = format!(
                "{}{time}",
                if place.is_empty() {
                    String::new()
                } else {
                    format!("{place}  ")
                }
            );
            let sender = fit(found.sender.as_str(), 18);
            let after_sender = 1 + 18 + 2;
            let room = width.saturating_sub(after_sender + tail.width() + 2);
            let subject = fit(found.subject.as_str(), room);
            let snippet_room = room.saturating_sub(subject.width() + 2);
            let snippet = if snippet_room > 3 {
                fit(found.snippet.as_str(), snippet_room)
            } else {
                String::new()
            };
            let used = after_sender
                + subject.width()
                + if snippet.is_empty() {
                    0
                } else {
                    2 + snippet.width()
                };
            let mut spans = vec![
                mark(chosen, theme),
                Span::styled(format!("{sender:<18}"), text.add_modifier(Modifier::BOLD)),
                Span::raw("  "),
                Span::styled(subject, text),
            ];
            if !snippet.is_empty() {
                spans.push(Span::styled(format!("  {snippet}"), dim));
            }
            spans.push(Span::raw(
                " ".repeat(width.saturating_sub(used + tail.width() + 1)),
            ));
            spans.push(Span::styled(tail, dim));
            frame.render_widget(Text::from(spans), row);
        }
    }
}

fn mark(chosen: bool, theme: &Theme) -> Span<'static> {
    Span::styled(if chosen { "▌" } else { " " }, theme.style(Role::Focus))
}

#[cfg(test)]
mod tests {
    use crate::app::{App, update};
    use crate::test_support::{
        app, ctrl, open_list, places, press, saved_search, screen, seed_places, serve, type_text,
    };

    /// A mail box of three saved searches, with the list open behind.
    fn with_saved(size: (u16, u16)) -> App {
        let mut app = app(size);
        let mut contents = places();
        contents.saved = vec![
            saved_search("waiting", "Waiting on reply", "is:unread"),
            saved_search("atlas", "Atlas", "label:atlas"),
            saved_search("receipts", "Receipts this month", "receipts"),
        ];
        seed_places(&mut app, contents);
        let opening = open_list(&mut app, 3);
        serve(&mut app, opening);
        app
    }

    #[test]
    fn slash_opens_a_box_under_the_top_bar_with_the_saved_row_the_input_and_the_footer() {
        let mut app = with_saved((120, 36));
        update(&mut app, press('/'));
        type_text(&mut app, "arch");
        let drawn = screen(120, 36, &app);
        let lines: Vec<&str> = drawn.lines().collect();
        // min(100, W - 4) wide, centred: columns 10 to 109, from the second row.
        let cell = |line: &str, at: usize| line.chars().nth(at);
        assert_eq!(cell(lines[1], 10), Some('╭'), "{drawn}");
        assert_eq!(cell(lines[1], 109), Some('╮'), "{drawn}");
        assert!(
            lines[2]
                .contains("Saved  Waiting on reply alt+1  Atlas alt+2  Receipts this month alt+3"),
            "{drawn}"
        );
        assert!(lines[2].contains("ctrl+s saves"), "{drawn}");
        assert!(lines[3].contains("⌕ arch"), "{drawn}");
        assert!(lines[3].contains("Esc │"), "{drawn}");
        assert!(drawn.contains("↑↓ move"), "{drawn}");
        assert!(drawn.contains("> commands only"), "{drawn}");
        assert!(drawn.contains("Local index"), "{drawn}");
    }

    #[test]
    fn typing_lists_commands_places_and_the_search_row_each_with_its_key() {
        let mut app = with_saved((120, 36));
        update(&mut app, press('/'));
        type_text(&mut app, "arch");
        let drawn = screen(120, 36, &app);
        let line_of = |needle: &str| {
            drawn
                .lines()
                .find(|line| line.contains(needle))
                .unwrap_or_else(|| panic!("no {needle}:\n{drawn}"))
                .to_owned()
        };
        assert!(drawn.contains("Commands"), "{drawn}");
        assert!(line_of("Archive").contains(" a "), "{}", line_of("Archive"));
        assert!(drawn.contains("Go to"), "{drawn}");
        assert!(line_of("in:Archive").contains("g r"), "{drawn}");
        assert!(drawn.contains("Search mail for “arch”"), "{drawn}");
        assert!(
            line_of("Archive").contains('▌'),
            "the first row is chosen: {}",
            line_of("Archive")
        );
    }

    #[test]
    fn ctrl_k_opens_it_with_commands_only() {
        let mut app = with_saved((120, 36));
        update(&mut app, ctrl('k'));
        let drawn = screen(120, 36, &app);
        assert!(drawn.contains("⌕ >"), "{drawn}");
        assert!(drawn.contains("Commands"), "{drawn}");
        assert!(!drawn.contains("Go to"), "{drawn}");
    }
}
