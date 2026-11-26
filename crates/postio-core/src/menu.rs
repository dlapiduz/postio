//! Where each command belongs in a platform menu bar, in the crate that owns the vocabulary so
//! frontends do not disagree (ADR 0019).
//! `section_for` is exhaustive over `CommandId`: adding a command does not compile until someone
//! says where it goes.
//! `None` is a decision, not an oversight: cursor motion, chord completion and text editing stay in
//! the palette and cheat sheet.

use crate::command::CommandId;
use postio_config::paths::Platform;

/// Apple's conventions: `Postio` and `Window` are AppKit's own and carry nothing from the registry,
/// so they are not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MenuSection {
    /// macOS has one; freedesktop has none, and `MenuSection::bar_for` folds it into `Edit` there.
    App,
    File,
    Edit,
    View,
    Go,
    Message,
    Format,
    Help,
}

impl MenuSection {
    pub const ALL: &'static [MenuSection] = &[
        MenuSection::App,
        MenuSection::File,
        MenuSection::Edit,
        MenuSection::View,
        MenuSection::Go,
        MenuSection::Message,
        MenuSection::Format,
        MenuSection::Help,
    ];

    /// The application menu is absent where there is none; its commands reach Edit through
    /// `section_on`.
    pub fn bar_for(platform: Platform) -> Vec<MenuSection> {
        MenuSection::ALL
            .iter()
            .copied()
            .filter(|section| *section != MenuSection::App || platform == Platform::Apple)
            .collect()
    }

    pub fn title(self) -> &'static str {
        match self {
            MenuSection::App => "Postio",
            MenuSection::File => "File",
            MenuSection::Edit => "Edit",
            MenuSection::View => "View",
            MenuSection::Go => "Go",
            MenuSection::Message => "Message",
            MenuSection::Format => "Format",
            MenuSection::Help => "Help",
        }
    }
}

