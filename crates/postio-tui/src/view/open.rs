//! The open message (terminal.md, "The open message").
//!
//! A frame over the list, or a pane beside it, holds three header rows --
//! the step keys, the subject and `Esc ✕`; the position line; the action
//! row -- and under a hairline one column that scrolls as a whole: the
//! thread line, the subject, the labels, the sender grid, the action card,
//! the body, the attachments and the fold line. [`document`] lays the column
//! out as rows; [`draw`] puts the header and the visible rows on screen and
//! records what each is for, so the mouse does what the keys do.
//!
//! Everything from mail reaches here as `SafeText` or as lines `reader`
//! already made safe.

use chrono::{DateTime, Local};
use postio_core::CommandId;
use postio_ui::focus_dialog;
use postio_ui::terminal::SafeText;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::App;
use crate::conversation::{At, Member};
use crate::reader::{Block, LineTarget};
use crate::row::Row;
use crate::theme::{Role, Theme};
use crate::view::bottom::cap;
use crate::view::fit;
use crate::view::hit::{Hits, Target};
use crate::view::wrap::wrap;

/// A piece of a row that is a click of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// Columns in from the column's left edge.
    pub start: u16,
    /// How wide it is.
    pub width: u16,
    /// The command a click runs.
    pub command: &'static str,
}

/// One row of the scrolling column.
#[derive(Debug, Clone)]
pub struct DocLine {
    /// What is drawn.
    pub line: Line<'static>,
    /// What a click on the row, outside its segments, stands for.
    pub at: At,
    /// The clicks within it.
    pub segments: Vec<Segment>,
}

/// The column, row by row.
#[derive(Debug, Clone, Default)]
pub struct Document {
    /// Every row, top to bottom.
    pub lines: Vec<DocLine>,
}

impl Document {
    /// What a click on column `column` of row `row` stands for.
    pub fn click(&self, row: usize, column: u16) -> Click {
        let Some(line) = self.lines.get(row) else {
            return Click::Nothing;
        };
        match line
            .segments
            .iter()
            .find(|segment| column >= segment.start && column < segment.start + segment.width)
        {
            Some(segment) => Click::Command(segment.command),
            None => Click::At(line.at.clone()),
        }
    }
}

/// What a click on the column did land on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Click {
    /// Nothing to act on.
    Nothing,
    /// A command's own words.
    Command(&'static str),
    /// A link, a fold or an attachment.
    At(At),
}

/// A builder of the column's rows.
struct Rows<'a> {
    width: usize,
    theme: &'a Theme,
    lines: Vec<DocLine>,
}

impl<'a> Rows<'a> {
    fn push(&mut self, line: Line<'static>, at: At, segments: Vec<Segment>) {
        self.lines.push(DocLine { line, at, segments });
    }

    fn plain(&mut self, line: Line<'static>) {
        self.push(line, At::Nothing, Vec::new());
    }

    fn blank(&mut self) {
        self.plain(Line::default());
    }

    fn hairline(&mut self) {
        self.plain(Line::styled(
            "─".repeat(self.width),
            self.theme.style(Role::Dim),
        ));
    }
}

/// The width of `spans`.
fn width_of(spans: &[Span]) -> usize {
    spans.iter().map(|span| span.content.width()).sum()
}

/// `names` as a header draws them: the first few, and how many are left.
fn names_line(names: &[SafeText]) -> String {
    use postio_ui::reader::header::RECIPIENTS_SHOWN;
    let shown: Vec<&str> = names
        .iter()
        .take(RECIPIENTS_SHOWN)
        .map(SafeText::as_str)
        .collect();
    let mut said = shown.join(", ");
    if names.len() > RECIPIENTS_SHOWN {
        let hidden = names.len() - RECIPIENTS_SHOWN;
        said.push_str(&format!(
            " and {hidden} {}",
            if hidden == 1 { "other" } else { "others" }
        ));
    }
    said
}

/// The message on screen: the member the keyboard is on.
fn shown(app: &App) -> Option<(Option<&Row>, &Member)> {
    let reading = app.reading()?;
    let member = reading.members.get(reading.current)?;
    Some((app.row(reading.row), member))
}

/// The marker's sentence found in the body's own words, as the character
/// ranges to mark on each line of `lines`: whitespace is not part of what is
/// matched, so a sentence the sender wrapped is still found.
pub(super) fn locate(lines: &[Line<'static>], excerpt: &str) -> Vec<(usize, usize, usize)> {
    let wanted: Vec<&str> = excerpt.split_whitespace().collect();
    if wanted.is_empty() {
        return Vec::new();
    }
    // Every word of the body: its line and where it starts and ends.
    let mut words: Vec<(usize, usize, usize, String)> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let text = line.to_string();
        let mut start = None;
        let mut word = String::new();
        let mut at = 0;
        for c in text.chars() {
            if c.is_whitespace() {
                if let Some(from) = start.take() {
                    words.push((index, from, at, std::mem::take(&mut word)));
                }
            } else {
                if start.is_none() {
                    start = Some(at);
                }
                word.push(c);
            }
            at += 1;
        }
        if let Some(from) = start {
            words.push((index, from, at, word));
        }
    }
    let Some(first) = words
        .windows(wanted.len())
        .position(|run| run.iter().zip(&wanted).all(|(word, want)| word.3 == *want))
    else {
        return Vec::new();
    };
    let run = &words[first..first + wanted.len()];
    let mut ranges: Vec<(usize, usize, usize)> = Vec::new();
    for (line, from, to, _) in run {
        match ranges.last_mut() {
            Some((last, _, end)) if last == line => *end = *to,
            _ => ranges.push((*line, *from, *to)),
        }
    }
    ranges
}

/// `line` with characters `from..to` drawn in `style` on top of what they
/// were.
pub(super) fn restyle(line: &Line<'static>, from: usize, to: usize, style: Style) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut at = 0;
    for span in &line.spans {
        let mut before = String::new();
        let mut inside = String::new();
        let mut after = String::new();
        for c in span.content.chars() {
            let target = if at < from {
                &mut before
            } else if at < to {
                &mut inside
            } else {
                &mut after
            };
            target.push(c);
            at += 1;
        }
        for (text, marked) in [(before, false), (inside, true), (after, false)] {
            if !text.is_empty() {
                spans.push(Span::styled(
                    text,
                    if marked {
                        span.style.patch(style)
                    } else {
                        span.style
                    },
                ));
            }
        }
    }
    Line::from(spans).style(line.style)
}

