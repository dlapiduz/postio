use std::collections::BTreeSet;

use chrono::{DateTime, TimeZone, Utc};
use postio_dav::{CalendarBackend, DavError, DavObject, FETCH_BATCH, Listed};
use postio_model::calendar::NewCalendar;
use postio_model::egress::{EgressEvent, EgressOutcome, EgressSubsystem};
use postio_model::ids::AccountId;
use postio_storage::Store;
use postio_storage::repository::{
    CalendarRepository, ChangeKind, ChangeState, EgressLogRepository, EventChange, RemoteCalendar,
};
use postio_storage::store::Connection;

use crate::calendar::{AHEAD, BEHIND, NewObject, store_object_on};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Synced {
    pub fetched: usize,
    pub removed: usize,
    pub sent: usize,
    pub conflicts: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CalendarSyncError {
    #[error(transparent)]
    Server(#[from] DavError),
    #[error("the calendar store failed: {0}")]
    Store(String),
}

fn stored(error: impl std::fmt::Display) -> CalendarSyncError {
    CalendarSyncError::Store(error.to_string())
}

pub async fn sync_account<Z: TimeZone + Sync>(
    database: &Store,
    backend: &dyn CalendarBackend,
    account: AccountId,
    now: DateTime<Utc>,
    zone: &Z,
) -> Result<Synced, CalendarSyncError> {
    let outcome = run(database, backend, account, now, zone).await;
    let connection = database.connect().await.map_err(stored)?;
    let log = EgressLogRepository::new(&connection);
    for opened in backend.connections() {
        log.record(&EgressEvent {
            at: now,
            subsystem: EgressSubsystem::Caldav,
            account: Some(account),
            host: opened.host,
            port: opened.port,
            outcome: if opened.reached {
                EgressOutcome::Connected
            } else {
                EgressOutcome::Failed
            },
        })
        .await
        .map_err(stored)?;
    }
    match &outcome {
        Ok(synced) => tracing::info!(
            account = account.get(),
            fetched = synced.fetched,
            removed = synced.removed,
            sent = synced.sent,
            conflicts = synced.conflicts,
            failed = synced.failed,
            "synced calendars"
        ),
        Err(error) => tracing::warn!(account = account.get(), %error, "calendar sync stopped"),
    }
    outcome
}

async fn run<Z: TimeZone + Sync>(
    database: &Store,
    backend: &dyn CalendarBackend,
    account: AccountId,
    now: DateTime<Utc>,
    zone: &Z,
) -> Result<Synced, CalendarSyncError> {
    let found = backend.calendars().await?;
    let connection = database.connect().await.map_err(stored)?;
    let calendars = CalendarRepository::new(&connection);
    for remote in &found {
        calendars
            .upsert_remote(
                account,
                &NewCalendar {
                    account: Some(account),
                    href: Some(remote.href.clone()),
                    name: remote.name.clone(),
                    colour: remote.colour.clone(),
                    writable: remote.writable,
                    schedules: false,
                },
            )
            .await
            .map_err(stored)?;
    }
    let mut synced = Synced::default();
    for calendar in calendars.remote(account).await.map_err(stored)? {
        push(&connection, backend, &calendar, &mut synced, now, zone).await?;
        match pull(&connection, backend, &calendar, &mut synced, now, zone).await {
            Err(CalendarSyncError::Server(DavError::Forbidden)) => {
                calendars
                    .set_error(calendar.id, "forbidden")
                    .await
                    .map_err(stored)?;
            }
            other => other?,
        }
    }
    Ok(synced)
}

async fn keep<Z: TimeZone + Sync>(
    connection: &Connection,
    calendar: &RemoteCalendar,
    object: &DavObject,
    now: DateTime<Utc>,
    zone: &Z,
) -> Result<(), CalendarSyncError> {
    let mine = CalendarRepository::new(connection)
        .held_at(calendar.id, &object.href)
        .await
        .map_err(stored)?
        .and_then(|held| held.my_partstat);
    store_object_on(
        connection,
        NewObject {
            calendar: calendar.id,
            ics: &object.ics,
            href: Some(object.href.clone()),
            etag: Some(object.etag.clone()),
            my_partstat: mine,
            source_message: None,
        },
        zone,
        now,
    )
    .await
    .map_err(stored)?;
    Ok(())
}

async fn fetch_and_keep<Z: TimeZone + Sync>(
    connection: &Connection,
    backend: &dyn CalendarBackend,
    calendar: &RemoteCalendar,
    listed: &[Listed],
    now: DateTime<Utc>,
    zone: &Z,
) -> Result<usize, CalendarSyncError> {
    let held = CalendarRepository::new(connection);
    let mut wanted = Vec::new();
    for item in listed {
        let Some(current) = held
            .held_at(calendar.id, &item.href)
            .await
            .map_err(stored)?
        else {
            wanted.push(item.href.clone());
            continue;
        };
        let ours_waits = held.unsettled(current.id).await.map_err(stored)?;
        if !ours_waits && current.etag.as_deref() != Some(item.etag.as_str()) {
            wanted.push(item.href.clone());
        }
    }
    let mut fetched = 0;
    for batch in wanted.chunks(FETCH_BATCH) {
        for object in backend.fetch(&calendar.href, batch).await? {
            keep(connection, calendar, &object, now, zone).await?;
            fetched += 1;
        }
    }
    Ok(fetched)
}

async fn pull<Z: TimeZone + Sync>(
    connection: &Connection,
    backend: &dyn CalendarBackend,
    calendar: &RemoteCalendar,
    synced: &mut Synced,
    now: DateTime<Utc>,
    zone: &Z,
) -> Result<(), CalendarSyncError> {
    let calendars = CalendarRepository::new(connection);
    if let Some(token) = &calendar.token {
        match backend.changes(&calendar.href, Some(token)).await {
            Ok(changes) => {
                synced.fetched +=
                    fetch_and_keep(connection, backend, calendar, &changes.changed, now, zone)
                        .await?;
                for href in &changes.removed {
                    if let Some(gone) =
                        calendars.held_at(calendar.id, href).await.map_err(stored)?
                    {
                        calendars.delete_event(gone.id).await.map_err(stored)?;
                        postio_index::events::forget_event(connection, gone.id)
                            .await
                            .map_err(stored)?;
                        synced.removed += 1;
                    }
                }
                calendars
                    .set_synced(calendar.id, changes.token.as_deref(), now)
                    .await
                    .map_err(stored)?;
                return Ok(());
            }
            Err(DavError::TokenExpired) => {
                calendars
                    .set_synced(calendar.id, None, now)
                    .await
                    .map_err(stored)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    let window = backend
        .listing(&calendar.href, Some((now - BEHIND, now + AHEAD)))
        .await?;
    synced.fetched += fetch_and_keep(connection, backend, calendar, &window, now, zone).await?;
    let seen: BTreeSet<&str> = window.iter().map(|item| item.href.as_str()).collect();
    let rest: Vec<Listed> = backend
        .listing(&calendar.href, None)
        .await?
        .into_iter()
        .filter(|item| !seen.contains(item.href.as_str()))
        .collect();
    synced.fetched += fetch_and_keep(connection, backend, calendar, &rest, now, zone).await?;
    let token = backend.changes(&calendar.href, None).await?.token;
    calendars
        .set_synced(calendar.id, token.as_deref(), now)
        .await
        .map_err(stored)?;
    Ok(())
}

fn href_for(uid: &str) -> String {
    let safe: String = uid
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.@".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("{safe}.ics")
}

async fn push<Z: TimeZone + Sync>(
    connection: &Connection,
    backend: &dyn CalendarBackend,
    calendar: &RemoteCalendar,
    synced: &mut Synced,
    now: DateTime<Utc>,
    zone: &Z,
) -> Result<(), CalendarSyncError> {
    let calendars = CalendarRepository::new(connection);
    for change in calendars.queued(calendar.id).await.map_err(stored)? {
        let state = match send(connection, backend, calendar, &change, now, zone).await {
            Ok(state) => state,
            Err(CalendarSyncError::Server(error)) if error.is_transient() => {
                return Err(error.into());
            }
            Err(CalendarSyncError::Server(DavError::Forbidden)) => {
                calendars
                    .set_writable(calendar.id, false)
                    .await
                    .map_err(stored)?;
                ChangeState::Failed
            }
            Err(CalendarSyncError::Server(_)) => ChangeState::Failed,
            Err(other) => return Err(other),
        };
        match state {
            ChangeState::Sent => synced.sent += 1,
            ChangeState::Conflict => synced.conflicts += 1,
            ChangeState::Failed => synced.failed += 1,
        }
        calendars.settle(change.id, state).await.map_err(stored)?;
    }
    Ok(())
}

async fn send<Z: TimeZone + Sync>(
    connection: &Connection,
    backend: &dyn CalendarBackend,
    calendar: &RemoteCalendar,
    change: &EventChange,
    now: DateTime<Utc>,
    zone: &Z,
) -> Result<ChangeState, CalendarSyncError> {
    let calendars = CalendarRepository::new(connection);
    if change.kind == ChangeKind::Delete {
        let Some(href) = &change.href else {
            return Ok(ChangeState::Sent);
        };
        return match backend
            .delete(&calendar.href, href, change.base_etag.as_deref())
            .await
        {
            Ok(()) => Ok(ChangeState::Sent),
            Err(DavError::Changed) => Ok(ChangeState::Conflict),
            Err(error) => Err(error.into()),
        };
    }
    let Some(event) = change.event else {
        return Ok(ChangeState::Sent);
    };
    let Some(ours) = calendars.held(event).await.map_err(stored)? else {
        return Ok(ChangeState::Sent);
    };
    let (href, etag) = match (&ours.href, &ours.etag) {
        (Some(href), etag) => (href.clone(), etag.clone()),
        (None, _) => (href_for(&ours.uid), None),
    };
    match backend
        .put(&calendar.href, &href, &ours.object, etag.as_deref())
        .await
    {
        Ok(etag) => {
            calendars
                .set_remote(ours.id, &href, &etag)
                .await
                .map_err(stored)?;
            Ok(ChangeState::Sent)
        }
        Err(DavError::Changed) => {
            let Some(theirs) = backend
                .fetch(&calendar.href, std::slice::from_ref(&href))
                .await?
                .into_iter()
                .next()
            else {
                return Ok(ChangeState::Conflict);
            };
            let base = change.base.as_deref().unwrap_or(&theirs.ics);
            let Some(merged) =
                postio_calendar::merge(base, &ours.object, &theirs.ics).map_err(stored)?
            else {
                return Ok(ChangeState::Conflict);
            };
            match backend
                .put(&calendar.href, &href, &merged, Some(&theirs.etag))
                .await
            {
                Ok(etag) => {
                    keep(
                        connection,
                        calendar,
                        &DavObject {
                            href,
                            etag,
                            ics: merged,
                        },
                        now,
                        zone,
                    )
                    .await?;
                    Ok(ChangeState::Sent)
                }
                Err(DavError::Changed) => Ok(ChangeState::Conflict),
                Err(error) => Err(error.into()),
            }
        }
        Err(error) => Err(error.into()),
    }
}
