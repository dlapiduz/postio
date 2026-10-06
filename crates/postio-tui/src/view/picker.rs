//! The pickers, drawn (terminal.md, "Pickers"): snooze, remind, label and
//! move, in a framed box 48 wide anchored under the focused row's subject
//! column, or above the row when it does not fit below.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line as Text, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear};
use unicode_width::UnicodeWidthStr;

use crate::pickers::{Kind, Picker};
use crate::theme::{Role, Theme};
use crate::view::fit;
use crate::view::hit::{Hits, Target};
use postio_core::{CommandId, Keymap};
use postio_ui::label_colour::{Rgb, label_colour};

/// The box's width.
const WIDTH: u16 = 48;
/// The most rows the list shows before it scrolls.
const ROWS: usize = 10;

/// What the picker is anchored to, and what it may use.
pub struct Place {
    /// The whole screen the picker may be drawn in, less the bottom line.
    pub area: Rect,
    /// Where the focused row was drawn, when it is in view.
    pub row: Option<Rect>,
    /// Where the subject column starts.
    pub subject_x: u16,
}

/// Where a box `width` x `height` goes: under the row at the subject
/// column, above it when it does not fit below.
pub fn anchored(place: &Place, width: u16, height: u16) -> Rect {
    let area = place.area;
    let width = width.min(area.width);
    let x = place
        .subject_x
        .min((area.x + area.width).saturating_sub(width))
        .max(area.x);
    let bottom = area.y + area.height;
    let y = match place.row {
        Some(row) if row.y + row.height + height <= bottom => row.y + row.height,
        Some(row) if row.y >= area.y + height => row.y - height,
        Some(_) | None => bottom.saturating_sub(height).max(area.y),
    };
    Rect::new(x, y, width, height.min(area.height))
}

