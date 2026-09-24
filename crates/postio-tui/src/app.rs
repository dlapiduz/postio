//! The terminal frontend's state, and the one function that changes it.
//!
//! `update` takes an [`Input`] and returns the [`Effect`]s it asks for --
//! a request of the daemon, a redraw, quitting -- and does no I/O itself.
//! That is what lets every behaviour be driven by synthetic input in a test,
//! with no terminal and no daemon (research R11).
//!
//! # The list is a window
//!
//! A mailbox is never loaded (Principle V). The list is
//! `postio_ui::list::ListWindow`, the desktop app's and the macOS frontend's
//! own window, paged by `postio_ui::paging::Paging`; after every input,
//! [`App`] walks only the rows in view and asks for whichever of their pages
//! is not already here or on its way.

use crossterm::event::KeyEvent;
use postio_model::ListScope;
use postio_ui::keymap::{KeyContext, Outcome};
use postio_ui::list::ListWindow;
use postio_ui::paging::{Fetch, Page, Paging};

use crate::input::Keys;
use crate::layout::{self, Requested, Shown};
use crate::row::Row;
use crate::view::list::Visible;

/// Something that happened, from the terminal or from the daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// The terminal is now this many columns and rows.
    Resize(u16, u16),
    /// A key was pressed.
    Key(KeyEvent),
    /// A list was opened, and has this many rows.
    Opened {
        /// What the list shows.
        scope: ListScope,
        /// How many rows it has.
        total: u32,
    },
    /// The daemon said something happened.
    Host(postio_core::Event),
    /// The cursor has rested on `message` since [`Effect::Rest`] asked.
    Rested(postio_model::MessageId),
    /// A conversation asked for by [`Effect::ReadConversation`] arrived.
    Conversation {
        /// Which.
        thread: postio_model::ThreadId,
        /// Its messages, oldest first, or why there are none.
        members: Result<Vec<postio_model::listing::MessageSummary>, String>,
    },
    /// A body asked for by [`Effect::ReadBody`] arrived.
    Body {
        /// Whose.
        message: postio_model::MessageId,
        /// The body, or why there is none.
        answer: Result<postio_client::protocol::Body, String>,
    },
    /// What the sidebar holds, read afresh.
    Sidebar(crate::sidebar::Contents),
    /// A list was counted again, after an event said it changed.
    Recounted {
        /// Which list.
        scope: ListScope,
        /// How many rows it has now.
        total: u32,
    },
    /// A page asked for by [`Effect::Fetch`] arrived, or failed.
    Page {
        /// The list's generation when it was asked for.
        generation: u64,
        /// Which page.
        page: u32,
        /// The rows, or why there are none.
        rows: Result<Page<Row>, String>,
    },
}

/// Something `update` asks the loop to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Draw a frame.
    Redraw,
    /// Leave.
    Quit,
    /// Wait [`READ_REST`], then answer with [`Input::Rested`].
    Rest(postio_model::MessageId),
    /// Read a conversation and answer with [`Input::Conversation`].
    ReadConversation(postio_model::ThreadId),
    /// Read a body and answer with [`Input::Body`].
    ReadBody(postio_model::MessageId),
    /// Open a list: count it and answer with [`Input::Opened`].
    Open(ListScope),
    /// Read the sidebar's contents again and answer with [`Input::Sidebar`].
    RefreshSidebar,
    /// Count a list again and answer with [`Input::Recounted`].
    Recount(ListScope),
    /// Send a command to the daemon, aimed with [`App::state`].
    Send(postio_core::Command),
    /// Read a page of the list and answer with [`Input::Page`].
    Fetch {
        /// The list's generation now; a page for an older one is dropped.
        generation: u64,
        /// Which page.
        page: u32,
        /// What to read.
        fetch: Fetch,
    },
}

/// How long the cursor must rest on a row before its body is read.
///
/// A keystroke reads no body -- `j` held down is a scroll, and reading each
/// row it passes would be a body per keystroke for mail nobody looked at
/// (Principle V). Short enough that stopping on a row reads as immediate.
pub const READ_REST: std::time::Duration = std::time::Duration::from_millis(120);

/// Everything the terminal frontend knows.
pub struct App {
    size: (u16, u16),
    requested: Requested,
    keys: Keys,
    list: ListWindow<Row>,
    paging: Paging,
    /// The row the keyboard is on.
    cursor: u32,
    /// The first row in view.
    top: u32,
    /// What the host aims this frontend's commands with; the client sends
    /// a snapshot of it with each one (ADR 0041).
    state: postio_core::SharedState,
    /// What is marked.
    selection: postio_ui::selection::SelectionState,
    /// The list being shown.
    scope: Option<ListScope>,
    /// What the status line says about the last thing done: the undo offer,
    /// a refusal, an error.
    notice: Option<String>,
    /// Where the keyboard is.
    focus: Focus,
    /// The sidebar's lines.
    sidebar: Vec<crate::sidebar::Line>,
    /// The sidebar line the keyboard is on.
    sidebar_cursor: usize,
    /// Each account's sync status, folded from the daemon's events.
    trackers: postio_ui::status::Trackers,
    /// Which account the list on screen belongs to.
    account: Option<postio_model::AccountId>,
    /// Every folder, to find the account a list belongs to.
    folders: Vec<postio_model::mailbox::Mailbox>,
    /// The message the reader shows, and how.
    reading: Option<crate::conversation::Reading>,
    /// The message the cursor was last seen resting towards.
    resting: Option<postio_model::MessageId>,
    /// The first reader line in view.
    reader_top: usize,
}

/// Which pane the keyboard is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    /// The message list.
    #[default]
    List,
    /// The sidebar.
    Sidebar,
    /// The reading pane.
    Reader,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("size", &self.size)
            .field("cursor", &self.cursor)
            .field("total", &self.list.total())
            .finish_non_exhaustive()
    }
}

