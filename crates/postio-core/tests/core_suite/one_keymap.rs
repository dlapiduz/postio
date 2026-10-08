//! One keymap for every app (specs/007-postio-focus SC-015,
//! contracts/keymap.md).
//!
//! The maintainer's answer was that one table of keys is the default for
//! every Postio app. The registry stays its one source, and four apps read
//! it: the classic desktop app, the terminal, Focus and macOS. Enumerated
//! across them, for every app:
//!
//! - every command it offers has a default key;
//! - no key is bound to two commands in one context;
//! - a command's key is the same wherever it is offered.
//!
//! And the contract's rows are the registry's: each command has the key the
//! contract gives it, and is offered by exactly the apps the contract names.
//! A Focus row is reachable in Focus alone -- its key is reserved in the
//! others, not reused.

use std::collections::BTreeMap;

use postio_config::KeyBindings;
use postio_config::keys::expand_mod;
use postio_config::paths::Platform;
use postio_core::config::Keymap;
use postio_core::{Availability, CommandId, Context, Frontend, Scope, registry};
use postio_model::AccountId;

/// Every app that reads the registry, with the platform whose `mod` it
/// means: the terminal and Postio on Linux, and Postio on the Mac, which
/// registers as Focus on `Platform::Apple` (specs/009-focus-macos R5).
const APPS: [(Frontend, Platform); 3] = [
    (Frontend::Terminal, Platform::Freedesktop),
    (Frontend::Focus, Platform::Freedesktop),
    (Frontend::Focus, Platform::Apple),
];

/// Who offers a row, in the contract's words.
#[derive(Debug, Clone, Copy)]
enum Offered {
    /// Every app that has the surface.
    All,
    /// Every app that draws a message as pixels: all but the terminal.
    Graphical,
    /// `Requirement::Focus`: Focus, in either toolkit (the terminal is Focus, C29).
    Focus,
}

impl Offered {
    fn by(self, app: Frontend) -> bool {
        match self {
            Offered::All => true,
            Offered::Graphical => app != Frontend::Terminal,
            Offered::Focus => matches!(app, Frontend::Terminal | Frontend::Focus),
        }
    }
}

/// One row of contracts/keymap.md.
struct Row {
    /// The command id, as `[keys]` names it.
    id: &'static str,
    /// The default key, as the registry spells it (`mod+` unexpanded).
    key: &'static str,
    /// Its alternates, in order.
    alternates: &'static [&'static str],
    /// Who offers it.
    offered: Offered,
    /// The contexts the contract names for it. Each has to be one the
    /// command is reachable in; empty where the contract names none.
    contexts: &'static [Context],
}

const fn row(id: &'static str, key: &'static str, offered: Offered) -> Row {
    Row {
        id,
        key,
        alternates: &[],
        offered,
        contexts: &[],
    }
}

const fn with(
    id: &'static str,
    key: &'static str,
    alternates: &'static [&'static str],
    offered: Offered,
    contexts: &'static [Context],
) -> Row {
    Row {
        id,
        key,
        alternates,
        offered,
        contexts,
    }
}

use Offered::{All, Focus, Graphical};

