//! One keymap for every app (US7 scenarios 2, 3 and 5, T061).
//!
//! Scenario 2 on a display: a command registered at run time reaches Focus's
//! key map, keyed, with no code of Focus's. Scenarios 3 and 5 need no
//! display: they enumerate the one default keymap through each app's own
//! resolver.

use std::time::Instant;

use postio_core::{ActionId, Context, ContextSet, Frontend, Keymap, registry};
use postio_ui::keymap::{Chord, KeyContext, Outcome, Resolver};

use crate::support;

/// Press `binding` -- a chord, or a sequence of them -- in `context`.
fn press(resolver: &mut Resolver, binding: &str, context: KeyContext) -> Outcome {
    let mut outcome = Outcome::Unhandled;
    for chord in binding.split(' ') {
        let chord: Chord = chord
            .parse()
            .unwrap_or_else(|_| panic!("{binding:?} is not a key"));
        outcome = resolver.press(&chord, context, false, Instant::now());
    }
    outcome
}

/// Where a command's key is pressed: its first context, or the list for a
/// command that works everywhere.
fn context_of(contexts: ContextSet) -> KeyContext {
    if contexts == ContextSet::ANY {
        return KeyContext::List;
    }
    contexts
        .iter()
        .next()
        .map(KeyContext::from)
        .unwrap_or(KeyContext::Global)
}

const APPS: [Frontend; 3] = [Frontend::Terminal, Frontend::Focus, Frontend::Macos];

pub fn a_registered_command_reaches_the_key_map_with_its_key() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        // Before anything resolves the keymap, as an extension loads.
        let ext = registry::register(registry::ExtCommand {
            id: "test:sort-by-sender".to_owned(),
            title: "Sort by sender".to_owned(),
            default_binding: Some("alt+z".to_owned()),
            alternate_bindings: Vec::new(),
            contexts: ContextSet::from_slice(&[Context::List]),
            destructive: false,
            recovery: postio_core::Recovery::None,
        })
        .expect("it registers");
        let (_fixture, window) = support::three_in_the_inbox().await;
        assert_eq!(
            window.keymap().bindings(ActionId::Ext(ext)),
            ["alt+z".to_owned()],
            "the window's keymap binds it"
        );
        support::press(&window, "question", gtk::gdk::ModifierType::SHIFT_MASK);
        assert!(
            crate::settle_until(async || {
                window.key_map().is_some_and(|dialog| {
                    support::with_class(&dialog, "focus-keymap-row")
                        .iter()
                        .any(|row| {
                            let said = support::texts(row);
                            said.first().is_some_and(|title| title == "Sort by sender")
                                && said.contains(&"alt+z".to_owned())
                        })
                })
            })
            .await,
            "the registered command is not in the key map with its key"
        );
    });
}

pub fn no_default_key_means_two_things_and_each_app_runs_the_same_key() {
    let keymap = Keymap::defaults();
    for app in APPS {
        let (_, problems) = Resolver::from_commands_for(keymap, app);
        assert!(
            problems.is_empty(),
            "{app:?}: the one keymap binds cleanly: {problems:?}"
        );
    }
    let mut checked = 0;
    for spec in registry::all() {
        let Some(binding) = keymap.binding(spec.id) else {
            continue;
        };
        let context = context_of(spec.contexts);
        for app in APPS
            .into_iter()
            .filter(|app| spec.requires.offered_by(*app))
        {
            let (mut resolver, _) = Resolver::from_commands_for(keymap, app);
            assert_eq!(
                press(&mut resolver, binding, context),
                Outcome::Command(spec.id.as_str().into()),
                "{app:?}: {binding} in {context:?} runs {}",
                spec.id.as_str()
            );
            checked += 1;
        }
    }
    assert!(checked > 200, "only {checked} keys were checked");
}
