use postio_config::KeyBindings;
use postio_config::paths::Platform;
use postio_core::CommandId;
use postio_core::config::Keymap;
use postio_core::registry;

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
    let golden = include_str!("../golden/linux-bindings.txt");
    assert_eq!(
        table(Platform::Freedesktop),
        golden,
        "the Linux binding table moved; `mod` was supposed to be invisible here"
    );
}

fn offered_on_the_mac(table: &str) -> String {
    table
        .lines()
        .filter(|row| {
            let id = row.split('\t').next().unwrap_or_default();
            id.parse::<postio_core::ActionId>()
                .map(|id| registry::offered_on(id, Platform::Apple))
                .unwrap_or(true)
        })
        .map(|row| format!("{row}\n"))
        .collect()
}

fn offered_on_linux(table: &str) -> String {
    table
        .lines()
        .filter(|row| {
            let id = row.split('\t').next().unwrap_or_default();
            id.parse::<postio_core::ActionId>()
                .map(|id| registry::offered_on(id, Platform::Freedesktop))
                .unwrap_or(true)
        })
        .map(|row| format!("{row}\n"))
        .collect()
}

fn as_the_mac_spells_it(table: &str) -> String {
    table
        .lines()
        .map(|row| {
            let (id, keys) = row.split_once('\t').unwrap_or((row, ""));
            let keys = keys
                .split(" | ")
                .map(|binding| {
                    binding
                        .split(' ')
                        .map(|chord| {
                            chord
                                .split('+')
                                .map(|part| match part {
                                    "ctrl" => "cmd",
                                    "Delete" => "BackSpace",
                                    other => other,
                                })
                                .collect::<Vec<_>>()
                                .join("+")
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect::<Vec<_>>()
                .join(" | ");
            format!("{id}\t{keys}\n")
        })
        .collect()
}

#[test]
fn apple_gets_command_wherever_freedesktop_gets_control() {
    let linux = table(Platform::Freedesktop);
    let apple = table(Platform::Apple);
    assert_ne!(linux, apple, "nothing was translated at all");
    // ⌘W closes the front window on the Mac, and `alt+BackSpace` is its forget-recent
    // (`registry::binding_offered_on`).
    let linux = as_the_mac_spells_it(&offered_on_the_mac(&offered_on_linux(&linux)))
        .replace("quit\tcmd+q | cmd+w\n", "quit\tcmd+q\n")
        .replace(
            "back_to_words\tcmd+BackSpace | alt+BackSpace\n",
            "back_to_words\tcmd+BackSpace\n",
        );
    assert_eq!(
        linux,
        offered_on_linux(&offered_on_the_mac(&apple)),
        "the two tables differ somewhere other than the primary modifier and Delete"
    );
    assert!(apple.contains("quit\tcmd+q\n"), "⌘W is not Quit on the Mac");
}

#[test]
fn a_command_the_mac_does_not_offer_is_unbound_there_and_nowhere_else() {
    let apple = table(Platform::Apple);
    for row in apple.lines() {
        let (id, bindings) = row.split_once('\t').unwrap_or((row, ""));
        let Ok(action) = id.parse::<postio_core::ActionId>() else {
            continue;
        };
        if !registry::offered_on(action, Platform::Apple) {
            assert!(
                bindings.is_empty(),
                "`{id}` is not offered on the Mac and holds `{bindings}`"
            );
        }
    }
}

#[test]
fn no_binding_reaches_the_resolver_still_saying_mod() {
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
