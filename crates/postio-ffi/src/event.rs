//! What the frontend hears, and the rules for adding to it.

/// Why a connection stopped in a way retrying will not fix.
///
/// Crosses because the remedies are different and only one of them is the
/// user's: a rejected credential needs them to re-enter it, and a broken
/// network path recovers on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FailureReasonFfi {
    /// The server refused the credential. **Never retried on a timer** —
    /// retrying a rejected credential is how an account gets locked.
    Auth,
    /// The network path is broken in a way backoff has given up on. Recovers
    /// on its own; nothing for the user to fix in Postio.
    Network,
    /// The server accepted the connection and is refusing the work.
    Server,
    /// Something else the supervisor could not classify.
    Other,
}

/// What an account's connection is doing.
///
/// Four states rather than a boolean, and the distinction earns its keep:
/// **offline** means working from the local store, **connecting** means wait,
/// and **failing** means stop waiting and go and fix something. Rendering
/// `Failing` as "offline" tells the user to check their network when the
/// answer is their password, and they will wait for a reconnect that is never
/// coming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ConnectionStateFfi {
    /// Working from the local database only.
    Offline,
    /// Establishing a connection, or waiting out a backoff that will retry.
    Connecting,
    /// Connected, with an idle or streaming session.
    Online,
    /// Stopped on something retrying will not fix, and waiting for a person.
    Failing {
        /// What kind of person-shaped problem it is.
        reason: FailureReasonFfi,
    },
}

impl From<postio_core::ConnectionState> for ConnectionStateFfi {
    fn from(state: postio_core::ConnectionState) -> Self {
        use postio_core::{ConnectionState, FailureReason};
        match state {
            ConnectionState::Offline => ConnectionStateFfi::Offline,
            ConnectionState::Connecting => ConnectionStateFfi::Connecting,
            ConnectionState::Online => ConnectionStateFfi::Online,
            ConnectionState::Failing { reason } => ConnectionStateFfi::Failing {
                reason: match reason {
                    FailureReason::Auth | FailureReason::NoPassword => FailureReasonFfi::Auth,
                    FailureReason::Network => FailureReasonFfi::Network,
                    FailureReason::Server => FailureReasonFfi::Server,
                    _ => FailureReasonFfi::Other,
                },
            },
        }
    }
}

