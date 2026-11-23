//! The folders popover: a box 44 wide under the strip's left end, the filter and the places in
//! sections.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line as Text, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear};
use unicode_width::UnicodeWidthStr;

use crate::folders::Folders;
use crate::theme::{Role, Theme};
use crate::view::fit;
use crate::view::hit::{Hits, Target};
use postio_core::CommandId;
use postio_ui::label_colour::{Rgb, label_colour};
use postio_ui::places::{Entry, Mark, Section};

const WIDTH: u16 = 44;

pub fn outer(area: Rect, lines: u16) -> Rect {
    let width = WIDTH.min(area.width.saturating_sub(2));
    Rect::new(
        area.x + 1,
        area.y + 2,
        width,
        (4 + lines).min(area.height.saturating_sub(3)),
    )
}

pub fn draw(
    frame: &mut Frame,
    area: Rect,
    folders: &Folders,
    hint: &dyn Fn(CommandId) -> Option<String>,
    theme: &Theme,
    hits: &mut Hits,
) {
    let entries = folders.entries();
    let mut lines: Vec<(Option<Section>, Option<usize>)> = Vec::new();
    let mut section = None;
    for (index, (_, entry)) in entries.iter().enumerate() {
        if section != Some(entry.section) {
            section = Some(entry.section);
            lines.push((Some(entry.section), None));
        }
        lines.push((None, Some(index)));
    }
    let box_ = outer(area, u16::try_from(lines.len().max(1)).unwrap_or(u16::MAX));
    if box_.width < 20 || box_.height < 5 {
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
    let dim = theme.style(Role::Dim);
    let text = theme.style(Role::Text);
    let accent = theme.style(Role::Accent);
    let typed = folders.filter();
    let mut spans = vec![Span::styled("⌕ ", accent)];
    if typed.is_empty() {
        spans.push(Span::styled(postio_ui::places::FILTER_PLACEHOLDER, dim));
    } else {
        spans.push(Span::styled(
            fit(typed, usize::from(inner.width).saturating_sub(2)),
            text,
        ));
    }
    frame.render_widget(
        Text::from(spans),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    let before: String = typed.chars().take(folders.caret()).collect();
    frame.set_cursor_position(Position::new(
        (inner.x + 2 + u16::try_from(before.width()).unwrap_or(0)).min(inner.x + inner.width - 1),
        inner.y,
    ));
    frame.render_widget(
        Text::styled("─".repeat(usize::from(inner.width)), dim),
        Rect::new(inner.x, inner.y + 1, inner.width, 1),
    );
    if entries.is_empty() {
        frame.render_widget(
            Text::styled("No place matches", dim),
            Rect::new(inner.x + 1, inner.y + 2, inner.width - 1, 1),
        );
    }
    let chosen = folders.chosen();
    let shown = usize::from(inner.height.saturating_sub(2));
    let chosen_line = lines
        .iter()
        .position(|(_, at)| *at == Some(chosen))
        .unwrap_or(0);
    let first = (chosen_line + 1).saturating_sub(shown);
    let hue = crate::row::accent_hue();
    for (offset, (heading, at)) in lines.iter().skip(first).take(shown).enumerate() {
        let row = Rect::new(
            inner.x,
            inner.y + 2 + u16::try_from(offset).unwrap_or(0),
            inner.width,
            1,
        );
        if let Some(section) = heading {
            frame.render_widget(
                Text::styled(section.title(), dim.add_modifier(Modifier::BOLD)),
                Rect::new(row.x + 1, row.y, row.width - 1, 1),
            );
            continue;
        }
        let Some(index) = at else { continue };
        let entry: &Entry = &entries[*index].1;
        let is_chosen = *index == chosen;
        if is_chosen {
            frame
                .buffer_mut()
                .set_style(row, theme.style(Role::Surface));
        }
        hits.add(row, Target::PlaceRow(*index));
        let key = entry.go.and_then(hint).unwrap_or_default();
        let count = entry.count.clone().unwrap_or_default();
        let tail = format!("{count:>9}  {key:<3}");
        let room = usize::from(row.width).saturating_sub(tail.width() + 4);
        let mut spans = vec![Span::styled(
            if is_chosen { "▌" } else { " " },
            theme.style(Role::Focus),
        )];
        let name = fit(&entry.name, room);
        let mark_width = match &entry.mark {
            Mark::Dot(stored) => {
                let rgb = label_colour(&entry.name, stored.as_deref().and_then(Rgb::from_hex), hue);
                spans.push(Span::styled(" ●", theme.label(rgb)));
                2
            }
            Mark::Role(_) => {
                spans.push(Span::raw("  "));
                2
            }
        };
        spans.push(Span::styled(
            format!(" {name}"),
            if is_chosen {
                text.add_modifier(Modifier::BOLD)
            } else {
                text
            },
        ));
        let used = 1 + mark_width + 1 + name.width();
        spans.push(Span::raw(" ".repeat(
            usize::from(row.width).saturating_sub(used + tail.width()),
        )));
        spans.push(Span::styled(format!("{count:>9}"), dim));
        spans.push(Span::styled(format!("  {key:<3}"), accent));
        frame.render_widget(Text::from(spans), row);
    }
}

#[cfg(test)]
mod tests {
    use crate::app::{App, Effect, update};
    use crate::places::{Features, Places};
    use crate::test_support::{
        app, asked, click, folder, hits_of, label, open_list, places_arrive, press, screen,
        seed_places, serve, type_text,
    };
    use crate::view::hit::Target;
    use crossterm::event::{KeyCode, KeyModifiers};
    use postio_model::mailbox::{Mailbox, MailboxRole};
    use postio_model::{AccountId, ListScope, MailboxId};

    fn mailboxes() -> Vec<Mailbox> {
        let mut folders = vec![
            folder(1, "INBOX", MailboxRole::Inbox, 3),
            folder(2, "Archive", MailboxRole::Archive, 0),
            folder(3, "Sent", MailboxRole::Sent, 0),
            folder(6, "Projects", MailboxRole::Regular, 2),
        ];
        folders[0].counts.total = 312;
        folders[3].counts.total = 40;
        folders
    }

    fn opened(filtering: bool) -> App {
        let mut app = app((120, 36));
        seed_places(
            &mut app,
            Places {
                accounts: vec![crate::test_support::account()],
                folders: mailboxes(),
                counts: Vec::new(),
                saved: Vec::new(),
                features: Features {
                    filtering,
                    ..Features::default()
                },
            },
        );
        let opening = open_list(&mut app, 3);
        serve(&mut app, opening);
        app.counts_for_tests(postio_client::protocol::FocusCounts {
            conversations: 312,
            unread: 41,
            has_action: 7,
            filtered_today: 186,
        });
        app
    }

    fn box_opened(app: &mut App, outbox: u32) -> Vec<Effect> {
        update(app, press('g'));
        let effects = update(app, press('l'));
        let mut work = label(7, "Atlas");
        work.color = Some("#3584e4".into());
        places_arrive(
            app,
            &effects,
            &mailboxes(),
            &[(work, 12), (label(8, "Harbor"), 0)],
            &[(AccountId::new(1), outbox)],
        );
        effects
    }

    #[test]
    fn g_l_opens_a_44_wide_box_under_the_strips_left_end_with_its_sections_and_keys() {
        let mut app = opened(true);
        let effects = box_opened(&mut app, 2);
        assert!(
            asked(&effects)
                .iter()
                .any(|(_, request)| *request == postio_focus::Request::Places),
            "{effects:?}"
        );
        let drawn = screen(120, 36, &app);
        let lines: Vec<&str> = drawn.lines().collect();
        assert_eq!(lines[2].chars().nth(1), Some('╭'), "{drawn}");
        assert_eq!(lines[2].chars().nth(44), Some('╮'), "44 wide: {drawn}");
        for wanted in ["Go to folder or label", "Mailboxes", "Folders", "Labels"] {
            assert!(drawn.contains(wanted), "{wanted}:\n{drawn}");
        }
        let row = |name: &str| {
            lines
                .iter()
                .skip(3)
                .find(|line| line.contains(name))
                .unwrap_or_else(|| panic!("no {name}:\n{drawn}"))
                .to_owned()
        };
        assert!(row("Archive").contains("g a"), "{}", row("Archive"));
        assert!(row("Sent").contains("g t"), "{}", row("Sent"));
        assert!(row("Snoozed").contains("g b"), "{}", row("Snoozed"));
        assert!(row("Flagged").contains("g s"), "{}", row("Flagged"));
        assert!(row("Outbox").contains('2'), "{}", row("Outbox"));
        assert!(row("Filtered").contains("186 today") && row("Filtered").contains("g f"));
        assert!(row("Projects").contains("40"), "{}", row("Projects"));
        assert!(
            row("Atlas").contains('●') && row("Atlas").contains("12"),
            "{}",
            row("Atlas")
        );
        assert!(
            row("Inbox").contains('▌'),
            "the first row is chosen: {}",
            row("Inbox")
        );
    }

    #[test]
    fn typing_filters_and_enter_goes_there() {
        let mut app = opened(true);
        box_opened(&mut app, 2);
        type_text(&mut app, "proj");
        let drawn = screen(120, 36, &app);
        assert!(drawn.contains("proj"), "{drawn}");
        assert!(!drawn.contains("Archive"), "{drawn}");
        let effects = update(
            &mut app,
            crate::test_support::key(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert!(
            crate::test_support::opens(&effects, ListScope::Mailbox(MailboxId::new(6))),
            "{effects:?}"
        );
        assert!(
            !screen(120, 36, &app).contains("Mailboxes"),
            "the box is put away"
        );
        assert_eq!(app.focus(), crate::app::Focus::List);
    }

    #[test]
    fn the_g_keys_work_from_inside_and_escape_closes() {
        let mut app = opened(true);
        box_opened(&mut app, 0);
        update(&mut app, press('g'));
        let effects = update(&mut app, press('a'));
        assert!(
            crate::test_support::opens(&effects, ListScope::Mailbox(MailboxId::new(2))),
            "{effects:?}"
        );
        box_opened(&mut app, 0);
        update(&mut app, press('g'));
        update(&mut app, press('x'));
        let drawn = screen(120, 36, &app);
        assert!(
            drawn.contains("gx"),
            "two letters that are no key are typed:\n{drawn}"
        );
        update(
            &mut app,
            crate::test_support::key(KeyCode::Esc, KeyModifiers::NONE),
        );
        assert!(!screen(120, 36, &app).contains("Mailboxes"));
    }

    #[test]
    fn a_click_on_the_place_opens_it_and_a_click_on_a_row_goes_there() {
        let mut app = opened(true);
        let hits = hits_of(120, 36, &app);
        let place = hits.at(2, 1).expect("the strip's place");
        assert_eq!(place.target, Target::Command("go_to_folders"));
        let effects = update(&mut app, click(place.target, false, false));
        places_arrive(&mut app, &effects, &mailboxes(), &[], &[]);
        let drawn = screen(120, 36, &app);
        let row = drawn
            .lines()
            .position(|line| line.contains("Archive"))
            .unwrap();
        let hits = hits_of(120, 36, &app);
        let at = hits.at(6, u16::try_from(row).unwrap()).expect("a row");
        let effects = update(&mut app, click(at.target, false, false));
        assert!(
            crate::test_support::opens(&effects, ListScope::Mailbox(MailboxId::new(2))),
            "{effects:?}"
        );
    }
}
