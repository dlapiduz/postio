//! The digest window: the message frame holding one delivery's summary,
//! plain list or email. What came from mail is made safe here.

use chrono::{DateTime, Local};
use postio_core::CommandId;
use postio_ui::digest::Page;
use postio_ui::terminal::SafeText;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::digest::Window;
use crate::surface::Part;
use crate::theme::{Role, Theme};
use crate::view::bottom::cap;
use crate::view::fit;
use crate::view::hit::{Hits, Target};
use crate::view::wrap::wrap;

const GAP: usize = 2;

#[derive(Debug, Clone)]
pub struct Ln {
    pub line: Line<'static>,
    pub reference: Option<usize>,
}

pub fn summary_lines(
    window: &Window,
    theme: &Theme,
    width: usize,
    key: Option<String>,
) -> (Vec<Ln>, Option<(usize, usize)>) {
    let view = window.view();
    let text = theme.style(Role::Text);
    let dim = theme.style(Role::Dim);
    let surface = theme.style(Role::Surface);
    let mut lines: Vec<Ln> = Vec::new();
    let mut focus = None;
    for (index, topic) in view.topics.iter().enumerate() {
        if index > 0 {
            lines.push(Ln {
                line: Line::default(),
                reference: None,
            });
        }
        lines.push(Ln {
            line: Line::styled(
                SafeText::new(&topic.heading).as_str().to_owned(),
                text.add_modifier(Modifier::BOLD),
            ),
            reference: None,
        });
        for statement in &topic.statements {
            let at = statement.index as usize;
            let focused = window.reference() == Some(at);
            let mark = format!("[{}]", statement.number);
            let mark_style = if focused {
                theme
                    .style(Role::Accent)
                    .add_modifier(Modifier::REVERSED | Modifier::BOLD)
            } else {
                theme.style(Role::Accent)
            };
            let paragraph = Line::from(vec![
                Span::styled(SafeText::new(&statement.text).as_str().to_owned(), text),
                Span::raw(" "),
                Span::styled(mark, mark_style),
            ]);
            let start = lines.len();
            for line in wrap(&paragraph, width) {
                lines.push(Ln {
                    line,
                    reference: Some(at),
                });
            }
            if focused && let Some(shown) = &view.card {
                for line in card(shown, key.as_deref(), theme, width) {
                    lines.push(Ln {
                        line: line.style(surface),
                        reference: Some(at),
                    });
                }
            }
            if focused {
                focus = Some((start, lines.len()));
            }
        }
    }
    if let Some(footer) = &view.footer {
        lines.push(Ln {
            line: Line::default(),
            reference: None,
        });
        for line in wrap(
            &Line::styled(SafeText::new(footer).as_str().to_owned(), dim),
            width,
        ) {
            lines.push(Ln {
                line,
                reference: None,
            });
        }
    }
    (lines, focus)
}

fn card(
    card: &postio_focus::DigestCard,
    key: Option<&str>,
    theme: &Theme,
    width: usize,
) -> Vec<Line<'static>> {
    let open = match key {
        Some(key) => format!("{} {}", cap(key), card.hint),
        None => card.hint.clone(),
    };
    let pad = |text: String, style| {
        let text = fit(&text, width.saturating_sub(2));
        let gap = width.saturating_sub(text.width() + 2);
        Line::from(vec![
            Span::raw("  "),
            Span::styled(text, style),
            Span::raw(" ".repeat(gap)),
        ])
    };
    vec![
        pad(
            SafeText::new(&card.title).as_str().to_owned(),
            theme.style(Role::Text),
        ),
        pad(open, theme.style(Role::Accent)),
    ]
}

pub fn email_lines(
    app: &App,
    window: &Window,
    theme: &Theme,
    width: u16,
    now: DateTime<Local>,
) -> Vec<Line<'static>> {
    let Some(email) = &window.view().email else {
        return Vec::new();
    };
    let mut out: Vec<Line<'static>> = Vec::new();
    if let Some(banner) = &email.banner {
        out.extend(wrap(
            &Line::styled(
                SafeText::new(banner).as_str().to_owned(),
                theme.style(Role::Accent),
            ),
            usize::from(width),
        ));
        out.push(Line::default());
    }
    let reading = app.reading().filter(|reading| reading.row == email.message);
    if reading.is_none() {
        out.push(Line::styled("…", theme.style(Role::Dim)));
        return out;
    }
    let document = crate::view::open::document(app, theme, width, now);
    // The column opens with the subject and pills, which the title and the
    // header already say: it starts at the sender grid, under the hairline.
    let from = document
        .lines
        .iter()
        .position(|line| line.line.to_string().starts_with('─'))
        .map_or(0, |at| at + 1);
    let mut lines: Vec<Line<'static>> = document
        .lines
        .iter()
        .skip(from)
        .map(|line| line.line.clone())
        .collect();
    if let Some(excerpt) = email
        .excerpt
        .as_deref()
        .filter(|excerpt| !excerpt.is_empty())
    {
        let mark = crate::view::open::surface_underline(theme);
        for (index, start, end) in crate::view::open::locate(&lines, excerpt) {
            lines[index] = crate::view::open::restyle(&lines[index], start, end, mark);
        }
    }
    out.extend(lines);
    out
}

