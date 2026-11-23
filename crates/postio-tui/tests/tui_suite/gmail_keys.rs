use crossterm::event::{KeyCode, KeyModifiers};
use postio_core::registry;
use postio_model::{EmailAddress, MessageBody, MessageId};
use postio_tui::app::{App, Effect, Focus, Input, update};
use postio_tui::test_support::{app, key, open_list, open_message, places, seed_places, serve};
use postio_ui::keymap::{Binding, Chord, Key, Modifiers};

/// Legacy terminals set SHIFT on a capital and leave a shifted symbol bare; the kitty protocol sets
/// SHIFT on both.
fn deliveries(chord: &Chord) -> Vec<Input> {
    let mut modifiers = KeyModifiers::NONE;
    for (keymap, terminal) in [
        (Modifiers::CTRL, KeyModifiers::CONTROL),
        (Modifiers::ALT, KeyModifiers::ALT),
        (Modifiers::SHIFT, KeyModifiers::SHIFT),
    ] {
        if chord.modifiers.contains(keymap) {
            modifiers |= terminal;
        }
    }
    let code = match chord.key {
        Key::Char(character) => KeyCode::Char(character),
        Key::Named("Return") => KeyCode::Enter,
        Key::Named("Escape") => KeyCode::Esc,
        Key::Named("Delete") => KeyCode::Delete,
        Key::Named("Right") => KeyCode::Right,
        Key::Named("Left") => KeyCode::Left,
        Key::Named("Up") => KeyCode::Up,
        Key::Named("Down") => KeyCode::Down,
        Key::Named(name) => panic!("no terminal key for {name}"),
    };
    let mut all = vec![key(code, modifiers)];
    if let KeyCode::Char(character) = code
        && modifiers == KeyModifiers::NONE
        && !character.is_ascii_lowercase()
        && !character.is_ascii_digit()
    {
        all.push(key(code, KeyModifiers::SHIFT));
    }
    if let KeyCode::Char(character) = code
        && modifiers == KeyModifiers::SHIFT
        && character.is_ascii_uppercase()
    {
        all.push(key(code, KeyModifiers::NONE));
    }
    all
}

fn in_list() -> App {
    let mut app = app((120, 30));
    seed_places(&mut app, places());
    let opening = open_list(&mut app, 10);
    serve(&mut app, opening);
    app
}

fn in_reader() -> App {
    let mut app = in_list();
    open_message(
        &mut app,
        MessageId::new(1),
        vec![EmailAddress::new(None::<String>, "ada@example.com")],
        MessageBody {
            text: Some("Hello".into()),
            html: None,
        },
    );
    assert_eq!(app.focus(), Focus::Reader, "the keyboard is in the message");
    app
}

fn press_binding(app: &mut App, binding: &str, delivery: usize) -> Vec<Effect> {
    let parsed: Binding = binding.parse().expect("a binding");
    let mut effects = Vec::new();
    for chord in parsed.chords() {
        let all = deliveries(chord);
        let input = all
            .into_iter()
            .nth(delivery)
            .or_else(|| deliveries(chord).into_iter().next());
        effects.extend(update(app, input.expect("an input")));
    }
    effects
}

fn how_many_deliveries(binding: &str) -> usize {
    let parsed: Binding = binding.parse().expect("a binding");
    parsed
        .chords()
        .iter()
        .map(|chord| deliveries(chord).len())
        .max()
        .unwrap_or(1)
}

fn asks(effects: &[Effect], needle: &str) -> bool {
    effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::Send(_)
                | Effect::Focus {
                    request: postio_focus::Request::Send { .. } | postio_focus::Request::Post(_),
                    ..
                }
        ) && format!("{effect:?}").contains(needle)
    })
}

fn opens(effects: &[Effect], needle: &str) -> bool {
    effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::Focus {
                request: postio_focus::Request::OpenScope { .. },
                ..
            }
        ) && format!("{effect:?}").contains(needle)
    })
}

const VERBS: &[(&str, &str)] = &[
    ("archive", "Archive"),
    ("delete", "Delete"),
    ("flag", "Flag"),
    ("toggle_read", "ToggleRead"),
    ("undo", "Undo"),
];

fn bindings_of(id: &str) -> Vec<&'static str> {
    registry::all()
        .find(|spec| spec.id.as_str() == id)
        .unwrap_or_else(|| panic!("no command {id}"))
        .bindings()
        .filter(|binding| {
            // Plain keys only: the chorded alternates (`mod+z`, `alt+Down`) are what a terminal
            // sends as ctrl or alt, and undo has its own test.
            !binding.contains('+')
        })
        .collect()
}