/// One thing that happened, on its way to a repaint.
///
/// # Adding a variant
///
/// This is the boundary's **optional** tier, and the rules exist because a
/// frontend on the other side of an FFI cannot be recompiled in step with this
/// enum (ADR 0019 Q2):
///
/// 1. **Append.** New variants go at the end. A frontend built against an
///    older boundary matches on discriminants it already knows.
/// 2. **Every variant is ignorable.** A frontend that handles none of these
///    must still be correct, only stale. Nothing here may be the sole carrier
///    of a state change the frontend is *required* to act on — that belongs in
///    the required floor, where a missing implementation is a compile error.
/// 3. **Fields are ids and counts, never content.** The same rule the logs
///    live under: a subject line crossing here would end up in a crash report.
/// 4. **Never remove a variant to "clean up".** Removing one renumbers every
///    variant after it, and the frontend that has not been rebuilt will
///    misread all of them. Deprecate in the doc comment instead.
///
/// The contract is here rather than in a document because this is the file
/// somebody has open when they are adding a variant.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum UiEvent {
    /// An account's mailbox tree changed: renamed, created, subscribed.
    MailboxesChanged {
        /// The account whose mailboxes changed.
        account: i64,
    },
    /// A mailbox's message list changed enough that the pane must reload.
    MessageListChanged {
        /// The account the mailbox belongs to.
        account: i64,
        /// The affected mailbox.
        mailbox: i64,
    },
    /// These messages changed in place — flags, labels, read state.
    MessagesChanged {
        /// The account the messages belong to.
        account: i64,
        /// The affected messages.
        messages: Vec<i64>,
    },
    /// These messages left a mailbox: archived, deleted or moved away.
    MessagesRemoved {
        /// The account the mailbox belongs to.
        account: i64,
        /// The mailbox they left.
        mailbox: i64,
        /// The messages that left it.
        messages: Vec<i64>,
    },
    /// New mail arrived — the trigger for a notification.
    NewMail {
        /// The account it arrived at.
        account: i64,
        /// The mailbox it landed in.
        mailbox: i64,
        /// The newly delivered messages.
        messages: Vec<i64>,
    },
    /// The conversation asked for has been read and can now be drawn.
    ///
    /// Boundary-local: the reading pane's read is this frontend's, and the
    /// engine has no event for it. Carries the thread so a pane that has moved on can drop a
    /// read that arrived late rather than drawing the wrong conversation
    /// under someone's cursor.
    ConversationReady {
        /// The conversation that was read.
        thread: i64,
    },
    /// An account's connection changed.
    ConnectionChanged {
        /// The account.
        account: i64,
        /// What it is doing now.
        state: ConnectionStateFfi,
    },
    /// How far a re-index has got.
    ///
    /// Boundary-local: re-indexing is something a person
    /// asked this window for, not something the engine does on its own. A
    /// pass over five thousand messages takes long enough that a button with
    /// no progress is indistinguishable from a button that does nothing
    /// (#1284).
    ReindexProgress {
        /// The account being re-indexed.
        account: i64,
        /// Messages indexed so far.
        done: u32,
        /// Messages to index in total.
        total: u32,
    },
    /// How far a synchronisation has got.
    ///
    /// The only thing a first run has to show that something is happening: a
    /// backfill of a large mailbox is otherwise minutes of an empty list.
    SyncProgress {
        /// The account being synchronised.
        account: i64,
        /// Units completed.
        done: u32,
        /// Units expected.
        total: u32,
    },
    /// The outcome of something the user asked for.
    ///
    /// Four core events with one shape: each is a sentence, already phrased
    /// for a person by the layer that knows what happened, and the frontend's
    /// job is to show it rather than to compose it. They crossed as
    /// [`UiEvent::Other`] until #1577, which meant that on macOS **nothing an
    /// action reported ever reached anybody**: a send that failed, a verb
    /// refused because nothing was selected, an archive that could be taken
    /// back — all of it arrived, was named, and was dropped.
    ///
    /// The rejected command's id does not cross. A frontend that branched on
    /// it would be re-deciding what the core already decided, and the whole
    /// point of the sentence is that the decision was made once.
    Notice {
        /// Which of the four it is, for how it should be drawn.
        kind: NoticeKindFfi,
        /// What to say, phrased for the user by the core.
        message: String,
        /// Whether the undo stack can take it back — draw an Undo affordance.
        ///
        /// Only ever true for [`NoticeKindFfi::Completed`]. A refusal changed
        /// nothing and a failure did not finish, so there is nothing to
        /// return to.
        undoable: bool,
    },
    /// Something happened that this boundary does not model yet.
    ///
    /// Deliberately not a silent drop. The core's event vocabulary is larger
    /// than the macOS frontend's, and will stay larger while the frontend is
    /// being built — so an unmodelled event arrives as a name the far side can
    /// log. A frontend that is *behind* is a normal state; a frontend that was
    /// never told is a bug, and this is the difference.
    Other {
        /// The core variant's name, for a log line on the far side.
        kind: String,
    },
    /// `[keys]` changed while the app ran: every key, menu item and keycap
    /// may mean something else now (specs/009-focus-macos FR-031).
    KeymapChanged,
    /// The rows Focus surfaces in the inbox -- digests, reminders -- changed,
    /// so the list re-reads them (spec 007's `SurfacedChanged`).
    SurfacedChanged,
    /// Focus's list changed over, was re-read, or changed length: redraw it
    /// whole, `total` rows long (specs/009-focus-macos T027).
    FocusListChanged {
        /// How many rows the list draws now.
        total: u32,
    },
    /// A page of Focus's list landed: redraw its rows.
    FocusPageReady {
        /// Which page.
        page: u32,
    },
    /// The cursor is on `position`: draw its ring there and bring it into
    /// view (specs/009-focus-macos T046).
    FocusCursor {
        /// The row.
        position: u32,
        /// Whether the list scrolls back to its very top.
        to_top: bool,
    },
    /// The selection changed: redraw the rows' boxes and the bar's words.
    FocusSelection {
        /// The selected messages, when the selection names them.
        selected: Vec<i64>,
        /// Whether everything the view shows is selected (a predicate).
        everything: bool,
        /// "3 selected", or nothing with nothing selected.
        summary: Option<String>,
    },
    /// The list's one heading while `!` narrows it, or back to the day
    /// headings with none.
    FocusHeading {
        /// "Has action · 7".
        text: Option<String>,
    },
    /// Scroll the list back to its very top.
    FocusListToTop,
    /// Say something in the toast: the host's words.
    FocusToast {
        /// What to say.
        text: String,
        /// How to draw it.
        kind: ToastKindFfi,
        /// Whether Undo can take it back.
        undoable: bool,
        /// How long it stays, when not the usual (an answer's window).
        seconds: Option<u32>,
    },
    /// Show `message` in the message window: opening it, or in place of the
    /// one it shows. `index` of `total` is its row's place in the list.
    FocusOpenMessage {
        /// The message.
        message: i64,
        /// Its row's place.
        index: u32,
        /// How many rows the list draws.
        total: u32,
    },
    /// A draft not yet on its way opens in the composer.
    FocusOpenDraft {
        /// The draft's message.
        message: i64,
    },
    /// A digest's row opens its window. `delivery` is the digest delivery,
    /// as `FocusRowFfi.id` carries it.
    FocusOpenDigest {
        /// The delivery.
        delivery: i64,
    },
    /// Close this surface, then say `focus_surface_closed`.
    FocusCloseSurface {
        /// Which.
        kind: crate::focus_list::SurfaceKindFfi,
    },
    /// The open message does this.
    FocusReader {
        /// What.
        verb: crate::focus_list::ReaderVerbFfi,
    },
    /// Nothing is over the list: the keyboard goes back to it, on the
    /// cursor's row.
    FocusKeyboardHome,
    /// Show the command bar, opened `mode`'s way, its field holding `text`
    /// with `select` selected (the chip being edited), or the caret at the
    /// end. Said again while the bar is up, it is only new words for the
    /// field. Say `focus_surface_opened(Bar)` once it shows; the controller
    /// has already put it on the stack, so that changes nothing.
    FocusOpenBar {
        /// How it was opened.
        mode: crate::focus_bar::BarModeFfi,
        /// The field's words.
        text: String,
        /// What of them is selected.
        select: Option<crate::focus_bar::BarSelectFfi>,
    },
    /// Redraw the bar's lines, whole.
    FocusBarLines {
        /// Everything the bar draws under its field.
        view: crate::focus_bar::BarViewFfi,
    },
    /// The list shows this place now: what the header strip's Inbox ▾
    /// button names.
    FocusPlace {
        /// "Inbox", "Receipts", "Snoozed".
        name: String,
    },
    /// Show the folders popover, anchored to Inbox ▾, listing
    /// `focus_places`.
    FocusOpenPlaces,
    /// The places were read again: the popover asks `focus_places` anew.
    FocusPlacesChanged,
    /// Show Filtered, the view of what Focus filed away.
    FocusShowFiltered,
    /// Run this registry command as the Mac's own -- a line of the bar the
    /// controller does not answer, such as Compose or Settings -- the way a
    /// menu item would run it.
    FocusRun {
        /// The registry command.
        command: String,
    },
    /// Show this picker, hung from its anchor, in a transient popover
    /// (specs/009-focus-macos T089). Say `focus_surface_opened(Picker)` once
    /// it shows; the controller has already put it on the stack, so that
    /// changes nothing. It closes on `FocusCloseSurface { kind: Picker }`,
    /// and any other way it closes is `focus_surface_closed(Picker)`.
    FocusOpenPicker {
        /// Everything it draws.
        view: crate::focus_pickers::PickerViewFfi,
    },
    /// Redraw the picker up, whole: its rows read, a label toggled, the
    /// field's text and hint changed. Its row tokens replace the last.
    FocusPickerRows {
        /// Everything it draws.
        view: crate::focus_pickers::PickerViewFfi,
    },
    /// Put the keyboard in the picker's date field (`Tab`).
    FocusPickerField,
    /// The banner under the header strip, full width, or none
    /// (specs/009-focus-macos T096). Said only when it changes.
    FocusBanner {
        /// What it says, or `None` to take it away.
        banner: Option<crate::focus_states::BannerFfi>,
    },
    /// What the toolbar's sync label says now. Said only when it changes.
    FocusSyncLabel {
        /// "Synced 09:30", "Syncing 1,200 of 8,400", "Offline".
        text: String,
        /// The mark beside it.
        mark: crate::focus_states::SyncMarkFfi,
    },
    /// The page an empty list shows in its place, or the list again with
    /// `None`. The list is what is drawn until this says otherwise.
    FocusEmpty {
        /// What the page says.
        page: Option<crate::focus_states::EmptyPageFfi>,
    },
    /// How far filling in an account's message bodies has come
    /// (specs/009-focus-macos T099). It crossed as `Other` before.
    BackfillProgress {
        /// The account.
        account: i64,
        /// Messages the queue has finished with.
        done: u32,
        /// Messages that have entered the queue.
        total: u32,
    },
    /// Show the key map (`?`), as a sheet over the main window
    /// (specs/009-focus-macos T103). The controller has put it on the stack;
    /// it closes on `FocusCloseSurface { kind: KeyMap }`, and any other way
    /// it closes is `focus_surface_closed(KeyMap)`. On `KeymapChanged` while
    /// it is up, draw `focus_key_map()` again.
    FocusOpenKeyMap {
        /// What it draws.
        sheet: crate::focus_keymap::KeyMapSheetFfi,
    },
}

