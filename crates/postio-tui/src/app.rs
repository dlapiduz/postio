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
        }
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

    /// A list opened: show it from the top.
    fn open(&mut self, scope: ListScope, total: u32) -> Vec<Effect> {
        self.paging.open(scope);
        self.scope = Some(scope);
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
        Input::Key(key) => match app.keys.press(&key, KeyContext::List, false) {
            Outcome::Command(id) => app.command(&id),
            Outcome::Pending(_) | Outcome::Unhandled => Vec::new(),
        },
        Input::Opened { scope, total } => app.open(scope, total),
        Input::Page {
            generation,
            page,
            rows,
        } => app.page(generation, page, rows),
    };
    effects.extend(app.fetches());
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
