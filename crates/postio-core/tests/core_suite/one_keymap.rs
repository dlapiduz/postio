use std::collections::BTreeMap;

use postio_config::KeyBindings;
use postio_config::keys::expand_mod;
use postio_config::paths::Platform;
use postio_core::config::Keymap;
use postio_core::{Availability, CommandId, Context, Frontend, Scope, registry};
use postio_model::AccountId;

const APPS: [(Frontend, Platform); 3] = [
    (Frontend::Terminal, Platform::Freedesktop),
    (Frontend::Focus, Platform::Freedesktop),
    (Frontend::Focus, Platform::Apple),
];

#[derive(Debug, Clone, Copy)]
enum Offered {
    All,
    Graphical,
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

struct Row {
    id: &'static str,
    key: &'static str,
    alternates: &'static [&'static str],
    offered: Offered,
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

const CONTRACT: &[Row] = &[
    with("next_message", "j", &["Down"], All, &[]),
    with("prev_message", "k", &["Up"], All, &[]),
    row("first_message", "g g", All),
    row("last_message", "G", All),
    with("open_message", "Return", &["o"], All, &[]),
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
    with("select_all", "* a", &["mod+a", "X"], All, &[]),
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
        "next_in_thread",
        "]",
        &["n", "alt+Down"],
        All,
        &[Context::Conversation, Context::Reader],
    ),
    with(
        "prev_in_thread",
        "[",
        &["p", "alt+Up"],
        All,
        &[Context::Conversation, Context::Reader],
    ),
    with("reply", "r", &["mod+r"], All, &[]),
    with("reply_all", "a", &["mod+shift+r"], All, &[]),
    with("forward", "f", &["mod+shift+f"], All, &[]),
    with("archive", "e", &["mod+shift+a"], All, &[Context::Digest]),
    with("delete", "#", &["Delete"], All, &[]),
    row("snooze", "b", All),
    row("unsnooze", "B", All),
    with(
        "remind_if_no_reply",
        "h",
        &["mod+h"],
        Focus,
        &[Context::Reader, Context::Composer],
    ),
    with("toggle_read", "I", &["U"], All, &[]),
    row("add_label", "l", All),
    row("move", "v", All),
    with("undo", "z", &["mod+z"], All, &[]),
    with("compose", "c", &["mod+n"], All, &[]),
    row("accept_invite", "y", Focus),
    row("decline_invite", "Y", Focus),
    with(
        "unsubscribe",
        "M",
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
    row("view_source", "V", Focus),
    row("open_attachment_or_link", "A", Focus),
    row("cheat_sheet", "?", All),
    row("flag", "s", All),
    row("prev_view", "u", All),
    with(
        "expand_thread",
        "Right",
        &[],
        Graphical,
        &[Context::List, Context::Results, Context::Filtered],
    ),
    with(
        "collapse_thread",
        "Left",
        &[],
        Graphical,
        &[Context::List, Context::Results, Context::Filtered],
    ),
    with("first_in_thread", "{", &[], Graphical, &[Context::Reader]),
    with("latest_in_thread", "}", &[], Graphical, &[Context::Reader]),
    with(
        "first_unread_in_thread",
        "g u",
        &[],
        Graphical,
        &[Context::Reader, Context::Outline],
    ),
    with(
        "outline_first_unread",
        "u",
        &[],
        Graphical,
        &[Context::Outline],
    ),
    with(
        "thread_outline",
        "alt+t",
        &[],
        Graphical,
        &[Context::Reader, Context::List],
    ),
    with(
        "thread_menu",
        "alt+period",
        &[],
        Graphical,
        &[Context::Reader],
    ),
    with(
        "reply_to_latest",
        "alt+e",
        &[],
        Graphical,
        &[Context::Composer, Context::Results],
    ),
    with(
        "mute_thread",
        "m",
        &[],
        Graphical,
        &[Context::List, Context::Results],
    ),
    with(
        "split_thread",
        "alt+shift+s",
        &[],
        Graphical,
        &[Context::Reader],
    ),
    row("move_this_message", "alt+v", Graphical),
    with(
        "delete_this_message",
        "alt+BackSpace",
        &["alt+Delete"],
        Graphical,
        &[Context::Results],
    ),
    with(
        "forward_what_1",
        "1",
        &[],
        Graphical,
        &[Context::ForwardChooser],
    ),
    with(
        "forward_what_2",
        "2",
        &[],
        Graphical,
        &[Context::ForwardChooser],
    ),
    with(
        "forward_what_3",
        "3",
        &[],
        Graphical,
        &[Context::ForwardChooser],
    ),
    with(
        "show_newer",
        "mod+shift+Down",
        &[],
        Graphical,
        &[Context::Composer],
    ),
    with(
        "expand_quoted",
        "mod+shift+q",
        &[],
        Graphical,
        &[Context::Composer],
    ),
    row("darken_message", "alt+d", Graphical),
    with("search", "/", &["alt+mod+f"], All, &[]),
    row("command_palette", "mod+k", All),
    row("go_to_inbox", "g i", All),
    row("go_to_folders", "g l", All),
    row("go_to_drafts", "g d", All),
    row("go_to_sent", "g t", All),
    row("go_to_archive", "g a", All),
    row("go_to_snoozed", "g b", All),
    row("go_to_flagged", "g s", All),
    row("go_to_outbox", "g o", Focus),
    row("go_to_junk", "g j", Focus),
    row("go_to_trash", "g #", Focus),
    row("go_to_filtered", "g f", Focus),
    row("go_to_digest_rules", "g r", Focus),
    row("saved_search_1", "alt+1", All),
    row("saved_search_2", "alt+2", All),
    row("saved_search_3", "alt+3", All),
    row("saved_search_4", "alt+4", All),
    row("toggle_has_action", "!", Focus),
    with("save_search", "mod+s", &[], All, &[Context::Search]),
    with(
        "back_to_words",
        "mod+BackSpace",
        &["alt+BackSpace"],
        Focus,
        &[Context::Search, Context::Results],
    ),
    with("toggle_result_order", "alt+o", &[], All, &[Context::Search]),
    with(
        "show_all_results",
        "mod+Return",
        &[],
        Graphical,
        &[Context::Search],
    ),
    with(
        "exclude_suggestion",
        "alt+Return",
        &[],
        Graphical,
        &[Context::Search],
    ),
    with(
        "forget_recent",
        "alt+BackSpace",
        &["alt+Delete"],
        Graphical,
        &[Context::Search],
    ),
    with(
        "history_back",
        "mod+bracketleft",
        &[],
        Graphical,
        &[Context::List, Context::Results],
    ),
    with(
        "history_forward",
        "mod+bracketright",
        &[],
        Graphical,
        &[Context::List, Context::Results],
    ),
    with(
        "results_threads",
        "mod+1",
        &[],
        Graphical,
        &[Context::Results],
    ),
    with(
        "results_files",
        "mod+2",
        &[],
        Graphical,
        &[Context::Results],
    ),
    with(
        "results_people",
        "mod+3",
        &[],
        Graphical,
        &[Context::Results],
    ),
    with("quick_look", "space", &[], Graphical, &[Context::Results]),
    with("next_match", "]", &[], Graphical, &[Context::Results]),
    with("prev_match", "[", &[], Graphical, &[Context::Results]),
    with(
        "pick_relaxation_1",
        "1",
        &[],
        Graphical,
        &[Context::Results],
    ),
    with(
        "pick_relaxation_2",
        "2",
        &[],
        Graphical,
        &[Context::Results],
    ),
    with(
        "pick_relaxation_3",
        "3",
        &[],
        Graphical,
        &[Context::Results],
    ),
    with(
        "pick_relaxation_4",
        "4",
        &[],
        Graphical,
        &[Context::Results],
    ),
    with("save_file", "mod+Down", &[], Graphical, &[Context::Results]),
    with(
        "step_range_back",
        "alt+Left",
        &[],
        Graphical,
        &[Context::Results],
    ),
    with(
        "step_range_forward",
        "alt+Right",
        &[],
        Graphical,
        &[Context::Results],
    ),
    with("picker_choose_1", "1", &[], Focus, &[Context::Picker]),
    with("picker_choose_2", "2", &[], Focus, &[Context::Picker]),
    with("picker_choose_3", "3", &[], Focus, &[Context::Picker]),
    with("picker_choose_4", "4", &[], Focus, &[Context::Picker]),
    with("picker_type_date", "tab", &[], Focus, &[Context::Picker]),
    with("picker_toggle", "space", &[], Focus, &[Context::Picker]),
    with("picker_confirm", "Return", &[], Focus, &[Context::Picker]),
    with(
        "capture_task",
        "T",
        &[],
        Focus,
        &[Context::List, Context::Reader, Context::Capture],
    ),
    with(
        "capture_note",
        "N",
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
    with(
        "source_tab_verification",
        "1",
        &[],
        Focus,
        &[Context::Source],
    ),
    with("source_tab_headers", "2", &[], Focus, &[Context::Source]),
    with("source_tab_raw", "3", &[], Focus, &[Context::Source]),
    with("copy_source", "mod+c", &[], Focus, &[Context::Source]),
    with("save_source", "mod+s", &[], Focus, &[Context::Source]),
    with("unfold_part", "Return", &[], Focus, &[Context::Source]),
    with("accept_key", "a", &[], Focus, &[Context::Source]),
    with("forget_key", "x", &[], Focus, &[Context::Source]),
    with("import_key", "i", &[], Focus, &[Context::Source]),
    with("next_reference", "]", &[], Focus, &[Context::Digest]),
    with("prev_reference", "[", &[], Focus, &[Context::Digest]),
    with(
        "toggle_digest_summary",
        "tab",
        &[],
        Focus,
        &[Context::Digest],
    ),
    with("remove_account", "Delete", &[], All, &[Context::Accounts]),
    row("refresh", "F5", All),
];

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
    // A `[keys]` entry naming a retired id is reported as unknown (contracts/config.md).
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
        "archive_thread",
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
                .filter(|binding| registry::binding_offered_on(spec.id.into(), binding, platform))
                .map(|binding| expand_mod(binding, platform))
                .collect();
            assert_eq!(
                keymap.bindings(spec.id),
                expected.as_slice(),
                "{app:?} on {platform:?} does not give `{}` the registry's key",
                spec.id
            );
            let reserved: Vec<String> = spec
                .bindings()
                .filter(|binding| {
                    [Platform::Freedesktop, Platform::Apple]
                        .iter()
                        .any(|platform| {
                            !registry::binding_offered_on(spec.id.into(), binding, *platform)
                        })
                })
                .map(|binding| binding.replace("mod+", "ctrl+"))
                .collect();
            let as_written: Vec<String> = keymap
                .bindings(spec.id)
                .iter()
                .map(|binding| match binding.as_str() {
                    "BackSpace" if platform == Platform::Apple => "Delete".to_owned(),
                    other => other.replace("cmd+", "ctrl+"),
                })
                .filter(|binding| !reserved.contains(binding))
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

#[test]
fn the_search_bar_s_keys_resolve_on_both_platforms() {
    let mac = Keymap::resolve_on(&KeyBindings::default(), Platform::Apple);
    let linux = Keymap::resolve_on(&KeyBindings::default(), Platform::Freedesktop);

    assert_eq!(mac.bindings(CommandId::ShowAllResults), ["cmd+Return"]);
    assert_eq!(mac.bindings(CommandId::ForgetRecent), ["alt+BackSpace"]);
    assert_eq!(linux.bindings(CommandId::ShowAllResults), ["ctrl+Return"]);
    assert_eq!(linux.bindings(CommandId::ForgetRecent), ["alt+Delete"]);
    assert_eq!(
        linux.command_for(Context::Search, "alt+Delete"),
        Some(postio_core::ActionId::Builtin(CommandId::ForgetRecent))
    );

    assert_eq!(
        mac.bindings(CommandId::BackToWords),
        ["cmd+BackSpace"],
        "the terminal's alternate is not the Mac's: alt+BackSpace forgets a recent"
    );
    let terminal = Keymap::for_terminal(&KeyBindings::default());
    for keymap in [&linux, &terminal] {
        assert_eq!(
            keymap.bindings(CommandId::BackToWords),
            ["ctrl+BackSpace", "alt+BackSpace"],
            "a terminal delivers ctrl+BackSpace as plain backspace"
        );
    }
}

#[test]
fn the_results_views_keys_resolve_on_both_platforms() {
    let mac = Keymap::resolve_on(&KeyBindings::default(), Platform::Apple);
    let linux = Keymap::resolve_on(&KeyBindings::default(), Platform::Freedesktop);

    for (id, key) in [
        (CommandId::HistoryBack, "cmd+bracketleft"),
        (CommandId::HistoryForward, "cmd+bracketright"),
        (CommandId::ResultsConversations, "cmd+1"),
        (CommandId::ResultsFiles, "cmd+2"),
        (CommandId::ResultsPeople, "cmd+3"),
    ] {
        assert_eq!(mac.bindings(id), [key], "{id} on the Mac");
        assert_eq!(
            linux.bindings(id),
            [key.replace("cmd+", "ctrl+")],
            "{id} on Linux"
        );
        assert!(registry::get(id).available_in(Context::Results), "{id}");
        assert!(linux.offers(id), "{id} is not offered on Linux");
    }

    for (id, key) in [
        (CommandId::NextMessage, "j"),
        (CommandId::PrevMessage, "k"),
        (CommandId::ToggleSelection, "x"),
        (CommandId::SelectAll, "* a"),
        (CommandId::Archive, "e"),
        (CommandId::Reply, "r"),
        (CommandId::Snooze, "b"),
        (CommandId::AddLabel, "l"),
        (CommandId::Move, "v"),
        (CommandId::ToggleRead, "I"),
        (CommandId::Delete, "#"),
        (CommandId::Delete, "BackSpace"),
        (CommandId::OpenMessage, "Return"),
        (CommandId::ToggleHasAction, "!"),
        (CommandId::Undo, "z"),
        (CommandId::Undo, "cmd+z"),
        (CommandId::SaveSearch, "cmd+s"),
        (CommandId::ToggleResultOrder, "alt+o"),
        (CommandId::Search, "/"),
    ] {
        assert!(
            registry::get(id).available_in(Context::Results),
            "`{id}` is not reachable in the results"
        );
        assert!(
            mac.bindings(id).iter().any(|bound| bound == key),
            "`{id}` is not on `{key}` on the Mac: {:?}",
            mac.bindings(id)
        );
    }
}

#[test]
fn the_timeline_steps_on_alt_arrows_on_both_platforms() {
    let mac = Keymap::resolve_on(&KeyBindings::default(), Platform::Apple);
    let linux = Keymap::resolve_on(&KeyBindings::default(), Platform::Freedesktop);
    for (id, key) in [
        (CommandId::StepRangeBack, "alt+Left"),
        (CommandId::StepRangeForward, "alt+Right"),
    ] {
        assert_eq!(mac.bindings(id), [key], "{id} on the Mac");
        assert_eq!(linux.bindings(id), [key], "{id} on Linux");
        assert!(registry::get(id).available_in(Context::Results), "{id}");
    }
}

#[test]
fn quick_look_is_space_and_its_matches_are_brackets() {
    let mac = Keymap::resolve_on(&KeyBindings::default(), Platform::Apple);
    let linux = Keymap::resolve_on(&KeyBindings::default(), Platform::Freedesktop);
    for (id, key) in [
        (CommandId::QuickLook, "space"),
        (CommandId::NextMatch, "]"),
        (CommandId::PrevMatch, "["),
    ] {
        assert_eq!(mac.bindings(id), [key], "{id} on the Mac");
        assert_eq!(
            mac.command_for(Context::Results, key),
            Some(postio_core::ActionId::Builtin(id)),
            "{key}"
        );
        for keymap in [&mac, &linux] {
            assert_eq!(
                keymap.command_for(Context::Results, key),
                Some(postio_core::ActionId::Builtin(id)),
                "{key}"
            );
        }
        assert_eq!(linux.bindings(id), [key], "{id} on Linux");
    }
}

#[test]
fn relaxations_are_numbers_and_back_to_words_reaches_the_results() {
    let mac = Keymap::resolve_on(&KeyBindings::default(), Platform::Apple);
    let linux = Keymap::resolve_on(&KeyBindings::default(), Platform::Freedesktop);
    for (id, key) in [
        (CommandId::PickRelaxation1, "1"),
        (CommandId::PickRelaxation2, "2"),
        (CommandId::PickRelaxation3, "3"),
        (CommandId::PickRelaxation4, "4"),
    ] {
        assert_eq!(mac.bindings(id), [key], "{id} on the Mac");
        assert_eq!(
            mac.command_for(Context::Results, key),
            Some(postio_core::ActionId::Builtin(id)),
            "{key}"
        );
        assert_eq!(linux.bindings(id), [key], "{id} on Linux");
    }
    assert_eq!(
        mac.command_for(Context::Results, "cmd+BackSpace"),
        Some(postio_core::ActionId::Builtin(CommandId::BackToWords)),
        "D24: ⌘⌫ in the results"
    );
    assert_eq!(
        linux.command_for(Context::Results, "ctrl+BackSpace"),
        Some(postio_core::ActionId::Builtin(CommandId::BackToWords)),
        "D24 on Linux"
    );
}

#[test]
fn exclude_suggestion_is_option_return() {
    let mac = Keymap::resolve_on(&KeyBindings::default(), Platform::Apple);
    let linux = Keymap::resolve_on(&KeyBindings::default(), Platform::Freedesktop);
    assert_eq!(mac.bindings(CommandId::ExcludeSuggestion), ["alt+Return"]);
    assert_eq!(
        mac.command_for(Context::Search, "alt+Return"),
        Some(postio_core::ActionId::Builtin(CommandId::ExcludeSuggestion))
    );
    assert_eq!(
        linux.command_for(Context::Search, "alt+Return"),
        Some(postio_core::ActionId::Builtin(CommandId::ExcludeSuggestion))
    );
}

#[test]
fn saving_a_file_is_command_down() {
    let mac = Keymap::resolve_on(&KeyBindings::default(), Platform::Apple);
    let linux = Keymap::resolve_on(&KeyBindings::default(), Platform::Freedesktop);
    assert_eq!(mac.bindings(CommandId::SaveFile), ["cmd+Down"]);
    assert_eq!(
        mac.command_for(Context::Results, "cmd+Down"),
        Some(postio_core::ActionId::Builtin(CommandId::SaveFile))
    );
    assert_eq!(
        linux.command_for(Context::Results, "ctrl+Down"),
        Some(postio_core::ActionId::Builtin(CommandId::SaveFile))
    );
}

#[test]
fn the_source_surface_answers_its_own_keys_and_esc_closes_it() {
    let linux = Keymap::resolve_on(&KeyBindings::default(), Platform::Freedesktop);
    let mac = Keymap::resolve_on(&KeyBindings::default(), Platform::Apple);
    for (key, id) in [
        ("1", CommandId::SourceTabVerification),
        ("2", CommandId::SourceTabHeaders),
        ("3", CommandId::SourceTabRaw),
        ("Return", CommandId::UnfoldPart),
        ("Escape", CommandId::Back),
        ("ctrl+f", CommandId::FindInMessage),
        ("ctrl+s", CommandId::SaveSource),
        ("ctrl+c", CommandId::CopySource),
    ] {
        assert_eq!(
            linux.command_for(Context::Source, key),
            Some(postio_core::ActionId::Builtin(id)),
            "`{key}` on the Source surface"
        );
    }
    assert_eq!(
        mac.command_for(Context::Source, "cmd+s"),
        Some(postio_core::ActionId::Builtin(CommandId::SaveSource))
    );
    assert_eq!(
        linux.command_for(Context::Source, "a"),
        Some(postio_core::ActionId::Builtin(CommandId::AcceptKey)),
        "over the source, a accepts the signer's key rather than archiving the row behind"
    );
}
