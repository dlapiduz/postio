//! What Focus says about its own state: the one banner row under the strip
//! and the empty inbox in the list's place (terminal.md, "States").
//!
//! Which banner and what each says are `postio_ui::focus_state`'s; this
//! draws them. Every state keeps a mark that is not a colour: an error's
//! heading is led by `✕` and bold, and the first sync's bar is `━` over
//! `─`.

use postio_core::CommandId;
use postio_ui::focus_state::{Banner, BannerAction, EmptyInbox};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::theme::{Role, Theme};
use crate::view::bottom::cap;
use crate::view::fit;
use crate::view::hit::{Hits, Target};

/// The first sync's bar is this many cells.
const BAR: usize = 16;

/// The command a banner's button runs.
fn command_of(action: BannerAction) -> CommandId {
    match action {
        BannerAction::Retry => CommandId::Refresh,
        BannerAction::UpdatePassword => CommandId::UpdateCredential,
    }
}

/// `━` for what is done and `─` for what is left, `fraction` of `cells`.
fn bar(fraction: f64, cells: usize) -> String {
    let done = ((fraction * cells as f64).round() as usize).min(cells);
    format!("{}{}", "━".repeat(done), "─".repeat(cells - done))
}

/// Draw `banner` into `area`, one row on the surface background: its
/// heading in bold, its sentence, and on the right its button with the key
/// that does the same -- or, for a first sync, how far it has come.
pub fn banner(
    frame: &mut Frame,
    area: Rect,
    banner: &Banner,
    app: &App,
    theme: &Theme,
    hits: &mut Hits,
) {
    if theme.style(Role::Surface).bg.is_some() {
        frame
            .buffer_mut()
            .set_style(area, theme.style(Role::Surface));
    }
    let width = usize::from(area.width);
    let heading_style = if banner.is_error() {
        theme.style(Role::Error)
    } else {
        theme.style(Role::Text)
    }
    .add_modifier(Modifier::BOLD);
    // Without colour an error is still marked.
    let heading = if banner.is_error() {
        format!("✕ {}", banner.heading())
    } else {
        banner.heading()
    };

    // What stands on the right, which never gives way.
    let mut right: Vec<Span> = Vec::new();
    let mut button = None;
    if let Some((label, action)) = banner.action() {
        let command = command_of(action);
        right.push(Span::styled(label, theme.style(Role::Accent)));
        // Signing in again has a key only in the accounts; Retry's is the
        // list's own.
        if action == BannerAction::Retry
            && let Some(key) = app.hint(command)
        {
            right.push(Span::styled(
                format!(" {}", cap(&key)),
                theme.style(Role::Dim),
            ));
        }
        button = Some(command);
    }
    if let Banner::FirstSync { done, total } = banner {
        let said = postio_ui::focus_state::syncing(*done, *total);
        right.push(Span::styled(format!("{said} "), theme.style(Role::Dim)));
        right.push(Span::styled(
            bar(banner.progress().unwrap_or(0.0), BAR),
            theme.style(Role::Accent),
        ));
    }
    let right_width: usize = right.iter().map(|span| span.content.width()).sum();

    let room = width.saturating_sub(2 + right_width + if right_width > 0 { 2 } else { 0 });
    let heading_cut = fit(&heading, room);
    let sentence_room = room.saturating_sub(heading_cut.width() + 2);
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(heading_cut.clone(), heading_style),
    ];
    if sentence_room >= 8 {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            fit(&banner.sentence(), sentence_room),
            theme.style(Role::Text),
        ));
    }
    let used: usize = spans.iter().map(|span| span.content.width()).sum();
    frame.render_widget(Line::from(spans), area);
    debug_assert!(used <= width);
    if right_width > 0 {
        let x = area.x + u16::try_from(width.saturating_sub(right_width + 1)).unwrap_or(0);
        let w = u16::try_from(right_width).unwrap_or(0);
        frame.render_widget(Line::from(right), Rect::new(x, area.y, w, 1));
        if let Some(command) = button {
            hits.add(
                Rect::new(x, area.y, w, 1),
                Target::Command(command.as_str()),
            );
        }
    }
}

