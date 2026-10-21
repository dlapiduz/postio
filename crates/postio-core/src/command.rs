//! Commands: the one way anything happens in Postio.
//!
//! The frontend never mutates state directly. It sends a [`Command`] down and
//! reacts to [`Event`](crate::Event)s coming up, which is what keeps a second
//! frontend possible and what lets the keymap, the `Ctrl+K` palette, the `?`
//! cheat sheet and the right-click menu all be generated from one table.
//!
//! # Ids are a file format
//!
//! [`CommandId`] is a stable string, not an integer: `[keys]` in `config.toml`
//! refers to commands by id, so renaming one silently breaks a user's
//! configuration. The strings here are the same vocabulary
//! `postio-config`'s `DEFAULT_BINDINGS` uses, and a test in
//! `tests/command_registry.rs` holds the two together.
//!
//! # Ids versus commands
//!
//! A [`CommandId`] names *what can be done* and is what the registry, the
//! keymap and the palette deal in. A [`Command`] is *one invocation*, and may
//! carry a target or a payload the id cannot express. `Command::default_for`
//! turns an id into the invocation a keystroke or a palette row implies, with
//! the payload left for app state to resolve.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use postio_model::{
    AccountId, AddressId, ContactGroup, ContactGroupId, ContactId, ContactState, DraftId,
    EmailAddress, JoinReceipt, LabelId, MailboxId, MailboxRole, MessageId, OperationRange,
    PersonEdit, PersonFields, ThreadId,
};

/// How long an answer to an invitation waits in the outbox before it may
/// leave, and so how long it can be taken back (specs/007-postio-focus
/// FR-102, research R9): about ten seconds, the toast's own life. Here
/// rather than with the verb that applies it, so every frontend's toast
/// says the same window the engine keeps.
pub const RSVP_WINDOW: std::time::Duration = std::time::Duration::from_secs(10);
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Declares every command once, and generates what used to be written out
/// four times: the [`CommandId`] list, the [`Command`] enum, the arm of
/// [`Command::id`] and the arm of [`Command::default_for`].
///
/// Each entry is one verb: its docs, its payload (each field with the value
/// a keystroke implies, which is what `default_for` builds), and its stable
/// string id. The entries after the `;` are invocations of a verb that
/// another id already names -- who asked differs, the verb does not -- so
/// they carry the id they answer and have no default of their own.
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
        /// The stable identifier of a command.
        ///
        /// Serializes as its string id, so a `[keys]` entry, a palette row and a
        /// log line all spell a command the same way.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum CommandId {
            $( $(#[$meta])* $variant, )+
        }

        impl CommandId {
            /// Every command id, in the order the cheat sheet and palette list
            /// them: navigation, message actions, search, compose, view.
            pub const ALL: &'static [CommandId] = &[ $( CommandId::$variant, )+ ];

            /// The stable string id, as `[keys]` in `config.toml` spells it.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $( CommandId::$variant => $id, )+
                }
            }
        }

        /// One invocation of a command.
        ///
        /// Payload fields that a keystroke cannot supply are `Option`al: `None` means
        /// "ask the user" — [`Command::Move`] with no mailbox opens the mailbox picker,
        /// [`Command::Search`] with no query focuses the search field.
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
            /// The registry id this invocation belongs to.
            pub fn id(&self) -> CommandId {
                match self {
                    $( Command::$variant { .. } => CommandId::$variant, )+
                    $( Command::$alias { .. } => CommandId::$target, )*
                }
            }

            /// The invocation a keystroke or a palette row implies: it targets the
            /// current selection and leaves every choice for app state to resolve.
            ///
            /// This is what makes one registry able to feed both the keymap and the
            /// palette — neither has to know a command's payload shape.
            pub fn default_for(id: CommandId) -> Command {
                match id {
                    $( CommandId::$variant => Command::$variant $({ $( $field: $default ),* })?, )+
                }
            }
        }
    };
}