impl App {
    /// A frontend in a terminal of `size`, resolving keys with `keys`.
    pub fn new(size: (u16, u16), keys: Keys) -> App {
        App {
            size,
            requested: Requested::default(),
            keys,
            list: ListWindow::new(),
            paging: Paging::default(),
            cursor: 0,
            top: 0,
            state: postio_core::SharedState::default(),
            selection: postio_ui::selection::SelectionState::new(),
            scope: None,
            notice: None,
            focus: Focus::List,
            sidebar: Vec::new(),
            sidebar_cursor: 0,
            trackers: postio_ui::status::Trackers::default(),
            account: None,
            folders: Vec::new(),
            reading: None,
            resting: None,
            reader_top: 0,
        }
    }

    /// The first reader line in view.
    pub fn reader_top(&self) -> usize {
        self.reader_top
    }

    /// The row for `message`, if it is resident.
    pub fn row(&self, message: postio_model::MessageId) -> Option<&Row> {
        self.list.row_of(message)
    }

    /// What the reader shows: the message, and its rendered body.
    pub fn reading(&self) -> Option<&crate::conversation::Reading> {
        self.reading.as_ref()
    }

    /// What the status line says about the connection of the account on
    /// screen: `offline`, `syncing`, `idle` -- the desktop's own words.
    pub fn sync_line(&self) -> Option<String> {
        let account = self.account?;
        let (state, detail) = self
            .trackers
            .status(account)
            .lines(std::time::Instant::now());
        Some(format!("{state} · {detail}"))
    }

    /// Which account `scope` belongs to.
    fn account_of(&self, scope: ListScope) -> Option<postio_model::AccountId> {
        match scope {
            ListScope::Mailbox(mailbox) => self
                .folders
                .iter()
                .find(|folder| folder.id == mailbox)
                .map(|folder| folder.account_id),
            ListScope::Account(account)
            | ListScope::Flagged(account)
            | ListScope::Snoozed(account)
            | ListScope::Outbox(account) => Some(account),
            ListScope::Unified | ListScope::Thread(_) => None,
        }
    }

    /// Where the keyboard is.
    pub fn focus(&self) -> Focus {
        self.focus
    }

    /// The sidebar's lines, and which one the keyboard would be on.
    pub fn sidebar(&self) -> (&[crate::sidebar::Line], usize) {
        (&self.sidebar, self.sidebar_cursor)
    }

    /// The keymap context the keyboard is in.
    fn key_context(&self) -> KeyContext {
        match self.focus {
            Focus::List => KeyContext::List,
            Focus::Sidebar => KeyContext::Sidebar,
            // The conversation, as the desktop reading pane is: where `J`/`K`
            // walk messages and `O` expands.
            Focus::Reader => KeyContext::Conversation,
        }
    }

    /// What the status line says about the last thing done.
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// Put `sentence` on the status line. Through `SafeText`: these are
    /// Postio's own sentences, but an error can quote a folder name, and a
    /// folder name came from a server.
    fn say(&mut self, sentence: &str) -> Vec<Effect> {
        self.notice = Some(postio_ui::terminal::SafeText::new(sentence).to_string());
        vec![Effect::Redraw]
    }

    /// The same app, mirroring into `state`: the one the client snapshots.
    pub fn with_state(mut self, state: postio_core::SharedState) -> App {
        self.state = state;
        self
    }

    /// The state the client snapshots with each command.
    pub fn state(&self) -> postio_core::SharedState {
        self.state.clone()
    }

    /// What the user asked to see.
    pub fn requested(&self) -> Requested {
        self.requested
    }

    /// What the terminal has room for.
    pub fn shown(&self) -> Shown {
        layout::shown(self.size.0, self.size.1, self.requested)
    }

    /// The row the keyboard is on.
    pub fn cursor(&self) -> u32 {
        self.cursor
    }

    /// How many rows the list has.
    pub fn total(&self) -> u32 {
        self.list.total()
    }

    /// How many list rows fit: everything but the status line.
    pub fn list_height(&self) -> u32 {
        u32::from(self.size.1.saturating_sub(1))
    }

