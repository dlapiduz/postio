//! What a reading pane draws of a message, read for every frontend.
//!
//! Moved from the classic app's reading pane (`docs/archive/specs/005-tui-frontend` T018):
//! the desktop read a message's body, its row and its send state on one
//! connection per message, and a conversation's members on one reader turn
//! (#1609). The store's owner does both now, and a frontend asks once.

use postio_client::protocol::{Body, Reading};
use postio_model::MessageId;
use postio_model::listing::StoreError;
use postio_session::reading::Body as Stored;
use postio_storage::Store;
use postio_storage::repository::{MessageRepository, ThreadRepository};
use postio_ui::reader::document::Absent;

/// A stored body, as it crosses to a frontend.
pub fn wire_body(stored: Stored) -> Body {
    match stored {
        Stored::Ready {
            body,
            encoding_problems,
        } => Body::Ready {
            body,
            encoding_problems,
        },
        Stored::Absent(Absent::Partial) => Body::Partial,
        Stored::Absent(Absent::Offline) => Body::Offline,
        Stored::Absent(Absent::Missing) => Body::Missing,
        Stored::Absent(Absent::Empty) => Body::Empty,
        Stored::Absent(Absent::ForeignDraft) => Body::ForeignDraft,
    }
}

/// Everything a pane draws of `message`, read on `connection`.
///
/// The parts are metadata the sync already stored -- `BODYSTRUCTURE`, not
/// bytes -- so asking for them costs a row read and never a fetch.
pub async fn load(
    connection: &postio_storage::Checkout,
    message: MessageId,
    offline: bool,
) -> Reading {
    // The row the body was decided from, rather than a second read of it.
    // A draft this machine owns is answered from its buffer without reading
    // the row, so for that one the row is still read here.
    let (body, fetched) =
        postio_session::reading::load_with_row(connection, message, offline).await;
    let row = match fetched {
        Some(row) => Some(row),
        None => MessageRepository::new(connection)
            .get(message)
            .await
            .ok()
            .flatten(),
    };
    let send_state = MessageRepository::new(connection)
        .send_state(message)
        .await
        .unwrap_or_default();
    Reading {
        message,
        body: wire_body(body),
        row: row.map(Box::new),
        send_state,
    }
}

/// [`load`] for each of `messages`, on one reader turn, in the order asked.
pub async fn readings(
    database: &Store,
    messages: &[MessageId],
    offline: bool,
) -> Result<Vec<Reading>, StoreError> {
    let reader = database.read().await.map_err(StoreError::from)?;
    let mut read = Vec::with_capacity(messages.len());
    for &message in messages {
        read.push(load(&reader, message, offline).await);
    }
    Ok(read)
}

/// [`readings`] for `thread`'s first `limit` members, oldest first.
pub async fn thread_readings(
    database: &Store,
    thread: postio_model::ThreadId,
    limit: usize,
    offline: bool,
) -> Result<Vec<Reading>, StoreError> {
    let reader = database.read().await.map_err(StoreError::from)?;
    let members = ThreadRepository::new(&reader)
        .member_ids(thread)
        .await
        .map_err(StoreError::from)?;
    let mut read = Vec::new();
    for message in members.into_iter().take(limit) {
        read.push(load(&reader, message, offline).await);
    }
    Ok(read)
}

/// How long a body asked for and never delivered stays asked for. After
/// that, the next reading of it asks again: the fetch failed, or the link
/// was down, and a person opening it again is asking again.
const ASKED_PATIENCE: std::time::Duration = std::time::Duration::from_secs(60);

/// The bodies a reading pane has asked the engines for, and when (#1788).
///
/// A pane draws a message whose body is not here as "Downloading this
/// message", and that is a promise: reading it is the person opening it,
/// which is the one thing allowed to reach for its words ahead of the
/// backfill (`ARCHITECTURE.md` §11). Every frontend reads a pane through
/// [`readings`], so the promise is kept here once rather than in each of
/// them. A pane is read again on every repaint, though, and a second ask
/// while the first is on the wire is a second fetch (`Backfill::finished`
/// re-offers it), so each message is asked for once until its body lands.
#[derive(Default)]
pub(crate) struct AskedBodies {
    asked: std::sync::Mutex<std::collections::HashMap<MessageId, std::time::Instant>>,
}

impl AskedBodies {
    /// Which of `readings` to ask the engines for now: those whose body is
    /// not here and not asked for within [`ASKED_PATIENCE`]. A body that is
    /// here is forgotten, so a message is never remembered past its fetch.
    pub(crate) fn to_ask(&self, readings: &[Reading], now: std::time::Instant) -> Vec<MessageId> {
        let mut asked = self.asked.lock().expect("never poisoned");
        asked.retain(|_, when| now.saturating_duration_since(*when) < ASKED_PATIENCE);
        let mut ask = Vec::new();
        for reading in readings {
            match reading.body {
                Body::Partial => {
                    if let std::collections::hash_map::Entry::Vacant(first) =
                        asked.entry(reading.message)
                    {
                        first.insert(now);
                        ask.push(reading.message);
                    }
                }
                // Offline is the frontend saying "do not reach"; a foreign
                // draft, a missing or an empty body has nothing to fetch.
                Body::Offline | Body::Missing | Body::Empty | Body::ForeignDraft => {}
                Body::Ready { .. } => {
                    asked.remove(&reading.message);
                }
            }
        }
        ask
    }
}

#[cfg(test)]
mod asked_tests {
    use super::*;

    fn reading(message: i64, body: Body) -> Reading {
        Reading {
            message: MessageId::new(message),
            body,
            row: None,
            send_state: None,
        }
    }

    #[test]
    fn a_body_not_here_is_asked_for_once_until_it_lands() {
        let asked = AskedBodies::default();
        let now = std::time::Instant::now();
        let absent = [reading(1, Body::Partial)];
        assert_eq!(asked.to_ask(&absent, now), vec![MessageId::new(1)]);
        assert!(asked.to_ask(&absent, now).is_empty(), "a repaint");
        let later = now + ASKED_PATIENCE;
        assert_eq!(
            asked.to_ask(&absent, later),
            vec![MessageId::new(1)],
            "never delivered, so opening it again asks again"
        );
    }

    #[test]
    fn only_a_body_not_here_and_reachable_is_asked_for() {
        let asked = AskedBodies::default();
        let now = std::time::Instant::now();
        let read = [
            reading(1, Body::Offline),
            reading(2, Body::Missing),
            reading(3, Body::Empty),
            reading(4, Body::ForeignDraft),
            reading(5, Body::Partial),
        ];
        assert_eq!(asked.to_ask(&read, now), vec![MessageId::new(5)]);
    }
}
