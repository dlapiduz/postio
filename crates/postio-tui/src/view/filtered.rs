//! Filtered (terminal.md, "Filtered"): the strip, the tab line, the rows
//! under their days, the footer, and the sweep's question.
//!
//! The strip is `‹ Inbox g i   Filtered · 186 today   Nothing here is
//! deleted automatically      Sweep the inbox… F`. Under it the tabs, then
//! each day's rows with the reason pill before the time. Every part is a
//! click for the command its key runs.

use chrono::{DateTime, Local};
use postio_core::CommandId;
use postio_ui::filtered;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::filtered::{Item, Slot};
use crate::surface::Part;
use crate::theme::{Role, Theme};
use crate::view::bottom::cap;
use crate::view::fit;
use crate::view::hit::{Hits, Target};

/// The gap between two items of the strip.
const GAP: usize = 3;

/// Where the sender starts: the cursor, the mark and a space.
const SENDER_X: usize = 3;

/// The widest a pill is drawn.
const PILL: usize = 30;

/// One piece of a line: its spans and the click it is.
struct Piece {
    spans: Vec<Span<'static>>,
    target: Option<Target>,
}

impl Piece {
    fn width(&self) -> usize {
        self.spans.iter().map(|span| span.content.width()).sum()
    }
}

/// The strip's pieces: where this goes back to, what it is, the note and,
/// on the right, the sweep.
fn strip_pieces(app: &App, theme: &Theme) -> (Vec<Piece>, Piece) {
    let key = |command| {
        app.hint(command)
            .map(|key| Span::styled(format!(" {}", cap(&key)), theme.style(Role::Accent)))
    };
    let mut back = vec![Span::styled("‹ Inbox", theme.style(Role::Text))];
    back.extend(key(CommandId::GoToInbox));
    let today = app
        .focus_counts()
        .map(|counts| counts.filtered_today)
        .or_else(|| app.filtered().map(|view| view.counts()[0]))
        .unwrap_or(0);
    let title = vec![
        Span::styled(
            filtered::TITLE,
            theme.style(Role::Text).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" · {}", filtered::today_short(today)),
            theme.style(Role::Dim),
        ),
    ];
    let note = filtered::NOTE.split(" · ").next().unwrap_or(filtered::NOTE);
    let mut sweep = vec![Span::styled(
        filtered::SWEEP_BUTTON,
        theme.style(Role::Text),
    )];
    sweep.extend(key(CommandId::SweepInbox));
    (
        vec![
            Piece {
                spans: back,
                target: Some(Target::Command(CommandId::GoToInbox.as_str())),
            },
            Piece {
                spans: title,
                target: None,
            },
            Piece {
                spans: vec![Span::styled(note.to_owned(), theme.style(Role::Dim))],
                target: None,
            },
        ],
        Piece {
            spans: sweep,
            target: Some(Target::Command(CommandId::SweepInbox.as_str())),
        },
    )
}

/// The strip: it takes the inbox strip's row.
pub fn strip(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, hits: &mut Hits) {
    if theme.style(Role::Surface).bg.is_some() {
        frame
            .buffer_mut()
            .set_style(area, theme.style(Role::Surface));
    }
    let (mut left, sweep) = strip_pieces(app, theme);
    let width = usize::from(area.width).saturating_sub(2);
    let span = |pieces: &[Piece]| {
        pieces.iter().map(Piece::width).sum::<usize>() + GAP * pieces.len().saturating_sub(1)
    };
    // Narrowing: the note goes first, then the title's count.
    while left.len() > 1 && span(&left) + GAP + sweep.width() > width {
        left.pop();
    }
    let mut x = area.x + 1;
    for piece in left {
        let w = u16::try_from(piece.width()).unwrap_or(u16::MAX);
        frame.render_widget(Line::from(piece.spans), Rect::new(x, area.y, w, 1));
        if let Some(target) = piece.target {
            hits.add(Rect::new(x, area.y, w, 1), target);
        }
        x += w + GAP as u16;
    }
    let w = u16::try_from(sweep.width()).unwrap_or(0);
    if usize::from(w) + 2 <= usize::from(area.width) {
        let at = (area.x + area.width).saturating_sub(w + 1);
        frame.render_widget(Line::from(sweep.spans), Rect::new(at, area.y, w, 1));
        if let Some(target) = sweep.target {
            hits.add(Rect::new(at, area.y, w, 1), target);
        }
    }
}

