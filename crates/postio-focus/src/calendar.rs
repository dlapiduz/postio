use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone};
use postio_core::CommandId;
use postio_model::calendar::{AgendaAnchor, AgendaCursor, AgendaEntry, EventDetail};
use postio_ui::calendar::{self as words, AgendaRow, EventView};
use postio_ui::hints::Hint;

use crate::cursor::Rows;
use crate::feed::Step;
use crate::{FocusController, Intent, Reply, Request, SurfaceKind};

pub const AGENDA_PAGE: u32 = 50;
const NEAR_EDGE: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Replace,
    Before,
    After,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarView {
    pub rows: Vec<AgendaRow>,
    pub cursor: Option<usize>,
    pub loaded: bool,
    pub empty: Option<String>,
    pub footer: Vec<Hint>,
    pub open: Option<EventView>,
    pub popped: bool,
    pub open_keys: Vec<Hint>,
    pub editor: Option<postio_ui::calendar::editor::EditorView>,
}

#[derive(Debug)]
pub(crate) struct Calendar {
    entries: Vec<AgendaEntry>,
    cursor: Option<usize>,
    stamp: u64,
    loaded: bool,
    asked_before: bool,
    asked_after: bool,
    ended_before: bool,
    ended_after: bool,
    pending_cursor: Option<DateTime<chrono::Utc>>,
    pub(crate) open: Option<EventDetail>,
    detail_stamp: u64,
    pub(crate) popped: bool,
    pub(crate) editor: Option<crate::event_editor::Editor>,
    edit_when_open: bool,
    pub(crate) follow: Option<DateTime<chrono::Utc>>,
    pub(crate) undo_follows: Option<DateTime<chrono::Utc>>,
}

impl Calendar {
    fn new() -> Self {
        Calendar {
            entries: Vec::new(),
            cursor: None,
            stamp: 0,
            loaded: false,
            asked_before: false,
            asked_after: false,
            ended_before: false,
            ended_after: false,
            pending_cursor: None,
            open: None,
            detail_stamp: 0,
            popped: false,
            editor: None,
            edit_when_open: false,
            follow: None,
            undo_follows: None,
        }
    }

    pub(crate) fn current(&self) -> Option<&AgendaEntry> {
        self.entries.get(self.cursor?)
    }
}

fn day_start(day: NaiveDate) -> DateTime<chrono::Utc> {
    let midnight = day.and_hms_opt(0, 0, 0).unwrap_or_default();
    Local
        .from_local_datetime(&midnight)
        .earliest()
        .map_or_else(|| midnight.and_utc(), |at| at.to_utc())
}

impl FocusController {
    pub(crate) fn open_calendar(&mut self) -> Vec<Step> {
        if self.calendar.is_some() {
            return Vec::new();
        }
        let mut steps = self.calendar_up();
        let today = self.now().date_naive();
        steps.extend(self.calendar_go_to(today));
        steps
    }

    fn calendar_up(&mut self) -> Vec<Step> {
        if self.calendar_popped() {
            self.focused = crate::Window::Calendar;
            return vec![Step::Show(Intent::PresentWindow(crate::Window::Calendar))];
        }
        if self.calendar.is_some() {
            return Vec::new();
        }
        self.calendar = Some(Calendar::new());
        self.surfaces
            .opened(SurfaceKind::Calendar, self.policy.caps.stacking)
    }

    pub(crate) fn calendar_show(&mut self, found: &AgendaEntry) -> Vec<Step> {
        let mut steps = self.calendar_up();
        steps.extend(self.calendar_go_to(found.start.with_timezone(&Local).date_naive()));
        let Some(calendar) = self.calendar.as_mut() else {
            return steps;
        };
        calendar.pending_cursor = Some(found.start);
        calendar.detail_stamp += 1;
        steps.push(Step::Ask(Request::EventDetail {
            cursor: found.cursor(),
            stamp: calendar.detail_stamp,
        }));
        steps
    }

    fn calendar_go_to(&mut self, day: NaiveDate) -> Vec<Step> {
        let now = self.now().to_utc();
        let Some(calendar) = self.calendar.as_mut() else {
            return Vec::new();
        };
        calendar.stamp += 1;
        calendar.entries.clear();
        calendar.cursor = None;
        calendar.open = None;
        calendar.asked_before = false;
        calendar.asked_after = true;
        calendar.ended_before = false;
        calendar.ended_after = false;
        calendar.pending_cursor = Some(now.max(day_start(day)));
        let mut steps = self.draw_calendar();
        steps.push(Step::Ask(Request::AgendaPage {
            anchor: AgendaAnchor::From(day_start(day)),
            limit: AGENDA_PAGE,
            stamp: self.calendar.as_ref().map_or(0, |calendar| calendar.stamp),
            placement: Placement::Replace,
        }));
        steps
    }

