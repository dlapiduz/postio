use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{AccountId, CalendarId, EventId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Calendar {
    pub id: CalendarId,
    pub account: Option<AccountId>,
    pub href: Option<String>,
    pub name: String,
    pub colour: Option<String>,
    pub writable: bool,
    pub shown: bool,
    pub schedules: bool,
    pub default_alarm_minutes: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCalendar {
    pub account: Option<AccountId>,
    pub href: Option<String>,
    pub name: String,
    pub colour: Option<String>,
    pub writable: bool,
    pub schedules: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgendaCursor {
    pub start: DateTime<Utc>,
    pub event: EventId,
}

/// Where a page of the agenda starts: at a time, or either side of a row already shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgendaAnchor {
    From(DateTime<Utc>),
    After(AgendaCursor),
    Before(AgendaCursor),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventStatus {
    Confirmed,
    Tentative,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgendaEntry {
    pub event: EventId,
    pub calendar: CalendarId,
    pub calendar_name: String,
    pub colour: Option<String>,
    pub title: String,
    pub location: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub all_day: bool,
    pub floating: bool,
    pub recurrence_id: Option<String>,
    pub status: EventStatus,
    pub my_partstat: Option<String>,
    /// A change made here that the server has not taken yet.
    pub queued: bool,
}

impl AgendaEntry {
    pub fn cursor(&self) -> AgendaCursor {
        AgendaCursor {
            start: self.start,
            event: self.event,
        }
    }
}
