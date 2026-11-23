//! Focus's key map, grouped: this table says only which group a command sits in; keys come from the
//! keymap in force, so a rebind shows with no change here.
//! `group` is an exhaustive match over `CommandId`, so a new command does not compile until someone
//! places it.

use postio_core::{CommandId, Context};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    MoveAndSelect,
    Open,
    Act,
    Invites,
    GoAndFind,
    InSearch,
    DigestsAndFiltering,
    Obsidian,
}

impl Group {
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

/// Focus's window and what opens over it, plus the Mac's search results. Not the compose dialog or
/// command bar, which teach their own keys.
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

/// `None` for a command the key map does not show; each case is commented.
pub fn group(command: CommandId) -> Option<Group> {
    use CommandId as C;
    use Group as G;
    match command {
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
        | C::NextInConversation
        | C::PrevInConversation
        | C::FirstInConversation
        | C::LatestInConversation
        | C::FirstUnreadInConversation
        | C::ExpandThread
        | C::CollapseThread
        | C::ScrollReaderDown
        | C::ScrollReaderUp => Some(G::MoveAndSelect),

        C::OpenMessage
        | C::ContactPopover
        | C::ThreadOutline
        | C::ThreadMenu
        | C::ViewSource
        | C::OpenAttachmentOrLink
        | C::ViewOriginal
        | C::ToggleReaderView
        | C::DarkenMessage
        | C::SwitchTreatment
        | C::ToggleReadingPane
        | C::FindInMessage
        | C::FindNext
        | C::FindPrevious
        | C::ZoomIn
        | C::ZoomOut
        | C::ZoomReset
        | C::ShowImages
        | C::AlwaysShowImages => Some(G::Open),

        C::Reply
        | C::ReplyAll
        | C::Forward
        | C::Compose
        | C::Archive
        | C::MuteThread
        | C::SplitThread
        | C::ReplyToLatest
        | C::MoveThisMessage
        | C::DeleteThisMessage
        | C::Delete
        | C::Move
        | C::ToggleRead
        | C::Flag
        | C::Snooze
        | C::Unsnooze
        | C::RemindIfNoReply
        | C::AddLabel
        | C::Unsubscribe
        | C::ClearListMail
        | C::Undo
        | C::MarkSent
        | C::RetrySend
        | C::CancelSend
        | C::DismissMarker
        | C::MoreActions
        | C::PickerChoose1
        | C::PickerChoose2
        | C::PickerChoose3
        | C::PickerChoose4
        | C::PickerTypeDate
        | C::PickerToggle
        | C::PickerConfirm => Some(G::Act),

        C::AcceptInvite | C::DeclineInvite => Some(G::Invites),

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
        | C::OpenContacts
        | C::ContactsSuggestions
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
        | C::FilterFrom
        | C::FilterTo
        | C::FilterDate
        | C::FilterAnywhere
        | C::FilterLabel
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

        C::CaptureTask
        | C::CaptureNote
        | C::CaptureChangeProject
        | C::CaptureUseSubject
        | C::CaptureWrite => Some(G::Obsidian),

        // Contacts has a surface of its own still to be drawn; `OpenContacts` is in Go and find
        // above.
        C::ContactOpenCard
        | C::ContactAdd
        | C::ContactIgnore
        | C::ContactArrivalPrev
        | C::ContactArrivalNext
        | C::ContactSetArrival
        | C::ContactShowMail
        | C::ContactCompose
        | C::ContactsFilter
        | C::ContactsTabPrevious
        | C::ContactsTabNext
        | C::ContactsToggleEveryone
        | C::ContactJoin
        | C::ContactAddAddress
        | C::ContactDetachAddress
        | C::ContactSetPreferred
        | C::ContactNew
        | C::ContactEdit
        | C::ContactSave
        | C::ContactDelete
        | C::ContactRestore
        | C::ContactsToggleDeleted
        | C::SuggestionDismiss
        | C::ContactGroupNew
        | C::ContactGroupRename
        | C::ContactGroupAdd
        | C::ContactGroupRemove
        | C::ContactsImport
        | C::ContactsExport
        | C::ContactsSync => None,
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
        | C::Heading1
        | C::Heading2
        | C::Heading3
        | C::Heading4
        | C::Heading5
        | C::Heading6
        | C::NormalText
        | C::Strikethrough
        | C::InlineCode
        | C::CodeBlock
        | C::TaskList
        | C::ToggleTask
        | C::Indent
        | C::Outdent
        | C::Divider
        | C::Table
        | C::ClearFormatting
        | C::ToggleMarkdownSource
        | C::AttachRemoteImage
        | C::BulletList
        | C::NumberedList
        | C::InsertLink
        | C::QuoteBlock
        | C::ShowNewer
        | C::ExpandQuoted
        | C::OutlineFirstUnread
        | C::ForwardWhat1
        | C::ForwardWhat2
        | C::ForwardWhat3 => None,
        C::EditExternally | C::TogglePreview => None,
        C::ToggleFold | C::ExpandAll => None,
        C::ToggleAccountEnabled
        | C::RemoveAccount
        | C::UpdateCredential
        | C::RebuildAccountIndex
        | C::SetDefaultAccount
        | C::MapMailboxRole => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyMapRow {
    pub action: postio_core::ActionId,
    pub title: &'static str,
    pub keys: Vec<String>,
}

/// A group with no rows is left out.
pub fn key_map(
    keymap: &postio_core::Keymap,
    frontend: postio_core::Frontend,
) -> Vec<(Group, Vec<KeyMapRow>)> {
    let shown = postio_core::ContextSet::from_slice(KEY_MAP_CONTEXTS);
    let mut rows: Vec<(Group, KeyMapRow)> = postio_core::registry::every_action()
        .filter(|spec| spec.requires.offered_by(frontend))
        .filter(|spec| spec.contexts.intersects(shown))
        .filter_map(|spec| {
            let postio_core::ActionId::Builtin(command) = spec.id;
            let placed = group(command)?;
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

/// Less what the registry keeps off `platform`, so the Mac's key map does not teach a key its app
/// does not answer.
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

pub const TITLE: &str = "Keys";

pub const SUBTITLE: &str = "Single keys act on the focused row, or on the selection if there is one. \
     On macOS, Ctrl becomes \u{2318}.";

pub const REBIND_FOOTER: &str = "Rebind anything in ~/.config/postio/config.toml under [keys]";

pub fn subtitle(platform: postio_config::paths::Platform) -> &'static str {
    match platform {
        postio_config::paths::Platform::Apple => {
            "Single keys act on the focused row, or on the selection if there is one."
        }
        postio_config::paths::Platform::Freedesktop => SUBTITLE,
    }
}

pub fn rebind_footer(platform: postio_config::paths::Platform) -> &'static str {
    match platform {
        postio_config::paths::Platform::Apple => {
            "Rebind anything in ~/Library/Application Support/Postio/config.toml under [keys]"
        }
        postio_config::paths::Platform::Freedesktop => REBIND_FOOTER,
    }
}

pub const MOUSE_FOOTER: &str = "The mouse works everywhere: every key has a visible button.";

pub const MORE_BELOW: &str = "More below";

pub const CLOSE_WORD: &str = "close";
pub const CLOSE_OR: &str = "or";

pub const CLOSE_COMMANDS: [postio_core::CommandId; 2] = [
    postio_core::CommandId::CheatSheet,
    postio_core::CommandId::Back,
];

pub const COLUMNS: usize = 4;

/// A group stays whole, and a column starts afresh rather than split one once it would pass an even
/// share.
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
        let chord =
            postio_config::keys::expand_mod("mod+shift+a", postio_config::paths::Platform::host());
        assert_eq!(archive.keys, ["e".to_owned(), chord]);
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
        let chord =
            postio_config::keys::expand_mod("mod+shift+a", postio_config::paths::Platform::host());
        assert_eq!(archive.keys, ["w".to_owned(), chord]);
    }

    #[test]
    fn the_groups_come_in_order_and_hold_only_what_focus_offers() {
        let map = key_map(Keymap::defaults(), Frontend::Focus);
        let order: Vec<Group> = map.iter().map(|(group, _)| *group).collect();
        let mut expected: Vec<Group> = Group::ALL.to_vec();
        expected.retain(|group| order.contains(group));
        assert_eq!(order, expected, "the key map's own order");
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
                let postio_core::ActionId::Builtin(command) = row.action;
                assert_eq!(super::group(command), Some(*group));
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