/// contracts/keymap.md, table by table.
const CONTRACT: &[Row] = &[
    // -- Message surfaces: List, Conversation, Reader --------------------
    with("next_message", "j", &["Down"], All, &[]),
    with("prev_message", "k", &["Up"], All, &[]),
    row("first_message", "g g", All),
    row("last_message", "G", All),
    // The `l` alternate is dropped: `l` is the label key now.
    with("open_message", "Return", &["Right"], All, &[]),
    row("toggle_selection", "x", All),
    with(
        "extend_selection_down",
        "J",
        &["shift+Down"],
        All,
        &[Context::List, Context::Reader, Context::Search],
    ),
    with(
        "extend_selection_up",
        "K",
        &["shift+Up"],
        All,
        &[Context::List, Context::Reader, Context::Search],
    ),
    with("select_all", "X", &["mod+a"], All, &[]),
    with(
        "back",
        "Escape",
        &[],
        All,
        &[
            Context::Picker,
            Context::Digest,
            Context::Filtered,
            Context::Capture,
        ],
    ),
    with(
        "next_in_conversation",
        "]",
        &["alt+Down"],
        All,
        &[Context::Conversation, Context::Reader],
    ),
    with(
        "prev_in_conversation",
        "[",
        &["alt+Up"],
        All,
        &[Context::Conversation, Context::Reader],
    ),
    // The second layer's chords (#1306): alternates, in every app.
    with("reply", "e", &["mod+r"], All, &[]),
    with("reply_all", "E", &["mod+shift+r"], All, &[]),
    with("forward", "f", &["mod+shift+f"], All, &[]),
    with("archive", "a", &["mod+shift+a"], All, &[]),
    with("archive_thread", "A", &[], All, &[Context::Digest]),
    row("delete", "Delete", All),
    row("snooze", "s", All),
    row("unsnooze", "B", All),
    // `mod+h` is the composer's key, where `h` types.
    with(
        "remind_if_no_reply",
        "h",
        &["mod+h"],
        Focus,
        &[Context::Reader, Context::Composer],
    ),
    row("toggle_read", "r", All),
    row("add_label", "l", All),
    row("move", "m", All),
    row("undo", "mod+z", All),
    with("compose", "c", &["mod+n"], All, &[]),
    row("accept_invite", "y", Focus),
    row("decline_invite", "Y", Focus),
    with(
        "unsubscribe",
        "U",
        &[],
        All,
        &[Context::Reader, Context::Digest],
    ),
    with(
        "digest_rule",
        "d",
        &[],
        Focus,
        &[Context::Reader, Context::Digest],
    ),
    row("view_source", "v", Focus),
    row("open_attachment_or_link", "o", Focus),
    row("cheat_sheet", "?", All),
    row("flag", "*", All),
    row("prev_view", "Left", All),
    row("darken_message", "alt+d", Graphical),
    // -- Going places: Global, and the surfaces that go ------------------
    with("search", "/", &["alt+mod+f"], All, &[]),
    row("command_palette", "mod+k", All),
    row("go_to_inbox", "g i", All),
    row("go_to_folders", "g o", All),
    row("go_to_drafts", "g t", All),
    row("go_to_sent", "g s", All),
    row("go_to_archive", "g r", All),
    row("go_to_snoozed", "g z", All),
    row("go_to_flagged", "g *", All),
    row("go_to_outbox", "g b", Focus),
    row("go_to_junk", "g j", Focus),
    row("go_to_trash", "g #", Focus),
    row("go_to_filtered", "g f", Focus),
    row("go_to_digest_rules", "g d", Focus),
    row("saved_search_1", "alt+1", All),
    row("saved_search_2", "alt+2", All),
    row("saved_search_3", "alt+3", All),
    row("saved_search_4", "alt+4", All),
    row("toggle_has_action", "!", Focus),
    // -- Search ---------------------------------------------------------
    with("save_search", "mod+s", &[], All, &[Context::Search]),
    with(
        "back_to_words",
        "mod+BackSpace",
        &["alt+BackSpace"],
        Focus,
        &[Context::Search],
    ),
    with("toggle_result_order", "alt+o", &[], All, &[Context::Search]),
    // -- Pickers --------------------------------------------------------
    with("picker_choose_1", "1", &[], Focus, &[Context::Picker]),
    with("picker_choose_2", "2", &[], Focus, &[Context::Picker]),
    with("picker_choose_3", "3", &[], Focus, &[Context::Picker]),
    with("picker_choose_4", "4", &[], Focus, &[Context::Picker]),
    with("picker_type_date", "tab", &[], Focus, &[Context::Picker]),
    with("picker_toggle", "space", &[], Focus, &[Context::Picker]),
    with("picker_confirm", "Return", &[], Focus, &[Context::Picker]),
    // -- Obsidian: the capture sheet (milestone 3, T158) ------------------
    // `t` and `n` open it from a message, and switch it between a task and
    // a note while it is up, as its toggles' keycaps say (screen 25).
    with(
        "capture_task",
        "t",
        &[],
        Focus,
        &[Context::List, Context::Reader, Context::Capture],
    ),
    with(
        "capture_note",
        "n",
        &[],
        Focus,
        &[Context::List, Context::Reader, Context::Capture],
    ),
    with(
        "capture_change_project",
        "mod+p",
        &[],
        Focus,
        &[Context::Capture],
    ),
    with(
        "capture_use_subject",
        "alt+s",
        &[],
        Focus,
        &[Context::Capture],
    ),
    with(
        "capture_write",
        "mod+Return",
        &["alt+Return"],
        Focus,
        &[Context::Capture],
    ),
    // -- Digests and Filtered -------------------------------------------
    with(
        "stop_digesting_sender",
        "D",
        &[],
        Focus,
        &[Context::Digest, Context::Reader],
    ),
    with("restore_filtered", "R", &[], Focus, &[Context::Filtered]),
    with("filtered_tab_1", "1", &[], Focus, &[Context::Filtered]),
    with("filtered_tab_2", "2", &[], Focus, &[Context::Filtered]),
    with("filtered_tab_3", "3", &[], Focus, &[Context::Filtered]),
    with("filtered_tab_4", "4", &[], Focus, &[Context::Filtered]),
    with("filtered_tab_5", "5", &[], Focus, &[Context::Filtered]),
    with("filtered_tab_6", "6", &[], Focus, &[Context::Filtered]),
    with("filtered_tab_7", "7", &[], Focus, &[Context::Filtered]),
    with("next_reference", "]", &[], Focus, &[Context::Digest]),
    with("prev_reference", "[", &[], Focus, &[Context::Digest]),
    with(
        "toggle_digest_summary",
        "tab",
        &[],
        Focus,
        &[Context::Digest],
    ),
    // -- The account list's verbs, from Settings' Accounts section (T258) --
    with("remove_account", "Delete", &[], All, &[Context::Accounts]),
    // `R` is Filtered's now; refresh keeps F5.
    row("refresh", "F5", All),
];

