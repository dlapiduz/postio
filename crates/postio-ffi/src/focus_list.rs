//! Focus's list at the boundary (specs/009-focus-macos T027).
//!
//! The rules are the controller's (`postio_focus`, ADR 0045), exactly as GTK
//! drives them: opening a place counts it and changes the list over, a page
//! is read with its pills and the surfaced rows spliced in, an engine event
//! re-reads what it moved. [`FocusDriver`] is the Mac's driver: it runs each
//! request the controller asks for on the session's runtime, keeps the rows
//! in a `ListWindow` the table reads synchronously, and tells Swift what to
//! redraw through the session's own events.
//!
//! A row crosses with the words a row draws, made by the same presenters the
//! GTK row uses, so the two cannot word a row differently.

use std::sync::{Arc, Mutex};

use postio_client::Client;
use postio_config::paths::Platform;
use postio_core::Event;
use postio_focus::{Effect, FocusController, Input, Intent, Policy, Request, Ticket};
use postio_model::listing::ThreadSummary;
use postio_model::{FocusScope, ListScope};
use postio_ui::focus_list::{Conversation, FocusRow};
use postio_ui::focus_row;
use postio_ui::list::{ListWindow, Lookup};

use crate::event::UiEvent;

/// Which of Focus's lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FocusScopeFfi {
    /// Focus's inbox, with what it surfaces spliced in.
    Inbox,
    /// Only the conversations that need an action (`!`).
    HasAction,
    /// Snoozed mail (`g z`).
    Snoozed,
    /// Flagged mail (`g *`).
    Flagged,
}

impl From<FocusScopeFfi> for ListScope {
    fn from(scope: FocusScopeFfi) -> Self {
        ListScope::Focus(match scope {
            FocusScopeFfi::Inbox => FocusScope::Inbox,
            FocusScopeFfi::HasAction => FocusScope::HasAction,
            FocusScopeFfi::Snoozed => FocusScope::Snoozed,
            FocusScopeFfi::Flagged => FocusScope::Flagged,
        })
    }
}

/// A surface over Focus's list, which takes the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SurfaceKindFfi {
    /// One message, in its window.
    Message,
    /// A digest's window.
    Digest,
    /// The composer.
    Composer,
    /// The capture sheet.
    Capture,
    /// Settings.
    Settings,
    /// The key map.
    KeyMap,
    /// Any other dialog or sheet.
    Dialog,
    /// The command bar.
    Bar,
    /// The Filtered view.
    Filtered,
    /// A picker anchored to a row.
    Picker,
    /// A row's menu.
    RowMenu,
}

impl From<SurfaceKindFfi> for postio_focus::SurfaceKind {
    fn from(kind: SurfaceKindFfi) -> Self {
        use postio_focus::SurfaceKind as Kind;
        match kind {
            SurfaceKindFfi::Message => Kind::Message,
            SurfaceKindFfi::Digest => Kind::Digest,
            SurfaceKindFfi::Composer => Kind::Composer,
            SurfaceKindFfi::Capture => Kind::Capture,
            SurfaceKindFfi::Settings => Kind::Settings,
            SurfaceKindFfi::KeyMap => Kind::KeyMap,
            SurfaceKindFfi::Dialog => Kind::Dialog,
            SurfaceKindFfi::Bar => Kind::Bar,
            SurfaceKindFfi::Filtered => Kind::Filtered,
            SurfaceKindFfi::Picker => Kind::Picker,
            SurfaceKindFfi::RowMenu => Kind::RowMenu,
        }
    }
}

impl SurfaceKindFfi {
    /// The controller's kind, as the boundary names it; `None` for one this
    /// build does not draw.
    fn of(kind: postio_focus::SurfaceKind) -> Option<Self> {
        use postio_focus::SurfaceKind as Kind;
        Some(match kind {
            Kind::Message => SurfaceKindFfi::Message,
            Kind::Digest => SurfaceKindFfi::Digest,
            Kind::Composer => SurfaceKindFfi::Composer,
            Kind::Capture => SurfaceKindFfi::Capture,
            Kind::Settings => SurfaceKindFfi::Settings,
            Kind::KeyMap => SurfaceKindFfi::KeyMap,
            Kind::Dialog => SurfaceKindFfi::Dialog,
            Kind::Bar => SurfaceKindFfi::Bar,
            Kind::Filtered => SurfaceKindFfi::Filtered,
            Kind::Picker => SurfaceKindFfi::Picker,
            Kind::RowMenu => SurfaceKindFfi::RowMenu,
            _ => return None,
        })
    }
}

