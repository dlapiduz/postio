//! The capture sheet (terminal.md, "Obsidian capture"): a frame 72 wide
//! with the Task and Note pair, the text, due quick picks, the project with
//! its reason, and the exact markdown line as the preview.
//!
//! The chosen of a pair and the pick of the day are reversed, so they show
//! without a colour; every control is a click.

use postio_core::CommandId;
use postio_ui::capture::{self, Mode};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::capture::Field;
use crate::surface::Part;
use crate::theme::{Role, Theme};
use crate::view::bottom::cap;
use crate::view::boxed::{self, Button};
use crate::view::fit;
use crate::view::hit::{Hits, Target};
use crate::view::wrap::wrap;

/// How wide the frame is.
pub const WIDTH: u16 = 72;

/// The most projects the chooser lists at once.
const LISTED: usize = 6;

/// Draw the sheet over `area`, when one is open.
pub fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, hits: &mut Hits) {
    let Some(sheet) = app.capture() else {
        return;
    };
    let (dim, text, accent) = (
        theme.style(Role::Dim),
        theme.style(Role::Text),
        theme.style(Role::Accent),
    );
    let width = WIDTH.min(area.width);
    let room = usize::from(width).saturating_sub(4);
    let preview = wrap(&Line::styled(sheet.preview(), text), room.max(1));
    let place = sheet.place();
    let footnote = wrap(
        &Line::styled(capture::footnote(sheet.mode(), &place, sheet.due()), dim),
        room.max(1),
    );
    let choices = sheet.choices();
    let listed = choices.len().min(LISTED);
    let project_rows = if sheet.chooser().is_some() {
        3 + listed.max(1)
    } else {
        2
    };
    let due_rows = if sheet.mode().has_due() { 3 } else { 0 };
    let height = 2
        + 3
        + 3
        + due_rows
        + project_rows
        + 1
        + 1
        + preview.len()
        + 1
        + footnote.len()
        + usize::from(sheet.error().is_some())
        + 3;
    let height = u16::try_from(height).unwrap_or(u16::MAX);
    let inside = boxed::draw(frame, area, width, height, "Capture", theme, hits);
    let x = inside.x + 1;
    let inner = inside.width.saturating_sub(2);
    let mut y = inside.y;
    let rect = |y: u16| Rect::new(x, y, inner, 1);

    // The pair: Task and Note, the chosen one reversed.
    let mut at = x;
    for (index, mode) in [Mode::Task, Mode::Note].into_iter().enumerate() {
        let command = if index == 0 {
            CommandId::CaptureTask
        } else {
            CommandId::CaptureNote
        };
        let key = app
            .hint(command)
            .map(|key| format!(" {}", cap(&key)))
            .unwrap_or_default();
        let words = format!(" {}{key} ", mode.field());
        let style = if sheet.mode() == mode {
            text.add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            text
        };
        let w = u16::try_from(words.width()).unwrap_or(0);
        frame.render_widget(Line::styled(words, style), Rect::new(at, y, w, 1));
        hits.add(
            Rect::new(at, y, w, 1),
            Target::Surface(Part::CaptureMode, index),
        );
        at += w + 1;
    }
    y += 1;
    let source = sheet.source();
    let from = format!(
        "From {} · {} · {}",
        source.sender, source.subject, source.when
    );
    frame.render_widget(Line::styled(fit(&from, usize::from(inner)), dim), rect(y));
    y += 2;

    // The text.
    let focused = |field| sheet.field() == field && sheet.chooser().is_none();
    frame.render_widget(
        Line::styled(
            sheet.mode().field(),
            if focused(Field::Text) {
                text.add_modifier(Modifier::BOLD)
            } else {
                dim
            },
        ),
        rect(y),
    );
    y += 1;
    let value = sheet.text().value();
    let cursor = sheet.text().visual_cursor();
    let room = usize::from(inner);
    // The field scrolls to keep the caret in it.
    let skip = cursor.saturating_sub(room.saturating_sub(1));
    let shown: String = value.chars().skip(skip).collect();
    frame.render_widget(Line::styled(fit(&shown, room), text), rect(y));
    hits.add(rect(y), Target::Surface(Part::CaptureText, 0));
    if focused(Field::Text) {
        frame.set_cursor_position(Position::new(
            x + u16::try_from(cursor - skip).unwrap_or(0),
            y,
        ));
    }
    y += 1;
    if source.sentence.is_some() {
        let key = app
            .hint(CommandId::CaptureUseSubject)
            .map(|key| format!(" {}", cap(&key)))
            .unwrap_or_default();
        let lead = "The sentence from the mail, as written";
        let link = "use the subject instead";
        let w = u16::try_from(link.width() + key.width()).unwrap_or(0);
        frame.render_widget(Line::styled(fit(lead, usize::from(inner)), dim), rect(y));
        let start = x + inner.saturating_sub(w);
        frame.render_widget(
            Line::from(vec![Span::styled(link, text), Span::styled(key, accent)]),
            Rect::new(start, y, w.min(inner), 1),
        );
        hits.add(
            Rect::new(start, y, w.min(inner), 1),
            Target::Command(CommandId::CaptureUseSubject.as_str()),
        );
    }
    y += 2;

    // The due day, for a task.
    if sheet.mode().has_due() {
        frame.render_widget(
            Line::from(vec![
                Span::styled(
                    "Due  ",
                    if focused(Field::Due) {
                        text.add_modifier(Modifier::BOLD)
                    } else {
                        dim
                    },
                ),
                Span::styled(capture::due_label(sheet.due()), text),
            ]),
            rect(y),
        );
        let picks = sheet.picks();
        let words: Vec<String> = picks
            .iter()
            .map(|pick| format!(" {} ", pick.words))
            .collect();
        y += 1;
        let mut at = x + 5;
        for (index, (pick, word)) in picks.iter().zip(words).enumerate() {
            let chosen = pick.day == sheet.due();
            let style = match (chosen, focused(Field::Due)) {
                (true, true) => text.add_modifier(Modifier::REVERSED | Modifier::BOLD),
                (true, false) => text.add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                _ => dim,
            };
            let w = u16::try_from(word.width()).unwrap_or(0);
            frame.render_widget(Line::styled(word, style), Rect::new(at, y, w, 1));
            hits.add(
                Rect::new(at, y, w, 1),
                Target::Surface(Part::CapturePick, index),
            );
            at += w + 1;
        }
        y += 2;
    }

    // The project, and why.
    frame.render_widget(
        Line::styled(
            fit(&sheet.project_title(), usize::from(inner)),
            if focused(Field::Project) { text } else { dim },
        ),
        rect(y),
    );
    y += 1;
    let change_key = app
        .hint(CommandId::CaptureChangeProject)
        .map(|key| format!(" {}", cap(&key)))
        .unwrap_or_default();
    let change = format!("Change{change_key}");
    let change_w = u16::try_from(change.width()).unwrap_or(0);
    let left_room = usize::from(inner).saturating_sub(usize::from(change_w) + 2);
    let name = fit(&place, left_room);
    let note = fit(&sheet.note(), left_room.saturating_sub(name.width() + 2));
    frame.render_widget(
        Line::from(vec![
            Span::styled(name, text.add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(note, dim),
        ]),
        rect(y),
    );
    let change_at = x + inner - change_w;
    frame.render_widget(
        Line::from(vec![
            Span::styled("Change", text),
            Span::styled(change_key, accent),
        ]),
        Rect::new(change_at, y, change_w, 1),
    );
    hits.add(
        Rect::new(change_at, y, change_w, 1),
        Target::Command(CommandId::CaptureChangeProject.as_str()),
    );
    y += 1;
    if let Some(chooser) = sheet.chooser() {
        let prompt = format!("Filter projects in your vault: {}", chooser.filter.value());
        frame.render_widget(
            Line::styled(fit(&prompt, usize::from(inner)), text),
            rect(y),
        );
        frame.set_cursor_position(Position::new(
            x + u16::try_from(
                "Filter projects in your vault: ".width() + chooser.filter.visual_cursor(),
            )
            .unwrap_or(0)
            .min(inner.saturating_sub(1)),
            y,
        ));
        y += 1;
        let top = chooser.at.saturating_sub(LISTED - 1);
        for (index, (project, open)) in choices.iter().enumerate().skip(top).take(LISTED) {
            let name = project
                .as_ref()
                .map_or_else(|| capture::INBOX.to_owned(), |project| project.name.clone());
            let note = match project {
                Some(project) => project.note.display().to_string(),
                None => capture::inbox_note(&sheet.tasks_note()),
            };
            let count = capture::open_count(*open);
            let mark = if project.as_ref() == sheet.project() {
                "✓"
            } else {
                " "
            };
            let left = usize::from(inner).saturating_sub(count.width() + 5);
            let name = fit(&name, 20);
            let note = fit(&note, left.saturating_sub(name.width() + 2));
            let gap = left.saturating_sub(name.width() + 2 + note.width());
            let row = Line::from(vec![
                Span::styled(
                    if index == chooser.at { "▌" } else { " " },
                    theme.style(Role::Focus),
                ),
                Span::styled(mark, accent),
                Span::raw(" "),
                Span::styled(name, text.add_modifier(Modifier::BOLD)),
                Span::raw("  "),
                Span::styled(note, dim),
                Span::raw(" ".repeat(gap + 2)),
                Span::styled(count, dim),
            ]);
            let at = Rect::new(x, y, inner, 1);
            frame.render_widget(row, at);
            hits.add(at, Target::Surface(Part::CaptureProject, index));
            y += 1;
        }
        if choices.is_empty() {
            frame.render_widget(Line::styled("No project matches", dim), rect(y));
            y += 1;
        }
    }
    y += 1;

    // The exact line.
    frame.render_widget(
        Line::styled(
            sheet.mode().preview_title(),
            text.add_modifier(Modifier::BOLD),
        ),
        rect(y),
    );
    y += 1;
    for line in preview {
        frame.render_widget(line, rect(y));
        y += 1;
    }
    y += 1;
    for line in footnote {
        frame.render_widget(line, rect(y));
        y += 1;
    }
    if let Some(error) = sheet.error() {
        frame.render_widget(
            Line::styled(fit(error, usize::from(inner)), theme.style(Role::Error)),
            rect(y),
        );
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
                target: Target::Surface(Part::CaptureCancel, 0),
                primary: false,
            },
            Button {
                words: sheet.mode().button().to_owned(),
                key: key(CommandId::CaptureWrite),
                target: Target::Command(CommandId::CaptureWrite.as_str()),
                primary: true,
            },
        ],
        theme,
        hits,
    );
}