    pub(crate) fn calendar_answers(id: CommandId) -> bool {
        matches!(
            id,
            CommandId::Back
                | CommandId::PopOut
                | CommandId::NextMessage
                | CommandId::PrevMessage
                | CommandId::FirstMessage
                | CommandId::LastMessage
                | CommandId::OpenMessage
                | CommandId::CalendarAgenda
                | CommandId::CalendarPrevious
                | CommandId::CalendarNext
                | CommandId::CalendarToday
                | CommandId::GoToCalendar
                | CommandId::Search
                | CommandId::CommandPalette
                | CommandId::NewEvent
                | CommandId::EditEvent
                | CommandId::Delete
                | CommandId::MoveEvent
                | CommandId::DuplicateEvent
                | CommandId::SaveEvent
                | CommandId::SaveAndSendEvent
        )
    }

    pub(crate) fn calendar_command(&mut self, id: CommandId, rows: &dyn Rows) -> Option<Vec<Step>> {
        if self.calendar.as_ref()?.editor.is_some() {
            return self.editor_command(id);
        }
        if matches!(id, CommandId::Search | CommandId::CommandPalette) {
            return self.going(id, rows);
        }
        if id == CommandId::Undo {
            if let Some(calendar) = self.calendar.as_mut() {
                calendar.follow = calendar.undo_follows.take();
            }
            return None;
        }
        let now = self.now();
        let calendar = self.calendar.as_mut()?;
        if id == CommandId::NewEvent {
            calendar.editor = Some(crate::event_editor::Editor::new(now));
            return Some(self.draw_calendar());
        }
        let acted_on = match &calendar.open {
            Some(detail) => Some(detail.entry.clone()),
            None => calendar.current().cloned(),
        };
        match (id, acted_on) {
            (CommandId::Delete, Some(entry)) if entry.recurrence_id.is_some() => {
                calendar.editor = Some(crate::event_editor::Editor::deleting(&entry));
                return Some(self.draw_calendar());
            }
            (CommandId::Delete, Some(entry)) => {
                calendar.open = None;
                calendar.undo_follows = Some(entry.start);
                let mut steps = self.verbs.post(postio_core::Command::DeleteEvent {
                    event: postio_model::calendar::EventTarget {
                        occurrence: entry.cursor(),
                        recurrence_id: None,
                    },
                    scope: None,
                });
                steps.extend(self.draw_calendar());
                return Some(steps);
            }
            (CommandId::MoveEvent, Some(entry)) => {
                calendar.editor = Some(crate::event_editor::Editor::moving(&entry));
                return Some(self.draw_calendar());
            }
            (CommandId::DuplicateEvent, Some(entry)) => {
                calendar.follow = Some(entry.start);
                return Some(
                    self.verbs
                        .post(postio_core::Command::DuplicateEventOf { event: entry.event }),
                );
            }
            (CommandId::Delete | CommandId::MoveEvent | CommandId::DuplicateEvent, None) => {
                return Some(Vec::new());
            }
            _ => {}
        }
        if id == CommandId::EditEvent {
            return Some(match &calendar.open {
                Some(detail) if detail.writable => {
                    calendar.editor = Some(crate::event_editor::Editor::of(detail));
                    self.draw_calendar()
                }
                Some(_) => Vec::new(),
                None => {
                    calendar.edit_when_open = true;
                    self.calendar_open()
                }
            });
        }
        if calendar.open.is_some() {
            return match id {
                CommandId::Back | CommandId::OpenMessage => {
                    calendar.open = None;
                    Some(self.draw_calendar())
                }
                CommandId::CheatSheet => None,
                _ => Some(Vec::new()),
            };
        }
        let today = self.now().date_naive();
        let shown_day = self
            .calendar
            .as_ref()
            .and_then(Calendar::current)
            .map_or(today, |entry| {
                entry.start.with_timezone(&Local).date_naive()
            });
        match id {
            CommandId::Back => Some(self.close_calendar()),
            CommandId::NextMessage => Some(self.calendar_step(1)),
            CommandId::PrevMessage => Some(self.calendar_step(-1)),
            CommandId::FirstMessage => Some(self.calendar_step(i64::MIN / 2)),
            CommandId::LastMessage => Some(self.calendar_step(i64::MAX / 2)),
            CommandId::OpenMessage => Some(self.calendar_open()),
            CommandId::CalendarToday | CommandId::CalendarAgenda => {
                Some(self.calendar_go_to(today))
            }
            CommandId::CalendarNext => Some(self.calendar_go_to(shown_day + Duration::days(7))),
            CommandId::CalendarPrevious => Some(self.calendar_go_to(shown_day - Duration::days(7))),
            CommandId::GoToCalendar => Some(Vec::new()),
            CommandId::CheatSheet => None,
            id if crate::bar::goes(id) || is_destination(id) => {
                let mut steps = self.close_calendar();
                steps.extend(self.list_steps(id, rows));
                Some(steps)
            }
            _ => Some(Vec::new()),
        }
    }