/// What the open message does, as its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ReaderVerbFfi {
    /// Close the More menu.
    CloseMore,
    /// Close find.
    CloseFind,
    /// Find in the message.
    FindInMessage,
    /// The next match.
    FindNext,
    /// The previous match.
    FindPrevious,
    /// App colours or the original.
    SwitchTreatment,
    /// The raw source, in place of the content.
    ViewSource,
    /// The More menu.
    ShowMore,
    /// Step through the conversation.
    StepThread {
        /// By how many messages.
        by: i32,
    },
}

impl ReaderVerbFfi {
    fn of(verb: postio_focus::ReaderVerb) -> Option<Self> {
        use postio_focus::ReaderVerb as Verb;
        Some(match verb {
            Verb::CloseMore => ReaderVerbFfi::CloseMore,
            Verb::CloseFind => ReaderVerbFfi::CloseFind,
            Verb::FindInMessage => ReaderVerbFfi::FindInMessage,
            Verb::FindNext => ReaderVerbFfi::FindNext,
            Verb::FindPrevious => ReaderVerbFfi::FindPrevious,
            Verb::SwitchTreatment => ReaderVerbFfi::SwitchTreatment,
            Verb::ViewSource => ReaderVerbFfi::ViewSource,
            Verb::ShowMore => ReaderVerbFfi::ShowMore,
            Verb::StepThread(by) => ReaderVerbFfi::StepThread { by },
            _ => return None,
        })
    }
}

/// What kind of row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FocusRowKindFfi {
    /// A conversation.
    Conversation,
    /// A reminder that found no reply, drawn as its conversation.
    Reminder,
    /// A digest delivery: one row for everything it holds.
    Digest,
}

/// A label pill: its name, and its stored colour if it has one.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LabelPillFfi {
    /// The label's name.
    pub name: String,
    /// The colour the label was given, as `#rrggbb`, if any.
    pub color: Option<String>,
}

/// One action a marked row offers, with its button's words.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FocusRowActionFfi {
    /// The registry command it runs.
    pub command: String,
    /// The button's words: "Reply", "Accept".
    pub label: String,
}

/// A marked row's second line.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MarkerLineFfi {
    /// The kind chip: "Invite", "Question", "To-do" or "No reply".
    pub chip: String,
    /// The date: "Tue 29 Sep · 10:00–10:45".
    pub date: Option<String>,
    /// The sentence the marker is about, verbatim.
    pub quote: Option<String>,
    /// What stands where the actions would: "Accepted", "Past".
    pub status: Option<String>,
    /// The actions that answer it, in order.
    pub actions: Vec<FocusRowActionFfi>,
}

/// One row of Focus's list, with the words it draws.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FocusRowFfi {
    /// What kind of row.
    pub kind: FocusRowKindFfi,
    /// The message the row opens, or the digest delivery for a digest row.
    pub id: i64,
    /// The conversation, for a conversation or reminder row.
    pub thread: Option<i64>,
    /// Every conversation the row stands for, each copy of a conversation
    /// folded from several accounts (spec 007 T161): what a verb reaches.
    pub threads: Vec<i64>,
    /// The sender column: names, or "Digest" words for a digest.
    pub sender: String,
    /// The subject column.
    pub subject: String,
    /// The first line, as sent.
    pub preview: Option<String>,
    /// The time column: "16:02", "Tue", "12 Sep".
    pub time: String,
    /// The day heading this row sits under: "Today · Saturday 26 September".
    pub day_heading: String,
    /// Unread is bold.
    pub unread: bool,
    /// How many messages, when more than one.
    pub count_badge: Option<String>,
    /// Whether a paperclip is drawn.
    pub has_attachments: bool,
    /// A draft on its way or stopped says which.
    pub send_state: Option<String>,
    /// The label pills, at most two.
    pub pills: Vec<LabelPillFfi>,
    /// The marker's line, for a marked row.
    pub marker: Option<MarkerLineFfi>,
    /// Whether opening it writes rather than reads: a draft not yet on its
    /// way, which opens in the composer.
    pub writes: bool,
}