/// The action card's rows: the chip, the date and the quote in the accent,
/// and on the right what answers it, on the surface background.
fn card(
    rows: &mut Rows,
    row: &Row,
    now: DateTime<Local>,
    hint: &dyn Fn(CommandId) -> Option<String>,
    capture: bool,
) {
    let Some(marker) = &row.marker else {
        return;
    };
    let theme = rows.theme;
    let line = postio_ui::focus_row::marker_line(marker, now.with_timezone(&chrono::Utc), &Local)
        .capturing(capture);
    let accent = theme.style(Role::Accent);
    let surface = theme.style(Role::Surface);
    let width = rows.width;

    let mut left: Vec<Span<'static>> = vec![Span::styled(
        line.chip.to_owned(),
        accent.add_modifier(Modifier::BOLD),
    )];
    if let Some(date) = &line.date {
        left.push(Span::styled(format!("  {date}"), accent));
    }
    if let Some(quote) = &line.quote {
        left.push(Span::styled(
            format!("  “{}”", SafeText::new(quote).as_str()),
            accent.add_modifier(Modifier::ITALIC),
        ));
    }
    // What answers it, then Dismiss, each its words and its key.
    let mut answers: Vec<(String, Option<&'static str>)> = Vec::new();
    match line.status {
        Some(status) => answers.push((status.to_owned(), None)),
        None => {
            for (command, word) in &line.actions {
                if let Some(key) = hint(*command) {
                    answers.push((format!("{word} {}", cap(&key)), Some(command.as_str())));
                }
            }
        }
    }
    if let Some(key) = hint(CommandId::DismissMarker) {
        answers.push((
            format!("Dismiss {}", cap(&key)),
            Some(CommandId::DismissMarker.as_str()),
        ));
    }
    let answers_width: usize = answers.iter().map(|(text, _)| text.width()).sum::<usize>()
        + 2 * answers.len().saturating_sub(1);

    let pad = |spans: &mut Vec<Span<'static>>, to: usize| {
        let used = width_of(spans);
        if used < to {
            spans.push(Span::raw(" ".repeat(to - used)));
        }
    };
    let left_width = width_of(&left);
    let make = |mut spans: Vec<Span<'static>>, answers: &[(String, Option<&'static str>)]| {
        let mut segments = Vec::new();
        let total: usize = answers.iter().map(|(text, _)| text.width()).sum::<usize>()
            + 2 * answers.len().saturating_sub(1);
        pad(&mut spans, width.saturating_sub(total));
        let mut x = width_of(&spans);
        for (index, (text, command)) in answers.iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw("  "));
                x += 2;
            }
            let w = text.width();
            spans.push(Span::styled(text.clone(), theme.style(Role::Dim)));
            if let Some(command) = command {
                segments.push(Segment {
                    start: u16::try_from(x).unwrap_or(0),
                    width: u16::try_from(w).unwrap_or(0),
                    command,
                });
            }
            x += w;
        }
        pad(&mut spans, width);
        (Line::from(spans).style(surface), segments)
    };
    if left_width + 2 + answers_width <= width {
        let (card, segments) = make(left, &answers);
        rows.push(card, At::Nothing, segments);
    } else {
        // Too wide for one row: the words on the first, what answers on the
        // second; the words give way to fit.
        let mut first = left;
        if left_width > width {
            let words: String = first.iter().map(|span| span.content.as_ref()).collect();
            first = vec![Span::styled(fit(&words, width), accent)];
        }
        pad(&mut first, width);
        rows.push(Line::from(first).style(surface), At::Nothing, Vec::new());
        let (second, segments) = make(Vec::new(), &answers);
        rows.push(second, At::Nothing, segments);
    }
}

