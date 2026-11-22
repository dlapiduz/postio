/// Crosses because only one remedy is the user's: a rejected credential needs re-entering, a broken
/// network recovers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FailureReasonFfi {
    /// The server refused the credential. **Never retried on a timer** —
    /// retrying a rejected credential is how an account gets locked.
    Auth,
    /// The network path is broken in a way backoff has given up on. Recovers
    /// on its own; nothing for the user to fix in Postio.
    Network,
    Server,
    Other,
}

/// Four states, not a boolean: rendering `Failing` as "offline" sends the user to their network
/// when the answer is their password.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ConnectionStateFfi {
    Offline,
    Connecting,
    Online,
    Failing { reason: FailureReasonFfi },
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

/// The optional tier (ADR 0019 Q2): append variants at the end and never remove or reorder, since
/// frontends are not rebuilt in step. Every variant must be ignorable; fields are ids and counts,
/// never content.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum UiEvent {
    MailboxesChanged {
        account: i64,
    },
    MessageListChanged {
        account: i64,
        mailbox: i64,
    },
    MessagesChanged {
        account: i64,
        messages: Vec<i64>,
    },
    MessagesRemoved {
        account: i64,
        mailbox: i64,
        messages: Vec<i64>,
    },
    NewMail {
        account: i64,
        mailbox: i64,
        messages: Vec<i64>,
    },
    /// An open thread window or expansion re-reads its `ThreadView`; `members_added` drives the
    /// new-reply banner.
    /// Once per thread per pass.
    ThreadChanged {
        account: i64,
        thread: i64,
        members_added: Vec<i64>,
    },
    /// Boundary-local, no engine event. Carries the thread so a pane that has moved on drops a late
    /// read.
    ConversationReady {
        thread: i64,
    },
    ConnectionChanged {
        account: i64,
        state: ConnectionStateFfi,
    },
    /// Boundary-local: a person asked this window for the re-index, and a button without progress
    /// looks dead (#1284).
    ReindexProgress {
        account: i64,
        done: u32,
        total: u32,
    },
    /// The only thing a first run has to show: a large backfill is otherwise minutes of an empty
    /// list.
    SyncProgress {
        account: i64,
        done: u32,
        total: u32,
    },
    /// The outcome of something the user asked for, as a sentence already phrased by the core.
    /// The rejected command's id does not cross: a frontend that branched on it would re-decide
    /// what the core decided.
    Notice {
        kind: NoticeKindFfi,
        message: String,
        /// Draw an Undo affordance. Only true for `Completed`: a refusal or failure changed
        /// nothing.
        undoable: bool,
    },
    /// Not a silent drop: an event this boundary does not model arrives as a name the far side can
    /// log.
    Other {
        kind: String,
    },
    /// `[keys]` changed while the app ran: every key, menu item and keycap may mean something else.
    KeymapChanged,
    /// The surfaced rows (digests, reminders) changed, so the list re-reads them.
    SurfacedChanged,
    /// Redraw the list whole, `total` rows long.
    FocusListChanged {
        total: u32,
    },
    FocusPageReady {
        page: u32,
    },
    /// Draw the ring on `position` and bring it into view.
    FocusCursor {
        position: u32,
        to_top: bool,
    },
    FocusSelection {
        selected: Vec<i64>,
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
    FocusListToTop,
    /// Draw `children` after the row at `anchor`, replacing any it had; no animation under Reduce
    /// Motion.
    FocusSplice {
        anchor: u32,
        /// One per message, oldest first, then the unsent reply.
        children: Vec<crate::ChildRowFfi>,
    },
    /// Draw the footer: the scope in words, the keys that matter, and what acts on what. Sent after
    /// every cursor move.
    FocusFooter {
        view: crate::FooterViewFfi,
    },
    FocusUnsplice {
        anchor: u32,
    },
    FocusToast {
        text: String,
        kind: ToastKindFfi,
        undoable: bool,
        /// How long it stays, when not the usual (an answer's window).
        seconds: Option<u32>,
    },
    /// Show `message` in the message window: opening it, or in place of the
    /// one it shows. `index` of `total` is its row's place in the list.
    FocusOpenMessage {
        message: i64,
        index: u32,
        total: u32,
    },
    /// A digest's row opens its window. `delivery` is the digest delivery,
    /// as `FocusRowFfi.id` carries it.
    FocusOpenDigest {
        delivery: i64,
    },
    /// Show the known-sender popover, or draw it again whole. The controller has put it on the
    /// stack as `ContactPopover`; it closes on `FocusCloseSurface`, and any other way is
    /// `focus_surface_closed(ContactPopover)`.
    FocusContactPopover {
        view: crate::focus_contacts::ContactPopoverViewFfi,
    },
    /// Draw the Contacts frame whole in the list's place; read rows with `focus_contacts_row` and
    /// the card with `focus_contacts_card`. Closes on `FocusCloseSurface { kind: Contacts }`.
    FocusContacts {
        view: crate::focus_contacts::ContactsViewFfi,
    },
    /// Contacts rows landed: draw `count` rows from `first`, or the whole
    /// list when `count` is 0 (it started over).
    FocusContactsRows {
        first: u32,
        count: u32,
    },
    FocusContactsCursor {
        index: Option<u32>,
    },
    /// The card beside the Contacts list: draw it whole, or empty while it
    /// is read.
    FocusContactsCard {
        card: Option<crate::focus_contacts::ContactCardViewFfi>,
    },
    FocusContactMerge {
        /// The sheet, or `None` when it is gone.
        view: Option<crate::focus_contact_merge::MergeSheetViewFfi>,
    },
    FocusContactEdit {
        /// The form, or `None` when it is gone.
        view: Option<crate::focus_contact_edit::ContactEditViewFfi>,
    },
    FocusContactsSettings {
        view: crate::focus_contacts_settings::SuggestionsViewFfi,
    },
    FocusContactsFocusFilter,
    /// Open a composer to this address, as a `mailto:` link would, and say
    /// `focus_surface_opened(Composer)` once it shows.
    FocusWriteTo {
        address: String,
    },
    FocusCloseSurface {
        kind: crate::focus_list::SurfaceKindFfi,
    },
    FocusReader {
        verb: crate::focus_list::ReaderVerbFfi,
    },
    /// The message window's thread bar, when it opens and on every step.
    /// No chips means a thread of one: draw no bar.
    FocusThreadBar {
        view: crate::focus_thread_bar::ThreadBarFfi,
    },
    /// The line above the subject, "Message 4 of 6 · ] newer · [ older";
    /// empty for none.
    FocusKicker {
        text: String,
    },
    FocusOlderBanner {
        banner: Option<crate::focus_thread_bar::ThreadBannerFfi>,
    },
    /// A reply arrived while the window is on another message, which stays
    /// on screen: the accent-tinted banner with "Read it", or none.
    FocusNewReply {
        banner: Option<crate::focus_thread_bar::ThreadBannerFfi>,
    },
    /// Open Thread ▾'s menu (a 340-point popover from the button). The controller has put it on
    /// the stack; say `focus_surface_closed(ThreadMenu)` if it closes itself.
    FocusThreadMenu {
        view: crate::focus_thread_bar::ThreadMenuFfi,
    },
    /// Open the thread outline (a 440-point popover), or redraw it whole. The
    /// controller has put it on the stack; say `focus_surface_closed(Outline)`
    /// if it closes itself.
    FocusOutline {
        view: crate::focus_thread_bar::OutlineFfi,
    },
    FocusOutlineFocusFilter,
    FocusReplyTo {
        /// "Ana Ruiz, Mon 21 Sep (message 1)".
        text: Option<String>,
    },
    FocusKeyboardHome,
    /// Show the command bar, opened `mode`'s way, its field holding `text` with `select` selected,
    /// or the caret at the end. Said again while up, it is only new words. Say
    /// `focus_surface_opened(Bar)` once it shows.
    FocusOpenBar {
        mode: crate::focus_bar::BarModeFfi,
        text: String,
        select: Option<crate::focus_bar::BarSelectFfi>,
    },
    FocusBarLines {
        view: crate::focus_bar::BarViewFfi,
    },
    FocusPlace {
        name: String,
    },
    FocusOpenPlaces,
    /// The places were read again: the popover asks `focus_places` anew.
    FocusPlacesChanged,
    FocusShowFiltered,
    /// Run this registry command as the Mac's own -- a line of the bar the
    /// controller does not answer, such as Compose or Settings -- the way a
    /// menu item would run it.
    FocusRun {
        command: String,
    },
    /// Show this picker in a transient popover hung from its anchor. Say
    /// `focus_surface_opened(Picker)` once shown; it closes on `FocusCloseSurface { kind: Picker
    /// }`, and any other way is `focus_surface_closed(Picker)`.
    FocusOpenPicker {
        view: crate::focus_pickers::PickerViewFfi,
    },
    /// Redraw the picker up, whole: its rows read, a label toggled, the
    /// field's text and hint changed. Its row tokens replace the last.
    FocusPickerRows {
        view: crate::focus_pickers::PickerViewFfi,
    },
    FocusPickerField,
    /// The banner under the header strip, or none. Said only when it changes.
    FocusBanner {
        banner: Option<crate::focus_states::BannerFfi>,
    },
    /// What the toolbar's sync label says now. Said only when it changes.
    FocusSyncLabel {
        /// "Synced 09:30", "Syncing 1,200 of 8,400", "Offline".
        text: String,
        mark: crate::focus_states::SyncMarkFfi,
    },
    /// The page an empty list shows in its place, or the list again with `None`.
    FocusEmpty {
        page: Option<crate::focus_states::EmptyPageFfi>,
    },
    /// How far filling in an account's message bodies has come.
    BackfillProgress {
        account: i64,
        done: u32,
        total: u32,
    },
    /// Show the key map (`?`) as a sheet. It closes on `FocusCloseSurface { kind: KeyMap }` (say
    /// nothing back);
    /// any other way is `focus_surface_closed(KeyMap)`. On `KeymapChanged` while up, draw
    /// `focus_key_map()` again.
    FocusOpenKeyMap {
        sheet: crate::focus_keymap::KeyMapSheetFfi,
    },
    /// Draw Filtered whole in the list's place, after `FocusShowFiltered` and whenever its tabs,
    /// rows or focus change. Closes on `FocusCloseSurface { kind: Filtered }`.
    FocusFiltered {
        view: crate::focus_surfaces::FilteredViewFfi,
    },
    /// A thread opened under Filtered's row at `anchor` (an index into its rows): draw `children`
    /// after it, replacing any it had.
    FocusFilteredSplice {
        anchor: u32,
        children: Vec<crate::ChildRowFfi>,
    },
    FocusFilteredUnsplice {
        anchor: u32,
    },
    /// Filtered's keyboard moved to the row at `index` (`j`/`k`, a click):
    /// a display position, an open thread's children counted.
    FocusFilteredFocus {
        index: Option<u32>,
    },
    /// Filtered's selection changed: check the boxes of `selected` (and no
    /// others), and draw the bar, or take it down with `None`.
    FocusFilteredSelection {
        selected: Vec<i64>,
        bar: Option<crate::focus_surfaces::FilteredBarFfi>,
    },
    /// Draw the digest's window whole, after `FocusOpenDigest` and on every change. Closes on
    /// `FocusCloseSurface { kind: Digest }`.
    FocusDigest {
        view: crate::focus_surfaces::DigestViewFfi,
    },
    /// Ask before doing it: an alert with Cancel and one button. On yes,
    /// say `focus_confirmed(confirm.token)`; on no, nothing.
    FocusConfirm {
        confirm: crate::focus_surfaces::ConfirmFfi,
    },
    /// Show the digest-this-sender sheet. Closes on `FocusCloseSurface { kind: Dialog }` (say
    /// nothing back); any other way is `focus_surface_closed(Dialog)`.
    FocusOpenRule {
        view: crate::focus_surfaces::RuleViewFfi,
    },
    FocusRule {
        view: crate::focus_surfaces::RuleViewFfi,
    },
    /// Show the capture window, replacing the secondary window open. Closes on `FocusCloseSurface {
    /// kind: Capture }`.
    FocusOpenCapture {
        view: crate::focus_surfaces::CaptureViewFfi,
    },
    FocusCapture {
        view: crate::focus_surfaces::CaptureViewFfi,
    },
    /// Open the composer, or while open replace its content (the controller saved it first), on a
    /// draft of `kind` answering `message`: `newDraft()`, `replyDraft`, `forwardDraft`, or
    /// `draftForMessage` for `Draft`. Say `focus_surface_closed(Composer)` only for a close the
    /// toolkit made, and `focus_composer_edited()` on every edit.
    FocusComposer {
        kind: crate::focus_compose::ComposerKindFfi,
        /// The message it answers, or the draft's own message.
        message: Option<i64>,
    },
    /// Draw the *What to forward* chooser above To, or redraw it with the chosen card ringed. While
    /// it holds the keyboard, `1` `2` `3` resolve in `UiContext::ForwardChooser`.
    FocusForwardChooser {
        view: crate::focus_compose::ChooserFfi,
    },
    /// Draw the older-reply bar under the composer's title, or (`None`) take it away.
    FocusOlderReply {
        view: Option<crate::focus_compose::OlderReplyFfi>,
    },
    /// Show these messages, newer than the one answered, read-only in the
    /// composer; `None` puts them away.
    FocusNewerMessages {
        /// The messages, oldest first.
        ids: Option<Vec<i64>>,
    },
    /// The forwarded or quoted messages as collapsed rows (`false`) or
    /// expanded (`true`).
    FocusQuotedExpanded {
        expanded: bool,
    },
    /// The engine is about to change the composer's draft: answer with
    /// `focus_composer_draft(edit, draft)`, the draft as it is now with
    /// what was typed.
    FocusComposerDraftWanted {
        edit: crate::focus_compose::DraftEditFfi,
    },
    /// Put this draft in the composer in place of its own, keeping the
    /// caret where it can be kept. Its `from` and `path` are empty: the
    /// composer keeps its own.
    FocusComposerDraft {
        draft: crate::DraftFfi,
    },
    /// Save what the composer holds -- open, or just closed -- now, and say
    /// how it went with `focus_draft_saved(composition, …)`.
    FocusSaveDraft {
        /// Echoed back to `focus_draft_saved`.
        composition: u64,
    },
    /// A toast with one way to put right what it says is missing: a button
    /// labelled `label` that runs `command` ("Add account").
    FocusOffer {
        text: String,
        label: String,
        command: String,
    },
    /// Draw the search dropdown whole in place of the bar's lines; a `FocusBarLines` after it puts
    /// them back.
    FocusDropdown {
        view: crate::focus_search::DropdownViewFfi,
    },
    /// The query's chips and words in the field, and the filter bar's buttons, after any edit.
    FocusQuery {
        view: crate::focus_search::QueryViewFfi,
    },
    /// The main window shows a search's results: this frame, whole --
    /// entering them, and after every query, tab or sort change. Read the
    /// rows with `focus_search_row`.
    FocusResults {
        view: crate::focus_search::ResultsViewFfi,
    },
    /// Results rows `first..first + count` changed: read them again.
    FocusResultsPage {
        first: u64,
        count: u64,
    },
    FocusResultsCursor {
        position: u64,
    },
    /// A result opened in place: draw its matching messages and the lines between them after the
    /// row at `anchor`, or none to take them away. Results after it keep their positions.
    FocusResultSplice {
        splice: Option<crate::ResultSpliceFfi>,
    },
    /// The focus ring is on this child of the opened result, or back on
    /// the result's own row.
    FocusResultChildFocus {
        child: Option<u32>,
    },
    /// The message window was opened from a search: its banner, or none.
    FocusMatchBanner {
        banner: Option<crate::MatchBannerFfi>,
    },
    /// Back to the inbox: Esc's last rung, or ⌘[. The list's own cursor and
    /// selection follow.
    FocusLeaveResults,
    /// Hang this filter popover from its button, or redraw the one open, whole; `None` closes it.
    FocusPopover {
        view: Option<crate::focus_search::PopoverViewFfi>,
    },
    /// Draw Quick Look over the results, whole, in place of the one showing; `None` closes it.
    FocusQuickLook {
        view: Option<crate::focus_search::QuickLookViewFfi>,
    },
    /// Hang the Save popover from the toolbar's Save search button (⌘S); `None` takes it down. Save
    /// is `focus_search_save`.
    FocusSavePopover {
        view: Option<crate::focus_search::SaveViewFfi>,
    },
    /// The results found nothing: draw this page in the rows' place; `None` takes it away. A way
    /// out is run by invoking `pick_relaxation_<n>`.
    FocusRelaxations {
        view: Option<crate::focus_search::NoResultsViewFfi>,
    },
    /// Hand the system this copy of a file: Quick Look on it, or a save panel to put it somewhere;
    /// `None` closes the
    /// Quick Look panel. Say `focus_search_file_done` when the person closes the panel, or the save
    /// is done or cancelled.
    FocusFileCopy {
        copy: Option<crate::focus_search::FileCopyFfi>,
    },
    /// A message's body is now on this machine (#1788): if a window shows it, read its document
    /// again in place.
    BodyLoaded {
        account: i64,
        message: i64,
    },
    /// The server would not give up the body (#1918): read the document again so the plate stops
    /// saying it is downloading.
    BodyFailed {
        account: i64,
        message: i64,
    },
    /// An account's link is back and a window drew this message as not downloaded (#1986): read it
    /// again, which asks for the body.
    BodyReachable {
        account: i64,
        message: i64,
    },
    /// `[sync] notify` or `[sync] badge` changed while the app ran: read `notices` again and
    /// repaint the badge.
    /// Boundary-local.
    NoticesChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ToastKindFfi {
    Completed,
    Undone,
    Notice,
}

/// A refusal is a quiet hint, not an alarm; a completion may offer Undo; a failure must be hard to
/// miss.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum NoticeKindFfi {
    Completed,
    Undone,
    /// A verb could not run (nothing selected, nothing to undo, offline): a quiet hint, not a
    /// dialog.
    Refused,
    Failed,
}

