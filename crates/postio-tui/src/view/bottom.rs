//! The bottom line: the bulk bar while anything is selected; a toast takes the right end.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, Tone};
use crate::theme::{Role, Theme};
use crate::view::fit;
use crate::view::hit::{Hits, Target};
use postio_core::CommandId;

/// A key as a cap is spelled on a screen: the registry's `Escape` is `Esc`.
pub fn cap(key: &str) -> String {
    if key == "+" {
        return key.to_owned();
    }
    key.split('+')
        .map(|piece| match piece {
            "Escape" => "Esc",
            "Delete" => "Del",
            "Return" => "↵",
            other => other,
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// `✓ Archived 3 messages · Undo z`, or `✕` and a failure's words.
pub struct Toast {
    pub spans: Vec<Span<'static>>,
    pub undo: Option<std::ops::Range<usize>>,
    pub offer: Option<std::ops::Range<usize>>,
}

impl Toast {
    pub fn width(&self) -> usize {
        self.spans.iter().map(|span| span.content.width()).sum()
    }
}

pub fn toast(app: &App, theme: &Theme) -> Option<Toast> {
    let notice = app.notice()?;
    let mut spans = Vec::new();
    match app.notice_tone() {
        Tone::Failed => spans.push(Span::styled("✕ ", theme.style(Role::Error))),
        Tone::Worked => spans.push(Span::styled("✓ ", theme.style(Role::Success))),
        Tone::Plain => {}
    }
    let text = match app.notice_tone() {
        Tone::Failed => theme.style(Role::Error),
        _ => theme.style(Role::Text),
    };
    spans.push(Span::styled(notice.to_owned(), text));
    let mut undo = None;
    if let Some(key) = app.notice_undo() {
        spans.push(Span::styled(" · ", theme.style(Role::Dim)));
        let from: usize = spans.iter().map(|span| span.content.width()).sum();
        spans.push(Span::styled("Undo ", theme.style(Role::Dim)));
        spans.push(Span::styled(cap(key), theme.style(Role::Accent)));
        let to: usize = spans.iter().map(|span| span.content.width()).sum();
        undo = Some(from..to);
    }
    let mut offer = None;
    if let Some(label) = app.notice_offer() {
        spans.push(Span::styled(" · ", theme.style(Role::Dim)));
        let from: usize = spans.iter().map(|span| span.content.width()).sum();
        spans.push(Span::styled(label.to_owned(), theme.style(Role::Accent)));
        if let Some(key) = app.hint(CommandId::ClearListMail) {
            spans.push(Span::styled(
                format!(" {}", cap(&key)),
                theme.style(Role::Dim),
            ));
        }
        let to: usize = spans.iter().map(|span| span.content.width()).sum();
        offer = Some(from..to);
    }
    Some(Toast { spans, undo, offer })
}

struct Piece {
    spans: Vec<Span<'static>>,
    command: Option<&'static str>,
}

impl Piece {
    fn width(&self) -> usize {
        self.spans.iter().map(|span| span.content.width()).sum()
    }
}

fn verb(app: &App, theme: &Theme, command: CommandId, label: &str) -> Piece {
    let mut spans = vec![Span::styled(label.to_owned(), theme.style(Role::Text))];
    if let Some(key) = app.hint(command) {
        spans.push(Span::styled(
            format!(" {}", cap(&key)),
            theme.style(Role::Dim),
        ));
    }
    Piece {
        spans,
        command: Some(command.as_str()),
    }
}

fn selection_keys(app: &App, theme: &Theme) -> Vec<(Span<'static>, Option<&'static str>)> {
    let dim = theme.style(Role::Dim);
    let mut spans: Vec<(Span<'static>, Option<&'static str>)> = Vec::new();
    let part = |spans: &mut Vec<(Span<'static>, Option<&'static str>)>,
                pieces: Vec<(String, Option<&'static str>)>| {
        if !spans.is_empty() {
            spans.push((Span::styled("  ", dim), None));
        }
        spans.extend(
            pieces
                .into_iter()
                .map(|(text, command)| (Span::styled(text, dim), command)),
        );
    };
    if let Some(key) = app.hint(CommandId::ToggleSelection) {
        part(
            &mut spans,
            vec![(
                format!("{} toggle", cap(&key)),
                Some(CommandId::ToggleSelection.as_str()),
            )],
        );
    }
    let extend: Vec<(String, Option<&'static str>)> =
        [CommandId::ExtendSelectionDown, CommandId::ExtendSelectionUp]
            .into_iter()
            .filter_map(|command| Some((cap(&app.hint(command)?), Some(command.as_str()))))
            .collect();
    if !extend.is_empty() {
        let mut pieces = Vec::new();
        for (index, (key, command)) in extend.into_iter().enumerate() {
            if index > 0 {
                pieces.push((" ".to_owned(), None));
            }
            pieces.push((key, command));
        }
        pieces.push((" extend".to_owned(), None));
        part(&mut spans, pieces);
    }
    if let Some(key) = app.hint(CommandId::Back) {
        part(
            &mut spans,
            vec![(
                format!("{} clear", cap(&key)),
                Some(CommandId::Back.as_str()),
            )],
        );
    }
    spans
}

pub fn bulk_bar(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, hits: &mut Hits) -> bool {
    let Some(summary) = app.selection_summary() else {
        return false;
    };
    let width = usize::from(area.width);
    let mut left = vec![Piece {
        spans: vec![Span::styled(
            summary,
            theme.style(Role::Text).add_modifier(Modifier::BOLD),
        )],
        command: None,
    }];
    left.extend(
        postio_ui::focus_dialog::BULK
            .iter()
            .map(|item| verb(app, theme, item.command, item.label)),
    );
    let toast = toast(app, theme);
    let mut right: Vec<(Span<'static>, Option<&'static str>)> = match &toast {
        Some(toast) => toast
            .spans
            .iter()
            .map(|span| (span.clone(), None))
            .collect(),
        None => selection_keys(app, theme),
    };
    let right_width = |spans: &[(Span, Option<&'static str>)]| {
        spans
            .iter()
            .map(|(span, _)| span.content.width())
            .sum::<usize>()
    };
    // Narrowing: the keys go first, then the last verbs; a toast keeps its
    // end of the line and gives up verbs before it is cut.
    let is_toast = toast.is_some();
    let used = |left: &[Piece], right: &[(Span, Option<&'static str>)]| {
        1 + left.iter().map(Piece::width).sum::<usize>()
            + GAP * left.len().saturating_sub(1)
            + if right.is_empty() {
                0
            } else {
                GAP + right_width(right)
            }
            + 1
    };
    if !is_toast && used(&left, &right) > width {
        right.clear();
    }
    while left.len() > 2 && used(&left, &right) > width {
        left.pop();
    }
    if used(&left, &right) > width && is_toast {
        let room = width.saturating_sub(used(&left, &[]) + GAP);
        let words: String = right
            .iter()
            .map(|(span, _)| span.content.as_ref())
            .collect();
        right = vec![(
            Span::styled(fit(&words, room), theme.style(Role::Text)),
            None,
        )];
    }
    let toast_whole = toast
        .as_ref()
        .is_some_and(|toast| toast.width() == right_width(&right));
    let mut x = area.x + 1;
    for piece in left {
        let w = u16::try_from(piece.width()).unwrap_or(u16::MAX);
        if x + w > area.x + area.width {
            break;
        }
        frame.render_widget(Line::from(piece.spans), Rect::new(x, area.y, w, 1));
        if let Some(command) = piece.command {
            hits.add(Rect::new(x, area.y, w, 1), Target::Command(command));
        }
        x += w + GAP as u16;
    }
    let w = u16::try_from(right_width(&right)).unwrap_or(0);
    if w > 0 {
        let at = (area.x + area.width).saturating_sub(w + 1);
        let mut x = at;
        for (span, command) in &right {
            let width = u16::try_from(span.content.width()).unwrap_or(0);
            if let Some(command) = command {
                hits.add(Rect::new(x, area.y, width, 1), Target::Command(command));
            }
            x += width;
        }
        frame.render_widget(
            Line::from(right.into_iter().map(|(span, _)| span).collect::<Vec<_>>()),
            Rect::new(at, area.y, w, 1),
        );
        if let Some(toast) = toast.filter(|_| toast_whole) {
            toast_targets(hits, at, area.y, toast.undo.as_ref(), toast.offer.as_ref());
        }
    }
    true
}

pub fn toast_targets(
    hits: &mut Hits,
    x: u16,
    y: u16,
    undo: Option<&std::ops::Range<usize>>,
    offer: Option<&std::ops::Range<usize>>,
) {
    let region = |range: &std::ops::Range<usize>| {
        Rect::new(
            x + u16::try_from(range.start).unwrap_or(0),
            y,
            u16::try_from(range.len()).unwrap_or(0),
            1,
        )
    };
    if let Some(undo) = undo {
        hits.add(region(undo), Target::Command(CommandId::Undo.as_str()));
    }
    if let Some(offer) = offer {
        hits.add(region(offer), Target::NoticeOffer);
    }
}

const GAP: usize = 2;

#[cfg(test)]
mod tests {
    use postio_ui::focus_list::FocusRow;

    use crate::app::{App, update};
    use crate::test_support::{
        app, conversation, hits_of, local, places, press, screen, seed_places, show_focus,
    };

    fn inbox(size: (u16, u16)) -> App {
        let mut app = app(size);
        seed_places(&mut app, places());
        let rows = (1..=6)
            .map(|id| {
                FocusRow::conversation(conversation(
                    id,
                    "Ada Moreno",
                    &format!("Subject {id}"),
                    "",
                    local(23, 9, id as u32),
                ))
            })
            .collect();
        show_focus(&mut app, rows);
        app
    }

    fn bottom(drawn: &str) -> &str {
        drawn.lines().last().expect("a screen")
    }

    #[test]
    fn nothing_selected_means_no_bulk_bar() {
        let drawn = screen(120, 24, &inbox((120, 24)));
        assert!(!bottom(&drawn).contains("selected"), "{drawn}");
    }

    #[test]
    fn escape_clears_the_selection_and_the_bar_with_it() {
        let mut app = inbox((140, 24));
        update(&mut app, press('x'));
        update(
            &mut app,
            crate::test_support::key(
                crossterm::event::KeyCode::Esc,
                crossterm::event::KeyModifiers::NONE,
            ),
        );
        let drawn = screen(140, 24, &app);
        assert!(!bottom(&drawn).contains("selected"), "{drawn}");
    }

    #[test]
    fn the_unsubscribe_notice_draws_its_one_action_and_a_click_takes_it_up() {
        let mut app = inbox((120, 24));
        app.confirm_unsubscribed_for_test("news.example.com", &[7, 8]);
        let drawn = screen(120, 24, &app);
        let line = bottom(&drawn);
        assert!(
            line.contains("Unsubscribed from news.example.com") && line.contains("Archive 2 C"),
            "{drawn}"
        );
        let x = u16::try_from(line[..line.find("Archive 2").unwrap()].chars().count()).unwrap();
        let hit = hits_of(120, 24, &app)
            .at(x, 23)
            .expect("the button is a target");
        let effects = update(
            &mut app,
            crate::app::Input::Pointer(crate::app::Pointer::Click {
                hit,
                ctrl: false,
                shift: false,
            }),
        );
        assert!(
            effects.iter().any(|effect| matches!(
                effect,
                crate::app::Effect::Send(postio_core::Command::Archive { .. })
            )),
            "{effects:?}"
        );
    }
}
