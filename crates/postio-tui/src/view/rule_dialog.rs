//! The rule dialog (terminal.md, "Digests"): a frame 64 wide holding the
//! form and its preview.
//!
//! From, how often and when, "Match a list or a search instead…", what the
//! rule would have caught in the last 90 days, the note, and Cancel and
//! Create. The control the keyboard is on is reversed, so it shows without
//! a colour; each is a click.

use chrono::{DateTime, Local};
use postio_core::CommandId;
use postio_ui::digest;
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

/// How wide the frame is.
pub const WIDTH: u16 = 64;

/// The label column: `From     `.
const LABEL: u16 = 9;

/// Draw the dialog over `area`, when one is open.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    theme: &Theme,
    now: DateTime<Local>,
    hits: &mut Hits,
) {
    let Some(form) = app.rule_form() else {
        return;
    };
    let (dim, text, accent) = (
        theme.style(Role::Dim),
        theme.style(Role::Text),
        theme.style(Role::Accent),
    );
    let width = WIDTH.min(area.width);
    let room = usize::from(width).saturating_sub(4 + usize::from(LABEL));
    let note = wrap(&Line::styled(form.note(), dim), room.max(1));
    let shown = form.preview().map_or(0, |preview| preview.first.len());
    let height = 12
        + u16::try_from(shown + note.len()).unwrap_or(0)
        + u16::from(form.like_this().is_some())
        + u16::from(form.error().is_some())
        + u16::from(
            form.preview()
                .is_some_and(|p| digest::preview_more(p.count, shown).is_some()),
        );
    let heading = crate::view::fit(form.heading(), usize::from(width).saturating_sub(6));
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

    // From.
    label(frame, y, "From");
    if form.query_mode() {
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
            Line::styled(fit(form.senders(), room), text),
            Rect::new(value_x, y, u16::try_from(room).unwrap_or(0), 1),
        );
        y += 1;
        let words = "Match a list or a search instead…";
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
        if form.like_this().is_some() {
            let key = app
                .hint(CommandId::DigestLikeThis)
                .map(|key| format!(" {}", cap(&key)))
                .unwrap_or_default();
            let words = "Digest mail like this";
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

    // Deliver: how often, on which day, at what time.
    y += 1;
    label(frame, y, "Deliver");
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
    let cadence = digest::CADENCES[schedule.cadence.min(2)].1;
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
            chrono::NaiveDate::from_isoywd_opt(2026, 1, digest::WEEKDAYS[schedule.weekday.min(6)])
                .map(|date| date.format("%A").to_string())
                .unwrap_or_default()
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

    // What it would have caught.
    if let Some(preview) = form.preview() {
        frame.render_widget(
            Line::styled(
                fit(
                    &digest::preview_heading(preview.count),
                    usize::from(inside.width) - 2,
                ),
                text.add_modifier(Modifier::BOLD),
            ),
            Rect::new(x, y, inside.width - 2, 1),
        );
        y += 1;
        for (subject, when) in &preview.first {
            let day = digest::preview_day(when.with_timezone(&Local), now);
            let room = usize::from(inside.width).saturating_sub(6 + day.width());
            let subject = fit(subject.as_str(), room);
            let gap = room.saturating_sub(subject.width());
            frame.render_widget(
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(subject, text),
                    Span::raw(" ".repeat(gap + 2)),
                    Span::styled(day, dim),
                ]),
                Rect::new(x, y, inside.width - 2, 1),
            );
            y += 1;
        }
        if let Some(more) = digest::preview_more(preview.count, preview.first.len()) {
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
    if let Some(error) = form.error() {
        let said = fit(error, usize::from(inside.width) - 2);
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
                part: Part::RuleCancel,
                primary: false,
            },
            Button {
                words: digest::create_words(form.editing()).to_owned(),
                key: key(CommandId::PickerConfirm),
                part: Part::RuleCreate,
                primary: true,
            },
        ],
        theme,
        hits,
    );
}
