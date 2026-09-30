//! What a reading pane draws of a message, read for every frontend.
//!
//! Moved from `postio-app`'s reading pane (`specs/005-tui-frontend` T018):
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
