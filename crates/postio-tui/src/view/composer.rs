//! The composer, in the reading pane (US3).

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use postio_ui::terminal::SafeText;

use crate::composer::{Composer, Field};
use crate::theme::{Role, Theme};
use crate::view::fit;
use crate::view::hit::{Hits, Target};

/// The width of the field labels, so the values line up.
const LABEL: u16 = 9;

/// The composer in `area`: its three header rows -- Detach, the title and
/// close; what will be sent and what has happened to the draft; the verbs --
/// then a hairline, and under it the fields and the body.
pub fn screen(
    frame: &mut Frame,
    area: Rect,
    app: &crate::app::App,
    theme: &Theme,
    now: chrono::DateTime<chrono::Local>,
    hits: &mut Hits,
) {
    use crate::view::bottom::cap;
    use unicode_width::UnicodeWidthStr;
    let Some(composer) = app.composer() else {
        return;
    };
    if area.width < 20 || area.height < 8 {
        return;
    }
    let dim = theme.style(Role::Dim);
    let text = theme.style(Role::Text);
    let inner = usize::from(area.width);
    let click = |hits: &mut Hits, x: u16, y: u16, width: usize, id: &'static str| {
        hits.add(
            Rect::new(x, y, u16::try_from(width).unwrap_or(0), 1),
            Target::ComposerAction(id),
        );
    };

    // Row 1: Detach, the title, Esc ✕.
    let detach = match app.hint(postio_core::CommandId::DetachComposer) {
        Some(key) => format!("Detach {}", cap(&key)),
        None => "Detach".to_owned(),
    };
    frame.render_widget(
        Line::styled(detach.clone(), dim),
        Rect::new(area.x + 1, area.y, area.width - 1, 1),
    );
    click(hits, area.x + 1, area.y, detach.width(), "detach_composer");
    let close = match app.hint(postio_core::CommandId::Back) {
        Some(key) => format!("{} ✕", cap(&key)),
        None => "✕".to_owned(),
    };
    let close_x = area.x + u16::try_from(inner.saturating_sub(close.width() + 1)).unwrap_or(0);
    frame.render_widget(
        Line::styled(close.clone(), text),
        Rect::new(
            close_x,
            area.y,
            u16::try_from(close.width()).unwrap_or(0),
            1,
        ),
    );
    click(hits, close_x, area.y, close.width(), "back");
    let title = postio_ui::compose::title(composer.kind());
    let at = area.x + u16::try_from(inner.saturating_sub(title.width()) / 2).unwrap_or(0);
    frame.render_widget(
        Line::styled(title, text.add_modifier(Modifier::BOLD)),
        Rect::new(at, area.y, u16::try_from(title.width()).unwrap_or(0), 1),
    );

    // Row 2: what will be sent, and what has happened to the draft.
    let words = composer.words();
    let summary = format!(
        "Markdown · {}",
        if words == 1 {
            "1 word".to_owned()
        } else {
            format!("{words} words")
        }
    );
    let subtitle = postio_ui::compose::subtitle(&summary, &app.saved_note().unwrap_or_default());
    let subtitle = fit(&subtitle, inner.saturating_sub(2));
    let at = area.x + u16::try_from(inner.saturating_sub(subtitle.width()) / 2).unwrap_or(0);
    frame.render_widget(
        Line::styled(subtitle.clone(), dim),
        Rect::new(
            at,
            area.y + 1,
            u16::try_from(subtitle.width()).unwrap_or(0),
            1,
        ),
    );

    // Row 3: the verbs, the first the one a message is for.
    let remind_key = app
        .keymap()
        .bindings(postio_core::CommandId::RemindIfNoReply)
        .iter()
        .find(|binding| binding.contains('+'))
        .map(|binding| cap(binding));
    let verbs: Vec<(String, Option<String>, &'static str)> = vec![
        (
            "Send".to_owned(),
            app.hint(postio_core::CommandId::Send),
            "send",
        ),
        (
            "Send later".to_owned(),
            app.hint(postio_core::CommandId::ScheduleSend),
            "schedule_send",
        ),
        (
            "Attach".to_owned(),
            app.hint(postio_core::CommandId::AttachFile),
            "attach_file",
        ),
        (
            postio_ui::compose::remind_words(composer.remind_at()),
            remind_key,
            "remind_if_no_reply",
        ),
    ];
    let mut x = area.x + 1;
    let end = area.x + area.width;
    for (index, (word, key, id)) in verbs.into_iter().enumerate() {
        let key = key.map(|key| cap(&key));
        let shown = match &key {
            Some(key) => format!("{word} {key}"),
            None => word.clone(),
        };
        let w = u16::try_from(shown.width()).unwrap_or(u16::MAX);
        if x + w > end {
            break;
        }
        let primary = |style: ratatui::style::Style| {
            if index == 0 {
                style.add_modifier(Modifier::BOLD)
            } else {
                style
            }
        };
        let mut spans = vec![Span::styled(word.clone(), primary(text))];
        if let Some(key) = key {
            spans.push(Span::styled(format!(" {key}"), primary(dim)));
        }
        frame.render_widget(Line::from(spans), Rect::new(x, area.y + 2, w, 1));
        click(hits, x, area.y + 2, shown.width(), id);
        x += w + 2;
    }
    frame.render_widget(
        Line::styled("─".repeat(inner), dim),
        Rect::new(area.x, area.y + 3, area.width, 1),
    );

    // The fields and the body.
    let body = Rect::new(area.x, area.y + 4, area.width, area.height - 4);
    draw(
        frame,
        body,
        composer,
        app.preview_shown(),
        app.focus() == crate::app::Focus::Composer
            && app.scheduling().is_none()
            && app.reminding().is_none()
            && app.path_prompt().is_none(),
        &[],
        theme,
        hits,
    );
    if let Some(times) = app.scheduling() {
        draw_schedule(frame, body, "Send later", times, theme, now);
    }
    if let Some(times) = app.reminding() {
        draw_schedule(
            frame,
            body,
            postio_ui::pickers::REMIND_TITLE,
            times,
            theme,
            now,
        );
    }
    if let Some(typed) = app.path_prompt() {
        draw_path_prompt(frame, body, typed, theme);
    }
}

