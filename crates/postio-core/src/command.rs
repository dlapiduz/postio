//! Commands: the one way anything happens. The frontend sends a `Command` and reacts to events.
//!
//! `CommandId` is a stable string: `[keys]` in `config.toml` names commands by id, so a rename
//! breaks configs.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use postio_model::{
    AccountId, AddressId, ContactGroup, ContactGroupId, ContactId, ContactState, DraftId,
    EmailAddress, JoinReceipt, LabelId, MailboxId, MailboxRole, MessageId, OperationRange,
    PersonEdit, PersonFields, ThreadId,
};

/// How long an RSVP answer waits in the outbox, and so can be taken back; matches the toast's life.
pub const RSVP_WINDOW: std::time::Duration = std::time::Duration::from_secs(10);
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Declares each command once: the `CommandId`, the `Command` variant, `id` and `default_for`.
/// Entries after the `;` are other invocations of an existing id: who asked differs, the verb does
/// not.
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
        /// Serializes as its string id.
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

        /// Fields a keystroke cannot supply are `Option`: `None` means ask the user (`Move` with no
        /// mailbox opens the picker).
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

            /// Targets the current selection and leaves every choice to app state, so one registry
            /// feeds both the keymap and the palette.
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
    /// Stays a predicate, never a list of ids; see `state::Selection`.
    SelectAll => "select_all",
    PrevView => "prev_view",
    Back => "back",
    ToggleResultOrder => "toggle_result_order",
    /// Moves between messages inside the open conversation; `j`/`k` move between threads (#1007).
    NextInConversation => "next_in_thread",
    PrevInConversation => "prev_in_thread",
    /// The only way to collapse the focused message, since landing on one expands it.
    ToggleFold => "toggle_fold",
    /// No payload: reader view is per-message, so it always means the message on screen.
    ViewOriginal => "view_original",
    /// No payload. Every message opens as sent, so this is the only way into reader view.
    ToggleReaderView => "toggle_reader_view",
    DarkenMessage => "darken_message",
    /// Per message; "Always for this sender" is the reader's to store.
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
        /// The destination; `None` opens the mailbox picker.
        to: Option<MailboxId> = None,
    } => "move",
    Flag {
        target: MessageTarget = MessageTarget::Selection,
        /// The state to set; `None` toggles.
        flagged: Option<bool> = None,
    } => "flag",
    ToggleRead {
        target: MessageTarget = MessageTarget::Selection,
        /// The state to set; `None` toggles.
        unread: Option<bool> = None,
    } => "toggle_read",
    /// `None` from a keystroke with no picker behind it: the handler's default answers.
    Snooze {
        target: MessageTarget = MessageTarget::Selection,
        /// When it comes back; `None` for the handler's default.
        until: Option<DateTime<Utc>> = None,
    } => "snooze",
    Unsnooze {
        target: MessageTarget = MessageTarget::Selection,
    } => "unsnooze",
    /// `None` clears the reminder; a value rather than a toggle, so undo can dispatch the inverse.
    RemindIfNoReply {
        target: MessageTarget = MessageTarget::Selection,
        /// When to bring it back; `None` clears the reminder.
        at: Option<DateTime<Utc>> = None,
    } => "remind_if_no_reply",
    AddLabel {
        target: MessageTarget = MessageTarget::Selection,
        /// The label; `None` opens the label picker.
        label: Option<LabelId> = None,
        /// On, off, or `None` to toggle; a value so undo can dispatch the inverse (#780).
        on: Option<bool> = None,
    } => "add_label",
    /// Undone within a window, like a send, not from the undo stack.
    AcceptInvite {
        message: Option<MessageId> = None,
    } => "accept_invite",
    DeclineInvite {
        message: Option<MessageId> = None,
    } => "decline_invite",
    /// Open the digest-rule dialog: a new rule for the target's sender, or,
    /// in a digest, the digest's own rule and cadence.
    DigestRule {
        target: MessageTarget = MessageTarget::Selection,
    } => "digest_rule",
    /// A direction, not a toggle, so undo can dispatch the inverse.
    StopDigestingSender {
        target: MessageTarget = MessageTarget::Selection,
        /// `true` stops the digest; `false` is undo's way back.
        stopped: bool = true,
        /// Undo's memory, `None` from a keystroke: the rule before the sender left, since a rule
        /// left empty is removed.
        kept: Option<KeptRule> = None,
    } => "stop_digesting_sender",
    ViewSource {
        message: Option<MessageId> = None,
    } => "view_source",
    OpenAttachmentOrLink {
        message: Option<MessageId> = None,
    } => "open_attachment_or_link",
    /// A dismissal is a correction: it never returns on that message, and repeated dismissals
    /// stop that kind for the sender.
    DismissMarker {
        target: MessageTarget = MessageTarget::Selection,
        /// `true` dismisses; `false` is undo's way back.
        dismissed: bool = true,
    } => "dismiss_marker",
    /// The verbs a narrow message dialog folds out of its action row: Label, Move and Delete.
    MoreActions => "more_actions",
    /// Config key `[focus] reading`.
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
    /// For a user who learnt the message arrived otherwise: discarding loses it and resending
    /// duplicates it (ADR 0021 Decision 3, #674).
    MarkSent {
        draft: Option<DraftId> = None,
    } => "mark_sent",
    /// The way out of `Failed` and `Unconfirmed` other than discarding; the Outbox has no composer
    /// open to press Send in.
    RetrySend {
        draft: Option<DraftId> = None,
    } => "retry_send",
    /// Refused once submission has started: never cancel what may be in flight (ADR 0021).
    CancelSend {
        draft: Option<DraftId> = None,
    } => "cancel_send",
    AttachFile {
        /// The file; `None` opens the file chooser.
        path: Option<PathBuf> = None,
    } => "attach_file",
    /// One composition either way: detaching moves the draft rather than forking it.
    DetachComposer => "detach_composer",
    /// Putting them away only works while both are empty, else hidden recipients would still be
    /// sent to; otherwise it moves the keyboard to `Cc`.
    CopyFields => "copy_fields",
    /// Pasting and dropping already reached it; this makes it reachable from the palette and `?`.
    InsertImage => "insert_image",
    /// Where the body is not text an editor can open (the desktop's rich editor) the frontend says
    /// so.
    EditExternally => "edit_externally",
    /// The desktop composer shows formatting as written, so it has nothing to preview.
    TogglePreview => "toggle_preview",
    Bold => "bold",
    Italic => "italic",
    /// Only the first level has a command; `heading_2` and `heading_3` are separate ids.
    Heading1 => "heading_1",
    BulletList => "bullet_list",
    NumberedList => "numbered_list",
    InsertLink => "insert_link",
    QuoteBlock => "quote_block",

    Undo => "undo",
    CommandPalette => "command_palette",
    CheatSheet => "cheat_sheet",
    Settings => "settings",
    /// A registered command so the palette and cheat sheet carry it (ADR 0012 Q1).
    AddAccount => "add_account",
    EditConfig => "edit_config",
    /// A command so a terminal, with no close button, can reach it (Principle II).
    Quit => "quit",
    ShowImages => "show_images",
    AlwaysShowImages => "always_show_images",
    /// One-click `List-Unsubscribe`, only on this deliberate act.
    Unsubscribe => "unsubscribe",
    /// The unsubscribe notice's button as a command (#1953). The frontend holds the pending offer,
    /// so there is no payload; with no offer on screen it says so and does nothing.
    ClearListMail => "clear_list_mail",
    GoToFolders => "go_to_folders",
    /// One variant per role, since `Command::id` is total and a `GoTo(MailboxRole)` would have no
    /// id.
    /// Other roles are reached through `#`.
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
    /// One variant per entry, for the same reason as the destinations.
    SavedSearch1 => "saved_search_1",
    SavedSearch2 => "saved_search_2",
    SavedSearch3 => "saved_search_3",
    SavedSearch4 => "saved_search_4",
    ToggleHasAction => "toggle_has_action",
    /// No payload: offered only while an account row has focus, so the target is that row (ADR 0005
    /// Q6c).
    ToggleAccountEnabled => "toggle_account_enabled",
    RemoveAccount => "remove_account",
    UpdateCredential => "update_credential",
    /// Local mail only; nothing here reaches the network (#981).
    RebuildAccountIndex => "rebuild_account_index",
    /// Settles which account a new message comes from; a reply's from address is
    /// `postio_model::reply`'s.
    SetDefaultAccount => "set_default_account",
    /// `path: None` is a value, back to automatic, not "ask"; `account` and `role` `None` ask (ADR
    /// 0035).
    MapMailboxRole {
        account: Option<AccountId> = None,
        role: Option<MailboxRole> = None,
        path: Option<String> = None,
    } => "map_mailbox_role",
    /// Cycles because a keystroke carries no argument; the sidebar rows name a scope directly.
    NextScope => "next_scope",
    Refresh => "refresh",

    ScrollReaderDown => "scroll_reader_down",
    ScrollReaderUp => "scroll_reader_up",

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
    /// Applies filtering to mail filed before it was turned on; the frontend previews with
    /// `Client::sweep_preview` first.
    SweepInbox => "sweep_inbox",

    /// Frontend-only: the sheet writes through `Client::capture_task`.
    CaptureTask => "capture_task",
    CaptureNote => "capture_note",
    CaptureChangeProject => "capture_change_project",
    CaptureUseSubject => "capture_use_subject",
    CaptureWrite => "capture_write",
    /// Present only when `[focus.model]` has `like_this` on; UI-only, no store write.
    DigestLikeThis {
        message: Option<MessageId> = None,
    } => "digest_like_this",

    OpenContacts => "open_contacts",
    /// The sender under the cursor, or `address` when a name was clicked.
    ContactPopover {
        address: Option<String> = None,
    } => "contact_popover",
    /// Open the contact's card from the popover. Held until the card exists.
    ContactOpenCard => "contact_open_card",
    ContactAdd => "contact_add",
    ContactIgnore => "contact_ignore",
    ContactArrivalPrev => "contact_arrival_prev",
    ContactArrivalNext => "contact_arrival_next",
    /// Applies at once and can be undone.
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
    /// Both `None` means whichever row is focused.
    ContactDelete {
        person: Option<ContactId> = None,
        group: Option<ContactGroupId> = None,
    } => "contact_delete",
    ContactRestore {
        person: Option<ContactId> = None,
        /// Live when not given; undo knows what they were and a key does not.
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
        /// Who; empty means the marked people, or the one under the cursor.
        people: Vec<ContactId> = Vec::new(),
    } => "contact_group_add",
    ContactGroupRemove {
        group: Option<ContactGroupId> = None,
        /// Who; empty means the marked people, or the one under the cursor.
        people: Vec<ContactId> = Vec::new(),
    } => "contact_group_remove",
    ContactsImport => "contacts_import",
    ContactsExport => "contacts_export",
    /// A dry run: shows what would happen before anything does.
    ContactsSync => "contacts_sync",

    ExpandThread => "expand_thread",
    CollapseThread => "collapse_thread",
    FirstInConversation => "first_in_thread",
    LatestInConversation => "latest_in_thread",
    FirstUnreadInConversation => "first_unread_in_thread",
    /// Inside the outline `g u` would read as two keys of a list's type-ahead.
    OutlineFirstUnread => "outline_first_unread",
    ThreadOutline => "thread_outline",
    ThreadMenu => "thread_menu",
    ReplyToLatest => "reply_to_latest",
    /// Frontends reach it through `Req::Mute` and `Req::Unmute`; this is the form undo replays.
    MuteThread {
        thread: Option<ThreadId> = None,
        muted: bool = true,
    } => "mute_thread",
    /// `rejoin` is undo's half: fold the split thread back into the original, and forget the split.
    SplitThread {
        /// The message to split off, with the replies to it.
        message: Option<MessageId> = None,
        /// Undo's half: `(split thread, original thread)`.
        rejoin: Option<(ThreadId, ThreadId)> = None,
    } => "split_thread",
    MoveThisMessage => "move_this_message",
    DeleteThisMessage => "delete_this_message",
    ForwardWhat1 => "forward_what_1",
    ForwardWhat2 => "forward_what_2",
    ForwardWhat3 => "forward_what_3",
    ShowNewer => "show_newer",
    ExpandQuoted => "expand_quoted",
    ;
    /// The same verb as `ToggleRead`, so `id` answers it and the registry has one entry.
    ///
    /// Not recorded on the undo stack: one per message rested on would bury the archive you want
    /// back. The reversal is `r` (#71).
    MarkReadOnDwell {
        message: MessageId,
    } => ToggleRead,
    /// The same verb as `e` elsewhere, so `Command::id` answers `Archive`: one registry row, one
    /// key.
    ArchiveDigest {
        delivery: postio_model::DeliveryId,
        archived: bool,
    } => Archive,
    /// The same verb, the other direction: `Command::id` answers `SweepInbox`, so it needs no key
    /// or registry row.
    UnsweepInbox {
        target: MessageTarget,
    } => SweepInbox,
}

