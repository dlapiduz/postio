//! Filtered: the strip, the tab line, the rows under their days and the
//! footer. Every part is a click for the command its key runs.

use postio_core::CommandId;
use postio_focus::FilteredLine;
use postio_ui::filtered;
use postio_ui::terminal::SafeText;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::filtered::Slot;
use crate::surface::Part;
use crate::theme::{Role, Theme};
use crate::view::bottom::cap;
use crate::view::fit;
use crate::view::hit::{Hits, Target};

const GAP: usize = 3;

const SENDER_X: usize = 3;

const PILL: usize = 30;

struct Piece {
    spans: Vec<Span<'static>>,
    target: Option<Target>,
}

impl Piece {
    fn width(&self) -> usize {
        self.spans.iter().map(|span| span.content.width()).sum()
    }
}

fn strip_pieces(app: &App, theme: &Theme) -> (Vec<Piece>, Piece) {
    let key = |command| {
        app.hint(command)
            .map(|key| Span::styled(format!(" {}", cap(&key)), theme.style(Role::Accent)))
    };
    let mut back = vec![Span::styled("‹ Inbox", theme.style(Role::Text))];
    back.extend(key(CommandId::GoToInbox));
    let view = app.filtered().map(|shown| shown.view());
    let today = app
        .focus_counts()
        .map(|counts| counts.filtered_today)
        .or_else(|| view.and_then(|view| view.tabs.first().map(|tab| tab.count)))
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
        view.map_or(filtered::SWEEP_BUTTON, |view| view.sweep.as_str())
            .to_owned(),
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

fn tabs(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, hits: &mut Hits) {
    let Some(shown) = app.filtered() else {
        return;
    };
    let tabs = &shown.view().tabs;
    let words = |index: usize, named: bool| {
        let key = app
            .hint(filtered::TAB_COMMANDS[index])
            .map(|key| cap(&key))
            .unwrap_or_default();
        let mut said = key;
        if named {
            if !said.is_empty() {
                said.push(' ');
            }
            said.push_str(&tabs[index].name);
        }
        if !said.is_empty() {
            said.push(' ');
        }
        said.push_str(&tabs[index].count.to_string());
        said
    };
    let room = usize::from(area.width).saturating_sub(2);
    let full: usize = (0..tabs.len())
        .map(|index| words(index, true).width())
        .sum::<usize>()
        + 2 * 6;
    // Too wide: only the tab on screen keeps its name.
    let named_all = full <= room;
    let mut x = area.x + 1;
    for (index, tab) in tabs.iter().enumerate() {
        let on = tab.on;
        let text = words(index, named_all || on);
        let w = u16::try_from(text.width()).unwrap_or(u16::MAX);
        if x + w > area.x + area.width {
            break;
        }
        let style = if on {
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

fn sender_width(width: usize) -> usize {
    if width >= 100 { 20 } else { 16 }
}

fn row_line(line: &FilteredLine, cursor: bool, width: usize, theme: &Theme) -> Line<'static> {
    let unread = !line.summary.seen;
    let emphasis = if unread {
        theme.style(Role::Unread)
    } else {
        theme.style(Role::Text)
    };
    let sender_room = sender_width(width);
    let sender = fit(SafeText::new(&line.sender).as_str(), sender_room - 1);
    let sender_pad = sender_room.saturating_sub(sender.width());
    let pill = fit(SafeText::new(&line.pill).as_str(), PILL);
    let time = format!("{:>5}", line.time);
    let trail = format!("{pill}  {time}");
    let room = width.saturating_sub(SENDER_X + sender_room + trail.width() + 2);
    let subject = fit(SafeText::new(&line.subject).as_str(), room);
    let mut left = room.saturating_sub(subject.width());
    let mark = if line.selected {
        "✓"
    } else if unread {
        "●"
    } else {
        " "
    };
    let mut spans = vec![
        Span::styled(if cursor { "▌" } else { " " }, theme.style(Role::Focus)),
        Span::styled(mark, theme.style(Role::Text)),
        Span::raw(" "),
        Span::styled(sender, emphasis),
        Span::raw(" ".repeat(sender_pad)),
        Span::styled(subject, emphasis),
    ];
    let preview = SafeText::new(line.preview.as_deref().unwrap_or(""));
    if !preview.as_str().is_empty() && left >= 4 {
        let first = format!(" {}", fit(preview.as_str(), left - 1));
        left -= first.width();
        spans.push(Span::styled(first, theme.style(Role::Dim)));
    }
    spans.push(Span::raw(" ".repeat(left + 2)));
    spans.push(Span::styled(pill, theme.style(Role::Dim)));
    spans.push(Span::raw("  "));
    spans.push(Span::styled(time, theme.style(Role::Dim)));
    Line::from(spans)
}

fn footer(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, hits: &mut Hits) {
    let Some(shown) = app.filtered() else {
        return;
    };
    let view = shown.view();
    let mut x = area.x + 1;
    if let Some(bar) = &view.bar {
        let said = SafeText::new(&bar.summary);
        let w = u16::try_from(said.as_str().width()).unwrap_or(u16::MAX);
        frame.render_widget(
            Line::styled(said.as_str().to_owned(), theme.style(Role::Text)),
            Rect::new(x, area.y, w.min(area.width), 1),
        );
        x += w + GAP as u16;
    }
    let hints = view.bar.as_ref().map_or(&view.footer, |bar| &bar.hints);
    for hint in hints {
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

pub fn body(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, hits: &mut Hits) {
    let Some(shown) = app.filtered() else {
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
    let view = shown.view();
    for (offset, slot) in shown
        .slots()
        .iter()
        .skip(shown.top())
        .take(usize::from(rows.height))
        .enumerate()
    {
        let y = rows.y + u16::try_from(offset).unwrap_or(0);
        let line = Rect::new(rows.x, y, rows.width, 1);
        match slot {
            Slot::Heading(first) => {
                let heading = view.rows[*first].heading.as_deref().unwrap_or_default();
                frame.render_widget(
                    Line::styled(
                        format!(
                            " {}",
                            fit(SafeText::new(heading).as_str(), width.saturating_sub(1))
                        ),
                        theme.style(Role::Text).add_modifier(Modifier::BOLD),
                    ),
                    line,
                );
            }
            Slot::Item(index) => {
                frame.render_widget(
                    row_line(
                        &view.rows[*index],
                        Some(*index) == shown.cursor(),
                        width,
                        theme,
                    ),
                    line,
                );
                hits.add(line, Target::Surface(Part::FilteredRow, *index));
            }
        }
    }
    if let Some(empty) = &view.empty {
        frame.render_widget(
            Line::styled(format!(" {empty}"), theme.style(Role::Dim)),
            Rect::new(rows.x, rows.y, rows.width, 1),
        );
    }
}
