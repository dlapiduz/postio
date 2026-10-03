//! The bottom line: the bulk bar, and the words the toast says.
//!
//! While anything is selected the line is the bulk bar: how many, the verbs
//! that act on them, each with the key the keymap gives it, and on the right
//! the keys that change the selection. A toast, while there is one, takes
//! the right end in place of those keys. Every verb is a click for the
//! command its key runs.

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
    match key {
        "Escape" => "Esc".to_owned(),
        "Delete" => "Del".to_owned(),
        "Return" => "↵".to_owned(),
        other => other.to_owned(),
    }
}

/// The toast as it is drawn: `✓ Archived 3 messages · Undo ctrl+z`, or
/// `✕` and the words of a failure, and where Undo is within it.
pub struct Toast {
    /// The mark, the words and the undo offer.
    pub spans: Vec<Span<'static>>,
    /// The columns, within the spans, that a click undoes from.
    pub undo: Option<std::ops::Range<usize>>,
}

impl Toast {
    /// How wide it is.
    pub fn width(&self) -> usize {
        self.spans.iter().map(|span| span.content.width()).sum()
    }
}

/// What the toast says, or `None` when there is none.
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
    Some(Toast { spans, undo })
}

/// One thing on the bar: its spans and the command a click on it runs.
struct Piece {
    spans: Vec<Span<'static>>,
    command: Option<&'static str>,
}

impl Piece {
    fn width(&self) -> usize {
        self.spans.iter().map(|span| span.content.width()).sum()
    }
}

/// A verb: its words, then its key when the keymap gives one.
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

/// The keys that change the selection, as the keymap gives them:
/// `x toggle  J K extend  Esc clear`.
fn selection_keys(app: &App, theme: &Theme) -> Vec<Span<'static>> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(key) = app.hint(CommandId::ToggleSelection) {
        parts.push(format!("{} toggle", cap(&key)));
    }
    let extend: Vec<String> = [CommandId::ExtendSelectionDown, CommandId::ExtendSelectionUp]
        .into_iter()
        .filter_map(|command| app.hint(command))
        .map(|key| cap(&key))
        .collect();
    if !extend.is_empty() {
        parts.push(format!("{} extend", extend.join(" ")));
    }
    if let Some(key) = app.hint(CommandId::Back) {
        parts.push(format!("{} clear", cap(&key)));
    }
    vec![Span::styled(parts.join("  "), theme.style(Role::Dim))]
}

/// Draw the bulk bar into `area` and answer whether there was one to draw:
/// only while something is selected.
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
    let mut right = match &toast {
        Some(toast) => toast.spans.clone(),
        None => selection_keys(app, theme),
    };
    let right_width = |spans: &[Span]| spans.iter().map(|span| span.content.width()).sum::<usize>();
    // Narrowing: the keys go first, then the last verbs; a toast keeps its
    // end of the line and gives up verbs before it is cut.
    let is_toast = toast.is_some();
    let used = |left: &[Piece], right: &[Span]| {
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
        let words: String = right.iter().map(|span| span.content.as_ref()).collect();
        right = vec![Span::styled(fit(&words, room), theme.style(Role::Text))];
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
        frame.render_widget(Line::from(right), Rect::new(at, area.y, w, 1));
        if let Some(undo) = toast.filter(|_| toast_whole).and_then(|toast| toast.undo) {
            toast_target(hits, at, area.y, &undo);
        }
    }
    true
}

/// Undo is a click for the command its key runs.
pub fn toast_target(hits: &mut Hits, x: u16, y: u16, undo: &std::ops::Range<usize>) {
    let from = x + u16::try_from(undo.start).unwrap_or(0);
    let width = u16::try_from(undo.len()).unwrap_or(0);
    hits.add(
        Rect::new(from, y, width, 1),
        Target::Command(CommandId::Undo.as_str()),
    );
}

