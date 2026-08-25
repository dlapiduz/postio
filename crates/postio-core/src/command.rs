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

use postio_model::{DraftId, LabelId, MailboxId, MessageId, OperationRange, ThreadId};
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
    /// Show the whole thread the focused message belongs to.
    Thread => "thread",
    /// Show only unread messages in the open thread, or show everything again.
    ToggleThreadUnread => "toggle_thread_unread",
    /// Reverse which end of the open thread comes first.
    ToggleThreadOrder => "toggle_thread_order",
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
    /// Attach a label to the selection.
    AddLabel => "add_label",
    /// Focus the search field.
    Search => "search",
    /// Start a new message.
    Compose => "compose",
    /// Send what is in the composer.
    Send => "send",
    /// Save the composer's contents as a draft.
    SaveDraft => "save_draft",
    /// Throw away the draft in the composer.
    DiscardDraft => "discard_draft",
    /// Attach a file to the draft.
    AttachFile => "attach_file",
    /// Move the composition between the reading pane and a window of its own.
    DetachComposer => "detach_composer",
    /// Undo the last undoable action.
    Undo => "undo",
    /// Open the command palette.
    CommandPalette => "command_palette",
    /// Show the keyboard cheat sheet.
    CheatSheet => "cheat_sheet",
    /// Show the settings panel.
    Settings => "settings",
    /// Open `config.toml` in the user's editor.
    EditConfig => "edit_config",
    /// Show or hide the sidebar.
    ToggleSidebar => "toggle_sidebar",
    /// Put the keyboard in the folder list.
    FocusSidebar => "focus_sidebar",
    /// Move to the next folder in the sidebar.
    NextFolder => "next_folder",
    /// Move to the previous folder in the sidebar.
    PrevFolder => "prev_folder",
    /// Ask the sync engine to check for new mail now.
    Refresh => "refresh",
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
        /// The mailbox those messages are in now.
        ///
        /// Carried rather than derived because the handler needs it only to
        /// say which list has to reload, and asking the store "where are these
        /// 81,717 messages" to answer that would be the read this whole shape
        /// exists to avoid.
        from: MailboxId,
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
    /// Show a thread, or the focused message's thread when `thread` is `None`.
    Thread {
        /// The thread to show; `None` means the focused message's thread.
        thread: Option<ThreadId>,
    },
    /// Toggle the open thread's unread-only filter.
    ToggleThreadUnread,
    /// Reverse the open thread's message order.
    ToggleThreadOrder,

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
    /// Attach a label.
    AddLabel {
        /// What to label.
        target: MessageTarget,
        /// The label; `None` opens the label picker.
        label: Option<LabelId>,
    },

    // -- Search ----------------------------------------------------------
    /// Search, or focus the search field when `query` is `None`.
    Search {
        /// The query to run; `None` focuses the empty search field.
        query: Option<String>,
    },

    // -- Compose ---------------------------------------------------------
    /// Start a new message, optionally from an existing draft.
    Compose {
        /// A draft to resume; `None` starts an empty one.
        draft: Option<DraftId>,
    },
    /// Send the composer's message.
    Send,
    /// Save the composer's message as a draft.
    SaveDraft,
    /// Throw away the composer's draft.
    DiscardDraft,
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

    // -- View and application --------------------------------------------
    /// Undo the last undoable action.
    Undo,
    /// Open the command palette.
    CommandPalette,
    /// Show the keyboard cheat sheet.
    CheatSheet,
    /// Show the settings panel.
    Settings,
    /// Open `config.toml` in the user's editor.
    EditConfig,
    /// Show or hide the sidebar.
    ToggleSidebar,
    /// Put the keyboard in the folder list.
    FocusSidebar,
    /// Move to the next folder.
    NextFolder,
    /// Move to the previous folder.
    PrevFolder,
    /// Check for new mail now.
    Refresh,
}

impl Command {
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
            Command::AddLabel { label, .. } => Command::AddLabel { target, label },
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
            Command::Thread { .. } => CommandId::Thread,
            Command::ToggleThreadUnread => CommandId::ToggleThreadUnread,
            Command::ToggleThreadOrder => CommandId::ToggleThreadOrder,
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
            Command::AddLabel { .. } => CommandId::AddLabel,
            Command::Search { .. } => CommandId::Search,
            Command::Compose { .. } => CommandId::Compose,
            Command::Send => CommandId::Send,
            Command::SaveDraft => CommandId::SaveDraft,
            Command::DiscardDraft => CommandId::DiscardDraft,
            Command::AttachFile { .. } => CommandId::AttachFile,
            Command::DetachComposer => CommandId::DetachComposer,
            Command::Undo => CommandId::Undo,
            Command::CommandPalette => CommandId::CommandPalette,
            Command::CheatSheet => CommandId::CheatSheet,
            Command::Settings => CommandId::Settings,
            Command::EditConfig => CommandId::EditConfig,
            Command::ToggleSidebar => CommandId::ToggleSidebar,
            Command::FocusSidebar => CommandId::FocusSidebar,
            Command::NextFolder => CommandId::NextFolder,
            Command::PrevFolder => CommandId::PrevFolder,
            Command::Refresh => CommandId::Refresh,
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
            CommandId::Thread => Command::Thread { thread: None },
            CommandId::ToggleThreadUnread => Command::ToggleThreadUnread,
            CommandId::ToggleThreadOrder => Command::ToggleThreadOrder,
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
            CommandId::AddLabel => Command::AddLabel {
                target: MessageTarget::Selection,
                label: None,
            },
            CommandId::Search => Command::Search { query: None },
            CommandId::Compose => Command::Compose { draft: None },
            CommandId::Send => Command::Send,
            CommandId::SaveDraft => Command::SaveDraft,
            CommandId::DiscardDraft => Command::DiscardDraft,
            CommandId::AttachFile => Command::AttachFile { path: None },
            CommandId::DetachComposer => Command::DetachComposer,
            CommandId::Undo => Command::Undo,
            CommandId::CommandPalette => Command::CommandPalette,
            CommandId::CheatSheet => Command::CheatSheet,
            CommandId::Settings => Command::Settings,
            CommandId::EditConfig => Command::EditConfig,
            CommandId::ToggleSidebar => Command::ToggleSidebar,
            CommandId::FocusSidebar => Command::FocusSidebar,
            CommandId::NextFolder => Command::NextFolder,
            CommandId::PrevFolder => Command::PrevFolder,
            CommandId::Refresh => Command::Refresh,
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
