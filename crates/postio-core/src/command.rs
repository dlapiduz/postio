//! Commands: the one way anything happens in Postio. A frontend sends a `Command` down and reacts
//! to `Event`s coming up.
//! `CommandId` is a stable string because `[keys]` in `config.toml` names commands by id: renaming
//! one silently breaks a user's configuration.
//! A `CommandId` names what can be done; a `Command` is one invocation and may carry a target or
//! payload.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use postio_model::calendar::{EditScope, EventFields, EventTarget};
use postio_model::ids::{CalendarId, EventChangeId, EventId};
use postio_model::{
    AccountId, AddressId, ContactGroup, ContactGroupId, ContactId, ContactState, DraftId,
    EmailAddress, JoinReceipt, LabelId, MailboxId, MailboxRole, MessageId, OperationRange,
    PersonEdit, PersonFields, ThreadId,
};

/// About ten seconds, the toast's own life; here so every frontend's toast says the window the
/// engine keeps.
pub const RSVP_WINDOW: std::time::Duration = std::time::Duration::from_secs(10);

pub fn answer_invite(id: CommandId, message: Option<MessageId>) -> Option<Command> {
    match id {
        CommandId::AcceptInvite => Some(Command::AcceptInvite { message }),
        CommandId::DeclineInvite => Some(Command::DeclineInvite { message }),
        CommandId::TentativeInvite => Some(Command::TentativeInvite { message }),
        _ => None,
    }
}
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Declares every command once. Entries after the `;` are invocations of a verb another id already
/// names:
/// they carry that id and have no default.
macro_rules! commands {
    (
        $(
            $(#[$meta:meta])*
            $variant:ident
            $({ $( $(#[$fmeta:meta])* $field:ident : $ty:ty = $default:expr ),* $(,)? })?
            => $id:literal
        ),+ $(,)?
        ;
        $(
            $(#[$ameta:meta])*
            $alias:ident
            { $( $(#[$afmeta:meta])* $afield:ident : $aty:ty ),* $(,)? }
            => $target:ident
        ),* $(,)?
    ) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum CommandId {
            $( $(#[$meta])* $variant, )+
        }

        impl CommandId {
            /// In the order the cheat sheet and palette list them.
            pub const ALL: &'static [CommandId] = &[ $( CommandId::$variant, )+ ];

            pub const fn as_str(self) -> &'static str {
                match self {
                    $( CommandId::$variant => $id, )+
                }
            }
        }

        /// A payload field a keystroke cannot supply is `Option`: `None` means "ask the user".
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        pub enum Command {
            $(
                $(#[$meta])*
                $variant $({ $( $(#[$fmeta])* $field: $ty ),* })?,
            )+
            $(
                $(#[$ameta])*
                $alias { $( $(#[$afmeta])* $afield: $aty ),* },
            )*
        }

        impl Command {
            pub fn id(&self) -> CommandId {
                match self {
                    $( Command::$variant { .. } => CommandId::$variant, )+
                    $( Command::$alias { .. } => CommandId::$target, )*
                }
            }

            /// Targets the current selection and leaves every choice for app state to resolve.
            pub fn default_for(id: CommandId) -> Command {
                match id {
                    $( CommandId::$variant => Command::$variant $({ $( $field: $default ),* })?, )+
                }
            }
        }
    };
}

commands! {
    NextMessage => "next_message",
    PrevMessage => "prev_message",
    FirstMessage => "first_message",
    LastMessage => "last_message",
    OpenMessage {
        message: Option<MessageId> = None,
    } => "open_message",
    ToggleSelection {
        message: Option<MessageId> = None,
    } => "toggle_selection",
    ExtendSelectionDown => "extend_selection_down",
    ExtendSelectionUp => "extend_selection_up",
    /// Never resolved into a list of ids on the way through; see `crate::state::Selection`.
    SelectAll => "select_all",
    PrevView => "prev_view",
    Back => "back",
    ToggleResultOrder => "toggle_result_order",
    /// No payload: `j`/`k` move between threads, these between messages inside the open one.
    NextInConversation => "next_in_thread",
    PrevInConversation => "prev_in_thread",
    /// The only way to collapse the focused message: landing on one expands it.
    ToggleFold => "toggle_fold",
    ViewOriginal => "view_original",
    /// Every message opens as sent, so this is the only way into reader view.
    ToggleReaderView => "toggle_reader_view",
    DarkenMessage => "darken_message",
    /// The message's alone: "Always for this sender" is the reader's to store.
    SwitchTreatment => "switch_treatment",
    FindInMessage => "find_in_message",
    FindNext => "find_next",
    FindPrevious => "find_previous",
    ZoomIn => "zoom_in",
    ZoomOut => "zoom_out",
    ZoomReset => "zoom_reset",
    ExpandAll => "expand_all",

    Reply {
        message: Option<MessageId> = None,
    } => "reply",
    ReplyAll {
        message: Option<MessageId> = None,
    } => "reply_all",
    Forward {
        message: Option<MessageId> = None,
    } => "forward",
    Archive {
        target: MessageTarget = MessageTarget::Selection,
    } => "archive",
    Delete {
        target: MessageTarget = MessageTarget::Selection,
    } => "delete",
    Move {
        target: MessageTarget = MessageTarget::Selection,
        to: Option<MailboxId> = None,
    } => "move",
    Flag {
        target: MessageTarget = MessageTarget::Selection,
        flagged: Option<bool> = None,
    } => "flag",
    ToggleRead {
        target: MessageTarget = MessageTarget::Selection,
        unread: Option<bool> = None,
    } => "toggle_read",
    /// The time is the picker's; `None` is a keystroke with no picker, answered by the handler's
    /// default.
    Snooze {
        target: MessageTarget = MessageTarget::Selection,
        until: Option<DateTime<Utc>> = None,
    } => "snooze",
    Unsnooze {
        target: MessageTarget = MessageTarget::Selection,
    } => "unsnooze",
    /// `None` clears the reminder: a value rather than a toggle, so undo can dispatch the inverse.
    RemindIfNoReply {
        target: MessageTarget = MessageTarget::Selection,
        at: Option<DateTime<Utc>> = None,
    } => "remind_if_no_reply",
    AddLabel {
        target: MessageTarget = MessageTarget::Selection,
        label: Option<LabelId> = None,
        /// On, off, or `None` to toggle: undo dispatches the inverse, so removing a label must be
        /// something a `Command` can say.
        on: Option<bool> = None,
    } => "add_label",
    /// Undone within a window rather than from the undo stack, as a send is (research R9).
    AcceptInvite {
        message: Option<MessageId> = None,
    } => "accept_invite",
    DeclineInvite {
        message: Option<MessageId> = None,
    } => "decline_invite",
    DigestRule {
        target: MessageTarget = MessageTarget::Selection,
    } => "digest_rule",
    /// A direction rather than a toggle, so undo can dispatch the inverse.
    StopDigestingSender {
        target: MessageTarget = MessageTarget::Selection,
        stopped: bool = true,
        /// Undo's memory, `None` from a keystroke: the rule as it stood, so taking it back restores
        /// it exactly.
        kept: Option<KeptRule> = None,
    } => "stop_digesting_sender",
    ViewSource {
        message: Option<MessageId> = None,
    } => "view_source",
    OpenAttachmentOrLink {
        message: Option<MessageId> = None,
    } => "open_attachment_or_link",
    /// A dismissal never returns on that message; dismissing one kind from one sender repeatedly
    /// stops that kind for the sender.
    DismissMarker {
        target: MessageTarget = MessageTarget::Selection,
        dismissed: bool = true,
    } => "dismiss_marker",
    MoreActions => "more_actions",
    ToggleReadingPane => "toggle_reading_pane",

    Search {
        query: Option<String> = None,
    } => "search",
    /// No payload: the open box says which query.
    SaveSearch => "save_search",
    BackToWords => "back_to_words",
    ShowAllResults => "show_all_results",
    ForgetRecent => "forget_recent",
    ExcludeSuggestion => "exclude_suggestion",
    HistoryBack => "history_back",
    HistoryForward => "history_forward",
    ResultsConversations => "results_threads",
    ResultsFiles => "results_files",
    ResultsPeople => "results_people",
    FilterFrom => "filter_from",
    FilterTo => "filter_to",
    FilterDate => "filter_date",
    FilterAnywhere => "filter_anywhere",
    FilterLabel => "filter_label",
    StepRangeBack => "step_range_back",
    StepRangeForward => "step_range_forward",
    QuickLook => "quick_look",
    NextMatch => "next_match",
    PrevMatch => "prev_match",
    PickRelaxation1 => "pick_relaxation_1",
    PickRelaxation2 => "pick_relaxation_2",
    PickRelaxation3 => "pick_relaxation_3",
    PickRelaxation4 => "pick_relaxation_4",
    SaveFile => "save_file",

    Compose {
        draft: Option<DraftId> = None,
    } => "compose",
    Send => "send",
    /// No payload: choosing a time is a widget interaction inside the composer.
    ScheduleSend => "schedule_send",
    SaveDraft => "save_draft",
    DiscardDraft => "discard_draft",
    /// `None` means the draft in view. Without it an `Unconfirmed` draft's only exits are discard
    /// or a duplicate send (ADR 0021, #674).
    MarkSent {
        draft: Option<DraftId> = None,
    } => "mark_sent",
    /// The Outbox can offer this where a message on its way has no composer open. `None` means the
    /// draft in view.
    RetrySend {
        draft: Option<DraftId> = None,
    } => "retry_send",
    /// Refused once the submission has started: ADR 0021 keeps exactly-once by never cancelling
    /// what may be in flight.
    CancelSend {
        draft: Option<DraftId> = None,
    } => "cancel_send",
    AttachFile {
        path: Option<PathBuf> = None,
    } => "attach_file",
    /// Detaching moves the draft rather than forking it. Purely a view concern.
    DetachComposer => "detach_composer",
    /// Raising always works; putting away only works while both fields are empty, since a hidden
    /// row holding addresses would still send to them.
    /// Otherwise the keyboard moves to `Cc`.
    CopyFields => "copy_fields",
    /// The one of the three outcomes (link, inline image, attachment) that had no command; paste
    /// and drop reached it but the palette could not.
    InsertImage => "insert_image",
    /// Where the body is not text an editor can open, the frontend says so.
    EditExternally => "edit_externally",
    /// The desktop's composer shows formatting as written, so it has nothing to preview.
    TogglePreview => "toggle_preview",
    Bold => "bold",
    Italic => "italic",
    /// One command per level, so each of the six is one chord away.
    Heading1 => "heading_1",
    Heading2 => "heading_2",
    Heading3 => "heading_3",
    Heading4 => "heading_4",
    Heading5 => "heading_5",
    Heading6 => "heading_6",
    NormalText => "normal_text",
    BulletList => "bullet_list",
    NumberedList => "numbered_list",
    InsertLink => "insert_link",
    QuoteBlock => "quote_block",
    Strikethrough => "strikethrough",
    InlineCode => "inline_code",
    CodeBlock => "code_block",
    TaskList => "task_list",
    ToggleTask => "toggle_task",
    Indent => "indent",
    Outdent => "outdent",
    Divider => "divider",
    /// In the Markdown view and the terminal, a two-by-two template to type over.
    Table => "table",
    ClearFormatting => "clear_formatting",
    ToggleMarkdownSource => "toggle_markdown_source",
    /// The one composer gesture that reaches the network, only on this press.
    AttachRemoteImage => "attach_remote_image",

    Undo => "undo",
    CommandPalette => "command_palette",
    CheatSheet => "cheat_sheet",
    Settings => "settings",
    /// A registered command rather than a button, so the palette and cheat sheet carry it (ADR 0012
    /// Q1).
    AddAccount => "add_account",
    EditConfig => "edit_config",
    /// A command not in the registry does not exist (Principle II), so a terminal needs one.
    Quit => "quit",
    /// A command not in the registry does not exist (Principle II); a terminal has no banner to
    /// click.
    ShowImages => "show_images",
    AlwaysShowImages => "always_show_images",
    /// Only ever on a deliberate act.
    Unsubscribe => "unsubscribe",
    /// The notice's own button, as a command (#1953). The frontend holds the pending offer, so
    /// there is no payload;
    /// with no offer on screen it says so and does nothing.
    ClearListMail => "clear_list_mail",
    GoToFolders => "go_to_folders",
    /// One variant each rather than `GoTo(MailboxRole)`: `Command::id` is total, and `GoTo(Junk)`
    /// would have no command id.
    GoToInbox => "go_to_inbox",
    GoToDrafts => "go_to_drafts",
    GoToSent => "go_to_sent",
    GoToFlagged => "go_to_flagged",
    GoToArchive => "go_to_archive",
    GoToSnoozed => "go_to_snoozed",
    GoToOutbox => "go_to_outbox",
    GoToJunk => "go_to_junk",
    GoToTrash => "go_to_trash",
    GoToFiltered => "go_to_filtered",
    GoToDigestRules => "go_to_digest_rules",
    /// One variant for each of the four, as the destinations above: `SavedSearch(9)` would have no
    /// command id.
    SavedSearch1 => "saved_search_1",
    SavedSearch2 => "saved_search_2",
    SavedSearch3 => "saved_search_3",
    SavedSearch4 => "saved_search_4",
    ToggleHasAction => "toggle_has_action",
    /// No payload: only offered while `Context::Accounts` is active, so the target is the focused
    /// row (ADR 0005 Q6c).
    ToggleAccountEnabled => "toggle_account_enabled",
    RemoveAccount => "remove_account",
    UpdateCredential => "update_credential",
    /// Local mail only: nothing here reaches the network (#981).
    RebuildAccountIndex => "rebuild_account_index",
    /// Settles which account a new message comes from; not a reply's from address, which comes from
    /// the message replied to (#960).
    SetDefaultAccount => "set_default_account",
    /// `path: None` is a value, "back to automatic", not "ask"; a second command for it would be a
    /// key in the reference for nothing.
    MapMailboxRole {
        account: Option<AccountId> = None,
        role: Option<MailboxRole> = None,
        path: Option<String> = None,
    } => "map_mailbox_role",
    CreateFolder {
        account: Option<AccountId> = None,
        parent: Option<MailboxId> = None,
        name: Option<String> = None,
    } => "create_folder",
    /// Cycling because a keystroke has no argument to carry a scope; the sidebar's rows name one
    /// directly.
    NextScope => "next_scope",
    Refresh => "refresh",

    ScrollReaderDown => "scroll_reader_down",
    ScrollReaderUp => "scroll_reader_up",

    /// One variant per option, like the saved searches.
    PickerChoose1 => "picker_choose_1",
    PickerChoose2 => "picker_choose_2",
    PickerChoose3 => "picker_choose_3",
    PickerChoose4 => "picker_choose_4",
    PickerTypeDate => "picker_type_date",
    PickerToggle => "picker_toggle",
    PickerConfirm => "picker_confirm",
    NextReference => "next_reference",
    PrevReference => "prev_reference",
    ToggleDigestSummary => "toggle_digest_summary",
    RestoreFiltered {
        target: MessageTarget = MessageTarget::Selection,
        restored: bool = true,
    } => "restore_filtered",
    FilteredTab1 => "filtered_tab_1",
    FilteredTab2 => "filtered_tab_2",
    FilteredTab3 => "filtered_tab_3",
    FilteredTab4 => "filtered_tab_4",
    FilteredTab5 => "filtered_tab_5",
    FilteredTab6 => "filtered_tab_6",
    FilteredTab7 => "filtered_tab_7",
    SourceTabVerification => "source_tab_verification",
    SourceTabHeaders => "source_tab_headers",
    SourceTabRaw => "source_tab_raw",
    CopySource => "copy_source",
    SaveSource => "save_source",
    UnfoldPart => "unfold_part",
    AcceptKey {
        message: Option<MessageId> = None,
        accepted: bool = true,
        kept: Option<String> = None,
    } => "accept_key",
    ForgetKey {
        message: Option<MessageId> = None,
        forgotten: bool = true,
        kept: Option<KeptKey> = None,
    } => "forget_key",
    ImportKey {
        message: Option<MessageId> = None,
    } => "import_key",
    /// The deliberate command that applies filtering to mail filed before it was turned on; the
    /// frontend shows what would move first.
    SweepInbox => "sweep_inbox",

    /// Frontend-only: the sheet writes through `Client::capture_task`.
    CaptureTask => "capture_task",
    CaptureNote => "capture_note",
    CaptureChangeProject => "capture_change_project",
    CaptureUseSubject => "capture_use_subject",
    CaptureWrite => "capture_write",
    /// A UI-only command with no store write; present only when `[focus.model]` has `like_this` on.
    DigestLikeThis {
        message: Option<MessageId> = None,
    } => "digest_like_this",

    OpenContacts => "open_contacts",
    ContactPopover {
        address: Option<String> = None,
    } => "contact_popover",
    ContactOpenCard => "contact_open_card",
    ContactAdd => "contact_add",
    ContactIgnore => "contact_ignore",
    ContactArrivalPrev => "contact_arrival_prev",
    ContactArrivalNext => "contact_arrival_next",
    ContactSetArrival {
        address: String = String::new(),
        arrival: Option<postio_model::ContactArrival> = None,
    } => "contact_set_arrival",
    ContactShowMail => "contact_show_mail",
    ContactCompose => "contact_compose",
    ContactsFilter => "contacts_filter",
    ContactsTabPrevious => "contacts_tab_previous",
    ContactsTabNext => "contacts_tab_next",
    ContactsToggleEveryone => "contacts_toggle_everyone",
    /// The word on screen is Merge; the id keeps the name the contracts gave it.
    ContactJoin {
        action: ContactJoinAction = ContactJoinAction::Ask,
    } => "contact_join",
    ContactAddAddress {
        action: ContactAddressAction = ContactAddressAction::Ask,
    } => "contact_add_address",
    ContactDetachAddress {
        address: Option<AddressId> = None,
    } => "contact_detach_address",
    ContactSetPreferred {
        person: Option<ContactId> = None,
        address: Option<AddressId> = None,
    } => "contact_set_preferred",
    ContactNew {
        action: ContactNewAction = ContactNewAction::Ask,
    } => "contact_new",
    ContactEdit {
        action: ContactEditAction = ContactEditAction::Ask,
    } => "contact_edit",
    /// The form is the controller's, so the command carries nothing.
    ContactSave => "contact_save",
    ContactDelete {
        person: Option<ContactId> = None,
        group: Option<ContactGroupId> = None,
    } => "contact_delete",
    ContactRestore {
        person: Option<ContactId> = None,
        /// Live when not given. Undo knows what they were and a key does not.
        state: Option<ContactState> = None,
    } => "contact_restore",
    ContactsToggleDeleted => "contacts_toggle_deleted",
    ContactsSuggestions => "contacts_suggestions",
    SuggestionDismiss {
        pair: Option<(ContactId, ContactId)> = None,
    } => "suggestion_dismiss",
    ContactGroupNew {
        action: ContactGroupNewAction = ContactGroupNewAction::Ask,
    } => "contact_group_new",
    ContactGroupRename {
        group: Option<ContactGroupId> = None,
        name: Option<String> = None,
    } => "contact_group_rename",
    ContactGroupAdd {
        group: Option<ContactGroupId> = None,
        people: Vec<ContactId> = Vec::new(),
    } => "contact_group_add",
    ContactGroupRemove {
        group: Option<ContactGroupId> = None,
        people: Vec<ContactId> = Vec::new(),
    } => "contact_group_remove",
    ContactsImport => "contacts_import",
    ContactsExport => "contacts_export",
    ContactsSync => "contacts_sync",

    ExpandThread => "expand_thread",
    CollapseThread => "collapse_thread",
    FirstInConversation => "first_in_thread",
    LatestInConversation => "latest_in_thread",
    FirstUnreadInConversation => "first_unread_in_thread",
    /// From inside the outline, where `g u` would read as two keys of a list's type-ahead.
    OutlineFirstUnread => "outline_first_unread",
    ThreadOutline => "thread_outline",
    ThreadMenu => "thread_menu",
    ReplyToLatest => "reply_to_latest",
    /// Frontends use `Req::Mute` and `Req::Unmute`; this is the form undo replays. `thread: None`
    /// asks which.
    MuteThread {
        thread: Option<ThreadId> = None,
        muted: bool = true,
    } => "mute_thread",
    /// Frontends use `Req::Split`. `rejoin` is undo's half: fold the split thread back into the
    /// original and forget the split.
    SplitThread {
        message: Option<MessageId> = None,
        rejoin: Option<(ThreadId, ThreadId)> = None,
    } => "split_thread",
    MoveThisMessage => "move_this_message",
    DeleteThisMessage => "delete_this_message",
    ForwardWhat1 => "forward_what_1",
    ForwardWhat2 => "forward_what_2",
    ForwardWhat3 => "forward_what_3",
    ShowNewer => "show_newer",
    ExpandQuoted => "expand_quoted",
    GoToCalendar => "go_to_calendar",
    CalendarAgenda => "calendar_agenda",
    CalendarPrevious => "calendar_previous",
    CalendarNext => "calendar_next",
    CalendarToday => "calendar_today",
    PopOut => "pop_out",
    PopIn => "pop_in",
    CloseWindow => "close_window",
    NewEvent => "new_event",
    EditEvent => "edit_event",
    MoveEvent => "move_event",
    DuplicateEvent => "duplicate_event",
    TentativeInvite {
        message: Option<MessageId> = None,
    } => "tentative_invite",
    /// Never sends: inviting is `SaveAndSendEvent`, so a save cannot reach anyone by accident.
    SaveEvent {
        event: Option<EventTarget> = None,
        calendar: Option<CalendarId> = None,
        fields: Option<EventFields> = None,
        scope: Option<EditScope> = None,
    } => "save_event",
    SaveAndSendEvent {
        event: Option<EventTarget> = None,
        calendar: Option<CalendarId> = None,
        fields: Option<EventFields> = None,
        scope: Option<EditScope> = None,
    } => "save_and_send_event",
    EmailAttendees => "email_attendees",
    ;
    /// The same verb as `ToggleRead` (same handler, one registry entry), differing only in not
    /// being recorded on the undo stack.
    /// Undo takes back what you did; one entry per message rested on would bury the archive you
    /// want back. `r` reverses it (#71).
    MarkReadOnDwell {
        message: MessageId,
    } => ToggleRead,
    /// The same verb as `e` elsewhere ("archive everything this row stands for"): `Command::id`
    /// answers `CommandId::Archive`, one registry row, one key.
    ArchiveDigest {
        delivery: postio_model::DeliveryId,
        archived: bool,
    } => Archive,
    /// Undo's way back from a sweep: `Command::id` answers `CommandId::SweepInbox`, so it needs no
    /// key or registry row of its own.
    UnsweepInbox {
        target: MessageTarget,
    } => SweepInbox,
    DiscardFolder {
        mailbox: MailboxId,
    } => CreateFolder,
    DeleteEvent {
        event: EventTarget,
        scope: Option<EditScope>,
    } => Delete,
    MoveEventTo {
        event: EventTarget,
        start: DateTime<Utc>,
        scope: Option<EditScope>,
    } => MoveEvent,
    DuplicateEventOf {
        event: EventId,
    } => DuplicateEvent,
    RevertEventChange {
        change: EventChangeId,
    } => SaveEvent,
}

impl fmt::Display for CommandId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A `[keys]` entry naming a command this build does not know is a warning, never a failure:
/// the config layer keeps the line so a downgrade and upgrade does not eat it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownCommand(String);

impl UnknownCommand {
    pub fn new(text: impl Into<String>) -> Self {
        UnknownCommand(text.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UnknownCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown command `{}`", self.0)
    }
}

impl std::error::Error for UnknownCommand {}

impl FromStr for CommandId {
    type Err = UnknownCommand;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        CommandId::ALL
            .iter()
            .copied()
            .find(|id| id.as_str() == text)
            .ok_or_else(|| UnknownCommand(text.to_owned()))
    }
}

impl Serialize for CommandId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CommandId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// A keystroke does not know which rows are selected, so the common case is `Selection` and app
/// state resolves it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageTarget {
    #[default]
    Selection,
    Messages(Vec<MessageId>),
    Thread(ThreadId),
    /// The unified list's group expansion (#184, ADR 0005 Q2): a deduped row's copies live in
    /// several accounts' threads, and one gesture must hit every copy as one undo.
    Threads(Vec<ThreadId>),
    /// Only undo builds one, to take back a whole-mailbox action: a bulk archive writes one queue
    /// row per message, so two integers name them all without holding the ids.
    Batch {
        range: OperationRange,
        /// Carried rather than looked up: a batch names no row to read an account off.
        account: AccountId,
        /// Carried because the handler needs it only to say which list reloads.
        /// `None` when the undone action never moved them together (a bulk flag over a smart
        /// folder, #52).
        from: Option<MailboxId>,
    },
}

/// A keystroke is `Ask`; undo invokes `Unjoin` with what the merge handed back: one verb, three
/// shapes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactJoinAction {
    Ask,
    Join {
        into: ContactId,
        others: Vec<ContactId>,
        name: String,
        organization: Option<String>,
    },
    Unjoin(JoinReceipt),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactNewAction {
    Ask,
    Create {
        name: Option<String>,
        addresses: Vec<EmailAddress>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactEditAction {
    Ask,
    Edit {
        person: ContactId,
        edit: PersonEdit,
    },
    Put {
        person: ContactId,
        fields: PersonFields,
    },
    Switch {
        address: String,
        switch: postio_model::CardSwitch,
        on: bool,
    },
    AddSender {
        address: String,
        details: postio_model::SignatureDetails,
    },
    RestoreSender {
        person: ContactId,
        prior: postio_model::SenderPrior,
    },
    Detail {
        address: String,
        kind: postio_model::DetailKind,
        value: String,
        keep: Option<bool>,
    },
    Ignore {
        address: String,
        ignored: bool,
    },
    Save {
        person: Option<ContactId>,
        form: Box<postio_model::ContactForm>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactGroupNewAction {
    Ask,
    Create {
        name: String,
        members: Vec<ContactId>,
    },
    Restore {
        group: ContactGroup,
        members: Vec<ContactId>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactAddressAction {
    Ask,
    /// Refused while a live person owns the address.
    Add {
        person: ContactId,
        address: EmailAddress,
    },
    /// `to: None` lets go of the address; also how undo puts one back.
    Put {
        address: AddressId,
        to: Option<ContactId>,
        /// Undo's half: the state `to` was in before an earlier move left them with no address.
        revive: Option<ContactState>,
    },
}

/// Kept by the undo entry of a verb that changed it; `position` matters because file order is match
/// order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptKey {
    pub fingerprint: String,
    pub keyblock: Vec<u8>,
    pub primary_uid: Option<String>,
    pub sources: Vec<String>,
    pub addresses: Vec<String>,
    pub first_seen: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptRule {
    pub position: u32,
    pub toml: String,
}

impl Command {
    /// A caller re-aiming a verb (the list does, for a row standing for a conversation, ADR 0015
    /// Q3) must tell one already aimed from one pointing at the selection.
    pub fn target(&self) -> Option<&MessageTarget> {
        match self {
            Command::Archive { target }
            | Command::Delete { target }
            | Command::Move { target, .. }
            | Command::Flag { target, .. }
            | Command::ToggleRead { target, .. }
            | Command::Snooze { target, .. }
            | Command::Unsnooze { target }
            | Command::RemindIfNoReply { target, .. }
            | Command::AddLabel { target, .. }
            | Command::DigestRule { target }
            | Command::StopDigestingSender { target, .. }
            | Command::DismissMarker { target, .. }
            | Command::RestoreFiltered { target, .. } => Some(target),
            _ => None,
        }
    }

    /// Commands that do not act on messages are returned unchanged: pointing `Send` at a selection
    /// is meaningless.
    #[must_use]
    pub fn with_target(self, target: MessageTarget) -> Self {
        match self {
            Command::Archive { .. } => Command::Archive { target },
            Command::Delete { .. } => Command::Delete { target },
            Command::Move { to, .. } => Command::Move { target, to },
            Command::Flag { flagged, .. } => Command::Flag { target, flagged },
            Command::ToggleRead { unread, .. } => Command::ToggleRead { target, unread },
            Command::Snooze { until, .. } => Command::Snooze { target, until },
            Command::Unsnooze { .. } => Command::Unsnooze { target },
            Command::RemindIfNoReply { at, .. } => Command::RemindIfNoReply { target, at },
            Command::AddLabel { label, on, .. } => Command::AddLabel { target, label, on },
            Command::DigestRule { .. } => Command::DigestRule { target },
            Command::StopDigestingSender { stopped, kept, .. } => Command::StopDigestingSender {
                target,
                stopped,
                kept,
            },
            Command::RestoreFiltered { restored, .. } => {
                Command::RestoreFiltered { target, restored }
            }
            Command::DismissMarker { dismissed, .. } => {
                Command::DismissMarker { target, dismissed }
            }
            other => other,
        }
    }

    /// Destructive commands always carry a `Recovery` (see `crate::registry`).
    pub fn is_destructive(&self) -> bool {
        crate::registry::get(self.id()).destructive
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_snake_case_and_stable() {
        for id in CommandId::ALL {
            let text = id.as_str();
            assert!(
                text.chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit()),
                "`{text}` is not a stable snake_case id"
            );
        }
    }

    #[test]
    fn an_unknown_id_reports_what_it_saw() {
        let error = "summarize".parse::<CommandId>().unwrap_err();
        assert_eq!(error.as_str(), "summarize");
        assert!(error.to_string().contains("summarize"));
    }

    #[test]
    fn a_targeted_command_keeps_its_target() {
        let command = Command::Move {
            target: MessageTarget::Thread(ThreadId::new(3)),
            to: Some(MailboxId::new(9)),
        };
        assert_eq!(command.id(), CommandId::Move);
        assert_ne!(command, Command::default_for(CommandId::Move));
    }

    #[test]
    fn a_command_can_be_pointed_at_one_message() {
        let one = MessageTarget::Messages(vec![MessageId::new(7)]);

        assert_eq!(
            Command::Archive {
                target: MessageTarget::Selection
            }
            .with_target(one.clone()),
            Command::Archive {
                target: one.clone()
            }
        );
        assert_eq!(
            Command::Move {
                target: MessageTarget::Selection,
                to: Some(MailboxId::new(3)),
            }
            .with_target(one.clone()),
            Command::Move {
                target: one,
                to: Some(MailboxId::new(3)),
            },
            "retargeting keeps the rest of the invocation"
        );
    }

    #[test]
    fn a_command_that_does_not_act_on_messages_ignores_a_target() {
        let command = Command::Send;

        assert_eq!(
            command
                .clone()
                .with_target(MessageTarget::Messages(vec![MessageId::new(1)])),
            command
        );
    }

    #[test]
    fn the_default_target_is_the_selection() {
        assert_eq!(MessageTarget::default(), MessageTarget::Selection);
    }

    #[test]
    fn every_verb_that_acts_on_messages_reports_and_takes_a_target() {
        // `target` and `with_target` list the same verbs by hand, so every default invocation goes
        // through both.
        let aimed = MessageTarget::Thread(ThreadId::new(5));
        let mut aimable = 0;
        for id in CommandId::ALL {
            let verb = Command::default_for(*id);
            let moved = verb.clone().with_target(aimed.clone());
            assert_eq!(moved.id(), verb.id(), "retargeting keeps the verb");
            if verb.target().is_some() {
                aimable += 1;
                assert_eq!(moved.target(), Some(&aimed), "{id} took the target");
            } else {
                assert_eq!(moved, verb, "{id} acts on no messages, so it is unchanged");
            }
        }
        assert!(aimable >= 13, "only {aimable} verbs take a target");
    }
}
