//! Focus's key map, grouped (specs/007-postio-focus research R4, screen 20).
//!
//! The key map lists every key Postio Focus offers, in the groups the design
//! draws: Move and select, Open, Act, Invites, Go and find, In search,
//! Digests and filtering, and Obsidian once it exists. This table says only
//! which group a command sits in. The key is the registry's, read through
//! the keymap in force, so a `[keys]` rebind shows in the key map with no
//! change here -- and a table that holds no bindings is not a second
//! binding table (`scripts/checks/check-one-default-binding-table.py`).
//!
//! # Why a `match`, and not a field
//!
//! A `group` field on every `CommandSpec` would edit every row of the
//! registry for something one app reads (research R4 rejected it for that
//! reason). [`group`] is exhaustive over [`CommandId`] instead, the way the
//! menu sections are (`postio_core::menu`): a new command does not compile
//! until somebody says where Focus's key map puts it, or that it puts it
//! nowhere.
//!
//! No toolkit here. Focus's key map dialog draws these groups.

use postio_core::{CommandId, Context};

/// A group of Focus's key map, in the order the key map draws them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// The cursor, the selection, and the way out.
    MoveAndSelect,
    /// Opening a message, and what is in it.
    Open,
    /// The verbs, on the row under the cursor or on the selection.
    Act,
    /// Answering an invitation from its row.
    Invites,
    /// Going somewhere, and finding something.
    GoAndFind,
    /// The keys a search answers to.
    InSearch,
    /// Digests, Filtered, and the has-action filter.
    DigestsAndFiltering,
    /// Tasks and notes into the vault (milestone 3): nothing yet.
    Obsidian,
}

impl Group {
    /// Every group, in the key map's order.
    pub const ALL: [Group; 8] = [
        Group::MoveAndSelect,
        Group::Open,
        Group::Act,
        Group::Invites,
        Group::GoAndFind,
        Group::InSearch,
        Group::DigestsAndFiltering,
        Group::Obsidian,
    ];

    /// The heading the key map draws (contracts/focus-surface.md, screen 20).
    pub fn title(self) -> &'static str {
        match self {
            Group::MoveAndSelect => "Move and select",
            Group::Open => "Open",
            Group::Act => "Act (row or selection)",
            Group::Invites => "Invites",
            Group::GoAndFind => "Go and find",
            Group::InSearch => "In search",
            Group::DigestsAndFiltering => "Digests and filtering",
            Group::Obsidian => "Obsidian",
        }
    }
}

/// The contexts the key map describes: Focus's window and what opens over
/// it -- the list, the reading dialog, search, the pickers, a digest and
/// Filtered -- and the Mac's search results (spec 010), whose own keys only
/// the Mac offers.
///
/// Not the compose dialog, which teaches its own keys on its buttons and its
/// footer (contracts/focus-surface.md, "Compose"), and not the command bar,
/// which shows each row's key beside it.
pub const KEY_MAP_CONTEXTS: &[Context] = &[
    Context::List,
    Context::Reader,
    Context::Search,
    Context::Picker,
    Context::Digest,
    Context::Filtered,
    Context::Capture,
    Context::Results,
];

