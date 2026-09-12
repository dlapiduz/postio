//! The command registry is the single source of truth behind the keymap, the
//! command palette, the `?` cheat sheet, the context menu and the focused-row
//! key hints. These tests encode the structural guarantees docs/PRODUCT.md §8 asks for:
//! every command has an id, a human title and a default binding, so the three
//! surfaces cannot drift apart.
//!
//! This file used to carry `CONFIG_BINDINGS`, a hand-copied 16-entry table
//! kept "rather than a dependency on `postio-config`, on purpose — the point
//! of the test is that the two crates agree without one deriving from the
//! other". Both crates having their own table is what #1227 turned out to be:
//! the copies drifted to 16, 23 and 79 entries, and nothing failed. The
//! registry is the only default table now, so there is nothing left to agree
//! with — what the canvas settled on is asserted against the *resolved*
//! keymap below, which is the key a finger actually presses.

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
fn bindings_are_the_ones_the_canvas_settled_on() {
    // docs/PRODUCT.md §8 records these as the resolved bindings; the canvas is
    // where they were settled, over an earlier brief that proposed `r` reply.
    let expected = [
        ("reply", "e"),
        ("archive", "a"),
        ("archive_thread", "A"),
        ("undo", "u"),
        ("compose", "c"),
        ("search", "/"),
        ("command_palette", "ctrl+k"),
        ("cheat_sheet", "?"),
        ("back", "Escape"),
        ("next_message", "j"),
        ("prev_message", "k"),
        ("prev_view", "h"),
        ("first_message", "g g"),
        ("last_message", "G"),
    ];
    // Against the *resolved* table, not the registry literal: what the canvas
    // settled on is the key a user presses, and since #669 the registry spells
    // that `mod+k` so a Mac can render the same decision as Command.
    let keymap = Keymap::resolve_on(&KeyBindings::default(), Platform::Freedesktop);
    for (id, key) in expected {
        let parsed: CommandId = id.parse().expect("known command id");
        assert_eq!(keymap.binding(parsed), Some(key), "binding for {id}");
    }
    // `l` opens, as the canvas navigation set requires, without displacing the
    // `Return` default that config.toml already documents.
    assert!(
        registry::get(CommandId::OpenMessage)
            .bindings()
            .any(|b| b == "l"),
        "`l` must open the focused message"
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
    let json = serde_json::to_string(&CommandId::ArchiveThread).unwrap();
    assert_eq!(json, "\"archive_thread\"");
    let back: CommandId = serde_json::from_str(&json).unwrap();
    assert_eq!(back, CommandId::ArchiveThread);
}

#[test]
fn bindings_do_not_collide_within_a_context() {
    for context in Context::ALL {
        let mut seen: BTreeMap<&str, CommandId> = BTreeMap::new();
        for spec in registry::for_context(*context) {
            for binding in spec.bindings() {
                if let Some(other) = seen.insert(binding, spec.id) {
                    panic!(
                        "`{binding}` is bound to both `{other}` and `{}` in the \
                         {context} context",
                        spec.id
                    );
                }
            }
        }
    }
}

#[test]
fn destructive_commands_offer_a_way_back() {
    // docs/PRODUCT.md §1: destructive operations require appropriate confirmation/undo.
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
    // #1481. `Recovery::Undo` has one meaning -- "reversible from the undo
    // stack, and `u` works" -- and `Send` claimed it while nothing recorded a
    // send there: `UndoKind` has no variant for one. So the registry promised
    // a key that did nothing, and worse than nothing, since `u` after a send
    // reverses whatever unrelated archive was underneath.
    //
    // A send *is* reversible, until the drainer takes it. It is simply not
    // reversible from that stack, which is what `Recovery::Window` says.
    assert_eq!(
        registry::get(CommandId::Send).recovery,
        Recovery::Window,
        "a send is reversible for a window, not from the undo stack"
    );

    // And the rule that makes the distinction worth having: every command
    // claiming `Undo` must be a thing the undo stack can actually hold, which
    // means a `UndoKind` exists for it. Stated as a list because the stack
    // takes message operations and the check cannot see across crates.
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

    // The palette is reachable from everywhere, or it is not universal.
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
    // A count, so adding a context is a deliberate act rather than something
    // that happens on the way past. It was 8 and the ceiling was the same
    // number, because `ContextSet` packed one bit per context into a `u8`;
    // `Accounts` (#471) is the ninth and widened it to a `u16`. `Keys`
    // (#881) is the tenth, still inside that widened ceiling. The ceiling
    // is no longer written down twice -- `context.rs`'s
    // `every_context_fits_the_set` derives it from the integer itself, so
    // this is only the deliberate-act tripwire.
    assert_eq!(Context::ALL.len(), 10);
}

#[test]
fn every_registry_entry_can_be_invoked_from_the_palette() {
    // Selecting a palette row yields a command carrying no more context than the
    // row itself had; app state resolves `Selection` and the `None` payloads.
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

// ---------------------------------------------------------------------------
// Availability that depends on state, not on which surface has focus
// ---------------------------------------------------------------------------

/// `Move` is the first command whose availability turns on *state* rather
/// than [`Context`], and ADR 0005 Q4 asks for the shape to be settled here
/// rather than special-cased at each surface (#182).
///
/// A destination has to be one mailbox in one account. In `Scope::Unified`
/// there is no such thing — the view spans every enabled account — so `Move`
/// is *unavailable*, not a no-op that silently does nothing. The registry
/// evaluates that, so the palette, the cheat sheet and the key hints all
/// agree without any of them knowing why.
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

    // Everything else the list can do is untouched. A state predicate that
    // quietly narrowed the whole surface would be the worse bug.
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

/// The scope-blind form still answers for everything, because
/// `docs/keybindings.md` documents the whole vocabulary rather than one
/// session's state — a reader looking up `m` must find it.
#[test]
fn the_scope_blind_listing_still_documents_every_command() {
    let documented: Vec<CommandId> = registry::for_context(Context::List).map(|s| s.id).collect();
    assert!(
        documented.contains(&CommandId::Move),
        "the reference documents the vocabulary, not the current scope"
    );
}

/// Adding a second account is a *command*, not a button somewhere.
///
/// ADR 0012 Q1 decided the entry point that way, and `docs/ARCHITECTURE.md`
/// §2 says why it has to be: a command that is not in the registry does not
/// exist. It would be in neither the palette nor the `?` cheat sheet, which
/// is exactly where a keyboard-first user looks for "add another account"
/// before they go hunting in a settings panel.
#[test]
fn adding_an_account_is_reachable_wherever_settings_is() {
    let spec = registry::get(CommandId::AddAccount);

    assert_eq!(
        spec.contexts,
        registry::get(CommandId::Settings).contexts,
        "add account is reached from the same places settings is (ADR 0012 Q1); \
         a narrower set would hide it from a surface that offers settings"
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
    // ADR 0005 Q6c, #471. #464 gave each account row three affordances as a
    // GtkSimpleActionGroup, reachable by mouse and by Tab but not by the
    // palette, the cheat sheet or a bindable key -- because none of them were
    // registry entries. The registry is what makes those three surfaces work,
    // so the fix is entries, and this is the table the ADR settled.
    let expected = [
        (
            CommandId::ToggleAccountEnabled,
            "Return",
            false,
            Recovery::None,
        ),
        // Destructive, and the only one of the three that is: it soft-deletes
        // an account. `d` matches DeleteSavedSearch's spelling in the
        // neighbouring list, which is the same verb on the same shape of row.
        (CommandId::RemoveAccount, "d", true, Recovery::Undo),
        // `c` for credential. ADR 0005 Q6c asked for no binding at all; that
        // rested on "ten commands already have none", and none do. PRODUCT.md
        // §8 requires one of every command, so it has one.
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

/// The default-account marker is set from a keyboard row like the other four
/// account commands (#960).
///
/// `Context::Accounts` already carries `Return`, `d`, `c` and `r`, so this is
/// a fifth row in the registry rather than a new surface — which is what makes
/// the palette entry, the cheat-sheet row and the focused-row key hint come
/// from one place and stay in step. `m` for "make default", free within this
/// context because `Move`'s `m` is scoped to the message surfaces.
///
/// `bindings_do_not_collide_within_a_context` is what would catch the shadow
/// if that ever stopped being true; this test is the decision itself.
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

// ---------------------------------------------------------------------------
// Availability before the store is open (#1114)
// ---------------------------------------------------------------------------

/// The commands that go on meaning something with no store behind the window.
///
/// #1114's rule, and the list is short on purpose: **a window on screen with
/// no store behind it must not offer verbs that cannot run.** What survives is
/// the chrome — how you find out what you can do, how you leave what you
/// opened, and where the keyboard is — none of which reads or writes mail.
///
/// `EditConfig` is here because it is the one *repair* that needs no store: a
/// start held up by a keyring or a config problem is exactly when somebody
/// wants their `config.toml`, and it opens a file in an editor.
const WITHOUT_A_STORE: &[CommandId] = &[
    CommandId::CommandPalette,
    CommandId::CheatSheet,
    CommandId::Back,
    CommandId::ToggleSidebar,
    CommandId::CyclePane,
    CommandId::CyclePaneBack,
    CommandId::EditConfig,
];

/// Every command decides, and a new one cannot forget to.
///
/// The registry is the single source of the keyboard, the palette and the
/// cheat sheet, so this is the one place the decision has to be recorded —
/// and an assertion over the whole table is what makes the next command
/// author answer the question rather than inherit whatever `requires` happens
/// to default to.
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

/// What the palette and the cheat sheet list before the store is open.
///
/// They both go through [`registry::reachable_in`], so asserting here is
/// asserting for both — which is the whole reason the requirement is data on
/// the row rather than a check at each surface.
#[test]
fn the_vocabulary_before_the_store_is_the_chrome_and_nothing_else() {
    let account = Scope::Account(AccountId::new(1));
    let closed = Availability {
        scope: account,
        store_open: false,
    };
    let open = Availability {
        scope: account,
        store_open: true,
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

/// `Move` needs *both*, and one requirement per row could not say so.
///
/// It was the only row with a requirement before #1114 and it is the row that
/// proves the field had to become a set: a unified view has no destination to
/// name, and a window with no store has no folder to move into either.
#[test]
fn a_command_can_need_more_than_one_thing_at_once() {
    let requires = registry::get(CommandId::Move).requires;
    assert!(requires.contains(Requirement::SingleAccount));
    assert!(requires.contains(Requirement::StoreOpen));

    let unified_and_open = Availability {
        scope: Scope::Unified,
        store_open: true,
    };
    let account_and_closed = Availability {
        scope: Scope::Account(AccountId::new(1)),
        store_open: false,
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