/// Draw `said` centred in `area`: its heading in bold, the line under it,
/// when the next digest comes, and the shortcuts on one line.
pub fn empty(frame: &mut Frame, area: Rect, said: &EmptyInbox, theme: &Theme, hits: &mut Hits) {
    let centred = |text_width: usize| {
        area.x + u16::try_from(usize::from(area.width).saturating_sub(text_width) / 2).unwrap_or(0)
    };
    let mut lines: Vec<(Vec<Span>, usize)> = Vec::new();
    let single = |text: &str, style| {
        let cut = fit(text, usize::from(area.width).saturating_sub(2));
        let w = cut.width();
        (vec![Span::styled(cut, style)], w)
    };
    lines.push(single(
        &said.heading,
        theme.style(Role::Text).add_modifier(Modifier::BOLD),
    ));
    for line in said.detail.iter().chain(said.next_digest.iter()) {
        lines.push(single(line, theme.style(Role::Dim)));
    }
    let height = lines.len() + 2;
    let top =
        area.y + u16::try_from(usize::from(area.height).saturating_sub(height) / 3).unwrap_or(0);
    let mut y = top;
    for (spans, w) in lines {
        if y >= area.y + area.height {
            return;
        }
        frame.render_widget(
            Line::from(spans),
            Rect::new(centred(w), y, u16::try_from(w).unwrap_or(0), 1),
        );
        y += 1;
    }
    y += 1;
    if y >= area.y + area.height {
        return;
    }
    // The shortcuts, each its words and its key, on one line.
    let mut pieces: Vec<(Vec<Span>, usize, CommandId)> = Vec::new();
    for (key, what, command) in &said.shortcuts {
        let mut label = what.clone();
        if *command == CommandId::GoToFiltered {
            "Filtered".clone_into(&mut label);
        }
        let label = capitalised(&label);
        let mut spans = vec![Span::styled(label.clone(), theme.style(Role::Text))];
        let mut w = label.width();
        if let Some(key) = key {
            let key = cap(key);
            w += 1 + key.width();
            spans.push(Span::styled(format!(" {key}"), theme.style(Role::Dim)));
        }
        pieces.push((spans, w, *command));
    }
    const GAP: usize = 3;
    let total: usize =
        pieces.iter().map(|(_, w, _)| w).sum::<usize>() + GAP * pieces.len().saturating_sub(1);
    let mut x = centred(total);
    for (spans, w, command) in pieces {
        let w = u16::try_from(w).unwrap_or(0);
        if x + w > area.x + area.width {
            break;
        }
        frame.render_widget(Line::from(spans), Rect::new(x, y, w, 1));
        hits.add(Rect::new(x, y, w, 1), Target::Command(command.as_str()));
        x += w + GAP as u16;
    }
}

