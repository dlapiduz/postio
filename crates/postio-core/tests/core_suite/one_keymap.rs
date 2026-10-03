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

/// Every app that reads the registry.
const APPS: [Frontend; 3] = [Frontend::Terminal, Frontend::Focus, Frontend::Macos];

/// Who offers a row, in the contract's words.
#[derive(Debug, Clone, Copy)]
enum Offered {
    /// Every app that has the surface.
    All,
    /// Every app that draws a message as pixels: all but the terminal.
    Graphical,
    /// The three-pane app -- macOS -- and not Focus, whichever toolkit
    /// draws it.
    ThreePane,
    /// `Requirement::Focus`: Focus, in either toolkit (the terminal is Focus, C29).
    Focus,
}

impl Offered {
    fn by(self, app: Frontend) -> bool {
        match self {
            Offered::All => true,
            Offered::Graphical => app != Frontend::Terminal,
            Offered::ThreePane => app == Frontend::Macos,
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

use Offered::{All, Focus, Graphical, ThreePane};

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
    with("toggle_result_order", "O", &[], All, &[Context::Search]),
    // -- Pickers --------------------------------------------------------
    with("picker_choose_1", "1", &[], Focus, &[Context::Picker]),
    with("picker_choose_2", "2", &[], Focus, &[Context::Picker]),
    with("picker_choose_3", "3", &[], Focus, &[Context::Picker]),
    with("picker_choose_4", "4", &[], Focus, &[Context::Picker]),
    with("picker_type_date", "Tab", &[], Focus, &[Context::Picker]),
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
        "Tab",
        &[],
        Focus,
        &[Context::Digest],
    ),
    // -- Classic-only surfaces: "delete" has one key ---------------------
    // Only the apps with a folder list offer the first (T166): Focus has
    // none. The account list's verbs are Focus's too, from Settings'
    // Accounts section (T258).
    with(
        "delete_saved_search",
        "Delete",
        &[],
        ThreePane,
        &[Context::Sidebar],
    ),
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

/// The platform whose modifier `app`'s `mod` means.
fn platform_of(app: Frontend) -> Platform {
    match app {
        Frontend::Macos => Platform::Apple,
        Frontend::Terminal | Frontend::Focus => Platform::Freedesktop,
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
        for app in APPS {
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

/// "All" is every app that has the surface (the contract's legend), and
/// Focus has no folder sidebar, no panes to cycle and no parts panel: one
/// list, with mail opened in dialogs. The commands that work on those are
/// the other three apps', and their keys stay free in Focus. Flag is not
/// among them: Focus offers it on `*` with no mark on the row (spec C13).
#[test]
fn focus_offers_nothing_that_works_on_a_surface_it_does_not_have() {
    for command in [
        CommandId::ToggleSidebar,
        CommandId::CyclePane,
        CommandId::CyclePaneBack,
        CommandId::OpenParts,
    ] {
        let spec = registry::get(command);
        for app in APPS {
            assert_eq!(
                spec.requires.met_by(open_in(app)),
                app == Frontend::Macos,
                "`{command}` for {app:?}"
            );
        }
    }
}

#[test]
fn a_renamed_command_keeps_no_old_name() {
    // No backwards compatibility: a `[keys]` entry naming the old id is
    // reported as unknown, the same as any other (contracts/config.md).
    for retired in ["mark_unread", "focus_sidebar"] {
        assert!(
            retired.parse::<CommandId>().is_err(),
            "`{retired}` still names a command"
        );
    }
}

#[test]
fn every_command_an_app_offers_has_a_key() {
    for app in APPS {
        let keymap = Keymap::resolve_on(&KeyBindings::default(), platform_of(app));
        for spec in registry::all() {
            // Not offered, and so unbound, where the platform has no
            // surface for it (`registry::offered_on`).
            if !spec.requires.met_by(open_in(app))
                || !registry::offered_on(spec.id.into(), platform_of(app))
            {
                continue;
            }
            assert!(
                keymap.binding(spec.id).is_some_and(|key| !key.is_empty()),
                "{app:?} offers `{}` with no key",
                spec.id
            );
        }
    }
}

#[test]
fn no_key_means_two_commands_in_one_context_in_any_app() {
    let mut problems = Vec::new();
    for app in APPS {
        let keymap = Keymap::resolve_on(&KeyBindings::default(), platform_of(app));
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
                            "{app:?}, {context}: `{binding}` is both `{other}` and `{id}`"
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
    // Command on macOS, and nothing else may differ.
    for spec in registry::all() {
        let mut keys: Vec<(Frontend, Vec<String>)> = Vec::new();
        for app in APPS {
            if !spec.requires.met_by(open_in(app))
                || !registry::offered_on(spec.id.into(), platform_of(app))
            {
                continue;
            }
            let keymap = Keymap::resolve_on(&KeyBindings::default(), platform_of(app));
            let expected: Vec<String> = spec
                .bindings()
                .map(|binding| expand_mod(binding, platform_of(app)))
                .collect();
            assert_eq!(
                keymap.bindings(spec.id),
                expected.as_slice(),
                "{app:?} does not give `{}` the registry's key",
                spec.id
            );
            let as_written: Vec<String> = keymap
                .bindings(spec.id)
                .iter()
                .map(|binding| binding.replace("cmd+", "ctrl+"))
                .collect();
            keys.push((app, as_written));
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
