//! The command registry is the single source of truth behind the keymap, the
//! command palette, the `?` cheat sheet, the context menu and the focused-row
//! key hints. These tests encode the structural guarantees docs/PRODUCT.md §8 asks for:
//! every command has an id, a human title and a default binding, so the three
//! surfaces cannot drift apart.

use std::collections::{BTreeMap, BTreeSet};

use postio_config::KeyBindings;
use postio_config::paths::Platform;
use postio_core::config::Keymap;
use postio_core::{Command, CommandId, Context, MessageTarget, Recovery, Scope, registry};
use postio_model::AccountId;

/// The id vocabulary `postio-config`'s `DEFAULT_BINDINGS` already fixed.
/// `[keys]` in `config.toml` references commands by these strings, so they are
/// part of the file format: changing one breaks user configuration.
///
/// Kept as a literal copy rather than a dependency on `postio-config`, on
/// purpose — the point of the test is that the two crates agree without one
/// deriving from the other.
const CONFIG_BINDINGS: &[(&str, &str)] = &[
    ("next_message", "j"),
    ("prev_message", "k"),
    ("open_message", "Return"),
    ("back", "Escape"),
    ("thread", "t"),
    ("archive", "a"),
    ("archive_thread", "A"),
    ("undo", "u"),
    ("reply", "e"),
    ("reply_all", "E"),
    ("forward", "f"),
    ("compose", "c"),
    ("search", "/"),
    ("command_palette", "mod+k"),
    ("cheat_sheet", "?"),
    ("settings", "mod+comma"),
    ("edit_config", "mod+e"),
];

#[test]
fn registry_is_enumerable_and_non_empty() {
    let all: Vec<_> = registry::all().collect();
    assert!(
        all.len() >= CONFIG_BINDINGS.len(),
        "the registry must cover at least every command config knows about"
    );
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
fn ids_and_defaults_match_the_config_crate_vocabulary() {
    for (id, key) in CONFIG_BINDINGS {
        let parsed: CommandId = id
            .parse()
            .unwrap_or_else(|_| panic!("registry is missing command id `{id}` used by [keys]"));
        assert_eq!(
            registry::get(parsed).default_binding,
            *key,
            "default binding for `{id}` disagrees with postio-config"
        );
    }
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
        ("thread", "t"),
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
    // `Accounts` (#471) is the ninth and widened it to a `u16`. The ceiling
    // is no longer written down twice -- `context.rs`'s
    // `every_context_fits_the_set` derives it from the integer itself, so
    // this is only the deliberate-act tripwire.
    assert_eq!(Context::ALL.len(), 9);
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

    let in_account: Vec<CommandId> = registry::reachable_in(Context::List, account)
        .filter_map(|action| action.id.builtin())
        .collect();
    assert!(
        in_account.contains(&CommandId::Move),
        "moving into a folder is exactly what an account scope is for"
    );

    let unified: Vec<CommandId> = registry::reachable_in(Context::List, Scope::Unified)
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
    assert_eq!(spec.requires, None, "any scope can gain an account");
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