/// The group Focus's key map draws `command` in, or `None` for a command it
/// does not show.
///
/// `None` is a decision, and each is commented: the command is the
/// composer's, taught in the compose dialog, or it works on a surface only
/// the three-pane apps have.
pub fn group(command: CommandId) -> Option<Group> {
    use CommandId as C;
    use Group as G;
    match command {
        // ── Move and select ─────────────────────────────────────────────
        C::NextMessage
        | C::PrevMessage
        | C::FirstMessage
        | C::LastMessage
        | C::ToggleSelection
        | C::ExtendSelectionDown
        | C::ExtendSelectionUp
        | C::SelectAll
        | C::Back
        | C::PrevView
        // The reading dialog steps through the thread on these.
        | C::NextInConversation
        | C::PrevInConversation
        | C::ScrollReaderDown
        | C::ScrollReaderUp => Some(G::MoveAndSelect),

        // ── Open ────────────────────────────────────────────────────────
        C::OpenMessage
        | C::ViewSource
        | C::OpenAttachmentOrLink
        | C::ViewOriginal
        | C::ToggleReaderView
        | C::DarkenMessage
        | C::SwitchTreatment
        // Where a message opens, beside the list or over it.
        | C::ToggleReadingPane
        | C::FindInMessage
        | C::FindNext
        | C::FindPrevious
        | C::ZoomIn
        | C::ZoomOut
        | C::ZoomReset
        | C::ShowImages
        | C::AlwaysShowImages => Some(G::Open),

        // ── Act (row or selection) ──────────────────────────────────────
        C::Reply
        | C::ReplyAll
        | C::Forward
        | C::Compose
        | C::Archive
        | C::ArchiveThread
        | C::Delete
        | C::Move
        | C::ToggleRead
        // `*`, with no mark on the row (C13).
        | C::Flag
        | C::Snooze
        | C::Unsnooze
        | C::RemindIfNoReply
        | C::AddLabel
        | C::Unsubscribe
        | C::Undo
        | C::MarkSent
        | C::RetrySend
        | C::CancelSend
        // The marker is on the row it acts on, as the row's other verbs are.
        | C::DismissMarker
        // The verbs a narrow dialog folds away are the row's verbs too.
        | C::MoreActions
        // A picker is how snooze, move and label are answered, so its keys
        // sit with the verbs that open it.
        | C::PickerChoose1
        | C::PickerChoose2
        | C::PickerChoose3
        | C::PickerChoose4
        | C::PickerTypeDate
        | C::PickerToggle
        | C::PickerConfirm => Some(G::Act),

        // ── Invites ─────────────────────────────────────────────────────
        C::AcceptInvite | C::DeclineInvite => Some(G::Invites),

        // ── Go and find ─────────────────────────────────────────────────
        C::Search
        | C::CommandPalette
        | C::CheatSheet
        | C::GoToFolders
        | C::GoToInbox
        | C::GoToDrafts
        | C::GoToSent
        | C::GoToArchive
        | C::GoToSnoozed
        | C::GoToOutbox
        | C::GoToJunk
        | C::GoToTrash
        | C::GoToFlagged
        | C::SavedSearch1
        | C::SavedSearch2
        | C::SavedSearch3
        | C::SavedSearch4
        | C::NextScope
        | C::Refresh
        | C::Settings
        | C::AddAccount
        | C::EditConfig
        | C::Quit => Some(G::GoAndFind),

        // ── In search ───────────────────────────────────────────────────
        C::SaveSearch
        | C::BackToWords
        | C::ToggleResultOrder
        | C::ShowAllResults
        | C::ExcludeSuggestion
        | C::ForgetRecent
        | C::HistoryBack
        | C::HistoryForward
        | C::ResultsConversations
        | C::ResultsFiles
        | C::ResultsPeople
        | C::StepRangeBack
        | C::StepRangeForward
        | C::QuickLook
        | C::NextMatch
        | C::PrevMatch
        | C::PickRelaxation1
        | C::PickRelaxation2
        | C::PickRelaxation3
        | C::PickRelaxation4
        | C::SaveFile => Some(G::InSearch),

        // ── Digests and filtering ───────────────────────────────────────
        C::DigestRule
        | C::StopDigestingSender
        | C::GoToFiltered
        | C::GoToDigestRules
        | C::ToggleHasAction
        | C::RestoreFiltered
        | C::FilteredTab1
        | C::FilteredTab2
        | C::FilteredTab3
        | C::FilteredTab4
        | C::FilteredTab5
        | C::FilteredTab6
        | C::FilteredTab7
        | C::SweepInbox
        | C::NextReference
        | C::PrevReference
        | C::ToggleDigestSummary
        | C::DigestLikeThis => Some(G::DigestsAndFiltering),

        // ── Obsidian ────────────────────────────────────────────────────
        C::CaptureTask
        | C::CaptureNote
        | C::CaptureChangeProject
        | C::CaptureUseSubject
        | C::CaptureWrite => Some(G::Obsidian),

        // ── Not in the key map ──────────────────────────────────────────
        // The composer's own keys, which the compose dialog teaches on its
        // buttons and its footer.
        C::Send
        | C::ScheduleSend
        | C::SaveDraft
        | C::DiscardDraft
        | C::AttachFile
        | C::DetachComposer
        | C::CopyFields
        | C::InsertImage
        | C::Bold
        | C::Italic
        | C::BulletList
        | C::NumberedList
        | C::InsertLink
        | C::QuoteBlock => None,
        // The terminal composer's (`Requirement::Terminal`).
        C::EditExternally | C::TogglePreview => None,
        // A stacked conversation pane's, which Focus's dialog is not.
        C::ToggleFold | C::ExpandAll => None,
        // The account list's own keys.
        C::ToggleAccountEnabled
        | C::RemoveAccount
        | C::UpdateCredential
        | C::RebuildAccountIndex
        | C::SetDefaultAccount
        | C::MapMailboxRole => None,
    }
}

