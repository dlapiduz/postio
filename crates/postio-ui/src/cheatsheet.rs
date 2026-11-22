//! The `?` cheat sheet, rendered from the registry and the live `Keymap` so rebinding changes it.
//! Lists what is reachable where the reader stands; the full reference is `docs/keybindings.md`.

use postio_core::{ActionId, Availability, Context, ContextSet, Keymap, registry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The command, or `None` for the one box's prefixes (`>`, `#`, `@`), which are typed inside a
    /// surface.
    pub id: Option<ActionId>,
    pub title: &'static str,
    /// The binding in force; `None` means palette-only.
    pub binding: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: &'static str,
    pub rows: Vec<Row>,
}

const EVERYWHERE: &str = "Everywhere";

const IN_THE_BOX: &str = "In the search box";

fn heading(context: Context) -> &'static str {
    match context {
        Context::List => "Message list",
        Context::Conversation => "Thread",
        Context::Reader => "Reading",
        Context::Composer => "Composing",
        Context::Search => "Search",
        Context::Palette => "Command palette",
        // "Folders", not "Sidebar": UI copy says folder.
        Context::Sidebar => "Folders",
        Context::Parts => "Parts",
        // The only part of the settings panel these keys reach (ADR 0005 Q6c).
        Context::Accounts => "Accounts",
        Context::Keys => "Keybindings",
        Context::Picker => "Pickers",
        Context::Digest => "Digests",
        Context::Filtered => "Filtered",
        Context::Capture => "Capture",
        Context::Results => "Search results",
        Context::Contacts => "Contacts",
        Context::ContactPopover => "Contact popover",
        Context::Outline => "Thread outline",
        Context::ForwardChooser => "Forward",
    }
}

pub fn sections(keymap: &Keymap, context: Context, state: Availability) -> Vec<Section> {
    let mut everywhere = Section {
        title: EVERYWHERE,
        rows: Vec::new(),
    };
    let mut here = Section {
        title: heading(context),
        rows: Vec::new(),
    };

    // Not what this platform has no surface for: the sheet would teach a key the app ignores.
    for spec in registry::reachable_in(context, state).filter(|spec| keymap.offers(spec.id)) {
        let ActionId::Builtin(id) = spec.id;
        let row = Row {
            id: Some(spec.id),
            title: spec.title,
            binding: keymap.binding(id).map(str::to_owned),
        };

        if spec.contexts == ContextSet::ANY {
            everywhere.rows.push(row);
        } else {
            here.rows.push(row);
        }
    }

    let sections = vec![here];

    let mut out = Vec::with_capacity(sections.len() + 2);
    if !everywhere.rows.is_empty() {
        out.push(everywhere);
    }
    if let Some(prefixes) = prefix_section() {
        out.push(prefixes);
    }
    out.extend(sections.into_iter().filter(|s| !s.rows.is_empty()));
    out
}

/// Generated from `crate::finder::MODES`, in sheet order, so a new mode appears with no other edit.
/// Search is skipped: it has no prefix.
fn prefix_section() -> Option<Section> {
    let rows: Vec<Row> = crate::finder::MODES
        .iter()
        .filter_map(|mode| {
            Some(Row {
                id: None,
                title: mode.purpose,
                binding: Some(mode.prefix?.to_string()),
            })
        })
        .collect();
    (!rows.is_empty()).then_some(Section {
        title: IN_THE_BOX,
        rows,
    })
}

