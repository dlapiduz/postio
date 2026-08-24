//! A registered command is discoverable in the surfaces that teach the app.
//!
//! `postio-plp4`, the view half. `postio-core`'s own suite proves the
//! vocabulary widened; this proves the widening reached the two surfaces that
//! make a command findable — `Ctrl+K` and `?`.
//!
//! # Why this is a separate assertion
//!
//! `ARCHITECTURE.md` §2: a command that is not in the registry does not
//! exist, "not merely unbound, but absent from every way a user could discover
//! it". The corollary is that registering it and *not* reaching these surfaces
//! is the same failure wearing a different hat — the extension mechanism would
//! have bypassed the thing that makes commands discoverable, which is the
//! specific outcome the bead was filed to prevent.
//!
//! So these assert over the real `sections()` and `entries()` the widgets are
//! built from, not over the registry they read.
//!
//! Nothing here needs a display: both are pure functions over the registry and
//! the keymap, which is why they were built that way.

use postio_core::registry::{self, ExtCommand};
use postio_core::{ActionId, CommandId, Context, ContextSet, Keymap, Recovery};
use postio_gtk::{cheatsheet, palette};

/// A namespaced id nothing else in this binary uses.
fn unique_id(name: &str) -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    format!("gtktest{}:{name}", NEXT.fetch_add(1, Ordering::Relaxed))
}

fn register(id: &str, title: &str, binding: Option<&str>) -> postio_core::ExtId {
    registry::register(ExtCommand {
        id: id.to_string(),
        title: title.to_string(),
        default_binding: binding.map(str::to_owned),
        alternate_bindings: Vec::new(),
        contexts: ContextSet::from_slice(&[Context::List]),
        destructive: false,
        recovery: Recovery::None,
    })
    .expect("it registers")
}

#[test]
fn a_registered_command_is_findable_in_the_palette() {
    let id = unique_id("summarise-thread");
    let ext = register(&id, "Summarise thread", Some("ctrl+shift+s"));
    let keymap = Keymap::resolve(&Default::default());

    // By title, the way a user reaches for it.
    let found = palette::entries(&keymap, Context::List, "summarise");
    let row = found
        .iter()
        .find(|entry| entry.id == ActionId::Ext(ext))
        .expect("typing its title in Ctrl+K does not find it");
    assert_eq!(row.title, "Summarise thread");
    assert_eq!(
        row.binding.as_deref(),
        Some("ctrl+shift+s"),
        "the palette shows the key it is actually bound to, the same as for a \
         built-in — read from the live keymap, not from the registration"
    );

    // By its namespaced id, which is what a log line or a config file spells.
    assert!(
        palette::entries(&keymap, Context::List, &id)
            .iter()
            .any(|entry| entry.id == ActionId::Ext(ext)),
        "it cannot be found by the id `[keys]` names it with"
    );

    // And the built-ins are unharmed beside it.
    assert!(
        palette::entries(&keymap, Context::List, "archive")
            .iter()
            .any(|entry| entry.id == ActionId::Builtin(CommandId::Archive)),
    );
}

#[test]
fn the_palette_still_respects_the_context_predicate() {
    // An extension declares its contexts like anything else, and offering a
    // row the user can only be disappointed by is the thing `entries` filters
    // for. Registered for the list only.
    let id = unique_id("list-only");
    let ext = register(&id, "List only", None);
    let keymap = Keymap::resolve(&Default::default());

    assert!(
        palette::entries(&keymap, Context::List, "list only")
            .iter()
            .any(|entry| entry.id == ActionId::Ext(ext))
    );
    assert!(
        !palette::entries(&keymap, Context::Composer, "list only")
            .iter()
            .any(|entry| entry.id == ActionId::Ext(ext)),
        "it turned up in the composer, where it was never registered"
    );
}

#[test]
fn the_cheat_sheet_teaches_it_under_where_it_came_from() {
    let id = unique_id("file-to-receipts");
    let ext = register(&id, "File to receipts", Some("ctrl+shift+f"));
    let namespace = ext.namespace();

    let sections = cheatsheet::sections(&Keymap::resolve(&Default::default()));
    let section = sections
        .iter()
        .find(|section| section.title == namespace)
        .expect("`?` has no section for the namespace the command came from");

    let row = section
        .rows
        .iter()
        .find(|row| row.id == Some(ActionId::Ext(ext)))
        .expect("the command is not on the sheet");
    assert_eq!(row.title, "File to receipts");
    assert_eq!(row.binding.as_deref(), Some("ctrl+shift+f"));

    // Provenance is the grouping, and it is last: the built-in sheet is a
    // stable thing people learn, and a plugin must not be able to reorder it.
    let built_in_sections = sections
        .iter()
        .position(|section| section.title == "Message list")
        .expect("a built-in section");
    let extension_section = sections
        .iter()
        .position(|section| section.title == namespace)
        .expect("the extension section");
    assert!(
        extension_section > built_in_sections,
        "extension sections must come after the built-in ones"
    );
}

#[test]
fn a_key_bound_to_a_registered_command_resolves_through_the_real_resolver() {
    // The keymap resolver is what a key press actually goes through. A
    // command that is in the registry and not in the resolver is bound on
    // paper and dead in the hand.
    let id = unique_id("triage");
    register(&id, "Triage", Some("ctrl+shift+g"));

    let (resolver, problems) =
        postio_gtk::keymap::Keymap::from_commands(&Keymap::resolve(&Default::default()));
    assert!(
        problems.iter().all(|problem| !problem.contains(&id)),
        "the binding could not be parsed: {problems:?}"
    );
    let bound = resolver
        .binding_for(postio_gtk::keymap::KeyContext::List, &id)
        .expect("the resolver does not know the command, so its key does nothing");
    // Compared as parsed bindings, not as strings: the resolver prints a
    // chord in its own normalised form (`ctrl+G`), so a string comparison here
    // would be asserting the display convention rather than the binding.
    assert_eq!(
        bound,
        &"ctrl+shift+g"
            .parse::<postio_gtk::keymap::Binding>()
            .expect("the key the command asked for is parseable"),
        "the resolver bound it to something other than the key it asked for"
    );
}