/// One row of the key map: the command, its registry title, and every key
/// the keymap in force binds it to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyMapRow {
    /// The command: built in, or registered at run time.
    pub action: postio_core::ActionId,
    /// What the registry calls it.
    pub title: &'static str,
    /// Its keys, the default first, as `[keys]` spells them.
    pub keys: Vec<String>,
}

/// Focus's key map for `frontend` under `keymap`: each group with a row, in the key map's
/// order, and in each group the commands Focus offers in the key map's
/// contexts, in the registry's order -- built-ins first, then commands
/// registered at run time, which join Act. A group with no rows is left out, so
/// Obsidian appears when its commands do (spec C9).
pub fn key_map(
    keymap: &postio_core::Keymap,
    frontend: postio_core::Frontend,
) -> Vec<(Group, Vec<KeyMapRow>)> {
    let shown = postio_core::ContextSet::from_slice(KEY_MAP_CONTEXTS);
    let mut rows: Vec<(Group, KeyMapRow)> = postio_core::registry::every_action()
        .filter(|spec| spec.requires.offered_by(frontend))
        .filter(|spec| spec.contexts.intersects(shown))
        .filter_map(|spec| {
            // A built-in sits where the table puts it. A command registered
            // at run time joins the verbs: it is a thing done to mail, and
            // it needs no word from Focus to be taught (US7 scenario 2).
            let placed = match spec.id {
                postio_core::ActionId::Builtin(command) => group(command)?,
                postio_core::ActionId::Ext(_) => Group::Act,
            };
            Some((
                placed,
                KeyMapRow {
                    action: spec.id,
                    title: spec.title,
                    keys: keymap.bindings(spec.id).to_vec(),
                },
            ))
        })
        .collect();
    Group::ALL
        .iter()
        .filter_map(|group| {
            let (in_group, rest): (Vec<_>, Vec<_>) =
                rows.drain(..).partition(|(of, _)| of == group);
            rows = rest;
            let in_group: Vec<KeyMapRow> = in_group.into_iter().map(|(_, row)| row).collect();
            (!in_group.is_empty()).then_some((*group, in_group))
        })
        .collect()
}

/// [`key_map`] for an app on `platform`: less what the registry keeps off
/// it (`postio_core::registry::offered_on`), so the Mac's key map does not
/// teach a key its app does not answer. A group left with no rows is left
/// out.
pub fn key_map_on(
    keymap: &postio_core::Keymap,
    frontend: postio_core::Frontend,
    platform: postio_config::paths::Platform,
) -> Vec<(Group, Vec<KeyMapRow>)> {
    key_map(keymap, frontend)
        .into_iter()
        .filter_map(|(group, mut rows)| {
            rows.retain(|row| postio_core::registry::offered_on(row.action, platform));
            (!rows.is_empty()).then_some((group, rows))
        })
        .collect()
}

/// The key map's title.
pub const TITLE: &str = "Keys";

/// The line beside the title.
pub const SUBTITLE: &str = "Single keys act on the focused row, or on the selection if there is one. \
     On macOS, Ctrl becomes \u{2318}.";

/// The footer's line about rebinding.
pub const REBIND_FOOTER: &str = "Rebind anything in ~/.config/postio/config.toml under [keys]";

/// The line beside the title on `platform`. The Mac draws its keys with
/// \u{2318} already, so the sentence saying Ctrl becomes \u{2318} is
/// Linux's alone.
pub fn subtitle(platform: postio_config::paths::Platform) -> &'static str {
    match platform {
        postio_config::paths::Platform::Apple => {
            "Single keys act on the focused row, or on the selection if there is one."
        }
        postio_config::paths::Platform::Freedesktop => SUBTITLE,
    }
}

/// The footer's line about rebinding on `platform`, naming the file that
/// platform reads (`postio_config::paths`; spec 009 C3).
pub fn rebind_footer(platform: postio_config::paths::Platform) -> &'static str {
    match platform {
        postio_config::paths::Platform::Apple => {
            "Rebind anything in ~/Library/Application Support/Postio/config.toml under [keys]"
        }
        postio_config::paths::Platform::Freedesktop => REBIND_FOOTER,
    }
}

