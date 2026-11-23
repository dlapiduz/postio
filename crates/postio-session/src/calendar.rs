use std::ops::Range;

use chrono::{DateTime, Duration, TimeZone, Utc};
use postio_model::calendar::{
    AgendaAnchor, AgendaCursor, AgendaEntry, EventDetail, EventStatus, Participation,
};
use postio_model::ids::{CalendarId, EventId, MessageId};
use postio_model::listing::StoreError;
use postio_storage::Store;
use postio_storage::repository::{
    CalendarRepository, SettingsRepository, StoredEvent, StoredOccurrence,
};
use postio_storage::store::Connection;

const WINDOW_KEY: &str = "calendar.expanded";
const BEHIND: Duration = Duration::days(365);
const AHEAD: Duration = Duration::days(2 * 365);
const MARGIN: Duration = Duration::days(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Expanded {
    pub from: DateTime<Utc>,
    pub until: DateTime<Utc>,
}

impl Expanded {
    fn around(now: DateTime<Utc>) -> Self {
        Expanded {
            from: now - BEHIND,
            until: now + AHEAD,
        }
    }

    fn range(self) -> Range<DateTime<Utc>> {
        self.from..self.until
    }
}

fn failed(error: impl std::fmt::Display) -> StoreError {
    tracing::warn!(%error, "calendar store read or write failed");
    StoreError::new("Could not read the calendar")
}

async fn expanded(connection: &Connection, now: DateTime<Utc>) -> Result<Expanded, StoreError> {
    let stored = SettingsRepository::new(connection)
        .get(WINDOW_KEY)
        .await
        .map_err(failed)?;
    let parsed = stored.and_then(|text| {
        let (from, until) = text.split_once(' ')?;
        Some(Expanded {
            from: DateTime::from_timestamp_millis(from.parse().ok()?)?,
            until: DateTime::from_timestamp_millis(until.parse().ok()?)?,
        })
    });
    Ok(parsed.unwrap_or_else(|| Expanded::around(now)))
}

async fn set_expanded(connection: &Connection, window: Expanded) -> Result<(), StoreError> {
    let value = format!(
        "{} {}",
        window.from.timestamp_millis(),
        window.until.timestamp_millis()
    );
    SettingsRepository::new(connection)
        .set(WINDOW_KEY, &value)
        .await
        .map_err(failed)
}

fn occurrences<Z: TimeZone>(
    ics: &[u8],
    window: Range<DateTime<Utc>>,
    zone: &Z,
) -> Result<Vec<StoredOccurrence>, postio_calendar::CalendarError> {
    Ok(postio_calendar::expand(ics, window)?
        .into_iter()
        .map(|occurrence| {
            let when = occurrence.on(zone);
            StoredOccurrence {
                start: when.start,
                end: when.end,
                all_day: occurrence.all_day,
                floating: occurrence.floating,
                recurrence_id: occurrence.recurrence_id,
            }
        })
        .collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewObject<'a> {
    pub calendar: CalendarId,
    pub ics: &'a [u8],
    pub href: Option<String>,
    pub etag: Option<String>,
    pub my_partstat: Option<String>,
    pub source_message: Option<MessageId>,
}

pub async fn store_object<Z: TimeZone>(
    database: &Store,
    object: NewObject<'_>,
    zone: &Z,
    now: DateTime<Utc>,
) -> Result<EventId, StoreError> {
    let connection = database.connect().await.map_err(failed)?;
    store_object_on(&connection, object, zone, now).await
}

pub async fn store_object_on<Z: TimeZone>(
    connection: &Connection,
    object: NewObject<'_>,
    zone: &Z,
    now: DateTime<Utc>,
) -> Result<EventId, StoreError> {
    let events = postio_calendar::parse_object(object.ics).map_err(failed)?;
    let master = events
        .iter()
        .find(|event| event.recurrence_id.is_none())
        .or_else(|| events.first())
        .ok_or_else(|| failed(postio_calendar::CalendarError::NoEvent))?
        .clone();
    postio_index::events::ensure(connection)
        .await
        .map_err(failed)?;
    let window = expanded(connection, now).await?;
    let rows = occurrences(object.ics, window.range(), zone).map_err(failed)?;
    let starts_at = rows.first().map_or(now, |row| row.start);
    let event = StoredEvent {
        calendar: object.calendar,
        uid: master.uid.clone(),
        href: object.href,
        etag: object.etag,
        object: object.ics.to_vec(),
        title: master.title.clone(),
        location: master.location.clone(),
        starts_at,
        recurs: master.recurs || events.len() > 1,
        status: match master.status {
            postio_calendar::Status::Confirmed => EventStatus::Confirmed,
            postio_calendar::Status::Tentative => EventStatus::Tentative,
            postio_calendar::Status::Cancelled => EventStatus::Cancelled,
        },
        my_partstat: object.my_partstat,
        source_message: object.source_message,
    };
    let document = postio_index::events::EventDocument {
        title: master.title.clone(),
        location: master.location.clone(),
        notes: master.notes.clone(),
        people: master
            .organiser
            .iter()
            .chain(master.attendees.iter().map(|attendee| &attendee.address))
            .map(ToString::to_string)
            .collect(),
    };
    let id = postio_storage::sql::in_scope(connection, |scope| async move {
        let calendars = CalendarRepository::new(&scope);
        let id = calendars.put_event(&event).await?;
        calendars.set_occurrences(id, &rows).await?;
        postio_index::events::index_event(&scope, id, &document)
            .await
            .map_err(failed)?;
        set_expanded(&scope, window).await?;
        Ok::<_, StoreError>(id)
    })
    .await?;
    tracing::info!(event = id.get(), "stored a calendar object");
    Ok(id)
}

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