commands! {
    // -- Navigation ------------------------------------------------------
    /// Select the next message.
    NextMessage => "next_message",
    /// Select the previous message.
    PrevMessage => "prev_message",
    /// Select the first message.
    FirstMessage => "first_message",
    /// Select the last message.
    LastMessage => "last_message",
    /// Open a message, or the focused one when `message` is `None`.
    OpenMessage {
        /// The message to open; `None` means the focused row.
        message: Option<MessageId> = None,
    } => "open_message",
    /// Add a message to the selection, or take it out again.
    ToggleSelection {
        /// The message to toggle; `None` means the focused row.
        message: Option<MessageId> = None,
    } => "toggle_selection",
    /// Extend the selection to the next message down.
    ExtendSelectionDown => "extend_selection_down",
    /// Extend the selection to the previous message up.
    ExtendSelectionUp => "extend_selection_up",
    /// Select every message the list is showing.
    ///
    /// Never resolved into a list of ids on the way through — see
    /// [`crate::state::Selection`] for why that matters.
    SelectAll => "select_all",
    /// Return to the previous view.
    PrevView => "prev_view",
    /// Leave the current overlay, search or composer.
    Back => "back",
    /// Switch a search's results between ranked and date order (#499).
    ToggleResultOrder => "toggle_result_order",
    /// Walk down the open conversation's stack (#1007).
    ///
    /// No payload: `j`/`k` move between *threads* in the list, and these
    /// move between messages inside the one that is open. Two axes, two
    /// pairs of keys, and which one you are on is a fact about where the
    /// keyboard is rather than about what you pressed.
    NextInConversation => "next_in_conversation",
    /// Walk up the open conversation's stack (#1007).
    PrevInConversation => "prev_in_conversation",
    /// Fold or unfold the conversation's focused message (#1007).
    ///
    /// The only way to *collapse* the focused message: landing on one
    /// expands it, so a collapsed-and-focused message is a state only this
    /// reaches.
    ToggleFold => "toggle_fold",
    /// Leave reader view for the sender's own markup (#1009).
    ///
    /// No payload: it always means the message on screen. Reader view is a
    /// per-message state, so there is nothing else it could mean.
    ViewOriginal => "view_original",
    /// Draw the message on screen in reader view, or back as its sender
    /// built it (spec 006 FR-031).
    ///
    /// No payload, like [`Command::ViewOriginal`]: it always means the
    /// message on screen. Every message opens as sent, so this is the only
    /// way into reader view.
    ToggleReaderView => "toggle_reader_view",
    /// Darken the focused paper message, or show it as sent again (spec 006 FR-013a). No payload: it means the message on screen.
    DarkenMessage => "darken_message",
    /// Switch the message on screen between app colours and the original on
    /// paper (specs/007-postio-focus T213). No payload: it means the message
    /// on screen, and is the message's alone -- "Always for this sender" is
    /// the reader's to store, beside the other per-sender settings.
    SwitchTreatment => "switch_treatment",
    /// Find text in the message on screen (spec 006 FR-018). No payload: it means the message on screen.
    FindInMessage => "find_in_message",
    /// Go to the next match of the open find (spec 006 FR-018). No payload: it means the message on screen.
    FindNext => "find_next",
    /// Go to the previous match of the open find (spec 006 FR-018). No payload: it means the message on screen.
    FindPrevious => "find_previous",
    /// Draw messages one step larger (spec 006 FR-021). No payload: it means the message on screen.
    ZoomIn => "zoom_in",
    /// Draw messages one step smaller (spec 006 FR-021). No payload: it means the message on screen.
    ZoomOut => "zoom_out",
    /// Draw messages at their actual size (spec 006 FR-021). No payload: it means the message on screen.
    ZoomReset => "zoom_reset",
    /// Expand every collapsed message in the open conversation (#1004).
    ///
    /// No payload: it means the conversation on screen, which is the only
    /// one there is.
    ExpandAll => "expand_all",

    // -- Message actions -------------------------------------------------
    /// Reply to the sender.
    Reply {
        /// The message to reply to; `None` means the focused message.
        message: Option<MessageId> = None,
    } => "reply",
    /// Reply to everyone.
    ReplyAll {
        /// The message to reply to; `None` means the focused message.
        message: Option<MessageId> = None,
    } => "reply_all",
    /// Forward a message.
    Forward {
        /// The message to forward; `None` means the focused message.
        message: Option<MessageId> = None,
    } => "forward",
    /// Archive messages.
    Archive {
        /// What to archive.
        target: MessageTarget = MessageTarget::Selection,
    } => "archive",
    /// Archive a whole thread.
    ArchiveThread {
        /// The thread to archive; `None` means the focused message's thread.
        thread: Option<ThreadId> = None,
    } => "archive_thread",
    /// Move messages to the trash.
    Delete {
        /// What to delete.
        target: MessageTarget = MessageTarget::Selection,
    } => "delete",
    /// Move messages to another mailbox.
    Move {
        /// What to move.
        target: MessageTarget = MessageTarget::Selection,
        /// The destination; `None` opens the mailbox picker.
        to: Option<MailboxId> = None,
    } => "move",
    /// Set or toggle the flagged state.
    Flag {
        /// What to flag.
        target: MessageTarget = MessageTarget::Selection,
        /// The state to set; `None` toggles.
        flagged: Option<bool> = None,
    } => "flag",
    /// Set or toggle the unread state.
    ToggleRead {
        /// What to mark.
        target: MessageTarget = MessageTarget::Selection,
        /// The state to set; `None` toggles.
        unread: Option<bool> = None,
    } => "toggle_read",
    /// Hide the selection from every ordinary list until a time.
    ///
    /// The time is the picker's (specs/007-postio-focus research R6): one of
    /// the shared presets, or a date typed and read by `parse_when`. `None`
    /// is what a keystroke with no picker behind it sends -- the classic
    /// app's `s` -- and the handler's own default answers it, as it always
    /// has.
    Snooze {
        /// What to snooze.
        target: MessageTarget = MessageTarget::Selection,
        /// When it comes back; `None` for the handler's default.
        until: Option<DateTime<Utc>> = None,
    } => "snooze",
    /// Cancel a snooze immediately.
    Unsnooze {
        /// What to unsnooze.
        target: MessageTarget = MessageTarget::Selection,
    } => "unsnooze",
    /// Bring the conversation back if nobody has replied by a time, or stop
    /// waiting for a reply.
    ///
    /// The time is the remind picker's, one of the shared presets or a typed
    /// date (specs/007-postio-focus research R6). `None` clears the
    /// conversation's reminder: it is the value rather than a toggle, so
    /// undo can dispatch the inverse, as [`Command::AddLabel`]'s `on` does.
    /// Focus opens the picker on the key; nothing sends a keystroke's
    /// `None` meaning "ask".
    RemindIfNoReply {
        /// Which conversations: each targeted message's.
        target: MessageTarget = MessageTarget::Selection,
        /// When to bring it back; `None` clears the reminder.
        at: Option<DateTime<Utc>> = None,
    } => "remind_if_no_reply",
    /// Attach a label, or take one off.
    AddLabel {
        /// What to label.
        target: MessageTarget = MessageTarget::Selection,
        /// The label; `None` opens the label picker.
        label: Option<LabelId> = None,
        /// On, off, or `None` to toggle.
        ///
        /// [`Command::Flag`]'s shape, and for its reason: `u` takes an action
        /// back by *dispatching its inverse*, so removing a label has to be
        /// something a `Command` can say. One registered verb that can do
        /// both beats a second entry in the registry that has no binding, no
        /// menu item and no way for a person to reach it -- which is what
        /// `AddLabel` itself was before #766 removed it (#780).
        on: Option<bool> = None,
    } => "add_label",
    /// Accept the invitation a message carries, and send the reply.
    ///
    /// Undone within a window rather than from the undo stack, the way a send
    /// is: the reply is on its way once the window closes (research R9).
    AcceptInvite {
        /// The invitation; `None` means the focused message.
        message: Option<MessageId> = None,
    } => "accept_invite",
    /// Decline the invitation a message carries, and send the reply.
    DeclineInvite {
        /// The invitation; `None` means the focused message.
        message: Option<MessageId> = None,
    } => "decline_invite",
    /// Open the digest-rule dialog: a new rule for the target's sender, or,
    /// in a digest, the digest's own rule and cadence.
    DigestRule {
        /// Whose sender the rule is for.
        target: MessageTarget = MessageTarget::Selection,
    } => "digest_rule",
    /// Stop gathering a sender's mail into a digest, or start again.
    ///
    /// A direction rather than a toggle, so undo can dispatch the inverse.
    StopDigestingSender {
        /// Whose sender.
        target: MessageTarget = MessageTarget::Selection,
        /// `true` stops the digest; `false` is undo's way back.
        stopped: bool = true,
        /// Undo's memory, `None` from a keystroke: the rule as it stood
        /// before the sender was taken out of it, so taking that back puts
        /// the rule back exactly -- a rule left holding nobody is removed,
        /// and undo has to be able to write it again.
        kept: Option<KeptRule> = None,
    } => "stop_digesting_sender",
    /// Show the message as it arrived: its raw RFC 822 text (research R2).
    ViewSource {
        /// The message; `None` means the one on screen.
        message: Option<MessageId> = None,
    } => "view_source",
    /// Choose one of a message's links or parts, and open it.
    OpenAttachmentOrLink {
        /// The message; `None` means the one on screen.
        message: Option<MessageId> = None,
    } => "open_attachment_or_link",
    /// Take a wrong marker off a message, or bring it back (spec 007
    /// FR-108). A dismissal is a correction: it never returns on that
    /// message, and dismissing one kind from one sender again and again
    /// stops that kind for the sender.
    DismissMarker {
        /// Whose markers.
        target: MessageTarget = MessageTarget::Selection,
        /// `true` dismisses; `false` is undo's way back.
        dismissed: bool = true,
    } => "dismiss_marker",
    /// Show the verbs a narrow message dialog folds out of its action row:
    /// Label, Move and Delete (spec 007 T206).
    MoreActions => "more_actions",
    /// Open messages in a pane beside the list, or in the dialog over it
    /// (spec 007 T232): `[focus] reading`.
    ToggleReadingPane => "toggle_reading_pane",

    // -- Search ----------------------------------------------------------
    /// Search, or focus the search field when `query` is `None`.
    Search {
        /// The query to run; `None` focuses the empty search field.
        query: Option<String> = None,
    } => "search",
    /// Save the query the search box currently holds as a pinned folder.
    ///
    /// No payload: the box being open is what says which query, the same
    /// way `EditConfig` needs no path because there is only one file.
    SaveSearch => "save_search",
    /// Leave a search's chips for the plain words it was typed as.
    BackToWords => "back_to_words",
    /// Open the words in the search bar as a results view.
    ShowAllResults => "show_all_results",
    /// Forget the highlighted recent search.
    ForgetRecent => "forget_recent",
    /// Exclude the dropdown's focused person, label or folder: its chip,
    /// negated (spec 010 US7).
    ExcludeSuggestion => "exclude_suggestion",
    /// Back through the search history.
    HistoryBack => "history_back",
    /// Forward through the search history.
    HistoryForward => "history_forward",
    /// The results' Conversations tab.
    ResultsConversations => "results_conversations",
    /// The results' Files tab.
    ResultsFiles => "results_files",
    /// The results' People tab.
    ResultsPeople => "results_people",
    /// Step the timeline's range a month earlier (`after:` and `before:`
    /// both).
    StepRangeBack => "step_range_back",
    /// Step the timeline's range a month later.
    StepRangeForward => "step_range_forward",
    /// Open Quick Look on the focused result, or close it (spec 010 US4).
    QuickLook => "quick_look",
    /// Ring Quick Look's next match card.
    NextMatch => "next_match",
    /// Ring Quick Look's previous match card.
    PrevMatch => "prev_match",
    /// Run the first looser search a search that found nothing offers
    /// (spec 010 US6).
    PickRelaxation1 => "pick_relaxation_1",
    /// Run the second looser search.
    PickRelaxation2 => "pick_relaxation_2",
    /// Run the third looser search.
    PickRelaxation3 => "pick_relaxation_3",
    /// Run the fourth looser search.
    PickRelaxation4 => "pick_relaxation_4",
    /// Save the file under the ring on the results' Files tab.
    SaveFile => "save_file",

    // -- Compose ---------------------------------------------------------
    /// Start a new message, optionally from an existing draft.
    Compose {
        /// A draft to resume; `None` starts an empty one.
        draft: Option<DraftId> = None,
    } => "compose",
    /// Send the composer's message.
    Send => "send",
    /// Open the picker for sending the composer's message later.
    ///
    /// No payload, the same way `AttachFile { path: None }` opens a chooser
    /// rather than naming a file: choosing a time is a widget interaction
    /// inside the composer, not something the keymap or the palette can
    /// resolve on the command's behalf.
    ScheduleSend => "schedule_send",
    /// Save the composer's message as a draft.
    SaveDraft => "save_draft",
    /// Throw away the composer's draft.
    DiscardDraft => "discard_draft",
    /// Settle a draft whose send could not be confirmed: it did arrive
    /// (ADR 0021 Decision 3, #674).
    ///
    /// `None` means the draft in view. A user who has checked with the
    /// recipient and learnt the message got there otherwise has only two
    /// exits -- discard, which throws the message away, or send again, which
    /// duplicates it -- and an `Unconfirmed` draft with no honest way out is
    /// a dead end.
    MarkSent {
        /// Which draft, or the one in view.
        draft: Option<DraftId> = None,
    } => "mark_sent",
    /// Put a send back on the queue after it stopped.
    ///
    /// The way out of `Failed` and `Unconfirmed` that is not "throw it away".
    /// Until now the only retry was to open the draft and press Send again,
    /// which works -- `queue_send` is what the composer calls, and it is what
    /// this calls -- but is not something the Outbox could offer, because a
    /// message on its way has no composer open.
    ///
    /// `None` means the draft in view.
    RetrySend {
        /// Which draft, or the one in view.
        draft: Option<DraftId> = None,
    } => "retry_send",
    /// Take a queued send back off the queue, leaving the draft editable.
    ///
    /// Refused once the submission has started: ADR 0021 keeps exactly-once
    /// by never cancelling something that may already be in flight, and
    /// `DraftRepository::cancel_send` reports which of those happened.
    ///
    /// `None` means the draft in view.
    CancelSend {
        /// Which draft, or the one in view.
        draft: Option<DraftId> = None,
    } => "cancel_send",
    /// Attach a file to the draft.
    AttachFile {
        /// The file; `None` opens the file chooser.
        path: Option<PathBuf> = None,
    } => "attach_file",
    /// Move the composition between the reading pane and its own window.
    ///
    /// A toggle, and one composition either way: detaching moves the draft
    /// rather than forking it, so there is never a second composer to keep in
    /// step. Purely a view concern -- nothing downstream of the frontend can
    /// tell which container the draft is being typed into.
    DetachComposer => "detach_composer",
    /// Raise the draft's Cc and Bcc rows, or put them away again.
    ///
    /// A toggle with an asymmetric half. Raising always works; putting away
    /// only works while both fields are empty, because a hidden row that still
    /// held addresses would keep those recipients on the draft and still send
    /// to them, under a sender who could no longer see them. When it will not
    /// put them away it moves the keyboard to `Cc` instead, so the refusal is
    /// visible rather than silent.
    CopyFields => "copy_fields",
    /// Put a picture in the body, at the caret.
    ///
    /// The third of the three outcomes FR-049 asks the composer to keep
    /// distinct — a link on text, an image *inside* the body, a file attached
    /// alongside — and the only one that had no command. Pasting and dropping
    /// reached it, which meant it was absent from the palette and the `?`
    /// sheet and unreachable by anyone who does neither.
    InsertImage => "insert_image",
    /// Hand the body to the person's own editor (`$EDITOR`) and take back
    /// what it saved (docs/archive/specs/005-tui-frontend FR-022). Where the body is not
    /// text an editor can open -- the desktop's rich editor -- the frontend
    /// says so.
    EditExternally => "edit_externally",
    /// Show the draft as it will arrive, beside or instead of the text being
    /// written (docs/archive/specs/005-tui-frontend FR-021). The desktop's composer shows
    /// formatting as it is written, so it has nothing to preview.
    TogglePreview => "toggle_preview",
    /// Make the selection bold, or un-bold it.
    Bold => "bold",
    /// Make the selection italic, or straighten it.
    Italic => "italic",
    /// Turn the current block into a bulleted list, or back.
    BulletList => "bullet_list",
    /// Turn the current block into a numbered list, or back.
    NumberedList => "numbered_list",
    /// Turn a link on the selection, asking for the address.
    InsertLink => "insert_link",
    /// Turn the current block into a quote, or back.
    QuoteBlock => "quote_block",

    // -- View and application --------------------------------------------
    /// Undo the last undoable action.
    Undo => "undo",
    /// Open the command palette.
    CommandPalette => "command_palette",
    /// Show the keyboard cheat sheet.
    CheatSheet => "cheat_sheet",
    /// Show the settings panel.
    Settings => "settings",
    /// Add another account to this installation.
    ///
    /// The entry point ADR 0012 Q1 asked for: a registered command rather
    /// than a button, so the palette and the cheat sheet carry it without
    /// either of them learning that accounts exist.
    AddAccount => "add_account",
    /// Open `config.toml` in the user's editor.
    EditConfig => "edit_config",
    /// Quit Postio.
    ///
    /// The desktop app always had its window's close button for this and
    /// never needed a command; a terminal has no close button, and a command
    /// that is not in the registry does not exist (Principle II), so it is
    /// one here and the desktop gains a key for it too.
    Quit => "quit",
    /// Load this message's remote images, this once.
    ///
    /// The reading pane's banner had this as a button and nothing else; a
    /// command that is not in the registry does not exist (Principle II), and
    /// a terminal has no banner to click.
    ShowImages => "show_images",
    /// Always load remote images from this message's sender.
    AlwaysShowImages => "always_show_images",
    /// Leave the mailing list this message came from, by its one-click
    /// `List-Unsubscribe` -- only ever on this deliberate act.
    Unsubscribe => "unsubscribe",
    /// Take up the offer the unsubscribe notice is making: archive or delete
    /// (`[focus] after_unsubscribe`) the mail the list already sent, as one
    /// undoable bulk command on exactly the messages the notice counted.
    ///
    /// The notice's own button, as a command (#1953): a notice's action is
    /// reached from the keyboard the way `i a` reaches the blocked-images
    /// notice's. The frontend holds the pending offer, so the id carries no
    /// payload; with no offer on screen it says so and does nothing.
    ClearListMail => "clear_list_mail",
    /// Go to the folders: the classic app puts the keyboard in its folder
    /// list, and Focus opens its folders popover.
    GoToFolders => "go_to_folders",
    /// Go to a mailbox by the role it wears, rather than by its name.
    ///
    /// One variant each rather than `GoTo(MailboxRole)`, because [`Command::id`]
    /// is total: a `GoTo(MailboxRole::Junk)` would be a value with no command
    /// id, and the type would then permit something the registry cannot
    /// answer. Four roles have a sequence; the rest are reached through the
    /// box, which is what `#` is for.
    GoToInbox => "go_to_inbox",
    /// Go to the drafts.
    GoToDrafts => "go_to_drafts",
    /// Go to the sent mail.
    GoToSent => "go_to_sent",
    /// Go to the flagged mail.
    GoToFlagged => "go_to_flagged",
    /// Go to the archive.
    GoToArchive => "go_to_archive",
    /// Go to the snoozed mail.
    GoToSnoozed => "go_to_snoozed",
    /// Go to the outbox: the sends under way.
    GoToOutbox => "go_to_outbox",
    /// Go to the junk.
    GoToJunk => "go_to_junk",
    /// Go to the trash.
    GoToTrash => "go_to_trash",
    /// Go to what was filtered out of the inbox.
    GoToFiltered => "go_to_filtered",
    /// Go to the digest rules.
    GoToDigestRules => "go_to_digest_rules",
    /// Run the first pinned `[saved_searches]` entry.
    ///
    /// One variant for each of the four, for the reason the destinations
    /// above are: [`Command::id`] is total, and `SavedSearch(9)` would be a
    /// value with no command id.
    SavedSearch1 => "saved_search_1",
    /// Run the second pinned `[saved_searches]` entry.
    SavedSearch2 => "saved_search_2",
    /// Run the third pinned `[saved_searches]` entry.
    SavedSearch3 => "saved_search_3",
    /// Run the fourth pinned `[saved_searches]` entry.
    SavedSearch4 => "saved_search_4",
    /// Show only the mail that asks something of you, or everything again.
    ToggleHasAction => "toggle_has_action",
    /// Enable or disable the focused account.
    ///
    /// No payload, like `Archive`'s selection and for the same reason:
    /// these are only ever offered while `Context::Accounts` is active, which
    /// means an account row has focus, which means the target is that row --
    /// exactly as `Archive`'s target is the current selection (ADR 0005 Q6c).
    ToggleAccountEnabled => "toggle_account_enabled",
    /// Remove the focused account.
    RemoveAccount => "remove_account",
    /// Update the focused account's stored credential.
    UpdateCredential => "update_credential",
    /// Rebuild the focused account's local search index.
    ///
    /// No payload, the same reason as the two above: the target is the row
    /// with focus. Local mail only -- nothing here reaches the network; see
    /// `postio_session::reindex_account`'s own doc for why (#981).
    RebuildAccountIndex => "rebuild_account_index",
    /// Make the focused account the default (#960).
    ///
    /// No payload, the same reason as the three above: the target is the row
    /// with focus. It settles exactly one question — which account a new
    /// message comes from when the message itself does not say — and in
    /// particular not a reply's from address, which
    /// [`postio_model::reply`] decides from the message being replied to.
    SetDefaultAccount => "set_default_account",
    /// Point one of an account's roles at one of its folders (ADR 0035).
    ///
    /// The one verb whose `None` does not always mean "ask": `account` and
    /// `role` follow the rule at the top of this enum -- a keystroke cannot
    /// supply them, so `None` asks -- but `path: None` is a value in its own
    /// right, **back to automatic**, because "stop choosing" is exactly what
    /// a person picking the first entry of the pane's dropdown means, and a
    /// second command for it would be a key in the reference for nothing.
    MapMailboxRole {
        /// Whose map; `None` means the focused account row.
        account: Option<AccountId> = None,
        /// Which role is being pointed somewhere; `None` asks.
        role: Option<MailboxRole> = None,
        /// The folder's server path, or `None` for automatic.
        path: Option<String> = None,
    } => "map_mailbox_role",
    /// Move to the next account scope: unified, then each account in turn.
    ///
    /// Cycling rather than `SetScope(id)` because a keystroke has no argument
    /// to carry one, and because the sidebar's own rows are the surface for
    /// naming a scope directly — the same split clicking a folder already
    /// has with walking to one.
    NextScope => "next_scope",
    /// Check for new mail now.
    Refresh => "refresh",

    // -- Reader --------------------------------------------------------
    /// Scroll the reading pane down by about a screenful.
    ScrollReaderDown => "scroll_reader_down",
    /// Scroll the reading pane up by about a screenful.
    ScrollReaderUp => "scroll_reader_up",

    // -- Pickers, digests and Filtered (specs/007-postio-focus) ------------
    /// Choose a picker's first option. One variant per option, like the
    /// saved searches.
    PickerChoose1 => "picker_choose_1",
    /// Choose a picker's second option.
    PickerChoose2 => "picker_choose_2",
    /// Choose a picker's third option.
    PickerChoose3 => "picker_choose_3",
    /// Choose a picker's fourth option.
    PickerChoose4 => "picker_choose_4",
    /// Type a date into a picker instead of choosing one.
    PickerTypeDate => "picker_type_date",
    /// Turn the option under a picker's cursor on or off.
    PickerToggle => "picker_toggle",
    /// Confirm a picker's choice.
    PickerConfirm => "picker_confirm",
    /// Go to the digest summary's next reference.
    NextReference => "next_reference",
    /// Go to the digest summary's previous reference.
    PrevReference => "prev_reference",
    /// Switch a digest between its summary and its messages.
    ToggleDigestSummary => "toggle_digest_summary",
    /// Put a filtered message back in the inbox, and never filter its sender.
    RestoreFiltered {
        /// What to restore.
        target: MessageTarget = MessageTarget::Selection,
        /// `true` restores; `false` is undo's way back.
        restored: bool = true,
    } => "restore_filtered",
    /// Show Filtered's first reason tab.
    FilteredTab1 => "filtered_tab_1",
    /// Show Filtered's second reason tab.
    FilteredTab2 => "filtered_tab_2",
    /// Show Filtered's third reason tab.
    FilteredTab3 => "filtered_tab_3",
    /// Show Filtered's fourth reason tab.
    FilteredTab4 => "filtered_tab_4",
    /// Show Filtered's fifth reason tab.
    FilteredTab5 => "filtered_tab_5",
    /// Show Filtered's sixth reason tab.
    FilteredTab6 => "filtered_tab_6",
    /// Show Filtered's seventh reason tab.
    FilteredTab7 => "filtered_tab_7",
    /// File away what is already in the inbox, by the rules that file
    /// arrivals away (spec 007 FR-118): the deliberate command that applies
    /// filtering to mail filed before it was turned on. The frontend shows
    /// what would move first (`Client::sweep_preview`).
    SweepInbox => "sweep_inbox",

    // -- Obsidian capture (specs/007-postio-focus milestone 3) -------------
    /// Open the capture sheet for a task from the message (spec 007 US15),
    /// or switch the open sheet to a task. Frontend-only: the sheet writes
    /// through `Client::capture_task`.
    CaptureTask => "capture_task",
    /// Open the capture sheet for a note from the message, or switch the
    /// open sheet to a note.
    CaptureNote => "capture_note",
    /// Choose another project for the capture.
    CaptureChangeProject => "capture_change_project",
    /// Use the message's subject as the capture's text.
    CaptureUseSubject => "capture_use_subject",
    /// Write the capture into the vault.
    CaptureWrite => "capture_write",
    /// "Digest mail like this" (spec 007 US14, T155): open the rule dialog
    /// pre-filled from `message`, its candidate queries checked by the
    /// user's local model (`Client::digest_like_this`). Present only when
    /// `[focus.model]` has `like_this` on (FR-171); a UI-only command, with
    /// no store write of its own.
    DigestLikeThis {
        /// The message to check other mail against, resolved by app state
        /// the way [`Command::OpenMessage`]'s is.
        message: Option<MessageId> = None,
    } => "digest_like_this",

    // -- Contacts (specs/005-contacts) -----------------------------------
    /// Open Contacts.
    OpenContacts => "open_contacts",
    /// Open the contact popover on the sender under the cursor, or on
    /// `address` when a name was clicked (specs/011-focus-contacts S6).
    ContactPopover {
        /// Whose; `None` means the sender of the row or message the keyboard
        /// is on.
        address: Option<String> = None,
    } => "contact_popover",
    /// Open the contact's card from the popover. Held until the card exists.
    ContactOpenCard => "contact_open_card",
    /// Save the person the new-sender popover is about, keeping what their
    /// signature said.
    ContactAdd => "contact_add",
    /// Never offer the new-sender popover for this sender again.
    ContactIgnore => "contact_ignore",
    /// Step the popover's arrival control to the segment before.
    ContactArrivalPrev => "contact_arrival_prev",
    /// Step the popover's arrival control to the segment after.
    ContactArrivalNext => "contact_arrival_next",
    /// Choose where one sender's mail arrives, or put the choice back
    /// (specs/011-focus-contacts S6). Applies at once and can be undone.
    ContactSetArrival {
        /// Whose: any of the person's addresses.
        address: String = String::new(),
        /// Where: `None` is by the default rules.
        arrival: Option<postio_model::ContactArrival> = None,
    } => "contact_set_arrival",
    /// Show the focused person's mail: a `with:` search over every address
    /// they own.
    ContactShowMail => "contact_show_mail",
    /// Open a draft to the focused person's preferred address.
    ContactCompose => "contact_compose",
    /// Move the keyboard to the Contacts filter.
    ContactsFilter => "contacts_filter",
    /// The kind tab before this one.
    ContactsTabPrevious => "contacts_tab_previous",
    /// The kind tab after this one.
    ContactsTabNext => "contacts_tab_next",
    /// Show everyone from mail, or only the people the user made or wrote to.
    ContactsToggleEveryone => "contacts_toggle_everyone",
    /// Merge the marked people, or a suggested pair, into one person
    /// (specs/005-contacts FR-012; the word on screen is Merge, the id keeps
    /// the name the contracts gave it), or take a merge back.
    ContactJoin {
        /// What is asked: a keystroke asks which name the merged person keeps.
        action: ContactJoinAction = ContactJoinAction::Ask,
    } => "contact_join",
    /// Give a person an address, or put an address back where it was.
    ContactAddAddress {
        /// What is asked: a keystroke asks for the address.
        action: ContactAddressAction = ContactAddressAction::Ask,
    } => "contact_add_address",
    /// Make an address a person of its own (FR-014).
    ContactDetachAddress {
        /// The address; `None` means the one the keyboard is on.
        address: Option<AddressId> = None,
    } => "contact_detach_address",
    /// Offer an address first (FR-016).
    ContactSetPreferred {
        /// Whose; `None` means the focused person.
        person: Option<ContactId> = None,
        /// Which; `None` means the focused address.
        address: Option<AddressId> = None,
    } => "contact_set_preferred",
    /// Make a person (FR-020), or ask for one.
    ContactNew {
        /// What is asked: a keystroke asks for the name and addresses.
        action: ContactNewAction = ContactNewAction::Ask,
    } => "contact_new",
    /// Edit a person (FR-021), or put an edit back.
    ContactEdit {
        /// What is asked: a keystroke opens the editor on the focused person.
        action: ContactEditAction = ContactEditAction::Ask,
    } => "contact_edit",
    /// Save the Edit form in front of the person (specs/011-focus-contacts
    /// S8). The form is the controller's, so the command carries nothing.
    ContactSave => "contact_save",
    /// Delete a person (FR-023), or a group (FR-040). Both `None` means
    /// whichever row is focused.
    ContactDelete {
        /// Who.
        person: Option<ContactId> = None,
        /// Or which group.
        group: Option<ContactGroupId> = None,
    } => "contact_delete",
    /// Bring a deleted person back (FR-023a).
    ContactRestore {
        /// Who; `None` means the focused one.
        person: Option<ContactId> = None,
        /// The state they come back in; live when not given. Undo knows what
        /// they were and a key does not.
        state: Option<ContactState> = None,
    } => "contact_restore",
    /// Show the deleted people, or the default view again.
    ContactsToggleDeleted => "contacts_toggle_deleted",
    /// Show who might be the same person, or the people again.
    ContactsSuggestions => "contacts_suggestions",
    /// Say the focused suggestion is two people, for good.
    SuggestionDismiss {
        /// The two people; `None` means the focused suggestion.
        pair: Option<(ContactId, ContactId)> = None,
    } => "suggestion_dismiss",
    /// Make a group (FR-040), ask for its name, or put a deleted one back.
    ContactGroupNew {
        /// What is asked: a keystroke asks for the name.
        action: ContactGroupNewAction = ContactGroupNewAction::Ask,
    } => "contact_group_new",
    /// Rename a group.
    ContactGroupRename {
        /// Which; `None` means the focused group.
        group: Option<ContactGroupId> = None,
        /// Its new name; `None` asks.
        name: Option<String> = None,
    } => "contact_group_rename",
    /// Add people to a group.
    ContactGroupAdd {
        /// Which group; `None` asks which.
        group: Option<ContactGroupId> = None,
        /// Who; empty means the marked people, or the one under the cursor.
        people: Vec<ContactId> = Vec::new(),
    } => "contact_group_add",
    /// Take people out of a group.
    ContactGroupRemove {
        /// Which group; `None` means the group on screen.
        group: Option<ContactGroupId> = None,
        /// Who; empty means the marked people, or the one under the cursor.
        people: Vec<ContactId> = Vec::new(),
    } => "contact_group_remove",
    /// Import people from a vCard file (FR-050); the surface asks which.
    ContactsImport => "contacts_import",
    /// Export people to a vCard file (FR-050): the marked people, or what the
    /// list shows; the surface asks where.
    ContactsExport => "contacts_export",
    /// Sync with the Mac's Contacts, as a dry run: turn it on, and see what
    /// would happen before anything does (specs/011-focus-contacts S12).
    ContactsSync => "contacts_sync",
    ;
    /// Mark one message read because the cursor rested on it long enough to
    /// have been read — not because anyone asked.
    ///
    /// # Why this is a variant rather than a command of its own
    ///
    /// It is the *same verb*: [`id`](Command::id) answers
    /// [`CommandId::ToggleRead`], so it routes to the same handler, appears in
    /// the registry once, and does not invent a second spelling of "mark
    /// read" for the palette and the cheat sheet to disagree about. What
    /// differs is only who asked — and that changes exactly one thing, which
    /// is that it is not recorded on the undo stack. A registry command would
    /// also have had to carry a key binding it would never be reached by;
    /// `tests/command_registry.rs` requires one of every entry.
    ///
    /// # Why it is not undoable
    ///
    /// Undo takes back what *you* did. Reading a mailbox produces one of
    /// these per message rested on, so recording them would bury the archive
    /// you actually want back under a drift of marks you never asked for, and
    /// undo would stop meaning anything predictable. The reversal is `r`
    /// (mark read or unread), which is bound, in the palette and on the cheat
    /// sheet. See #71.
    MarkReadOnDwell {
        /// The message the cursor rested on.
        message: MessageId,
    } => ToggleRead,
    /// Archive a whole digest, or bring it back (spec 007 FR-125): `⇧A` in
    /// the digest window. Every message it holds is archived, and its row
    /// leaves the inbox.
    ///
    /// The same verb as `A` everywhere else -- "archive everything this row
    /// stands for" -- so [`Command::id`] answers
    /// [`CommandId::ArchiveThread`], as [`Command::MarkReadOnDwell`] answers
    /// `ToggleRead`'s: one row in the registry, one key.
    ArchiveDigest {
        /// Which delivery.
        delivery: postio_model::DeliveryId,
        /// `true` archives it; `false` is undo's way back.
        archived: bool,
    } => ArchiveThread,
    /// Undo's way back from a sweep: the messages return to their inboxes,
    /// and the decisions that filed them away go with the sweep that made
    /// them.
    ///
    /// The same verb, the other direction, as [`Command::MarkReadOnDwell`]
    /// is [`CommandId::ToggleRead`]'s other provenance: [`Command::id`]
    /// answers [`CommandId::SweepInbox`], so it routes to the same handler
    /// and needs no key or registry row of its own.
    UnsweepInbox {
        /// What the sweep moved.
        target: MessageTarget,
    } => SweepInbox,
}

