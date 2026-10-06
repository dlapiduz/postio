//! The digest rules (terminal.md, "Digests"): Filtered's full view with a
//! rule to a row -- its name, what it matches, when it delivers, when it
//! next does, and what it holds now -- and the question over removing one.

use chrono::{DateTime, Local};
use postio_core::CommandId;
use postio_ui::digest;
use postio_ui::terminal::SafeText;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::surface::Part;
use crate::theme::{Role, Theme};
use crate::view::bottom::cap;
use crate::view::boxed::{self, Button};
use crate::view::fit;
use crate::view::hit::{Hits, Target};
use crate::view::wrap::wrap;

const GAP: usize = 3;

/// The strip: back, the title with its count, and what held mail does.
pub fn strip(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, hits: &mut Hits) {
    if theme.style(Role::Surface).bg.is_some() {
        frame
            .buffer_mut()
            .set_style(area, theme.style(Role::Surface));
    }
    let count = app.rules().map_or(0, |rules| rules.rules().len());
    let mut back = vec![Span::styled("‹ Inbox", theme.style(Role::Text))];
    if let Some(key) = app.hint(CommandId::GoToInbox) {
        back.push(Span::styled(
            format!(" {}", cap(&key)),
            theme.style(Role::Accent),
        ));
    }
    let pieces: Vec<(Vec<Span<'static>>, Option<Target>)> = vec![
        (back, Some(Target::Command(CommandId::GoToInbox.as_str()))),
        (
            vec![
                Span::styled(
                    digest::RULES_TITLE,
                    theme.style(Role::Text).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" · {}", digest::rule_count(count)),
                    theme.style(Role::Dim),
                ),
            ],
            None,
        ),
        (
            vec![Span::styled(digest::RULES_SUBTITLE, theme.style(Role::Dim))],
            None,
        ),
    ];
    let width = usize::from(area.width).saturating_sub(2);
    let size = |pieces: &[(Vec<Span<'static>>, Option<Target>)]| {
        pieces
            .iter()
            .map(|(spans, _)| spans.iter().map(|s| s.content.width()).sum::<usize>())
            .sum::<usize>()
            + GAP * pieces.len().saturating_sub(1)
    };
    let mut pieces = pieces;
    while pieces.len() > 1 && size(&pieces) > width {
        pieces.pop();
    }
    let mut x = area.x + 1;
    for (spans, target) in pieces {
        let w = u16::try_from(spans.iter().map(|s| s.content.width()).sum::<usize>()).unwrap_or(0);
        frame.render_widget(Line::from(spans), Rect::new(x, area.y, w, 1));
        if let Some(target) = target {
            hits.add(Rect::new(x, area.y, w, 1), target);
        }
        x += w + GAP as u16;
    }
}

/// The rows, or what to do with none, and the footer.
pub fn body(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    theme: &Theme,
    now: DateTime<Local>,
    hits: &mut Hits,
) {
    let Some(list) = app.rules() else {
        return;
    };
    if area.height < 2 {
        return;
    }
    footer(
        frame,
        Rect::new(area.x, area.y + area.height - 1, area.width, 1),
        app,
        theme,
        hits,
    );
    let rows = Rect::new(area.x, area.y, area.width, area.height - 1);
    if list.rules().is_empty() {
        let hint = digest::rules_empty_hint(app.keymap());
        for (offset, (text, style)) in [
            (
                digest::RULES_EMPTY.to_owned(),
                theme.style(Role::Text).add_modifier(Modifier::BOLD),
            ),
            (hint, theme.style(Role::Dim)),
        ]
        .into_iter()
        .enumerate()
        {
            let y = rows.y + rows.height / 3 + u16::try_from(offset).unwrap_or(0);
            let w = u16::try_from(text.width()).unwrap_or(0).min(rows.width);
            frame.render_widget(
                Line::styled(text, style),
                Rect::new(rows.x + rows.width.saturating_sub(w) / 2, y, w, 1),
            );
        }
        return;
    }
    let width = usize::from(area.width);
    for (offset, rule) in list
        .rules()
        .iter()
        .enumerate()
        .skip(list.top())
        .take(usize::from(rows.height))
    {
        let y = rows.y + u16::try_from(offset - list.top()).unwrap_or(0);
        let line = Rect::new(rows.x, y, rows.width, 1);
        let holds = list.holds().get(offset).copied().unwrap_or(0);
        frame.render_widget(
            row_line(rule, holds, offset == list.cursor(), width, theme, now),
            line,
        );
        hits.add(line, Target::Surface(Part::RuleRow, offset));
    }
}