    pub(crate) fn close_calendar(&mut self) -> Vec<Step> {
        let mut steps = self.leave_calendar();
        if !self.has_surface() {
            steps.push(Step::Show(Intent::KeyboardHome));
        }
        steps
    }

    pub(crate) fn leave_calendar(&mut self) -> Vec<Step> {
        if self.calendar_popped() || !self.surfaces.has(SurfaceKind::Calendar) {
            return Vec::new();
        }
        self.calendar = None;
        self.surfaces.dismiss(SurfaceKind::Calendar);
        vec![Step::Show(Intent::CloseSurface(SurfaceKind::Calendar))]
    }

    fn calendar_step(&mut self, by: i64) -> Vec<Step> {
        let Some(calendar) = self.calendar.as_mut() else {
            return Vec::new();
        };
        let total = calendar.entries.len() as i64;
        if total == 0 {
            return Vec::new();
        }
        let from = calendar.cursor.map_or(0, |at| at as i64);
        let to = (from + by).clamp(0, total - 1) as usize;
        let mut steps = Vec::new();
        if calendar.cursor != Some(to) {
            calendar.cursor = Some(to);
            steps.extend(self.draw_calendar());
        }
        steps.extend(self.calendar_page_ahead());
        steps
    }

    pub(crate) fn calendar_point(&mut self, row: usize) -> Vec<Step> {
        let Some(calendar) = self.calendar.as_ref() else {
            return Vec::new();
        };
        let rows = words::agenda_rows(&calendar.entries, self.now().to_utc(), &Local);
        let Some(AgendaRow::Occurrence(picked)) = rows.get(row) else {
            return self.draw_calendar();
        };
        let at = calendar
            .entries
            .iter()
            .position(|entry| entry.cursor() == picked.cursor);
        let from = calendar.cursor;
        match (at, from) {
            (Some(at), Some(from)) => self.calendar_step(at as i64 - from as i64),
            (Some(at), None) => self.calendar_step(at as i64),
            _ => Vec::new(),
        }
    }

    pub(crate) fn calendar_at_root(&self) -> bool {
        self.calendar
            .as_ref()
            .is_none_or(|calendar| calendar.open.is_none())
    }

    fn calendar_page_ahead(&mut self) -> Vec<Step> {
        let Some(calendar) = self.calendar.as_mut() else {
            return Vec::new();
        };
        let Some(cursor) = calendar.cursor else {
            return Vec::new();
        };
        let mut steps = Vec::new();
        if cursor + NEAR_EDGE >= calendar.entries.len()
            && !calendar.asked_after
            && !calendar.ended_after
            && let Some(last) = calendar.entries.last()
        {
            calendar.asked_after = true;
            steps.push(Step::Ask(Request::AgendaPage {
                anchor: AgendaAnchor::After(last.cursor()),
                limit: AGENDA_PAGE,
                stamp: calendar.stamp,
                placement: Placement::After,
            }));
        }
        if cursor < NEAR_EDGE
            && !calendar.asked_before
            && !calendar.ended_before
            && let Some(first) = calendar.entries.first()
        {
            calendar.asked_before = true;
            steps.push(Step::Ask(Request::AgendaPage {
                anchor: AgendaAnchor::Before(first.cursor()),
                limit: AGENDA_PAGE,
                stamp: calendar.stamp,
                placement: Placement::Before,
            }));
        }
        steps
    }

    fn calendar_open(&mut self) -> Vec<Step> {
        let Some(calendar) = self.calendar.as_mut() else {
            return Vec::new();
        };
        let Some(entry) = calendar.current() else {
            return Vec::new();
        };
        let cursor: AgendaCursor = entry.cursor();
        calendar.detail_stamp += 1;
        vec![Step::Ask(Request::EventDetail {
            cursor,
            stamp: calendar.detail_stamp,
        })]
    }