/// The gap between two things on the bar.
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
    fn x_and_j_k_select_and_the_bar_counts_them_and_offers_the_verbs_with_their_keys() {
        let mut app = inbox((140, 24));
        update(&mut app, press('x'));
        update(&mut app, press('J'));
        let drawn = screen(140, 24, &app);
        let line = bottom(&drawn);
        assert!(line.trim_start().starts_with("2 selected"), "{drawn}");
        for wanted in [
            "Archive a",
            "Snooze s",
            "Mark read r",
            "Digest these… d",
            "Label l",
            "Move m",
            "Delete Del",
            "x toggle  J K extend  Esc clear",
        ] {
            assert!(line.contains(wanted), "{wanted} missing:\n{drawn}");
        }
        assert!(
            line.trim_end().ends_with("Esc clear"),
            "the selection keys are on the right:\n{drawn}"
        );
    }

    #[test]
    fn the_bars_keys_follow_a_rebinding() {
        let mut bindings = postio_config::KeyBindings::default();
        bindings
            .overrides_mut()
            .insert("archive".to_owned(), "z".to_owned());
        let mut app = crate::test_support::app_with_keys((140, 24), &bindings);
        seed_places(&mut app, places());
        show_focus(
            &mut app,
            vec![FocusRow::conversation(conversation(
                1,
                "Ada",
                "Hello",
                "",
                local(23, 9, 0),
            ))],
        );
        update(&mut app, press('x'));
        let drawn = screen(140, 24, &app);
        assert!(bottom(&drawn).contains("Archive z"), "{drawn}");
    }

    #[test]
    fn a_click_on_a_verb_runs_it_on_the_selection() {
        let mut app = inbox((140, 24));
        update(&mut app, press('x'));
        update(&mut app, press('j'));
        update(&mut app, press('x'));
        let drawn = screen(140, 24, &app);
        let line = bottom(&drawn);
        let x = u16::try_from(line[..line.find("Archive").unwrap()].chars().count()).unwrap();
        let hit = hits_of(140, 24, &app).at(x, 23).expect("a verb is there");
        let effects = update(
            &mut app,
            crate::app::Input::Pointer(crate::app::Pointer::Click {
                hit,
                ctrl: false,
                shift: false,
            }),
        );
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, crate::app::Effect::Send(_))),
            "{effects:?}"
        );
        assert_eq!(
            app.state()
                .read(|state| state.resolve(&postio_core::MessageTarget::Selection)),
            Some(postio_core::Resolved::Messages(vec![
                postio_model::MessageId::new(1),
                postio_model::MessageId::new(2)
            ])),
            "the verb is about the two selected"
        );
        assert!(app.selection().is_empty(), "what was acted on is let go of");
    }

    #[test]
    fn x_with_shift_selects_all_as_a_predicate_and_the_bar_counts_the_list() {
        let mut app = inbox((140, 24));
        update(&mut app, press('X'));
        let drawn = screen(140, 24, &app);
        assert!(
            bottom(&drawn).trim_start().starts_with("6 selected"),
            "{drawn}"
        );
        let effects = update(&mut app, press('a'));
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, crate::app::Effect::Send(_))),
            "{effects:?}"
        );
        assert!(
            matches!(
                app.state()
                    .read(|state| state.resolve(&postio_core::MessageTarget::Selection)),
                Some(postio_core::Resolved::Everything { .. })
            ),
            "a predicate, never the rows on screen"
        );
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
    fn a_toast_takes_the_right_end_in_place_of_the_selection_keys() {
        let mut app = inbox((140, 24));
        update(&mut app, press('x'));
        update(
            &mut app,
            crate::app::Input::Host(postio_core::Event::ActionCompleted {
                description: "Archived 3 messages".into(),
                undoable: true,
            }),
        );
        let drawn = screen(140, 24, &app);
        let line = bottom(&drawn);
        assert!(line.contains("1 selected"), "{drawn}");
        assert!(line.contains("Archived 3 messages"), "{drawn}");
        assert!(!line.contains("J K extend"), "{drawn}");
    }

    fn done(app: &mut App, description: &str, undoable: bool) {
        update(
            app,
            crate::app::Input::Host(postio_core::Event::ActionCompleted {
                description: description.into(),
                undoable,
            }),
        );
    }

    #[test]
    fn a_toast_is_a_mark_the_words_and_the_undo_key_and_says_so_without_colour() {
        let mut app = inbox((120, 24));
        done(&mut app, "Archived 3 messages", true);
        let drawn = screen(120, 24, &app);
        assert_eq!(
            bottom(&drawn).trim_end(),
            " ✓ Archived 3 messages · Undo ctrl+z",
            "{drawn}"
        );
        done(&mut app, "Label added", false);
        let drawn = screen(120, 24, &app);
        assert_eq!(bottom(&drawn).trim_end(), " ✓ Label added", "{drawn}");
        update(
            &mut app,
            crate::app::Input::Host(postio_core::Event::CommandRejected {
                command: postio_core::CommandId::Archive.into(),
                reason: "Nothing to archive".into(),
            }),
        );
        let drawn = screen(120, 24, &app);
        assert_eq!(
            bottom(&drawn).trim_end(),
            " ✕ Nothing to archive",
            "{drawn}"
        );
    }

    #[test]
    fn a_click_on_the_toasts_undo_undoes() {
        let mut app = inbox((120, 24));
        done(&mut app, "Archived 3 messages", true);
        let drawn = screen(120, 24, &app);
        let line = bottom(&drawn);
        let x = u16::try_from(line[..line.find("Undo").unwrap()].chars().count()).unwrap();
        let hit = hits_of(120, 24, &app).at(x, 23).expect("Undo is a target");
        let effects = update(
            &mut app,
            crate::app::Input::Pointer(crate::app::Pointer::Click {
                hit,
                ctrl: false,
                shift: false,
            }),
        );
        assert!(
            effects.contains(&crate::app::Effect::Send(postio_core::Command::Undo)),
            "{effects:?}"
        );
    }

    #[test]
    fn beside_the_bulk_bar_the_toast_keeps_its_undo_at_the_right_end() {
        let mut app = inbox((140, 24));
        update(&mut app, press('x'));
        done(&mut app, "Archived 3 messages", true);
        let drawn = screen(140, 24, &app);
        let line = bottom(&drawn);
        assert!(line.contains("1 selected"), "{drawn}");
        assert!(
            line.trim_end()
                .ends_with("✓ Archived 3 messages · Undo ctrl+z"),
            "{drawn}"
        );
    }

    #[test]
    fn narrowing_drops_the_selection_keys_then_the_last_verbs_and_keeps_the_count() {
        let mut app = inbox((60, 14));
        update(&mut app, press('x'));
        let drawn = screen(60, 14, &app);
        let line = bottom(&drawn);
        assert!(line.contains("1 selected"), "{drawn}");
        assert!(line.contains("Archive a"), "{drawn}");
        assert!(!line.contains("Esc clear"), "{drawn}");
        assert!(!line.contains("Delete"), "{drawn}");
    }
}
