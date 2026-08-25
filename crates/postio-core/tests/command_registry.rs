//! The command registry is the single source of truth behind the keymap, the
//! command palette, the `?` cheat sheet, the context menu and the focused-row
//! key hints. These tests encode the structural guarantees docs/PRODUCT.md §8 asks for:
//! every command has an id, a human title and a default binding, so the three
//! surfaces cannot drift apart.

use std::collections::{BTreeMap, BTreeSet};

use postio_core::{Command, CommandId, Context, MessageTarget, Recovery, registry};

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
    ("command_palette", "ctrl+k"),
    ("cheat_sheet", "?"),
    ("settings", "ctrl+comma"),
    ("edit_config", "ctrl+e"),
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
    for (id, key) in expected {
        let parsed: CommandId = id.parse().expect("known command id");
        assert_eq!(
            registry::get(parsed).default_binding,
            key,
            "binding for {id}"
        );
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
fn the_thread_view_toggles_are_only_meaningful_in_the_thread() {
    // postio-yzc: the unread filter and the order toggle are properties of
    // the thread column itself, so binding them anywhere else would let a
    // key do something in a context with nothing for it to act on.
    for id in [CommandId::ToggleThreadUnread, CommandId::ToggleThreadOrder] {
        for context in Context::ALL {
            assert_eq!(
                registry::get(id).available_in(*context),
                *context == Context::Thread,
                "{id} should be reachable in Thread and nowhere else, but \
                 {context} disagrees"
            );
        }
    }
}

#[test]
fn contexts_round_trip_through_strings() {
    for context in Context::ALL {
        assert_eq!(context.as_str().parse::<Context>().unwrap(), *context);
    }
    // A count, so adding a context is a deliberate act rather than something
    // that happens on the way past. `ContextSet` packs one bit per context
    // into a `u8`, so this is also the ceiling: an eighth is the last that
    // fits, and a ninth needs the representation widened first.
    assert_eq!(Context::ALL.len(), 8);
    assert!(
        Context::ALL.len() <= 8,
        "ContextSet is a u8; widen it before adding another context"
    );
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

// -- Scope gating (#182, ADR 0005 Q4) ----------------------------------------
//
// Move is the first command whose availability depends on *state* rather than
// `Context`: it has no meaning in the unified scope, because a unified view
// is not a mailbox and "move to..." from it names no destination tree. The
// gate is a registry predicate -- the same machinery that keeps a composer
// command out of the message list -- so the palette, the cheat sheet and the
// key hints cannot drift apart about it.

#[test]
fn move_is_unavailable_in_the_unified_scope() {
    use postio_core::state::Scope;

    let in_account: Vec<_> = registry::reachable_in(
        Context::List,
        Scope::Account(postio_model::ids::AccountId::new(1)),
    )
    .map(|spec| spec.id)
    .collect();
    let in_unified: Vec<_> = registry::reachable_in(Context::List, Scope::Unified)
        .map(|spec| spec.id)
        .collect();

    let move_id = postio_core::ActionId::Builtin(CommandId::Move);
    assert!(
        in_account.contains(&move_id),
        "Move must stay reachable in a real account: {in_account:?}"
    );
    assert!(
        !in_unified.contains(&move_id),
        "Move offered in Unified is a destination picker over no tree"
    );
}

#[test]
fn unified_hides_nothing_else() {
    use postio_core::state::Scope;

    // The gate exists for commands that *name a destination inside one
    // account's tree*. Archive, delete and flag all act per message, and
    // every message knows its account -- so they stay. A second command
    // joining Move here should be a decision, not a drift; this test is
    // where that decision becomes visible.
    for context in Context::ALL {
        let in_account: BTreeSet<_> = registry::reachable_in(
            *context,
            Scope::Account(postio_model::ids::AccountId::new(1)),
        )
        .map(|spec| spec.id)
        .collect();
        let in_unified: BTreeSet<_> = registry::reachable_in(*context, Scope::Unified)
            .map(|spec| spec.id)
            .collect();

        let hidden: Vec<_> = in_account.difference(&in_unified).collect();
        assert!(
            hidden
                .iter()
                .all(|id| **id == postio_core::ActionId::Builtin(CommandId::Move)),
            "Unified hides more than Move in {context:?}: {hidden:?}"
        );
    }
}

#[test]
fn account_scope_equals_todays_reachability() {
    use postio_core::state::Scope;

    // "Existing single-account behaviour unchanged": with a real account on
    // screen, the gated iterator is exactly the ungated one.
    for context in Context::ALL {
        let gated: Vec<_> = registry::reachable_in(
            *context,
            Scope::Account(postio_model::ids::AccountId::new(1)),
        )
        .map(|spec| spec.id)
        .collect();
        let ungated: Vec<_> = registry::reachable(*context).map(|spec| spec.id).collect();
        assert_eq!(gated, ungated, "in {context:?}");
    }
}