/// The tab line: `1 All 186  2 Promotions 90 …`, the tab on screen bold and
/// underlined, and each a click.
fn tabs(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, hits: &mut Hits) {
    let Some(view) = app.filtered() else {
        return;
    };
    let counts = view.counts();
    let words = |index: usize, named: bool| {
        let (_, name) = filtered::TABS[index];
        let key = app
            .hint(filtered::TAB_COMMANDS[index])
            .map(|key| cap(&key))
            .unwrap_or_default();
        let mut said = key;
        if named {
            if !said.is_empty() {
                said.push(' ');
            }
            said.push_str(name);
        }
        if !said.is_empty() {
            said.push(' ');
        }
        said.push_str(&counts[index].to_string());
        said
    };
    let room = usize::from(area.width).saturating_sub(2);
    let full: usize = (0..7)
        .map(|index| words(index, true).width())
        .sum::<usize>()
        + 2 * 6;
    // Too wide: only the tab on screen keeps its name.
    let named_all = full <= room;
    let mut x = area.x + 1;
    for index in 0..7 {
        let text = words(index, named_all || index == view.tab());
        let w = u16::try_from(text.width()).unwrap_or(u16::MAX);
        if x + w > area.x + area.width {
            break;
        }
        let style = if index == view.tab() {
            theme
                .style(Role::Text)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            theme.style(Role::Dim)
        };
        frame.render_widget(Line::styled(text, style), Rect::new(x, area.y, w, 1));
        hits.add(
            Rect::new(x, area.y, w, 1),
            Target::Surface(Part::FilteredTab, index),
        );
        x += w + 2;
    }
}

/// The sender's column in a body `width` cells wide.
fn sender_width(width: usize) -> usize {
    if width >= 100 { 20 } else { 16 }
}

/// A row: the cursor, a mark, the sender, the subject and its first line,
/// then the reason pill and the time.
fn row_line(
    item: &Item,
    cursor: bool,
    width: usize,
    theme: &Theme,
    now: DateTime<Local>,
) -> Line<'static> {
    let row = &item.row;
    let emphasis = if row.unread {
        theme.style(Role::Unread)
    } else {
        theme.style(Role::Text)
    };
    let sender_room = sender_width(width);
    let sender = fit(row.from.as_str(), sender_room - 1);
    let sender_pad = sender_room.saturating_sub(sender.width());
    let pill = fit(item.pill.as_str(), PILL);
    let time = format!("{:>5}", postio_ui::row::timestamp(row.when, now));
    let trail = format!("{pill}  {time}");
    let room = width.saturating_sub(SENDER_X + sender_room + trail.width() + 2);
    let subject = fit(row.subject.as_str(), room);
    let mut left = room.saturating_sub(subject.width());
    let mut spans = vec![
        Span::styled(if cursor { "▌" } else { " " }, theme.style(Role::Focus)),
        Span::styled(if row.unread { "●" } else { " " }, theme.style(Role::Text)),
        Span::raw(" "),
        Span::styled(sender, emphasis),
        Span::raw(" ".repeat(sender_pad)),
        Span::styled(subject, emphasis),
    ];
    if !row.preview.as_str().is_empty() && left >= 4 {
        let first = format!(" {}", fit(row.preview.as_str(), left - 1));
        left -= first.width();
        spans.push(Span::styled(first, theme.style(Role::Dim)));
    }
    spans.push(Span::raw(" ".repeat(left + 2)));
    spans.push(Span::styled(pill, theme.style(Role::Dim)));
    spans.push(Span::raw("  "));
    spans.push(Span::styled(time, theme.style(Role::Dim)));
    Line::from(spans)
}

/// The footer's pieces, each a click for its command.
fn footer(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, hits: &mut Hits) {
    let mut x = area.x + 1;
    for hint in filtered::footer(app.keymap()) {
        let command = match hint.label.as_str() {
            "restore + never filter sender" => Some(CommandId::RestoreFiltered),
            "inbox" => Some(CommandId::GoToInbox),
            "open" => Some(CommandId::OpenMessage),
            _ => None,
        };
        // The key as this terminal sends it, when the hint is one command's.
        let key = command
            .and_then(|command| app.hint(command))
            .map(|key| cap(&key))
            .unwrap_or_else(|| cap(&hint.key));
        let key = key.replace("Return", "↵");
        let text = format!("{key} {}", hint.label);
        let w = u16::try_from(text.width()).unwrap_or(u16::MAX);
        if x + w > area.x + area.width {
            break;
        }
        frame.render_widget(
            Line::from(vec![
                Span::styled(key.clone(), theme.style(Role::Accent)),
                Span::styled(format!(" {}", hint.label), theme.style(Role::Dim)),
            ]),
            Rect::new(x, area.y, w, 1),
        );
        if let Some(command) = command {
            hits.add(
                Rect::new(x, area.y, w, 1),
                Target::Command(command.as_str()),
            );
        }
        x += w + GAP as u16;
    }
}