pub fn extent(app: &App, width: u16) -> usize {
    let Some(window) = app.digest() else {
        return 0;
    };
    let theme = Theme::plain();
    match window.page() {
        Page::Summary => summary_lines(window, &theme, usize::from(width), None)
            .0
            .len(),
        Page::List => window.view().rows.len(),
        Page::Email => email_lines(app, window, &theme, width, Local::now()).len(),
    }
}

pub fn focus_rows(app: &App, width: u16) -> Option<(usize, usize)> {
    let window = app.digest()?;
    summary_lines(window, &Theme::plain(), usize::from(width), None).1
}

fn list_line(
    line: &postio_focus::DigestLine,
    cursor: bool,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let sender_room = if width >= 100 { 20 } else { 16 };
    let sender = fit(SafeText::new(&line.sender).as_str(), sender_room - 1);
    let pad = sender_room.saturating_sub(sender.width());
    let time = format!("{:>5}", line.time);
    let room = width.saturating_sub(3 + sender_room + time.width() + 2);
    let subject = fit(SafeText::new(&line.subject).as_str(), room);
    let mut left = room.saturating_sub(subject.width());
    let mut spans = vec![
        Span::styled(if cursor { "▌" } else { " " }, theme.style(Role::Focus)),
        Span::raw("  "),
        Span::styled(sender, theme.style(Role::Text)),
        Span::raw(" ".repeat(pad)),
        Span::styled(subject, theme.style(Role::Text)),
    ];
    let preview = SafeText::new(line.preview.as_deref().unwrap_or(""));
    if !preview.as_str().is_empty() && left >= 4 {
        let first = format!(" {}", fit(preview.as_str(), left - 1));
        left -= first.width();
        spans.push(Span::styled(first, theme.style(Role::Dim)));
    }
    spans.push(Span::raw(" ".repeat(left + 2)));
    spans.push(Span::styled(time, theme.style(Role::Dim)));
    Line::from(spans)
}

struct Hint {
    key: String,
    words: &'static str,
    command: Option<&'static str>,
}

fn hints(app: &App, window: &Window) -> Vec<Hint> {
    let key = |command| app.hint(command).map(|key| cap(&key)).unwrap_or_default();
    let both = |a, b| format!("{} {}", key(a), key(b));
    let mut hints = Vec::new();
    match window.page() {
        Page::Summary => {
            hints.push(Hint {
                key: both(CommandId::NextReference, CommandId::PrevReference),
                words: "next / previous reference",
                command: Some(CommandId::NextReference.as_str()),
            });
            hints.push(Hint {
                key: key(CommandId::OpenMessage),
                words: "open",
                command: Some(CommandId::OpenMessage.as_str()),
            });
        }
        Page::List => hints.push(Hint {
            key: key(CommandId::OpenMessage),
            words: "open",
            command: Some(CommandId::OpenMessage.as_str()),
        }),
        Page::Email => {
            hints.push(Hint {
                key: both(CommandId::NextMessage, CommandId::PrevMessage),
                words: "next / previous source",
                command: Some(CommandId::NextMessage.as_str()),
            });
            hints.push(Hint {
                key: key(CommandId::Back),
                words: "back",
                command: Some(CommandId::Back.as_str()),
            });
        }
    }
    if window.page() != Page::Email && window.view().tabs {
        hints.push(Hint {
            key: key(CommandId::ToggleDigestSummary),
            words: "summary / messages",
            command: Some(CommandId::ToggleDigestSummary.as_str()),
        });
    }
    hints.push(Hint {
        key: key(CommandId::StopDigestingSender),
        words: "stop digesting the sender",
        command: Some(CommandId::StopDigestingSender.as_str()),
    });
    hints.push(Hint {
        key: key(CommandId::Unsubscribe),
        words: "unsubscribe",
        command: Some(CommandId::Unsubscribe.as_str()),
    });
    hints
}

