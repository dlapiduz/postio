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
/// Filtered.
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
        C::SaveSearch | C::BackToWords | C::ToggleResultOrder => Some(G::InSearch),

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
        | C::NextReference
        | C::PrevReference
        | C::ToggleDigestSummary => Some(G::DigestsAndFiltering),

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
        // The three-pane apps' surfaces (`Requirement::ThreePane`): flags,
        // the sidebar, the panes, and the parts panel. Focus has none.
        C::Flag | C::ToggleSidebar | C::CyclePane | C::CyclePaneBack | C::OpenParts => None,
        // A stacked conversation pane's, which Focus's dialog is not.
        C::ToggleFold | C::ExpandAll | C::ToggleRail => None,
        // The folder list's own keys.
        C::NextFolder
        | C::PrevFolder
        | C::ToggleFolder
        | C::RenameSavedSearch
        | C::MoveSavedSearchUp
        | C::MoveSavedSearchDown
        | C::DeleteSavedSearch => None,
        // The account list's own keys.
        C::ToggleAccountEnabled
        | C::RemoveAccount
        | C::UpdateCredential
        | C::RebuildAccountIndex
        | C::SetDefaultAccount
        | C::MapMailboxRole => None,
        // The parts panel's own keys.
        C::NextPart
        | C::PrevPart
        | C::OpenPart
        | C::SavePart
        | C::SaveAllParts
        | C::OpenPartExternally
        | C::RenderPartOnce => None,
    }
}