/// Draw `picker`.
pub fn draw(
    frame: &mut Frame,
    place: &Place,
    picker: &Picker,
    keymap: &Keymap,
    hint: &dyn Fn(CommandId) -> Option<String>,
    theme: &Theme,
    hits: &mut Hits,
) {
    let rows = picker.rows();
    let sections = rows.iter().filter(|row| row.section.is_some()).count();
    let listed = (rows.len() + sections).clamp(1, ROWS);
    let filter = matches!(picker.kind(), Kind::Label | Kind::Move);
    let date = !filter;
    let inner_width = usize::from(WIDTH.min(place.area.width).saturating_sub(2));
    let note = crate::view::wrap::wrap(
        &Text::raw(picker.footnote(keymap)),
        inner_width.saturating_sub(2),
    );
    let height = 2
        + u16::from(filter)
        + u16::try_from(listed).unwrap_or(0)
        + u16::from(date)
        + u16::try_from(note.len()).unwrap_or(0);
    let box_ = anchored(place, WIDTH, height);
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
    let dim = theme.style(Role::Dim);
    let text = theme.style(Role::Text);
    let accent = theme.style(Role::Accent);
    // The title and what it acts on, in the top border.
    let title = format!(" {} ", picker.title());
    let title_width = u16::try_from(title.width()).unwrap_or(0);
    frame.render_widget(
        Text::styled(title, accent.add_modifier(Modifier::BOLD)),
        Rect::new(box_.x + 2, box_.y, title_width.min(box_.width - 4), 1),
    );
    let room = usize::from(box_.width).saturating_sub(usize::from(title_width) + 7);
    let target = fit(picker.target(), room);
    if !target.is_empty() {
        let shown = format!(" {target} ");
        let w = u16::try_from(shown.width()).unwrap_or(0);
        frame.render_widget(
            Text::styled(shown, dim),
            Rect::new(box_.x + box_.width - 2 - w, box_.y, w, 1),
        );
    }
    let inner = Rect::new(box_.x + 1, box_.y + 1, box_.width - 2, box_.height - 2);
    let mut y = inner.y;
    if filter {
        let typed = picker.typed();
        let mut spans = vec![Span::styled("⌕ ", accent)];
        match (typed.is_empty(), picker.placeholder()) {
            (true, Some(placeholder)) => spans.push(Span::styled(placeholder, dim)),
            _ => spans.push(Span::styled(
                fit(typed, usize::from(inner.width).saturating_sub(2)),
                text,
            )),
        }
        frame.render_widget(Text::from(spans), Rect::new(inner.x, y, inner.width, 1));
        let before: String = typed.chars().take(picker.caret()).collect();
        frame.set_cursor_position(Position::new(
            (inner.x + 2 + u16::try_from(before.width()).unwrap_or(0))
                .min(inner.x + inner.width - 1),
            y,
        ));
        y += 1;
    }
    // The rows, scrolled to keep the chosen one in view.
    let chosen = picker.chosen();
    let mut lines: Vec<(Option<&'static str>, Option<usize>)> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        if let Some(section) = row.section {
            lines.push((Some(section), None));
        }
        lines.push((None, Some(index)));
    }
    let chosen_line = lines
        .iter()
        .position(|(_, at)| *at == Some(chosen))
        .unwrap_or(0);
    let first = (chosen_line + 1).saturating_sub(listed);
    let hue = crate::row::accent_hue();
    for (offset, (section, at)) in lines.iter().skip(first).take(listed).enumerate() {
        let row_rect = Rect::new(
            inner.x,
            y + u16::try_from(offset).unwrap_or(0),
            inner.width,
            1,
        );
        if let Some(section) = section {
            frame.render_widget(
                Text::styled(*section, dim.add_modifier(Modifier::BOLD)),
                Rect::new(row_rect.x + 1, row_rect.y, row_rect.width - 1, 1),
            );
            continue;
        }
        let Some(index) = at else { continue };
        let row = &rows[*index];
        let is_chosen = *index == chosen;
        if is_chosen {
            frame
                .buffer_mut()
                .set_style(row_rect, theme.style(Role::Surface));
        }
        hits.add(row_rect, Target::PickRow(*index));
        let mut spans = vec![
            Span::styled(if is_chosen { "▌" } else { " " }, theme.style(Role::Focus)),
            Span::styled(row.number.map_or(" ".to_owned(), |n| n.to_string()), accent),
            Span::raw(" "),
        ];
        let mut used = 3;
        if let Some((name, stored)) = &row.dot {
            let rgb = label_colour(name, stored.as_deref().and_then(Rgb::from_hex), hue);
            spans.push(Span::styled("● ", theme.label(rgb)));
            used += 2;
        }
        let detail = row.detail.clone();
        let room = usize::from(row_rect.width).saturating_sub(used + detail.width() + 2);
        let name = fit(&row.name, room);
        used += name.width();
        spans.push(Span::styled(
            name,
            if is_chosen {
                text.add_modifier(Modifier::BOLD)
            } else {
                text
            },
        ));
        let gap = usize::from(row_rect.width).saturating_sub(used + detail.width() + 1);
        spans.push(Span::raw(" ".repeat(gap)));
        spans.push(Span::styled(detail, dim));
        frame.render_widget(Text::from(spans), row_rect);
    }
    y += u16::try_from(listed).unwrap_or(0);
    if date {
        let line = Rect::new(inner.x, y, inner.width, 1);
        let typed = picker.typed();
        let mut spans = vec![Span::raw(" ")];
        if picker.in_field() {
            spans.push(Span::styled("⌕ ", accent));
            spans.push(Span::styled(
                fit(typed, usize::from(inner.width).saturating_sub(4)),
                text,
            ));
            let before: String = typed.chars().take(picker.caret()).collect();
            frame.set_cursor_position(Position::new(
                (inner.x + 3 + u16::try_from(before.width()).unwrap_or(0))
                    .min(inner.x + inner.width - 1),
                y,
            ));
        } else {
            spans.push(Span::styled(postio_ui::pickers::DATE_PLACEHOLDER, dim));
        }
        frame.render_widget(Text::from(spans), line);
        let right = match (picker.date_hint(), hint(CommandId::PickerTypeDate)) {
            (Some(said), _) => Some((said, dim)),
            (None, Some(key)) if !picker.in_field() => Some((key, accent)),
            _ => None,
        };
        if let Some((said, style)) = right {
            let w = u16::try_from(said.width()).unwrap_or(0);
            if w + 2 < inner.width {
                frame.render_widget(
                    Text::styled(said, style),
                    Rect::new(inner.x + inner.width - w - 1, y, w, 1),
                );
            }
        }
        y += 1;
    }
    for (offset, line) in note.into_iter().enumerate() {
        let text: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        frame.render_widget(
            Text::styled(text, dim),
            Rect::new(
                inner.x + 1,
                y + u16::try_from(offset).unwrap_or(0),
                inner.width - 1,
                1,
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Local, Utc};
    use crossterm::event::{KeyCode, KeyModifiers};
    use postio_core::Command;
    use postio_ui::pickers::when_label;

    use crate::app::{App, Effect, update};
    use crate::test_support::{app, key, open_list, places, press, screen, seed_places, serve};

    fn opened(height: u16) -> App {
        let mut app = app((120, height));
        seed_places(&mut app, places());
        let opening = open_list(&mut app, 3);
        serve(&mut app, opening);
        app
    }

    fn sent(effects: &[Effect]) -> Vec<Command> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Send(command) => Some(command.clone()),
                _ => None,
            })
            .collect()
    }

    fn snooze_presets() -> [(&'static str, DateTime<Local>); 4] {
        postio_ui::schedule::snooze_presets(Local::now())
    }

    #[test]
    fn s_opens_the_snooze_box_under_the_rows_subject_column_with_numbered_presets() {
        let mut app = opened(36);
        update(&mut app, press('s'));
        let drawn = screen(120, 36, &app);
        let lines: Vec<&str> = drawn.lines().collect();
        let top = lines
            .iter()
            .position(|line| line.contains("Snooze until"))
            .unwrap_or_else(|| panic!("no snooze box:\n{drawn}"));
        // The cursor's row is the first under the day heading: row 3.
        assert_eq!(top, 4, "under the focused row:\n{drawn}");
        let corner = lines[top].chars().position(|c| c == '╭').expect("a corner");
        assert_eq!(
            corner,
            usize::from(crate::view::list::subject_x(120, false)),
            "under the subject column:\n{drawn}"
        );
        assert_eq!(
            lines[top].chars().nth(corner + 47),
            Some('╮'),
            "48 wide:\n{drawn}"
        );
        assert!(lines[top].contains("Ada · Message 0"), "{drawn}");
        let now = Local::now();
        for (index, (name, at)) in snooze_presets().iter().enumerate() {
            let line = lines
                .iter()
                .find(|line| line.contains(name))
                .unwrap_or_else(|| panic!("no {name}:\n{drawn}"));
            assert!(line.contains(&format!("{}", index + 1)), "{line}");
            assert!(line.contains(&when_label(*at, now)), "{line}");
        }
        assert!(drawn.contains("Or type a date: “tue 9am”"), "{drawn}");
        assert!(drawn.contains("Tab"), "{drawn}");
        assert!(
            drawn.contains("Snoozed mail is") && drawn.contains("under g z."),
            "{drawn}"
        );
    }

    #[test]
    fn a_number_chooses_a_preset_and_snoozes_the_row_until_then() {
        let mut app = opened(36);
        update(&mut app, press('s'));
        let effects = update(&mut app, press('2'));
        let until = snooze_presets()[1].1.with_timezone(&Utc);
        let commands = sent(&effects);
        assert!(
            commands.iter().any(|command| matches!(
                command,
                Command::Snooze { until: Some(at), .. } if *at == until
            )),
            "{commands:?}"
        );
        assert!(
            !screen(120, 36, &app).contains("Snooze until"),
            "the box closes"
        );
    }

    #[test]
    fn tab_goes_to_the_typed_date_and_enter_takes_it_when_it_is_one() {
        let mut app = opened(36);
        update(&mut app, press('s'));
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        for c in "tue 9am".chars() {
            update(&mut app, press(c));
        }
        let drawn = screen(120, 36, &app);
        assert!(drawn.contains("tue 9am"), "{drawn}");
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let wanted = postio_ui::pickers::typed("tue 9am", Local::now())
            .unwrap()
            .with_timezone(&Utc);
        assert!(
            sent(&effects).iter().any(|command| matches!(
                command,
                Command::Snooze { until: Some(at), .. } if *at == wanted
            )),
            "{effects:?}"
        );
    }

    #[test]
    fn words_that_are_no_date_send_nothing_and_say_what_to_type() {
        let mut app = opened(36);
        update(&mut app, press('s'));
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        for c in "receipts".chars() {
            update(&mut app, press(c));
        }
        assert!(screen(120, 36, &app).contains("A day and a time"));
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(sent(&effects).is_empty(), "{effects:?}");
        assert!(
            screen(120, 36, &app).contains("Snooze until"),
            "it stays open"
        );
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!screen(120, 36, &app).contains("Snooze until"));
    }

    #[test]
    fn h_opens_remind_and_sends_the_reminder() {
        let mut app = opened(36);
        update(&mut app, press('h'));
        let drawn = screen(120, 36, &app);
        assert!(drawn.contains("Remind me if no one replies by"), "{drawn}");
        assert!(drawn.contains("In 2 working days"), "{drawn}");
        let effects = update(&mut app, press('1'));
        assert!(
            sent(&effects)
                .iter()
                .any(|command| matches!(command, Command::RemindIfNoReply { at: Some(_), .. })),
            "{effects:?}"
        );
    }

    #[test]
    fn near_the_bottom_the_box_opens_above_the_row() {
        let mut app = app((120, 24));
        seed_places(&mut app, places());
        let opening = open_list(&mut app, 20);
        serve(&mut app, opening);
        for _ in 0..17 {
            update(&mut app, press('j'));
        }
        update(&mut app, press('s'));
        let drawn = screen(120, 24, &app);
        let lines: Vec<&str> = drawn.lines().collect();
        let top = lines
            .iter()
            .position(|line| line.contains("Snooze until"))
            .expect("a box");
        let row = lines
            .iter()
            .position(|line| line.contains('▌'))
            .expect("the row");
        assert!(top < row, "above the row:\n{drawn}");
    }
}