impl fmt::Display for CommandId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The error from parsing a string that names no known command.
///
/// A `[keys]` entry for a command this build does not know is a warning, never
/// a hard failure — the config layer keeps the line so a downgrade and upgrade
/// round trip does not eat it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownCommand(String);

impl UnknownCommand {
    /// Name `text` as something that is not a command in this build.
    pub fn new(text: impl Into<String>) -> Self {
        UnknownCommand(text.into())
    }

    /// The text that named no command.
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

/// What a message action applies to.
///
/// A keystroke knows it means "archive", not *which* rows are selected, so the
/// common case is [`MessageTarget::Selection`] and app state resolves it. The
/// palette, the context menu and undo replay name their messages explicitly.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageTarget {
    /// Whatever the user has selected right now; resolved by app state.
    #[default]
    Selection,
    /// These messages specifically.
    Messages(Vec<MessageId>),
    /// Every message in this thread.
    Thread(ThreadId),
    /// Every message in each of these threads.
    ///
    /// The unified list's group expansion (#184, ADR 0005 Q2): a deduped
    /// row stands for one conversation whose copies live in several
    /// accounts' threads, and an action on it must hit every copy in one
    /// gesture — one undo, one unit per account.
    Threads(Vec<ThreadId>),
    /// Every message a run of queue rows named.
    ///
    /// Only undo builds one of these, and only to take back a whole-mailbox
    /// action. A bulk archive writes one queue row per message in a single
    /// statement, so the run those rows occupy names all of them in two
    /// integers — which is what lets one `u` reverse 81,717 messages without
    /// anything holding 81,717 ids. See [`OperationRange`].
    Batch {
        /// The queue rows the bulk action wrote.
        range: OperationRange,
        /// The account those messages belong to.
        ///
        /// Carried rather than looked up: a batch names no row to read an
        /// account off, and the one mailbox read it used to take was only
        /// ever answering this.
        account: AccountId,
        /// The mailbox those messages are in now, when they are all in one.
        ///
        /// Carried rather than derived because the handler needs it only to
        /// say which list has to reload, and asking the store "where are these
        /// 81,717 messages" to answer that would be the read this whole shape
        /// exists to avoid.
        ///
        /// `None` when the action being undone never moved them together —
        /// a bulk flag over a smart folder leaves every message where it was,
        /// across as many folders as it spanned (#52).
        from: Option<MailboxId>,
    },
}