impl FocusRowFfi {
    /// `row` in words, as of now, in the local zone.
    pub(crate) fn of(row: &FocusRow) -> Self {
        let now = postio_ui::clock::now();
        let today = now.date_naive();
        let day_heading = focus_row::day_heading(postio_ui::focus_list::day_of(row), today);
        match row {
            FocusRow::Conversation(conversation) => {
                conversation_row(FocusRowKindFfi::Conversation, conversation, day_heading)
            }
            FocusRow::Reminder { row, .. } => {
                conversation_row(FocusRowKindFfi::Reminder, row, day_heading)
            }
            FocusRow::Digest(digest) => {
                let senders: Vec<String> = digest
                    .senders
                    .iter()
                    .map(|sender| sender.display().to_owned())
                    .collect();
                FocusRowFfi {
                    kind: FocusRowKindFfi::Digest,
                    id: digest.delivery.get(),
                    thread: None,
                    threads: Vec::new(),
                    sender: focus_row::digest_title(digest.cadence),
                    subject: focus_row::digest_subject(&digest.rule, digest.count),
                    preview: Some(focus_row::digest_line(
                        digest.summary_line.as_deref(),
                        &senders,
                    ))
                    .filter(|line| !line.is_empty()),
                    time: postio_ui::row::timestamp(digest.at, now),
                    day_heading,
                    unread: false,
                    count_badge: focus_row::count_badge(digest.count),
                    has_attachments: false,
                    send_state: None,
                    pills: Vec::new(),
                    marker: None,
                    writes: false,
                }
            }
        }
    }
}

fn conversation_row(
    kind: FocusRowKindFfi,
    conversation: &Conversation,
    day_heading: String,
) -> FocusRowFfi {
    let summary: &ThreadSummary = &conversation.summary;
    let now = postio_ui::clock::now();
    let message = &summary.representative;
    let marker = summary.marker.as_ref().map(|marker| {
        let line = focus_row::marker_line(marker, now.to_utc(), &chrono::Local);
        MarkerLineFfi {
            chip: line.chip.to_owned(),
            date: line.date,
            quote: line.quote,
            status: line.status.map(str::to_owned),
            actions: line
                .actions
                .iter()
                .map(|(command, label)| FocusRowActionFfi {
                    command: command.to_string(),
                    label: (*label).to_owned(),
                })
                .collect(),
        }
    });
    FocusRowFfi {
        kind,
        id: message.id.get(),
        thread: summary.id.map(|thread| thread.get()),
        threads: summary
            .id
            .into_iter()
            .chain(summary.copies.iter().copied())
            .map(|thread| thread.get())
            .collect(),
        sender: focus_row::row_names(message),
        subject: message.subject.clone().unwrap_or_default(),
        preview: message.preview.clone(),
        time: postio_ui::row::timestamp(summary.last_at, now),
        day_heading,
        unread: summary.has_unread(),
        count_badge: focus_row::count_badge(summary.message_count),
        has_attachments: summary.has_attachments,
        send_state: message
            .send_state
            .filter(|state| {
                !matches!(
                    state,
                    postio_model::DraftState::Editing | postio_model::DraftState::Sent
                )
            })
            .map(|state| postio_ui::row::send_state_word(state).to_owned()),
        pills: conversation
            .labels
            .iter()
            .take(focus_row::MAX_PILLS)
            .map(|label| LabelPillFfi {
                name: label.name.clone(),
                color: label.color.clone(),
            })
            .collect(),
        marker,
        writes: !postio_ui::focus_dialog::opens_to_read(message.send_state),
    }
}

/// The Mac's rows, as the controller asks about them.
struct RowsView<'a>(&'a ListWindow<FocusRowFfi>);