/// The column of the open message at `width` columns, as of `now`.
pub fn document(app: &App, theme: &Theme, width: u16, now: DateTime<Local>) -> Document {
    let Some((row, member)) = shown(app) else {
        return Document::default();
    };
    let reading = app.reading().expect("shown has a reading");
    let hint = |command| app.hint(command);
    let mut rows = Rows {
        width: usize::from(width),
        theme,
        lines: Vec::new(),
    };
    let dim = theme.style(Role::Dim);
    let text = theme.style(Role::Text);
    let accent = theme.style(Role::Accent);

    // The thread line.
    if let Some(chip) = focus_dialog::thread_chip(row.map_or(1, |row| row.count)) {
        let mut spans = vec![Span::styled(chip, dim)];
        let mut segments = Vec::new();
        if let Some(key) = hint(CommandId::PrevInConversation) {
            spans.push(Span::raw("  "));
            let start = width_of(&spans);
            let key = cap(&key);
            spans.push(Span::styled(key.clone(), accent));
            spans.push(Span::styled(
                format!(" {}", focus_dialog::EARLIER_MESSAGE),
                dim,
            ));
            segments.push(Segment {
                start: u16::try_from(start).unwrap_or(0),
                width: u16::try_from(key.width() + 1 + focus_dialog::EARLIER_MESSAGE.width())
                    .unwrap_or(0),
                command: CommandId::PrevInConversation.as_str(),
            });
        }
        rows.push(Line::from(spans), At::Nothing, segments);
    }

    // The subject, in bold.
    let subject = match row {
        Some(row) if !row.subject.as_str().trim().is_empty() => row.subject.as_str(),
        _ => "(no subject)",
    };
    for line in wrap(
        &Line::styled(subject.to_owned(), text.add_modifier(Modifier::BOLD)),
        usize::from(width),
    )
    .into_iter()
    .take(3)
    {
        rows.plain(line);
    }

    // The pills, and the way to add one.
    let mut spans: Vec<Span<'static>> = Vec::new();
    for pill in row.iter().flat_map(|row| &row.labels) {
        spans.push(Span::styled("●", theme.label(pill.colour)));
        spans.push(Span::styled(format!("{}  ", pill.name.as_str()), dim));
    }
    let mut segments = Vec::new();
    let start = width_of(&spans);
    let add = match hint(CommandId::AddLabel) {
        Some(key) => format!("+ Label {}", cap(&key)),
        None => "+ Label".to_owned(),
    };
    segments.push(Segment {
        start: u16::try_from(start).unwrap_or(0),
        width: u16::try_from(add.width()).unwrap_or(0),
        command: CommandId::AddLabel.as_str(),
    });
    spans.push(Span::styled(add, dim));
    rows.push(Line::from(spans), At::Nothing, segments);
    rows.hairline();

    // From, To and Cc, the date on the right of From.
    let label = |name: &str| Span::styled(format!("{name:<6}"), dim);
    let date = postio_ui::focus_row::message_date(member.when, now);
    let from = match &member.address {
        Some(address) if address != member.from.as_str() && !member.from.as_str().is_empty() => {
            format!(
                "{} <{}>",
                member.from.as_str(),
                SafeText::new(address).as_str()
            )
        }
        Some(address) => SafeText::new(address).as_str().to_owned(),
        None => member.from.as_str().to_owned(),
    };
    let room = usize::from(width).saturating_sub(6 + date.width() + 2);
    let from = fit(&from, room);
    let gap = usize::from(width).saturating_sub(6 + from.width() + date.width());
    rows.plain(Line::from(vec![
        label("From"),
        Span::styled(from, text),
        Span::raw(" ".repeat(gap)),
        Span::styled(date, dim),
    ]));
    for (name, people) in [("To", &member.to), ("Cc", &member.cc)] {
        if people.is_empty() {
            continue;
        }
        rows.plain(Line::from(vec![
            label(name),
            Span::styled(
                fit(&names_line(people), usize::from(width).saturating_sub(6)),
                text,
            ),
        ]));
    }
    rows.hairline();

    // The action card, on the message the marker is about.
    let carded = row.is_some_and(|row| app.marker_shown(row, member));
    if let (true, Some(row)) = (carded, row) {
        card(&mut rows, row, now, &hint, app.captures());
        rows.blank();
    } else {
        rows.blank();
    }

    // The body.
    let notice = |text: String| Line::styled(text, dim);
    if member.images_allowed {
        rows.plain(notice(
            "Remote images allowed — a terminal draws none".to_owned(),
        ));
    } else if member.held_back.remote_images + member.held_back.trackers > 0 {
        rows.plain(notice(format!(
            "{} · i i to show",
            member.held_back.summary()
        )));
    }
    let mut closing_fold: Option<(Line<'static>, At)> = None;
    match &member.body {
        Some(body) => {
            let mut lines = body.lines();
            let mut targets = body.targets();
            // A closed fold at the very end stands under the attachments.
            if matches!(body.blocks.last(), Some(Block::Fold { folded: true, .. }))
                && let (Some(line), Some(target)) = (lines.pop(), targets.pop())
            {
                let at = match target {
                    LineTarget::Fold(block) => At::Fold {
                        member: reading.current,
                        block,
                    },
                    _ => At::Nothing,
                };
                closing_fold = Some((line, at));
            }
            // The marker's sentence, in its own words.
            if carded
                && let Some(excerpt) = row
                    .and_then(|row| row.marker.as_ref())
                    .and_then(|marker| marker.excerpt.as_deref())
            {
                let mark = surface_underline(theme);
                for (index, from, to) in locate(&lines, excerpt) {
                    lines[index] = restyle(&lines[index], from, to, mark);
                }
            }
            for (line, target) in lines.iter().zip(targets) {
                let at = match target {
                    LineTarget::Text => At::Nothing,
                    LineTarget::Fold(block) => At::Fold {
                        member: reading.current,
                        block,
                    },
                    LineTarget::Link(link) => At::Link(body.links[link].as_str().to_owned()),
                };
                for wrapped in wrap(line, usize::from(width)) {
                    rows.push(wrapped, at.clone(), Vec::new());
                }
            }
        }
        None => rows.plain(Line::raw("…")),
    }

    // The attachments, as chips that wrap.
    let parts = member.attachments();
    if !parts.is_empty() {
        rows.blank();
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut at_part: Vec<(usize, usize)> = Vec::new();
        let mut used = 0;
        for (index, part) in parts.iter().enumerate() {
            let name = SafeText::new(part.filename.as_deref().unwrap_or(&part.mime_type));
            let chip = format!(
                "⎘ {} {}",
                name.as_str(),
                postio_ui::format::human_size(part.size)
            );
            let chip = fit(&chip, usize::from(width));
            let w = chip.width();
            if used > 0 && used + 3 + w > usize::from(width) {
                rows.push(
                    Line::from(std::mem::take(&mut spans)),
                    At::Nothing,
                    part_segments(&at_part, reading.current),
                );
                at_part.clear();
                used = 0;
            }
            if used > 0 {
                spans.push(Span::raw("   "));
                used += 3;
            }
            at_part.push((index, used));
            spans.push(Span::styled(chip, text));
            used += w;
            at_part.push((index, used));
        }
        rows.push(
            Line::from(spans),
            At::Nothing,
            part_segments(&at_part, reading.current),
        );
    }
    if let Some((line, at)) = closing_fold {
        rows.push(line, at, Vec::new());
    }
    if let Some(find) = app.find() {
        let text: Vec<String> = rows.lines.iter().map(|row| row.line.to_string()).collect();
        let found = postio_ui::find::matches(&text, find.query());
        let on = find
            .current()
            .map(|at| at.min(found.len().saturating_sub(1)));
        for (index, at) in found.iter().enumerate() {
            let style = find_style(theme, on == Some(index));
            let row = &mut rows.lines[at.line];
            row.line = restyle(&row.line, at.from, at.to, style);
        }
    }
    Document { lines: rows.lines }
}

/// A find match: the accent reversed, so it reads without colour; the one the
/// field is on besides bold and underlined.
pub(super) fn find_style(theme: &Theme, current: bool) -> Style {
    let style = theme.style(Role::Accent).add_modifier(Modifier::REVERSED);
    if current {
        style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else {
        style
    }
}

/// A part chip's click regions are drawn as parts of the row: every chip
/// on it is the part itself.
fn part_segments(_edges: &[(usize, usize)], _member: usize) -> Vec<Segment> {
    Vec::new()
}

/// The marker's sentence in the body: on the surface, with an accent
/// underline.
pub(super) fn surface_underline(theme: &Theme) -> Style {
    theme
        .style(Role::Surface)
        .patch(theme.style(Role::Accent))
        .add_modifier(Modifier::UNDERLINED)
}

/// A header's verb: its words and its key, and the command a click runs.
fn verb(app: &App, theme: &Theme, command: CommandId, label: &str) -> (Vec<Span<'static>>, usize) {
    let mut spans = vec![Span::styled(label.to_owned(), theme.style(Role::Text))];
    if let Some(key) = app.hint(command) {
        spans.push(Span::styled(
            format!(" {}", cap(&key)),
            theme.style(Role::Dim),
        ));
    }
    let width = width_of(&spans);
    (spans, width)
}

/// The verbs on the action row, in order: the received message's, or the
/// ones that settle a draft's send.
fn action_verbs(app: &App, folded: bool) -> Vec<focus_dialog::Verb> {
    let state = app.row_of_open().and_then(|row| row.send_state);
    match focus_dialog::send_verbs(state) {
        Some(allowed) => focus_dialog::SEND_TOOLBAR
            .iter()
            .filter(|item| allowed.contains(&item.command))
            .copied()
            .collect(),
        None => focus_dialog::OPEN_TOOLBAR
            .iter()
            .filter(|item| {
                if folded {
                    !focus_dialog::FOLDED.contains(&item.command)
                } else {
                    item.command != CommandId::MoreActions
                }
            })
            .copied()
            .collect(),
    }
}

/// Whether the action row folds Label, Move and Delete into More at this
/// width: a narrow frame, or a row that would not fit.
pub fn folded(app: &App, width: u16, inside: u16) -> bool {
    if app
        .row_of_open()
        .is_some_and(|row| focus_dialog::send_verbs(row.send_state).is_some())
    {
        return false;
    }
    if crate::layout::folds_into_more(width) {
        return true;
    }
    let theme = crate::theme::Theme::plain();
    let all: Vec<focus_dialog::Verb> = focus_dialog::OPEN_TOOLBAR
        .iter()
        .filter(|item| item.command != CommandId::MoreActions)
        .copied()
        .collect();
    let used: usize = all
        .iter()
        .map(|item| verb(app, &theme, item.command, item.label).1)
        .sum::<usize>()
        + 2 * all.len().saturating_sub(1);
    used + 2 > usize::from(inside)
}

/// The open message as an overlay: the list behind it drawn dimmed, and over
/// it a rounded frame covering every row but the top bar and the bottom
/// line.
pub fn over_list(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    theme: &Theme,
    now: DateTime<Local>,
    hits: &mut Hits,
) {
    let Some((outer, inside)) = framed(frame, area, theme, hits) else {
        return;
    };
    draw(frame, inside, outer.width, app, theme, now, hits);
    if let Some(menu) = app.menu() {
        draw_menu(frame, inside, menu, theme, hits);
    }
}

/// A rounded frame covering every row of `area` but the first and the last,
/// the rows behind it dimmed and taking no click, its header's hairline
/// meeting the border. Answers the frame and what is inside it.
pub fn framed(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    hits: &mut Hits,
) -> Option<(Rect, Rect)> {
    use ratatui::widgets::{Block, BorderType, Borders, Clear};
    let outer = crate::layout::open_frame(area);
    if outer.width < 12 || outer.height < 8 {
        return None;
    }
    // Everything under the frame is dimmed and takes no click.
    let behind = Rect::new(
        area.x,
        area.y + 1,
        area.width,
        area.height.saturating_sub(2),
    );
    frame
        .buffer_mut()
        .set_style(behind, Style::default().add_modifier(Modifier::DIM));
    hits.add(behind, Target::Overlay);
    frame.render_widget(Clear, outer);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.style(Role::Dim));
    let inside = block.inner(outer);
    frame.render_widget(block, outer);
    hits.add(outer, Target::Overlay);
    let tee = outer.y + 4;
    let buffer = frame.buffer_mut();
    buffer[(outer.x, tee)].set_symbol("├");
    buffer[(outer.x + outer.width - 1, tee)].set_symbol("┤");
    Some((outer, inside))
}

/// A small framed list in the middle of `area`.
pub(super) fn draw_menu(
    frame: &mut Frame,
    area: Rect,
    menu: &crate::app::Menu,
    theme: &Theme,
    hits: &mut Hits,
) {
    use ratatui::widgets::{Block, BorderType, Borders, Clear};
    let words = menu
        .items
        .iter()
        .map(|item| item.words.width() + 2 + item.detail.width())
        .max()
        .unwrap_or(10)
        .max(menu.title.width() + 4);
    let width = u16::try_from((words + 6).min(usize::from(area.width).saturating_sub(4)))
        .unwrap_or(area.width)
        .max(20);
    let height = u16::try_from(menu.items.len() + 2)
        .unwrap_or(area.height)
        .min(area.height.saturating_sub(2));
    let rect = Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 3,
        width.min(area.width),
        height,
    );
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(format!(" {} ", menu.title))
        .border_style(theme.style(Role::Dim));
    let inside = block.inner(rect);
    frame.render_widget(block, rect);
    hits.add(rect, Target::Overlay);
    let room = usize::from(inside.width).saturating_sub(2);
    for (index, item) in menu
        .items
        .iter()
        .take(usize::from(inside.height))
        .enumerate()
    {
        let y = inside.y + u16::try_from(index).unwrap_or(0);
        let here = index == menu.at;
        let detail = fit(&item.detail, room / 2);
        let words = fit(&item.words, room.saturating_sub(detail.width() + 2));
        let gap = room.saturating_sub(words.width() + detail.width());
        let line = Line::from(vec![
            Span::styled(if here { "▌" } else { " " }, theme.style(Role::Focus)),
            Span::styled(
                words,
                if here {
                    theme.style(Role::Text).add_modifier(Modifier::BOLD)
                } else {
                    theme.style(Role::Text)
                },
            ),
            Span::raw(" ".repeat(gap)),
            Span::styled(detail, theme.style(Role::Dim)),
        ]);
        let row = Rect::new(inside.x, y, inside.width, 1);
        frame.render_widget(line, row);
        hits.add(row, Target::MenuRow(index));
    }
}