/// Read aloud, a separate title and key become "Archive a"; this is a sentence.
pub fn spoken(row: &Row) -> String {
    match &row.binding {
        Some(binding) => format!("{}, press {binding}", row.title),
        None => format!("{}, no keyboard shortcut", row.title),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_core::{CommandId, Scope};
    use postio_model::AccountId;

    fn defaults() -> Keymap {
        Keymap::resolve(&postio_config::KeyBindings::default())
    }

    #[test]
    fn no_heading_says_conversation() {
        for context in Context::ALL {
            assert!(
                !heading(*context).to_lowercase().contains("conversation"),
                "{context:?} is headed {:?}",
                heading(*context)
            );
        }
    }

    #[test]
    fn a_command_the_platform_does_not_offer_is_not_taught_there() {
        use postio_config::paths::Platform;
        let (context, state) = in_the_list();
        for platform in [Platform::Freedesktop, Platform::Apple] {
            let keymap = Keymap::resolve_on(&postio_config::KeyBindings::default(), platform);
            let taught = sections(&keymap, context, state)
                .iter()
                .flat_map(|section| section.rows.iter())
                .any(|row| row.id == Some(ActionId::Builtin(CommandId::NextScope)));
            assert_eq!(
                taught,
                postio_core::registry::offered_on(
                    ActionId::Builtin(CommandId::NextScope),
                    platform
                ),
                "{platform:?}"
            );
        }
    }

    fn in_the_list() -> (Context, Availability) {
        (
            Context::List,
            Availability::open(Scope::Account(AccountId::new(1))),
        )
    }

    #[test]
    fn every_reachable_command_appears_exactly_once() {
        let (context, scope) = in_the_list();
        let keymap = defaults();
        let listed: Vec<ActionId> = sections(&keymap, context, scope)
            .into_iter()
            .flat_map(|section| section.rows)
            .filter_map(|row| row.id)
            .collect();
        let offered =
            || registry::reachable_in(context, scope).filter(|spec| keymap.offers(spec.id));

        for spec in offered() {
            let count = listed.iter().filter(|id| **id == spec.id).count();
            assert_eq!(
                count, 1,
                "`{}` appears {count} times; a reader wants one answer",
                spec.id
            );
        }
        assert_eq!(listed.len(), offered().count());
    }

    #[test]
    fn the_sheet_lists_only_what_is_reachable_here() {
        let (context, scope) = in_the_list();
        let listed: Vec<ActionId> = sections(&defaults(), context, scope)
            .into_iter()
            .flat_map(|section| section.rows)
            .filter_map(|row| row.id)
            .collect();

        assert!(
            listed.contains(&ActionId::Builtin(CommandId::Archive)),
            "archiving is the list's whole job"
        );
        assert!(
            !listed.contains(&ActionId::Builtin(CommandId::Send)),
            "sending belongs to the composer; offering it here teaches a key \
             that does nothing"
        );
    }

    /// Every composer command reaches both the palette and the sheet, with its key.
    #[test]
    fn every_composer_command_is_in_the_palette_and_the_sheet() {
        let keymap = defaults();
        let scope = Availability::open(Scope::Account(AccountId::new(1)));

        let reachable: Vec<_> = postio_core::registry::reachable_in(Context::Composer, scope)
            .filter(|spec| keymap.offers(spec.id))
            .collect();
        assert!(
            !reachable.is_empty(),
            "the registry offers nothing in the composer, so this test would \
             pass by having nothing to check"
        );

        let in_palette: Vec<ActionId> =
            crate::palette::entries(&keymap, Context::Composer, scope, "")
                .into_iter()
                .map(|entry| entry.id)
                .collect();
        let in_sheet: Vec<ActionId> = sections(&keymap, Context::Composer, scope)
            .into_iter()
            .flat_map(|section| section.rows)
            .filter_map(|row| row.id)
            .collect();

        for spec in &reachable {
            assert!(
                in_palette.contains(&spec.id),
                "{:?} is reachable in the composer but not in the palette: a \
                 command nobody can find is a command that does not exist",
                spec.id
            );
            assert!(
                in_sheet.contains(&spec.id),
                "{:?} is reachable in the composer but not on the `?` sheet",
                spec.id
            );
        }

        let keyless: Vec<ActionId> = sections(&keymap, Context::Composer, scope)
            .into_iter()
            .flat_map(|section| section.rows)
            .filter(|row| row.id.is_some() && row.binding.is_none())
            .filter_map(|row| row.id)
            .collect();
        assert!(
            keyless.is_empty(),
            "these composer rows name a command and no key: {keyless:?}"
        );
    }

    #[test]
    fn a_window_with_no_store_is_taught_only_what_it_can_do() {
        // Before the store opens the sheet must not teach keys that cannot work (#1114).
        let ids = |state| -> Vec<ActionId> {
            sections(&defaults(), Context::List, state)
                .into_iter()
                .flat_map(|section| section.rows)
                .filter_map(|row| row.id)
                .collect()
        };
        let account = Scope::Account(AccountId::new(1));
        let waiting = ids(Availability {
            store_open: false,
            ..Availability::open(account)
        });
        let open = ids(Availability::open(account));

        for mail in [CommandId::Archive, CommandId::Reply, CommandId::Search] {
            assert!(
                !waiting.contains(&ActionId::Builtin(mail)),
                "the sheet teaches `{mail}` at a window with no store behind it"
            );
            assert!(open.contains(&ActionId::Builtin(mail)));
        }
        assert!(
            waiting.contains(&ActionId::Builtin(CommandId::CheatSheet)),
            "and the sheet still lists the key that opened it, or there is \
             no way back to it"
        );
    }

    #[test]
    fn a_unified_view_lists_no_move() {
        let ids = |scope| -> Vec<ActionId> {
            sections(&defaults(), Context::List, scope)
                .into_iter()
                .flat_map(|section| section.rows)
                .filter_map(|row| row.id)
                .collect()
        };

        assert!(
            ids(Availability::open(Scope::Account(AccountId::new(1))))
                .contains(&ActionId::Builtin(CommandId::Move)),
            "an account view can name a destination"
        );
        assert!(
            !ids(Availability::open(Scope::Unified)).contains(&ActionId::Builtin(CommandId::Move)),
            "a unified view cannot, so the sheet must not teach `m` there"
        );
    }

    #[test]
    fn a_command_reachable_everywhere_is_filed_under_everywhere() {
        let (context, scope) = in_the_list();
        let sections = sections(&defaults(), context, scope);
        let everywhere = sections
            .iter()
            .find(|section| section.title == EVERYWHERE)
            .expect("an Everywhere section");

        let ids: Vec<ActionId> = everywhere.rows.iter().filter_map(|row| row.id).collect();
        assert!(ids.contains(&ActionId::Builtin(CommandId::CommandPalette)));
        assert!(ids.contains(&ActionId::Builtin(CommandId::Back)));
        assert_eq!(
            sections.first().map(|section| section.title),
            Some(EVERYWHERE),
            "and it leads, because it is what applies wherever the reader is"
        );
    }

    #[test]
    fn a_reachable_command_is_filed_under_the_surface_the_reader_is_on() {
        let from_the_list = sections(&defaults(), in_the_list().0, in_the_list().1);
        let list = from_the_list
            .iter()
            .find(|section| section.title == heading(Context::List))
            .expect("a Message list section");

        assert!(
            list.rows
                .iter()
                .any(|row| row.id == Some(ActionId::Builtin(CommandId::Archive))),
            "archive works in the list, the thread and the reader; standing in \
             the list, the sheet says so under the list"
        );

        let from_the_reader = sections(&defaults(), Context::Reader, in_the_list().1);
        let reading = from_the_reader
            .iter()
            .find(|section| section.title == heading(Context::Reader))
            .expect("a Reading section");
        assert!(
            reading
                .rows
                .iter()
                .any(|row| row.id == Some(ActionId::Builtin(CommandId::Archive))),
            "the heading has to name where the reader is, not where the \
             command was first declared"
        );
    }

    #[test]
    fn no_section_is_printed_empty() {
        for section in sections(&defaults(), in_the_list().0, in_the_list().1) {
            assert!(
                !section.rows.is_empty(),
                "`{}` has no keys under it",
                section.title
            );
        }
    }

    #[test]
    fn rows_carry_the_binding_in_force() {
        let archive = sections(&defaults(), in_the_list().0, in_the_list().1)
            .into_iter()
            .flat_map(|section| section.rows)
            .find(|row| row.id == Some(ActionId::Builtin(CommandId::Archive)))
            .expect("archive");

        assert_eq!(archive.binding.as_deref(), Some("e"));
    }

    #[test]
    fn rebinding_a_key_changes_the_sheet() {
        let mut overrides = postio_config::KeyBindings::default();
        overrides
            .overrides_mut()
            .insert("archive".to_owned(), "y".to_owned());

        let archive = sections(
            &Keymap::resolve(&overrides),
            in_the_list().0,
            in_the_list().1,
        )
        .into_iter()
        .flat_map(|section| section.rows)
        .find(|row| row.id == Some(ActionId::Builtin(CommandId::Archive)))
        .expect("archive");

        assert_eq!(
            archive.binding.as_deref(),
            Some("y"),
            "with no code edit: the sheet reads the live keymap"
        );
    }

    #[test]
    fn a_command_with_no_key_is_still_listed() {
        // Commands only: the box's prefixes are typed characters, not bindings, so an empty keymap
        // does not silence them.
        let listed: Vec<Row> = sections(&Keymap::default(), in_the_list().0, in_the_list().1)
            .into_iter()
            .flat_map(|section| section.rows)
            .filter(|row| row.id.is_some())
            .collect();

        assert_eq!(
            listed.len(),
            registry::reachable_in(in_the_list().0, in_the_list().1).count()
        );
        assert!(
            listed.iter().all(|row| row.binding.is_none()),
            "and \"this exists and has no key\" is an answer worth printing"
        );
    }

    #[test]
    fn a_row_reads_as_a_sentence() {
        assert_eq!(
            spoken(&Row {
                id: Some(ActionId::Builtin(CommandId::Archive)),
                title: "Archive",
                binding: Some("a".to_owned()),
            }),
            "Archive, press a"
        );
        assert_eq!(
            spoken(&Row {
                id: Some(ActionId::Builtin(CommandId::Archive)),
                title: "Archive",
                binding: None,
            }),
            "Archive, no keyboard shortcut"
        );
    }

    #[test]
    fn the_sheet_teaches_every_prefix_the_one_box_understands() {
        let section = sections(&defaults(), in_the_list().0, in_the_list().1)
            .into_iter()
            .find(|section| section.title == IN_THE_BOX)
            .expect("the sheet has no section for the box's prefixes");

        for mode in crate::finder::MODES {
            let Some(prefix) = mode.prefix else {
                continue;
            };
            let row = section
                .rows
                .iter()
                .find(|row| row.binding.as_deref() == Some(&prefix.to_string()))
                .unwrap_or_else(|| panic!("`{prefix}` is not on the sheet"));
            assert_eq!(
                row.title, mode.purpose,
                "`{prefix}` is listed as something other than what it does"
            );
            assert!(
                row.id.is_none(),
                "a prefix is not a command and must not claim to run one"
            );
        }

        assert_eq!(
            section.rows.len(),
            crate::finder::MODES
                .iter()
                .filter(|m| m.prefix.is_some())
                .count(),
            "the section is not generated from the MODES table — a mode added \
             later would not appear"
        );
    }

    #[test]
    fn search_is_not_listed_as_a_prefix_because_it_has_none() {
        assert_eq!(
            crate::finder::MODES[0].prefix,
            None,
            "search is the prefixless mode, and first in the table"
        );
        let section = sections(&defaults(), in_the_list().0, in_the_list().1)
            .into_iter()
            .find(|section| section.title == IN_THE_BOX)
            .expect("a prefix section");
        assert!(
            !section
                .rows
                .iter()
                .any(|row| row.title == crate::finder::MODES[0].purpose),
            "search is listed as though it needed a prefix typed to reach it"
        );
    }

    #[test]
    fn the_sheet_prints_only_everywhere_and_the_readers_own_surface() {
        for context in [Context::List, Context::Composer, Context::Sidebar] {
            let printed = sections(&defaults(), context, in_the_list().1);
            let titles: Vec<&str> = printed.iter().map(|section| section.title).collect();

            let expected: Vec<&str> = [EVERYWHERE, IN_THE_BOX, heading(context)]
                .into_iter()
                .filter(|title| titles.contains(title))
                .collect();
            assert_eq!(
                titles, expected,
                "after filtering to what is reachable there are only two \
                 useful headings — what works everywhere, and what works \
                 here. A third means the filter let something through."
            );
        }
    }
}