/// How a Focus toast is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ToastKindFfi {
    /// A verb ran.
    Completed,
    /// An undo was applied.
    Undone,
    /// A command could not run: a quiet hint.
    Notice,
}

/// What an outcome was.
///
/// The four are drawn differently and mean different things: a completion may
/// offer to be taken back, a refusal is a quiet hint rather than an alarm,
/// and a failure is the one that has to be hard to miss. The classic app drew
/// each with its own toast, which is the shape this is named after.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum NoticeKindFfi {
    /// A verb ran. *Archived 12 messages.*
    Completed,
    /// An undo was applied. *Archived 12 messages, undone.*
    Undone,
    /// A verb could not run — nothing selected, nothing to undo, offline.
    ///
    /// **Not an error.** The answer is a quiet hint, not a dialog: the user
    /// asked for something that does not apply, which is an ordinary thing to
    /// do with a keyboard.
    Refused,
    /// Something failed and the user should know.
    Failed,
}

impl From<FailureReasonFfi> for postio_core::FailureReason {
    /// Back the other way, for the wording.
    ///
    /// `Other` becomes `Config` rather than gaining a variant of its own:
    /// the core's four are exhaustive, the boundary's fifth exists only
    /// because `_ =>` above is a forward-compatibility hatch, and "check this
    /// account's settings" is the right thing to say about a failure this
    /// build cannot name.
    fn from(reason: FailureReasonFfi) -> Self {
        match reason {
            FailureReasonFfi::Auth => Self::Auth,
            FailureReasonFfi::Network => Self::Network,
            FailureReasonFfi::Server => Self::Server,
            FailureReasonFfi::Other => Self::Config,
        }
    }
}