#[cfg(test)]
mod more {
    use std::collections::BTreeSet;

    use crossterm::event::{KeyCode, KeyModifiers};
    use postio_core::Command;
    use postio_model::{AccountId, LabelId, MailboxId};

    use crate::app::{Effect, Input, update};
    use crate::test_support::{
        app, click, folder, hits_of, key, label, open_list, press, screen, seed_places, serve,
        type_text,
    };

    fn opened() -> crate::app::App {
        let mut app = app((120, 36));
        let mut contents = crate::test_support::places();
        contents.folders.push(folder(
            6,
            "Receipts",
            postio_model::mailbox::MailboxRole::Regular,
            0,
        ));
        contents.folders.push(folder(
            7,
            "Projects",
            postio_model::mailbox::MailboxRole::Regular,
            0,
        ));
        seed_places(&mut app, contents);
        let opening = open_list(&mut app, 3);
        serve(&mut app, opening);
        app
    }

    fn labels_arrive(app: &mut crate::app::App) {
        let mut atlas = label(7, "Atlas");
        atlas.color = Some("#3584e4".into());
        update(
            app,
            Input::LabelPicker {
                account: AccountId::new(1),
                labels: vec![atlas, label(8, "Harbor")],
                counts: vec![(LabelId::new(7), 12), (LabelId::new(8), 3)],
                applied: BTreeSet::from([LabelId::new(8)]),
            },
        );
    }

