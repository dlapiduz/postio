use std::time::Instant;

use postio_core::{ContextSet, Frontend, Keymap, registry};
use postio_ui::keymap::{Chord, KeyContext, Outcome, Resolver};

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

const APPS: [Frontend; 2] = [Frontend::Terminal, Frontend::Focus];

#[test]
fn no_default_key_means_two_things_and_each_app_runs_the_same_key() {
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
