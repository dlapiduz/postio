use chrono::{DateTime, TimeZone, Utc};
use postio_model::calendar::{AgendaAnchor, AgendaCursor, AgendaEntry, EventDetail, Participation};
use postio_model::listing::StoreError;
use postio_storage::Store;
use postio_storage::repository::{CalendarRepository, SettingsRepository};
use postio_storage::store::Connection;
use postio_sync::calendar::{AHEAD, BEHIND, MARGIN, expanded, failed, occurrences, set_expanded};
pub use postio_sync::calendar::{Expanded, NewObject, store_object, store_object_on};

pub async fn agenda_page<Z: TimeZone>(
    database: &Store,
    anchor: AgendaAnchor,
    limit: u32,
    zone: &Z,
    now: DateTime<Utc>,
) -> Result<Vec<AgendaEntry>, StoreError> {
    let connection = database.connect().await.map_err(failed)?;
    let window = expanded(&connection, now).await?;
    let at = match anchor {
        AgendaAnchor::From(at) => at,
        AgendaAnchor::After(cursor) | AgendaAnchor::Before(cursor) => cursor.start,
    };
    if at + MARGIN > window.until || at - MARGIN < window.from {
        widen(&connection, window, at, zone).await?;
    }
    CalendarRepository::new(&connection)
        .agenda(anchor, limit)
        .await
        .map_err(failed)
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EventFound {
    pub entries: Vec<AgendaEntry>,
    pub ignored: Vec<String>,
}

pub async fn search_events<Z: TimeZone>(
    database: &Store,
    text: &str,
    zone: &Z,
    now: DateTime<Utc>,
    limit: u32,
) -> Result<EventFound, StoreError> {
    let today = now.with_timezone(zone).date_naive();
    let query = postio_search::parse(text, today);
    let connection = database.read().await.map_err(failed)?;
    let hits = postio_index::events::search_events(&connection, &query, zone, limit)
        .await
        .map_err(failed)?;
    let from = query
        .filters()
        .find_map(|clause| match clause.filter {
            postio_search::query::Filter::After(day) if !clause.negated => zone
                .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap_or_default())
                .earliest()
                .map(|at| at.with_timezone(&Utc)),
            _ => None,
        })
        .unwrap_or(now);
    let entries = CalendarRepository::new(&connection)
        .next_of(&hits.events, from)
        .await
        .map_err(failed)?;
    tracing::info!(found = entries.len(), "searched the calendar");
    Ok(EventFound {
        entries,
        ignored: hits.ignored,
    })
}

pub async fn event_detail(
    database: &Store,
    cursor: AgendaCursor,
) -> Result<Option<EventDetail>, StoreError> {
    let connection = database.read().await.map_err(failed)?;
    let calendars = CalendarRepository::new(&connection);
    let Some(entry) = calendars.occurrence(cursor).await.map_err(failed)? else {
        return Ok(None);
    };
    let Some((object, writable)) = calendars.object(cursor.event).await.map_err(failed)? else {
        return Ok(None);
    };
    let events = postio_calendar::parse_object(&object).map_err(failed)?;
    let master = events.iter().find(|event| event.recurrence_id.is_none());
    let event = events
        .iter()
        .find(|event| event.recurrence_id.is_some() && event.recurrence_id == entry.recurrence_id)
        .or(master)
        .or_else(|| events.first());
    let Some(event) = event else {
        return Ok(None);
    };
    Ok(Some(EventDetail {
        notes: event.notes.clone(),
        organiser: event.organiser.clone(),
        attendees: event
            .attendees
            .iter()
            .map(|attendee| (attendee.address.clone(), participation(attendee.partstat)))
            .collect(),
        zone: event.zone.clone(),
        repeat: master.and_then(|master| master.repeat.clone()),
        minutes_before: event
            .alarms
            .iter()
            .filter(|alarm| alarm.action != postio_calendar::AlarmAction::Email)
            .filter_map(|alarm| match alarm.trigger {
                postio_calendar::Trigger::SecondsBeforeStart(seconds) => Some(seconds / 60),
                _ => None,
            })
            .collect(),
        writable,
        entry,
    }))
}

fn participation(partstat: postio_calendar::PartStat) -> Participation {
    match partstat {
        postio_calendar::PartStat::Accepted => Participation::Accepted,
        postio_calendar::PartStat::Declined => Participation::Declined,
        postio_calendar::PartStat::Tentative => Participation::Tentative,
        postio_calendar::PartStat::Delegated => Participation::Delegated,
        postio_calendar::PartStat::NeedsAction | postio_calendar::PartStat::Other => {
            Participation::NeedsAction
        }
    }
}

async fn widen<Z: TimeZone>(
    connection: &Connection,
    window: Expanded,
    at: DateTime<Utc>,
    zone: &Z,
) -> Result<(), StoreError> {
    let wider = Expanded {
        from: window.from.min(at - BEHIND),
        until: window.until.max(at + AHEAD),
    };
    let recurring = CalendarRepository::new(connection)
        .recurring()
        .await
        .map_err(failed)?;
    let count = recurring.len();
    postio_storage::sql::in_scope(connection, |scope| async move {
        let calendars = CalendarRepository::new(&scope);
        for (event, ics) in &recurring {
            let mut rows = Vec::new();
            if wider.from < window.from {
                rows.extend(occurrences(ics, wider.from..window.from, zone).unwrap_or_default());
            }
            if wider.until > window.until {
                rows.extend(occurrences(ics, window.until..wider.until, zone).unwrap_or_default());
            }
            calendars.add_occurrences(*event, &rows).await?;
        }
        set_expanded(&scope, wider).await?;
        Ok::<_, StoreError>(())
    })
    .await?;
    tracing::info!(events = count, "widened the calendar window");
    Ok(())
}

const WINDOW_STATE_KEY: &str = "calendar.window";

pub async fn calendar_window(
    database: &Store,
) -> Result<postio_model::calendar::CalendarWindowState, StoreError> {
    let connection = database.read().await.map_err(failed)?;
    let stored = SettingsRepository::new(&connection)
        .get(WINDOW_STATE_KEY)
        .await
        .map_err(failed)?;
    Ok(stored
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default())
}

pub async fn set_calendar_window(
    database: &Store,
    state: postio_model::calendar::CalendarWindowState,
) -> Result<(), StoreError> {
    let connection = database.connect().await.map_err(failed)?;
    let text = serde_json::to_string(&state).map_err(failed)?;
    SettingsRepository::new(&connection)
        .set(WINDOW_STATE_KEY, &text)
        .await
        .map_err(failed)
}
