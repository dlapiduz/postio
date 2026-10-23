//! One keymap for every app (US7 scenario 2, T061): a command registered at
//! run time reaches Focus's key map, keyed, with no code of Focus's.
//!
//! Scenarios 3 and 5 need no display; they are in
//! `crates/postio-ui/tests/ui_suite/one_keymap.rs` (ADR 0049).

use postio_core::{ActionId, Context, ContextSet, registry};

use crate::support;

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