/// `None` when the command is deliberately not a menu item. Exhaustive: a new command is a compile
/// error here until placed.
pub fn section_for(command: CommandId) -> Option<MenuSection> {
    use CommandId as C;
    use MenuSection as M;
    match command {
        // Cursor motion is in the palette and the cheat sheet; menu rows for it crowd out the ones
        // people use.
        C::NextMessage | C::PrevMessage | C::ExtendSelectionDown | C::ExtendSelectionUp => None,
        C::FirstMessage | C::LastMessage => Some(M::Go),
        C::GoToFolders => Some(M::Go),
        C::GoToInbox | C::GoToDrafts | C::GoToSent | C::GoToFlagged => Some(M::Go),
        C::GoToArchive | C::GoToSnoozed => Some(M::Go),
        C::GoToOutbox | C::GoToJunk | C::GoToTrash => Some(M::Go),
        C::SavedSearch1 | C::SavedSearch2 | C::SavedSearch3 | C::SavedSearch4 => Some(M::Go),
        C::NextScope => Some(M::Go),
        // Its own commands act on a focused row, and a global menu cannot say which person.
        C::OpenContacts | C::GoToCalendar => Some(M::Go),
        C::CalendarAgenda
        | C::CalendarPrevious
        | C::CalendarNext
        | C::CalendarToday
        | C::NewEvent
        | C::EditEvent
        | C::MoveEvent
        | C::DuplicateEvent
        | C::TentativeInvite
        | C::SaveEvent
        | C::SaveAndSendEvent
        | C::EmailAttendees => None,
        C::PopOut | C::PopIn | C::CloseWindow => Some(M::App),
        C::NextInConversation | C::PrevInConversation => Some(M::Go),
        C::FirstInConversation | C::LatestInConversation | C::FirstUnreadInConversation => {
            Some(M::Go)
        }
        C::Back | C::PrevView => Some(M::Go),

        C::Compose | C::Send | C::ScheduleSend | C::SaveDraft | C::DiscardDraft => Some(M::File),
        // All three are about a message that left the composer and has not arrived.
        C::AttachFile | C::DetachComposer | C::MarkSent | C::RetrySend | C::CancelSend => {
            Some(M::File)
        }
        C::Refresh => Some(M::File),
        C::ContactsImport | C::ContactsExport | C::ContactsSync => Some(M::File),

        C::Undo | C::SelectAll | C::ToggleSelection => Some(M::Edit),
        C::Search | C::SaveSearch => Some(M::Edit),
        C::Settings | C::EditConfig | C::AddAccount | C::Quit => Some(M::App),
        C::ShowImages | C::AlwaysShowImages | C::Unsubscribe | C::ClearListMail => Some(M::Message),
        // Settings commands act on the focused settings row; meaningless in a global menu.
        C::ToggleAccountEnabled
        | C::RemoveAccount
        | C::UpdateCredential
        | C::RebuildAccountIndex
        | C::SetDefaultAccount
        | C::MapMailboxRole => None,

        C::ToggleFold | C::ExpandAll => Some(M::View),
        C::ExpandThread | C::CollapseThread | C::ThreadOutline => Some(M::View),
        C::ShowNewer | C::ExpandQuoted => Some(M::View),
        // This raises and lowers rows; it does not do anything to the draft.
        C::CopyFields => Some(M::View),
        C::ToggleResultOrder => Some(M::View),
        C::FilterFrom | C::FilterTo | C::FilterDate | C::FilterAnywhere | C::FilterLabel => {
            Some(M::View)
        }
        C::ViewOriginal | C::ToggleReaderView => Some(M::View),
        C::DarkenMessage | C::SwitchTreatment | C::ZoomIn | C::ZoomOut | C::ZoomReset => {
            Some(M::View)
        }
        C::FindInMessage | C::FindNext | C::FindPrevious => Some(M::Edit),
        C::CommandPalette => Some(M::View),

        // A sequence like `g g` has no key equivalent, so the cheat sheet is where the keyboard is
        // described in full.
        C::CheatSheet => Some(M::Help),
        // A menu item that scrolls by one screen is a control, not a command.
        C::ScrollReaderDown | C::ScrollReaderUp => None,

        C::Reply | C::ReplyAll | C::Forward => Some(M::Message),
        C::Archive | C::Delete | C::Move => Some(M::Message),
        C::Flag | C::ToggleRead | C::AddLabel => Some(M::Message),
        C::Snooze | C::Unsnooze => Some(M::Message),
        C::OpenMessage => Some(M::Message),
        C::ReplyToLatest | C::MuteThread | C::SplitThread => Some(M::Message),
        C::MoveThisMessage | C::DeleteThisMessage => Some(M::Message),

        C::Bold | C::Italic | C::BulletList | C::NumberedList => Some(M::Format),
        C::InsertImage => Some(M::Format),
        C::InsertLink | C::QuoteBlock => Some(M::Format),
        C::Heading1
        | C::Heading2
        | C::Heading3
        | C::Heading4
        | C::Heading5
        | C::Heading6
        | C::NormalText => Some(M::Format),
        C::Strikethrough
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
        | C::AttachRemoteImage => Some(M::Format),
        // The terminal composer's own; the terminal has no menu bar.
        C::EditExternally | C::TogglePreview => None,

        // Focus has no menu bar, and no other app offers these: a menu item would never run.
        C::RemindIfNoReply
        | C::AcceptInvite
        | C::DeclineInvite
        | C::DigestRule
        | C::StopDigestingSender
        | C::ViewSource
        | C::OpenAttachmentOrLink
        | C::DismissMarker
        | C::MoreActions
        | C::ToggleReadingPane
        | C::BackToWords
        | C::ShowAllResults
        | C::ForgetRecent
        | C::ExcludeSuggestion
        | C::HistoryBack
        | C::HistoryForward
        | C::ResultsConversations
        | C::ResultsFiles
        | C::ResultsPeople
        | C::ResultsEvents
        | C::StepRangeBack
        | C::StepRangeForward
        | C::QuickLook
        | C::NextMatch
        | C::PrevMatch
        | C::PickRelaxation1
        | C::PickRelaxation2
        | C::PickRelaxation3
        | C::PickRelaxation4
        | C::SaveFile
        | C::ContactPopover
        | C::ContactOpenCard
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
        | C::ContactsSuggestions
        | C::SuggestionDismiss
        | C::ContactGroupNew
        | C::ContactGroupRename
        | C::ContactGroupAdd
        | C::ContactGroupRemove
        | C::GoToFiltered
        | C::GoToDigestRules
        | C::ToggleHasAction
        | C::PickerChoose1
        | C::PickerChoose2
        | C::PickerChoose3
        | C::PickerChoose4
        | C::PickerTypeDate
        | C::PickerToggle
        | C::PickerConfirm
        | C::OutlineFirstUnread
        | C::ThreadMenu
        | C::ForwardWhat1
        | C::ForwardWhat2
        | C::ForwardWhat3
        | C::NextReference
        | C::PrevReference
        | C::ToggleDigestSummary
        | C::RestoreFiltered
        | C::FilteredTab1
        | C::FilteredTab2
        | C::FilteredTab3
        | C::FilteredTab4
        | C::FilteredTab5
        | C::FilteredTab6
        | C::FilteredTab7
        | C::SweepInbox
        | C::CaptureTask
        | C::CaptureNote
        | C::CaptureChangeProject
        | C::CaptureUseSubject
        | C::CaptureWrite
        | C::DigestLikeThis => None,
    }
}