/// Draw Filtered's body into `area`: the tab line, the rows, the footer.
pub fn body(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    theme: &Theme,
    now: DateTime<Local>,
    hits: &mut Hits,
) {
    let Some(view) = app.filtered() else {
        return;
    };
    if area.height < 3 {
        return;
    }
    tabs(
        frame,
        Rect::new(area.x, area.y, area.width, 1),
        app,
        theme,
        hits,
    );
    footer(
        frame,
        Rect::new(area.x, area.y + area.height - 1, area.width, 1),
        app,
        theme,
        hits,
    );
    let rows = Rect::new(area.x, area.y + 1, area.width, area.height - 2);
    let width = usize::from(area.width);
    let today = now.date_naive();
    for (offset, slot) in view
        .slots()
        .iter()
        .skip(view.top())
        .take(usize::from(rows.height))
        .enumerate()
    {
        let y = rows.y + u16::try_from(offset).unwrap_or(0);
        let line = Rect::new(rows.x, y, rows.width, 1);
        match slot {
            Slot::Heading(first) => {
                frame.render_widget(
                    Line::styled(
                        format!(
                            " {}",
                            fit(&view.heading(*first, today), width.saturating_sub(1))
                        ),
                        theme.style(Role::Text).add_modifier(Modifier::BOLD),
                    ),
                    line,
                );
            }
            Slot::Item(index) => {
                let item = &view.items()[*index];
                frame.render_widget(
                    row_line(item, *index == view.cursor(), width, theme, now),
                    line,
                );
                hits.add(line, Target::Surface(Part::FilteredRow, *index));
            }
        }
    }
    if view.items().is_empty() {
        frame.render_widget(
            Line::styled(" Nothing filtered here", theme.style(Role::Dim)),
            Rect::new(rows.x, rows.y, rows.width, 1),
        );
    }
}

/// The sweep's question over the window: how many would move, where, and
/// how to take it back.
pub fn sweep(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, hits: &mut Hits) {
    let Some(count) = app.sweep() else {
        return;
    };
    let undo = app.hint(CommandId::Undo);
    let body = filtered::sweep_body(count, undo.as_deref());
    let width = 76.min(area.width);
    let text_width = usize::from(width).saturating_sub(4).max(1);
    let lines = crate::view::wrap::wrap(&Line::styled(body, theme.style(Role::Text)), text_width);
    let height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_add(5);
    let inside = crate::view::boxed::draw(
        frame,
        area,
        width,
        height,
        filtered::SWEEP_HEADING,
        theme,
        hits,
    );
    for (offset, line) in lines.into_iter().enumerate() {
        let y = inside.y + 1 + u16::try_from(offset).unwrap_or(0);
        if y >= inside.y + inside.height {
            break;
        }
        frame.render_widget(
            line,
            Rect::new(inside.x + 1, y, inside.width.saturating_sub(2), 1),
        );
    }
    // The buttons: Cancel, and the sweep, each with its key.
    let y = inside.y + inside.height.saturating_sub(1);
    let button = |words: String, command: CommandId, primary: bool| {
        let key = app.hint(command).map(|key| cap(&key)).unwrap_or_default();
        let style = if primary {
            theme.style(Role::Text).add_modifier(Modifier::BOLD)
        } else {
            theme.style(Role::Text)
        };
        (
            vec![
                Span::styled(format!("[ {words}"), style),
                Span::styled(format!(" {key}"), theme.style(Role::Accent)),
                Span::styled(" ]", style),
            ],
            words.width() + key.width() + 5,
        )
    };
    let (cancel, cancel_width) = button("Cancel".to_owned(), CommandId::Back, false);
    let (go, go_width) = button(filtered::sweep_action(count), CommandId::OpenMessage, true);
    let total = cancel_width + 2 + go_width;
    let mut x =
        inside.x + u16::try_from(usize::from(inside.width).saturating_sub(total) / 2).unwrap_or(0);
    for (spans, w, part) in [
        (cancel, cancel_width, Part::SweepCancel),
        (go, go_width, Part::SweepConfirm),
    ] {
        let w = u16::try_from(w).unwrap_or(0);
        frame.render_widget(Line::from(spans), Rect::new(x, y, w, 1));
        hits.add(Rect::new(x, y, w, 1), Target::Surface(part, 0));
        x += w + 2;
    }
}
