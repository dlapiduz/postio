use std::collections::{BTreeMap, BTreeSet};

use postio_config::KeyBindings;
use postio_config::paths::Platform;
use postio_core::config::Keymap;
use postio_core::{
    Availability, Command, CommandId, Context, MessageTarget, Recovery, Requirement, Scope,
    registry,
};
use postio_model::AccountId;

#[test]
fn registry_is_enumerable_and_non_empty() {
    let all: Vec<_> = registry::all().collect();
    assert!(!all.is_empty(), "the registry must hold commands");
    assert_eq!(all.len(), CommandId::ALL.len());
}

#[test]
fn every_command_has_an_id_a_title_and_a_default_binding() {
    for spec in registry::all() {
        assert!(
            !spec.id.as_str().is_empty(),
            "{:?} has an empty id",
            spec.id
        );
        assert!(
            !spec.title.is_empty(),
            "{} has no human-readable title",
            spec.id
        );
        assert!(
            !spec.default_binding.is_empty(),
            "{} has no default binding; docs/PRODUCT.md §8 requires every command to \
             have a keyboard shortcut",
            spec.id
        );
        assert!(
            !spec.contexts.is_empty(),
            "{} is available in no context, so nothing could ever invoke it",
            spec.id
        );
    }
}

#[test]
fn every_command_id_resolves_to_exactly_one_spec() {
    for id in CommandId::ALL {
        let spec = registry::get(*id);
        assert_eq!(spec.id, *id);
    }
}

#[test]
fn command_ids_are_unique() {
    let unique: BTreeSet<&str> = CommandId::ALL.iter().map(|id| id.as_str()).collect();
    assert_eq!(unique.len(), CommandId::ALL.len(), "duplicate command id");
}

#[test]
fn bindings_are_gmails() {
    let expected = [
        ("reply", "r"),
        ("reply_all", "a"),
        ("forward", "f"),
        ("archive", "e"),
        ("delete", "#"),
        ("flag", "s"),
        ("snooze", "b"),
        ("move", "v"),
        ("add_label", "l"),
        ("toggle_read", "I"),
        ("next_in_thread", "]"),
        ("prev_in_thread", "["),
        ("expand_all", ";"),
        ("select_all", "* a"),
        ("more_actions", "."),
        ("capture_task", "T"),
        ("undo", "z"),
        ("compose", "c"),
        ("search", "/"),
        ("command_palette", "ctrl+k"),
        ("cheat_sheet", "?"),
        ("back", "Escape"),
        ("next_message", "j"),
        ("prev_message", "k"),
        ("prev_view", "u"),
        ("first_message", "g g"),
        ("last_message", "G"),
    ];
    let keymap = Keymap::resolve_on(&KeyBindings::default(), Platform::Freedesktop);
    for (id, key) in expected {
        let parsed: CommandId = id.parse().expect("known command id");
        assert_eq!(keymap.binding(parsed), Some(key), "binding for {id}");
    }
    assert!(
        registry::get(CommandId::OpenMessage)
            .bindings()
            .any(|b| b == "o"),
        "`o` must open the focused message"
    );
    for (id, key) in [
        (CommandId::PrevView, "u"),
        (CommandId::ToggleRead, "U"),
        (CommandId::Undo, "mod+z"),
        (CommandId::Delete, "Delete"),
    ] {
        assert!(
            registry::get(id).bindings().any(|b| b == key),
            "`{key}` must reach {id:?}"
        );
    }
    assert!(
        !registry::get(CommandId::OpenMessage)
            .bindings()
            .any(|b| b == "l"),
        "`l` is the label key, and opens nothing"
    );
}

