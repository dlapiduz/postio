//! The search results, drawn over the body: the query and its chips, the
//! tabs, the counts, then the rows in their groups. Quick Look, the
//! timeline and the filter popovers are not drawn here.

use postio_focus::{ResultRow, Run};
use postio_search::results::ResultsTab;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line as Text, Span};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::theme::{Role, Theme};
use crate::view::fit;
use crate::view::hit::{Hits, Target};

/// Lines above the rows: the query, the tabs and the counts.
pub const HEADER: u16 = 3;

fn plain(runs: &[Run]) -> String {
    let said: String = runs.iter().map(|run| run.text.as_str()).collect();
    postio_ui::terminal::SafeText::new(&said).to_string()
}

pub fn body(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, hits: &mut Hits) {
    let Some(results) = app.results() else {
        return;
    };
    let (dim, text, accent) = (
        theme.style(Role::Dim),
        theme.style(Role::Text),
        theme.style(Role::Accent),
    );
    let width = usize::from(area.width);
    let mut y = area.y;
    let mut spans = vec![Span::styled(" ⌕ ", accent)];
    if let Some(query) = &results.query {
        let mut x = area.x + 3;
        for chip in &query.chips {
            let said = format!("{}:{}", chip.operator, chip.value);
            let style = if chip.excluded {
                accent.add_modifier(Modifier::BOLD | Modifier::CROSSED_OUT)
            } else {
                accent.add_modifier(Modifier::BOLD)
            };
            let w = u16::try_from(said.width()).unwrap_or(0);
            hits.add(Rect::new(x, y, w, 1), Target::ResultChip(chip.token));
            spans.push(Span::styled(said, style));
            spans.push(Span::raw("  "));
            x += w + 2;
        }
        spans.push(Span::styled(query.words.clone(), text));
    }
    frame.render_widget(Text::from(spans), Rect::new(area.x, y, area.width, 1));
    y += 1;
    let mut spans = vec![Span::raw(" ")];
    let mut x = area.x + 1;
    for (index, tab) in results.view.tabs.iter().enumerate() {
        let said = format!("{} {}", tab.label, tab.count);
        let style = if tab.selected {
            accent.add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            dim
        };
        let w = u16::try_from(said.width()).unwrap_or(0);
        hits.add(Rect::new(x, y, w, 1), Target::ResultTab(index));
        spans.push(Span::styled(said, style));
        spans.push(Span::raw("   "));
        x += w + 3;
    }
    frame.render_widget(Text::from(spans), Rect::new(area.x, y, area.width, 1));
    y += 1;
    let counts = format!(" {}  {}", results.view.count_line, results.view.sub_line);
    frame.render_widget(
        Text::styled(fit(&counts, width), dim),
        Rect::new(area.x, y, area.width, 1),
    );
    y += 1;
    let foot = u16::from(!results.view.footer.is_empty());
    let room = area.y + area.height - y - foot;
    if let Some(none) = &results.none {
        let mut lines = vec![none.title.clone(), none.body.clone(), String::new()];
        lines.extend(none.relaxations.iter().map(|relaxation| {
            format!(
                "{}  {}  {}",
                relaxation.key.clone().unwrap_or_default(),
                relaxation.label,
                relaxation.count
            )
        }));
        for (offset, line) in lines.iter().take(usize::from(room)).enumerate() {
            frame.render_widget(
                Text::styled(
                    fit(&format!("  {line}"), width),
                    if offset == 0 { text } else { dim },
                ),
                Rect::new(
                    area.x,
                    y + u16::try_from(offset).unwrap_or(0),
                    area.width,
                    1,
                ),
            );
        }
    } else {
        rows(
            frame,
            Rect::new(area.x, y, area.width, room),
            app,
            theme,
            hits,
        );
    }
    if foot > 0 {
        frame.render_widget(
            Text::styled(fit(&format!(" {}", results.view.footer), width), dim),
            Rect::new(area.x, area.y + area.height - 1, area.width, 1),
        );
    }
}

