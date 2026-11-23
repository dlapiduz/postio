use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use postio_model::calendar::AgendaAnchor;
use postio_model::ids::EventId;
use postio_model::listing::StoreError;
use postio_storage::Store;
use postio_storage::repository::{CalendarRepository, DueReminder, PlannedReminder, ReminderState};

use postio_sync::calendar::failed;

pub const MISSED_WINDOW: Duration = Duration::hours(12);
pub const SNOOZE: Duration = Duration::minutes(5);
const PLANNED_AT_ONCE: u32 = 500;
const DUE_AT_ONCE: u32 = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub reminder: DueReminder,
    pub missed: bool,
}

pub async fn plan_reminders(
    database: &Store,
    from: DateTime<Utc>,
    until: DateTime<Utc>,
) -> Result<usize, StoreError> {
    let connection = database.connect().await.map_err(failed)?;
    let calendars = CalendarRepository::new(&connection);
    let occurrences = calendars
        .agenda(AgendaAnchor::From(from), PLANNED_AT_ONCE)
        .await
        .map_err(failed)?;
    let mut alarms: HashMap<EventId, Vec<postio_calendar::Alarm>> = HashMap::new();
    let mut planned = Vec::new();
    for occurrence in occurrences.iter().filter(|entry| entry.start < until) {
        if let std::collections::hash_map::Entry::Vacant(slot) = alarms.entry(occurrence.event) {
            let found = calendars
                .object(occurrence.event)
                .await
                .map_err(failed)?
                .and_then(|(object, _)| postio_calendar::parse_object(&object).ok())
                .and_then(|events| events.into_iter().next())
                .map(|event| event.alarms)
                .unwrap_or_default();
            slot.insert(found);
        }
        for due in
            postio_calendar::due_times(&alarms[&occurrence.event], occurrence.start, occurrence.end)
        {
            planned.push(PlannedReminder {
                event: occurrence.event,
                occurrence: occurrence.start,
                due,
            });
        }
    }
    calendars.plan_reminders(&planned).await.map_err(failed)?;
    Ok(planned.len())
}

pub async fn due_notices(
    database: &Store,
    now: DateTime<Utc>,
    started: DateTime<Utc>,
) -> Result<Vec<Notice>, StoreError> {
    let connection = database.connect().await.map_err(failed)?;
    let calendars = CalendarRepository::new(&connection);
    let mut notices = Vec::new();
    for reminder in calendars
        .pending_by(now, DUE_AT_ONCE)
        .await
        .map_err(failed)?
    {
        if now - reminder.due > MISSED_WINDOW {
            calendars
                .settle_reminder(&reminder, ReminderState::Missed)
                .await
                .map_err(failed)?;
            continue;
        }
        calendars
            .settle_reminder(&reminder, ReminderState::Fired)
            .await
            .map_err(failed)?;
        notices.push(Notice {
            missed: reminder.due < started,
            reminder,
        });
    }
    tracing::info!(count = notices.len(), "reminders due");
    Ok(notices)
}

pub async fn snooze(
    database: &Store,
    reminder: &DueReminder,
    now: DateTime<Utc>,
) -> Result<(), StoreError> {
    let connection = database.connect().await.map_err(failed)?;
    CalendarRepository::new(&connection)
        .snooze_reminder(reminder, now + SNOOZE)
        .await
        .map_err(failed)
}

pub async fn next_due(database: &Store) -> Result<Option<DateTime<Utc>>, StoreError> {
    let connection = database.read().await.map_err(failed)?;
    CalendarRepository::new(&connection)
        .next_reminder()
        .await
        .map_err(failed)
}