/// Freedesktop has no application menu, so its three commands appear under Edit.
pub fn section_on(command: CommandId, platform: Platform) -> Option<MenuSection> {
    if !crate::registry::offered_on(crate::ActionId::Builtin(command), platform) {
        return None;
    }
    match section_for(command) {
        Some(MenuSection::App) if platform != Platform::Apple => Some(MenuSection::Edit),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_a_platform_does_not_offer_is_no_menu_item_there() {
        for platform in [Platform::Freedesktop, Platform::Apple] {
            for command in [CommandId::DetachComposer, CommandId::NextScope] {
                let offered =
                    crate::registry::offered_on(crate::ActionId::Builtin(command), platform);
                assert_eq!(
                    section_on(command, platform).is_some(),
                    offered && section_for(command).is_some(),
                    "`{command}` on {platform:?}"
                );
            }
        }
        assert_eq!(section_on(CommandId::DetachComposer, Platform::Apple), None);
    }

    #[test]
    fn every_section_holds_something() {
        for section in MenuSection::ALL {
            assert!(
                crate::registry::all().any(|spec| section_for(spec.id) == Some(*section)),
                "{section:?} would draw an empty menu"
            );
        }
    }

    #[test]
    fn most_of_the_registry_reaches_a_menu() {
        // Guards a table that fell out of date and started answering `None` broadly. Contacts' own
        // commands are excluded: they act on a focused row.
        let counted = || {
            crate::registry::all().filter(|spec| spec.contexts != crate::Context::Contacts.as_set())
        };
        let total = counted().count();
        let placed = counted()
            .filter(|spec| section_for(spec.id).is_some())
            .count();
        assert!(
            placed * 2 > total,
            "only {placed} of {total} commands reach a menu"
        );
    }

    #[test]
    fn settings_lives_in_the_application_menu_on_apple_and_in_edit_elsewhere() {
        // `⌘,` is in the application menu on macOS only; freedesktop folds the same three verbs
        // into Edit (#1158).
        use postio_config::paths::Platform;
        for command in [
            CommandId::Settings,
            CommandId::EditConfig,
            CommandId::AddAccount,
        ] {
            assert_eq!(section_for(command), Some(MenuSection::App), "{command:?}");
            assert_eq!(
                section_on(command, Platform::Apple),
                Some(MenuSection::App),
                "{command:?}"
            );
            assert_eq!(
                section_on(command, Platform::Freedesktop),
                Some(MenuSection::Edit),
                "{command:?}"
            );
        }
    }

    #[test]
    fn the_bar_shows_an_application_menu_only_where_the_platform_has_one() {
        use postio_config::paths::Platform;
        let apple = MenuSection::bar_for(Platform::Apple);
        let freedesktop = MenuSection::bar_for(Platform::Freedesktop);

        assert_eq!(
            apple.first(),
            Some(&MenuSection::App),
            "the application menu is the leftmost one on macOS"
        );
        assert!(
            !freedesktop.contains(&MenuSection::App),
            "freedesktop drew an application menu it has no place for"
        );
        let without_app: Vec<_> = apple
            .iter()
            .filter(|section| **section != MenuSection::App)
            .copied()
            .collect();
        assert_eq!(without_app, freedesktop);
    }

    #[test]
    fn no_section_draws_an_empty_menu_on_either_platform() {
        use postio_config::paths::Platform;
        for platform in [Platform::Apple, Platform::Freedesktop] {
            for section in MenuSection::bar_for(platform) {
                assert!(
                    crate::registry::all()
                        .any(|spec| section_on(spec.id, platform) == Some(section)),
                    "{section:?} would draw an empty menu on {platform:?}"
                );
            }
        }
    }
}