fn rows(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, hits: &mut Hits) {
    let Some(results) = app.results() else {
        return;
    };
    let (dim, text, accent) = (
        theme.style(Role::Dim),
        theme.style(Role::Text),
        theme.style(Role::Accent),
    );
    let mut y = area.y;
    let mut position = results.top;
    while y < area.y + area.height && position < results.view.rows {
        if let Some(group) = results.view.groups.iter().find(|group| {
            group.first == position
                && results
                    .view
                    .tabs
                    .iter()
                    .any(|t| t.selected && t.tab == ResultsTab::Conversations)
        }) {
            frame.render_widget(
                Text::styled(
                    fit(
                        &format!(" {} · {}", group.title, group.count),
                        usize::from(area.width),
                    ),
                    dim.add_modifier(Modifier::BOLD),
                ),
                Rect::new(area.x, y, area.width, 1),
            );
            y += 1;
            if y >= area.y + area.height {
                break;
            }
        }
        let row = Rect::new(area.x, y, area.width, 1);
        let on = position == results.cursor;
        if on {
            frame
                .buffer_mut()
                .set_style(row, theme.style(Role::Surface));
        }
        let (left, right, checked) = line_of(app, position);
        let mut spans = vec![
            Span::styled(if on { "▌" } else { " " }, theme.style(Role::Focus)),
            Span::styled(if checked { "✓ " } else { "  " }, accent),
        ];
        let right_width = right.width();
        let room = usize::from(area.width).saturating_sub(right_width + 4);
        spans.push(Span::styled(
            fit(&left, room),
            if on {
                text.add_modifier(Modifier::BOLD)
            } else {
                text
            },
        ));
        frame.render_widget(Text::from(spans), row);
        let w = u16::try_from(right_width).unwrap_or(0);
        frame.render_widget(
            Text::styled(right, dim),
            Rect::new(row.x + row.width.saturating_sub(w + 1), row.y, w, 1),
        );
        hits.add(row, Target::ResultRow(position));
        position += 1;
        y += 1;
    }
}

/// What a result says at the left and at the right, and whether it is checked.
fn line_of(app: &App, position: u64) -> (String, String, bool) {
    let tab = app
        .results()
        .and_then(|results| results.view.tabs.iter().find(|tab| tab.selected))
        .map_or(ResultsTab::Conversations, |tab| tab.tab);
    match tab {
        ResultsTab::Conversations => match app.result_row(position) {
            Some(row) => (
                said_of(&row),
                format!("{}  {}", row.folder, row.date),
                row.checked,
            ),
            None => ("…".to_owned(), String::new(), false),
        },
        ResultsTab::Files => match app.result_file(position) {
            Some(card) => (
                format!("{}  {}", plain(&card.name), card.subject),
                card.meta,
                false,
            ),
            None => ("…".to_owned(), String::new(), false),
        },
        ResultsTab::People => match app.result_person(position) {
            Some(person) => (
                format!("{}  {}", person.name, person.address),
                format!("{}  {}", person.messages, person.last),
                false,
            ),
            None => ("…".to_owned(), String::new(), false),
        },
    }
}

fn said_of(row: &ResultRow) -> String {
    let mut said = format!(
        "{:<16}{}",
        fit(
            &postio_ui::terminal::SafeText::new(&row.sender).to_string(),
            15
        ),
        plain(&row.subject)
    );
    let passage = plain(&row.passage);
    if !passage.is_empty() {
        said.push_str(" — ");
        said.push_str(&passage);
    }
    said
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};
    use postio_core::Command;

    use crate::app::{App, Effect, Focus, update};
    use crate::test_support::{
        app, key, open_list, places, places_arrive, press, results_arrive, screen, seed_places,
        sent, serve, type_text,
    };

    fn searched() -> (App, Vec<Effect>) {
        let mut app = app((120, 36));
        seed_places(&mut app, places());
        let opening = open_list(&mut app, 3);
        serve(&mut app, opening);
        let opened = update(&mut app, press('/'));
        let folders = app.folders_for_tests();
        places_arrive(&mut app, &opened, &folders, &[], &[]);
        type_text(&mut app, "tide");
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        results_arrive(&mut app, &effects, &["Tide tables", "Atlas budget"]);
        (app, effects)
    }

    #[test]
    fn the_bar_s_search_shows_the_results_and_a_message_found_acts_on_itself() {
        let (mut app, _) = searched();
        let drawn = screen(120, 36, &app);
        assert!(
            !drawn.contains("Saved searches"),
            "the bar is closed:\n{drawn}"
        );
        assert!(
            drawn.contains("Tide tables") && drawn.contains("Atlas budget"),
            "{drawn}"
        );
        assert!(drawn.contains("tide"), "{drawn}");
        assert_eq!(app.cursor(), 0, "the inbox's cursor stays where it was");
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::Reader);
        let effects = update(&mut app, press('e'));
        assert!(
            sent(&effects)
                .iter()
                .any(|command| matches!(command, Command::Archive { .. }))
                || effects
                    .iter()
                    .any(|effect| matches!(effect, Effect::Send(Command::Archive { .. }))),
            "{effects:?}"
        );
    }

    #[test]
    fn escape_leaves_the_results_for_the_inbox() {
        let (mut app, _) = searched();
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        let drawn = screen(120, 36, &app);
        assert!(!drawn.contains("Tide tables"), "{drawn}");
        assert!(drawn.contains("Message 0"), "{drawn}");
    }
}
