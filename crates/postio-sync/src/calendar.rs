use std::ops::Range;

use chrono::{DateTime, Duration, TimeZone, Utc};
use postio_model::calendar::EventStatus;
use postio_model::ids::{CalendarId, EventId, MessageId};
use postio_model::listing::StoreError;
use postio_storage::Store;
use postio_storage::repository::{
    CalendarRepository, SettingsRepository, StoredEvent, StoredOccurrence,
};
use postio_storage::store::Connection;

const WINDOW_KEY: &str = "calendar.expanded";
pub const BEHIND: Duration = Duration::days(365);
pub const AHEAD: Duration = Duration::days(2 * 365);
pub const MARGIN: Duration = Duration::days(60);

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

pub fn failed(error: impl std::fmt::Display) -> StoreError {
    tracing::warn!(%error, "calendar store read or write failed");
    StoreError::new("Could not read the calendar")
}

pub async fn expanded(connection: &Connection, now: DateTime<Utc>) -> Result<Expanded, StoreError> {
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

pub async fn set_expanded(connection: &Connection, window: Expanded) -> Result<(), StoreError> {
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

pub fn occurrences<Z: TimeZone>(
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
