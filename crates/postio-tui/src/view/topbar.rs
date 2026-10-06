//! The bar across the top (terminal.md, "The screen").
//!
//! `Compose c` at the left; the command field, `min(60, W - 40)` wide and
//! centred, holding its placeholder and the keys that open it, or the query
//! once it is open; at the right the sync label and `? keys`. Below 80
//! columns the field shrinks to its placeholder's first word and its key.
//! Keys come from the keymap in force, so a rebound key is hinted as
//! rebound. Each control is a click target for the command its key runs.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::theme::{Role, Theme};
use crate::view::hit::{Hits, Target};
use postio_core::CommandId;

/// The field's width when there is room.
const FIELD: u16 = 60;
/// Below this width the field shrinks.
const NARROW: u16 = 80;
/// The gap between the bar's parts.
const GAP: u16 = 3;

/// The symbol that stands for a sync label's icon.
fn sync_mark(icon: &str) -> &'static str {
    match icon {
        "emblem-ok-symbolic" => "✓",
        "dialog-warning-symbolic" => "⚠",
        "network-offline-symbolic" => "○",
        _ => "↻",
    }
}

/// A piece of text as a width.
fn cells(text: &str) -> u16 {
    u16::try_from(text.width()).unwrap_or(u16::MAX)
}

/// Draw the bar into `area`, one row.
pub fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, hits: &mut Hits) {
    let dim = theme.style(Role::Dim);
    let accent = theme.style(Role::Accent);

    // Compose, at the left.
    let compose_key = app.hint(CommandId::Compose);
    let compose = format!(
        "Compose{}",
        compose_key
            .as_deref()
            .map_or(String::new(), |key| format!(" {key}"))
    );
    let compose_width = cells(&compose);
    let compose_at = area.x + 1;
    let mut spans = vec![Span::styled(
        "Compose",
        theme.style(Role::Text).add_modifier(Modifier::BOLD),
    )];
    if let Some(key) = &compose_key {
        spans.push(Span::styled(format!(" {key}"), accent));
    }
    frame.render_widget(
        Line::from(spans),
        Rect::new(compose_at, area.y, compose_width, 1),
    );
    hits.add(
        Rect::new(compose_at, area.y, compose_width, 1),
        Target::Command("compose"),
    );
    let left_end = compose_at + compose_width;

    // The right side: the sync label, then the keys.
    let keys = app.hint(CommandId::CheatSheet);
    let keys_text = keys
        .as_deref()
        .map_or("keys".to_owned(), |key| format!("{key} keys"));
    let keys_width = cells(&keys_text);
    let label = app.sync_label();
    let sync = format!("{} {}", sync_mark(label.icon), label.text);
    let sync_width = cells(&sync);
    let keys_at = (area.x + area.width).saturating_sub(keys_width + 1);
    frame.render_widget(
        Line::from(vec![
            Span::styled(keys.clone().unwrap_or_default(), accent),
            Span::styled(
                if keys.is_some() {
                    " keys".to_owned()
                } else {
                    "keys".to_owned()
                },
                dim,
            ),
        ]),
        Rect::new(keys_at, area.y, keys_width, 1),
    );
    hits.add(
        Rect::new(keys_at, area.y, keys_width, 1),
        Target::Command("cheat_sheet"),
    );

    // The field: centred, between what is either side of it.
    let search_key = app.hint(CommandId::Search);
    let palette_key = app.hint(CommandId::CommandPalette);
    let key_text = [search_key.as_deref(), palette_key.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    let narrow = area.width < NARROW;
    let placeholder = postio_ui::focus_row::COMMAND_PROMPT;
    let placeholder = if narrow {
        placeholder.split(' ').next().unwrap_or(placeholder)
    } else {
        placeholder
    };
    let mut width = if narrow {
        // The word, and the one key.
        (2 + cells(placeholder) + 2 + cells(search_key.as_deref().unwrap_or(""))).min(FIELD)
    } else {
        FIELD.min(area.width.saturating_sub(40))
    };
    let right_start = |with_sync: bool| {
        if with_sync {
            keys_at.saturating_sub(sync_width + GAP)
        } else {
            keys_at
        }
    };
    let mut with_sync = true;
    let centred = area.x + area.width.saturating_sub(width) / 2;
    let mut at = centred.max(left_end + GAP);
    if at + width + GAP > right_start(true) {
        // No room for the sync label beside the field: it goes.
        with_sync = false;
    }
    if at + width + 1 > right_start(with_sync) {
        // Nor for the field as wide: it shrinks.
        width = right_start(with_sync).saturating_sub(at + 1);
    }
    if width < 4 {
        at = left_end + 1;
        width = 0;
    }
    if with_sync {
        frame.render_widget(
            Line::styled(sync, dim),
            Rect::new(right_start(true), area.y, sync_width, 1),
        );
    }
    if width == 0 {
        return;
    }
    let field = Rect::new(at, area.y, width, 1);
    frame
        .buffer_mut()
        .set_style(field, theme.style(Role::Surface));
    hits.add(field, Target::Command("search"));
    let inner = Rect::new(field.x + 1, field.y, field.width.saturating_sub(2), 1);
    match app.bar_typed() {
        Some(typed) => super::bar::field(frame, inner, typed, theme),
        None => {
            frame.render_widget(
                Line::styled(format!("⌕ {placeholder}"), dim),
                Rect::new(inner.x, inner.y, inner.width, 1),
            );
            let key_width = cells(&key_text);
            // Its keys, at the field's right end, while they leave the
            // placeholder whole.
            if !key_text.is_empty() && cells(placeholder) + 2 + 3 + key_width <= inner.width {
                let mut keys = Vec::new();
                if let Some(key) = &search_key {
                    keys.push(Span::styled(key.clone(), accent));
                }
                if let Some(key) = &palette_key {
                    if !keys.is_empty() {
                        keys.push(Span::raw(" "));
                    }
                    keys.push(Span::styled(key.clone(), accent));
                }
                frame.render_widget(
                    Line::from(keys),
                    Rect::new(inner.x + inner.width - key_width, inner.y, key_width, 1),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::{app, hits_of, local, places, screen, seed_places};
    use crate::view::hit::Target;

    fn top(width: u16) -> (String, crate::app::App) {
        let mut app = app((width, 20));
        let mut contents = places();
        contents.folders[0].last_synced_at = Some(local(23, 16, 9));
        seed_places(&mut app, contents);
        (
            screen(width, 20, &app)
                .lines()
                .next()
                .expect("a top row")
                .to_owned(),
            app,
        )
    }

    #[test]
    fn the_top_bar_has_compose_the_field_the_sync_label_and_the_keys() {
        let (bar, _) = top(120);
        assert!(bar.trim_start().starts_with("Compose c"), "{bar}");
        assert!(
            bar.contains("⌕ Search mail, go to a folder, or run a command"),
            "{bar}"
        );
        assert!(bar.contains("/ ctrl+k"), "the field's keys: {bar}");
        assert!(bar.contains("✓ Synced 16:09   ? keys"), "{bar}");
        assert!(bar.trim_end().ends_with("? keys"), "{bar}");
        // 60 wide and centred: the field starts at column 30.
        let field = bar.chars().position(|c| c == '⌕').expect("the field");
        assert_eq!(field, 31, "the field is 60 wide and centred: {bar}");
    }

    #[test]
    fn below_80_columns_the_field_shrinks_to_its_first_word_and_its_key() {
        let (bar, _) = top(70);
        assert!(bar.contains("⌕ Search"), "{bar}");
        assert!(!bar.contains("go to a folder"), "{bar}");
        assert!(bar.contains("Compose c") && bar.contains("? keys"), "{bar}");
    }

    #[test]
    fn at_the_minimum_the_bar_keeps_compose_the_field_and_the_keys() {
        let (bar, _) = top(50);
        assert!(bar.contains("Compose c"), "{bar}");
        assert!(bar.contains("⌕ Search"), "{bar}");
        assert!(bar.trim_end().ends_with("? keys"), "{bar}");
    }

    #[test]
    fn a_rebound_key_is_hinted_as_rebound() {
        let mut bindings = postio_config::KeyBindings::default();
        bindings
            .overrides_mut()
            .insert("compose".into(), "N".into());
        let mut app = crate::test_support::app_with_keys((120, 20), &bindings);
        seed_places(&mut app, places());
        let drawn = screen(120, 20, &app);
        let bar = drawn.lines().next().expect("a top row");
        assert!(bar.contains("Compose N"), "{bar}");
        assert!(!bar.contains("Compose c"), "{bar}");
    }

    #[test]
    fn the_bars_controls_are_click_targets_for_their_commands() {
        let (bar, app) = top(120);
        let hits = hits_of(120, 20, &app);
        let at = |needle: &str| {
            let x = bar.find(needle).expect(needle);
            let x = u16::try_from(bar[..x].chars().count()).unwrap();
            hits.at(x, 0).map(|hit| hit.target)
        };
        assert_eq!(at("Compose"), Some(Target::Command("compose")));
        assert_eq!(at("Search mail"), Some(Target::Command("search")));
        assert_eq!(at("keys"), Some(Target::Command("cheat_sheet")));
    }

    #[test]
    fn the_query_replaces_the_placeholder_once_the_field_is_open() {
        let (_, mut app) = top(120);
        crate::test_support::type_text(&mut app, "/tide");
        let drawn = screen(120, 20, &app);
        let bar = drawn.lines().next().unwrap();
        assert!(bar.contains("⌕ tide"), "{bar}");
        assert!(!bar.contains("go to a folder"), "{bar}");
    }
}