#[test]
fn every_message_verb_takes_gmails_key_and_its_alternates_in_the_list_and_the_message() {
    let mut failures = Vec::new();
    for (id, effect) in VERBS {
        for binding in bindings_of(id) {
            for delivery in 0..how_many_deliveries(binding) {
                for (surface, make) in [
                    ("list", in_list as fn() -> App),
                    ("message", in_reader as fn() -> App),
                ] {
                    let mut app = make();
                    let effects = press_binding(&mut app, binding, delivery);
                    if !asks(&effects, effect) {
                        failures.push(format!(
                            "{id}: `{binding}` (delivery {delivery}) in the {surface}: {effects:?}"
                        ));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn goes(app: &App, effects: &[Effect], id: &str) -> bool {
    let opened = |needle: &str| opens(effects, needle);
    match id {
        "go_to_inbox" => opened("Focus(Inbox)"),
        "go_to_sent" | "go_to_drafts" | "go_to_archive" | "go_to_trash" => opened("Mailbox("),
        "go_to_junk" => app.notice().is_some_and(|said| said.contains("Junk")),
        "go_to_flagged" => opened("Flagged("),
        "go_to_snoozed" => opened("Snoozed("),
        "go_to_outbox" => opened("Outbox("),
        "go_to_filtered" => app.focus() == Focus::Filtered,
        "go_to_digest_rules" => app.focus() == Focus::Rules,
        "go_to_folders" => app.focus() == Focus::Folders,
        "next_scope" => opened("Mailbox(") || opened("Unified"),
        other => panic!("no expectation for {other}"),
    }
}

const GO_TO: &[&str] = &[
    "go_to_inbox",
    "go_to_sent",
    "go_to_drafts",
    "go_to_flagged",
    "go_to_archive",
    "go_to_snoozed",
    "go_to_outbox",
    "go_to_junk",
    "go_to_trash",
    "go_to_filtered",
    "go_to_digest_rules",
    "go_to_folders",
    "next_scope",
];

#[test]
fn the_go_to_sequences_go_where_gmail_goes_from_the_list_the_message_and_the_folders() {
    let mut failures = Vec::new();
    for state in ["list", "open", "folders"] {
        for id in GO_TO {
            if state == "folders" && *id == "go_to_folders" {
                continue;
            }
            // The registry offers the scope cycle in the sidebar and list, not in
            // `Context::Search`,
            // which is what the folders popover resolves in; there `g c` is left to the registry.
            if state == "folders" && *id == "next_scope" {
                continue;
            }
            for binding in bindings_of(id) {
                for delivery in 0..how_many_deliveries(binding) {
                    let (mut app, _) = sample::state(state, 120, 30);
                    let effects = press_binding(&mut app, binding, delivery);
                    if !goes(&app, &effects, id) {
                        failures.push(format!(
                            "{id}: `{binding}` (delivery {delivery}) in {state}: focus {:?}, {effects:?}",
                            app.focus()
                        ));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn g_i_goes_home_from_filtered_the_digest_and_the_rules() {
    let mut failures = Vec::new();
    for state in ["filtered", "digest", "digest-list", "rules"] {
        let (mut app, _) = sample::state(state, 120, 30);
        let before = app.focus();
        press_binding(&mut app, "g i", 0);
        if app.focus() == before || app.focus() != Focus::List {
            failures.push(format!(
                "`g i` in {state}: focus {before:?} to {:?}",
                app.focus()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

use postio_core::{Availability, Context, Scope};
use postio_tui::test_support::{sample, screen};

const GMAIL: &[&str] = &[
    "archive",
    "reply",
    "reply_all",
    "forward",
    "delete",
    "flag",
    "snooze",
    "move",
    "toggle_read",
    "undo",
    "open_message",
    "next_message",
    "prev_message",
    "next_in_thread",
    "prev_in_thread",
    "expand_all",
    "toggle_fold",
    "toggle_selection",
    "select_all",
    "capture_task",
    "capture_note",
    "view_source",
    "open_attachment_or_link",
    "unsubscribe",
    "go_to_inbox",
    "go_to_folders",
    "go_to_drafts",
    "go_to_sent",
    "go_to_flagged",
    "go_to_archive",
    "go_to_snoozed",
    "go_to_outbox",
    "go_to_junk",
    "go_to_trash",
    "go_to_filtered",
    "go_to_digest_rules",
    "next_scope",
];

/// Each surface as `sample::state` reaches it, with the registry contexts its keys resolve in,
/// innermost first. Text-entry surfaces take letters as letters and are not here.
const SURFACES: &[(&str, &[Context])] = &[
    ("list", &[Context::List]),
    (
        "open",
        &[Context::Reader, Context::Conversation, Context::List],
    ),
    ("filtered", &[Context::Filtered]),
    ("rules", &[Context::Filtered]),
    ("digest", &[Context::Digest]),
    ("digest-list", &[Context::Digest]),
    ("digest-email", &[Context::Digest]),
];

/// The registry offers these here but the app has nothing to act on; each is covered where it can
/// act.
fn no_effect_here(state: &str, id: &str) -> bool {
    matches!(
        (state, id),
        ("list", "open_attachment_or_link" | "unsubscribe")
            | ("open" | "digest-email", "open_message")
            | ("open", "next_in_thread" | "prev_in_thread")
    )
}

fn did_something(before: &str, after: &str, effects: &[Effect]) -> bool {
    before != after
        || effects
            .iter()
            .any(|effect| !matches!(effect, Effect::Redraw))
}

#[test]
fn every_gmail_key_does_its_command_on_every_surface_that_offers_it() {
    let terminal = Availability {
        frontend: postio_core::Frontend::Terminal,
        ..Availability::open(Scope::Account(postio_model::AccountId::new(1)))
    };
    let mut failures = Vec::new();
    let mut pressed = 0;
    for (state, chain) in SURFACES {
        for id in GMAIL {
            let spec = registry::all()
                .find(|spec| spec.id.as_str() == *id)
                .unwrap_or_else(|| panic!("no command {id}"));
            if !spec.requires.met_by(terminal) || !chain.iter().any(|c| spec.available_in(*c)) {
                continue;
            }
            if no_effect_here(state, id) {
                continue;
            }
            for binding in bindings_of(id) {
                for delivery in 0..how_many_deliveries(binding) {
                    let (mut app, _) = sample::state(state, 120, 30);
                    if *id == "prev_message" {
                        let off = if matches!(*state, "open" | "open-narrow" | "find" | "pane") {
                            "G"
                        } else {
                            "j"
                        };
                        press_binding(&mut app, off, 0);
                    }
                    let before = screen(120, 30, &app);
                    let effects = press_binding(&mut app, binding, delivery);
                    let after = screen(120, 30, &app);
                    pressed += 1;
                    if !did_something(&before, &after, &effects) {
                        failures.push(format!(
                            "{id}: `{binding}` (delivery {delivery}) in {state} did nothing"
                        ));
                    }
                }
            }
        }
    }
    assert!(pressed > 100, "only {pressed} presses: the table is empty");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn in_thread() -> App {
    use postio_tui::test_support::{conversation, enter_thread, local, show_focus};
    let mut app = in_list();
    let mut thread = conversation(1, "Ada", "Atlas", "", local(20, 9, 0));
    thread.message_count = 3;
    show_focus(
        &mut app,
        vec![postio_ui::focus_list::FocusRow::conversation(thread)],
    );
    enter_thread(&mut app, &[11, 12, 13]);
    assert_eq!(app.focus(), Focus::Reader);
    app
}

fn shown(app: &App) -> i64 {
    app.reading().expect("a message is open").shown.id.get() - 11
}

#[test]
fn n_and_p_walk_the_open_conversation_as_delivered() {
    let mut failures = Vec::new();
    for (id, step) in [("prev_in_thread", -1_isize), ("next_in_thread", 1)] {
        for binding in bindings_of(id) {
            for delivery in 0..how_many_deliveries(binding) {
                let mut app = in_thread();
                let start = if step > 0 {
                    press_binding(&mut app, "p", 0);
                    shown(&app)
                } else {
                    shown(&app)
                };
                press_binding(&mut app, binding, delivery);
                if shown(&app) as isize != start as isize + step {
                    failures.push(format!(
                        "{id}: `{binding}` (delivery {delivery}) moved {start} to {}",
                        shown(&app)
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn u_and_left_go_back_from_the_open_message_to_the_list() {
    let mut failures = Vec::new();
    for binding in bindings_of("prev_view") {
        let mut app = in_reader();
        press_binding(&mut app, binding, 0);
        if app.focus() != Focus::List || app.reading().is_some() {
            failures.push(format!(
                "`{binding}` left focus {:?}, message open: {}",
                app.focus(),
                app.reading().is_some()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_fold_key_names_the_keys_that_step_a_thread() {
    // `Z` on a message of a thread names the first keys of the walk: `]` and `[` (`n` and `p` are
    // alternates).
    let mut app = in_reader();
    press_binding(&mut app, "Z", 0);
    let said = app.notice().unwrap_or_default().to_owned();
    assert!(said.contains("; ] and [ step through"), "{said}");
}

#[test]
fn t_and_n_open_the_capture_sheet_and_switch_it_where_a_letter_is_not_typed() {
    use postio_ui::capture::Mode;
    let mut failures = Vec::new();
    for (id, mode) in [("capture_task", Mode::Task), ("capture_note", Mode::Note)] {
        for binding in bindings_of(id) {
            for delivery in 0..how_many_deliveries(binding) {
                for state in ["list", "open"] {
                    let (mut app, _) = sample_with_vault(state);
                    press_binding(&mut app, binding, delivery);
                    match app.capture() {
                        Some(sheet) if sheet.mode() == mode => {}
                        other => failures.push(format!(
                            "{id}: `{binding}` (delivery {delivery}) in {state}: {:?}",
                            other.map(|sheet| sheet.mode())
                        )),
                    }
                }
            }
        }
    }
    for (id, mode) in [("capture_note", Mode::Note), ("capture_task", Mode::Task)] {
        for binding in bindings_of(id) {
            for delivery in 0..how_many_deliveries(binding) {
                let (mut app, _) = sample_with_vault("capture");
                update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
                press_binding(&mut app, binding, delivery);
                let now = app.capture().map(|sheet| sheet.mode());
                if now != Some(mode) {
                    failures.push(format!(
                        "{id}: `{binding}` (delivery {delivery}) in the sheet: {now:?}"
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn sample_with_vault(state: &str) -> (App, postio_tui::caps::Colour) {
    let (mut app, colour) = sample::state("capture", 120, 30);
    if state != "capture" {
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::List, "the sheet is put away");
    }
    if state == "open" {
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
    }
    (app, colour)
}

#[test]
fn reply_reply_all_forward_and_the_pickers_are_what_their_gmail_keys_say() {
    use postio_focus::PickerKind as Kind;
    let mut failures = Vec::new();
    for (id, kind) in [
        ("reply", "Reply"),
        ("reply_all", "ReplyAll"),
        ("forward", "Forward"),
    ] {
        for binding in bindings_of(id) {
            for delivery in 0..how_many_deliveries(binding) {
                for (surface, make) in [
                    ("list", in_list as fn() -> App),
                    ("message", in_reader as fn() -> App),
                ] {
                    let mut app = make();
                    let effects = press_binding(&mut app, binding, delivery);
                    let wanted = effects.iter().any(|effect| {
                        matches!(effect, Effect::ReplySource { .. })
                            && format!("{effect:?}").contains(kind)
                    });
                    if !wanted {
                        failures.push(format!(
                            "{id}: `{binding}` (delivery {delivery}) in the {surface}: {effects:?}"
                        ));
                    }
                }
            }
        }
    }
    for (id, kind) in [
        ("snooze", Kind::Snooze),
        ("move", Kind::Move),
        ("add_label", Kind::Label),
    ] {
        for binding in bindings_of(id) {
            for (surface, make) in [
                ("list", in_list as fn() -> App),
                ("message", in_reader as fn() -> App),
            ] {
                let mut app = make();
                press_binding(&mut app, binding, 0);
                let opened = app.picker().map(|picker| picker.kind());
                if opened != Some(kind) {
                    failures.push(format!(
                        "{id}: `{binding}` in the {surface} opened {opened:?}, not {kind:?}"
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_verbs_filtered_and_the_rules_and_the_digest_offer_take_their_gmail_keys() {
    let mut failures = Vec::new();
    for (id, effect) in [
        ("delete", "Delete"),
        ("flag", "Flag"),
        ("toggle_read", "ToggleRead"),
    ] {
        for binding in bindings_of(id) {
            for delivery in 0..how_many_deliveries(binding) {
                let (mut app, _) = sample::state("filtered", 120, 30);
                let effects = press_binding(&mut app, binding, delivery);
                if !asks(&effects, effect) {
                    failures.push(format!(
                        "{id}: `{binding}` (delivery {delivery}) in Filtered: {effects:?}"
                    ));
                }
            }
        }
    }
    for binding in bindings_of("archive") {
        for delivery in 0..how_many_deliveries(binding) {
            let (mut app, _) = sample::state("digest", 120, 30);
            let effects = press_binding(&mut app, binding, delivery);
            if !asks(&effects, "ArchiveDigest") {
                failures.push(format!(
                    "archive: `{binding}` (delivery {delivery}) in a digest: {effects:?}"
                ));
            }
        }
    }
    for binding in bindings_of("unsubscribe") {
        for delivery in 0..how_many_deliveries(binding) {
            let (mut app, _) = sample::state("digest-list", 120, 30);
            let effects = press_binding(&mut app, binding, delivery);
            if !effects.iter().any(|e| {
                matches!(
                    e,
                    Effect::Focus {
                        request: postio_focus::Request::Unsubscribe(_),
                        ..
                    }
                )
            }) {
                failures.push(format!(
                    "unsubscribe: `{binding}` (delivery {delivery}) in a digest's list: {effects:?}"
                ));
            }
        }
    }
    for binding in bindings_of("delete") {
        for delivery in 0..how_many_deliveries(binding) {
            let (mut app, _) = sample::state("rules", 120, 30);
            let before = screen(120, 30, &app);
            press_binding(&mut app, binding, delivery);
            let after = screen(120, 30, &app);
            if before == after {
                failures.push(format!(
                    "delete: `{binding}` (delivery {delivery}) in the rules asked nothing"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn shows(line: &str, label: &str, key: &str) -> bool {
    let mut from = 0;
    while let Some(at) = line[from..].find(label) {
        let after = &line[from + at + label.len()..];
        let rest = after.trim_start_matches(' ');
        let gap = after.len() - rest.len();
        if (1..=6).contains(&gap)
            && rest.starts_with(key)
            && rest[key.len()..]
                .chars()
                .next()
                .is_none_or(|next| next == ' ' || next == '\u{b7}')
        {
            return true;
        }
        from += at + label.len();
    }
    false
}

/// What each moved command was called on screen and its key before the Gmail keymap.
const RETIRED: &[(&str, &str)] = &[
    ("Archive", "a"),
    ("Reply", "e"),
    ("Reply all", "A"),
    ("Delete", "Delete"),
    ("Flag", "*"),
    ("Snooze", "s"),
    ("Move", "m"),
    ("Undo", "ctrl+z"),
    ("Select all", "X"),
    ("Unsubscribe", "U"),
    ("Task", "t"),
    ("Note", "n"),
    ("Source", "v"),
    ("Attachments", "o"),
    ("Archive thread", "A"),
    ("Sent", "g s"),
    ("Drafts", "g t"),
    ("Flagged", "g *"),
    ("Archive", "g r"),
    ("Snoozed", "g z"),
    ("Outbox", "g b"),
    ("Folders", "g o"),
    ("Digest rules", "g d"),
    ("Filtered", "g f"),
];

#[test]
fn no_screen_the_terminal_draws_shows_a_key_that_moved() {
    // `Filtered g f` is still right; it proves the matcher finds a key at all, and is taken out
    // below.
    let mut screens: Vec<(String, String)> = Vec::new();
    for state in sample::STATES {
        let (app, _) = sample::state(state, 140, 36);
        screens.push(((*state).to_owned(), screen(140, 36, &app)));
    }
    let mut app = in_thread();
    screens.push(("thread".into(), screen(140, 36, &app)));
    press_binding(&mut app, "Z", 0);
    screens.push(("fold".into(), screen(140, 36, &app)));

    let mut failures = Vec::new();
    for (state, text) in &screens {
        for line in text.lines() {
            for (label, old) in RETIRED {
                if (*label, *old) == ("Filtered", "g f") {
                    continue;
                }
                if shows(line, label, old) {
                    failures.push(format!(
                        "{state}: `{}` still shows {label} on `{old}`",
                        line.trim()
                    ));
                }
            }
        }
    }
    assert!(shows("Archive e   Reply r", "Archive", "e"));
    assert!(!shows("Archive e   Reply r", "Archive", "a"));
    let list = &screens[0].1;
    assert!(list.lines().any(|line| shows(line, "Reply", "r")), "{list}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
