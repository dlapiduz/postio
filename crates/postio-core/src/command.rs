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

use postio_model::{AccountId, DraftId, LabelId, MailboxId, MessageId, OperationRange, ThreadId};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

macro_rules! command_ids {
    ($( $(#[$doc:meta])* $variant:ident => $id:literal ),+ $(,)?) => {
        /// The stable identifier of a command.
        ///
        /// Serializes as its string id, so a `[keys]` entry, a palette row and a
        /// log line all spell a command the same way.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum CommandId {
            $( $(#[$doc])* $variant, )+
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
    };
}

command_ids! {
    /// Move the selection to the next message.
    NextMessage => "next_message",
    /// Move the selection to the previous message.
    PrevMessage => "prev_message",
    /// Jump to the first message in the list.
    FirstMessage => "first_message",
    /// Jump to the last message in the list.
    LastMessage => "last_message",
    /// Open the focused message in the reading pane.
    OpenMessage => "open_message",
    /// Add the focused message to the selection, or take it out again.
    ToggleSelection => "toggle_selection",
    /// Extend the selection to the next message down.
    ExtendSelectionDown => "extend_selection_down",
    /// Extend the selection to the previous message up.
    ExtendSelectionUp => "extend_selection_up",
    /// Select every message the list is showing.
    SelectAll => "select_all",
    /// Step back to the previous view without leaving the keyboard.
    PrevView => "prev_view",
    /// Leave the current overlay, search or composer.
    Back => "back",
    /// Switch a search's results between ranked and date order.
    ToggleResultOrder => "toggle_result_order",
    /// Move to the next message inside the open conversation.
    NextInConversation => "next_in_conversation",
    /// Move to the previous message inside the open conversation.
    PrevInConversation => "prev_in_conversation",
    /// Fold or unfold the focused message of the open conversation.
    ToggleFold => "toggle_fold",
    /// Draw the message on screen as its sender wrote it, not reduced.
    ViewOriginal => "view_original",
    /// Open every collapsed message in the conversation.
    ExpandAll => "expand_all",
    /// Reply to the sender.
    Reply => "reply",
    /// Reply to everyone on the message.
    ReplyAll => "reply_all",
    /// Forward the message.
    Forward => "forward",
    /// Archive the selection.
    Archive => "archive",
    /// Archive every message in the thread.
    ArchiveThread => "archive_thread",
    /// Move the selection to the trash.
    Delete => "delete",
    /// Move the selection to another mailbox.
    Move => "move",
    /// Toggle the flagged state of the selection.
    Flag => "flag",
    /// Toggle the unread state of the selection.
    MarkUnread => "mark_unread",
    /// Hide the selection from every ordinary list for a while.
    Snooze => "snooze",
    /// Cancel a snooze immediately.
    Unsnooze => "unsnooze",
    /// Attach a label to the selection.
    AddLabel => "add_label",
    /// Focus the search field.
    Search => "search",
    /// Save the current search as a pinned folder in the sidebar.
    SaveSearch => "save_search",
    /// Start a new message.
    Compose => "compose",
    /// Send what is in the composer.
    Send => "send",
    /// Open the picker for sending later instead of now.
    ScheduleSend => "schedule_send",
    /// Save the composer's contents as a draft.
    SaveDraft => "save_draft",
    /// Throw away the draft in the composer.
    DiscardDraft => "discard_draft",
    /// Settle an unconfirmed send by hand: it did arrive.
    MarkSent => "mark_sent",
    /// Attach a file to the draft.
    AttachFile => "attach_file",
    /// Move the composition between the reading pane and a window of its own.
    DetachComposer => "detach_composer",
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
    /// Undo the last undoable action.
    Undo => "undo",
    /// Open the command palette.
    CommandPalette => "command_palette",
    /// Show the keyboard cheat sheet.
    CheatSheet => "cheat_sheet",
    /// Show the settings panel.
    Settings => "settings",
    /// Add another account to this installation.
    AddAccount => "add_account",
    /// Open `config.toml` in the user's editor.
    EditConfig => "edit_config",
    /// Show or hide the sidebar.
    ToggleSidebar => "toggle_sidebar",
    /// Put the keyboard in the folder list.
    FocusSidebar => "focus_sidebar",
    /// Move the keyboard to the next pane: sidebar, list, reader, round.
    CyclePane => "cycle_pane",
    /// Move the keyboard to the previous pane.
    CyclePaneBack => "cycle_pane_back",
    /// Move to the next folder in the sidebar.
    NextFolder => "next_folder",
    /// Move to the previous folder in the sidebar.
    PrevFolder => "prev_folder",
    /// Expand or collapse the focused folder's children.
    ToggleFolder => "toggle_folder",
    /// Rename the focused saved search.
    RenameSavedSearch => "rename_saved_search",
    /// Move the focused saved search up one place.
    MoveSavedSearchUp => "move_saved_search_up",
    /// Move the focused saved search down one place.
    MoveSavedSearchDown => "move_saved_search_down",
    /// Delete the focused saved search.
    DeleteSavedSearch => "delete_saved_search",
    /// Enable or disable the focused account.
    ToggleAccountEnabled => "toggle_account_enabled",
    /// Remove the focused account.
    RemoveAccount => "remove_account",
    /// Update the focused account's stored credential.
    UpdateCredential => "update_credential",
    /// Move to the next account scope: unified, then each account in turn.
    NextScope => "next_scope",
    /// Ask the sync engine to check for new mail now.
    Refresh => "refresh",
    /// Show the focused message's MIME structure.
    OpenParts => "open_parts",
    /// Move the parts panel's cursor to the next part.
    NextPart => "next_part",
    /// Move the parts panel's cursor to the previous part.
    PrevPart => "prev_part",
    /// Open the part under the parts panel's cursor.
    OpenPart => "open_part",
    /// Save the part under the parts panel's cursor.
    SavePart => "save_part",
    /// Save every part the message holds.
    SaveAllParts => "save_all_parts",
    /// Hand the part under the parts panel's cursor to the desktop.
    OpenPartExternally => "open_part_externally",
    /// Render a held-back part once, loading what it references.
    RenderPartOnce => "render_part_once",
    /// Scroll the reading pane down by about a screenful, without moving
    /// the keyboard off the message list.
    ScrollReaderDown => "scroll_reader_down",
    /// Scroll the reading pane up by about a screenful, without moving the
    /// keyboard off the message list.
    ScrollReaderUp => "scroll_reader_up",
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

/// One invocation of a command.
///
/// Payload fields that a keystroke cannot supply are `Option`al: `None` means
/// "ask the user" — [`Command::Move`] with no mailbox opens the mailbox picker,
/// [`Command::Search`] with no query focuses the search field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    // -- Navigation ------------------------------------------------------
    /// Select the next message.
    NextMessage,
    /// Select the previous message.
    PrevMessage,
    /// Select the first message.
    FirstMessage,
    /// Select the last message.
    LastMessage,
    /// Open a message, or the focused one when `message` is `None`.
    OpenMessage {
        /// The message to open; `None` means the focused row.
        message: Option<MessageId>,
    },
    /// Add a message to the selection, or take it out again.
    ToggleSelection {
        /// The message to toggle; `None` means the focused row.
        message: Option<MessageId>,
    },
    /// Extend the selection to the next message down.
    ExtendSelectionDown,
    /// Extend the selection to the previous message up.
    ExtendSelectionUp,
    /// Select every message the list is showing.
    ///
    /// Never resolved into a list of ids on the way through — see
    /// [`crate::state::Selection`] for why that matters.
    SelectAll,
    /// Return to the previous view.
    PrevView,
    /// Leave the current overlay, search or composer.
    Back,
    /// Switch a search's results between ranked and date order (#499).
    ToggleResultOrder,
    /// Walk down the open conversation's stack (#1007).
    ///
    /// No payload: `j`/`k` move between *threads* in the list, and these
    /// move between messages inside the one that is open. Two axes, two
    /// pairs of keys, and which one you are on is a fact about where the
    /// keyboard is rather than about what you pressed.
    NextInConversation,
    /// Walk up the open conversation's stack (#1007).
    PrevInConversation,
    /// Fold or unfold the conversation's focused message (#1007).
    ///
    /// The only way to *collapse* the focused message: landing on one
    /// expands it, so a collapsed-and-focused message is a state only this
    /// reaches.
    ToggleFold,
    /// Leave reader view for the sender's own markup (#1009).
    ///
    /// No payload: it always means the message on screen. Reader view is a
    /// per-message state, so there is nothing else it could mean.
    ViewOriginal,
    /// Expand every collapsed message in the open conversation (#1004).
    ///
    /// No payload: it means the conversation on screen, which is the only
    /// one there is.
    ExpandAll,

    // -- Message actions -------------------------------------------------
    /// Reply to the sender.
    Reply {
        /// The message to reply to; `None` means the focused message.
        message: Option<MessageId>,
    },
    /// Reply to everyone.
    ReplyAll {
        /// The message to reply to; `None` means the focused message.
        message: Option<MessageId>,
    },
    /// Forward a message.
    Forward {
        /// The message to forward; `None` means the focused message.
        message: Option<MessageId>,
    },
    /// Archive messages.
    Archive {
        /// What to archive.
        target: MessageTarget,
    },
    /// Archive a whole thread.
    ArchiveThread {
        /// The thread to archive; `None` means the focused message's thread.
        thread: Option<ThreadId>,
    },
    /// Move messages to the trash.
    Delete {
        /// What to delete.
        target: MessageTarget,
    },
    /// Move messages to another mailbox.
    Move {
        /// What to move.
        target: MessageTarget,
        /// The destination; `None` opens the mailbox picker.
        to: Option<MailboxId>,
    },
    /// Set or toggle the flagged state.
    Flag {
        /// What to flag.
        target: MessageTarget,
        /// The state to set; `None` toggles.
        flagged: Option<bool>,
    },
    /// Set or toggle the unread state.
    MarkUnread {
        /// What to mark.
        target: MessageTarget,
        /// The state to set; `None` toggles.
        unread: Option<bool>,
    },
    /// Hide the selection from every ordinary list for a while.
    ///
    /// No duration here: unlike [`Command::Move`]'s destination, "for how
    /// long" is a UI decision the handler makes, not one this registry-level
    /// shape carries — the same reason [`Command::ScheduleSend`] opens a
    /// picker rather than embedding a time.
    Snooze {
        /// What to snooze.
        target: MessageTarget,
    },
    /// Cancel a snooze immediately.
    Unsnooze {
        /// What to unsnooze.
        target: MessageTarget,
    },
    /// Mark one message read because the cursor rested on it long enough to
    /// have been read — not because anyone asked.
    ///
    /// # Why this is a variant rather than a command of its own
    ///
    /// It is the *same verb*: [`id`](Command::id) answers
    /// [`CommandId::MarkUnread`], so it routes to the same handler, appears in
    /// the registry once, and does not invent a second spelling of "mark
    /// read" for the palette and the cheat sheet to disagree about. What
    /// differs is only who asked — and that changes exactly one thing, which
    /// is that it is not recorded on the undo stack. A registry command would
    /// also have had to carry a key binding it would never be reached by;
    /// `tests/command_registry.rs` requires one of every entry.
    ///
    /// # Why it is not undoable
    ///
    /// `u` takes back what *you* did. Reading a mailbox produces one of these
    /// per message rested on, so recording them would bury the archive you
    /// actually want back under a drift of marks you never asked for, and
    /// `u` would stop meaning anything predictable. The reversal is `U`
    /// (mark unread), which is bound, in the palette and on the cheat sheet.
    /// See #71.
    MarkReadOnDwell {
        /// The message the cursor rested on.
        message: MessageId,
    },
    /// Attach a label, or take one off.
    AddLabel {
        /// What to label.
        target: MessageTarget,
        /// The label; `None` opens the label picker.
        label: Option<LabelId>,
        /// On, off, or `None` to toggle.
        ///
        /// [`Command::Flag`]'s shape, and for its reason: `u` takes an action
        /// back by *dispatching its inverse*, so removing a label has to be
        /// something a `Command` can say. One registered verb that can do
        /// both beats a second entry in the registry that has no binding, no
        /// menu item and no way for a person to reach it -- which is what
        /// `AddLabel` itself was before #766 removed it (#780).
        on: Option<bool>,
    },

    // -- Search ----------------------------------------------------------
    /// Search, or focus the search field when `query` is `None`.
    Search {
        /// The query to run; `None` focuses the empty search field.
        query: Option<String>,
    },
    /// Save the query the search box currently holds as a pinned folder.
    ///
    /// No payload: the box being open is what says which query, the same
    /// way `EditConfig` needs no path because there is only one file.
    SaveSearch,

    // -- Compose ---------------------------------------------------------
    /// Start a new message, optionally from an existing draft.
    Compose {
        /// A draft to resume; `None` starts an empty one.
        draft: Option<DraftId>,
    },
    /// Send the composer's message.
    Send,
    /// Open the picker for sending the composer's message later.
    ///
    /// No payload, the same way `AttachFile { path: None }` opens a chooser
    /// rather than naming a file: choosing a time is a widget interaction
    /// inside the composer, not something the keymap or the palette can
    /// resolve on the command's behalf.
    ScheduleSend,
    /// Save the composer's message as a draft.
    SaveDraft,
    /// Throw away the composer's draft.
    DiscardDraft,
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
        draft: Option<DraftId>,
    },
    /// Attach a file to the draft.
    AttachFile {
        /// The file; `None` opens the file chooser.
        path: Option<PathBuf>,
    },
    /// Move the composition between the reading pane and its own window.
    ///
    /// A toggle, and one composition either way: detaching moves the draft
    /// rather than forking it, so there is never a second composer to keep in
    /// step. Purely a view concern -- nothing downstream of the frontend can
    /// tell which container the draft is being typed into.
    DetachComposer,
    /// Make the selection bold, or un-bold it.
    Bold,
    /// Make the selection italic, or straighten it.
    Italic,
    /// Turn the current block into a bulleted list, or back.
    BulletList,
    /// Turn the current block into a numbered list, or back.
    NumberedList,
    /// Turn a link on the selection, asking for the address.
    InsertLink,
    /// Turn the current block into a quote, or back.
    QuoteBlock,

    // -- View and application --------------------------------------------
    /// Undo the last undoable action.
    Undo,
    /// Open the command palette.
    CommandPalette,
    /// Show the keyboard cheat sheet.
    CheatSheet,
    /// Show the settings panel.
    Settings,
    /// Add another account to this installation.
    ///
    /// The entry point ADR 0012 Q1 asked for: a registered command rather
    /// than a button, so the palette and the cheat sheet carry it without
    /// either of them learning that accounts exist.
    AddAccount,
    /// Open `config.toml` in the user's editor.
    EditConfig,
    /// Show or hide the sidebar.
    ToggleSidebar,
    /// Put the keyboard in the folder list.
    FocusSidebar,
    /// Move the keyboard to the next pane: sidebar, list, reader, round.
    ///
    /// The *top-level* meaning of bare Tab, for when a pane itself has the
    /// keyboard. Panes that own Tab for their own purpose -- a refine chip,
    /// recipient completion, the finder -- keep first claim on it (#494).
    CyclePane,
    /// Move the keyboard to the previous pane.
    CyclePaneBack,
    /// Move to the next folder.
    NextFolder,
    /// Move to the previous folder.
    PrevFolder,
    /// Expand or collapse the focused folder's children.
    ToggleFolder,
    /// Rename the focused saved search.
    RenameSavedSearch,
    /// Move the focused saved search up one place.
    MoveSavedSearchUp,
    /// Move the focused saved search down one place.
    MoveSavedSearchDown,
    /// Delete the focused saved search.
    DeleteSavedSearch,
    /// Enable or disable the focused account.
    ///
    /// No payload, like the saved-search verbs above and for the same reason:
    /// these are only ever offered while `Context::Accounts` is active, which
    /// means an account row has focus, which means the target is that row --
    /// exactly as `Archive`'s target is the current selection (ADR 0005 Q6c).
    ToggleAccountEnabled,
    /// Remove the focused account.
    RemoveAccount,
    /// Update the focused account's stored credential.
    UpdateCredential,
    /// Move to the next account scope: unified, then each account in turn.
    ///
    /// Cycling rather than `SetScope(id)` because a keystroke has no argument
    /// to carry one, and because the sidebar's own rows are the surface for
    /// naming a scope directly — the same split `NextFolder` and clicking a
    /// folder already have.
    NextScope,
    /// Check for new mail now.
    Refresh,

    // -- Parts panel -------------------------------------------------------
    /// Show the focused message's MIME structure.
    OpenParts,
    /// Move the parts panel's cursor to the next part.
    NextPart,
    /// Move the parts panel's cursor to the previous part.
    PrevPart,
    /// Open the part under the parts panel's cursor.
    OpenPart,
    /// Save the part under the parts panel's cursor.
    SavePart,
    /// Save every part the message holds.
    SaveAllParts,
    /// Hand the part under the parts panel's cursor to the desktop.
    OpenPartExternally,
    /// Render a held-back part once, loading what it references.
    RenderPartOnce,

    // -- Reader --------------------------------------------------------
    /// Scroll the reading pane down by about a screenful.
    ScrollReaderDown,
    /// Scroll the reading pane up by about a screenful.
    ScrollReaderUp,
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
            | Command::MarkUnread { target, .. }
            | Command::Snooze { target }
            | Command::Unsnooze { target }
            | Command::AddLabel { target, .. } => Some(target),
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
            Command::MarkUnread { unread, .. } => Command::MarkUnread { target, unread },
            Command::Snooze { .. } => Command::Snooze { target },
            Command::Unsnooze { .. } => Command::Unsnooze { target },
            Command::AddLabel { label, on, .. } => Command::AddLabel { target, label, on },
            other => other,
        }
    }

    /// The registry id this invocation belongs to.
    pub fn id(&self) -> CommandId {
        match self {
            Command::NextMessage => CommandId::NextMessage,
            Command::PrevMessage => CommandId::PrevMessage,
            Command::FirstMessage => CommandId::FirstMessage,
            Command::LastMessage => CommandId::LastMessage,
            Command::OpenMessage { .. } => CommandId::OpenMessage,
            Command::ToggleSelection { .. } => CommandId::ToggleSelection,
            Command::ExtendSelectionDown => CommandId::ExtendSelectionDown,
            Command::ExtendSelectionUp => CommandId::ExtendSelectionUp,
            Command::SelectAll => CommandId::SelectAll,
            Command::PrevView => CommandId::PrevView,
            Command::Back => CommandId::Back,
            Command::ToggleResultOrder => CommandId::ToggleResultOrder,
            Command::NextInConversation => CommandId::NextInConversation,
            Command::PrevInConversation => CommandId::PrevInConversation,
            Command::ToggleFold => CommandId::ToggleFold,
            Command::ViewOriginal => CommandId::ViewOriginal,
            Command::ExpandAll => CommandId::ExpandAll,
            Command::Reply { .. } => CommandId::Reply,
            Command::ReplyAll { .. } => CommandId::ReplyAll,
            Command::Forward { .. } => CommandId::Forward,
            Command::Archive { .. } => CommandId::Archive,
            Command::ArchiveThread { .. } => CommandId::ArchiveThread,
            Command::Delete { .. } => CommandId::Delete,
            Command::Move { .. } => CommandId::Move,
            Command::Flag { .. } => CommandId::Flag,
            // The same verb, invoked by the app rather than by the user — see
            // `MarkReadOnDwell`'s own docs.
            Command::MarkUnread { .. } | Command::MarkReadOnDwell { .. } => CommandId::MarkUnread,
            Command::Snooze { .. } => CommandId::Snooze,
            Command::Unsnooze { .. } => CommandId::Unsnooze,
            Command::AddLabel { .. } => CommandId::AddLabel,
            Command::Search { .. } => CommandId::Search,
            Command::SaveSearch => CommandId::SaveSearch,
            Command::Compose { .. } => CommandId::Compose,
            Command::Send => CommandId::Send,
            Command::ScheduleSend => CommandId::ScheduleSend,
            Command::SaveDraft => CommandId::SaveDraft,
            Command::DiscardDraft => CommandId::DiscardDraft,
            Command::MarkSent { .. } => CommandId::MarkSent,
            Command::AttachFile { .. } => CommandId::AttachFile,
            Command::DetachComposer => CommandId::DetachComposer,
            Command::Bold => CommandId::Bold,
            Command::Italic => CommandId::Italic,
            Command::BulletList => CommandId::BulletList,
            Command::NumberedList => CommandId::NumberedList,
            Command::InsertLink => CommandId::InsertLink,
            Command::QuoteBlock => CommandId::QuoteBlock,
            Command::Undo => CommandId::Undo,
            Command::CommandPalette => CommandId::CommandPalette,
            Command::CheatSheet => CommandId::CheatSheet,
            Command::Settings => CommandId::Settings,
            Command::AddAccount => CommandId::AddAccount,
            Command::EditConfig => CommandId::EditConfig,
            Command::ToggleSidebar => CommandId::ToggleSidebar,
            Command::FocusSidebar => CommandId::FocusSidebar,
            Command::CyclePane => CommandId::CyclePane,
            Command::CyclePaneBack => CommandId::CyclePaneBack,
            Command::NextFolder => CommandId::NextFolder,
            Command::PrevFolder => CommandId::PrevFolder,
            Command::ToggleFolder => CommandId::ToggleFolder,
            Command::RenameSavedSearch => CommandId::RenameSavedSearch,
            Command::MoveSavedSearchUp => CommandId::MoveSavedSearchUp,
            Command::MoveSavedSearchDown => CommandId::MoveSavedSearchDown,
            Command::DeleteSavedSearch => CommandId::DeleteSavedSearch,
            Command::ToggleAccountEnabled => CommandId::ToggleAccountEnabled,
            Command::RemoveAccount => CommandId::RemoveAccount,
            Command::UpdateCredential => CommandId::UpdateCredential,
            Command::NextScope => CommandId::NextScope,
            Command::Refresh => CommandId::Refresh,
            Command::OpenParts => CommandId::OpenParts,
            Command::NextPart => CommandId::NextPart,
            Command::PrevPart => CommandId::PrevPart,
            Command::OpenPart => CommandId::OpenPart,
            Command::SavePart => CommandId::SavePart,
            Command::SaveAllParts => CommandId::SaveAllParts,
            Command::OpenPartExternally => CommandId::OpenPartExternally,
            Command::RenderPartOnce => CommandId::RenderPartOnce,
            Command::ScrollReaderDown => CommandId::ScrollReaderDown,
            Command::ScrollReaderUp => CommandId::ScrollReaderUp,
        }
    }

    /// The invocation a keystroke or a palette row implies: it targets the
    /// current selection and leaves every choice for app state to resolve.
    ///
    /// This is what makes one registry able to feed both the keymap and the
    /// palette — neither has to know a command's payload shape.
    pub fn default_for(id: CommandId) -> Command {
        match id {
            CommandId::NextMessage => Command::NextMessage,
            CommandId::PrevMessage => Command::PrevMessage,
            CommandId::FirstMessage => Command::FirstMessage,
            CommandId::LastMessage => Command::LastMessage,
            CommandId::OpenMessage => Command::OpenMessage { message: None },
            CommandId::ToggleSelection => Command::ToggleSelection { message: None },
            CommandId::ExtendSelectionDown => Command::ExtendSelectionDown,
            CommandId::ExtendSelectionUp => Command::ExtendSelectionUp,
            CommandId::SelectAll => Command::SelectAll,
            CommandId::PrevView => Command::PrevView,
            CommandId::Back => Command::Back,
            CommandId::ToggleResultOrder => Command::ToggleResultOrder,
            CommandId::NextInConversation => Command::NextInConversation,
            CommandId::PrevInConversation => Command::PrevInConversation,
            CommandId::ToggleFold => Command::ToggleFold,
            CommandId::ViewOriginal => Command::ViewOriginal,
            CommandId::ExpandAll => Command::ExpandAll,
            CommandId::Reply => Command::Reply { message: None },
            CommandId::ReplyAll => Command::ReplyAll { message: None },
            CommandId::Forward => Command::Forward { message: None },
            CommandId::Archive => Command::Archive {
                target: MessageTarget::Selection,
            },
            CommandId::ArchiveThread => Command::ArchiveThread { thread: None },
            CommandId::Delete => Command::Delete {
                target: MessageTarget::Selection,
            },
            CommandId::Move => Command::Move {
                target: MessageTarget::Selection,
                to: None,
            },
            CommandId::Flag => Command::Flag {
                target: MessageTarget::Selection,
                flagged: None,
            },
            CommandId::MarkUnread => Command::MarkUnread {
                target: MessageTarget::Selection,
                unread: None,
            },
            CommandId::Snooze => Command::Snooze {
                target: MessageTarget::Selection,
            },
            CommandId::Unsnooze => Command::Unsnooze {
                target: MessageTarget::Selection,
            },
            CommandId::AddLabel => Command::AddLabel {
                target: MessageTarget::Selection,
                label: None,
                on: None,
            },
            CommandId::Search => Command::Search { query: None },
            CommandId::SaveSearch => Command::SaveSearch,
            CommandId::Compose => Command::Compose { draft: None },
            CommandId::Send => Command::Send,
            CommandId::ScheduleSend => Command::ScheduleSend,
            CommandId::SaveDraft => Command::SaveDraft,
            CommandId::DiscardDraft => Command::DiscardDraft,
            CommandId::MarkSent => Command::MarkSent { draft: None },
            CommandId::AttachFile => Command::AttachFile { path: None },
            CommandId::DetachComposer => Command::DetachComposer,
            CommandId::Bold => Command::Bold,
            CommandId::Italic => Command::Italic,
            CommandId::BulletList => Command::BulletList,
            CommandId::NumberedList => Command::NumberedList,
            CommandId::InsertLink => Command::InsertLink,
            CommandId::QuoteBlock => Command::QuoteBlock,
            CommandId::Undo => Command::Undo,
            CommandId::CommandPalette => Command::CommandPalette,
            CommandId::CheatSheet => Command::CheatSheet,
            CommandId::Settings => Command::Settings,
            CommandId::AddAccount => Command::AddAccount,
            CommandId::EditConfig => Command::EditConfig,
            CommandId::ToggleSidebar => Command::ToggleSidebar,
            CommandId::FocusSidebar => Command::FocusSidebar,
            CommandId::CyclePane => Command::CyclePane,
            CommandId::CyclePaneBack => Command::CyclePaneBack,
            CommandId::NextFolder => Command::NextFolder,
            CommandId::PrevFolder => Command::PrevFolder,
            CommandId::ToggleFolder => Command::ToggleFolder,
            CommandId::RenameSavedSearch => Command::RenameSavedSearch,
            CommandId::MoveSavedSearchUp => Command::MoveSavedSearchUp,
            CommandId::MoveSavedSearchDown => Command::MoveSavedSearchDown,
            CommandId::DeleteSavedSearch => Command::DeleteSavedSearch,
            CommandId::ToggleAccountEnabled => Command::ToggleAccountEnabled,
            CommandId::RemoveAccount => Command::RemoveAccount,
            CommandId::UpdateCredential => Command::UpdateCredential,
            CommandId::NextScope => Command::NextScope,
            CommandId::Refresh => Command::Refresh,
            CommandId::OpenParts => Command::OpenParts,
            CommandId::NextPart => Command::NextPart,
            CommandId::PrevPart => Command::PrevPart,
            CommandId::OpenPart => Command::OpenPart,
            CommandId::SavePart => Command::SavePart,
            CommandId::SaveAllParts => Command::SaveAllParts,
            CommandId::OpenPartExternally => Command::OpenPartExternally,
            CommandId::RenderPartOnce => Command::RenderPartOnce,
            CommandId::ScrollReaderDown => Command::ScrollReaderDown,
            CommandId::ScrollReaderUp => Command::ScrollReaderUp,
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
}
