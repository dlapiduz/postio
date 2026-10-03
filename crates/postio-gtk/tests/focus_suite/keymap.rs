//! The key map (US7, T058; screen 20): `?` opens it, `?` or `Escape` closes
//! it, and every key it shows is the key the keymap resolves -- pressed
//! through Focus's own resolver, not compared with the table that drew it.

use std::time::Instant;

use postio_core::{Keymap, registry};
use postio_ui::keymap::{Chord, KeyContext, Outcome};
use postio_ui::keymap_sheet::KEY_MAP_CONTEXTS;

use crate::support;

/// Press `?`: shift and the question mark.
fn question(window: &postio_gtk::window::FocusWindow) {
    support::press(window, "question", gtk::gdk::ModifierType::SHIFT_MASK);
}

/// The rows the open key map shows: each command's title and its keys.
fn rows_shown(window: &postio_gtk::window::FocusWindow) -> Vec<(String, Vec<String>)> {
    let Some(dialog) = window.key_map() else {
        return Vec::new();
    };
    support::with_class(&dialog, "focus-keymap-row")
        .iter()
        .map(|row| {
            let title = support::texts(&support::only(row, "focus-keymap-title")).join(" ");
            let keys = support::with_class(row, "postio-keyhint")
                .iter()
                .flat_map(support::texts)
                .collect();
            (title, keys)
        })
        .collect()
}

pub fn every_key_the_key_map_shows_runs_its_command() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = support::three_in_the_inbox().await;
        question(&window);
        assert!(
            crate::settle_until(async || {
                let rows = rows_shown(&window);
                !rows.is_empty() && rows.iter().all(|(title, _)| !title.is_empty())
            })
            .await,
            "? did not open the key map: {:?}",
            rows_shown(&window)
        );
        let rows = rows_shown(&window);
        let (mut resolver, _) = postio_gtk::keys::resolver(Keymap::defaults());
        let mut checked = 0;
        for (title, keys) in &rows {
            let spec = registry::all()
                .find(|spec| spec.title == title)
                .unwrap_or_else(|| panic!("{title:?} is not a registered title"));
            let context = KEY_MAP_CONTEXTS
                .iter()
                .find(|context| spec.contexts.contains(**context))
                .map(|context| KeyContext::from(*context))
                .unwrap_or(KeyContext::Global);
            assert!(!keys.is_empty(), "{title} shows no key");
            for key in keys {
                let mut outcome = Outcome::Unhandled;
                for chord in key.split(' ') {
                    let chord: Chord = chord
                        .parse()
                        .unwrap_or_else(|_| panic!("{title}: {key:?} is not a key"));
                    outcome = resolver.press(&chord, context, false, Instant::now());
                }
                assert_eq!(
                    outcome,
                    Outcome::Command(spec.id.as_str().into()),
                    "{title}: {key} in {context:?}"
                );
                checked += 1;
            }
        }
        assert!(checked > 30, "only {checked} keys were checked");
        // `a`, then the menu chord the second keyboard layer gave it.
        let chord =
            postio_config::keys::expand_mod("mod+shift+a", postio_config::paths::Platform::host());
        assert!(
            rows.iter().any(|(title, keys)| title == "Archive"
                && keys == &["a".to_owned(), chord.clone()]),
            "archive shows a and {chord}: {rows:?}"
        );

        let dialog = window.key_map().expect("the key map");
        let said = support::texts(&dialog).join(" ");
        assert!(
            said.contains("[keys]") && said.contains("config.toml"),
            "the footer names where to rebind: {said}"
        );
        // `j` behind the dialog is the dialog's, not the list's.
        let cursor = window.cursor_row().map(|row| row.id());
        support::keys(&window, &["j"]);
        crate::settle();
        assert_eq!(window.cursor_row().map(|row| row.id()), cursor);

        question(&window);
        assert!(
            crate::settle_until(async || window.key_map().is_none()).await,
            "? did not close the key map"
        );
        question(&window);
        assert!(crate::settle_until(async || window.key_map().is_some()).await);
        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || window.key_map().is_none()).await,
            "Escape did not close the key map"
        );
    });
}