#[test]
fn the_places_people_go_most_answer_to_two_keys() {
    let expected = [
        ("go_to_inbox", "g i"),
        ("go_to_drafts", "g d"),
        ("go_to_sent", "g t"),
        ("go_to_archive", "g a"),
        ("go_to_snoozed", "g b"),
        ("go_to_flagged", "g s"),
    ];
    let keymap = Keymap::resolve_on(&KeyBindings::default(), Platform::Freedesktop);
    for (id, key) in expected {
        let parsed: CommandId = id.parse().unwrap_or_else(|_| panic!("no command id {id}"));
        assert_eq!(keymap.binding(parsed), Some(key), "binding for {id}");

        let spec = registry::get(parsed);
        assert!(
            !spec.destructive,
            "{id} goes somewhere; it destroys nothing"
        );
        assert!(
            spec.contexts.contains(Context::List),
            "{id} has to work from the message list, which is where a person is \
             standing when they want to be somewhere else"
        );
        assert!(
            spec.contexts.contains(Context::Sidebar),
            "{id} has to work from the folder list too -- standing there is the \
             likeliest moment to want a different folder, and a key that works \
             one pane over and not here reads as broken"
        );
        assert!(
            !spec.contexts.contains(Context::Composer),
            "{id} must not fire in the composer, where `g` is a letter someone \
             is typing"
        );
    }
}

#[test]
fn search_is_reachable_from_the_folder_list() {
    let spec = registry::get(CommandId::Search);
    assert!(
        spec.contexts.contains(Context::Sidebar),
        "`/` has to open the box from the folder list too -- it is one pane          over from the list, and a key that works there and not here reads as          broken"
    );
    assert!(
        !spec.contexts.contains(Context::Composer),
        "`/` must not fire in the composer, where it is a character someone          is typing"
    );
}

#[test]
fn ids_round_trip_through_strings() {
    for id in CommandId::ALL {
        let text = id.as_str();
        assert_eq!(text.parse::<CommandId>().unwrap(), *id);
        assert_eq!(id.to_string(), text);
    }
    assert!("teleport".parse::<CommandId>().is_err());
}

#[test]
fn ids_serialize_as_their_stable_string() {
    let json = serde_json::to_string(&CommandId::ExpandThread).unwrap();
    assert_eq!(json, "\"expand_thread\"");
    let back: CommandId = serde_json::from_str(&json).unwrap();
    assert_eq!(back, CommandId::ExpandThread);
}