impl RowsView<'_> {
    /// The id the controller knows a row by: a digest's is its delivery,
    /// negated, as GTK's rows have it (`FocusRow::id`).
    fn facts_of(row: &FocusRowFfi) -> postio_focus::RowFacts {
        let digest = row.kind == FocusRowKindFfi::Digest;
        postio_focus::RowFacts {
            id: postio_model::MessageId::new(if digest { -row.id } else { row.id }),
            digest,
            writes: row.writes,
            threads: row
                .threads
                .iter()
                .copied()
                .map(postio_model::ThreadId::new)
                .collect(),
        }
    }
}

impl postio_focus::Rows for RowsView<'_> {
    fn len(&self) -> u32 {
        self.0.total()
    }
    fn facts(&self, position: u32) -> Option<postio_focus::RowFacts> {
        self.0.resident_at(position).map(Self::facts_of)
    }
    fn position_of(&self, message: postio_model::MessageId) -> Option<u32> {
        self.0.position_of(message)
    }
    /// A picker names the row by its sender column and subject.
    fn said(&self, position: u32) -> Option<(String, String)> {
        self.0
            .resident_at(position)
            .filter(|row| row.kind != FocusRowKindFfi::Digest)
            .map(|row| (row.sender.clone(), row.subject.clone()))
    }
}

/// Focus's list for the Mac: the controller, and the rows it delivered.
pub(crate) struct FocusDriver {
    focus: Mutex<FocusController>,
    list: Mutex<ListWindow<FocusRowFfi>>,
    client: Client,
    runtime: tokio::runtime::Handle,
    local: async_channel::Sender<UiEvent>,
    /// Pages read from the store, so a test can count what scrolling cost.
    page_reads: std::sync::atomic::AtomicUsize,
    /// The `config.toml` a saved search is written to, when there is one.
    config_path: Mutex<Option<std::path::PathBuf>>,
}

impl FocusDriver {
    pub(crate) fn new(
        client: Client,
        runtime: tokio::runtime::Handle,
        local: async_channel::Sender<UiEvent>,
    ) -> Arc<Self> {
        Arc::new(FocusDriver {
            focus: Mutex::new(FocusController::new(Policy::for_platform(Platform::Apple))),
            list: Mutex::new(ListWindow::new()),
            client,
            runtime,
            local,
            page_reads: std::sync::atomic::AtomicUsize::new(0),
            config_path: Mutex::new(None),
        })
    }

    /// Show `scope`: counted first, then the list changes over.
    pub(crate) fn open(self: &Arc<Self>, scope: ListScope) {
        let effects = self.focus.lock().expect("focus lock").open(scope);
        self.apply(effects);
    }

    /// Run `id` through the controller when it is one of its own -- the
    /// cursor's, the selection's, `!`, a verb on the list -- and say whether
    /// it was. The rest go where they always did.
    pub(crate) fn command(self: &Arc<Self>, id: postio_core::CommandId) -> bool {
        if !self.focus.lock().expect("focus lock").answers(id) {
            return false;
        }
        let effects = {
            let list = self.list.lock().expect("list lock");
            self.focus
                .lock()
                .expect("focus lock")
                .handle_on(Input::Command(id), &RowsView(&list))
        };
        self.apply(effects);
        true
    }

    /// Something the frontend reports that is not a command: a click, a
    /// scroll to the top. Applied over the list as it is held now.
    pub(crate) fn input(self: &Arc<Self>, input: Input) {
        let effects = {
            let list = self.list.lock().expect("list lock");
            self.focus
                .lock()
                .expect("focus lock")
                .handle_on(input, &RowsView(&list))
        };
        self.apply(effects);
    }

    /// `Tab` in the bar: whether the controller used it.
    pub(crate) fn bar_tab(self: &Arc<Self>) -> bool {
        let effects = self.focus.lock().expect("focus lock").handle(Input::BarTab);
        let used = !effects.is_empty();
        self.apply(effects);
        used
    }

    /// The folders popover's places holding `filter`, with their tokens.
    pub(crate) fn places(&self, filter: &str) -> Vec<(u64, postio_ui::places::Entry)> {
        self.focus.lock().expect("focus lock").places(filter)
    }