impl From<postio_core::Event> for UiEvent {
    fn from(event: postio_core::Event) -> Self {
        use postio_core::Event;
        match event {
            Event::MailboxesChanged { account } => UiEvent::MailboxesChanged {
                account: account.into(),
            },
            Event::MessageListChanged { account, mailbox } => UiEvent::MessageListChanged {
                account: account.into(),
                mailbox: mailbox.into(),
            },
            Event::MessagesChanged { account, messages } => UiEvent::MessagesChanged {
                account: account.into(),
                messages: messages.into_iter().map(Into::into).collect(),
            },
            Event::MessagesRemoved {
                account,
                mailbox,
                messages,
            } => UiEvent::MessagesRemoved {
                account: account.into(),
                mailbox: mailbox.into(),
                messages: messages.into_iter().map(Into::into).collect(),
            },
            Event::NewMail {
                account,
                mailbox,
                messages,
            } => UiEvent::NewMail {
                account: account.into(),
                mailbox: mailbox.into(),
                messages: messages.into_iter().map(Into::into).collect(),
            },
            Event::ConnectionChanged { account, state } => UiEvent::ConnectionChanged {
                account: account.into(),
                state: state.into(),
            },
            Event::SyncProgress {
                account,
                done,
                total,
            } => UiEvent::SyncProgress {
                account: account.into(),
                done,
                total,
            },
            // The four outcomes. Their payload is already a sentence written
            // for a person, so rule 3 does not apply to it the way it applies
            // to an id or a subject line: this *is* what the user is meant to
            // read.
            Event::ActionCompleted {
                description,
                undoable,
            } => UiEvent::Notice {
                kind: NoticeKindFfi::Completed,
                message: description,
                undoable,
            },
            Event::UndoPerformed { description } => UiEvent::Notice {
                kind: NoticeKindFfi::Undone,
                message: description,
                undoable: false,
            },
            Event::CommandRejected { reason, .. } => UiEvent::Notice {
                kind: NoticeKindFfi::Refused,
                message: reason,
                undoable: false,
            },
            Event::Error { message, .. } => UiEvent::Notice {
                kind: NoticeKindFfi::Failed,
                message,
                undoable: false,
            },
            // Rule 2 in practice: everything the boundary has not modelled yet
            // still arrives, named. `{:?}` would carry the payload, and rule 3
            // forbids that, so only the variant name crosses.
            Event::SurfacedChanged => UiEvent::SurfacedChanged,
            Event::BackfillProgress {
                account,
                done,
                total,
                ..
            } => UiEvent::BackfillProgress {
                account: account.into(),
                done,
                total,
            },
            other => UiEvent::Other {
                kind: variant_name(&other).to_string(),
            },
        }
    }
}

