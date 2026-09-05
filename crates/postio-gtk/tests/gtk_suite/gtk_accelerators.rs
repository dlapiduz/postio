//! The context menu carries the keys, and GTK can parse every one (#568).
//!
//! `list_view.rs` promised "each item carrying the key that does the same
//! thing" long before anything set one; this is the test that keeps the
//! promise kept. One `#[test]`, like the rest of `gtk_*`: GTK may be
//! initialised once per process (#41).

use gtk::prelude::*;
use postio_gtk::keymap::{Keymap, gtk_accelerator, trigger_for_command};
use postio_gtk::list_view::context_menu_model;

pub fn menu_items_carry_parseable_accelerators() {
    if !(adw::init().is_ok() && gtk::gdk::Display::default().is_some()) {
        eprintln!("skipping: no display");
        return;
    }

    let commands = postio_core::Keymap::resolve(&Default::default());
    let (table, problems) = Keymap::from_commands(&commands);
    assert!(
        problems.is_empty(),
        "default bindings are clean: {problems:?}"
    );

    // Every trigger the registry produces renders to a string GTK parses
    // back — the renderer cannot drift from the format menus read.
    let mut rendered = 0;
    for spec in postio_core::registry::every_action() {
        let Some(chord) = trigger_for_command(&table, spec.id.as_str()) else {
            continue;
        };
        let accelerator = gtk_accelerator(&chord)
            .unwrap_or_else(|| panic!("`{chord}` ({}) rendered no accelerator", spec.id));
        assert!(
            gtk::accelerator_parse(&accelerator).is_some(),
            "GTK cannot parse `{accelerator}` for `{}`",
            spec.id
        );
        rendered += 1;
    }
    assert!(rendered > 10, "only {rendered} commands had a trigger");

    // And the context menu actually sets them: every item whose command has
    // a single-chord binding carries it as its `accel` attribute.
    let menu = context_menu_model(&commands);
    let mut carried = 0;
    for position in 0..menu.n_items() {
        let action = menu
            .item_attribute_value(position, "target", None)
            .and_then(|value| value.str().map(str::to_owned))
            .expect("every item targets a command");
        let accel = menu
            .item_attribute_value(position, "accel", None)
            .and_then(|value| value.str().map(str::to_owned));
        let expected = trigger_for_command(&table, &action)
            .as_ref()
            .and_then(gtk_accelerator);
        assert_eq!(accel, expected, "`{action}`");
        if accel.is_some() {
            carried += 1;
        }
    }
    assert!(carried > 0, "no menu item carried a key");
}