/// The footer's line about the mouse.
pub const MOUSE_FOOTER: &str = "The mouse works everywhere: every key has a visible button.";

/// What the keys that close it are followed by, and joined with.
pub const CLOSE_WORD: &str = "close";
/// See [`CLOSE_WORD`].
pub const CLOSE_OR: &str = "or";

/// The commands whose keys close the key map.
pub const CLOSE_COMMANDS: [postio_core::CommandId; 2] = [
    postio_core::CommandId::CheatSheet,
    postio_core::CommandId::Back,
];

/// How many columns the groups are laid out in.
pub const COLUMNS: usize = 4;

/// Which groups go in which column: `sizes` is each group's row count, a
/// group stays whole, and a column is started afresh rather than split one
/// once it would pass an even share (a group is its rows and a heading
/// with its gap).
pub fn pack_columns(sizes: &[usize], columns: usize) -> Vec<Vec<usize>> {
    let weight = |rows: usize| rows + 2;
    let total: usize = sizes.iter().map(|rows| weight(*rows)).sum();
    let per_column = total.div_ceil(columns.max(1));
    let mut packed: Vec<Vec<usize>> = vec![Vec::new()];
    let mut filled = 0;
    for (index, rows) in sizes.iter().enumerate() {
        if filled > 0 && filled + weight(*rows) > per_column && packed.len() < columns {
            packed.push(Vec::new());
            filled = 0;
        }
        packed.last_mut().expect("a column").push(index);
        filled += weight(*rows);
    }
    packed
}

#[cfg(test)]
mod tests {
    use postio_core::{Frontend, Keymap, registry};

    use super::*;

    #[test]
    fn every_row_shows_the_keys_the_keymap_binds() {
        let keymap = Keymap::defaults();
        let map = key_map(keymap, Frontend::Focus);
        assert!(!map.is_empty(), "the key map has groups");
        for (_, rows) in &map {
            for row in rows {
                assert_eq!(
                    row.keys,
                    keymap.bindings(row.action).to_vec(),
                    "{} shows the keymap's keys",
                    row.title
                );
            }
        }
        let archive = map
            .iter()
            .flat_map(|(_, rows)| rows)
            .find(|row| row.action == CommandId::Archive.into())
            .expect("archive is in the key map");
        assert_eq!(archive.title, "Archive");
        // The mnemonic, then the menu chord the second keyboard layer gave
        // it.
        let chord =
            postio_config::keys::expand_mod("mod+shift+a", postio_config::paths::Platform::host());
        assert_eq!(archive.keys, ["a".to_owned(), chord]);
    }

    #[test]
    fn a_rebind_reaches_the_key_map() {
        let config =
            postio_config::Config::from_toml_str("[keys]\narchive = \"w\"\n").expect("a config");
        let keymap = postio_core::Keymap::resolve(&config.keys);
        let map = key_map(&keymap, Frontend::Focus);
        let archive = map
            .iter()
            .flat_map(|(_, rows)| rows)
            .find(|row| row.action == CommandId::Archive.into())
            .expect("archive is in the key map");
        // The file replaces the primary; the registry's alternates are not
        // the file's to replace.
        let chord =
            postio_config::keys::expand_mod("mod+shift+a", postio_config::paths::Platform::host());
        assert_eq!(archive.keys, ["w".to_owned(), chord]);
    }

    #[test]
    fn a_registered_command_joins_the_verbs_with_no_code_of_focus_s() {
        // US7 scenario 2: a command registered at run time is in the key map
        // with its key, through the registry alone.
        let ext = registry::register(registry::ExtCommand {
            id: "test:sort-by-sender".to_owned(),
            title: "Sort by sender".to_owned(),
            default_binding: Some("alt+z".to_owned()),
            alternate_bindings: Vec::new(),
            contexts: postio_core::ContextSet::from_slice(&[postio_core::Context::List]),
            destructive: false,
            recovery: postio_core::Recovery::None,
        })
        .expect("it registers");
        let keymap = postio_core::Keymap::resolve(&postio_config::KeyBindings::default());
        let map = key_map(&keymap, Frontend::Focus);
        let (group, row) = map
            .iter()
            .flat_map(|(group, rows)| rows.iter().map(move |row| (*group, row)))
            .find(|(_, row)| row.action == postio_core::ActionId::Ext(ext))
            .expect("the registered command is in the key map");
        assert_eq!(group, Group::Act);
        assert_eq!(row.title, "Sort by sender");
        assert_eq!(row.keys, ["alt+z"]);
    }