/// The variant's name, without its payload.
///
/// `format!("{event:?}")` would be shorter and would put subject lines and
/// addresses into a string that ends up in a far-side log. This takes the
/// debug rendering only as far as the first delimiter, which is the name.
fn variant_name(event: &postio_core::Event) -> String {
    let rendered = format!("{event:?}");
    rendered
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .next()
        .unwrap_or("Unknown")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_survive_the_crossing() {
        let event = postio_core::Event::MessagesRemoved {
            account: 3.into(),
            mailbox: 9.into(),
            messages: vec![11.into(), 12.into()],
        };
        assert_eq!(
            UiEvent::from(event),
            UiEvent::MessagesRemoved {
                account: 3,
                mailbox: 9,
                messages: vec![11, 12],
            }
        );
    }

    /// The four events that are the *outcome* of something the user did.
    ///
    /// They all fell to `Other`, so on macOS nothing an action reported ever
    /// reached anybody: a send that failed, a verb refused because nothing
    /// was selected, an archive that could be taken back. GTK answers each
    /// with a toast, and `u` and the toast's own button reach the same undo.
    #[test]
    fn an_outcome_crosses_with_its_sentence() {
        assert_eq!(
            UiEvent::from(postio_core::Event::ActionCompleted {
                description: "Archived 12 messages".to_owned(),
                undoable: true,
            }),
            UiEvent::Notice {
                kind: NoticeKindFfi::Completed,
                message: "Archived 12 messages".to_owned(),
                undoable: true,
            }
        );
        assert_eq!(
            UiEvent::from(postio_core::Event::UndoPerformed {
                description: "Archived 12 messages, undone".to_owned(),
            }),
            UiEvent::Notice {
                kind: NoticeKindFfi::Undone,
                message: "Archived 12 messages, undone".to_owned(),
                undoable: false,
            }
        );
        assert_eq!(
            UiEvent::from(postio_core::Event::CommandRejected {
                command: postio_core::CommandId::Archive.into(),
                reason: "Nothing is selected".to_owned(),
            }),
            UiEvent::Notice {
                kind: NoticeKindFfi::Refused,
                message: "Nothing is selected".to_owned(),
                undoable: false,
            }
        );
        assert_eq!(
            UiEvent::from(postio_core::Event::Error {
                message: "The server refused the password".to_owned(),
                account: None,
            }),
            UiEvent::Notice {
                kind: NoticeKindFfi::Failed,
                message: "The server refused the password".to_owned(),
                undoable: false,
            }
        );
    }

    /// An outcome's sentence is the core's, phrased for a person, and it is
    /// the *only* thing that crosses — the rejected command's id does not,
    /// because a frontend that branched on it would be re-deciding what the
    /// core already decided.
    #[test]
    fn a_refusal_carries_why_and_not_which() {
        let crossed = UiEvent::from(postio_core::Event::CommandRejected {
            command: postio_core::CommandId::Undo.into(),
            reason: "There is nothing to undo".to_owned(),
        });
        match crossed {
            UiEvent::Notice { message, kind, .. } => {
                assert_eq!(kind, NoticeKindFfi::Refused);
                assert_eq!(message, "There is nothing to undo");
                assert!(
                    !message.contains("undo_"),
                    "the id leaked into the sentence"
                );
            }
            other => panic!("expected Notice, got {other:?}"),
        }
    }

    #[test]
    fn an_unmodelled_variant_keeps_its_name_and_loses_its_payload() {
        // Rule 3: the name crosses so the far side can log it; the payload
        // does not, because a subject line in a crash report is exactly the
        // thing the logging rules exist to prevent.
        let event = postio_core::Event::ThreadChanged {
            account: 1.into(),
            thread: 2.into(),
        };
        let crossed = UiEvent::from(event);
        match crossed {
            UiEvent::Other { kind } => {
                assert_eq!(kind, "ThreadChanged");
                assert!(!kind.contains('1') && !kind.contains('2'));
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }
}
