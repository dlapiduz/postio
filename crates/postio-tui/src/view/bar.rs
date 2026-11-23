//! The command bar, drawn (terminal.md, "The command bar").
//!
//! A rounded box `min(100, W - 4)` wide, centred, under the top bar: the
//! saved searches, the input, then the controller's lines or its search
//! dropdown, and the footer. Whatever does not fit scrolls with the chosen
//! line.

use postio_core::CommandId;
use postio_focus::{BarLineKind, DropdownRow, DropdownView, Run};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line as Text, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear};
use unicode_width::UnicodeWidthStr;

use crate::bar::{Bar, Shown};
use crate::theme::{Role, Theme};
use crate::view::fit;
use crate::view::hit::{Hits, Target};

const WIDEST: u16 = 100;

fn cells(text: &str) -> u16 {
    u16::try_from(text.width()).unwrap_or(u16::MAX)
}

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

/// One line of the box: what it says, and the stop it is when the arrows
/// can rest on it.
struct Drawn {
    spans: Vec<Span<'static>>,
    /// Pills along the line: each one's stop, where it starts and how wide.
    pills: Vec<(usize, u16, u16)>,
    right: Option<(String, Style)>,
    stop: Option<usize>,
    heading: bool,
}

fn runs(runs: &[Run], base: Style, accent: Style) -> Vec<Span<'static>> {
    use postio_focus::RunStyle;
    runs.iter()
        .map(|run| {
            let style = match (run.highlighted, run.style) {
                (true, _) => accent.add_modifier(Modifier::BOLD),
                (_, RunStyle::Strong) => base.add_modifier(Modifier::BOLD),
                _ => base,
            };
            Span::styled(
                postio_ui::terminal::SafeText::new(&run.text).to_string(),
                style,
            )
        })
        .collect()
}

fn lines_of(bar: &Bar, theme: &Theme) -> Vec<Drawn> {
    let (dim, text, accent) = (
        theme.style(Role::Dim),
        theme.style(Role::Text),
        theme.style(Role::Accent),
    );
    let stops = bar.stops();
    let stop_of = |token: u64| stops.iter().position(|stop| *stop == token);
    let mut out = Vec::new();
    match bar.shown() {
        Shown::Lines(view) => {
            if let Some(echo) = &view.echo {
                out.push(Drawn {
                    spans: vec![Span::styled(format!(" {echo}"), dim)],
                    right: None,
                    stop: None,
                    pills: Vec::new(),
                    heading: false,
                });
            }
            for line in &view.lines {
                if line.kind == BarLineKind::Heading {
                    out.push(Drawn {
                        spans: vec![Span::styled(
                            format!(" {}", line.title),
                            dim.add_modifier(Modifier::BOLD),
                        )],
                        right: None,
                        stop: None,
                        pills: Vec::new(),
                        heading: true,
                    });
                    continue;
                }
                let mut spans = vec![Span::styled(
                    line.title.clone(),
                    if line.selectable { text } else { dim },
                )];
                if let Some(detail) = &line.detail {
                    spans.push(Span::styled(format!("  {detail}"), dim));
                }
                out.push(Drawn {
                    spans,
                    right: line.key.clone().map(|key| (key, accent)),
                    stop: stop_of(line.token).filter(|_| line.selectable),
                    pills: Vec::new(),
                    heading: false,
                });
            }
        }
        Shown::Dropdown(view) => {
            if !view.understood.is_empty() {
                let said: Vec<String> = view
                    .understood
                    .iter()
                    .map(|tile| format!("{}:{}", tile.op, tile.value))
                    .collect();
                out.push(Drawn {
                    spans: vec![Span::styled(format!(" {}", said.join("  ")), accent)],
                    right: None,
                    stop: None,
                    pills: Vec::new(),
                    heading: false,
                });
            }
            for section in &view.sections {
                out.push(Drawn {
                    spans: vec![Span::styled(
                        format!(" {}", section.title),
                        dim.add_modifier(Modifier::BOLD),
                    )],
                    right: section.note.clone().map(|note| (note, dim)),
                    stop: None,
                    pills: Vec::new(),
                    heading: true,
                });
                for row in &section.rows {
                    out.push(dropdown_row(row, text, dim, accent, stop_of(row.token)));
                }
                if !section.pills.is_empty() {
                    let mut spans = vec![Span::raw("  ")];
                    let mut at = 3;
                    let mut pills = Vec::new();
                    for pill in &section.pills {
                        let mut said = pill.label.clone();
                        for extra in [&pill.count, &pill.fresh, &pill.key].into_iter().flatten() {
                            said.push(' ');
                            said.push_str(extra);
                        }
                        let width = cells(&said);
                        if let Some(stop) = stop_of(pill.token) {
                            pills.push((stop, at, width));
                        }
                        spans.push(Span::styled(said, text));
                        spans.push(Span::raw("  "));
                        at += width + 2;
                    }
                    out.push(Drawn {
                        spans,
                        right: None,
                        stop: None,
                        pills,
                        heading: false,
                    });
                }
            }
        }
    }
    out
}

