use chrono::{DateTime, Utc};
use postio_model::calendar::{
    AgendaAnchor, AgendaCursor, AgendaEntry, Calendar, EventStatus, NewCalendar,
};
use postio_model::ids::{AccountId, CalendarId, EventId, MessageId};
use turso::Row;

use super::{from_millis, to_millis};
use crate::error::Result;
use crate::sql::{self, Bind as _, RowExt as _, bind};
use crate::store::Connection;

#[derive(Debug)]
pub struct CalendarRepository<'a> {
    connection: &'a Connection,
}

const COLUMNS: &str =
    "id, account_id, href, name, colour, writable, shown, schedules, default_alarm_minutes";

impl<'a> CalendarRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    pub async fn local(&self) -> Result<Calendar> {
        sql::one(
            self.connection,
            &format!(
                "SELECT {COLUMNS} FROM calendars WHERE account_id IS NULL ORDER BY id LIMIT 1"
            ),
            (),
            read_calendar,
        )
        .await
    }

    pub async fn list(&self) -> Result<Vec<Calendar>> {
        sql::all(
            self.connection,
            &format!("SELECT {COLUMNS} FROM calendars ORDER BY account_id IS NOT NULL, id"),
            (),
            read_calendar,
        )
        .await
    }

    pub async fn create(&self, calendar: &NewCalendar) -> Result<CalendarId> {
        sql::execute(
            self.connection,
            "INSERT INTO calendars (account_id, href, name, colour, writable, schedules)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            bind![
                calendar.account.map(AccountId::get),
                calendar.href.clone(),
                calendar.name.clone(),
                calendar.colour.clone(),
                calendar.writable,
                calendar.schedules,
            ],
        )
        .await?;
        Ok(CalendarId::new(self.connection.last_insert_rowid()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent {
    pub calendar: CalendarId,
    pub uid: String,
    pub href: Option<String>,
    pub etag: Option<String>,
    pub object: Vec<u8>,
    pub title: String,
    pub location: String,
    pub starts_at: DateTime<Utc>,
    pub recurs: bool,
    pub status: EventStatus,
    pub my_partstat: Option<String>,
    pub source_message: Option<MessageId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredOccurrence {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub all_day: bool,
    pub floating: bool,
    pub recurrence_id: Option<String>,
}

const OCCURRENCE_CHUNK: usize = 200;

const AGENDA: &str = "SELECT o.event_id, e.calendar_id, c.name, c.colour, e.title, e.location,
        o.start_utc, o.end_utc, o.all_day, o.floating, o.recurrence_id, e.status, e.my_partstat,
        EXISTS (SELECT 1 FROM event_changes x
                 WHERE x.event_id = o.event_id AND x.state IN ('queued', 'conflict', 'failed'))
   FROM event_occurrences o
   JOIN events e ON e.id = o.event_id
   JOIN calendars c ON c.id = e.calendar_id";

impl CalendarRepository<'_> {
    pub async fn put_event(&self, event: &StoredEvent) -> Result<EventId> {
        sql::execute(
            self.connection,
            "INSERT INTO events (calendar_id, uid, href, etag, object, title, location, starts_at,
                                 recurs, status, my_partstat, source_message_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT (calendar_id, uid) DO UPDATE SET
                href = excluded.href, etag = excluded.etag, object = excluded.object,
                title = excluded.title, location = excluded.location,
                starts_at = excluded.starts_at, recurs = excluded.recurs,
                status = excluded.status, my_partstat = excluded.my_partstat,
                source_message_id = coalesce(excluded.source_message_id, source_message_id)",
            bind![
                event.calendar.get(),
                event.uid,
                event.href,
                event.etag,
                event.object,
                event.title,
                event.location,
                to_millis(event.starts_at),
                event.recurs,
                status_text(event.status),
                event.my_partstat,
                event.source_message.map(MessageId::get),
            ],
        )
        .await?;
        let id: i64 = sql::one(
            self.connection,
            "SELECT id FROM events WHERE calendar_id = ?1 AND uid = ?2",
            bind![event.calendar.get(), event.uid],
            |row| row.col(0),
        )
        .await?;
        Ok(EventId::new(id))
    }

    pub async fn set_occurrences(
        &self,
        event: EventId,
        occurrences: &[StoredOccurrence],
    ) -> Result<()> {
        sql::execute(
            self.connection,
            "DELETE FROM event_occurrences WHERE event_id = ?1",
            [event.get()],
        )
        .await?;
        self.add_occurrences(event, occurrences).await
    }

    pub async fn add_occurrences(
        &self,
        event: EventId,
        occurrences: &[StoredOccurrence],
    ) -> Result<()> {
        for chunk in occurrences.chunks(OCCURRENCE_CHUNK) {
            let rows = vec!["(?, ?, ?, ?, ?, ?)"; chunk.len()].join(", ");
            let mut params = Vec::with_capacity(chunk.len() * 6);
            for occurrence in chunk {
                params.extend(bind![
                    event.get(),
                    to_millis(occurrence.start),
                    to_millis(occurrence.end),
                    occurrence.all_day,
                    occurrence.floating,
                    occurrence.recurrence_id,
                ]);
            }
            sql::execute(
                self.connection,
                &format!(
                    "INSERT OR IGNORE INTO event_occurrences
                        (event_id, start_utc, end_utc, all_day, floating, recurrence_id)
                     VALUES {rows}"
                ),
                params,
            )
            .await?;
        }
        Ok(())
    }

    pub async fn agenda(&self, anchor: AgendaAnchor, limit: u32) -> Result<Vec<AgendaEntry>> {
        let (condition, order, start, event) = match anchor {
            AgendaAnchor::From(at) => ("(o.start_utc, o.event_id) >= (?1, ?2)", "ASC", at, 0),
            AgendaAnchor::After(cursor) => (
                "(o.start_utc, o.event_id) > (?1, ?2)",
                "ASC",
                cursor.start,
                cursor.event.get(),
            ),
            AgendaAnchor::Before(cursor) => (
                "(o.start_utc, o.event_id) < (?1, ?2)",
                "DESC",
                cursor.start,
                cursor.event.get(),
            ),
        };
        let mut rows = sql::all(
            self.connection,
            &format!(
                "{AGENDA} WHERE {condition} AND c.shown = 1
                  ORDER BY o.start_utc {order}, o.event_id {order} LIMIT ?3"
            ),
            bind![to_millis(start), event, limit],
            read_entry,
        )
        .await?;
        if order == "DESC" {
            rows.reverse();
        }
        Ok(rows)
    }

    pub async fn occurrence(&self, cursor: AgendaCursor) -> Result<Option<AgendaEntry>> {
        sql::first(
            self.connection,
            &format!("{AGENDA} WHERE o.event_id = ?1 AND o.start_utc = ?2"),
            bind![cursor.event.get(), to_millis(cursor.start)],
            read_entry,
        )
        .await
    }

    pub async fn next_of(
        &self,
        events: &[EventId],
        from: DateTime<Utc>,
    ) -> Result<Vec<AgendaEntry>> {
        if events.is_empty() {
            return Ok(Vec::new());
        }
        let ids = (2..events.len() + 2)
            .map(|n| format!("?{n}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut params = vec![to_millis(from).bind()];
        params.extend(events.iter().map(|event| event.get().bind()));
        sql::all(
            self.connection,
            &format!(
                "{AGENDA} WHERE o.event_id IN ({ids}) AND o.start_utc = coalesce(
                    (SELECT min(x.start_utc) FROM event_occurrences x
                      WHERE x.event_id = o.event_id AND x.start_utc >= ?1),
                    (SELECT max(x.start_utc) FROM event_occurrences x WHERE x.event_id = o.event_id))
                 ORDER BY o.start_utc, o.event_id"
            ),
            params,
            read_entry,
        )
        .await
    }

    pub async fn object(&self, event: EventId) -> Result<Option<(Vec<u8>, bool)>> {
        sql::first(
            self.connection,
            "SELECT e.object, c.writable FROM events e JOIN calendars c ON c.id = e.calendar_id
              WHERE e.id = ?1",
            [event.get()],
            |row| Ok((row.col(0)?, row.col(1)?)),
        )
        .await
    }

    pub async fn answered(&self, uid: &str) -> Result<Option<(CalendarId, String, Vec<u8>)>> {
        sql::first(
            self.connection,
            "SELECT calendar_id, my_partstat, object FROM events
              WHERE uid = ?1 AND my_partstat IS NOT NULL ORDER BY id LIMIT 1",
            [uid],
            |row| Ok((CalendarId::new(row.col(0)?), row.col(1)?, row.col(2)?)),
        )
        .await
    }

    pub async fn cancel(&self, uid: &str) -> Result<u64> {
        sql::execute(
            self.connection,
            "UPDATE events SET status = 'cancelled' WHERE uid = ?1 AND my_partstat IS NOT NULL",
            [uid],
        )
        .await
    }

    pub async fn default_for(&self, account: AccountId) -> Result<Option<CalendarId>> {
        sql::first(
            self.connection,
            "SELECT id FROM calendars WHERE account_id = ?1 AND writable = 1 AND shown = 1
              ORDER BY id LIMIT 1",
            [account.get()],
            |row| Ok(CalendarId::new(row.col(0)?)),
        )
        .await
    }

    /// Only one the answer made: a synced copy is the server's to keep.
    pub async fn remove_answered(&self, message: MessageId) -> Result<u64> {
        sql::execute(
            self.connection,
            "DELETE FROM events WHERE source_message_id = ?1 AND href IS NULL",
            [message.get()],
        )
        .await
    }

    pub async fn recurring(&self) -> Result<Vec<(EventId, Vec<u8>)>> {
        sql::all_unbounded(
            self.connection,
            "SELECT id, object FROM events WHERE recurs = 1 ORDER BY id",
            (),
            |row| Ok((EventId::new(row.col(0)?), row.col(1)?)),
        )
        .await
    }
}

fn status_text(status: EventStatus) -> &'static str {
    match status {
        EventStatus::Confirmed => "confirmed",
        EventStatus::Tentative => "tentative",
        EventStatus::Cancelled => "cancelled",
    }
}

fn read_entry(row: &Row) -> Result<AgendaEntry> {
    let status: String = row.col(11)?;
    Ok(AgendaEntry {
        event: EventId::new(row.col(0)?),
        calendar: CalendarId::new(row.col(1)?),
        calendar_name: row.col(2)?,
        colour: row.col(3)?,
        title: row.col(4)?,
        location: row.col(5)?,
        start: from_millis(row.col(6)?),
        end: from_millis(row.col(7)?),
        all_day: row.col(8)?,
        floating: row.col(9)?,
        recurrence_id: row.col(10)?,
        status: match status.as_str() {
            "tentative" => EventStatus::Tentative,
            "cancelled" => EventStatus::Cancelled,
            _ => EventStatus::Confirmed,
        },
        my_partstat: row.col(12)?,
        queued: row.col(13)?,
    })
}

fn read_calendar(row: &Row) -> Result<Calendar> {
    let account: Option<i64> = row.col(1)?;
    let minutes: Option<i64> = row.col(8)?;
    Ok(Calendar {
        id: CalendarId::new(row.col(0)?),
        account: account.map(AccountId::new),
        href: row.col(2)?,
        name: row.col(3)?,
        colour: row.col(4)?,
        writable: row.col(5)?,
        shown: row.col(6)?,
        schedules: row.col(7)?,
        default_alarm_minutes: minutes.and_then(|minutes| u32::try_from(minutes).ok()),
    })
}
