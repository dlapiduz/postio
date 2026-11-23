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
    open: Option<EventDetail>,
    detail_stamp: u64,
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
        self.calendar = Some(Calendar::new());
        let mut steps = self
            .surfaces
            .opened(SurfaceKind::Calendar, self.policy.caps.stacking);
        let today = self.now().date_naive();
        steps.extend(self.calendar_go_to(today));
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
        )
    }

    pub(crate) fn calendar_command(&mut self, id: CommandId, rows: &dyn Rows) -> Option<Vec<Step>> {
        let calendar = self.calendar.as_mut()?;
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
        self.calendar = None;
        self.surfaces.dismiss(SurfaceKind::Calendar);
        let mut steps = vec![Step::Show(Intent::CloseSurface(SurfaceKind::Calendar))];
        if !self.has_surface() {
            steps.push(Step::Show(Intent::KeyboardHome));
        }
        steps
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
                    calendar.open = Some(*detail);
                }
                self.draw_calendar()
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn draw_calendar(&self) -> Vec<Step> {
        let Some(calendar) = self.calendar.as_ref() else {
            return Vec::new();
        };
        let now = self.now();
        let today = now.date_naive();
        let rows = words::agenda_rows(&calendar.entries, today, &Local);
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
        })))]
    }
}

fn is_destination(id: CommandId) -> bool {
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
