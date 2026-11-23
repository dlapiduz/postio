use crate::ids::{AccountId, CalendarId};

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
