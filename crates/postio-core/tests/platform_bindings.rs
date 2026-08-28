//! `mod` resolves to the right accelerator, and Linux does not move.
//!
//! The golden file was captured from the registry *before* `mod` existed, so
//! `the_freedesktop_table_is_byte_identical_to_before` is a genuine
//! before-and-after comparison rather than a restatement of the current code.

use postio_config::KeyBindings;
use postio_config::paths::Platform;
use postio_core::CommandId;
use postio_core::config::Keymap;
use postio_core::registry;

/// Every command and the bindings it holds, in registry order.
fn table(platform: Platform) -> String {
    let keymap = Keymap::resolve_on(&KeyBindings::default(), platform);
    let mut out = String::new();
    for spec in registry::every_action() {
        out.push_str(&format!(
            "{}\t{}\n",
            spec.id,
            keymap.bindings(spec.id).join(" | ")
        ));
    }
    out
}

#[test]
fn the_freedesktop_table_is_byte_identical_to_before() {
    // The argument for landing this before macOS ships bindings: if `mod` is
    // right, a Linux user cannot tell it happened.
    let golden = include_str!("golden/linux-bindings.txt");
    assert_eq!(
        table(Platform::Freedesktop),
        golden,
        "the Linux binding table moved; `mod` was supposed to be invisible here"
    );
}

#[test]
fn apple_gets_command_wherever_freedesktop_gets_control() {
    let linux = table(Platform::Freedesktop);
    let apple = table(Platform::Apple);
    assert_ne!(linux, apple, "nothing was translated at all");
    assert_eq!(
        linux.replace("ctrl+", "cmd+"),
        apple,
        "the two tables differ somewhere other than the primary modifier"
    );
}

#[test]
fn no_binding_reaches_the_resolver_still_saying_mod() {
    // `mod` is a config-file word. Below this point everything is a concrete
    // accelerator, because the resolver, the conflict check and the cheat
    // sheet all match on the literal string.
    for platform in [Platform::Freedesktop, Platform::Apple] {
        let table = table(platform);
        assert!(
            !table.contains("mod+"),
            "an unexpanded `mod+` survived resolution on {platform:?}:\n{table}"
        );
    }
}

#[test]
fn a_literal_ctrl_override_still_means_control_on_a_mac() {
    let mut overrides = KeyBindings::default();
    overrides
        .overrides_mut()
        .insert("archive".to_string(), "ctrl+shift+z".to_string());
    let keymap = Keymap::resolve_on(&overrides, Platform::Apple);
    assert_eq!(keymap.binding(CommandId::Archive), Some("ctrl+shift+z"));
}

#[test]
fn a_mod_override_beats_a_default_it_would_otherwise_collide_with() {
    // Expansion happens before the conflict check, so `mod+k` and the palette's
    // own `ctrl+k` are recognised as the same key rather than both claimed.
    let mut overrides = KeyBindings::default();
    overrides
        .overrides_mut()
        .insert("archive".to_string(), "mod+k".to_string());
    let keymap = Keymap::resolve_on(&overrides, Platform::Freedesktop);
    assert_eq!(keymap.binding(CommandId::Archive), Some("ctrl+k"));
    assert_ne!(
        keymap.binding(CommandId::CommandPalette),
        Some("ctrl+k"),
        "both commands claimed the same key"
    );
}