impl fmt::Display for CommandId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An unknown `[keys]` entry is a warning, not a failure: the config layer keeps the line so a
/// downgrade and upgrade round trip does not eat it.
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
    /// A deduped unified-list row stands for one conversation with copies in several accounts,
    /// hit in one gesture (#184, ADR 0005 Q2).
    Threads(Vec<ThreadId>),
    /// Only undo builds one: a bulk archive writes one queue row per message, so the run names all
    /// of them in two integers (see `OperationRange`).
    Batch {
        range: OperationRange,
        /// Carried rather than looked up: a batch names no row to read an account off.
        account: AccountId,
        /// Carried, not derived, to avoid a store read over the whole batch. `None` when the undone
        /// action never moved them together, as a bulk flag over a smart folder (#52).
        from: Option<MailboxId>,
    },
}

/// A keystroke is `Ask`: Contacts asks which name the merged person keeps, then invokes `Join`.
/// Undo invokes `Unjoin` with what the merge handed back.
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
    /// Keep (`Some(true)`), refuse (`Some(false)`) or forget (`None`) a phone
    /// or website of the person at `address`; undo puts it back.
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
        /// Whose; `None` makes a new person.
        person: Option<ContactId>,
        form: Box<postio_model::ContactForm>,
    },
}

/// What a [`Command::ContactGroupNew`] asks for.
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
    /// Give `person` the typed `address`; refused while a live person owns it.
    Add {
        person: ContactId,
        address: EmailAddress,
    },
    /// Give `address` to `to`, whoever has it -- the "move it" answer, and
    /// the way undo puts an address back. `to: None` lets go of it.
    Put {
        address: AddressId,
        to: Option<ContactId>,
        /// Undo's half: the state `to` was in before an earlier move left
        /// them with no address, to bring them back in.
        revive: Option<ContactState>,
    },
}

/// A digest rule as it stood, kept by undo: its TOML and its position, since file order is match
/// order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptRule {
    pub position: u32,
    pub toml: String,
}

impl Command {
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

    /// Commands that act on something other than a set of messages are returned unchanged.
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

    /// Destructive commands always carry a `Recovery` (see `registry`).
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
        // `target` and `with_target` list the same verbs by hand; exercise both for every default.
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