    /// The rows in view, for drawing. Reads only what is resident.
    pub fn visible(&self) -> Vec<Visible<'_>> {
        let end = (self.top + self.list_height()).min(self.list.total());
        (self.top..end)
            .map(|position| Visible {
                row: self
                    .list
                    .peek(position)
                    .and_then(|message| self.list.row_of(message)),
                cursor: position == self.cursor,
                selected: self
                    .list
                    .peek(position)
                    .is_some_and(|message| self.selection.contains(message)),
            })
            .collect()
    }

    /// Move the cursor to `position`, keeping it in view.
    fn move_to(&mut self, position: u32) {
        let last = self.list.total().saturating_sub(1);
        self.cursor = position.min(last);
        let height = self.list_height().max(1);
        if self.cursor < self.top {
            self.top = self.cursor;
        } else if self.cursor >= self.top + height {
            self.top = self.cursor + 1 - height;
        }
    }

    /// Ask for every page in view that is neither here nor on its way.
    fn fetches(&mut self) -> Vec<Effect> {
        let end = (self.top + self.list_height()).min(self.list.total());
        let mut wanted = Vec::new();
        for position in self.top..end {
            if let Some(postio_ui::list::Lookup::Missing { request }) = self.list.row_at(position) {
                wanted.extend(request);
            }
        }
        let generation = self.list.generation();
        wanted
            .into_iter()
            .filter_map(|page| {
                self.paging.fetch_for(page).map(|fetch| Effect::Fetch {
                    generation,
                    page,
                    fetch,
                })
            })
            .collect()
    }

    /// The message the cursor is on, if its row is here.
    fn cursor_message(&self) -> Option<postio_model::MessageId> {
        self.list.peek(self.cursor)
    }

    /// Run a command the keymap resolved to.
    ///
    /// Moving and marking are this frontend's own: they change what is on
    /// screen and nothing in the store. Everything else is aimed by
    /// `postio_core::aim` -- the rule every frontend shares for what a verb
    /// acts on -- mirrored into [`App::state`], and sent.
    fn command(&mut self, id: &str) -> Vec<Effect> {
        let last = self.list.total().saturating_sub(1);
        match id {
            "next_message" => self.move_to(self.cursor.saturating_add(1)),
            "prev_message" => self.move_to(self.cursor.saturating_sub(1)),
            "first_message" => self.move_to(0),
            "last_message" => self.move_to(last),
            "toggle_selection" => {
                if let Some(message) = self.cursor_message() {
                    self.selection.toggle(message);
                }
            }
            "extend_selection_down" | "extend_selection_up" => {
                let anchor = self.cursor_message();
                if self.selection.anchor().is_none()
                    && let Some(anchor) = anchor
                {
                    self.selection.select_only(anchor);
                }
                let next = if id == "extend_selection_down" {
                    self.cursor.saturating_add(1)
                } else {
                    self.cursor.saturating_sub(1)
                };
                self.move_to(next);
                if let Some(message) = self.cursor_message() {
                    self.selection.extend_to(message);
                }
            }
            "select_all" => self
                .selection
                .select_all(postio_ui::selection::Reach::default()),
            "quit" => return vec![Effect::Quit],
            "focus_sidebar" => self.focus = Focus::Sidebar,
            "cycle_pane" => {
                self.focus = match self.focus {
                    Focus::List => Focus::Reader,
                    Focus::Reader => Focus::Sidebar,
                    Focus::Sidebar => Focus::List,
                }
            }
            "cycle_pane_back" => {
                self.focus = match self.focus {
                    Focus::List => Focus::Sidebar,
                    Focus::Sidebar => Focus::Reader,
                    Focus::Reader => Focus::List,
                }
            }
            "back" if self.focus != Focus::List => self.focus = Focus::List,
            "expand_all" => self.toggle_folds(),
            "next_in_conversation" => self.walk_conversation(1),
            "prev_in_conversation" => self.walk_conversation(-1),
            "scroll_reader_down" => self.scroll_reader(1),
            "scroll_reader_up" => self.scroll_reader(-1),
            "back" => self.selection.clear(),
            "next_folder" => return self.walk_sidebar(1),
            "prev_folder" => return self.walk_sidebar(-1),
            other => return self.send(other),
        }
        vec![Effect::Redraw]
    }

    /// Aim a verb at what the user is looking at, and send it.
    fn send(&mut self, id: &str) -> Vec<Effect> {
        let Ok(id) = id.parse::<postio_core::CommandId>() else {
            return Vec::new();
        };
        let selection = self.selection.selection();
        let aim = postio_core::aim::Aim {
            scope: self
                .scope
                .and_then(|scope| postio_core::aim::view_scope(scope, &[])),
            selection: &selection,
            cursor: self.cursor_message(),
            rows: &self.list,
        };
        let command = postio_core::aim::command_for(id, &aim);
        let (quiet, _) = postio_core::bridge::event_channel();
        postio_core::aim::mirror(&self.state, &quiet, &aim);
        vec![Effect::Send(command)]
    }

    /// Expand every fold in what is being read, or fold them all again.
    fn toggle_folds(&mut self) {
        if let Some(reading) = self.reading.as_mut() {
            reading.toggle_folds();
        }
    }

    /// The reader's lines and each member's header line, as of now.
    fn reader_layout(&self) -> Option<(Vec<ratatui::text::Line<'static>>, Vec<usize>)> {
        self.reading
            .as_ref()
            .map(|reading| reading.layout(chrono::Local::now()))
    }

    /// Scroll the reader by `pages` screenfuls, overlapping two lines so the
    /// eye keeps its place.
    fn scroll_reader(&mut self, pages: isize) {
        let Some((lines, _)) = self.reader_layout() else {
            return;
        };
        let length = lines.len();
        let page = usize::from(self.size.1.saturating_sub(6)).max(1);
        let step = page.saturating_sub(2).max(1);
        self.reader_top = if pages >= 0 {
            (self.reader_top + step * pages.unsigned_abs()).min(length.saturating_sub(1))
        } else {
            self.reader_top.saturating_sub(step * pages.unsigned_abs())
        };
    }

    /// Move to the next or previous message of the conversation.
    fn walk_conversation(&mut self, step: isize) {
        let Some(reading) = self.reading.as_mut() else {
            return;
        };
        let last = reading.members.len().saturating_sub(1);
        reading.current = reading.current.saturating_add_signed(step).min(last);
        if let Some((_, headers)) = self.reader_layout() {
            let current = self.reading.as_ref().map_or(0, |reading| reading.current);
            self.reader_top = headers.get(current).copied().unwrap_or(0);
        }
    }

    /// The cursor stayed: read what it is on, if the reader is not already.
    fn rested(&mut self, message: postio_model::MessageId) -> Vec<Effect> {
        let here = self.cursor_message() == Some(message);
        let shown = self.reading.as_ref().map(|reading| reading.row) == Some(message);
        if !here || shown {
            return Vec::new();
        }
        let Some(row) = self.list.row_of(message) else {
            return Vec::new();
        };
        match row.thread {
            Some(thread) if row.is_thread && row.count > 1 => {
                self.reading = Some(crate::conversation::Reading {
                    row: message,
                    members: Vec::new(),
                    current: 0,
                });
                vec![Effect::ReadConversation(thread)]
            }
            _ => {
                self.reading = Some(crate::conversation::Reading {
                    row: message,
                    members: vec![crate::conversation::Member {
                        id: row.id,
                        from: row.from.clone(),
                        when: row.when,
                        body: None,
                    }],
                    current: 0,
                });
                self.reader_top = 0;
                vec![Effect::Redraw, Effect::ReadBody(message)]
            }
        }
    }

    /// A conversation's members arrived: read each one's body, and open on
    /// the newest.
    fn conversation(
        &mut self,
        thread: postio_model::ThreadId,
        members: Result<Vec<postio_model::listing::MessageSummary>, String>,
    ) -> Vec<Effect> {
        let Some(reading) = self.reading.as_mut() else {
            return Vec::new();
        };
        let wanted = self.list.row_of(reading.row).and_then(|row| row.thread) == Some(thread);
        if !wanted || !reading.members.is_empty() {
            return Vec::new();
        }
        let Ok(members) = members else {
            return Vec::new();
        };
        reading.members = members
            .iter()
            .map(crate::conversation::Member::from_summary)
            .collect();
        reading.current = reading.members.len().saturating_sub(1);
        let reads = reading
            .members
            .iter()
            .map(|member| Effect::ReadBody(member.id))
            .collect::<Vec<_>>();
        self.walk_conversation(0);
        let mut effects = vec![Effect::Redraw];
        effects.extend(reads);
        effects
    }

    /// A body arrived: put it on its member, if it is still being read.
    fn show(
        &mut self,
        message: postio_model::MessageId,
        answer: Result<postio_client::protocol::Body, String>,
    ) -> Vec<Effect> {
        use postio_client::protocol::Body;
        use postio_ui::reader::document::{
            Absent, Rendering, absent_html, body_html, suits_reader_view,
        };
        let absent = |state| crate::reader::from_html(&absent_html(state));
        let rendered = match answer {
            Ok(Body::Ready { body, .. }) if body.html.is_some() => {
                // The rule every reader applies: reader view for bulk mail,
                // the sender's own markup (sanitised, folded) otherwise.
                let rendering = if suits_reader_view(&body) {
                    Rendering::Reader
                } else {
                    Rendering::Original
                };
                let drawn = body_html(&body, postio_body::RemoteImages::Blocked, rendering);
                crate::reader::from_html(&drawn.html)
            }
            Ok(Body::Ready { body, .. }) => {
                crate::reader::from_text(body.text.as_deref().unwrap_or(""))
            }
            Ok(Body::Partial) => absent(Absent::Partial),
            Ok(Body::Offline) => absent(Absent::Offline),
            Ok(Body::Missing) => absent(Absent::Missing),
            Ok(Body::Empty) => absent(Absent::Empty),
            Ok(Body::ForeignDraft) => absent(Absent::ForeignDraft),
            Err(reason) => crate::reader::from_text(&reason),
        };
        let Some(member) = self.reading.as_mut().and_then(|reading| {
            reading
                .members
                .iter_mut()
                .find(|member| member.id == message)
        }) else {
            return Vec::new();
        };
        member.body = Some(rendered);
        // Bodies arrive in any order; keep the newest message's header in view.
        self.walk_conversation(0);
        vec![Effect::Redraw]
    }

    /// Take in the sidebar's contents, keeping the cursor on the list shown.
    fn fill_sidebar(&mut self, contents: &crate::sidebar::Contents) -> Vec<Effect> {
        self.sidebar = crate::sidebar::lines(contents);
        self.folders = contents.folders.clone();
        for account in &contents.accounts {
            self.trackers.note_last_sync(account.id, &contents.folders);
        }
        if let Some(scope) = self.scope {
            self.account = self.account_of(scope);
        }
        self.sidebar_cursor = self
            .sidebar
            .iter()
            .position(|line| line.opens.is_some() && line.opens == self.scope)
            .or_else(|| self.sidebar.iter().position(|line| line.opens.is_some()))
            .unwrap_or(0);
        vec![Effect::Redraw]
    }

    /// Move the sidebar cursor by `step` rows that open something, and open
    /// what it lands on -- as the desktop sidebar opens a folder when the
    /// selection moves to it.
    fn walk_sidebar(&mut self, step: isize) -> Vec<Effect> {
        let openable: Vec<usize> = self
            .sidebar
            .iter()
            .enumerate()
            .filter(|(_, line)| line.opens.is_some())
            .map(|(index, _)| index)
            .collect();
        let Some(here) = openable
            .iter()
            .position(|index| *index >= self.sidebar_cursor)
        else {
            return Vec::new();
        };
        let there = here.saturating_add_signed(step).min(openable.len() - 1);
        self.sidebar_cursor = openable[there];
        match self.sidebar[self.sidebar_cursor].opens {
            Some(scope) if Some(scope) != self.scope => vec![Effect::Redraw, Effect::Open(scope)],
            _ => vec![Effect::Redraw],
        }
    }

    /// The daemon said something happened.
    ///
    /// What a command said about itself goes on the status line; what
    /// changed in the store goes through the same paging plan the desktop
    /// list follows, so the two lists react to one event the same way.
    fn hear(&mut self, event: &postio_core::Event) -> Vec<Effect> {
        use postio_core::Event;
        // Every event is offered to the status line first: an error is both
        // something to say and the reason a failing account's line gives.
        let moved = self.trackers.apply(event, self.account);
        match event {
            Event::ActionCompleted {
                description,
                undoable,
            } => {
                let sentence = match (undoable, self.keys.key_for(KeyContext::List, "undo")) {
                    (true, Some(key)) => format!("{description} — {key} to undo"),
                    _ => description.clone(),
                };
                return self.say(&sentence);
            }
            Event::UndoPerformed { description } => return self.say(description),
            Event::CommandRejected { reason, .. } => return self.say(reason),
            Event::Error { message } => return self.say(message),
            _ => {}
        }
        if moved {
            return vec![Effect::Redraw];
        }
        if matches!(event, Event::MailboxesChanged { .. }) {
            return vec![Effect::RefreshSidebar];
        }
        match self.paging.plan(event) {
            postio_ui::paging::Plan::Ignore => Vec::new(),
            postio_ui::paging::Plan::InsertAtTop(count) => {
                if self.list.inserted_at_top(count) {
                    // The rows under the cursor moved down; follow them.
                    self.cursor = self.cursor.saturating_add(count);
                    self.top = self.top.saturating_add(count);
                }
                vec![Effect::Redraw]
            }
            postio_ui::paging::Plan::Refetch(messages) => {
                let generation = self.list.generation();
                let pages = self.list.pages_holding(messages);
                pages
                    .into_iter()
                    .filter(|page| self.list.note_pending(*page))
                    .filter_map(|page| {
                        self.paging.fetch_for(page).map(|fetch| Effect::Fetch {
                            generation,
                            page,
                            fetch,
                        })
                    })
                    .collect()
            }
            postio_ui::paging::Plan::Reload => self
                .scope
                .map(|scope| vec![Effect::Recount(scope)])
                .unwrap_or_default(),
        }
    }

    /// A list was counted again after it changed: keep the scroll, drop what
    /// is cached, and let the rows in view be read again.
    fn recounted(&mut self, scope: ListScope, total: u32) -> Vec<Effect> {
        if self.scope != Some(scope) {
            return Vec::new();
        }
        self.list.invalidate();
        let _ = self.list.set_total(total);
        self.move_to(self.cursor);
        vec![Effect::Redraw]
    }

    /// A list opened: show it from the top.
    fn open(&mut self, scope: ListScope, total: u32) -> Vec<Effect> {
        self.paging.open(scope);
        self.scope = Some(scope);
        self.account = self.account_of(scope);
        // A selection is relative to the list it was made in.
        self.selection.clear();
        self.list.reset(total);
        self.cursor = 0;
        self.top = 0;
        vec![Effect::Redraw]
    }

    /// A page arrived, or did not.
    fn page(&mut self, generation: u64, page: u32, rows: Result<Page<Row>, String>) -> Vec<Effect> {
        match rows {
            Ok(rows) => {
                // The count travels with every page: the rows and the total
                // are one read, so each page corrects the total the list was
                // opened with -- for its own generation only.
                if generation == self.list.generation() {
                    let _ = self.list.set_total(rows.total);
                }
                let delivered = self.list.deliver(generation, page, rows.rows);
                if delivered.stale {
                    Vec::new()
                } else {
                    vec![Effect::Redraw]
                }
            }
            Err(reason) => {
                tracing::debug!(page, "a page did not arrive: {reason}");
                self.list.abandon(generation, page);
                Vec::new()
            }
        }
    }
}