    #[test]
    fn the_groups_come_in_order_and_hold_only_what_focus_offers() {
        let map = key_map(Keymap::defaults(), Frontend::Focus);
        let order: Vec<Group> = map.iter().map(|(group, _)| *group).collect();
        let mut expected: Vec<Group> = Group::ALL.to_vec();
        expected.retain(|group| order.contains(group));
        assert_eq!(order, expected, "the key map's own order");
        // Obsidian appeared with its commands (milestone 3, T158).
        assert!(
            map.iter().any(|(group, rows)| *group == Group::Obsidian
                && rows
                    .iter()
                    .any(|row| row.action == CommandId::CaptureTask.into())),
            "the capture sheet's commands are taught under Obsidian"
        );
        for (group, rows) in &map {
            assert!(!rows.is_empty(), "{group:?} has rows");
            for row in rows {
                let spec = registry::spec(row.action).expect("a registered command");
                assert!(
                    spec.requires.offered_by(Frontend::Focus),
                    "{} is not Focus's",
                    row.title
                );
                if let postio_core::ActionId::Builtin(command) = row.action {
                    assert_eq!(super::group(command), Some(*group));
                }
            }
        }
        let rows: Vec<postio_core::ActionId> = map
            .iter()
            .flat_map(|(_, rows)| rows.iter().map(|row| row.action))
            .collect();
        assert!(
            rows.contains(&CommandId::Flag.into()),
            "Focus flags on `*` (C13)"
        );
        assert!(
            !rows.contains(&CommandId::Send.into()),
            "the composer teaches its own"
        );
        assert!(rows.contains(&CommandId::ToggleHasAction.into()));
        assert!(rows.contains(&CommandId::Undo.into()));
    }

    #[test]
    fn the_mac_names_its_own_file_and_says_nothing_of_ctrl() {
        use postio_config::paths::Platform;
        assert_eq!(rebind_footer(Platform::Freedesktop), REBIND_FOOTER);
        assert_eq!(
            rebind_footer(Platform::Apple),
            "Rebind anything in ~/Library/Application Support/Postio/config.toml under [keys]",
            "C3: the Mac's config.toml"
        );
        assert_eq!(subtitle(Platform::Freedesktop), SUBTITLE);
        assert!(
            SUBTITLE.starts_with(subtitle(Platform::Apple)),
            "the same sentence about single keys"
        );
        assert!(
            !subtitle(Platform::Apple).contains("Ctrl"),
            "the Mac's keys are drawn with \u{2318} already"
        );
    }

    #[test]
    fn a_platform_s_key_map_holds_only_what_it_offers() {
        use postio_config::paths::Platform;
        let shown = |platform| -> Vec<postio_core::ActionId> {
            let keymap = Keymap::resolve_on(&Default::default(), platform);
            key_map_on(&keymap, Frontend::Focus, platform)
                .into_iter()
                .flat_map(|(_, rows)| rows.into_iter().map(|row| row.action))
                .collect()
        };
        let mac = shown(Platform::Apple);
        let linux = shown(Platform::Freedesktop);
        for action in &mac {
            assert!(registry::offered_on(*action, Platform::Apple), "{action:?}");
        }
        assert!(
            linux.contains(&CommandId::DarkenMessage.into()),
            "Linux darkens a sender's design"
        );
        assert!(
            !mac.contains(&CommandId::DarkenMessage.into()),
            "the Mac does not (#1705), so its key map does not teach it"
        );
    }

    #[test]
    fn groups_stay_whole_and_columns_share_the_rows() {
        assert_eq!(
            pack_columns(&[4, 4, 4, 4], 4),
            vec![vec![0], vec![1], vec![2], vec![3]]
        );
        assert_eq!(pack_columns(&[10], 4), vec![vec![0]]);
        let packed = pack_columns(&[2, 2, 2, 2, 2, 2], 4);
        assert!(packed.len() <= 4);
        assert_eq!(packed.concat(), vec![0, 1, 2, 3, 4, 5]);
    }
}