    pub(crate) fn calendar_reply(&mut self, reply: Reply) -> Vec<Step> {
        let Some(calendar) = self.calendar.as_mut() else {
            return Vec::new();
        };
        match reply {
            Reply::AgendaPage {
                stamp,
                placement,
                answer,
            } if stamp == calendar.stamp => {
                let Ok(page) = answer else {
                    calendar.loaded = true;
                    return self.draw_calendar();
                };
                let full = page.len() as u32 >= AGENDA_PAGE;
                match placement {
                    Placement::Replace => {
                        calendar.ended_after = !full;
                        calendar.asked_after = false;
                        let at = calendar.pending_cursor.take();
                        calendar.cursor = at
                            .and_then(|at| page.iter().position(|entry| entry.end > at))
                            .or_else(|| page.len().checked_sub(1));
                        calendar.entries = page;
                    }
                    Placement::After => {
                        calendar.ended_after = !full;
                        calendar.asked_after = false;
                        calendar.entries.extend(page);
                    }
                    Placement::Before => {
                        calendar.ended_before = !full;
                        calendar.asked_before = false;
                        let added = page.len();
                        calendar.cursor = calendar.cursor.map(|at| at + added);
                        let mut entries = page;
                        entries.append(&mut calendar.entries);
                        calendar.entries = entries;
                    }
                }
                calendar.loaded = true;
                let mut steps = self.draw_calendar();
                steps.extend(self.calendar_page_ahead());
                steps
            }
            Reply::EventDetail { stamp, answer } if stamp == calendar.detail_stamp => {
                if let Ok(Some(detail)) = answer {
                    if std::mem::take(&mut calendar.edit_when_open) && detail.writable {
                        calendar.editor = Some(crate::event_editor::Editor::of(&detail));
                    }
                    calendar.open = Some(*detail);
                }
                self.draw_calendar()
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn calendar_heard(&mut self, event: &postio_core::Event) -> Vec<Step> {
        if !matches!(event, postio_core::Event::CalendarChanged) {
            return Vec::new();
        }
        let Some(calendar) = self.calendar.as_mut() else {
            return Vec::new();
        };
        let ringed = calendar.current().map(|entry| entry.start);
        let first = calendar.entries.first().map(|entry| entry.start);
        calendar.stamp += 1;
        calendar.pending_cursor = calendar.follow.take().or(ringed);
        calendar.asked_before = false;
        calendar.asked_after = false;
        let from = match (first, calendar.pending_cursor) {
            (Some(first), Some(at)) => first.min(at),
            (first, at) => first.or(at).unwrap_or_else(|| self.now().to_utc()),
        };
        let calendar = self.calendar.as_ref().expect("checked above");
        let mut steps = vec![Step::Ask(Request::AgendaPage {
            anchor: AgendaAnchor::From(from),
            limit: AGENDA_PAGE,
            stamp: calendar.stamp,
            placement: Placement::Replace,
        })];
        if calendar.open.is_some() {
            steps.extend(self.calendar_open());
        }
        steps
    }

    pub(crate) fn draw_calendar(&self) -> Vec<Step> {
        let Some(calendar) = self.calendar.as_ref() else {
            return Vec::new();
        };
        let now = self.now();
        let today = now.date_naive();
        let rows = words::agenda_rows(&calendar.entries, self.now().to_utc(), &Local);
        let cursor = calendar.cursor.and_then(|at| {
            let event = calendar.entries.get(at)?.cursor();
            rows.iter().position(|row| match row {
                AgendaRow::Occurrence(row) => row.cursor == event,
                AgendaRow::Day { .. } => false,
            })
        });
        let empty = (calendar.loaded && calendar.entries.is_empty())
            .then(|| words::empty_line("these weeks"));
        vec![Step::Show(Intent::Calendar(Box::new(CalendarView {
            rows,
            cursor,
            loaded: calendar.loaded,
            empty,
            footer: words::footer(self.bar.keymap()),
            open: calendar
                .open
                .as_ref()
                .map(|detail| words::event_view(detail, &Local, today)),
            popped: calendar.popped,
            open_keys: words::open_keys(
                self.bar.keymap(),
                calendar.open.as_ref().is_some_and(|detail| detail.writable),
            ),
            editor: self.editor_view(),
        })))]
    }
}

pub(crate) fn is_destination(id: CommandId) -> bool {
    matches!(
        id,
        CommandId::GoToInbox
            | CommandId::GoToDrafts
            | CommandId::GoToSent
            | CommandId::GoToFlagged
            | CommandId::GoToArchive
            | CommandId::GoToSnoozed
            | CommandId::GoToOutbox
            | CommandId::GoToJunk
            | CommandId::GoToTrash
            | CommandId::GoToFolders
            | CommandId::OpenContacts
    )
}