/// What a [`Command::ContactJoin`] asks for.
///
/// A keystroke is [`Ask`](Self::Ask): Contacts asks which name the merged
/// person keeps, then invokes [`Join`](Self::Join). Undo invokes
/// [`Unjoin`](Self::Unjoin) with what the merge handed back -- one verb, three
/// shapes, the way `Move` with no destination asks and with one moves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactJoinAction {
    /// Ask the user which name, and which organisation if theirs disagree.
    Ask,
    /// Merge `others` into `into`, under `name`.
    Join {
        /// The survivor.
        into: ContactId,
        /// The people merged into it.
        others: Vec<ContactId>,
        /// The merged person's name, which every merge ends with.
        name: String,
        /// The organisation chosen when theirs disagreed.
        organization: Option<String>,
    },
    /// Take a merge back.
    Unjoin(JoinReceipt),
}

/// What a [`Command::ContactNew`] asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactNewAction {
    /// Ask the user: the editor, empty.
    Ask,
    /// Make this person.
    Create {
        /// Their name, if the user gave one.
        name: Option<String>,
        /// Their addresses; at least one.
        addresses: Vec<EmailAddress>,
    },
}

/// What a [`Command::ContactEdit`] asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactEditAction {
    /// Ask the user: the editor, on the focused person.
    Ask,
    /// Set these fields.
    Edit {
        /// Whose.
        person: ContactId,
        /// What they become.
        edit: PersonEdit,
    },
    /// Put fields back exactly -- undo's half.
    Put {
        /// Whose.
        person: ContactId,
        /// What they were.
        fields: PersonFields,
    },
    /// Turn one of a card's switches on or off for the person at `address`;
    /// undo puts the switch back.
    Switch {
        /// Any of the person's addresses.
        address: String,
        /// Which switch.
        switch: postio_model::CardSwitch,
        /// What it becomes.
        on: bool,
    },
    /// "Add contact" in the new-sender popover: save the person who owns
    /// `address`, keeping what their signature said (specs/011-focus-contacts
    /// S9).
    AddSender {
        /// Any of the person's addresses.
        address: String,
        /// What the signature said.
        details: postio_model::SignatureDetails,
    },
    /// Put a person back as they were before [`ContactEditAction::AddSender`]
    /// -- undo's half.
    RestoreSender {
        /// Whose.
        person: ContactId,
        /// What they were.
        prior: postio_model::SenderPrior,
    },
    /// Keep (`Some(true)`), refuse (`Some(false)`) or forget (`None`) a phone
    /// or website of the person at `address`; undo puts it back.
    Detail {
        /// Any of the person's addresses.
        address: String,
        /// Phone or website.
        kind: postio_model::DetailKind,
        /// As written.
        value: String,
        /// What it becomes.
        keep: Option<bool>,
    },
    /// "Ignore sender": never offer the new-sender popover for them again,
    /// or put the choice back.
    Ignore {
        /// Any of the person's addresses.
        address: String,
        /// What it becomes.
        ignored: bool,
    },
    /// Save the Edit form (specs/011-focus-contacts S8): make a person, or
    /// make `person` what the form says, names, kind, addresses, groups and
    /// all, as one undoable step.
    Save {
        /// Whose; `None` makes a new person.
        person: Option<ContactId>,
        /// What the form holds.
        form: Box<postio_model::ContactForm>,
    },
}