/// Draw the open message into `area`: three header rows, a hairline, and the
/// column below, scrolled to `top`. `framed` is whether `area` is the inside
/// of a frame, whose border draws the hairline's tees.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    outer: u16,
    app: &App,
    theme: &Theme,
    now: DateTime<Local>,
    hits: &mut Hits,
) {
    if area.width < 10 || area.height < 5 {
        return;
    }
    let Some(reading) = app.reading() else {
        return;
    };
    let row = app.row_of_open();
    let dim = theme.style(Role::Dim);
    let text = theme.style(Role::Text);
    let accent = theme.style(Role::Accent);
    let inner = usize::from(area.width);
    let at = |y: u16| Rect::new(area.x, area.y + y, area.width, 1);

    // Row 1: the step keys, the subject, and Esc ✕.
    let mut left: Vec<Span<'static>> = Vec::new();
    let mut targets: Vec<(usize, usize, &'static str)> = Vec::new();
    for (arrow, command) in [("↑", CommandId::PrevMessage), ("↓", CommandId::NextMessage)] {
        if !left.is_empty() {
            left.push(Span::raw("  "));
        }
        let start = 1 + width_of(&left);
        left.push(Span::styled(format!("{arrow} "), dim));
        if let Some(key) = app.hint(command) {
            left.push(Span::styled(cap(&key), accent));
        }
        targets.push((start, 1 + width_of(&left), command.as_str()));
    }
    let mut right: Vec<Span<'static>> = Vec::new();
    if let Some(key) = app.hint(CommandId::Back) {
        right.push(Span::styled(format!("{} ", cap(&key)), dim));
    }
    right.push(Span::styled("✕", text));
    let right_width = width_of(&right);
    let title = if app.raw().is_some() {
        "Raw source".to_owned()
    } else {
        match row {
            Some(row) if !row.subject.as_str().trim().is_empty() => row.subject.as_str().to_owned(),
            _ => "(no subject)".to_owned(),
        }
    };
    let left_width = width_of(&left);
    let room = inner.saturating_sub(2 * (left_width.max(right_width) + 3));
    let title = fit(&title, room.max(8));
    let centred = inner.saturating_sub(title.width()) / 2;
    frame.render_widget(
        Line::from(left.clone()),
        Rect::new(
            area.x + 1,
            area.y,
            u16::try_from(left_width).unwrap_or(0),
            1,
        ),
    );
    frame.render_widget(
        Line::styled(title, text.add_modifier(Modifier::BOLD)),
        Rect::new(
            area.x + u16::try_from(centred).unwrap_or(0),
            area.y,
            area.width - u16::try_from(centred).unwrap_or(0),
            1,
        ),
    );
    let rx = area.x
        + area
            .width
            .saturating_sub(u16::try_from(right_width + 1).unwrap_or(0));
    frame.render_widget(
        Line::from(right),
        Rect::new(rx, area.y, u16::try_from(right_width).unwrap_or(0), 1),
    );
    for (from, to, command) in targets {
        hits.add(
            Rect::new(
                area.x + u16::try_from(from).unwrap_or(0),
                area.y,
                u16::try_from(to - from).unwrap_or(0),
                1,
            ),
            Target::Command(command),
        );
    }
    hits.add(
        Rect::new(rx, area.y, u16::try_from(right_width).unwrap_or(0), 1),
        Target::Command(CommandId::Back.as_str()),
    );

    // Row 2: where in the list and the thread.
    let position = app.position_line();
    let position = fit(&position, inner.saturating_sub(2));
    let at_x = inner.saturating_sub(position.width()) / 2;
    frame.render_widget(
        Line::styled(position, dim),
        Rect::new(
            area.x + u16::try_from(at_x).unwrap_or(0),
            area.y + 1,
            area.width - u16::try_from(at_x).unwrap_or(0),
            1,
        ),
    );

    // Row 3: the action row.
    if app.raw().is_none() {
        let fold = folded(app, outer, area.width);
        let mut x = area.x + 1;
        let end = area.x + area.width;
        for item in action_verbs(app, fold) {
            let (spans, w) = verb(app, theme, item.command, item.label);
            let w16 = u16::try_from(w).unwrap_or(u16::MAX);
            if x + w16 > end {
                break;
            }
            // The one primary verb of a draft's action row is bold.
            frame.render_widget(Line::from(spans), Rect::new(x, area.y + 2, w16, 1));
            hits.add(
                Rect::new(x, area.y + 2, w16, 1),
                Target::Command(item.command.as_str()),
            );
            x += w16 + 2;
        }
    }

    // The hairline, then the column.
    frame.render_widget(Line::styled("─".repeat(inner), dim), at(3));
    let find_height = app.find_rows().min(area.height.saturating_sub(5));
    let body = Rect::new(
        area.x,
        area.y + 4,
        area.width,
        area.height - 4 - find_height,
    );
    if find_height > 0 {
        let field = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
        draw_find(frame, field, app, theme, hits);
    }
    if let Some(raw) = app.raw() {
        draw_raw(frame, body, raw, app, theme, hits);
        return;
    }
    let column = crate::layout::column_width(outer).min(area.width);
    let x = area.x + (area.width - column) / 2;
    let doc = document(app, theme, column, now);
    let top = app.reader_top().min(doc.lines.len().saturating_sub(1));
    for (offset, line) in doc
        .lines
        .iter()
        .skip(top)
        .take(usize::from(body.height))
        .enumerate()
    {
        let y = body.y + u16::try_from(offset).unwrap_or(0);
        let rect = Rect::new(x, y, column, 1);
        frame.render_widget(line.line.clone(), rect);
        hits.add(rect, Target::Reader(Some(top + offset)));
        for segment in &line.segments {
            hits.add(
                Rect::new(x + segment.start, y, segment.width, 1),
                Target::Command(segment.command),
            );
        }
    }
    let _ = reading;
}

/// The find field, on the last row inside the frame: what is typed, how
/// many matches it has and which it is on, and the keys that step and close,
/// each a click.
fn draw_find(frame: &mut Frame, row: Rect, app: &App, theme: &Theme, hits: &mut Hits) {
    let Some(find) = app.find() else {
        return;
    };
    let dim = theme.style(Role::Dim);
    let text = theme.style(Role::Text);
    let accent = theme.style(Role::Accent);
    let total = app.find_total();
    let says = find.says(total);
    // From the right: the words and keys that act, then the count.
    let mut hints: Vec<(String, &'static str)> = Vec::new();
    for (command, word) in [
        (CommandId::FindNext, "next"),
        (CommandId::FindPrevious, "previous"),
        (CommandId::Back, "close"),
    ] {
        if let Some(key) = app.hint(command) {
            hints.push((format!("{} {word}", cap(&key)), command.as_str()));
        }
    }
    let inner = usize::from(row.width);
    let query_room = inner.saturating_sub(4 + says.width() + 2);
    // Keys give way first when the row is narrow, the last first.
    let mut keep = hints.len();
    let used = |keep: usize| {
        hints[..keep]
            .iter()
            .map(|(h, _)| h.width() + 2)
            .sum::<usize>()
    };
    while keep > 0 && used(keep) + says.width() + 12 > inner {
        keep -= 1;
    }
    let _ = query_room;
    let mut right: Vec<Span<'static>> = Vec::new();
    let mut targets: Vec<(usize, usize, &'static str)> = Vec::new();
    let mut at = 0;
    if !says.is_empty() {
        right.push(Span::styled(says.clone(), text));
        at += says.width();
    }
    for (hint, command) in &hints[..keep] {
        right.push(Span::raw("  "));
        at += 2;
        right.push(Span::styled(hint.clone(), dim));
        targets.push((at, at + hint.width(), command));
        at += hint.width();
    }
    let right_width = at;
    let query = find.query();
    let typed = fit(query, inner.saturating_sub(right_width + 5).max(1));
    let mut left = vec![Span::styled(" ⌕ ", accent)];
    if query.is_empty() {
        left.push(Span::styled("Find in message", dim));
    } else {
        left.push(Span::styled(typed.clone(), text));
    }
    frame.render_widget(Line::from(left), Rect::new(row.x, row.y, row.width, 1));
    let before: String = query.chars().take(find.caret()).collect();
    frame.set_cursor_position(ratatui::layout::Position::new(
        (row.x + 3 + u16::try_from(before.width()).unwrap_or(0)).min(row.x + row.width - 1),
        row.y,
    ));
    let rx = row.x
        + row
            .width
            .saturating_sub(u16::try_from(right_width + 1).unwrap_or(0));
    frame.render_widget(
        Line::from(right),
        Rect::new(rx, row.y, u16::try_from(right_width).unwrap_or(0), 1),
    );
    for (from, to, command) in targets {
        hits.add(
            Rect::new(
                rx + u16::try_from(from).unwrap_or(0),
                row.y,
                u16::try_from(to - from).unwrap_or(0),
                1,
            ),
            Target::Command(command),
        );
    }
}

/// The message as it came off the wire, wrapped to the frame.
fn draw_raw(
    frame: &mut Frame,
    area: Rect,
    raw: &crate::app::Raw,
    app: &App,
    theme: &Theme,
    hits: &mut Hits,
) {
    let top = app.reader_top();
    let width = usize::from(area.width).saturating_sub(2).max(1);
    let Some(text) = &raw.text else {
        frame.render_widget(
            Line::styled(" Reading the source…", theme.style(Role::Dim)),
            Rect::new(area.x, area.y, area.width, 1),
        );
        return;
    };
    let lines = raw_lines(text, width);
    let found = app
        .find()
        .map(|find| (find, postio_ui::find::matches(&lines, find.query())));
    let first = top.min(lines.len().saturating_sub(1));
    for (offset, line) in lines
        .iter()
        .skip(first)
        .take(usize::from(area.height))
        .enumerate()
    {
        let y = area.y + u16::try_from(offset).unwrap_or(0);
        let rect = Rect::new(area.x + 1, y, area.width - 1, 1);
        let mut drawn = Line::styled(line.clone(), theme.style(Role::Text));
        if let Some((find, found)) = &found {
            let on = find
                .current()
                .map(|at| at.min(found.len().saturating_sub(1)));
            for (index, at) in found.iter().enumerate() {
                if at.line == first + offset {
                    drawn = restyle(&drawn, at.from, at.to, find_style(theme, on == Some(index)));
                }
            }
        }
        frame.render_widget(drawn, rect);
        hits.add(Rect::new(area.x, y, area.width, 1), Target::Reader(None));
    }
}

/// The raw source's rows at `width` columns: each line of it broken at the
/// width, so nothing runs past the frame.
pub fn raw_lines(text: &SafeText, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    for line in text.as_str().lines() {
        let mut row = String::new();
        let mut used = 0;
        for c in line.chars() {
            let w = c.width().unwrap_or(0);
            if used + w > width {
                rows.push(std::mem::take(&mut row));
                used = 0;
            }
            row.push(c);
            used += w;
        }
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use postio_model::listing::{MarkerKind, MarkerSummary, MarkerWhen};
    use postio_model::{EmailAddress, MessageId, ThreadId};
    use postio_ui::focus_list::{Conversation, FocusRow};

    use crate::app::{App, Effect, Input, Pointer, update};
    use crate::test_support::{
        app, conversation, hits_of, key, label, local, places, press, screen, seed_places,
        show_focus, unread,
    };
    use crossterm::event::{KeyCode, KeyModifiers};

    const QUOTE: &str = "Please leave comments by Wednesday";

    /// The inbox of terminal.md's open message: a conversation of six, the
    /// newest asking for comments, with a label, a marker and two files.
    fn inbox(size: (u16, u16)) -> App {
        let mut app = app(size);
        seed_places(&mut app, places());
        let mut harbor = unread(conversation(
            6,
            "Lena Park",
            "Harbor API draft v3",
            "Uploaded v3",
            local(23, 11, 22),
        ));
        harbor.id = Some(ThreadId::new(60));
        harbor.representative.thread = Some(ThreadId::new(60));
        harbor.message_count = 6;
        harbor.marker = Some(MarkerSummary {
            kind: MarkerKind::Todo,
            when: Some(MarkerWhen::Due(local(30, 9, 0))),
            excerpt: Some(QUOTE.into()),
            answer: None,
            cancelled: false,
        });
        let mut rows = vec![FocusRow::Conversation(Conversation {
            summary: harbor,
            labels: vec![label(1, "Harbor")],
        })];
        for (id, who) in [(7, "Ben Adeyemi"), (8, "Grace Oyelaran")] {
            rows.push(FocusRow::conversation(unread(conversation(
                id,
                who,
                &format!("Later {id}"),
                "",
                local(23, 9, id as u32),
            ))));
        }
        show_focus(&mut app, rows);
        app
    }

    fn body(text: &str) -> Input {
        Input::Body {
            message: MessageId::new(6),
            answer: Ok(postio_client::protocol::Body::Ready {
                body: postio_model::MessageBody {
                    text: Some(text.into()),
                    html: None,
                },
                encoding_problems: false,
            }),
        }
    }

    /// The first message opened, its thread of six read, its body here.
    fn opened(size: (u16, u16)) -> App {
        let mut app = inbox(size);
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let members = (1..=6)
            .map(|id| {
                conversation(
                    id,
                    "Lena Park",
                    "Harbor API draft v3",
                    "",
                    local(23, 11, 22),
                )
                .representative
            })
            .collect();
        update(
            &mut app,
            Input::Conversation {
                thread: ThreadId::new(60),
                members: Ok(members),
            },
        );
        update(
            &mut app,
            Input::Addressed {
                message: MessageId::new(6),
                to: vec![
                    EmailAddress::new(None::<String>, "you@example.com"),
                    EmailAddress::new(Some("Ben Adeyemi"), "ben@example.com"),
                ],
                cc: vec![EmailAddress::new(
                    Some("Grace Oyelaran"),
                    "grace@example.com",
                )],
            },
        );
        update(
            &mut app,
            body(&format!(
                "Hi all,\nUploaded v3 of the Harbor API draft with the pagination changes.\n{QUOTE}\n\n> earlier\n> words"
            )),
        );
        app
    }

    fn lines_of(drawn: &str) -> Vec<&str> {
        drawn.lines().collect()
    }

    #[test]
    fn the_open_message_is_a_rounded_frame_under_the_top_bar_and_over_the_bottom_line() {
        let app = opened((120, 36));
        let drawn = screen(120, 36, &app);
        let lines = lines_of(&drawn);
        assert!(lines[0].contains("Compose"), "the top bar stays:\n{drawn}");
        let frame: Vec<char> = lines[1].chars().collect();
        assert_eq!(
            (frame[14], frame[105]),
            ('╭', '╮'),
            "92 wide at 120:\n{drawn}"
        );
        let last: Vec<char> = lines[34].chars().collect();
        assert_eq!((last[14], last[105]), ('╰', '╯'), "{drawn}");
        assert!(
            lines[35].trim().is_empty() || !lines[35].contains('│'),
            "{drawn}"
        );
        let tee: Vec<char> = lines[5].chars().collect();
        assert_eq!(
            (tee[14], tee[105]),
            ('├', '┤'),
            "the header's hairline:\n{drawn}"
        );
    }

    #[test]
    fn the_header_is_the_step_keys_the_subject_and_close_then_the_position_then_the_verbs() {
        let app = opened((120, 36));
        let drawn = screen(120, 36, &app);
        let lines = lines_of(&drawn);
        let step = lines[2];
        assert!(step.contains("↑ k  ↓ j"), "{drawn}");
        assert!(step.contains("Harbor API draft v3"), "{drawn}");
        assert!(step.contains("Esc ✕"), "{drawn}");
        assert!(lines[3].contains("Message 1 of 3 · thread of 6"), "{drawn}");
        let verbs = lines[4];
        for wanted in [
            "Reply e",
            "Reply all E",
            "Forward f",
            "Archive a",
            "Snooze s",
            "Remind h",
            "More .",
        ] {
            assert!(verbs.contains(wanted), "{wanted}:\n{drawn}");
        }
        assert!(
            !verbs.contains("Label") && !verbs.contains("Delete"),
            "92 columns of frame fold Label, Move and Delete into More:\n{drawn}"
        );
    }

    #[test]
    fn a_wide_frame_keeps_every_verb_and_has_no_more() {
        let app = opened((160, 36));
        let drawn = screen(160, 36, &app);
        let verbs = lines_of(&drawn)[4];
        for wanted in ["Label l", "Move m", "Delete Del"] {
            assert!(verbs.contains(wanted), "{wanted}:\n{drawn}");
        }
        assert!(!verbs.contains("More"), "{drawn}");
    }

    #[test]
    fn the_column_is_the_thread_line_subject_labels_the_sender_grid_the_card_and_the_body() {
        let app = opened((120, 36));
        let drawn = screen(120, 36, &app);
        let text = reader(&drawn);
        let at = |needle: &str| {
            text.iter()
                .position(|line| line.contains(needle))
                .unwrap_or_else(|| panic!("{needle} missing:\n{drawn}"))
        };
        let thread = at("Latest of 6 in this thread");
        assert!(text[thread].contains("[ earlier message"), "{drawn}");
        let subject = text
            .iter()
            .enumerate()
            .skip(thread)
            .find(|(_, line)| line.contains("Harbor API draft v3"))
            .map(|(at, _)| at)
            .unwrap();
        assert_eq!(subject, thread + 1, "{drawn}");
        assert_eq!(at("●Harbor"), subject + 1, "{drawn}");
        assert!(text[subject + 1].contains("+ Label l"), "{drawn}");
        assert!(text[subject + 2].contains("────"), "a hairline:\n{drawn}");
        let from = at("From");
        assert_eq!(from, subject + 3, "{drawn}");
        assert!(
            text[from].contains("Lena Park <lena@example.com>"),
            "{drawn}"
        );
        assert!(
            text[from].trim_end().ends_with("Wed, 23 Sep 2026 at 11:22")
                || text[from].trim_end().ends_with("Today, 11:22"),
            "{drawn}"
        );
        assert!(text[from + 1].contains("To"), "{drawn}");
        assert!(
            text[from + 1].contains("you@example.com, Ben Adeyemi"),
            "{drawn}"
        );
        assert!(
            text[from + 2].contains("Cc") && text[from + 2].contains("Grace Oyelaran"),
            "{drawn}"
        );
        assert!(text[from + 3].contains("────"), "{drawn}");
        let card = at("To-do");
        assert_eq!(card, from + 4, "{drawn}");
        assert!(text[card].contains("Wed 30 Sep"), "{drawn}");
        assert!(text[card].contains(&format!("“{QUOTE}”")), "{drawn}");
        // The words fill the first row, so what answers is on a second.
        assert!(
            text[card + 1].trim_end().ends_with("Snooze s  Dismiss -"),
            "{drawn}"
        );
        assert!(
            text[card + 2].trim().is_empty(),
            "a blank row after the card:\n{drawn}"
        );
        assert!(text[card + 3].contains("Hi all,"), "{drawn}");
        assert!(
            drawn.contains("▸ 2 quoted lines"),
            "the fold line:\n{drawn}"
        );
    }

    /// The column's text, one entry per frame row.
    fn reader(drawn: &str) -> Vec<String> {
        drawn
            .lines()
            .map(|line| {
                let chars: Vec<char> = line.chars().collect();
                chars
                    .get(15..105)
                    .map(|inside| inside.iter().collect::<String>())
                    .unwrap_or_default()
            })
            .collect()
    }

    #[test]
    fn the_quote_is_marked_in_the_body_on_the_surface_with_an_accent_underline() {
        use crate::caps::{Background, Colour};
        use ratatui::style::Modifier;
        let app = opened((120, 36));
        let theme =
            crate::theme::Theme::new(Colour::TrueColor, Background::Dark, &Default::default()).0;
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 36)).unwrap();
        terminal
            .draw(|frame| {
                crate::view::draw(frame, &app, &theme, crate::test_support::now());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let plain = screen(120, 36, &app);
        let (y, line) = lines_of(&plain)
            .into_iter()
            .enumerate()
            .rfind(|(_, line)| line.contains(QUOTE))
            .expect("the sentence is in the body");
        let x = u16::try_from(line[..line.find(QUOTE).unwrap()].chars().count()).unwrap();
        let cell = &buffer[(x + 3, u16::try_from(y).unwrap())];
        assert!(cell.modifier.contains(Modifier::UNDERLINED), "{cell:?}");
        assert_eq!(
            cell.bg,
            theme.style(crate::theme::Role::Surface).bg.unwrap(),
            "{cell:?}"
        );
        // Words around it are plain.
        let before = &buffer[(x.saturating_sub(2), u16::try_from(y).unwrap())];
        assert!(
            !before.modifier.contains(Modifier::UNDERLINED),
            "{before:?}"
        );
    }

    #[test]
    fn the_list_behind_is_dimmed() {
        use ratatui::style::Modifier;
        let app = opened((120, 36));
        let buffer = crate::test_support::buffer(120, 36, &app);
        // The strip and a list row under the frame's margin.
        assert!(buffer[(2, 1)].modifier.contains(Modifier::DIM), "the strip");
        assert!(buffer[(2, 4)].modifier.contains(Modifier::DIM), "the rows");
        assert!(
            !buffer[(2, 0)].modifier.contains(Modifier::DIM),
            "the top bar"
        );
    }

    #[test]
    fn only_the_body_shown_is_read_and_opening_reads_it_once() {
        let mut app = inbox((120, 36));
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::ReadConversation(ThreadId::new(60))),
            "{effects:?}"
        );
        assert!(
            !effects.iter().any(|e| matches!(e, Effect::ReadBody(_))),
            "{effects:?}"
        );
    }

    #[test]
    fn j_and_k_step_the_list_with_the_message_open_and_read_the_next_body() {
        let mut app = opened((120, 36));
        let effects = update(&mut app, press('j'));
        assert_eq!(app.cursor(), 1);
        assert!(
            effects.contains(&Effect::ReadBody(MessageId::new(7))),
            "{effects:?}"
        );
        let drawn = screen(120, 36, &app);
        assert!(lines_of(&drawn)[2].contains("Later 7"), "{drawn}");
        assert!(lines_of(&drawn)[3].contains("Message 2 of 3"), "{drawn}");
        update(&mut app, press('k'));
        assert_eq!(app.cursor(), 0);
        let effects = update(&mut app, press('k'));
        assert!(effects.is_empty(), "the first row stops it: {effects:?}");
    }

    #[test]
    fn brackets_step_the_thread_and_the_chip_shows_where() {
        let mut app = opened((120, 36));
        let effects = update(&mut app, press('['));
        assert!(
            effects.contains(&Effect::ReadBody(MessageId::new(5))),
            "{effects:?}"
        );
        let drawn = screen(120, 36, &app);
        assert!(drawn.contains("5 of 6 in the thread"), "{drawn}");
        update(&mut app, press(']'));
        assert!(screen(120, 36, &app).contains("thread of 6"));
    }

    #[test]
    fn escape_closes_back_to_the_same_row_with_the_selection_kept() {
        let mut app = opened((120, 36));
        // Select the second row, then come back to the first and open it.
        update(&mut app, press('j'));
        update(&mut app, press('x'));
        update(&mut app, press('k'));
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.front(), crate::app::Front::List);
        assert_eq!(app.cursor(), 0);
        assert!(app.selection().contains(MessageId::new(7)));
        let drawn = screen(120, 36, &app);
        assert!(!drawn.contains('╭'), "{drawn}");
    }

    #[test]
    fn the_message_counts_as_read_after_the_dwell_and_r_stops_the_clock() {
        let mut app = inbox((120, 36));
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let _ = effects;
        let members = (1..=6)
            .map(|id| conversation(id, "Lena Park", "s", "", local(23, 11, 22)).representative)
            .collect();
        let effects = update(
            &mut app,
            Input::Conversation {
                thread: ThreadId::new(60),
                members: Ok(members),
            },
        );
        let Some(Effect::ArmDwell {
            generation,
            message,
            after,
        }) = effects
            .iter()
            .find(|e| matches!(e, Effect::ArmDwell { .. }))
            .cloned()
        else {
            panic!("a clock for the message shown: {effects:?}");
        };
        assert_eq!(message, MessageId::new(6));
        assert_eq!(after, postio_ui::dwell::DWELL_TO_READ);
        let read = update(
            &mut app,
            Input::DwellDue {
                generation,
                message,
            },
        );
        assert!(
            read.contains(&Effect::Send(postio_core::Command::MarkReadOnDwell {
                message
            })),
            "{read:?}"
        );
        // r is the person's own say: the clock is stopped.
        update(&mut app, press('r'));
        let read = update(
            &mut app,
            Input::DwellDue {
                generation,
                message,
            },
        );
        assert!(read.is_empty(), "{read:?}");
    }

    #[test]
    fn stepping_to_another_message_starts_its_own_clock_and_the_old_one_does_nothing() {
        let mut app = opened((120, 36));
        let first = match update(&mut app, press('j')).iter().find_map(|e| match e {
            Effect::ArmDwell {
                generation,
                message,
                ..
            } => Some((*generation, *message)),
            _ => None,
        }) {
            Some(armed) => armed,
            None => panic!("armed"),
        };
        assert_eq!(first.1, MessageId::new(7));
        let stale = update(
            &mut app,
            Input::DwellDue {
                generation: first.0 - 1,
                message: MessageId::new(6),
            },
        );
        assert!(stale.is_empty(), "{stale:?}");
    }

    #[test]
    fn v_shows_the_raw_source_in_the_frame_and_escape_goes_back_to_the_message() {
        let mut app = opened((120, 36));
        let effects = update(&mut app, press('v'));
        assert!(
            effects.contains(&Effect::ReadSource(MessageId::new(6))),
            "{effects:?}"
        );
        update(
            &mut app,
            Input::Source {
                message: MessageId::new(6),
                raw: Ok(b"From: lena@example.com\r\nSubject: Harbor\x1b[2J\r\n\r\nbody".to_vec()),
            },
        );
        let drawn = screen(120, 36, &app);
        assert!(drawn.contains("Raw source"), "{drawn}");
        assert!(drawn.contains("From: lena@example.com"), "{drawn}");
        assert!(!drawn.contains('\u{1b}'), "made safe: {drawn:?}");
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(
            screen(120, 36, &app).contains("Hi all,"),
            "back to the message"
        );
        assert_eq!(app.front(), crate::app::Front::Reader);
    }

    #[test]
    fn o_lists_the_links_and_attachments_and_choosing_one_opens_it() {
        let mut app = inbox((120, 36));
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let nothing = update(&mut app, press('o'));
        assert!(app.menu().is_none(), "{nothing:?}");
        assert_eq!(app.notice(), Some(postio_ui::focus_target::NOTHING_TO_OPEN));
        update(
            &mut app,
            Input::Conversation {
                thread: ThreadId::new(60),
                members: Ok(vec![
                    conversation(6, "Lena Park", "s", "", local(23, 11, 22)).representative,
                ]),
            },
        );
        update(
            &mut app,
            Input::Body {
                message: MessageId::new(6),
                answer: Ok(postio_client::protocol::Body::Ready {
                    body: postio_model::MessageBody {
                        text: None,
                        html: Some(
                            "<p>See <a href=\"https://example.com/report\">the report</a></p>"
                                .into(),
                        ),
                    },
                    encoding_problems: false,
                }),
            },
        );
        update(&mut app, press('o'));
        let drawn = screen(120, 36, &app);
        assert!(drawn.contains("Open attachment or link"), "{drawn}");
        assert!(drawn.contains("https://example.com/report"), "{drawn}");
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::OpenLink("https://example.com/report".into())),
            "{effects:?}"
        );
        assert!(app.menu().is_none());
    }

    #[test]
    fn the_dot_opens_the_folded_verbs_and_a_choice_runs_its_command() {
        let mut app = opened((120, 36));
        let effects = update(&mut app, press('.'));
        assert!(app.menu().is_some(), "{effects:?} {:?}", app.notice());
        let drawn = screen(120, 36, &app);
        for wanted in ["Label", "Move", "Delete"] {
            assert!(drawn.contains(wanted), "{wanted}:\n{drawn}");
        }
        update(&mut app, press('j'));
        update(&mut app, press('j'));
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Send(postio_core::Command::Delete { .. }))),
            "{effects:?}"
        );
        // Wide enough to have no folded verbs: nothing opens.
        let mut wide = opened((160, 36));
        update(&mut wide, press('.'));
        assert!(wide.menu().is_none());
    }

    fn click_at(app: &mut App, size: (u16, u16), x: u16, y: u16) -> Vec<Effect> {
        let hit = hits_of(size.0, size.1, app).at(x, y).expect("a target");
        update(
            app,
            Input::Pointer(Pointer::Click {
                hit,
                ctrl: false,
                shift: false,
            }),
        )
    }

    fn find(app: &App, size: (u16, u16), word: &str, from_row: usize) -> (u16, u16) {
        let drawn = screen(size.0, size.1, app);
        let (y, line) = drawn
            .lines()
            .enumerate()
            .skip(from_row)
            .find(|(_, line)| line.contains(word))
            .unwrap_or_else(|| panic!("no {word}:\n{drawn}"));
        let x = line[..line.find(word).unwrap()].chars().count();
        (u16::try_from(x).unwrap(), u16::try_from(y).unwrap())
    }

    #[test]
    fn every_header_control_and_the_card_is_a_click_for_its_key() {
        let size = (120, 36);
        let mut app = opened(size);
        // An action row verb.
        let (x, y) = find(&app, size, "Archive a", 2);
        let effects = click_at(&mut app, size, x, y);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Send(postio_core::Command::Archive { .. }))),
            "{effects:?}"
        );
        // The card's Dismiss.
        let (x, y) = find(&app, size, "Dismiss -", 6);
        let effects = click_at(&mut app, size, x, y);
        assert!(
            effects.iter().any(|e| matches!(
                e,
                Effect::Send(postio_core::Command::DismissMarker {
                    dismissed: true,
                    ..
                })
            )),
            "{effects:?}"
        );
        assert!(
            !screen(size.0, size.1, &app).contains("To-do"),
            "the card goes"
        );
        // The thread chip steps back.
        let (x, y) = find(&app, size, "[ earlier message", 6);
        click_at(&mut app, size, x, y);
        assert_eq!(app.reading().unwrap().current, 4);
        // The down step key opens the next row.
        let (x, y) = find(&app, size, "↓ j", 2);
        click_at(&mut app, size, x, y);
        assert_eq!(app.cursor(), 1);
        // And the close.
        let (x, y) = find(&app, size, "Esc ✕", 2);
        click_at(&mut app, size, x, y);
        assert_eq!(app.front(), crate::app::Front::List);
    }

    #[test]
    fn a_click_outside_the_frame_does_nothing_and_the_wheel_scrolls_the_column() {
        let size = (120, 36);
        let mut app = opened(size);
        let effects = click_at(&mut app, size, 3, 10);
        assert!(effects.is_empty(), "{effects:?}");
        assert_eq!(app.front(), crate::app::Front::Reader);
        let mut app = inbox(size);
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        update(
            &mut app,
            Input::Conversation {
                thread: ThreadId::new(60),
                members: Ok(vec![
                    conversation(6, "Lena Park", "s", "", local(23, 11, 22)).representative,
                ]),
            },
        );
        let long: Vec<String> = (0..80).map(|n| format!("paragraph {n}")).collect();
        update(&mut app, body(&long.join("\n")));
        let hit = hits_of(size.0, size.1, &app).at(60, 20).unwrap();
        update(&mut app, Input::Pointer(Pointer::Wheel { hit, down: true }));
        assert!(app.reader_top() > 0);
        let drawn = screen(size.0, size.1, &app);
        assert!(
            !drawn.contains("Latest of"),
            "scrolled past the head:\n{drawn}"
        );
    }

    #[test]
    fn a_draft_on_its_way_offers_the_verbs_that_settle_it_and_not_the_received_ones() {
        let mut app = app((120, 36));
        seed_places(&mut app, places());
        let mut queued = unread(conversation(1, "Ada", "Queued note", "", local(23, 9, 0)));
        queued.representative.send_state = Some(postio_model::DraftState::Queued);
        show_focus(&mut app, vec![FocusRow::conversation(queued)]);
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let drawn = screen(120, 36, &app);
        let verbs = lines_of(&drawn)[4];
        assert!(
            verbs.contains("Cancel send") && verbs.contains("Edit"),
            "{drawn}"
        );
        assert!(!verbs.contains("Reply"), "{drawn}");
        assert!(
            lines_of(&drawn)[3].contains("Queued")
                || lines_of(&drawn)[3].contains("Message 1 of 1"),
            "{drawn}"
        );
    }

    #[test]
    fn hostile_text_in_the_header_and_body_reaches_the_screen_harmless() {
        let mut app = app((120, 36));
        seed_places(&mut app, places());
        let mut evil = unread(conversation(
            1,
            "Mallory\u{1b}[31m",
            "Pay\u{1b}]0;owned\u{7}",
            "",
            local(23, 9, 0),
        ));
        evil.representative.from = Some(EmailAddress::new(
            Some("Mallory\u{1b}[31m"),
            "m\u{1b}@example.com",
        ));
        show_focus(&mut app, vec![FocusRow::conversation(evil)]);
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        update(
            &mut app,
            Input::Body {
                message: MessageId::new(1),
                answer: Ok(postio_client::protocol::Body::Ready {
                    body: postio_model::MessageBody {
                        text: Some("pwn\u{1b}[2J\u{9b}".into()),
                        html: None,
                    },
                    encoding_problems: false,
                }),
            },
        );
        let drawn = screen(120, 36, &app);
        assert!(
            !drawn.contains('\u{1b}') && !drawn.contains('\u{9b}'),
            "{drawn:?}"
        );
    }
}