/// `text` with its first letter capital.
fn capitalised(text: &str) -> String {
    let mut letters = text.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use postio_core::{ConnectionState, Event, FailureReason};

    use crate::app::{App, Effect, Input, Pointer, update};
    use crate::test_support::{
        app, conversation, hits_of, local, places, screen, seed_places, show_focus,
    };
    use postio_ui::focus_list::FocusRow;

    const ACCOUNT: postio_model::AccountId = postio_model::AccountId::new(1);

    fn with_account(size: (u16, u16)) -> App {
        let mut app = app(size);
        let mut contents = places();
        contents.accounts[0].incoming.host = "imap.example.com".into();
        seed_places(&mut app, contents);
        show_focus(
            &mut app,
            vec![FocusRow::conversation(conversation(
                1,
                "Ada Moreno",
                "Hello",
                "",
                local(23, 9, 0),
            ))],
        );
        app
    }

    fn heard(app: &mut App, event: Event) {
        update(app, Input::Host(event));
    }

    /// The account's connection is up, as it is once mail is flowing.
    fn online(app: &mut App) {
        heard(
            app,
            Event::ConnectionChanged {
                account: ACCOUNT,
                state: ConnectionState::Online,
            },
        );
    }

    fn click_on(app: &mut App, size: (u16, u16), word: &str) -> Vec<Effect> {
        let drawn = screen(size.0, size.1, app);
        // The lowest line that says it: the top bar says Compose too.
        let lines: Vec<&str> = drawn.lines().collect();
        let (y, line) = lines
            .iter()
            .enumerate()
            .rfind(|(_, line)| line.contains(word))
            .unwrap_or_else(|| panic!("no {word}:\n{drawn}"));
        let x = u16::try_from(line[..line.find(word).unwrap()].chars().count()).unwrap();
        let hit = hits_of(size.0, size.1, app)
            .at(x, u16::try_from(y).unwrap())
            .expect("a target");
        update(
            app,
            Input::Pointer(Pointer::Click {
                hit,
                ctrl: false,
                shift: false,
            }),
        )
    }

    #[test]
    fn no_banner_while_nothing_is_wrong() {
        let app = with_account((120, 24));
        let drawn = screen(120, 24, &app);
        assert!(
            drawn.lines().nth(2).unwrap().contains("Today"),
            "the list starts under the strip:\n{drawn}"
        );
    }

    #[test]
    fn offline_is_one_banner_row_under_the_strip_with_retry_and_its_key() {
        let mut app = with_account((120, 24));
        heard(
            &mut app,
            Event::ConnectionChanged {
                account: ACCOUNT,
                state: ConnectionState::Offline,
            },
        );
        let drawn = screen(120, 24, &app);
        let row = drawn.lines().nth(2).unwrap();
        assert!(row.contains("You're offline"), "{drawn}");
        assert!(row.contains("syncs when you're back"), "{drawn}");
        assert!(row.trim_end().ends_with("Retry now F5"), "{drawn}");
        assert!(
            drawn.lines().nth(3).unwrap().contains("Today"),
            "the list gave the row up:\n{drawn}"
        );
        let effects = click_on(&mut app, (120, 24), "Retry now");
        assert!(
            effects.contains(&Effect::Send(postio_core::Command::Refresh)),
            "{effects:?}"
        );
    }

    #[test]
    fn a_refused_password_names_the_server_and_marks_itself_without_colour() {
        let mut app = with_account((120, 24));
        heard(
            &mut app,
            Event::ConnectionChanged {
                account: ACCOUNT,
                state: ConnectionState::Failing {
                    reason: FailureReason::Auth,
                },
            },
        );
        let drawn = screen(120, 24, &app);
        let row = drawn.lines().nth(2).unwrap();
        assert!(
            row.trim_start()
                .starts_with("✕ Can't sign in to imap.example.com"),
            "{drawn}"
        );
        assert!(row.trim_end().ends_with("Update password…"), "{drawn}");
        click_on(&mut app, (120, 24), "Update password…");
        assert!(
            app.first_run().is_some_and(|run| run.repairing()),
            "the credential flow opens for the account"
        );
    }

    #[test]
    fn the_first_sync_says_how_far_it_has_come_with_a_bar() {
        let mut app = with_account((120, 24));
        online(&mut app);
        heard(
            &mut app,
            Event::SyncProgress {
                account: ACCOUNT,
                done: 3,
                total: 9,
            },
        );
        let drawn = screen(120, 24, &app);
        let row = drawn.lines().nth(2).unwrap();
        assert!(row.contains("First sync"), "{drawn}");
        assert!(row.contains("Syncing 3 of 9"), "{drawn}");
        let filled = row.chars().filter(|c| *c == '━').count();
        let rest = row.chars().filter(|c| *c == '─').count();
        assert!(filled > 0 && rest > 0 && filled < rest, "a third: {row}");
    }

    #[test]
    fn a_narrow_banner_keeps_its_heading_and_its_action() {
        let mut app = with_account((60, 14));
        heard(
            &mut app,
            Event::ConnectionChanged {
                account: ACCOUNT,
                state: ConnectionState::Offline,
            },
        );
        let drawn = screen(60, 14, &app);
        let row = drawn.lines().nth(2).unwrap();
        assert!(row.contains("You're offline"), "{drawn}");
        assert!(row.trim_end().ends_with("Retry now F5"), "{drawn}");
    }

    fn empty_inbox(size: (u16, u16)) -> App {
        let mut app = app(size);
        seed_places(&mut app, places());
        show_focus(&mut app, Vec::new());
        online(&mut app);
        heard(
            &mut app,
            Event::SyncProgress {
                account: ACCOUNT,
                done: 9,
                total: 9,
            },
        );
        app
    }

    #[test]
    fn an_empty_inbox_says_so_centred_with_its_shortcuts_on_one_line() {
        let app = empty_inbox((120, 30));
        let drawn = screen(120, 30, &app);
        let heading = drawn
            .lines()
            .enumerate()
            .find(|(_, line)| line.contains("Inbox is empty"))
            .unwrap_or_else(|| panic!("{drawn}"));
        let at = heading.1.find("Inbox is empty").unwrap();
        let middle = (at + "Inbox is empty".len() / 2) as i32;
        assert!((middle - 60).abs() <= 2, "centred: {drawn}");
        let line = drawn
            .lines()
            .rfind(|line| line.contains("Compose c"))
            .unwrap_or_else(|| panic!("{drawn}"));
        assert!(line.contains("Archive g r"), "{drawn}");
        assert!(line.find("Archive g r") < line.find("Compose c"), "{drawn}");
        assert!(drawn.contains("Synced "), "when it last synced:\n{drawn}");
    }

    #[test]
    fn an_empty_inbox_before_a_sync_has_finished_says_what_it_waits_on() {
        let mut app = app((120, 30));
        seed_places(&mut app, places());
        show_focus(&mut app, Vec::new());
        online(&mut app);
        heard(
            &mut app,
            Event::SyncProgress {
                account: ACCOUNT,
                done: 3,
                total: 9,
            },
        );
        let drawn = screen(120, 30, &app);
        assert!(drawn.contains("Syncing your inbox…"), "{drawn}");
        assert!(!drawn.contains("Inbox is empty"), "{drawn}");
    }

    #[test]
    fn the_empty_block_is_for_the_inbox_with_no_conversations_only() {
        // With a conversation the list is shown.
        let app = with_account((120, 30));
        assert!(!screen(120, 30, &app).contains("Inbox is empty"));
        // The has-action filter finding nothing is not an empty inbox.
        let mut app = empty_inbox((120, 30));
        crate::test_support::show_scope(&mut app, postio_model::FocusScope::HasAction, Vec::new());
        assert!(!screen(120, 30, &app).contains("Inbox is empty"));
        // Nor is another folder.
        let mut app = empty_inbox((120, 30));
        let effects = crate::test_support::open_list(&mut app, 0);
        crate::test_support::serve(&mut app, effects);
        assert!(!screen(120, 30, &app).contains("Inbox is empty"));
    }

    #[test]
    fn a_shortcut_is_a_click_for_its_command() {
        let mut app = empty_inbox((120, 30));
        let effects = click_on(&mut app, (120, 30), "Compose");
        assert!(app.composer().is_some(), "compose opened: {effects:?}");
    }
}
