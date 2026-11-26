use postio_focus::CalendarView;
use postio_model::calendar::EditScope;
use postio_ui::calendar::editor::{EditorField, EditorView, scope_label};
use postio_ui::calendar::{AgendaRow, EventView};
use postio_ui::terminal::SafeText;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use crate::surface::Part;
use crate::theme::{Role, Theme};
use crate::view::fit;
use crate::view::hit::{Hits, Target};

pub fn strip(frame: &mut Frame, area: Rect, theme: &Theme) {
    let spans = vec![
        Span::styled(
            " Calendar",
            theme.style(Role::Text).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  Agenda", theme.style(Role::Dim)),
    ];
    frame.render_widget(Line::from(spans), area);
}

pub fn body(
    frame: &mut Frame,
    area: Rect,
    view: &CalendarView,
    editor_field: usize,
    theme: &Theme,
    hits: &mut Hits,
) {
    if area.height < 2 {
        return;
    }
    let rows = Rect::new(area.x, area.y, area.width, area.height - 1);
    let footer = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
    let width = usize::from(area.width);
    let height = usize::from(rows.height);
    let top = view
        .cursor
        .map_or(0, |at| at.saturating_sub(height.saturating_sub(1) / 2))
        .min(view.rows.len().saturating_sub(height));
    for (offset, row) in view.rows.iter().enumerate().skip(top).take(height) {
        let y = rows.y + u16::try_from(offset - top).unwrap_or(0);
        let line = Rect::new(rows.x, y, rows.width, 1);
        frame.render_widget(
            row_line(row, Some(offset) == view.cursor, width, theme),
            line,
        );
        if matches!(row, AgendaRow::Occurrence(_)) {
            hits.add(line, Target::Surface(Part::CalendarRow, offset));
        }
    }
    if let Some(empty) = &view.empty {
        frame.render_widget(
            Line::styled(format!(" {empty}"), theme.style(Role::Dim)),
            Rect::new(rows.x, rows.y, rows.width, 1),
        );
    }
    let keys: Vec<String> = view
        .footer
        .iter()
        .map(|hint| format!("{} {}", hint.key, hint.label))
        .collect();
    frame.render_widget(
        Line::styled(
            format!(" {}", fit(&keys.join(" \u{b7} "), width.saturating_sub(1))),
            theme.style(Role::Dim),
        ),
        footer,
    );
    if let Some(editing) = &view.editor {
        editor(frame, area, editing, editor_field, theme, hits);
    } else if let Some(open) = &view.open {
        event(frame, area, open, theme, hits);
    }
}

fn editor(
    frame: &mut Frame,
    area: Rect,
    view: &EditorView,
    at: usize,
    theme: &Theme,
    hits: &mut Hits,
) {
    let mut lines: Vec<(String, Role)> = Vec::new();
    for (index, (field, text)) in view.fields.iter().enumerate() {
        let shown = if text.is_empty() && index != at {
            field.placeholder().to_owned()
        } else {
            text.replace('\n', " \u{21b5} ")
        };
        let mark = if index == at { "\u{25b8}" } else { " " };
        let role = match (index == at, text.is_empty()) {
            (true, _) => Role::Focus,
            (false, true) => Role::Dim,
            (false, false) => Role::Text,
        };
        lines.push((
            format!(
                "{mark} {:<7} {}",
                field.label(),
                SafeText::new(&shown).as_str()
            ),
            role,
        ));
        if *field == EditorField::Ends {
            match &view.when {
                Ok(when) => lines.push((format!("          {when}"), Role::Dim)),
                Err(why) => lines.push((format!("          {why}"), Role::Error)),
            }
        }
    }
    if let Some(scope) = view.scope {
        let chosen = view.fields.len() == at;
        let words: Vec<String> = [EditScope::This, EditScope::ThisAndFollowing, EditScope::All]
            .iter()
            .map(|each| {
                if *each == scope {
                    format!("[{}]", scope_label(*each))
                } else {
                    scope_label(*each).to_owned()
                }
            })
            .collect();
        lines.push((
            format!(
                "{} {:<7} {}",
                if chosen { "\u{25b8}" } else { " " },
                "Which",
                words.join("  ")
            ),
            if chosen { Role::Focus } else { Role::Text },
        ));
    }
    if let Some(error) = &view.error {
        lines.push((error.clone(), Role::Error));
    }
    let keys: Vec<String> = view
        .keys
        .iter()
        .map(|hint| format!("{} {}", hint.key, hint.label))
        .chain(std::iter::once("Tab next field".to_owned()))
        .collect();
    lines.push((String::new(), Role::Text));
    lines.push((keys.join(" \u{b7} "), Role::Dim));
    let height = u16::try_from(lines.len() + 2).unwrap_or(u16::MAX);
    let inside = crate::view::boxed::draw(
        frame,
        area,
        area.width.min(72),
        height,
        &view.heading,
        theme,
        hits,
    );
    let width = usize::from(inside.width);
    for (offset, (line, role)) in lines.iter().take(usize::from(inside.height)).enumerate() {
        frame.render_widget(
            Line::styled(fit(line, width), theme.style(*role)),
            Rect::new(inside.x, inside.y + offset as u16, inside.width, 1),
        );
    }
}

fn row_line(row: &AgendaRow, ringed: bool, width: usize, theme: &Theme) -> Line<'static> {
    match row {
        AgendaRow::Day { label, .. } => Line::styled(
            format!(
                " {}",
                fit(SafeText::new(label).as_str(), width.saturating_sub(1))
            ),
            theme.style(Role::Text).add_modifier(Modifier::BOLD),
        ),
        AgendaRow::Occurrence(row) => {
            let mut text = format!("   {:<7} {}", row.time, SafeText::new(&row.title).as_str());
            if !row.location.is_empty() {
                text.push_str(&format!("  {}", SafeText::new(&row.location).as_str()));
            }
            if let Some(chip) = &row.chip {
                text.push_str(&format!("  [{}]", chip.label()));
            }
            if row.queued {
                text.push_str("  \u{21bb}");
            }
            let role = match (ringed, row.past) {
                (true, _) => Role::Selection,
                (false, true) => Role::Dim,
                (false, false) => Role::Text,
            };
            Line::styled(fit(&text, width), theme.style(role))
        }
    }
}

fn event(frame: &mut Frame, area: Rect, open: &EventView, theme: &Theme, hits: &mut Hits) {
    let mut lines = vec![open.when.clone()];
    lines.extend(open.zone.clone());
    lines.extend(open.repeat.clone());
    if !open.location.is_empty() {
        lines.push(open.location.clone());
    }
    lines.push(open.calendar.clone());
    lines.extend(open.people.iter().cloned());
    lines.extend(open.reminders.iter().cloned());
    if !open.notes.is_empty() {
        lines.push(String::new());
        lines.extend(open.notes.lines().map(str::to_owned));
    }
    let height = u16::try_from(lines.len() + 2).unwrap_or(u16::MAX);
    let inside = crate::view::boxed::draw(
        frame,
        area,
        area.width.min(64),
        height,
        SafeText::new(&open.title).as_str(),
        theme,
        hits,
    );
    let width = usize::from(inside.width);
    for (offset, line) in lines.iter().take(usize::from(inside.height)).enumerate() {
        frame.render_widget(
            Line::styled(
                fit(SafeText::new(line).as_str(), width),
                theme.style(Role::Text),
            ),
            Rect::new(inside.x, inside.y + offset as u16, inside.width, 1),
        );
    }
}
