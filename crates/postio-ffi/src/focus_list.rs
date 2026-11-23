//! Focus's list at the boundary. The controller (`postio_focus`, ADR 0045) holds the rules;
//! `FocusDriver` runs its requests on the session's runtime, keeps rows in a `ListWindow` the table
//! reads synchronously, and tells Swift what to redraw. Rows carry words from the same presenters
//! as GTK's.

use std::sync::{Arc, Mutex};

use postio_client::Client;
use postio_core::Event;
use postio_focus::{Effect, FocusController, Input, Intent, Policy, Request, Ticket};
use postio_model::listing::ThreadSummary;
use postio_model::{FocusScope, ListScope};
use postio_ui::focus_list::{Conversation, FocusRow};
use postio_ui::focus_row;
use postio_ui::list::{ListWindow, Lookup};
use postio_ui::thread_list::{self, Slot, Splice};

use crate::event::UiEvent;
use crate::{ChildRowFfi, ThreadRowFfi};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FocusScopeFfi {
    Inbox,
    HasAction,
    Snoozed,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SurfaceKindFfi {
    Message,
    Digest,
    Composer,
    Capture,
    Settings,
    KeyMap,
    Dialog,
    Bar,
    Filtered,
    Picker,
    RowMenu,
    ContactPopover,
    Contacts,
    ThreadMenu,
    Outline,
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
            SurfaceKindFfi::ContactPopover => Kind::ContactPopover,
            SurfaceKindFfi::Contacts => Kind::Contacts,
            SurfaceKindFfi::ThreadMenu => Kind::ThreadMenu,
            SurfaceKindFfi::Outline => Kind::Outline,
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
            Kind::ContactPopover => SurfaceKindFfi::ContactPopover,
            Kind::Contacts => SurfaceKindFfi::Contacts,
            Kind::ThreadMenu => SurfaceKindFfi::ThreadMenu,
            Kind::Outline => SurfaceKindFfi::Outline,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ReaderVerbFfi {
    CloseMore,
    CloseFind,
    FindInMessage,
    FindNext,
    FindPrevious,
    /// ⌘G in a window opened from a search: the next match in the message,
    /// and when it has none, `focus_match_edge(true)`.
    NextMatch,
    /// ⇧⌘G in a window opened from a search: the previous match, and when
    /// there is none, `focus_match_edge(false)`.
    PrevMatch,
    SwitchTreatment,
    ViewSource,
    ShowMore,
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
            Verb::NextMatch => ReaderVerbFfi::NextMatch,
            Verb::PrevMatch => ReaderVerbFfi::PrevMatch,
            Verb::SwitchTreatment => ReaderVerbFfi::SwitchTreatment,
            Verb::ShowMore => ReaderVerbFfi::ShowMore,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FocusRowKindFfi {
    Conversation,
    Reminder,
    Digest,
    Child,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LabelPillFfi {
    pub name: String,
    /// The colour the label was given, as `#rrggbb`, if any.
    pub color: Option<String>,
}

/// The colour a label's dot is drawn in, `#rrggbb`: `stored` when the label has one, else derived
/// from its name, kept off the accent whose hue is `accent_hue` degrees
/// (`postio_ui::label_colour`).
#[uniffi::export]
pub fn label_dot_colour(name: String, stored: Option<String>, accent_hue: f64) -> String {
    use postio_ui::label_colour::{Rgb, label_colour};
    label_colour(&name, stored.as_deref().and_then(Rgb::from_hex), accent_hue).to_hex()
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FocusRowActionFfi {
    pub command: String,
    /// The button's words: "Reply", "Accept".
    pub label: String,
}

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
    pub actions: Vec<FocusRowActionFfi>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FocusRowFfi {
    pub kind: FocusRowKindFfi,
    /// The message the row opens, or the digest delivery for a digest row.
    pub id: i64,
    pub thread: Option<i64>,
    /// Every conversation the row stands for, one per account copy: what a verb reaches.
    pub threads: Vec<i64>,
    pub sender: String,
    /// The address of the sender the column names first: what a click on
    /// the name asks the popover about. `None` for a digest, or a message
    /// with no sender.
    pub from_address: Option<String>,
    pub subject: String,
    pub preview: Option<String>,
    /// The time column: "16:02", "Tue", "12 Sep".
    pub time: String,
    /// The day heading this row sits under: "Today · Saturday 26 September".
    pub day_heading: String,
    pub unread: bool,
    pub count_badge: Option<String>,
    pub has_attachments: bool,
    pub send_state: Option<String>,
    /// The label pills, at most two.
    pub pills: Vec<LabelPillFfi>,
    pub marker: Option<MarkerLineFfi>,
    /// Whether opening it writes rather than reads: a draft not yet on its
    /// way, which opens in the composer.
    pub writes: bool,
    pub thread_row: Option<ThreadRowFfi>,
    pub child: Option<ChildRowFfi>,
}

impl FocusRowFfi {
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
                    from_address: None,
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
                    thread_row: None,
                    child: None,
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
        from_address: message.from.as_ref().map(|from| from.address.clone()),
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
        thread_row: None,
        child: None,
    }
}

/// A row the driver holds: the words it crossed with, and the row they were made from
/// (`Rows::row`).
#[derive(Debug, Clone)]
pub(crate) struct HeldRow {
    pub(crate) ffi: FocusRowFfi,
    pub(crate) row: FocusRow,
}

impl HeldRow {
    fn of(row: FocusRow) -> Self {
        HeldRow {
            ffi: FocusRowFfi::of(&row),
            row,
        }
    }
}

impl postio_ui::list::ListRow for HeldRow {
    fn id(&self) -> Option<postio_model::MessageId> {
        self.ffi.id()
    }

    fn thread(&self) -> Option<postio_model::ThreadId> {
        self.ffi.thread()
    }

    /// The row's own: a fired reminder is not its conversation's row.
    fn key(&self) -> Option<postio_ui::list::RowKey> {
        Some(self.row.key())
    }
}

struct RowsView<'a>(&'a ListWindow<HeldRow>);

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
        self.0
            .resident_at(position)
            .map(|held| Self::facts_of(&held.ffi))
    }
    fn position_of(&self, message: postio_model::MessageId) -> Option<u32> {
        self.0.position_of(message)
    }
    fn position_of_thread(&self, thread: postio_model::ThreadId) -> Option<u32> {
        self.0
            .position_of_key(postio_ui::list::RowKey::Thread(thread))
    }
    fn said(&self, position: u32) -> Option<(String, String)> {
        self.0
            .resident_at(position)
            .map(|held| &held.ffi)
            .filter(|row| row.kind != FocusRowKindFfi::Digest)
            .map(|row| (row.sender.clone(), row.subject.clone()))
    }
    fn row(&self, position: u32) -> Option<FocusRow> {
        self.0.resident_at(position).map(|held| held.row.clone())
    }
}

pub(crate) struct FocusDriver {
    focus: Mutex<FocusController>,
    list: Mutex<ListWindow<HeldRow>>,
    client: Client,
    runtime: tokio::runtime::Handle,
    local: async_channel::Sender<UiEvent>,
    /// Pages read from the store, so a test can count what scrolling cost.
    page_reads: std::sync::atomic::AtomicUsize,
    /// Conversation searches that ran to the end, so a test can count what typing quickly cost.
    search_reads: std::sync::atomic::AtomicUsize,
    /// Each search lane's task in flight: a newer request aborts it, which drops the client's call,
    /// and the host stops the read when its reply channel closes.
    lanes: Mutex<std::collections::HashMap<postio_focus::Lane, tokio::task::AbortHandle>>,
    config_path: Mutex<Option<std::path::PathBuf>>,
    /// The thread open in place, read by the table through display positions. A leaf lock: nothing
    /// is taken while held.
    expanded: Mutex<Option<Expanded>>,
    /// The account's own addresses: what "you" in a thread row is decided
    /// by (`focus_row::participants`). Read once the driver starts.
    me: Mutex<Vec<postio_model::EmailAddress>>,
}

struct Expanded {
    anchor: u32,
    children: Vec<ChildRowFfi>,
}

impl FocusDriver {
    pub(crate) fn new(
        client: Client,
        runtime: tokio::runtime::Handle,
        local: async_channel::Sender<UiEvent>,
    ) -> Arc<Self> {
        Arc::new(FocusDriver {
            focus: Mutex::new(FocusController::new(Policy::for_platform(crate::PLATFORM))),
            list: Mutex::new(ListWindow::new()),
            client,
            runtime,
            local,
            page_reads: std::sync::atomic::AtomicUsize::new(0),
            search_reads: std::sync::atomic::AtomicUsize::new(0),
            lanes: Mutex::new(std::collections::HashMap::new()),
            config_path: Mutex::new(None),
            expanded: Mutex::new(None),
            me: Mutex::new(Vec::new()),
        })
        .started()
    }

    fn started(self: Arc<Self>) -> Arc<Self> {
        let driver = Arc::clone(&self);
        self.runtime.spawn(async move {
            if let Ok(accounts) = driver.client.accounts().await {
                let mut me = Vec::new();
                for account in &accounts {
                    me.push(account.address.clone());
                    me.extend(account.identities.iter().map(|id| id.address.clone()));
                }
                *driver.me.lock().expect("me lock") = me;
            }
        });
        self
    }

    fn splice(&self) -> Option<Splice> {
        self.expanded
            .lock()
            .expect("expanded lock")
            .as_ref()
            .map(|open| Splice {
                anchor: open.anchor,
                children: open.children.len() as u32,
            })
    }

    fn present(&self, held: &HeldRow, position: u32) -> FocusRowFfi {
        let mut ffi = held.ffi.clone();
        let (FocusRow::Conversation(conversation)
        | FocusRow::Reminder {
            row: conversation, ..
        }) = &held.row
        else {
            return ffi;
        };
        if conversation.summary.message_count < 2 {
            return ffi;
        }
        let open = self
            .splice()
            .is_some_and(|splice| splice.anchor == position);
        let me = self.me.lock().expect("me lock");
        let view = focus_row::ThreadRowView::new(
            &conversation.summary,
            &conversation.labels,
            &me,
            open,
            postio_ui::clock::now(),
        );
        ffi.thread_row = Some(ThreadRowFfi::from(&view));
        ffi
    }

    /// The child at `index` under the row at `anchor`, drawn as a table row:
    /// on the anchor's day, so no heading is drawn between them.
    fn child_row(&self, anchor: u32, index: u32) -> Option<FocusRowFfi> {
        let child = self
            .expanded
            .lock()
            .expect("expanded lock")
            .as_ref()?
            .children
            .get(index as usize)?
            .clone();
        let (day_heading, thread, id) = self
            .list
            .lock()
            .expect("list lock")
            .resident_at(anchor)
            .map(|held| (held.ffi.day_heading.clone(), held.ffi.thread, held.ffi.id))
            .unwrap_or_default();
        Some(FocusRowFfi {
            kind: FocusRowKindFfi::Child,
            id,
            thread,
            threads: Vec::new(),
            sender: child.sender.clone(),
            from_address: None,
            subject: String::new(),
            preview: Some(child.line.clone()),
            time: child.time.clone(),
            day_heading,
            unread: child.bold,
            count_badge: None,
            has_attachments: child.attachment,
            send_state: None,
            pills: Vec::new(),
            marker: None,
            writes: false,
            thread_row: None,
            child: Some(child),
        })
    }

    /// The Mac's chevron click on `position` (a display position): the cursor goes to the row and
    /// the thread opens, or folds when open.
    pub(crate) fn chevron_clicked(self: &Arc<Self>, position: u32) {
        self.input(Input::Point(position));
        let open = self.splice().is_some_and(
            |splice| matches!(splice.slot(position), Slot::Row(row) if row == splice.anchor),
        );
        self.command(if open {
            postio_core::CommandId::CollapseThread
        } else {
            postio_core::CommandId::ExpandThread
        });
    }

    pub(crate) fn open(self: &Arc<Self>, scope: ListScope) {
        let effects = self.focus.lock().expect("focus lock").open(scope);
        self.apply(effects);
    }

    /// Run `id` through the controller when it is one of its own, and say whether it was.
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

    /// `input` over the list as it is held now: what an open thread needs to
    /// read the rows it sits among (`handle` would give it none).
    fn handle_over_rows(&self, input: Input) -> Vec<Effect> {
        let list = self.list.lock().expect("list lock");
        self.focus
            .lock()
            .expect("focus lock")
            .handle_on(input, &RowsView(&list))
    }

    /// Something the frontend reports that is not a command, applied over the list as held now.
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

    pub(crate) fn bar_tab(self: &Arc<Self>) -> bool {
        let effects = self.focus.lock().expect("focus lock").handle(Input::BarTab);
        let used = !effects.is_empty();
        self.apply(effects);
        used
    }

    pub(crate) fn press(
        &self,
        chord: &postio_ui::keymap::Chord,
        context: postio_ui::keymap::KeyContext,
        in_text_entry: bool,
        now: std::time::Instant,
    ) -> postio_ui::keymap::Outcome {
        self.focus
            .lock()
            .expect("focus lock")
            .press(chord, context, in_text_entry, now)
    }

    pub(crate) fn keymap(&self) -> postio_core::Keymap {
        self.focus.lock().expect("focus lock").keymap().clone()
    }

    pub(crate) fn places(&self, filter: &str) -> Vec<(u64, postio_ui::places::Entry)> {
        self.focus.lock().expect("focus lock").places(filter)
    }

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
        (focus.has_surface() || focus.in_results()).then(|| focus.key_context())
    }

    pub(crate) fn result_count(&self) -> u64 {
        self.focus.lock().expect("focus lock").result_count()
    }

    pub(crate) fn result_person(&self, position: u64) -> Option<postio_focus::PersonRow> {
        self.focus
            .lock()
            .expect("focus lock")
            .result_person(position)
    }

    /// The Files tab's card at `position`; `None` past the last.
    pub(crate) fn result_file(&self, position: u64) -> Option<postio_focus::FileCard> {
        self.focus.lock().expect("focus lock").result_file(position)
    }

    /// The result at `position`, or `None` while its page is on its way: a miss asks for the page,
    /// and `FocusResultsPage` says when.
    pub(crate) fn result_row(self: &Arc<Self>, position: u64) -> Option<postio_focus::ResultRow> {
        let effects = {
            let mut focus = self.focus.lock().expect("focus lock");
            if let Some(row) = focus.result_row(position) {
                return Some(row);
            }
            focus.results_wanted(position)
        };
        self.apply(effects);
        None
    }

    pub(crate) fn contacts_row(
        self: &Arc<Self>,
        index: u32,
    ) -> Option<postio_ui::contacts_view::RowView> {
        let effects = {
            let mut focus = self.focus.lock().expect("focus lock");
            if let Some(row) = focus.contacts_row(index) {
                return Some(row);
            }
            focus.contacts_wanted(index)
        };
        self.apply(effects);
        None
    }

    pub(crate) fn contacts_total(&self) -> u32 {
        self.focus.lock().expect("focus lock").contacts_total()
    }

    pub(crate) fn contacts_cursor(&self) -> Option<u32> {
        self.focus.lock().expect("focus lock").contacts_cursor()
    }

    pub(crate) fn contacts_card(&self) -> Option<postio_ui::contacts_view::CardView> {
        self.focus.lock().expect("focus lock").contacts_card()
    }

    pub(crate) fn contacts_settings(
        &self,
    ) -> Option<postio_ui::contacts_settings::SuggestionsView> {
        self.focus.lock().expect("focus lock").contacts_settings()
    }

    pub(crate) fn contact_edit(&self) -> Option<postio_ui::contact_edit::EditView> {
        self.focus.lock().expect("focus lock").contact_edit()
    }

    pub(crate) fn cursor_message(&self) -> Option<postio_model::MessageId> {
        // The list, then the controller: the one order every path takes
        // both in, so two threads can never hold one each.
        let list = self.list.lock().expect("list lock");
        let position = self.focus.lock().expect("focus lock").cursor()?;
        // The controller's cursor counts the children; a verb the list
        // aims acts on the thread's row (the message-only verbs are the
        // controller's, and aim themselves).
        let row = match thread_list::slot(self.splice(), position) {
            Slot::Row(row) | Slot::Child { row, .. } => row,
        };
        list.resident_at(row)
            .map(|held| &held.ffi)
            .filter(|row| row.kind != FocusRowKindFfi::Digest)
            .map(|row| postio_model::MessageId::new(row.id))
    }

    pub(crate) fn scope(&self) -> Option<ListScope> {
        self.focus.lock().expect("focus lock").scope()
    }

    /// The list as a source of facts about its rows, held only while a verb is aimed.
    pub(crate) fn rows(&self) -> std::sync::MutexGuard<'_, ListWindow<HeldRow>> {
        self.list.lock().expect("list lock")
    }

    #[cfg(feature = "testing")]
    pub(crate) fn page_reads(&self) -> usize {
        self.page_reads.load(std::sync::atomic::Ordering::Relaxed)
    }

    #[cfg(feature = "testing")]
    pub(crate) fn search_reads(&self) -> usize {
        self.search_reads.load(std::sync::atomic::Ordering::Relaxed)
    }

    #[cfg(feature = "testing")]
    pub(crate) fn resident_rows(&self) -> usize {
        self.list.lock().expect("list lock").resident_rows()
    }

    pub(crate) fn row_count(&self) -> u32 {
        let rows = self.list.lock().expect("list lock").total();
        thread_list::len(self.splice(), rows)
    }

    /// The row at `position`, or `None` while its page is on its way: a miss asks for the page, and
    /// `FocusPageReady` says when.
    /// Synchronous. The row standing for `message`, if its page is resident; reads nothing.
    pub(crate) fn row_of(&self, message: i64) -> Option<FocusRowFfi> {
        let list = self.list.lock().expect("list lock");
        let position = list.position_of(postio_model::MessageId::new(message))?;
        list.resident_at(position).map(|held| held.ffi.clone())
    }

    /// The row at `position` if its page is resident, else `None`: never asks for the page.
    pub(crate) fn row_resident(&self, position: u32) -> Option<FocusRowFfi> {
        match thread_list::slot(self.splice(), position) {
            Slot::Row(row) => {
                let list = self.list.lock().expect("list lock");
                list.resident_at(row).map(|held| self.present(held, row))
            }
            Slot::Child { row, index } => self.child_row(row, index),
        }
    }

    pub(crate) fn row_at(self: &Arc<Self>, position: u32) -> Option<FocusRowFfi> {
        let row = match thread_list::slot(self.splice(), position) {
            Slot::Row(row) => row,
            Slot::Child { row, index } => return self.child_row(row, index),
        };
        let (wanted, stamp) = {
            let mut list = self.list.lock().expect("list lock");
            let stamp = list.generation();
            match list.row_at(row)? {
                Lookup::Resident(held) => return Some(self.present(held, row)),
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

    pub(crate) fn event(self: &Arc<Self>, event: &Event) {
        let effects = self.handle_over_rows(Input::Event(event.clone()));
        self.apply(effects);
    }

    /// Do what the controller said, in order. Never called with a lock held.
    fn apply(self: &Arc<Self>, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Show(intent) => self.show(intent),
                Effect::Ask(ticket, request) => self.ask(ticket, request),
                Effect::Timer { token, after } => self.timer(token, after),
                _ => {}
            }
        }
    }

    fn show(self: &Arc<Self>, intent: Intent) {
        match intent {
            Intent::ReplaceSource { total } => {
                let open = self.expanded.lock().expect("expanded lock").take();
                if let Some(open) = open {
                    self.say(UiEvent::FocusUnsplice {
                        anchor: open.anchor,
                    });
                }
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
                let rows = rows.into_iter().map(HeldRow::of).collect();
                let (delivered, resized) = {
                    let mut list = self.list.lock().expect("list lock");
                    let resized = (list.generation() == stamp)
                        .then(|| list.set_total(total))
                        .flatten()
                        .is_some();
                    (list.deliver(stamp, page, rows), resized)
                };
                if resized {
                    self.say(UiEvent::FocusListChanged {
                        total: thread_list::len(self.splice(), total),
                    });
                }
                if !delivered.stale {
                    self.say(UiEvent::FocusPageReady { page });
                }
            }
            Intent::AbandonPage { stamp, page } | Intent::GiveUp { stamp, page } => {
                self.list.lock().expect("list lock").abandon(stamp, page);
            }
            Intent::RefreshList => {
                // Re-read the pages held in place: each keeps drawing until its page returns;
                // blanking them first made the list collapse and regrow on every delete.
                let (held, stamp) = {
                    let mut list = self.list.lock().expect("list lock");
                    let held: Vec<u32> = list
                        .resident_pages()
                        .into_iter()
                        .filter(|page| list.note_pending(*page))
                        .collect();
                    (held, list.generation())
                };
                if held.is_empty() {
                    let total = {
                        let mut list = self.list.lock().expect("list lock");
                        list.invalidate();
                        list.total()
                    };
                    self.say(UiEvent::FocusListChanged {
                        total: thread_list::len(self.splice(), total),
                    });
                    return;
                }
                for page in held {
                    let effects = self
                        .focus
                        .lock()
                        .expect("focus lock")
                        .page_wanted(page, stamp);
                    self.apply(effects);
                }
            }
            Intent::Filled => {
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
            Intent::Splice { anchor, children } => {
                let children: Vec<ChildRowFfi> = children.iter().map(ChildRowFfi::from).collect();
                *self.expanded.lock().expect("expanded lock") = Some(Expanded {
                    anchor,
                    children: children.clone(),
                });
                self.say(UiEvent::FocusSplice { anchor, children });
            }
            Intent::Footer(view) => self.say(UiEvent::FocusFooter { view: view.into() }),
            Intent::Unsplice { anchor } => {
                *self.expanded.lock().expect("expanded lock") = None;
                self.say(UiEvent::FocusUnsplice { anchor });
            }
            Intent::SingleHeading(text) => self.say(UiEvent::FocusHeading { text }),
            Intent::OpenMessage {
                message,
                index,
                total,
                host: postio_focus::Host::Own,
            } => self.say(UiEvent::FocusOpenMessage {
                message: message.get(),
                index,
                total,
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
            // The Mac's message window draws the raw source on this verb; the surface's tabs and
            // the list's V are its own to build (spec 016 T024).
            Intent::OpenSource {
                over: postio_focus::Over::Message,
                ..
            } => self.say(UiEvent::FocusReader {
                verb: ReaderVerbFfi::ViewSource,
            }),
            Intent::ThreadBar(view) => self.say(UiEvent::FocusThreadBar { view: view.into() }),
            Intent::Kicker(text) => self.say(UiEvent::FocusKicker { text }),
            Intent::OlderBanner(banner) => self.say(UiEvent::FocusOlderBanner {
                banner: banner.map(Into::into),
            }),
            Intent::NewReply(banner) => self.say(UiEvent::FocusNewReply {
                banner: banner.map(Into::into),
            }),
            Intent::ThreadMenu(view) => self.say(UiEvent::FocusThreadMenu {
                view: (*view).into(),
            }),
            Intent::Outline(view) => self.say(UiEvent::FocusOutline {
                view: (*view).into(),
            }),
            Intent::OutlineFocusFilter => self.say(UiEvent::FocusOutlineFocusFilter),
            Intent::ReplyTo(text) => self.say(UiEvent::FocusReplyTo { text }),
            Intent::KeyboardHome => self.say(UiEvent::FocusKeyboardHome),
            Intent::ListToTop => self.say(UiEvent::FocusListToTop),
            Intent::Toast {
                text,
                kind: postio_focus::ToastKind::Offer { label, command },
            } => self.say(UiEvent::FocusOffer {
                text,
                label,
                command: command.to_string(),
            }),
            Intent::Toast { text, kind } => {
                let seconds = Some(kind.seconds());
                let (kind, undoable) = match kind {
                    postio_focus::ToastKind::Completed { undoable, .. } => {
                        (crate::event::ToastKindFfi::Completed, undoable)
                    }
                    postio_focus::ToastKind::Undone => (crate::event::ToastKindFfi::Undone, false),
                    _ => (crate::event::ToastKindFfi::Notice, false),
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
            Intent::Dropdown(view) => self.say(UiEvent::FocusDropdown { view: view.into() }),
            Intent::ShowResults(view) => self.say(UiEvent::FocusResults {
                view: (*view).into(),
            }),
            Intent::ResultsPage { first, count } => {
                self.say(UiEvent::FocusResultsPage { first, count });
            }
            Intent::ResultsCursor(position) => {
                self.say(UiEvent::FocusResultsCursor { position });
            }
            Intent::ResultSplice(splice) => self.say(UiEvent::FocusResultSplice {
                splice: splice.map(|splice| (&*splice).into()),
            }),
            Intent::ResultChildFocus(child) => self.say(UiEvent::FocusResultChildFocus { child }),
            Intent::MatchBanner(banner) => self.say(UiEvent::FocusMatchBanner {
                banner: banner.map(Into::into),
            }),
            Intent::Query(view) => self.say(UiEvent::FocusQuery { view: view.into() }),
            Intent::LeaveResults => self.say(UiEvent::FocusLeaveResults),
            Intent::Popover(view) => self.say(UiEvent::FocusPopover {
                view: view.map(|view| (*view).into()),
            }),
            Intent::QuickLook(view) => self.say(UiEvent::FocusQuickLook {
                view: view.map(|view| (*view).into()),
            }),
            Intent::SavePopover(view) => self.say(UiEvent::FocusSavePopover {
                view: view.map(|view| (*view).into()),
            }),
            Intent::Relaxations(view) => self.say(UiEvent::FocusRelaxations {
                view: view.map(|view| (*view).into()),
            }),
            Intent::FileCopy(copy) => self.say(UiEvent::FocusFileCopy {
                copy: copy.map(Into::into),
            }),
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
            Intent::OpenKeyMap => {
                let sheet = crate::focus_keymap::sheet(&self.keymap());
                self.say(UiEvent::FocusOpenKeyMap { sheet });
            }
            // The digest's window draws its own email via `FocusDigest`, so a message hosted there
            // has no event.
            Intent::OpenMessage {
                host: postio_focus::Host::Digest,
                ..
            } => {}
            Intent::Filtered(view) => self.say(UiEvent::FocusFiltered {
                view: (*view).into(),
            }),
            Intent::FilteredSplice { anchor, children } => self.say(UiEvent::FocusFilteredSplice {
                anchor,
                children: children.iter().map(ChildRowFfi::from).collect(),
            }),
            Intent::FilteredUnsplice { anchor } => {
                self.say(UiEvent::FocusFilteredUnsplice { anchor });
            }
            Intent::FilteredFocus(index) => self.say(UiEvent::FocusFilteredFocus { index }),
            Intent::FilteredSelection { selected, bar } => {
                self.say(UiEvent::FocusFilteredSelection {
                    selected: selected.iter().map(|id| id.get()).collect(),
                    bar: bar.map(Into::into),
                });
            }
            Intent::Digest(view) => self.say(UiEvent::FocusDigest {
                view: (*view).into(),
            }),
            Intent::Confirm(confirm) => self.say(UiEvent::FocusConfirm {
                confirm: confirm.into(),
            }),
            Intent::OpenRule(view) => self.say(UiEvent::FocusOpenRule {
                view: (*view).into(),
            }),
            Intent::Rule(view) => self.say(UiEvent::FocusRule {
                view: (*view).into(),
            }),
            Intent::OpenCapture(view) => self.say(UiEvent::FocusOpenCapture {
                view: (*view).into(),
            }),
            Intent::Capture(view) => self.say(UiEvent::FocusCapture {
                view: (*view).into(),
            }),
            Intent::ContactPopover(view) => self.say(UiEvent::FocusContactPopover {
                view: (*view).into(),
            }),
            Intent::WriteTo(address) => self.say(UiEvent::FocusWriteTo { address }),
            Intent::Contacts(view) => self.say(UiEvent::FocusContacts {
                view: (*view).into(),
            }),
            Intent::ContactsRows { first, count } => {
                self.say(UiEvent::FocusContactsRows { first, count });
            }
            Intent::ContactsCursor(index) => self.say(UiEvent::FocusContactsCursor { index }),
            Intent::ContactsCard(card) => self.say(UiEvent::FocusContactsCard {
                card: card.map(|card| (*card).into()),
            }),
            Intent::ContactMerge(view) => self.say(UiEvent::FocusContactMerge {
                view: view.map(|view| (*view).into()),
            }),
            Intent::ContactEdit(view) => self.say(UiEvent::FocusContactEdit {
                view: view.map(|view| (*view).into()),
            }),
            Intent::ContactsSettings(view) => self.say(UiEvent::FocusContactsSettings {
                view: (*view).into(),
            }),
            Intent::ContactsFocusFilter => self.say(UiEvent::FocusContactsFocusFilter),
            Intent::Composer { kind, message } => {
                // A thread's unsent reply has no message: its own id rides in `message`.
                let message = match kind {
                    postio_focus::ComposerKind::ThreadDraft { draft } => Some(draft.get()),
                    _ => message.map(|message| message.get()),
                };
                if let Some(kind) = crate::focus_compose::ComposerKindFfi::of(kind) {
                    self.say(UiEvent::FocusComposer { kind, message });
                }
            }
            Intent::ForwardChooser(view) => self.say(UiEvent::FocusForwardChooser {
                view: (*view).into(),
            }),
            Intent::OlderReply(view) => self.say(UiEvent::FocusOlderReply {
                view: view.map(|view| (*view).into()),
            }),
            Intent::NewerMessages(ids) => self.say(UiEvent::FocusNewerMessages {
                ids: ids.map(|ids| ids.into_iter().map(|id| id.get()).collect()),
            }),
            Intent::QuotedExpanded(expanded) => {
                self.say(UiEvent::FocusQuotedExpanded { expanded });
            }
            Intent::ComposerDraftWanted(edit) => {
                if let Some(edit) = crate::focus_compose::DraftEditFfi::of(edit) {
                    self.say(UiEvent::FocusComposerDraftWanted { edit });
                }
            }
            Intent::ComposerDraft(draft) => self.say(UiEvent::FocusComposerDraft {
                draft: crate::compose::to_ffi(&draft, String::new(), String::new()),
            }),
            Intent::SaveDraft { composition } => {
                self.say(UiEvent::FocusSaveDraft { composition });
            }
            Intent::SaveSearch(save) => {
                let saved = self.save_search(&save);
                self.input(Input::SearchSaved(saved));
            }
            _ => {}
        }
    }

    fn save_search(
        &self,
        save: &postio_focus::SaveSearch,
    ) -> Result<postio_focus::SearchesSaved, String> {
        let path = self.config_path.lock().expect("config path lock").clone();
        postio_focus::save_search(path.as_deref(), save)
    }

    fn ask(self: &Arc<Self>, ticket: Ticket, request: Request) {
        // A post is said now, before anything after it is asked.
        let request = match postio_focus::perform_now(&self.client, request) {
            Ok(reply) => {
                let effects = self.handle_over_rows(Input::Reply(ticket, reply));
                self.apply(effects);
                return;
            }
            Err(request) => request,
        };
        if matches!(request, Request::Page { .. }) {
            self.page_reads
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        let searches = matches!(request, Request::Conversations { .. });
        let lane = request.lane();
        let driver = Arc::clone(self);
        let task = self.runtime.spawn(async move {
            let reply = postio_focus::perform(&driver.client, request).await;
            if searches {
                driver
                    .search_reads
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            let effects = driver.handle_over_rows(Input::Reply(ticket, reply));
            driver.apply(effects);
        });
        // The stamps keep a superseded answer off the screen; this keeps
        // it from costing the host a read nobody will draw.
        if let Some(lane) = lane
            && let Some(before) = self
                .lanes
                .lock()
                .expect("lanes lock")
                .insert(lane, task.abort_handle())
        {
            before.abort();
        }
    }

    /// Hand the controller `Input::Timer(token)` once `after` has passed (the composer's autosave).
    /// A superseded token changes nothing, so no timer is cancelled.
    fn timer(self: &Arc<Self>, token: u64, after: std::time::Duration) {
        let driver = Arc::clone(self);
        self.runtime.spawn(async move {
            tokio::time::sleep(after).await;
            driver.input(Input::Timer(token));
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