/// What a [`Command::ContactGroupNew`] asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactGroupNewAction {
    /// Ask for the name.
    Ask,
    /// Make this group of these people.
    Create {
        /// Its name.
        name: String,
        /// Its first members.
        members: Vec<ContactId>,
    },
    /// Put a deleted group back -- undo's half.
    Restore {
        /// The group as it was.
        group: ContactGroup,
        /// Every member it had.
        members: Vec<ContactId>,
    },
}

/// What a [`Command::ContactAddAddress`] asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactAddressAction {
    /// Ask the user for the address.
    Ask,
    /// Give `person` the typed `address`; refused while a live person owns it.
    Add {
        /// Whose.
        person: ContactId,
        /// The address as typed.
        address: EmailAddress,
    },
    /// Give `address` to `to`, whoever has it -- the "move it" answer, and
    /// the way undo puts an address back. `to: None` lets go of it.
    Put {
        /// The address.
        address: AddressId,
        /// Who gets it, or nobody.
        to: Option<ContactId>,
        /// Undo's half: the state `to` was in before an earlier move left
        /// them with no address, to bring them back in.
        revive: Option<ContactState>,
    },
}

/// A digest rule as it stood, kept by the undo entry of a verb that
/// changed it, so taking the verb back puts the rule back exactly (spec 007
/// FR-125): `[[focus.digests]]`'s own TOML for it, and where it stood in the
/// file, whose order is the order rules match in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptRule {
    /// Its place among the rules.
    pub position: u32,
    /// The rule, as `config.toml` spells it.
    pub toml: String,
}

