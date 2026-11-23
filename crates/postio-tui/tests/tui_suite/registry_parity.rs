use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use postio_core::{Availability, Context, Keymap, Scope, registry};
use postio_tui::input::Keys;
use postio_ui::keymap::{Binding, Chord, Key, KeyContext, Modifiers, Outcome};

const PALETTE_SURFACES: &[Context] = &[
    Context::List,
    Context::Search,
    Context::Reader,
    Context::Conversation,
    Context::Composer,
    Context::Accounts,
    Context::Picker,
    Context::Digest,
    Context::Filtered,
    Context::Capture,
];

const NOT_YET: &[(&str, &str)] = &[];

fn key_event(chord: &Chord) -> KeyEvent {
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
        Key::Named("Space") => KeyCode::Char(' '),
        Key::Named("Return") => KeyCode::Enter,
        Key::Named("Escape") => KeyCode::Esc,
        Key::Named("Tab") if modifiers.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
        Key::Named("Tab") => KeyCode::Tab,
        Key::Named("BackSpace") => KeyCode::Backspace,
        Key::Named("Delete") => KeyCode::Delete,
        Key::Named("Home") => KeyCode::Home,
        Key::Named("End") => KeyCode::End,
        Key::Named("Page_Up") => KeyCode::PageUp,
        Key::Named("Page_Down") => KeyCode::PageDown,
        Key::Named("Up") => KeyCode::Up,
        Key::Named("Down") => KeyCode::Down,
        Key::Named("Left") => KeyCode::Left,
        Key::Named("Right") => KeyCode::Right,
        Key::Named(name) => match name.strip_prefix('F').and_then(|n| n.parse().ok()) {
            Some(number) => KeyCode::F(number),
            None => panic!("no terminal key for {name}"),
        },
    };
    KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn runs(keymap: &Keymap, binding: &str, context: Context, command: &str) -> bool {
    let Ok(parsed) = binding.parse::<Binding>() else {
        return false;
    };
    let mut keys = Keys::new(keymap).0;
    // The composer and palette are text entry; Search's keys are pressed over its results.
    let typing = matches!(context, Context::Composer | Context::Palette);
    let mut outcome = Outcome::Unhandled;
    for chord in parsed.chords() {
        outcome = keys.press(&key_event(chord), KeyContext::from(context), typing);
    }
    matches!(outcome, Outcome::Command(ref id) if id == command)
}

#[test]
fn every_command_is_reachable_by_a_key_this_terminal_sends_and_by_the_palette() {
    let keymap = Keymap::for_terminal(&Default::default());
    let open = Availability {
        frontend: postio_core::Frontend::Terminal,
        ..Availability::open(Scope::Account(postio_model::AccountId::new(1)))
    };
    let mut unreachable = Vec::new();
    for spec in registry::all() {
        let id = spec.id.as_str();
        if NOT_YET.iter().any(|(pending, _)| *pending == id) {
            continue;
        }
        if !spec.requires.met_by(open) {
            continue;
        }
        let contexts: Vec<Context> = Context::ALL
            .iter()
            .copied()
            .filter(|context| spec.available_in(*context))
            .collect();
        match postio_ui::terminal::deliverable_binding(&keymap, spec.id, false) {
            None => unreachable.push(format!("{id}: no key a terminal can send")),
            Some(binding) => {
                if !contexts
                    .iter()
                    .any(|context| runs(&keymap, &binding, *context, id))
                {
                    unreachable.push(format!("{id}: `{binding}` does not run it in {contexts:?}"));
                }
            }
        }
        let offered = PALETTE_SURFACES.iter().any(|surface| {
            postio_ui::palette::entries(&keymap, *surface, open, "")
                .iter()
                .any(|entry| entry.id == postio_core::ActionId::Builtin(spec.id))
        });
        if !offered {
            unreachable.push(format!(
                "{id}: not in the palette on any surface ({contexts:?})"
            ));
        }
    }
    assert!(
        unreachable.is_empty(),
        "{} gap(s) between the registry and the terminal:\n{}",
        unreachable.len(),
        unreachable.join("\n")
    );
}

#[test]
fn nothing_waits_on_a_task_that_is_already_done() {
    for (id, _) in NOT_YET {
        assert!(
            registry::all().any(|spec| spec.id.as_str() == *id),
            "{id} is not a command"
        );
    }
}