/// Take in one input; say what should happen next.
pub fn update(app: &mut App, input: Input) -> Vec<Effect> {
    let mut effects = match input {
        Input::Resize(width, height) => {
            app.size = (width, height);
            vec![Effect::Redraw]
        }
        Input::Key(key) => match app.keys.press(&key, app.key_context(), false) {
            Outcome::Command(id) => app.command(&id),
            Outcome::Pending(_) | Outcome::Unhandled => Vec::new(),
        },
        Input::Opened { scope, total } => app.open(scope, total),
        Input::Host(event) => app.hear(&event),
        Input::Sidebar(contents) => app.fill_sidebar(&contents),
        Input::Rested(message) => app.rested(message),
        Input::Body { message, answer } => app.show(message, answer),
        Input::Conversation { thread, members } => app.conversation(thread, members),
        Input::Recounted { scope, total } => app.recounted(scope, total),
        Input::Page {
            generation,
            page,
            rows,
        } => app.page(generation, page, rows),
    };
    effects.extend(app.fetches());
    // The cursor landed on a different message: rest, then read it. Asked
    // after every input rather than only after a keystroke, because a row can
    // arrive under a still cursor -- a page landing, a list opening.
    let under = app.cursor_message();
    if under != app.resting {
        app.resting = under;
        if let Some(message) = under {
            effects.push(Effect::Rest(message));
        }
    }
    effects
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use crossterm::event::{KeyCode, KeyEventKind, KeyEventState, KeyModifiers};
    use postio_model::{MailboxId, MessageId};
    use postio_ui::terminal::SafeText;

    use super::*;
    use crate::layout::Pane;

    fn app(size: (u16, u16)) -> App {
        let keys = Keys::new(&postio_core::Keymap::resolve(&Default::default())).0;
        App::new(size, keys)
    }

    fn press(c: char) -> Input {
        Input::Key(KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn row(position: u32) -> Row {
        Row {
            id: MessageId::new(i64::from(position) + 1),
            thread: None,
            is_thread: false,
            from: SafeText::new("Ada"),
            subject: SafeText::new(&format!("Message {position}")),
            preview: SafeText::new(""),
            when: Utc.with_ymd_and_hms(2026, 9, 20, 9, 0, 0).unwrap(),
            unread: false,
            flagged: false,
            attachment: false,
            count: 1,
        }
    }

    /// Answer every fetch the way the daemon would, with rows for its range;
    /// return how many fetches there were.
    fn serve(app: &mut App, effects: Vec<Effect>) -> usize {
        let mut fetched = 0;
        let mut pending = effects;
        while let Some(effect) = pending.pop() {
            if let Effect::Fetch {
                generation,
                page,
                fetch: Fetch::Scope(request),
            } = effect
            {
                fetched += 1;
                let rows = (request.offset..request.offset + request.limit)
                    .filter(|position| *position < app.total())
                    .map(row)
                    .collect();
                pending.extend(update(
                    app,
                    Input::Page {
                        generation,
                        page,
                        rows: Ok(Page {
                            total: app.total(),
                            rows,
                        }),
                    },
                ));
            }
        }
        fetched
    }

    fn opened(app: &mut App, total: u32) -> Vec<Effect> {
        update(
            app,
            Input::Opened {
                scope: ListScope::Mailbox(MailboxId::new(1)),
                total,
            },
        )
    }

    #[test]
    fn a_resize_changes_what_is_shown_and_asks_the_daemon_nothing() {
        let mut app = app((160, 40));
        let asked = app.requested();

        let effects = update(&mut app, Input::Resize(60, 30));

        assert_eq!(effects, vec![Effect::Redraw], "a redraw and nothing else");
        assert_eq!(app.shown(), Shown::Panes(vec![Pane::List]));
        assert_eq!(app.requested(), asked, "what was asked for is untouched");

        update(&mut app, Input::Resize(160, 40));
        assert_eq!(
            app.shown(),
            Shown::Panes(vec![Pane::Sidebar, Pane::List, Pane::Reader]),
            "widening brings the sidebar back"
        );
    }

    #[test]
    fn opening_a_list_asks_only_for_the_pages_in_view() {
        let mut app = app((120, 30));
        let effects = opened(&mut app, 100_000);
        let pages: Vec<u32> = effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Fetch { page, .. } => Some(*page),
                _ => None,
            })
            .collect();
        assert!(!pages.is_empty(), "the first rows are asked for");
        assert!(
            pages.iter().all(|page| *page <= 1),
            "only the top: {pages:?}"
        );
    }

    #[test]
    fn walking_a_hundred_thousand_rows_reads_only_what_passes_through_view() {
        // Principle V as a count. A keystroke costs at most one page and the
        // page after it -- `ListWindow`'s read-ahead, which the desktop list
        // shares, so a fast scroll does not stall at a boundary -- and
        // walking 500 rows reads the pages those rows are on and no more.
        let mut app = app((120, 30));
        let opening = opened(&mut app, 100_000);
        serve(&mut app, opening);
        let mut reads = 0;
        for _ in 0..500 {
            let effects = update(&mut app, press('j'));
            let fetches = effects
                .iter()
                .filter(|effect| matches!(effect, Effect::Fetch { .. }))
                .count();
            assert!(fetches <= 2, "one keystroke, {fetches} reads");
            reads += serve(&mut app, effects);
        }
        assert_eq!(app.cursor(), 500);
        // 500 rows and a screenful are eleven pages of fifty, plus the one
        // read ahead.
        assert!(reads <= 12, "{reads} page reads for 500 rows");
    }

    #[test]
    fn the_cursor_stops_at_either_end() {
        let mut app = app((120, 30));
        let opening = opened(&mut app, 3);
        serve(&mut app, opening);
        update(&mut app, press('k'));
        assert_eq!(app.cursor(), 0);
        for _ in 0..10 {
            update(&mut app, press('j'));
        }
        assert_eq!(app.cursor(), 2);
        update(&mut app, press('g'));
        update(&mut app, press('g'));
        assert_eq!(app.cursor(), 0, "g g is the first row");
        update(&mut app, press('G'));
        assert_eq!(app.cursor(), 2, "G is the last");
    }

    fn mark(app: &mut App, position: u32) {
        while app.cursor() < position {
            update(app, press('j'));
        }
        update(app, press('x'));
    }

    #[test]
    fn a_verb_acts_on_what_is_marked_not_where_the_cursor_is() {
        // US1 scenario 3: three marked, the cursor on a fourth.
        let mut app = app((120, 30));
        let opening = opened(&mut app, 10);
        serve(&mut app, opening);
        for position in [1, 2, 3] {
            mark(&mut app, position);
        }
        update(&mut app, press('j'));
        assert_eq!(app.cursor(), 4);

        let effects = update(&mut app, press('a'));

        let sent: Vec<_> = effects
            .iter()
            .filter(|effect| matches!(effect, Effect::Send(_)))
            .collect();
        assert_eq!(sent.len(), 1, "{effects:?}");
        let marked: Vec<MessageId> = [1, 2, 3].map(|position| row(position).id).to_vec();
        assert_eq!(
            app.state()
                .read(|state| state.resolve(&postio_core::MessageTarget::Selection)),
            Some(postio_core::Resolved::Messages(marked)),
            "the host would archive the three marked rows"
        );
    }

    #[test]
    fn with_nothing_marked_a_verb_acts_on_the_cursor_row() {
        let mut app = app((120, 30));
        let opening = opened(&mut app, 10);
        serve(&mut app, opening);
        update(&mut app, press('j'));
        update(&mut app, press('a'));
        assert_eq!(
            app.state()
                .read(|state| state.resolve(&postio_core::MessageTarget::Selection)),
            Some(postio_core::Resolved::Messages(vec![row(1).id]))
        );
    }

    #[test]
    fn marked_rows_are_drawn_as_marked() {
        let mut app = app((120, 30));
        let opening = opened(&mut app, 10);
        serve(&mut app, opening);
        mark(&mut app, 2);
        let visible = app.visible();
        assert!(visible[2].selected);
        assert!(!visible[1].selected);
    }

    #[test]
    fn an_undoable_action_is_announced_and_u_sends_undo() {
        let mut app = app((120, 30));
        let opening = opened(&mut app, 10);
        serve(&mut app, opening);
        update(
            &mut app,
            Input::Host(postio_core::Event::ActionCompleted {
                description: "Archived 12 messages".into(),
                undoable: true,
            }),
        );
        assert_eq!(app.notice(), Some("Archived 12 messages — u to undo"));

        let effects = update(&mut app, press('u'));
        assert!(
            effects.contains(&Effect::Send(postio_core::Command::Undo)),
            "{effects:?}"
        );
        update(
            &mut app,
            Input::Host(postio_core::Event::UndoPerformed {
                description: "Unarchived 12 messages".into(),
            }),
        );
        assert_eq!(app.notice(), Some("Unarchived 12 messages"));
    }

    #[test]
    fn a_list_the_host_changed_is_counted_again_and_reread() {
        let mut app = app((120, 30));
        let opening = opened(&mut app, 10);
        serve(&mut app, opening);
        let scope = ListScope::Mailbox(MailboxId::new(1));
        let effects = update(
            &mut app,
            Input::Host(postio_core::Event::MessageListChanged {
                account: postio_model::AccountId::new(1),
                mailbox: MailboxId::new(1),
            }),
        );
        assert!(effects.contains(&Effect::Recount(scope)), "{effects:?}");

        let effects = update(&mut app, Input::Recounted { scope, total: 7 });
        assert_eq!(app.total(), 7);
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::Fetch { page: 0, .. })),
            "the rows in view are read again: {effects:?}"
        );
    }

    #[test]
    fn a_refusal_is_said_quietly() {
        let mut app = app((120, 30));
        update(
            &mut app,
            Input::Host(postio_core::Event::CommandRejected {
                command: postio_core::CommandId::Archive.into(),
                reason: "Nothing selected".into(),
            }),
        );
        assert_eq!(app.notice(), Some("Nothing selected"));
    }

    fn sidebar_contents() -> crate::sidebar::Contents {
        use postio_model::mailbox::{Mailbox, MailboxRole};
        let mut account = postio_model::Account::new(
            "ada",
            postio_model::EmailAddress::new(None::<String>, "ada@example.com"),
        );
        account.id = postio_model::AccountId::new(1);
        account.enabled = true;
        let folder = |id, name: &str, role| {
            let mut folder = Mailbox::new(account.id, name, None);
            folder.id = MailboxId::new(id);
            folder.role = role;
            folder.selectable = true;
            folder
        };
        crate::sidebar::Contents {
            accounts: vec![account.clone()],
            folders: vec![
                folder(1, "INBOX", MailboxRole::Inbox),
                folder(2, "Archive", MailboxRole::Archive),
            ],
            counts: Vec::new(),
            saved: Vec::new(),
        }
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Input {
        Input::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    #[test]
    fn walking_the_sidebar_opens_what_the_cursor_lands_on() {
        let mut app = app((160, 40));
        let opening = opened(&mut app, 3);
        serve(&mut app, opening);
        update(&mut app, Input::Sidebar(sidebar_contents()));
        update(&mut app, press('g'));
        update(&mut app, press('f'));
        assert_eq!(app.focus(), Focus::Sidebar, "g f focuses the sidebar");

        let effects = update(&mut app, press('j'));
        let opened_scope = effects.iter().find_map(|effect| match effect {
            Effect::Open(scope) => Some(*scope),
            _ => None,
        });
        let (lines, at) = app.sidebar();
        assert_eq!(opened_scope, lines[at].opens, "{effects:?}");
        assert!(opened_scope.is_some());
        assert_ne!(
            opened_scope,
            Some(ListScope::Mailbox(MailboxId::new(1))),
            "it moved off the inbox"
        );

        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::List, "Escape goes back to the list");
    }

    #[test]
    fn the_sidebar_cursor_starts_on_the_list_being_shown() {
        let mut app = app((160, 40));
        let opening = opened(&mut app, 3);
        serve(&mut app, opening);
        update(&mut app, Input::Sidebar(sidebar_contents()));
        let (lines, at) = app.sidebar();
        assert_eq!(lines[at].opens, Some(ListScope::Mailbox(MailboxId::new(1))));
    }

    #[test]
    fn the_status_line_says_offline_until_the_daemon_says_otherwise() {
        let mut app = app((160, 40));
        update(&mut app, Input::Sidebar(sidebar_contents()));
        let opening = opened(&mut app, 3);
        serve(&mut app, opening);
        let line = app.sync_line().expect("a line for the account on screen");
        assert!(line.starts_with("offline"), "{line}");

        update(
            &mut app,
            Input::Host(postio_core::Event::ConnectionChanged {
                account: postio_model::AccountId::new(1),
                state: postio_core::ConnectionState::Online,
            }),
        );
        let line = app.sync_line().unwrap();
        assert!(line.starts_with("idle"), "{line}");
    }

    fn rests(effects: &[Effect]) -> Vec<MessageId> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Rest(message) => Some(*message),
                _ => None,
            })
            .collect()
    }

    fn reads(effects: &[Effect]) -> usize {
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::ReadBody(_)))
            .count()
    }

    #[test]
    fn scrolling_reads_no_body_and_resting_reads_one() {
        let mut app = app((160, 40));
        let opening = opened(&mut app, 50);
        serve(&mut app, opening);
        let mut asked_to_rest = Vec::new();
        for _ in 0..10 {
            let effects = update(&mut app, press('j'));
            assert_eq!(reads(&effects), 0, "a keystroke reads no body");
            asked_to_rest.extend(rests(&effects));
        }
        // Every rest but the last is for a row the cursor has left.
        let mut read = 0;
        for message in asked_to_rest {
            read += reads(&update(&mut app, Input::Rested(message)));
        }
        assert_eq!(read, 1, "only the row the cursor stopped on is read");
    }

    #[test]
    fn a_body_that_arrives_is_drawn_in_the_reader() {
        let mut app = app((160, 40));
        let opening = opened(&mut app, 5);
        serve(&mut app, opening);
        let message = row(0).id;
        update(&mut app, Input::Rested(message));
        update(
            &mut app,
            Input::Body {
                message,
                answer: Ok(postio_client::protocol::Body::Ready {
                    body: postio_model::MessageBody {
                        text: Some("Hello Ada,\n\n> old words\n".into()),
                        html: None,
                    },
                    encoding_problems: false,
                }),
            },
        );
        let reading = app.reading().expect("the reader shows it");
        assert_eq!(reading.row, message);
        let drawn: String = reading
            .layout(chrono::Local::now())
            .0
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(drawn.contains("Hello Ada,"), "{drawn}");
        assert!(drawn.contains("▸ quoted text"), "{drawn}");
    }

    #[test]
    fn a_body_for_a_row_already_left_is_not_drawn() {
        let mut app = app((160, 40));
        let opening = opened(&mut app, 5);
        serve(&mut app, opening);
        update(&mut app, press('j'));
        update(
            &mut app,
            Input::Body {
                message: row(0).id,
                answer: Ok(postio_client::protocol::Body::Partial),
            },
        );
        assert!(
            app.reading()
                .is_none_or(|reading| reading.members.iter().all(|member| member.body.is_none())),
            "nothing drawn for a row already left"
        );
    }

    fn reading_a_long_quoted_reply(app: &mut App) {
        let opening = opened(app, 5);
        serve(app, opening);
        let message = row(0).id;
        update(app, Input::Rested(message));
        let mut text = String::from("Top line\n");
        for n in 0..80 {
            text.push_str(&format!("line {n}\n"));
        }
        text.push_str("> quoted words\n");
        update(
            app,
            Input::Body {
                message,
                answer: Ok(postio_client::protocol::Body::Ready {
                    body: postio_model::MessageBody {
                        text: Some(text),
                        html: None,
                    },
                    encoding_problems: false,
                }),
            },
        );
    }

    fn reader_text(app: &App) -> String {
        app.reading()
            .unwrap()
            .layout(chrono::Local::now())
            .0
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn in_the_reader_o_expands_the_quoted_history_and_folds_it_again() {
        let mut app = app((160, 40));
        reading_a_long_quoted_reply(&mut app);
        assert!(!reader_text(&app).contains("quoted words"));
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::Reader);
        update(&mut app, press('O'));
        assert!(reader_text(&app).contains("quoted words"), "expanded");
        update(&mut app, press('O'));
        assert!(!reader_text(&app).contains("quoted words"), "folded again");
    }

    #[test]
    fn space_scrolls_the_reader_a_screen_at_a_time() {
        let mut app = app((160, 40));
        reading_a_long_quoted_reply(&mut app);
        assert_eq!(app.reader_top(), 0);
        update(&mut app, press(' '));
        assert!(app.reader_top() > 20, "a screenful: {}", app.reader_top());
        update(&mut app, key(KeyCode::PageUp, KeyModifiers::NONE));
        assert_eq!(app.reader_top(), 0);
    }

    fn summary(id: i64, from: &str) -> postio_model::listing::MessageSummary {
        postio_model::listing::MessageSummary {
            id: MessageId::new(id),
            thread: Some(postio_model::ThreadId::new(9)),
            from: Some(postio_model::EmailAddress::new(None::<String>, from)),
            subject: Some("Plans".into()),
            preview: None,
            received_at: Utc.with_ymd_and_hms(2026, 9, 20, 9, 0, 0).unwrap(),
            seen: true,
            flagged: false,
            answered: false,
            send_state: None,
            send_at: None,
            has_attachments: false,
            thread_count: 3,
        }
    }

    /// A list of one conversation row, three messages long, being read.
    fn reading_a_conversation(app: &mut App) -> Vec<Effect> {
        let effects = opened(app, 1);
        let (generation, page) = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Fetch {
                    generation, page, ..
                } => Some((*generation, *page)),
                _ => None,
            })
            .unwrap();
        let mut conversation = row(0);
        conversation.id = MessageId::new(3);
        conversation.thread = Some(postio_model::ThreadId::new(9));
        conversation.is_thread = true;
        conversation.count = 3;
        update(
            app,
            Input::Page {
                generation,
                page,
                rows: Ok(Page {
                    total: 1,
                    rows: vec![conversation],
                }),
            },
        );
        let effects = update(app, Input::Rested(MessageId::new(3)));
        assert!(
            effects.contains(&Effect::ReadConversation(postio_model::ThreadId::new(9))),
            "a conversation row asks for its messages: {effects:?}"
        );
        update(
            app,
            Input::Conversation {
                thread: postio_model::ThreadId::new(9),
                members: Ok(vec![
                    summary(1, "ada@example.com"),
                    summary(2, "bea@example.com"),
                    summary(3, "cy@example.com"),
                ]),
            },
        )
    }

    #[test]
    fn a_conversation_row_reads_every_message_in_it() {
        let mut app = app((160, 40));
        let effects = reading_a_conversation(&mut app);
        let reads: Vec<MessageId> = effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::ReadBody(message) => Some(*message),
                _ => None,
            })
            .collect();
        assert_eq!(reads, [1, 2, 3].map(MessageId::new).to_vec());
        for id in 1..=3 {
            update(
                &mut app,
                Input::Body {
                    message: MessageId::new(id),
                    answer: Ok(postio_client::protocol::Body::Ready {
                        body: postio_model::MessageBody {
                            text: Some(format!("words of message {id}")),
                            html: None,
                        },
                        encoding_problems: false,
                    }),
                },
            );
        }
        let drawn = reader_text(&app);
        for id in 1..=3 {
            assert!(drawn.contains(&format!("words of message {id}")), "{drawn}");
        }
        for who in ["ada@example.com", "bea@example.com", "cy@example.com"] {
            assert!(drawn.contains(who), "{who} heads their message: {drawn}");
        }
    }

    #[test]
    fn j_and_k_in_the_reader_walk_the_conversation() {
        let mut app = app((160, 40));
        reading_a_conversation(&mut app);
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.reading().unwrap().current, 2, "it opens on the newest");
        update(&mut app, press('K'));
        assert_eq!(app.reading().unwrap().current, 1);
        let at_second = app.reader_top();
        update(&mut app, press('K'));
        assert_eq!(app.reading().unwrap().current, 0);
        assert!(app.reader_top() < at_second, "the reader moved up to it");
        update(&mut app, press('J'));
        assert_eq!(app.reading().unwrap().current, 1);
    }

    #[test]
    fn the_quit_command_quits() {
        let mut app = app((120, 30));
        let effects = update(
            &mut app,
            Input::Key(KeyEvent {
                code: KeyCode::Char('q'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }),
        );
        assert!(effects.contains(&Effect::Quit), "{effects:?}");
    }

    #[test]
    fn a_page_that_failed_is_asked_for_again() {
        let mut app = app((120, 30));
        let effects = opened(&mut app, 100);
        let Some(Effect::Fetch {
            generation, page, ..
        }) = effects
            .into_iter()
            .find(|effect| matches!(effect, Effect::Fetch { page: 0, .. }))
        else {
            panic!("the first page was asked for");
        };
        let effects = update(
            &mut app,
            Input::Page {
                generation,
                page,
                rows: Err("busy".into()),
            },
        );
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::Fetch { page: 0, .. })),
            "{effects:?}"
        );
    }
}
