use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use postio_model::ids::EventId;
use postio_search::ParsedQuery;
use postio_search::query::{Filter, TokenKind};
use postio_storage::Connection;
use postio_storage::sql::{self, Bind, RowExt as _};

use crate::error::Result;

const VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS event_search (
    event_id INTEGER PRIMARY KEY REFERENCES events(id) ON DELETE CASCADE,
    body     TEXT NOT NULL,
    people   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS event_search_fts ON event_search USING fts (body);";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EventDocument {
    pub title: String,
    pub location: String,
    pub notes: String,
    pub people: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EventHits {
    pub events: Vec<EventId>,
    pub ignored: Vec<String>,
}

pub async fn ensure(connection: &Connection) -> Result<()> {
    sql::batch(
        connection,
        "CREATE TABLE IF NOT EXISTS search_schema (half TEXT PRIMARY KEY, version INTEGER NOT NULL);",
    )
    .await?;
    if crate::index::half_version(connection, "events").await? == VERSION {
        return Ok(());
    }
    sql::batch(
        connection,
        "DROP INDEX IF EXISTS event_search_fts; DROP TABLE IF EXISTS event_search;",
    )
    .await?;
    sql::batch(connection, SCHEMA).await?;
    crate::index::set_half_version(connection, "events", VERSION).await?;
    Ok(())
}

pub async fn index_event(
    connection: &Connection,
    event: EventId,
    document: &EventDocument,
) -> Result<()> {
    let people = document.people.join("\n");
    let body = postio_model::fold::fold(&format!(
        "{}\n{}\n{}\n{people}",
        document.title, document.location, document.notes
    ));
    sql::execute(
        connection,
        "INSERT INTO event_search (event_id, body, people) VALUES (?1, ?2, ?3)
         ON CONFLICT (event_id) DO UPDATE SET body = excluded.body, people = excluded.people",
        postio_storage::bind![event.get(), body, people.to_lowercase()],
    )
    .await?;
    Ok(())
}

pub async fn forget_event(connection: &Connection, event: EventId) -> Result<()> {
    sql::execute(
        connection,
        "DELETE FROM event_search WHERE event_id = ?1",
        [event.get()],
    )
    .await?;
    Ok(())
}

pub async fn search_events<Z: TimeZone>(
    connection: &Connection,
    query: &ParsedQuery,
    zone: &Z,
    limit: u32,
) -> Result<EventHits> {
    let mut conditions = Vec::new();
    let mut params: Vec<turso::Value> = Vec::new();
    let mut ignored = Vec::new();
    let mut bind = |value: turso::Value, params: &mut Vec<turso::Value>| {
        params.push(value);
        format!("?{}", params.len())
    };
    if let Some(text) = query.fts_match() {
        let text = postio_model::fold::fold(&text);
        let at = bind(text.bind(), &mut params);
        conditions.push(format!(
            "e.id IN (SELECT event_id FROM event_search WHERE fts_match(body, {at}))"
        ));
    }
    for token in query.tokens() {
        let TokenKind::Filter(clause) = &token.kind else {
            continue;
        };
        let not = if clause.negated { "NOT " } else { "" };
        let condition = match &clause.filter {
            Filter::After(day) => {
                let at = bind(midnight(*day, zone).timestamp_millis().bind(), &mut params);
                format!(
                    "{not}EXISTS (SELECT 1 FROM event_occurrences o WHERE o.event_id = e.id AND o.start_utc >= {at})"
                )
            }
            Filter::Before(day) => {
                let at = bind(midnight(*day, zone).timestamp_millis().bind(), &mut params);
                format!(
                    "{not}EXISTS (SELECT 1 FROM event_occurrences o WHERE o.event_id = e.id AND o.start_utc < {at})"
                )
            }
            Filter::In(name) => {
                let at = bind(name.to_lowercase().bind(), &mut params);
                format!("{not}(instr(lower(c.name), {at}) > 0)")
            }
            Filter::From(person) => people(&[person.clone()], not, &mut params, &mut bind),
            Filter::With(addresses) => people(addresses, not, &mut params, &mut bind),
            other => {
                let keyword = other.field().keyword().to_owned();
                if !ignored.contains(&keyword) {
                    ignored.push(keyword);
                }
                continue;
            }
        };
        conditions.push(condition);
    }
    let filter = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };
    let at = bind(limit.bind(), &mut params);
    let events = sql::all(
        connection,
        &format!(
            "SELECT e.id FROM events e JOIN calendars c ON c.id = e.calendar_id
               LEFT JOIN event_search s ON s.event_id = e.id
             {filter} ORDER BY e.starts_at, e.id LIMIT {at}"
        ),
        params,
        |row| Ok(EventId::new(row.col(0)?)),
    )
    .await?;
    Ok(EventHits { events, ignored })
}

fn people(
    names: &[String],
    not: &str,
    params: &mut Vec<turso::Value>,
    bind: &mut impl FnMut(turso::Value, &mut Vec<turso::Value>) -> String,
) -> String {
    let any: Vec<String> = names
        .iter()
        .map(|name| {
            let at = bind(name.to_lowercase().bind(), params);
            format!("instr(coalesce(s.people, ''), {at}) > 0")
        })
        .collect();
    format!("{not}({})", any.join(" OR "))
}

fn midnight<Z: TimeZone>(day: NaiveDate, zone: &Z) -> DateTime<Utc> {
    let local = day.and_hms_opt(0, 0, 0).unwrap_or_default();
    zone.from_local_datetime(&local)
        .earliest()
        .map_or_else(|| local.and_utc(), |at| at.with_timezone(&Utc))
}