/// A rule's row.
fn row_line(
    rule: &postio_config::DigestRule,
    holds: u32,
    cursor: bool,
    width: usize,
    theme: &Theme,
    now: DateTime<Local>,
) -> Line<'static> {
    let name = SafeText::new(&rule.name);
    let matches = SafeText::new(&rule.queries.join(", "));
    let when = digest::rule_when(rule).unwrap_or_default();
    let next = rule
        .due()
        .ok()
        .and_then(|due| postio_ui::schedule::next_due(&due, &now))
        .map(digest::next_delivery)
        .unwrap_or_default();
    let held = digest::holds(holds);
    let name_room = 20;
    // What gives way as the terminal narrows: the next delivery, then when.
    let mut trail: Vec<String> = vec![when, next, held];
    let used = |trail: &[String]| trail.iter().map(|t| t.width() + 2).sum::<usize>();
    let mut at = 1;
    while used(&trail) + 3 + name_room + 12 > width && at >= 1 && trail.len() > 1 {
        trail.remove(at.min(trail.len() - 2));
        at = at.saturating_sub(1);
    }
    let room = width.saturating_sub(3 + name_room + used(&trail) + 1);
    let name = fit(name.as_str(), name_room - 1);
    let pad = name_room.saturating_sub(name.width());
    let matches = fit(matches.as_str(), room);
    let gap = room.saturating_sub(matches.width());
    let mut spans = vec![
        Span::styled(if cursor { "▌" } else { " " }, theme.style(Role::Focus)),
        Span::raw("  "),
        Span::styled(name, theme.style(Role::Text).add_modifier(Modifier::BOLD)),
        Span::raw(" ".repeat(pad)),
        Span::styled(matches, theme.style(Role::Text)),
        Span::raw(" ".repeat(gap)),
    ];
    for text in trail {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(text, theme.style(Role::Dim)));
    }
    Line::from(spans)
}

/// The footer's hints, each a click for its command.
fn footer(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, hits: &mut Hits) {
    let mut x = area.x + 1;
    for hint in digest::rules_footer(app.keymap()) {
        let command = match hint.label.as_str() {
            "edit" => CommandId::OpenMessage,
            "remove and release" => CommandId::Delete,
            _ => CommandId::Back,
        };
        let key = app
            .hint(command)
            .map(|key| cap(&key))
            .unwrap_or_else(|| cap(&hint.key));
        let w = u16::try_from(key.width() + 1 + hint.label.width()).unwrap_or(u16::MAX);
        if x + w > area.x + area.width {
            break;
        }
        frame.render_widget(
            Line::from(vec![
                Span::styled(key, theme.style(Role::Accent)),
                Span::styled(format!(" {}", hint.label), theme.style(Role::Dim)),
            ]),
            Rect::new(x, area.y, w, 1),
        );
        hits.add(
            Rect::new(x, area.y, w, 1),
            Target::Command(command.as_str()),
        );
        x += w + GAP as u16;
    }
}

/// The question over `Delete`: what comes back to the inbox.
pub fn remove(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, hits: &mut Hits) {
    let Some(remove) = app.rules().and_then(|rules| rules.removing()) else {
        return;
    };
    let width = 70.min(area.width);
    let lines = wrap(
        &Line::styled(digest::remove_body(remove.holds), theme.style(Role::Text)),
        usize::from(width).saturating_sub(4).max(1),
    );
    let height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_add(5);
    let title = SafeText::new(&postio_ui::focus_target::remove_rule_title(&remove.name));
    let inside = boxed::draw(frame, area, width, height, title.as_str(), theme, hits);
    for (offset, line) in lines.into_iter().enumerate() {
        let y = inside.y + 1 + u16::try_from(offset).unwrap_or(0);
        if y < inside.y + inside.height {
            frame.render_widget(
                line,
                Rect::new(inside.x + 1, y, inside.width.saturating_sub(2), 1),
            );
        }
    }
    let key = |command| app.hint(command).map(|key| cap(&key)).unwrap_or_default();
    boxed::buttons(
        frame,
        inside,
        inside.y + inside.height.saturating_sub(1),
        &[
            Button {
                words: "Cancel".to_owned(),
                key: key(CommandId::Back),
                target: Target::Surface(Part::RemoveCancel, 0),
                primary: false,
            },
            Button {
                words: "Remove rule".to_owned(),
                key: key(CommandId::OpenMessage),
                target: Target::Surface(Part::RemoveConfirm, 0),
                primary: true,
            },
        ],
        theme,
        hits,
    );
}