#[test]
fn bindings_do_not_collide_within_a_context() {
    // `alt+BackSpace` means two commands on two platforms, so a collision is two commands both live
    // on some platform.
    for context in Context::ALL {
        let mut seen: BTreeMap<(bool, &str), CommandId> = BTreeMap::new();
        for spec in registry::for_context(*context) {
            for binding in spec.bindings() {
                for platform in [Platform::Freedesktop, Platform::Apple] {
                    let action = postio_core::ActionId::Builtin(spec.id);
                    if !registry::offered_on(action, platform)
                        || !registry::binding_offered_on(action, binding, platform)
                    {
                        continue;
                    }
                    if let Some(other) =
                        seen.insert((platform == Platform::Apple, binding), spec.id)
                    {
                        panic!(
                            "`{binding}` is bound to both `{other}` and `{}` in the \
                             {context} context on {platform:?}",
                            spec.id
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn destructive_commands_offer_a_way_back() {
    for spec in registry::all() {
        if spec.destructive {
            assert_ne!(
                spec.recovery,
                Recovery::None,
                "{} is destructive but offers neither undo nor confirmation",
                spec.id
            );
        }
    }
    assert!(registry::get(CommandId::Archive).destructive);
    assert_eq!(registry::get(CommandId::Archive).recovery, Recovery::Undo);
    assert!(!registry::get(CommandId::Reply).destructive);
}

#[test]
fn recovery_undo_means_the_undo_stack_and_nothing_else_claims_it() {
    // #1481: `Send` claimed `Recovery::Undo` while no `UndoKind` exists for it, so `u` after a send
    // reversed an unrelated archive.
    assert_eq!(
        registry::get(CommandId::Send).recovery,
        Recovery::Window,
        "a send is reversible for a window, not from the undo stack"
    );

    for spec in registry::every_action() {
        if spec.recovery != Recovery::Undo {
            continue;
        }
        let id = spec.id.to_string();
        assert!(
            id != "send" && id != "schedule_send",
            "{id} reaches no undo stack and must not claim `Recovery::Undo`"
        );
    }
}

#[test]
fn context_filtering_drives_the_palette_and_cheat_sheet() {
    let list: Vec<CommandId> = registry::for_context(Context::List).map(|s| s.id).collect();
    assert!(list.contains(&CommandId::Archive));
    assert!(
        !list.contains(&CommandId::Send),
        "send belongs to the composer"
    );

    let composer: Vec<CommandId> = registry::for_context(Context::Composer)
        .map(|s| s.id)
        .collect();
    assert!(composer.contains(&CommandId::Send));
    assert!(
        !composer.contains(&CommandId::Archive),
        "archiving while composing would swallow a keystroke meant for the body"
    );

    for context in Context::ALL {
        assert!(
            registry::get(CommandId::CommandPalette).available_in(*context),
            "the command palette must be reachable from {context}"
        );
        assert!(registry::get(CommandId::Back).available_in(*context));
    }
}

#[test]
fn contexts_round_trip_through_strings() {
    for context in Context::ALL {
        assert_eq!(context.as_str().parse::<Context>().unwrap(), *context);
    }
    // A count, so adding a context is a deliberate act.
    assert_eq!(Context::ALL.len(), 20);
}

#[test]
fn every_registry_entry_can_be_invoked_from_the_palette() {
    for spec in registry::all() {
        let command = Command::default_for(spec.id);
        assert_eq!(
            command.id(),
            spec.id,
            "Command::default_for round trip failed for {}",
            spec.id
        );
    }
}

#[test]
fn commands_carry_their_target() {
    let archive = Command::Archive {
        target: MessageTarget::Selection,
    };
    assert_eq!(archive.id(), CommandId::Archive);
    assert!(archive.is_destructive());

    let explicit = Command::Archive {
        target: MessageTarget::Messages(vec![postio_model::MessageId::new(7)]),
    };
    assert_ne!(archive, explicit);
    assert_eq!(explicit.id(), CommandId::Archive);
}

#[test]
fn move_is_unavailable_in_unified_scope_and_available_in_an_account() {
    let account = Scope::Account(AccountId::new(1));

    let in_account: Vec<CommandId> =
        registry::reachable_in(Context::List, Availability::open(account))
            .filter_map(|action| action.id.builtin())
            .collect();
    assert!(
        in_account.contains(&CommandId::Move),
        "moving into a folder is exactly what an account scope is for"
    );

    let unified: Vec<CommandId> =
        registry::reachable_in(Context::List, Availability::open(Scope::Unified))
            .filter_map(|action| action.id.builtin())
            .collect();
    assert!(
        !unified.contains(&CommandId::Move),
        "a unified view is a view, never a destination: offering Move there \
         promises a folder the user cannot have picked"
    );

    for still_there in [
        CommandId::Archive,
        CommandId::Delete,
        CommandId::Reply,
        CommandId::Flag,
    ] {
        assert!(
            unified.contains(&still_there),
            "{still_there} does not need one account and must survive Unified"
        );
    }
}

#[test]
fn the_scope_blind_listing_still_documents_every_command() {
    let documented: Vec<CommandId> = registry::for_context(Context::List).map(|s| s.id).collect();
    assert!(
        documented.contains(&CommandId::Move),
        "the reference documents the vocabulary, not the current scope"
    );
}

#[test]
fn adding_an_account_is_reachable_wherever_settings_is() {
    let spec = registry::get(CommandId::AddAccount);

    assert_eq!(
        spec.contexts.with(Context::Contacts),
        registry::get(CommandId::Settings).contexts,
        "add account is reached from the same places settings is (ADR 0012 Q1); \
         a narrower set would hide it from a surface that offers settings"
    );
    assert!(
        !spec.contexts.contains(Context::Contacts),
        "Contacts is the one place it is not: `mod+shift+n` is New contact there"
    );
    assert!(
        spec.contexts.contains(Context::Sidebar),
        "ADR 0012 Q1 names the folder list specifically: it is where the \
         account being added will eventually appear"
    );
    assert!(
        !spec.destructive,
        "adding an account destroys nothing, so it must not ask first"
    );
    assert!(
        !spec.requires.contains(Requirement::SingleAccount),
        "any scope can gain an account"
    );
}

#[test]
fn the_account_row_actions_are_commands_in_the_account_list() {
    let expected = [
        (
            CommandId::ToggleAccountEnabled,
            "Return",
            false,
            Recovery::None,
        ),
        (CommandId::RemoveAccount, "Delete", true, Recovery::Undo),
        (CommandId::UpdateCredential, "c", false, Recovery::None),
    ];

    for (id, binding, destructive, recovery) in expected {
        let spec = registry::all()
            .find(|spec| spec.id == id)
            .unwrap_or_else(|| panic!("{id} is not in the registry"));
        assert_eq!(spec.default_binding, binding, "{id}'s default binding");
        assert_eq!(spec.destructive, destructive, "{id}'s destructiveness");
        assert_eq!(spec.recovery, recovery, "{id}'s recovery");
        assert!(
            spec.contexts.contains(Context::Accounts),
            "{id} must be reachable in the account list"
        );
        assert!(
            !spec.contexts.contains(Context::List),
            "{id} must not be reachable from the message list: its target is \
             the focused account row, and there is no such row there"
        );
    }
}

#[test]
fn setting_the_default_account_is_an_accounts_row_bound_to_m() {
    let spec = registry::get(CommandId::SetDefaultAccount);

    assert_eq!(
        spec.default_binding, "m",
        "#960 chose `m` for \"make default\"; the cheat sheet and the palette \
         both read this row"
    );
    assert!(
        spec.contexts.contains(Context::Accounts),
        "it belongs to the accounts page"
    );
    assert_eq!(
        spec.contexts,
        registry::get(CommandId::RebuildAccountIndex).contexts,
        "and to that page alone, exactly as the other four account rows do -- \
         a default marker reachable from the message list would be a fifth \
         meaning nobody asked for"
    );
    assert!(
        !spec.destructive,
        "marking an account is not destructive: nothing is lost and the \
         previous holder is still there"
    );
    assert_eq!(
        spec.recovery,
        Recovery::None,
        "the reversal is the same key on another row, exactly as \
         ToggleAccountEnabled argues -- there is nothing for the undo stack \
         to hold"
    );
}

const WITHOUT_A_STORE: &[CommandId] = &[
    CommandId::CommandPalette,
    CommandId::CheatSheet,
    CommandId::Back,
    CommandId::EditConfig,
    CommandId::Quit,
];

#[test]
fn every_command_but_the_chrome_needs_the_store_open() {
    for id in CommandId::ALL {
        let needs = registry::get(*id).requires.contains(Requirement::StoreOpen);
        let chrome = WITHOUT_A_STORE.contains(id);
        assert_eq!(
            needs,
            !chrome,
            "`{id}` {} the store open, and `WITHOUT_A_STORE` says it {}. A \
             command that reads or writes mail must not be offered before \
             there is mail to read; a command that is pure chrome must not \
             disappear from a window that is perfectly usable.",
            if needs {
                "requires"
            } else {
                "does not require"
            },
            if chrome { "does not" } else { "does" },
        );
    }
}

#[test]
fn the_terminal_composers_own_commands_are_offered_only_in_the_terminal() {
    let window = Availability::open(Scope::Account(AccountId::new(1)));
    let terminal = Availability {
        frontend: postio_core::Frontend::Terminal,
        ..window
    };
    let offered = |state| {
        registry::reachable_in(Context::Composer, state)
            .filter_map(|action| action.id.builtin())
            .collect::<Vec<CommandId>>()
    };
    for id in [CommandId::EditExternally, CommandId::TogglePreview] {
        assert!(!offered(window).contains(&id), "{id} offered in a window");
        assert!(
            offered(terminal).contains(&id),
            "{id} missing in the terminal"
        );
    }
    assert!(
        offered(window).contains(&CommandId::Bold),
        "the rest of the composer is offered in both"
    );
}

#[test]
fn the_vocabulary_before_the_store_is_the_chrome_and_nothing_else() {
    let account = Scope::Account(AccountId::new(1));
    let closed = Availability {
        scope: account,
        store_open: false,
        frontend: postio_core::Frontend::Focus,
    };
    let open = Availability {
        scope: account,
        store_open: true,
        frontend: postio_core::Frontend::Focus,
    };

    let before: Vec<CommandId> = registry::reachable_in(Context::List, closed)
        .filter_map(|action| action.id.builtin())
        .collect();
    let after: Vec<CommandId> = registry::reachable_in(Context::List, open)
        .filter_map(|action| action.id.builtin())
        .collect();

    for absent in [
        CommandId::Archive,
        CommandId::Delete,
        CommandId::Reply,
        CommandId::Compose,
        CommandId::Search,
        CommandId::Refresh,
        CommandId::NextMessage,
    ] {
        assert!(
            !before.contains(&absent),
            "`{absent}` is offered with no store behind the window, and it \
             cannot run: it would either do nothing or reach a store that is \
             not there"
        );
        assert!(
            after.contains(&absent),
            "`{absent}` did not come back once the store opened, so the \
             requirement is not a wait but a removal"
        );
    }

    assert!(
        !before.is_empty(),
        "a window with no store is still a window: the palette and the cheat \
         sheet have to list something, or there is no way to find out that \
         waiting is all there is to do"
    );
    for kept in [CommandId::CommandPalette, CommandId::CheatSheet] {
        assert!(
            before.contains(&kept),
            "`{kept}` is how somebody finds out what is available, so it \
             cannot itself be one of the things that is not"
        );
    }
}

#[test]
fn a_command_can_need_more_than_one_thing_at_once() {
    let requires = registry::get(CommandId::Move).requires;
    assert!(requires.contains(Requirement::SingleAccount));
    assert!(requires.contains(Requirement::StoreOpen));

    let unified_and_open = Availability {
        scope: Scope::Unified,
        store_open: true,
        frontend: postio_core::Frontend::Focus,
    };
    let account_and_closed = Availability {
        scope: Scope::Account(AccountId::new(1)),
        store_open: false,
        frontend: postio_core::Frontend::Focus,
    };
    for unmet in [unified_and_open, account_and_closed] {
        assert!(
            !registry::reachable_in(Context::List, unmet)
                .filter_map(|action| action.id.builtin())
                .any(|id| id == CommandId::Move),
            "Move survived {unmet:?}, so only one of its two requirements is \
             being evaluated"
        );
    }
}

#[test]
fn reader_view_is_a_command_beside_view_original() {
    let spec = registry::get(CommandId::ToggleReaderView);
    assert_eq!(spec.id.as_str(), "toggle_reader_view");
    assert_eq!(spec.title, "Reader view");
    assert_eq!(spec.default_binding, "mod+shift+o");
    assert_eq!(
        spec.contexts,
        registry::get(CommandId::ViewOriginal).contexts
    );
    assert!(!spec.destructive);
    assert_eq!(spec.recovery, Recovery::None);
    assert_eq!(
        Command::default_for(CommandId::ToggleReaderView).id(),
        CommandId::ToggleReaderView
    );
}

#[test]
fn the_reading_commands_are_registered_as_the_contract_says() {
    let reading = registry::get(CommandId::ViewOriginal).contexts;
    for (id, name, title, default, alternates) in [
        (
            CommandId::DarkenMessage,
            "darken_message",
            "Darken this message",
            "alt+d",
            &[][..],
        ),
        (
            CommandId::FindInMessage,
            "find_in_message",
            "Find in message",
            "mod+f",
            &[][..],
        ),
        (
            CommandId::FindNext,
            "find_next",
            "Next match",
            "mod+g",
            &["F3"][..],
        ),
        (
            CommandId::FindPrevious,
            "find_previous",
            "Previous match",
            "mod+shift+g",
            &["shift+F3"][..],
        ),
        (
            CommandId::ZoomIn,
            "zoom_in",
            "Zoom in",
            "mod+plus",
            &["mod+equal", "mod+KP_Add"][..],
        ),
        (
            CommandId::ZoomOut,
            "zoom_out",
            "Zoom out",
            "mod+minus",
            &["mod+KP_Subtract"][..],
        ),
        (
            CommandId::ZoomReset,
            "zoom_reset",
            "Actual size",
            "mod+0",
            &["mod+KP_0"][..],
        ),
    ] {
        let spec = registry::get(id);
        assert_eq!(spec.id.as_str(), name);
        assert_eq!(spec.title, title, "{name}");
        assert_eq!(spec.default_binding, default, "{name}");
        assert_eq!(spec.alternate_bindings, alternates, "{name}");
        let contexts = if id == CommandId::FindInMessage {
            reading.with(Context::Source)
        } else {
            reading
        };
        assert_eq!(spec.contexts, contexts, "{name}");
        assert!(!spec.destructive, "{name}");
        assert_eq!(Command::default_for(id).id(), id);
    }
}

#[test]
fn a_focus_only_command_is_offered_to_focus_and_the_terminal_that_draws_it() {
    use postio_core::registry::{Frontend, RequirementSet};
    let at = |frontend| Availability {
        frontend,
        ..Availability::open(Scope::Unified)
    };
    let apps = [Frontend::Terminal, Frontend::Focus];
    let offered_to = |requirement| {
        let set = RequirementSet::from_slice(&[requirement]);
        apps.into_iter()
            .filter(|app| set.met_by(at(*app)))
            .collect::<Vec<Frontend>>()
    };
    assert_eq!(
        offered_to(Requirement::Focus),
        [Frontend::Terminal, Frontend::Focus],
        "the terminal is Focus drawn in character cells (C29)"
    );
    assert_eq!(offered_to(Requirement::Terminal), [Frontend::Terminal]);
    assert_eq!(
        offered_to(Requirement::Graphical),
        [Frontend::Focus],
        "every app that draws a message as pixels can zoom and darken it"
    );
    let in_apps = |id| {
        let set = registry::get(id).requires;
        apps.into_iter()
            .filter(|app| set.met_by(at(*app)))
            .collect::<Vec<Frontend>>()
    };
    assert_eq!(
        in_apps(CommandId::SwitchTreatment),
        [Frontend::Focus],
        "app colours or the original is a Focus verb that needs pixels"
    );
    assert_eq!(
        in_apps(CommandId::CaptureTask),
        [Frontend::Terminal, Frontend::Focus],
        "a Focus row is reachable in the terminal"
    );
}

#[test]
fn the_search_bar_s_commands_are_offered_on_both_platforms() {
    use postio_config::paths::Platform;
    use postio_core::ActionId;
    for id in [CommandId::ShowAllResults, CommandId::ForgetRecent] {
        let action = ActionId::Builtin(id);
        for platform in [Platform::Apple, Platform::Freedesktop] {
            assert!(
                registry::offered_on(action, platform),
                "{id} on {platform:?}"
            );
        }
        assert!(registry::get(id).available_in(Context::Search), "{id}");
    }
    let offered = |id, binding, platform| {
        registry::binding_offered_on(ActionId::Builtin(id), binding, platform)
    };
    assert!(!offered(
        CommandId::BackToWords,
        "alt+BackSpace",
        Platform::Apple
    ));
    assert!(offered(
        CommandId::BackToWords,
        "alt+BackSpace",
        Platform::Freedesktop
    ));
    assert!(offered(
        CommandId::ForgetRecent,
        "alt+BackSpace",
        Platform::Apple
    ));
    assert!(!offered(
        CommandId::ForgetRecent,
        "alt+BackSpace",
        Platform::Freedesktop
    ));
    assert!(offered(
        CommandId::ForgetRecent,
        "alt+Delete",
        Platform::Freedesktop
    ));
    assert!(!offered(
        CommandId::ForgetRecent,
        "alt+Delete",
        Platform::Apple
    ));
}

#[test]
fn the_results_views_commands_are_offered_on_both_platforms() {
    use postio_config::paths::Platform;
    use postio_core::ActionId;
    for id in [
        CommandId::HistoryBack,
        CommandId::HistoryForward,
        CommandId::ResultsConversations,
        CommandId::ResultsFiles,
        CommandId::ResultsPeople,
    ] {
        let action = ActionId::Builtin(id);
        assert!(registry::offered_on(action, Platform::Apple), "{id}");
        assert!(registry::offered_on(action, Platform::Freedesktop), "{id}");
        assert!(registry::get(id).available_in(Context::Results), "{id}");
    }
    for id in [CommandId::HistoryBack, CommandId::HistoryForward] {
        assert!(registry::get(id).available_in(Context::List), "{id}");
    }
    for id in [
        CommandId::ResultsConversations,
        CommandId::ResultsFiles,
        CommandId::ResultsPeople,
    ] {
        assert_eq!(
            registry::get(id).contexts,
            Context::Results.as_set(),
            "{id}"
        );
    }
}

#[test]
fn the_timeline_steps_are_the_results_views_alone() {
    use postio_config::paths::Platform;
    use postio_core::ActionId;
    for id in [CommandId::StepRangeBack, CommandId::StepRangeForward] {
        let action = ActionId::Builtin(id);
        assert!(registry::offered_on(action, Platform::Apple), "{id}");
        assert!(registry::offered_on(action, Platform::Freedesktop), "{id}");
        assert_eq!(
            registry::get(id).contexts,
            Context::Results.as_set(),
            "{id} is the results view's alone"
        );
    }
    assert_eq!(CommandId::StepRangeBack.as_str(), "step_range_back");
    assert_eq!(CommandId::StepRangeForward.as_str(), "step_range_forward");
}

#[test]
fn quick_look_is_the_results_views_alone() {
    use postio_config::paths::Platform;
    use postio_core::ActionId;
    for id in [
        CommandId::QuickLook,
        CommandId::NextMatch,
        CommandId::PrevMatch,
    ] {
        let action = ActionId::Builtin(id);
        assert!(registry::offered_on(action, Platform::Apple), "{id}");
        assert!(registry::offered_on(action, Platform::Freedesktop), "{id}");
        assert_eq!(
            registry::get(id).contexts,
            Context::Results.as_set(),
            "{id} is the results view's alone"
        );
    }
    assert_eq!(CommandId::QuickLook.as_str(), "quick_look");
    assert_eq!(CommandId::NextMatch.as_str(), "next_match");
    assert_eq!(CommandId::PrevMatch.as_str(), "prev_match");
}

#[test]
fn relaxations_are_the_results_views_alone() {
    use postio_config::paths::Platform;
    use postio_core::ActionId;
    for (id, name) in [
        (CommandId::PickRelaxation1, "pick_relaxation_1"),
        (CommandId::PickRelaxation2, "pick_relaxation_2"),
        (CommandId::PickRelaxation3, "pick_relaxation_3"),
        (CommandId::PickRelaxation4, "pick_relaxation_4"),
    ] {
        let action = ActionId::Builtin(id);
        assert!(registry::offered_on(action, Platform::Apple), "{id}");
        assert!(registry::offered_on(action, Platform::Freedesktop), "{id}");
        assert_eq!(
            registry::get(id).contexts,
            Context::Results.as_set(),
            "{id} is the results view's alone"
        );
        assert_eq!(id.as_str(), name);
    }
    assert!(registry::get(CommandId::BackToWords).available_in(Context::Results));
    assert!(registry::get(CommandId::BackToWords).available_in(Context::Search));
}

#[test]
fn exclude_suggestion_is_the_dropdowns_alone() {
    use postio_config::paths::Platform;
    use postio_core::ActionId;
    let id = CommandId::ExcludeSuggestion;
    let action = ActionId::Builtin(id);
    assert!(registry::offered_on(action, Platform::Apple));
    assert!(registry::offered_on(action, Platform::Freedesktop));
    assert_eq!(registry::get(id).contexts, Context::Search.as_set());
    assert_eq!(id.as_str(), "exclude_suggestion");
}

#[test]
fn saving_a_file_is_the_results_views_alone() {
    use postio_config::paths::Platform;
    use postio_core::ActionId;
    let action = ActionId::Builtin(CommandId::SaveFile);
    assert!(registry::offered_on(action, Platform::Apple));
    assert!(registry::offered_on(action, Platform::Freedesktop));
    assert_eq!(
        registry::get(CommandId::SaveFile).contexts,
        Context::Results.as_set()
    );
    assert_eq!(CommandId::SaveFile.as_str(), "save_file");
}

#[test]
fn view_source_opens_source_and_verification() {
    assert_eq!(
        registry::get(CommandId::ViewSource).title,
        "Source and verification"
    );
    assert_eq!(Context::Source.to_string(), "source");
    assert_eq!("source".parse::<Context>().ok(), Some(Context::Source));
    let titles: BTreeMap<CommandId, &str> = registry::for_context(Context::Source)
        .map(|spec| (spec.id, spec.title))
        .collect();
    for (id, title) in [
        (CommandId::SourceTabVerification, "Verification"),
        (CommandId::SourceTabHeaders, "Headers"),
        (CommandId::SourceTabRaw, "Raw"),
        (CommandId::CopySource, "Copy all"),
        (CommandId::SaveSource, "Save as .eml…"),
        (CommandId::UnfoldPart, "Show"),
    ] {
        assert_eq!(titles.get(&id), Some(&title), "{id}'s command-bar row");
    }
}