    /// Where a saved search is written: the `config.toml` this session
    /// reads, when it reads a file.
    pub(crate) fn set_config_path(&self, path: Option<std::path::PathBuf>) {
        *self.config_path.lock().expect("config path lock") = path;
    }

    /// Whether a picker's filter is up and holds nothing: where a bare digit
    /// or space is the picker's rather than typing.
    pub(crate) fn in_empty_filter(&self) -> bool {
        self.focus.lock().expect("focus lock").in_empty_filter()
    }

    /// The key context the controller has in force, when a surface it knows
    /// of is over the list; `None` leaves it to the caller.
    pub(crate) fn key_context(&self) -> Option<postio_ui::keymap::KeyContext> {
        let focus = self.focus.lock().expect("focus lock");
        focus.has_surface().then(|| focus.key_context())
    }

    /// The message under the cursor, once its page has landed: what a verb
    /// that is not the list's own aims at.
    pub(crate) fn cursor_message(&self) -> Option<postio_model::MessageId> {
        // The list, then the controller: the one order every path takes
        // both in, so two threads can never hold one each.
        let list = self.list.lock().expect("list lock");
        let position = self.focus.lock().expect("focus lock").cursor()?;
        list.resident_at(position)
            .filter(|row| row.kind != FocusRowKindFfi::Digest)
            .map(|row| postio_model::MessageId::new(row.id))
    }

    /// The place the list is showing, once one has been opened: the
    /// controller's, which `!` changes as well as `open`.
    pub(crate) fn scope(&self) -> Option<ListScope> {
        self.focus.lock().expect("focus lock").scope()
    }

