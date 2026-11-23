//! The rule dialog: a frame 64 wide holding the form and its preview.

use postio_core::CommandId;
use postio_ui::terminal::SafeText;
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::rule_dialog::Field;
use crate::surface::Part;
use crate::theme::{Role, Theme};
use crate::view::bottom::cap;
use crate::view::boxed::{self, Button};
use crate::view::fit;
use crate::view::hit::{Hits, Target};
use crate::view::wrap::wrap;

pub const WIDTH: u16 = 64;

const LABEL: u16 = 9;

pub fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, hits: &mut Hits) {
    let Some(form) = app.rule_form() else {
        return;
    };
    let view = form.view();
    let (dim, text, accent) = (
        theme.style(Role::Dim),
        theme.style(Role::Text),
        theme.style(Role::Accent),
    );
    let width = WIDTH.min(area.width);
    let room = usize::from(width).saturating_sub(4 + usize::from(LABEL));
    let note = wrap(&Line::styled(view.note.clone(), dim), room.max(1));
    let height = 12
        + u16::try_from(view.preview.len() + note.len()).unwrap_or(0)
        + u16::from(view.like_this.is_some())
        + u16::from(view.error.is_some())
        + u16::from(view.more.is_some());
    let heading = crate::view::fit(
        SafeText::new(&view.heading).as_str(),
        usize::from(width).saturating_sub(6),
    );
    let inside = boxed::draw(frame, area, width, height, &heading, theme, hits);
    let fields = form.fields();
    let focused = form.field();
    let mark = |field: Field| -> Style {
        if focused == field {
            text.add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            text
        }
    };
    let ring = |field: Field| fields.iter().position(|f| *f == field).unwrap_or(0);
    let mut y = inside.y + 1;
    let x = inside.x + 1;
    let label = |frame: &mut Frame, y: u16, words: &str| {
        frame.render_widget(
            Line::styled(words.to_owned(), dim),
            Rect::new(x, y, LABEL, 1),
        );
    };
    let field_at = |hits: &mut Hits, rect: Rect, field: Field| {
        hits.add(rect, Target::Surface(Part::RuleField, ring(field)));
    };
    let value_x = x + LABEL;

    label(frame, y, &view.from_label);
    if view.query.is_some() {
        let value = form.query().value();
        let shown = fit(value, room);
        let w = u16::try_from(room).unwrap_or(0);
        frame.render_widget(
            Line::styled(
                format!("{shown:<room$}"),
                mark(Field::Query).remove_modifier(Modifier::BOLD),
            ),
            Rect::new(value_x, y, w, 1),
        );
        field_at(hits, Rect::new(value_x, y, w, 1), Field::Query);
        if focused == Field::Query {
            let column = form.query().visual_cursor().min(room.saturating_sub(1));
            frame.set_cursor_position(Position::new(
                value_x + u16::try_from(column).unwrap_or(0),
                y,
            ));
        }
        y += 1;
    } else {
        frame.render_widget(
            Line::styled(
                fit(
                    SafeText::new(view.from.as_deref().unwrap_or_default()).as_str(),
                    room,
                ),
                text,
            ),
            Rect::new(value_x, y, u16::try_from(room).unwrap_or(0), 1),
        );
        y += 1;
        let words = view.match_instead.clone().unwrap_or_default();
        let w = u16::try_from(words.width()).unwrap_or(0);
        frame.render_widget(
            Line::styled(
                words,
                mark(Field::MatchInstead).remove_modifier(Modifier::BOLD),
            ),
            Rect::new(value_x, y, w, 1),
        );
        field_at(hits, Rect::new(value_x, y, w, 1), Field::MatchInstead);
        y += 1;
        if let Some(words) = view.like_this.clone() {
            let key = app
                .hint(CommandId::DigestLikeThis)
                .map(|key| format!(" {}", cap(&key)))
                .unwrap_or_default();
            let w = u16::try_from(words.width() + key.width()).unwrap_or(0);
            frame.render_widget(
                Line::from(vec![
                    Span::styled(words, mark(Field::LikeThis).remove_modifier(Modifier::BOLD)),
                    Span::styled(key, accent),
                ]),
                Rect::new(value_x, y, w, 1),
            );
            field_at(hits, Rect::new(value_x, y, w, 1), Field::LikeThis);
            y += 1;
        }
    }

    y += 1;
    label(frame, y, &view.deliver_label);
    let schedule = form.schedule();
    let mut at = value_x;
    let put = |frame: &mut Frame,
               hits: &mut Hits,
               at: &mut u16,
               words: String,
               style: Style,
               field: Option<Field>| {
        let w = u16::try_from(words.width()).unwrap_or(0);
        frame.render_widget(Line::styled(words, style), Rect::new(*at, y, w, 1));
        if let Some(field) = field {
            field_at(hits, Rect::new(*at, y, w, 1), field);
        }
        *at += w;
    };
    let cadence = view.cadences[schedule.cadence.min(2)].clone();
    put(
        frame,
        hits,
        &mut at,
        format!(" {cadence} ▾ "),
        mark(Field::Cadence),
        Some(Field::Cadence),
    );
    if fields.contains(&Field::Day) {
        put(frame, hits, &mut at, " on ".to_owned(), dim, None);
        let day = if schedule.cadence == 1 {
            view.weekdays[schedule.weekday.min(6)].clone()
        } else {
            format!("day {}", schedule.month_day + 1)
        };
        put(
            frame,
            hits,
            &mut at,
            format!(" {day} ▾ "),
            mark(Field::Day),
            Some(Field::Day),
        );
    }
    put(frame, hits, &mut at, " at ".to_owned(), dim, None);
    let time_x = at;
    put(
        frame,
        hits,
        &mut at,
        format!(" {:<5} ", fit(form.time().value(), 5)),
        mark(Field::Time).remove_modifier(Modifier::BOLD),
        Some(Field::Time),
    );
    if focused == Field::Time {
        frame.set_cursor_position(Position::new(
            time_x + 1 + u16::try_from(form.time().visual_cursor().min(5)).unwrap_or(0),
            y,
        ));
    }
    y += 2;

    if let Some(heading) = &view.preview_heading {
        frame.render_widget(
            Line::styled(
                fit(heading, usize::from(inside.width) - 2),
                text.add_modifier(Modifier::BOLD),
            ),
            Rect::new(x, y, inside.width - 2, 1),
        );
        y += 1;
        for line in &view.preview {
            let subject = SafeText::new(&line.subject);
            let room = usize::from(inside.width).saturating_sub(6 + line.day.width());
            let subject = fit(subject.as_str(), room);
            let gap = room.saturating_sub(subject.width());
            frame.render_widget(
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(subject, text),
                    Span::raw(" ".repeat(gap + 2)),
                    Span::styled(line.day.clone(), dim),
                ]),
                Rect::new(x, y, inside.width - 2, 1),
            );
            y += 1;
        }
        if let Some(more) = &view.more {
            frame.render_widget(
                Line::styled(format!("  {more}"), dim),
                Rect::new(x, y, inside.width - 2, 1),
            );
            y += 1;
        }
        y += 1;
    }
    for line in note {
        frame.render_widget(
            line,
            Rect::new(value_x, y, u16::try_from(room).unwrap_or(0), 1),
        );
        y += 1;
    }
    if let Some(error) = &view.error {
        let said = fit(SafeText::new(error).as_str(), usize::from(inside.width) - 2);
        frame.render_widget(
            Line::styled(said, theme.style(Role::Error)),
            Rect::new(x, y, inside.width - 2, 1),
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
                target: Target::Surface(Part::RuleCancel, 0),
                primary: false,
            },
            Button {
                words: view.create.clone(),
                key: key(CommandId::PickerConfirm),
                target: Target::Surface(Part::RuleCreate, 0),
                primary: true,
            },
        ],
        theme,
        hits,
    );
}