/// What `app` is, with mail open in one account: the state in which it
/// offers the most.
fn open_in(app: Frontend) -> Availability {
    Availability {
        frontend: app,
        ..Availability::open(Scope::Account(AccountId::new(1)))
    }
}

#[test]
fn the_registry_holds_the_contracts_keys() {
    let mut problems = Vec::new();
    for row in CONTRACT {
        let Ok(id) = row.id.parse::<CommandId>() else {
            problems.push(format!("`{}` is not a command", row.id));
            continue;
        };
        let spec = registry::get(id);
        if spec.default_binding != row.key {
            problems.push(format!(
                "`{}` is on `{}`, and the contract puts it on `{}`",
                row.id, spec.default_binding, row.key
            ));
        }
        if spec.alternate_bindings != row.alternates {
            problems.push(format!(
                "`{}` has the alternates {:?}, and the contract gives it {:?}",
                row.id, spec.alternate_bindings, row.alternates
            ));
        }
        for (app, _) in APPS {
            let offered = spec.requires.met_by(open_in(app));
            if offered != row.offered.by(app) {
                problems.push(format!(
                    "`{}` is {} by {app:?}, and the contract says {:?}",
                    row.id,
                    if offered { "offered" } else { "not offered" },
                    row.offered
                ));
            }
        }
        for context in row.contexts {
            if !spec.available_in(*context) {
                problems.push(format!(
                    "`{}` is not reachable in {context}, and the contract puts it there",
                    row.id
                ));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "{} difference(s) between the registry and contracts/keymap.md:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

#[test]
fn a_renamed_command_keeps_no_old_name() {
    // No backwards compatibility: a `[keys]` entry naming the old id is
    // reported as unknown, the same as any other (contracts/config.md).
    // The three-pane app's own commands went with it (specs/009-focus-macos
    // R5): Focus has no sidebar, no panes to cycle and no parts panel.
    for retired in [
        "mark_unread",
        "focus_sidebar",
        "toggle_sidebar",
        "toggle_rail",
        "cycle_pane",
        "cycle_pane_back",
        "next_folder",
        "prev_folder",
        "toggle_folder",
        "rename_saved_search",
        "move_saved_search_up",
        "move_saved_search_down",
        "delete_saved_search",
        "open_parts",
        "next_part",
        "prev_part",
        "open_part",
        "save_part",
        "save_all_parts",
        "open_part_externally",
        "render_part_once",
    ] {
        assert!(
            retired.parse::<CommandId>().is_err(),
            "`{retired}` still names a command"
        );
    }
}

#[test]
fn every_command_an_app_offers_has_a_key() {
    for (app, platform) in APPS {
        let keymap = Keymap::resolve_on(&KeyBindings::default(), platform);
        for spec in registry::all() {
            // Not offered, and so unbound, where the platform has no
            // surface for it (`registry::offered_on`).
            if !spec.requires.met_by(open_in(app))
                || !registry::offered_on(spec.id.into(), platform)
            {
                continue;
            }
            assert!(
                keymap.binding(spec.id).is_some_and(|key| !key.is_empty()),
                "{app:?} on {platform:?} offers `{}` with no key",
                spec.id
            );
        }
    }
}

#[test]
fn no_key_means_two_commands_in_one_context_in_any_app() {
    let mut problems = Vec::new();
    for (app, platform) in APPS {
        let keymap = Keymap::resolve_on(&KeyBindings::default(), platform);
        for context in Context::ALL {
            let mut seen: BTreeMap<&str, CommandId> = BTreeMap::new();
            for action in registry::reachable_in(*context, open_in(app)) {
                let Some(id) = action.id.builtin() else {
                    continue;
                };
                for binding in keymap.bindings(id) {
                    if let Some(other) = seen.insert(binding.as_str(), id)
                        && other != id
                    {
                        problems.push(format!(
                            "{app:?} on {platform:?}, {context}: `{binding}` is both `{other}` and `{id}`"
                        ));
                    }
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn a_command_has_the_same_key_in_every_app_that_offers_it() {
    // The same key, spelled for each platform: `mod` is Ctrl on Linux and
    // Command on macOS, `Delete` is the Mac's BackSpace, and nothing else may
    // differ.
    for spec in registry::all() {
        let mut keys: Vec<((Frontend, Platform), Vec<String>)> = Vec::new();
        for (app, platform) in APPS {
            if !spec.requires.met_by(open_in(app))
                || !registry::offered_on(spec.id.into(), platform)
            {
                continue;
            }
            let keymap = Keymap::resolve_on(&KeyBindings::default(), platform);
            let expected: Vec<String> = spec
                .bindings()
                .map(|binding| expand_mod(binding, platform))
                .collect();
            assert_eq!(
                keymap.bindings(spec.id),
                expected.as_slice(),
                "{app:?} on {platform:?} does not give `{}` the registry's key",
                spec.id
            );
            // Back to one spelling: Command is Control, and the Mac's lone
            // BackSpace is the `Delete` it was written as (specs/009-focus-macos
            // M6). No registry default is a lone BackSpace, so that is
            // unambiguous; `mod+BackSpace` keeps its modifier and its key.
            let as_written: Vec<String> = keymap
                .bindings(spec.id)
                .iter()
                .map(|binding| match binding.as_str() {
                    "BackSpace" if platform_of(app) == Platform::Apple => "Delete".to_owned(),
                    other => other.replace("cmd+", "ctrl+"),
                })
                .collect();
            keys.push(((app, platform), as_written));
        }
        if let Some((first, key)) = keys.first() {
            for (app, other) in &keys[1..] {
                assert_eq!(
                    other, key,
                    "`{}` is `{key:?}` in {first:?} and `{other:?}` in {app:?}",
                    spec.id
                );
            }
        }
    }
}