/// Draw `composer` into `area`; with `focused`, the terminal's cursor goes
/// where the next letter will land.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    composer: &Composer,
    preview: Option<postio_config::Preview>,
    focused: bool,
    actions: &[Action],
    theme: &Theme,
    hits: &mut Hits,
) {
    let area = Rect::new(
        area.x + 1,
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    let mut fields = Vec::new();
    if composer.shows_identities() {
        fields.push((Field::From, "From"));
    }
    fields.push((Field::To, "To"));
    if composer.shows_extra_recipients() {
        fields.push((Field::Cc, "Cc"));
        fields.push((Field::Bcc, "Bcc"));
    }
    fields.push((Field::Subject, "Subject"));

    let value_width = area.width.saturating_sub(LABEL);
    let mut y = area.y;
    for (field, label) in fields {
        if y >= area.y + area.height {
            return;
        }
        let here = composer.field() == field;
        let label_style = if here {
            theme.style(Role::Accent).add_modifier(Modifier::BOLD)
        } else {
            theme.style(Role::Dim)
        };
        // The fields hold only what `Composer::new` sanitized and what was
        // typed; typing cannot produce a control character.
        let value = fit(composer.value(field), usize::from(value_width));
        let line = Line::from(vec![
            Span::styled(
                format!("{label:<width$}", width = usize::from(LABEL)),
                label_style,
            ),
            Span::styled(value, theme.style(Role::Text)),
        ]);
        frame.render_widget(line, Rect::new(area.x, y, area.width, 1));
        hits.add(
            Rect::new(area.x, y, area.width, 1),
            Target::ComposerField(field),
        );
        if focused && here {
            let column = u16::try_from(composer.cursor_in(field)).unwrap_or(u16::MAX);
            frame.set_cursor_position(Position::new(area.x + LABEL + column.min(value_width), y));
        }
        y += 1;
        if here {
            y = draw_suggestions(frame, area, y, composer, theme);
        }
    }
    // A rule between the headers and the body, as a sent message has.
    if y < area.y + area.height {
        frame.render_widget(
            Line::styled("─".repeat(usize::from(area.width)), theme.style(Role::Dim)),
            Rect::new(area.x, y, area.width, 1),
        );
        y += 1;
    }
    let mut height = (area.y + area.height).saturating_sub(y);
    // The buttons along the foot, as the desktop's composer has them: what
    // each does, and the key this terminal can send for it.
    if !actions.is_empty() && height > 2 {
        height -= 1;
        draw_actions(frame, area.x, y + height, area.width, actions, theme, hits);
    }
    // The draft's files, under the body: each by name and size, the way the
    // desktop lists them. A forwarded file's name is the sender's.
    let files = composer.attachments();
    let shown = files.len().min(ATTACHMENTS);
    let mut listed: Vec<Line> = files[..shown]
        .iter()
        .map(|file| {
            let name = SafeText::new(file.display_name());
            Line::from(vec![
                Span::styled("📎 ", theme.style(Role::Dim)),
                Span::styled(name.as_str().to_owned(), theme.style(Role::Text)),
                Span::styled(
                    format!("  {}", postio_ui::format::human_size(file.size)),
                    theme.style(Role::Dim),
                ),
            ])
        })
        .collect();
    if files.len() > shown {
        listed.push(Line::styled(
            format!("   and {} more", files.len() - shown),
            theme.style(Role::Dim),
        ));
    }
    let needed = u16::try_from(listed.len()).unwrap_or(u16::MAX);
    if needed > 0 && height > needed {
        height -= needed;
        for (offset, line) in listed.into_iter().enumerate() {
            let row = y + height + u16::try_from(offset).unwrap_or(u16::MAX);
            frame.render_widget(line, Rect::new(area.x, row, area.width, 1));
        }
    }
    // A reply's quote, folded to one line under the body: it is sent, it is
    // not edited here, and it would otherwise push what is being written off
    // the screen.
    let quoted = composer.quote_lines();
    if quoted > 0 && height > 1 {
        height -= 1;
        let summary = match quoted {
            1 => "▸ Quoted message, 1 line".to_owned(),
            lines => format!("▸ Quoted message, {lines} lines"),
        };
        frame.render_widget(
            Line::styled(
                fit(&summary, usize::from(area.width)),
                theme.style(Role::Quote),
            ),
            Rect::new(area.x, y + height, area.width, 1),
        );
    }
    if height == 0 {
        return;
    }
    let body = Rect::new(area.x, y, area.width, height);
    hits.add(body, Target::ComposerBody);
    // Where the textarea will have scrolled to, for where a click lands.
    composer.body_top(height);
    match preview {
        None => frame.render_widget(composer.body(), body),
        Some(postio_config::Preview::Toggle) => draw_preview(frame, body, composer),
        Some(postio_config::Preview::Split) => {
            let half = body.width / 2;
            frame.render_widget(
                composer.body(),
                Rect::new(body.x, body.y, half, body.height),
            );
            for row in body.y..body.y + body.height {
                frame.render_widget(
                    Line::styled("│", theme.style(Role::Dim)),
                    Rect::new(body.x + half, row, 1, 1),
                );
            }
            let right = Rect::new(
                body.x + half + 2,
                body.y,
                body.width.saturating_sub(half + 2),
                body.height,
            );
            draw_preview(frame, right, composer);
        }
    }
}

/// The message as it will arrive, from its top, in `area`.
fn draw_preview(frame: &mut Frame, area: Rect, composer: &Composer) {
    let lines = composer.preview().lines();
    for (offset, line) in lines.into_iter().take(usize::from(area.height)).enumerate() {
        let row = area.y + u16::try_from(offset).unwrap_or(u16::MAX);
        frame.render_widget(line, Rect::new(area.x, row, area.width, 1));
    }
}

/// How many of a draft's files are listed by name before the rest are
/// counted.
const ATTACHMENTS: usize = 4;

/// How many suggestions are shown at once.
const SUGGESTIONS: usize = 5;

/// Recipient suggestions under the field being typed in, from row `y`;
/// answers the row after them.
fn draw_suggestions(
    frame: &mut Frame,
    area: Rect,
    mut y: u16,
    composer: &Composer,
    theme: &Theme,
) -> u16 {
    let width = usize::from(area.width.saturating_sub(LABEL));
    for (index, candidate) in composer.suggestions().iter().take(SUGGESTIONS).enumerate() {
        if y >= area.y + area.height {
            break;
        }
        let chosen = index == composer.suggestion();
        // A contact's name is whatever a sender's header said.
        let label = SafeText::new(&postio_ui::recipients::candidate_label(candidate));
        let (mark, role) = if chosen {
            ("› ", Role::Surface)
        } else {
            ("  ", Role::Dim)
        };
        frame.render_widget(
            Line::styled(fit(&format!("{mark}{label}"), width), theme.style(role)),
            Rect::new(area.x + LABEL, y, area.width.saturating_sub(LABEL), 1),
        );
        y += 1;
    }
    y
}

/// The schedule-send picker, over the bottom of the composer: the four
/// One of the composer's buttons: its key as this terminal sends it, what it
/// says, and the command it runs.
pub type Action = (String, &'static str, &'static str);

/// The buttons, left to right: the first is the one a message is for, and
/// is filled; the rest are words with their keys.
fn draw_actions(
    frame: &mut Frame,
    x: u16,
    y: u16,
    width: u16,
    actions: &[Action],
    theme: &Theme,
    hits: &mut Hits,
) {
    let mut at = x;
    let end = x + width;
    for (index, (key, word, id)) in actions.iter().enumerate() {
        let label = format!(" {word} {key} ");
        let wide = u16::try_from(unicode_width::UnicodeWidthStr::width(label.as_str()))
            .unwrap_or(u16::MAX);
        if at + wide > end {
            break;
        }
        let line = if index == 0 {
            let filled = theme.style(Role::Surface).add_modifier(Modifier::BOLD);
            Line::from(vec![
                Span::styled(format!(" {word} "), filled.patch(theme.style(Role::Accent))),
                Span::styled(format!("{key} "), filled),
            ])
        } else {
            Line::from(vec![
                Span::styled(format!(" {word} "), theme.style(Role::Text)),
                Span::styled(format!("{key} "), theme.style(Role::Dim)),
            ])
        };
        let rect = Rect::new(at, y, wide, 1);
        frame.render_widget(line, rect);
        hits.add(rect, Target::ComposerAction(id));
        at += wide + 1;
    }
}

/// times, each with the number that picks it and when that is.
pub fn draw_schedule(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    times: &[(&'static str, chrono::DateTime<chrono::Local>)],
    theme: &Theme,
    now: chrono::DateTime<chrono::Local>,
) {
    let area = Rect::new(
        area.x + 2,
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    let wanted = u16::try_from(times.len() + 1).unwrap_or(u16::MAX);
    if area.height < wanted {
        return;
    }
    let top = area.y + area.height - wanted;
    let width = usize::from(area.width);
    let mut lines = vec![Line::styled(
        fit(&format!("{title} — a number picks, Esc goes back"), width),
        theme.style(Role::Accent).add_modifier(Modifier::BOLD),
    )];
    for (index, (label, when)) in times.iter().enumerate() {
        // The day only when it is not today: "18:00" this evening, "Tue
        // 08:00" otherwise.
        let at = if when.date_naive() == now.date_naive() {
            when.format("%H:%M").to_string()
        } else {
            when.format("%a %H:%M").to_string()
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", index + 1), theme.style(Role::Accent)),
            Span::styled(format!("{label:<18}"), theme.style(Role::Text)),
            Span::styled(at, theme.style(Role::Dim)),
        ]));
    }
    for (offset, line) in lines.into_iter().enumerate() {
        let y = top + u16::try_from(offset).unwrap_or(u16::MAX);
        let row = Rect::new(area.x, y, area.width, 1);
        frame.render_widget(ratatui::widgets::Clear, row);
        frame.render_widget(line, row);
    }
}

/// The path prompt (FR-027), over the last row of the composer, with the
/// terminal's cursor at its end.
pub fn draw_path_prompt(frame: &mut Frame, area: Rect, typed: &str, theme: &Theme) {
    let area = Rect::new(
        area.x + 2,
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    if area.height == 0 {
        return;
    }
    let row = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
    // What was typed or pasted here is the person's own, but a paste can
    // carry anything.
    let typed = SafeText::new(typed);
    let label = "Attach: ";
    let line = Line::from(vec![
        Span::styled(
            label,
            theme.style(Role::Accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            fit(
                typed.as_str(),
                usize::from(area.width).saturating_sub(label.len()),
            ),
            theme.style(Role::Text),
        ),
    ]);
    frame.render_widget(ratatui::widgets::Clear, row);
    frame.render_widget(line, row);
    let column = unicode_width::UnicodeWidthStr::width(typed.as_str()) + label.len();
    frame.set_cursor_position(Position::new(
        row.x
            + u16::try_from(column)
                .unwrap_or(u16::MAX)
                .min(row.width.saturating_sub(1)),
        row.y,
    ));
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};
    use postio_model::{AccountId, DraftKind, EmailAddress, MessageId};
    use postio_ui::focus_list::FocusRow;

    use crate::app::{App, Effect, Front, Input, Pointer, update};
    use crate::test_support::{
        alt, app, conversation, ctrl, hits_of, key, local, places, press, screen, seed_places,
        show_focus, type_text, unread,
    };

    fn inbox(size: (u16, u16)) -> App {
        let mut app = app(size);
        seed_places(&mut app, places());
        show_focus(
            &mut app,
            vec![FocusRow::conversation(unread(conversation(
                1,
                "Lena Park",
                "Harbor API draft v3",
                "",
                local(23, 11, 22),
            )))],
        );
        app
    }

    fn composing(size: (u16, u16), kind: DraftKind) -> App {
        let mut app = inbox(size);
        let mut draft = postio_model::Draft::new(AccountId::new(1));
        draft.kind = kind;
        draft.to = vec![EmailAddress::new(Some("Lena Park"), "lena@example.com")];
        draft.subject = "Re: Harbor API draft v3".into();
        app.compose(draft);
        app
    }

    fn lines_of(drawn: &str) -> Vec<&str> {
        drawn.lines().collect()
    }

    #[test]
    fn the_composer_is_the_frame_with_detach_the_title_and_close_then_the_subtitle_then_the_verbs()
    {
        let app = composing((120, 36), DraftKind::Reply);
        let drawn = screen(120, 36, &app);
        let lines = lines_of(&drawn);
        let top: Vec<char> = lines[1].chars().collect();
        assert_eq!((top[14], top[105]), ('╭', '╮'), "{drawn}");
        let detach = app.hint(postio_core::CommandId::DetachComposer).unwrap();
        assert!(
            lines[2].contains(&format!("Detach {}", crate::view::bottom::cap(&detach))),
            "{drawn}"
        );
        assert!(lines[2].contains("Reply"), "{drawn}");
        assert!(lines[2].contains("Esc ✕"), "{drawn}");
        assert!(lines[3].contains("Markdown · 0 words"), "{drawn}");
        let verbs = lines[4];
        for wanted in ["Send ", "Send later", "Attach", "Remind"] {
            assert!(verbs.contains(wanted), "{wanted}:\n{drawn}");
        }
        assert!(
            verbs.find("Send ").unwrap() < verbs.find("Send later").unwrap(),
            "{drawn}"
        );
        // The fields and the body are inside, under the hairline.
        let tee: Vec<char> = lines[5].chars().collect();
        assert_eq!((tee[14], tee[105]), ('├', '┤'), "{drawn}");
        assert!(
            lines[6].contains("To") && lines[6].contains("lena@example.com"),
            "{drawn}"
        );
        assert!(drawn.contains("Re: Harbor API draft v3"), "{drawn}");
        let titles = [
            (DraftKind::New, "New message"),
            (DraftKind::ReplyAll, "Reply to all"),
            (DraftKind::Forward, "Forward"),
        ];
        for (kind, title) in titles {
            let drawn = screen(120, 36, &composing((120, 36), kind));
            assert!(lines_of(&drawn)[2].contains(title), "{title}:\n{drawn}");
        }
    }

    #[test]
    fn the_subtitle_counts_the_words_and_says_when_the_draft_was_saved() {
        let mut app = composing((120, 36), DraftKind::Reply);
        // Into the body, then two words.
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        type_text(&mut app, "two words");
        let drawn = screen(120, 36, &app);
        assert!(
            lines_of(&drawn)[3].contains("Markdown · 2 words"),
            "{drawn}"
        );
        assert!(!lines_of(&drawn)[3].contains("saved"), "{drawn}");
        update(
            &mut app,
            Input::DraftSaved {
                generation: 1,
                saved: Ok(postio_model::DraftId::new(4)),
            },
        );
        let drawn = screen(120, 36, &app);
        assert!(
            lines_of(&drawn)[3].contains("Markdown · 2 words · Draft saved locally "),
            "{drawn}"
        );
    }

    #[test]
    fn the_list_behind_the_composer_is_dimmed() {
        use ratatui::style::Modifier;
        let app = composing((120, 36), DraftKind::New);
        let buffer = crate::test_support::buffer(120, 36, &app);
        assert!(buffer[(2, 4)].modifier.contains(Modifier::DIM));
    }

    #[test]
    fn detach_gives_the_composer_the_whole_screen_between_the_bars_and_detach_or_escape_brings_it_back()
     {
        let mut app = composing((120, 36), DraftKind::Reply);
        let hit = hits_of(120, 36, &app);
        let drawn = screen(120, 36, &app);
        let (y, line) = lines_of(&drawn)
            .into_iter()
            .enumerate()
            .find(|(_, line)| line.contains("Detach"))
            .unwrap();
        let x = u16::try_from(line[..line.find("Detach").unwrap()].chars().count()).unwrap();
        let target = hit.at(x, u16::try_from(y).unwrap()).expect("a click");
        update(
            &mut app,
            Input::Pointer(Pointer::Click {
                hit: target,
                ctrl: false,
                shift: false,
            }),
        );
        assert!(app.composer_detached());
        let drawn = screen(120, 36, &app);
        let lines = lines_of(&drawn);
        assert!(!drawn.contains('╭'), "no frame:\n{drawn}");
        assert!(lines[0].contains("Compose"), "the top bar stays:\n{drawn}");
        assert!(
            lines[1].contains("Detach") && lines[1].contains("Reply"),
            "{drawn}"
        );
        assert!(!lines[1].contains("Inbox"), "the strip gave way:\n{drawn}");
        // Escape brings it back to the frame, the draft still being written.
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.composer_detached());
        assert_eq!(app.front(), Front::Composer);
        assert!(screen(120, 36, &app).contains('╭'));
        // And the same key detaches again, and again brings it back.
        assert_eq!(
            app.hint(postio_core::CommandId::DetachComposer).as_deref(),
            Some("alt+o")
        );
        update(&mut app, alt('o'));
        assert!(app.composer_detached());
        update(&mut app, alt('o'));
        assert!(!app.composer_detached());
    }

    #[test]
    fn remind_asks_when_and_the_chosen_day_is_on_the_verb_and_goes_with_the_send() {
        let mut app = composing((120, 36), DraftKind::Reply);
        let remind = app
            .keymap()
            .bindings(postio_core::CommandId::RemindIfNoReply)
            .iter()
            .find(|binding| binding.contains('+'))
            .expect("a chord for it")
            .clone();
        assert_eq!(remind, "ctrl+h", "{remind}");
        update(&mut app, ctrl('h'));
        let drawn = screen(120, 36, &app);
        for (index, (name, _)) in postio_ui::schedule::remind_presets(chrono::Local::now())
            .iter()
            .enumerate()
        {
            assert!(drawn.contains(&format!("{} {name}", index + 1)), "{drawn}");
        }
        update(&mut app, press('2'));
        let at = app.composer().unwrap().remind_at().expect("chosen");
        let drawn = screen(120, 36, &app);
        let said = postio_ui::compose::remind_words(Some(at));
        assert!(lines_of(&drawn)[4].contains(&said), "{said}:\n{drawn}");
        let effects = update(&mut app, key(KeyCode::Char('s'), KeyModifiers::ALT));
        let sent = effects.iter().find_map(|effect| match effect {
            Effect::QueueSend { draft, .. } => Some(draft.remind_at),
            _ => None,
        });
        assert_eq!(sent, Some(Some(at)), "{effects:?}");
    }

    #[test]
    fn each_verb_in_the_header_is_a_click_for_its_command() {
        let mut app = composing((120, 36), DraftKind::Reply);
        let drawn = screen(120, 36, &app);
        let (y, line) = lines_of(&drawn)
            .into_iter()
            .enumerate()
            .find(|(_, line)| line.contains("Attach"))
            .unwrap();
        let x = u16::try_from(line[..line.find("Attach").unwrap()].chars().count()).unwrap();
        let hit = hits_of(120, 36, &app)
            .at(x, u16::try_from(y).unwrap())
            .expect("a target");
        update(
            &mut app,
            Input::Pointer(Pointer::Click {
                hit,
                ctrl: false,
                shift: false,
            }),
        );
        assert!(app.path_prompt().is_some(), "Attach asks for a path");
        // Close: the draft goes where an Escape takes it.
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        let (y, line) = lines_of(&screen(120, 36, &app))
            .into_iter()
            .map(str::to_owned)
            .enumerate()
            .find(|(_, line)| line.contains("Esc ✕"))
            .unwrap();
        let x = u16::try_from(
            line.find("Esc ✕")
                .map(|at| line[..at].chars().count())
                .unwrap(),
        )
        .unwrap();
        let hit = hits_of(120, 36, &app)
            .at(x, u16::try_from(y).unwrap())
            .expect("a target");
        update(
            &mut app,
            Input::Pointer(Pointer::Click {
                hit,
                ctrl: false,
                shift: false,
            }),
        );
        assert!(app.composer().is_none());
        assert_eq!(app.front(), Front::List);
    }

    #[test]
    fn replying_from_the_open_message_writes_in_the_frame_and_escape_goes_back_to_the_message() {
        let mut app = inbox((120, 36));
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        update(
            &mut app,
            Input::Body {
                message: MessageId::new(1),
                answer: Ok(postio_client::protocol::Body::Ready {
                    body: postio_model::MessageBody {
                        text: Some("Hello".into()),
                        html: None,
                    },
                    encoding_problems: false,
                }),
            },
        );
        for (key_char, kind) in [
            ('e', postio_body::replying::ReplyKind::Reply),
            ('E', postio_body::replying::ReplyKind::ReplyAll),
            ('f', postio_body::replying::ReplyKind::Forward),
        ] {
            let effects = update(&mut app, press(key_char));
            assert!(
                effects.contains(&Effect::ReplySource {
                    kind,
                    message: MessageId::new(1)
                }),
                "{effects:?}"
            );
        }
        let found = crate::composer::tests::a_message_and_its_account();
        app.compose(postio_body::replying::reply_draft(
            postio_body::replying::ReplyKind::Reply,
            &found.0,
            &found.1,
        ));
        assert_eq!(app.front(), Front::Composer);
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(
            app.front(),
            Front::Reader,
            "back to the message it answered"
        );
        assert!(screen(120, 36, &app).contains("Hello"));
    }
}