impl Command {
    /// What this invocation is aimed at, when it acts on messages at all.
    ///
    /// The read half of [`Command::with_target`]. A caller that wants to
    /// *re*-aim a verb — the list does, when the row under the cursor stands
    /// for a conversation rather than a message (ADR 0015 Q3) — has to be
    /// able to tell a verb already aimed somewhere specific from one still
    /// pointing at the selection.
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

    /// Point this invocation at `target`, if it acts on messages at all.
    ///
    /// The registry's default for every message action is
    /// [`MessageTarget::Selection`], which is right for a keystroke: `a`
    /// means "archive what I have chosen". A mouse can be more specific —
    /// a hover action on a row, or a drag that started on one — and saying
    /// so has to be possible without rebuilding the command by hand, or the
    /// two paths drift on which variant carries a target.
    ///
    /// Commands that act on something other than a set of messages are
    /// returned unchanged: pointing `Send` at a selection is not a narrower
    /// request, it is a meaningless one.
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

    /// Whether this command destroys something the user would have to rebuild
    /// by hand. Destructive commands always carry a
    /// [`Recovery`](crate::Recovery) — see [`crate::registry`].
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
        // What a hover action or a drag needs: the same verb, aimed at the
        // row under the pointer rather than at whatever is selected.
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
        // Aiming `Send` at a selection is not a narrower request; it is one
        // that means nothing, and quietly accepting it would invite a caller
        // to believe it did something.
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
        // `target` and `with_target` list the same verbs by hand; a verb
        // added to one and not the other would be readable but not aimable
        // (or the reverse), so every default invocation goes through both.
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