    /// The list, as a source of facts about its rows: what `aim` asks to
    /// tell a conversation row from a message. Held only while a verb is
    /// aimed.
    pub(crate) fn rows(&self) -> std::sync::MutexGuard<'_, ListWindow<FocusRowFfi>> {
        self.list.lock().expect("list lock")
    }

    /// How many pages have been read from the store since the session opened.
    #[cfg(feature = "testing")]
    pub(crate) fn page_reads(&self) -> usize {
        self.page_reads.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// How many rows the window is holding.
    #[cfg(feature = "testing")]
    pub(crate) fn resident_rows(&self) -> usize {
        self.list.lock().expect("list lock").resident_rows()
    }

    /// How many rows the list draws.
    pub(crate) fn row_count(&self) -> u32 {
        self.list.lock().expect("list lock").total()
    }

    /// The row at `position`, or `None` while its page is on its way: a miss
    /// asks for the page behind the caller, and `FocusPageReady` says when to
    /// draw it. Synchronous; what the table calls for every visible row.
    /// The row standing for `message`, if its page is resident; reads
    /// nothing.
    pub(crate) fn row_of(&self, message: i64) -> Option<FocusRowFfi> {
        let list = self.list.lock().expect("list lock");
        let position = list.position_of(postio_model::MessageId::new(message))?;
        list.resident_at(position).cloned()
    }

    pub(crate) fn row_at(self: &Arc<Self>, position: u32) -> Option<FocusRowFfi> {
        let (wanted, stamp) = {
            let mut list = self.list.lock().expect("list lock");
            let stamp = list.generation();
            match list.row_at(position)? {
                Lookup::Resident(row) => return Some(row.clone()),
                Lookup::Missing { request } => (request, stamp),
            }
        };
        for page in wanted {
            let effects = self
                .focus
                .lock()
                .expect("focus lock")
                .page_wanted(page, stamp);
            self.apply(effects);
        }
        None
    }

    /// An engine event, by the controller's rules.
    pub(crate) fn event(self: &Arc<Self>, event: &Event) {
        let effects = self
            .focus
            .lock()
            .expect("focus lock")
            .handle(Input::Event(event.clone()));
        self.apply(effects);
    }

    /// Do what the controller said, in order. Never called with a lock held.
    fn apply(self: &Arc<Self>, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Show(intent) => self.show(intent),
                Effect::Ask(ticket, request) => self.ask(ticket, request),
                _ => {}
            }
        }
    }

    fn show(self: &Arc<Self>, intent: Intent) {
        match intent {
            Intent::ReplaceSource { total } => {
                let stamp = self.list.lock().expect("list lock").reset(total);
                self.say(UiEvent::FocusListChanged { total });
                let effects = self.focus.lock().expect("focus lock").page_wanted(0, stamp);
                self.apply(effects);
            }
            Intent::PagePending { page, .. } => {
                self.list.lock().expect("list lock").note_pending(page);
            }
            Intent::DeliverPage {
                stamp,
                page,
                total,
                rows,
            } => {
                let rows = rows.iter().map(FocusRowFfi::of).collect();
                let (delivered, resized) = {
                    let mut list = self.list.lock().expect("list lock");
                    let resized = (list.generation() == stamp)
                        .then(|| list.set_total(total))
                        .flatten()
                        .is_some();
                    (list.deliver(stamp, page, rows), resized)
                };
                if resized {
                    self.say(UiEvent::FocusListChanged { total });
                }
                if !delivered.stale {
                    self.say(UiEvent::FocusPageReady { page });
                }
            }
            Intent::AbandonPage { stamp, page } | Intent::GiveUp { stamp, page } => {
                self.list.lock().expect("list lock").abandon(stamp, page);
            }
            Intent::RefreshList => {
                let total = {
                    let mut list = self.list.lock().expect("list lock");
                    list.invalidate();
                    list.total()
                };
                self.say(UiEvent::FocusListChanged { total });
            }
            Intent::Filled => {
                // The list landed: the cursor goes to its first row, back to
                // what `!` kept, or to what an undo brought back.
                let effects = {
                    let list = self.list.lock().expect("list lock");
                    let mut focus = self.focus.lock().expect("focus lock");
                    let opened = focus.take_opened();
                    focus.landed(&RowsView(&list), opened)
                };
                self.apply(effects);
            }
            Intent::Cursor { position, to_top } => {
                self.say(UiEvent::FocusCursor { position, to_top });
            }
            Intent::Selection { selection, summary } => {
                let (selected, everything) = match selection {
                    postio_core::state::Selection::These(ids) => {
                        (ids.iter().map(|id| id.get()).collect(), false)
                    }
                    postio_core::state::Selection::Everything { .. } => (Vec::new(), true),
                };
                self.say(UiEvent::FocusSelection {
                    selected,
                    everything,
                    summary,
                });
            }
            Intent::SingleHeading(text) => self.say(UiEvent::FocusHeading { text }),
            Intent::OpenMessage {
                message,
                index,
                total,
            } => self.say(UiEvent::FocusOpenMessage {
                message: message.get(),
                index,
                total,
            }),
            Intent::OpenDraft { message } => self.say(UiEvent::FocusOpenDraft {
                message: message.get(),
            }),
            Intent::OpenDigest { row } => self.say(UiEvent::FocusOpenDigest {
                delivery: -row.get(),
            }),
            Intent::CloseSurface(kind) => {
                if let Some(kind) = SurfaceKindFfi::of(kind) {
                    self.say(UiEvent::FocusCloseSurface { kind });
                }
            }
            Intent::Reader(verb) => {
                if let Some(verb) = ReaderVerbFfi::of(verb) {
                    self.say(UiEvent::FocusReader { verb });
                }
            }
            Intent::KeyboardHome => self.say(UiEvent::FocusKeyboardHome),
            Intent::ListToTop => self.say(UiEvent::FocusListToTop),
            Intent::Toast { text, kind } => {
                // How long it stays is the controller's, always said.
                let seconds = Some(kind.seconds());
                let (kind, undoable) = match kind {
                    postio_focus::ToastKind::Completed { undoable, .. } => {
                        (crate::event::ToastKindFfi::Completed, undoable)
                    }
                    postio_focus::ToastKind::Undone => (crate::event::ToastKindFfi::Undone, false),
                    postio_focus::ToastKind::Notice => (crate::event::ToastKindFfi::Notice, false),
                };
                self.say(UiEvent::FocusToast {
                    text,
                    kind,
                    undoable,
                    seconds,
                });
            }
            Intent::OpenBar { mode, text, select } => self.say(UiEvent::FocusOpenBar {
                mode: mode.into(),
                text,
                select: select.map(|(start, end)| crate::focus_bar::BarSelectFfi { start, end }),
            }),
            Intent::BarLines(view) => self.say(UiEvent::FocusBarLines { view: view.into() }),
            Intent::Place { name } => self.say(UiEvent::FocusPlace { name }),
            Intent::OpenPlaces => self.say(UiEvent::FocusOpenPlaces),
            Intent::PlacesChanged => self.say(UiEvent::FocusPlacesChanged),
            Intent::ShowFiltered => self.say(UiEvent::FocusShowFiltered),
            Intent::Run(command) => self.say(UiEvent::FocusRun {
                command: command.to_string(),
            }),
            Intent::OpenPicker(view) => self.say(UiEvent::FocusOpenPicker { view: view.into() }),
            Intent::PickerRows(view) => self.say(UiEvent::FocusPickerRows { view: view.into() }),
            Intent::PickerField => self.say(UiEvent::FocusPickerField),
            Intent::Banner(banner) => self.say(UiEvent::FocusBanner {
                banner: banner.map(Into::into),
            }),
            Intent::SyncLabel(label) => self.say(UiEvent::FocusSyncLabel {
                mark: crate::focus_states::SyncMarkFfi::of(&label),
                text: label.text,
            }),
            Intent::Empty(page) => self.say(UiEvent::FocusEmpty {
                page: page.map(Into::into),
            }),
            Intent::SaveSearch { query } => {
                let saved = self.save_search(&query);
                self.input(Input::SearchSaved(saved));
            }
            _ => {}
        }
    }

    /// Keep `query` as a saved search in this session's `config.toml`, as
    /// `postio_ui::saved_search` writes one -- `[saved_searches]` alone, the
    /// rest of the file as it was -- and answer with the saved searches now,
    /// or the sentence the toast says instead.
    fn save_search(&self, query: &str) -> Result<Vec<(String, String)>, String> {
        let Some(path) = self.config_path.lock().expect("config path lock").clone() else {
            return Err(postio_ui::focus_target::NO_CONFIG_TO_SAVE.to_owned());
        };
        postio_ui::saved_search::apply(&path, postio_ui::saved_search::Verb::Save { query })
            .and_then(|_| postio_config::Config::load_from_path(&path))
            .map(|config| postio_session::focus::saved_searches(&config))
            .map_err(|error| {
                tracing::warn!(%error, "Focus could not save the search");
                postio_ui::focus_target::SEARCH_NOT_WRITTEN.to_owned()
            })
    }

    fn ask(self: &Arc<Self>, ticket: Ticket, request: Request) {
        // A post is said now, before anything after it is asked.
        let request = match postio_focus::perform_now(&self.client, request) {
            Ok(reply) => {
                let effects = self
                    .focus
                    .lock()
                    .expect("focus lock")
                    .handle(Input::Reply(ticket, reply));
                self.apply(effects);
                return;
            }
            Err(request) => request,
        };
        if matches!(request, Request::Page { .. }) {
            self.page_reads
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        let driver = Arc::clone(self);
        self.runtime.spawn(async move {
            let reply = postio_focus::perform(&driver.client, request).await;
            let effects = driver
                .focus
                .lock()
                .expect("focus lock")
                .handle(Input::Reply(ticket, reply));
            driver.apply(effects);
        });
    }

    fn say(&self, event: UiEvent) {
        let _ = self.local.try_send(event);
    }
}

impl postio_ui::list::ListRow for FocusRowFfi {
    /// A conversation or reminder row stands for its message; a digest row
    /// stands for a delivery, which is no message.
    fn id(&self) -> Option<postio_model::MessageId> {
        (self.kind != FocusRowKindFfi::Digest).then(|| postio_model::MessageId::new(self.id))
    }

    fn thread(&self) -> Option<postio_model::ThreadId> {
        self.thread.map(postio_model::ThreadId::new)
    }
}