pub fn over_list(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    theme: &Theme,
    now: DateTime<Local>,
    hits: &mut Hits,
) {
    let Some(window) = app.digest() else {
        return;
    };
    let view = window.view();
    let Some((outer, inside)) = crate::view::open::framed(frame, area, theme, hits) else {
        return;
    };
    if inside.width < 20 || inside.height < 7 {
        return;
    }
    let (dim, text, accent) = (
        theme.style(Role::Dim),
        theme.style(Role::Text),
        theme.style(Role::Accent),
    );
    let width = usize::from(inside.width);
    // An email takes the header's subtitle line and Archive all away.
    let email = !window.page().shows_header();
    let row = |y: u16| Rect::new(inside.x, inside.y + y, inside.width, 1);

    // Row 1: Archive all (or the way back), the title, Esc ✕.
    let mut left: Vec<Span<'static>> = Vec::new();
    let (left_command, left_words) = if email {
        (CommandId::Back, format!("‹ {}", SafeText::new(&view.back)))
    } else {
        (CommandId::Archive, view.archive.clone())
    };
    left.push(Span::styled(
        left_words,
        if email {
            text
        } else {
            text.add_modifier(Modifier::BOLD)
        },
    ));
    if let Some(key) = app.hint(left_command) {
        left.push(Span::styled(format!(" {}", cap(&key)), accent));
    }
    let left_width: usize = left.iter().map(|span| span.content.width()).sum();
    let mut right: Vec<Span<'static>> = Vec::new();
    if let Some(key) = app.hint(CommandId::Back) {
        right.push(Span::styled(format!("{} ", cap(&key)), dim));
    }
    right.push(Span::styled("✕", text));
    let right_width: usize = right.iter().map(|span| span.content.width()).sum();
    let title = SafeText::new(&view.title).as_str().to_owned();
    let title = if email && title.trim().is_empty() {
        "(no subject)".to_owned()
    } else {
        title
    };
    let room = width.saturating_sub(2 * (left_width.max(right_width) + 3));
    let title = fit(&title, room.max(8));
    let centred = width.saturating_sub(title.width()) / 2;
    frame.render_widget(
        Line::from(left),
        Rect::new(
            inside.x + 1,
            inside.y,
            u16::try_from(left_width).unwrap_or(0),
            1,
        ),
    );
    hits.add(
        Rect::new(
            inside.x + 1,
            inside.y,
            u16::try_from(left_width).unwrap_or(0),
            1,
        ),
        Target::Command(left_command.as_str()),
    );
    frame.render_widget(
        Line::styled(title, text.add_modifier(Modifier::BOLD)),
        Rect::new(
            inside.x + u16::try_from(centred).unwrap_or(0),
            inside.y,
            inside.width - u16::try_from(centred).unwrap_or(0),
            1,
        ),
    );
    let rx = inside.x + inside.width - u16::try_from(right_width + 1).unwrap_or(1);
    frame.render_widget(
        Line::from(right),
        Rect::new(rx, inside.y, u16::try_from(right_width).unwrap_or(0), 1),
    );
    hits.add(
        Rect::new(rx, inside.y, u16::try_from(right_width).unwrap_or(0), 1),
        Target::Command(CommandId::Back.as_str()),
    );

    // Row 2: the subtitle.
    let subtitle = SafeText::new(&view.subtitle).as_str().to_owned();
    let subtitle = fit(&subtitle, width.saturating_sub(2));
    let at = width.saturating_sub(subtitle.width()) / 2;
    frame.render_widget(
        Line::styled(subtitle, dim),
        Rect::new(
            inside.x + u16::try_from(at).unwrap_or(0),
            inside.y + 1,
            inside.width - u16::try_from(at).unwrap_or(0),
            1,
        ),
    );

    // Row 3: the tabs, and the rule's line on the right.
    if !email {
        let has_summary = view.tabs;
        let on = |page| {
            if window.page() == page {
                text.add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                dim
            }
        };
        let mut x = inside.x + 1;
        let messages = view.list_tab.clone();
        let mut tabs: Vec<(String, ratatui::style::Style, usize)> = Vec::new();
        if has_summary {
            tabs.push(("Summary".to_owned(), on(Page::Summary), 0));
        }
        tabs.push((messages, on(Page::List), 1));
        for (words, style, index) in tabs {
            let w = u16::try_from(words.width()).unwrap_or(0);
            frame.render_widget(Line::styled(words, style), Rect::new(x, inside.y + 2, w, 1));
            hits.add(
                Rect::new(x, inside.y + 2, w, 1),
                Target::Surface(Part::DigestTab, index),
            );
            x += w + 2;
        }
        if has_summary && let Some(key) = app.hint(CommandId::ToggleDigestSummary) {
            let key = cap(&key);
            let w = u16::try_from(key.width()).unwrap_or(0);
            frame.render_widget(Line::styled(key, accent), Rect::new(x, inside.y + 2, w, 1));
            hits.add(
                Rect::new(x, inside.y + 2, w, 1),
                Target::Command(CommandId::ToggleDigestSummary.as_str()),
            );
            x += w + 2;
        }
        let mut said = view.rule_line.clone();
        let key = app
            .hint(CommandId::DigestRule)
            .map(|key| format!(" {}", cap(&key)))
            .unwrap_or_default();
        let room =
            (usize::from(inside.x + inside.width)).saturating_sub(usize::from(x) + 3 + key.width());
        if said.width() > room {
            said = postio_ui::digest::rule_line(None);
        }
        if said.width() <= room {
            let total = said.width() + key.width();
            let at = inside.x + inside.width - u16::try_from(total + 1).unwrap_or(1);
            frame.render_widget(
                Line::from(vec![Span::styled(said, dim), Span::styled(key, accent)]),
                Rect::new(at, inside.y + 2, u16::try_from(total).unwrap_or(0), 1),
            );
            hits.add(
                Rect::new(at, inside.y + 2, u16::try_from(total).unwrap_or(0), 1),
                Target::Command(CommandId::DigestRule.as_str()),
            );
        }
    }
    frame.render_widget(Line::styled("─".repeat(width), dim), row(3));

    // The page.
    let body_height = inside.height.saturating_sub(5);
    let body = Rect::new(inside.x, inside.y + 4, inside.width, body_height);
    let column = crate::layout::column_width(outer.width).min(inside.width);
    let x = inside.x + (inside.width - column) / 2;
    match window.page() {
        Page::Summary => {
            let (lines, _) = summary_lines(
                window,
                theme,
                usize::from(column),
                app.hint(CommandId::OpenMessage),
            );
            for (offset, ln) in lines
                .iter()
                .skip(window.top())
                .take(usize::from(body_height))
                .enumerate()
            {
                let rect = Rect::new(x, body.y + u16::try_from(offset).unwrap_or(0), column, 1);
                frame.render_widget(ln.line.clone(), rect);
                if let Some(index) = ln.reference {
                    hits.add(rect, Target::Surface(Part::DigestReference, index));
                }
            }
        }
        Page::List => {
            for (offset, item) in view
                .rows
                .iter()
                .enumerate()
                .skip(window.top())
                .take(usize::from(body_height))
            {
                let y = body.y + u16::try_from(offset - window.top()).unwrap_or(0);
                let rect = Rect::new(body.x, y, body.width, 1);
                frame.render_widget(
                    list_line(item, Some(offset) == window.cursor(), width, theme),
                    rect,
                );
                hits.add(rect, Target::Surface(Part::DigestRow, offset));
            }
            if view.rows.is_empty() {
                let said = if view.loading { "…" } else { "Nothing here" };
                frame.render_widget(
                    Line::styled(format!(" {said}"), dim),
                    Rect::new(body.x, body.y, body.width, 1),
                );
            }
        }
        Page::Email => {
            let lines = email_lines(app, window, theme, column, now);
            for (offset, line) in lines
                .iter()
                .skip(window.top())
                .take(usize::from(body_height))
                .enumerate()
            {
                frame.render_widget(
                    line.clone(),
                    Rect::new(x, body.y + u16::try_from(offset).unwrap_or(0), column, 1),
                );
            }
        }
    }

    // The last row: the window's keys, as many as fit.
    let keys = Rect::new(inside.x, inside.y + inside.height - 1, inside.width, 1);
    let all = hints(app, window);
    let widths: Vec<usize> = all
        .iter()
        .map(|hint| hint.key.width() + 1 + hint.words.width())
        .collect();
    // A margin of one when the whole line leaves it, else none.
    let whole: usize = widths.iter().sum::<usize>() + GAP * widths.len().saturating_sub(1);
    let margin = u16::from(whole + 2 <= usize::from(keys.width));
    let mut x = keys.x + margin;
    for (hint, w) in all.into_iter().zip(widths) {
        let w = u16::try_from(w).unwrap_or(u16::MAX);
        if x + w > keys.x + keys.width {
            break;
        }
        frame.render_widget(
            Line::from(vec![
                Span::styled(hint.key.clone(), accent),
                Span::styled(format!(" {}", hint.words), dim),
            ]),
            Rect::new(x, keys.y, w, 1),
        );
        if let Some(command) = hint.command {
            hits.add(Rect::new(x, keys.y, w, 1), Target::Command(command));
        }
        x += w + GAP as u16;
    }
}