impl From<FailureReasonFfi> for postio_core::FailureReason {
    /// Back the other way, for the wording. `Other` becomes `Config`: "check this account's
    /// settings" suits a failure this build cannot name.
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
            Event::ThreadChanged {
                account,
                thread,
                members_added,
            } => UiEvent::ThreadChanged {
                account: account.into(),
                thread: thread.into(),
                members_added: members_added.into_iter().map(Into::into).collect(),
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
            // Already a sentence for a person, so rule 3 (no content) does not apply.
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
            // Only the variant name crosses: `{:?}` would carry the payload (rule 3).
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
            Event::BodyLoaded {
                account, message, ..
            } => UiEvent::BodyLoaded {
                account: account.into(),
                message: message.into(),
            },
            Event::BodyFailed {
                account, message, ..
            } => UiEvent::BodyFailed {
                account: account.into(),
                message: message.into(),
            },
            Event::BodyReachable { account, message } => UiEvent::BodyReachable {
                account: account.into(),
                message: message.into(),
            },
            other => UiEvent::Other {
                kind: variant_name(&other).to_string(),
            },
        }
    }
}

/// The variant's name, without its payload: `format!("{event:?}")` would put subjects and addresses
/// in a far-side log.
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
    fn a_landed_body_crosses_with_its_message() {
        assert_eq!(
            UiEvent::from(postio_core::Event::BodyLoaded {
                account: 3.into(),
                message: 42.into(),
                sections: postio_core::event::Sections::Parts(vec!["2".to_owned()]),
            }),
            UiEvent::BodyLoaded {
                account: 3,
                message: 42,
            }
        );
    }

    #[test]
    fn a_thread_changing_crosses_with_the_messages_that_joined_it() {
        assert_eq!(
            UiEvent::from(postio_core::Event::ThreadChanged {
                account: 1.into(),
                thread: 2.into(),
                members_added: vec![3.into(), 4.into()],
            }),
            UiEvent::ThreadChanged {
                account: 1,
                thread: 2,
                members_added: vec![3, 4],
            }
        );
    }

    #[test]
    fn an_unmodelled_variant_keeps_its_name_and_loses_its_payload() {
        // Rule 3: the name crosses so the far side can log it, the payload does not.
        let event = postio_core::Event::ComposerClosed { draft: 2.into() };
        let crossed = UiEvent::from(event);
        match crossed {
            UiEvent::Other { kind } => {
                assert_eq!(kind, "ComposerClosed");
                assert!(!kind.contains('2'));
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }
}