fn dropdown_row(
    row: &DropdownRow,
    text: Style,
    dim: Style,
    accent: Style,
    stop: Option<usize>,
) -> Drawn {
    let mut spans = runs(&row.title, text, accent);
    if !row.detail.is_empty() {
        spans.push(Span::raw("  "));
        spans.extend(runs(&row.detail, dim, accent));
    }
    let right = match (&row.right, &row.key) {
        (Some(right), _) => Some((right.clone(), dim)),
        (None, Some(key)) => Some((key.clone(), accent)),
        _ => None,
    };
    Drawn {
        spans,
        right,
        stop: stop.filter(|_| row.selectable),
        pills: Vec::new(),
        heading: false,
    }
}

fn footer_of(bar: &Bar) -> String {
    match bar.shown() {
        Shown::Dropdown(DropdownView { hints, count, .. }) => {
            let mut said: Vec<String> = hints
                .iter()
                .map(|hint| format!("{} {}", hint.key, hint.label))
                .collect();
            said.extend(count.clone());
            said.join("  ")
        }
        Shown::Lines(_) => format!(
            "\u{2191}\u{2193} move  \u{21b5} run  {} commands only",
            postio_ui::finder::COMMANDS_ONLY
        ),
    }
}

/// Draw `bar` over `area`.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    bar: &Bar,
    hint: &dyn Fn(CommandId) -> Option<String>,
    theme: &Theme,
    hits: &mut Hits,
) {
    let lines = lines_of(bar, theme);
    let saved: &[String] = match bar.shown() {
        Shown::Lines(view) => &view.saved,
        Shown::Dropdown(_) => &[],
    };
    let typed = bar.typed();
    let fixed = 2 + u16::from(!saved.is_empty()) + 1 + 1 + 1;
    let room = area.height.saturating_sub(2).saturating_sub(fixed);
    let body = u16::try_from(lines.len()).unwrap_or(u16::MAX).min(room);
    let box_ = outer(area, fixed + body + u16::from(body > 0));
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
    let (dim, accent, text) = (
        theme.style(Role::Dim),
        theme.style(Role::Accent),
        theme.style(Role::Text),
    );
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
        for (index, name) in saved.iter().enumerate() {
            let key = hint(postio_ui::command_bar::SAVED[index.min(3)]).filter(|_| index < 4);
            let label = format!(
                "  {name}{}",
                key.as_deref().map(|k| format!(" {k}")).unwrap_or_default()
            );
            let width = cells(&label);
            if x + width + 12 > end {
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
        if let Some(save) = hint(CommandId::SaveSearch).filter(|_| !typed.trim().is_empty()) {
            let said = format!("{save} saves");
            let w = cells(&said);
            let at = Rect::new(end - w, y, w, 1);
            frame.render_widget(Text::styled(said, dim), at);
            hits.add(at, Target::Command(CommandId::SaveSearch.as_str()));
        }
        y += 1;
    }
    let esc = hint(CommandId::Back)
        .unwrap_or_default()
        .replace("Escape", "Esc");
    let mut spans = vec![Span::styled("⌕ ", accent)];
    let room = usize::from(inner.width).saturating_sub(4 + usize::from(cells(&esc)));
    if typed.is_empty() {
        spans.push(Span::styled(postio_ui::focus_row::COMMAND_PROMPT, dim));
    } else {
        spans.push(Span::styled(fit(typed, room), text));
        if let Shown::Dropdown(DropdownView {
            ghost: Some(ghost), ..
        }) = bar.shown()
        {
            let left = room.saturating_sub(typed.width());
            spans.push(Span::styled(fit(ghost, left), dim));
        }
    }
    frame.render_widget(Text::from(spans), Rect::new(inner.x, y, inner.width, 1));
    if !esc.is_empty() {
        let w = cells(&esc);
        let at = Rect::new(inner.x + inner.width - w - 1, y, w, 1);
        frame.render_widget(Text::styled(esc, dim), at);
        hits.add(at, Target::Command(CommandId::Back.as_str()));
    }
    let before: String = typed.chars().take(bar.caret()).collect();
    frame.set_cursor_position(Position::new(
        (inner.x + 2 + cells(&before)).min(inner.x + inner.width - 1),
        y,
    ));
    y += 1;
    rule(frame, y);
    y += 1;

    let chosen = bar
        .selected()
        .and_then(|token| bar.stops().iter().position(|stop| *stop == token));
    let shown = usize::from(body);
    let chosen_line = lines
        .iter()
        .position(|line| line.stop.is_some() && line.stop == chosen)
        .unwrap_or(0);
    let first = if shown == 0 {
        0
    } else {
        (chosen_line + 1).saturating_sub(shown)
    };
    for (offset, line) in lines.iter().skip(first).take(shown).enumerate() {
        let row = Rect::new(
            inner.x,
            y + u16::try_from(offset).unwrap_or(0),
            inner.width,
            1,
        );
        let on = line.stop.is_some() && line.stop == chosen;
        if on {
            frame
                .buffer_mut()
                .set_style(row, theme.style(Role::Surface));
        }
        let mut spans = vec![Span::styled(
            if on { "▌" } else { " " },
            theme.style(Role::Focus),
        )];
        let right_width = line.right.as_ref().map_or(0, |(said, _)| cells(said) + 1);
        let room = usize::from(row.width).saturating_sub(usize::from(right_width) + 2);
        let mut used = 0;
        for span in &line.spans {
            let content = fit(&span.content, room.saturating_sub(used));
            used += content.width();
            let style = if on && !line.heading {
                span.style.add_modifier(Modifier::BOLD)
            } else {
                span.style
            };
            spans.push(Span::styled(content, style));
        }
        frame.render_widget(Text::from(spans), row);
        if let Some((said, style)) = &line.right {
            let w = cells(said);
            frame.render_widget(
                Text::styled(said.clone(), *style),
                Rect::new(row.x + row.width - w - 1, row.y, w, 1),
            );
        }
        if let Some(stop) = line.stop {
            hits.add(row, Target::BarRow(stop));
        }
        for (stop, at, width) in &line.pills {
            hits.add(
                Rect::new(row.x + at, row.y, *width, 1),
                Target::BarRow(*stop),
            );
            if chosen == Some(*stop) {
                frame.buffer_mut().set_style(
                    Rect::new(row.x + at, row.y, *width, 1),
                    theme.style(Role::Surface),
                );
            }
        }
    }
    y += body;
    if body > 0 {
        rule(frame, y);
        y += 1;
    }
    frame.render_widget(
        Text::styled(
            fit(&footer_of(bar), usize::from(inner.width).saturating_sub(2)),
            dim,
        ),
        Rect::new(inner.x + 1, y, inner.width.saturating_sub(1), 1),
    );
    if matches!(bar.shown(), Shown::Lines(_)) {
        let move_width = cells("\u{2191}\u{2193} move");
        let run_width = cells("\u{21b5} run");
        let commands_width = cells(&format!(
            "{} commands only",
            postio_ui::finder::COMMANDS_ONLY
        ));
        for (offset, width, target) in [
            (move_width + 2, run_width, Target::BarRun),
            (
                move_width + 2 + run_width + 2,
                commands_width,
                Target::BarCommands,
            ),
        ] {
            if offset + width <= inner.width.saturating_sub(1) {
                hits.add(Rect::new(inner.x + 1 + offset, y, width, 1), target);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::{App, update};
    use crate::test_support::{
        app, ctrl, open_list, places, places_arrive, press, saved_search, screen, seed_places,
        serve, type_text,
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

    fn opened(app: &mut App, keys: impl FnOnce(&mut App) -> Vec<crate::app::Effect>) {
        let effects = keys(app);
        let folders = app.folders_for_tests();
        places_arrive(app, &effects, &folders, &[], &[]);
    }

    #[test]
    fn slash_opens_a_box_under_the_top_bar_with_the_saved_searches_the_input_and_the_hints() {
        let mut app = with_saved((120, 36));
        opened(&mut app, |app| update(app, press('/')));
        let drawn = screen(120, 36, &app);
        let lines: Vec<&str> = drawn.lines().collect();
        // min(100, W - 4) wide, centred: columns 10 to 109, from the second row.
        let cell = |line: &str, at: usize| line.chars().nth(at);
        assert_eq!(cell(lines[1], 10), Some('╭'), "{drawn}");
        assert_eq!(cell(lines[1], 109), Some('╮'), "{drawn}");
        assert!(lines[2].contains("⌕ Search mail"), "{drawn}");
        assert!(lines[2].contains("Esc │"), "{drawn}");
        assert!(drawn.contains("Saved searches"), "{drawn}");
        assert!(
            drawn.contains("Waiting on reply alt+1") && drawn.contains("Atlas alt+2"),
            "{drawn}"
        );
        assert!(drawn.contains("from:"), "{drawn}");
    }

    #[test]
    fn typing_goes_to_the_controller_and_what_it_says_is_drawn() {
        let mut app = with_saved((120, 36));
        opened(&mut app, |app| update(app, press('/')));
        type_text(&mut app, "from:ada tide");
        let drawn = screen(120, 36, &app);
        assert!(drawn.contains("⌕ from:ada tide"), "{drawn}");
        assert!(
            !drawn.contains("Saved searches"),
            "the empty state is gone:\n{drawn}"
        );
        assert!(drawn.contains("Return open"), "{drawn}");
    }

    #[test]
    fn ctrl_k_opens_it_with_commands_only() {
        let mut app = with_saved((120, 36));
        opened(&mut app, |app| update(app, ctrl('k')));
        let drawn = screen(120, 36, &app);
        assert!(drawn.contains("⌕ >"), "{drawn}");
        assert!(!drawn.contains("Go to"), "{drawn}");
    }
}