    fn commands(effects: &[Effect]) -> Vec<Command> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Send(command) => Some(command.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn l_lists_the_labels_with_what_is_applied_and_space_toggles_one() {
        let mut app = opened();
        let effects = update(&mut app, press('l'));
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::ReadLabelPicker { account, .. } if *account == AccountId::new(1))),
            "{effects:?}"
        );
        labels_arrive(&mut app);
        let drawn = screen(120, 36, &app);
        assert!(drawn.contains("Labels"), "{drawn}");
        assert!(drawn.contains("Filter, or type a new label"), "{drawn}");
        let line = |name: &str| {
            drawn
                .lines()
                .skip(3)
                .find(|line| line.contains(name))
                .unwrap()
                .to_owned()
        };
        assert!(
            line("Atlas").contains('●') && line("Atlas").contains("12"),
            "{}",
            line("Atlas")
        );
        assert!(line("Harbor").contains("✓ applied"), "{}", line("Harbor"));
        assert!(drawn.contains("toggles a label"), "{drawn}");

        let effects = update(&mut app, press(' '));
        assert!(
            commands(&effects).iter().any(|command| matches!(
                command,
                Command::AddLabel { label: Some(id), on: Some(true), .. } if *id == LabelId::new(7)
            )),
            "{effects:?}"
        );
        assert!(screen(120, 36, &app).contains("Labels"), "it stays open");
        let drawn = screen(120, 36, &app);
        assert!(
            drawn
                .lines()
                .any(|l| l.contains("Atlas") && l.contains("✓ applied")),
            "{drawn}"
        );
        // Toggling again takes it off.
        let effects = update(&mut app, press(' '));
        assert!(commands(&effects).iter().any(|command| matches!(
            command,
            Command::AddLabel {
                on: Some(false),
                ..
            }
        )));
    }

    #[test]
    fn typing_filters_and_a_name_nobody_has_offers_to_make_it() {
        let mut app = opened();
        update(&mut app, press('l'));
        labels_arrive(&mut app);
        type_text(&mut app, "rec");
        let drawn = screen(120, 36, &app);
        assert!(drawn.contains("Create label “rec”"), "{drawn}");
        assert!(!drawn.contains("Atlas"), "{drawn}");
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::CreateLabel {
                account: AccountId::new(1),
                name: "rec".into(),
                close: true
            }),
            "{effects:?}"
        );
        let made = update(
            &mut app,
            Input::LabelMade {
                label: Some(label(9, "rec")),
                close: true,
            },
        );
        assert!(
            commands(&made).iter().any(|command| matches!(
                command,
                Command::AddLabel { label: Some(id), on: Some(true), .. } if *id == LabelId::new(9)
            )),
            "{made:?}"
        );
        assert!(!screen(120, 36, &app).contains("Labels"), "closed");
    }

    #[test]
    fn m_lists_recent_then_all_folders_and_a_number_moves_to_a_recent_one() {
        let mut app = opened();
        let effects = update(&mut app, press('m'));
        assert!(effects.contains(&Effect::ReadRecentMoves), "{effects:?}");
        update(
            &mut app,
            Input::RecentMoves(vec![MailboxId::new(7), MailboxId::new(6)]),
        );
        let drawn = screen(120, 36, &app);
        assert!(drawn.contains("Move to folder"), "{drawn}");
        let at = |text: &str| drawn.lines().position(|line| line.contains(text)).unwrap();
        assert!(
            at("Recent") < at("Projects") && at("Projects") < at("All folders"),
            "{drawn}"
        );
        assert!(
            drawn
                .lines()
                .any(|l| l.contains('1') && l.contains("Projects")),
            "{drawn}"
        );
        let effects = update(&mut app, press('2'));
        assert!(
            commands(&effects).iter().any(|command| matches!(
                command,
                Command::Move { to: Some(to), .. } if *to == MailboxId::new(6)
            )),
            "{effects:?}"
        );
        assert!(
            effects.contains(&Effect::NoteMove(MailboxId::new(6))),
            "{effects:?}"
        );
    }

    #[test]
    fn a_filtered_folder_moves_on_enter_and_a_move_does_not_go_there() {
        let mut app = opened();
        update(&mut app, press('m'));
        update(&mut app, Input::RecentMoves(Vec::new()));
        type_text(&mut app, "arch");
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(commands(&effects).iter().any(|command| matches!(
            command,
            Command::Move { to: Some(to), .. } if *to == MailboxId::new(2)
        )));
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::Open(_)))
        );
    }

    #[test]
    fn a_click_chooses_a_row_and_a_click_outside_does_nothing() {
        let mut app = opened();
        update(&mut app, press('s'));
        let hits = hits_of(120, 36, &app);
        let drawn = screen(120, 36, &app);
        let row = drawn
            .lines()
            .position(|line| line.contains("Tomorrow morning"))
            .unwrap();
        let at = hits.at(30, u16::try_from(row).unwrap()).expect("a preset");
        let outside = hits.at(2, 20);
        if let Some(outside) = outside {
            let effects = update(&mut app, click(outside.target, false, false));
            assert!(effects.is_empty());
        }
        let effects = update(&mut app, click(at.target, false, false));
        assert!(
            commands(&effects)
                .iter()
                .any(|c| matches!(c, Command::Snooze { until: Some(_), .. }))
        );
    }

    #[test]
    fn the_composer_can_open_the_remind_picker_for_its_draft() {
        let mut app = opened();
        app.compose(postio_model::Draft::new(AccountId::new(1)));
        update(&mut app, press('x'));
        let effects = app.open_remind_picker_for_draft("Tide gate");
        assert!(!effects.is_empty());
        let drawn = screen(120, 36, &app);
        assert!(drawn.contains("Remind me if no one replies by"), "{drawn}");
        assert!(drawn.contains("Tide gate"), "{drawn}");
        let effects = update(&mut app, press('1'));
        assert!(
            commands(&effects).is_empty(),
            "nothing is sent for mail: {effects:?}"
        );
        assert!(app.draft_remind().is_some());
        assert_eq!(app.focus(), crate::app::Focus::Composer, "back to writing");
        app.clear_draft_remind();
        assert!(app.draft_remind().is_none());
    }
}
