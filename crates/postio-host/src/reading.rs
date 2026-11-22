//! What a reading pane draws of a message, for every frontend.

use postio_client::protocol::{Body, Reading};
use postio_model::listing::StoreError;
use postio_model::{AccountId, MessageId};
use postio_session::reading::Body as Stored;
use postio_storage::Store;
use postio_storage::repository::{MessageRepository, ThreadRepository};
use postio_ui::reader::document::Absent;

/// A frontend cannot say whether a request can run, so the host keeps the last link word per
/// account; when an account
/// comes back it tells the panes that showed the offline plate to read again (#1986).
#[derive(Default)]
pub(crate) struct Links {
    state: std::sync::Mutex<LinkState>,
}

#[derive(Default)]
struct LinkState {
    links: std::collections::HashMap<AccountId, postio_core::ConnectionState>,
    waiting: std::collections::HashMap<AccountId, std::collections::HashSet<MessageId>>,
}

impl Links {
    /// Returns the messages whose panes were told "offline", to read again, when it is now
    /// reachable.
    pub(crate) fn changed(
        &self,
        account: AccountId,
        link: postio_core::ConnectionState,
    ) -> Vec<MessageId> {
        let mut state = self.state.lock().expect("never poisoned");
        state.links.insert(account, link);
        if postio_ui::reader::document::reaches_server(Some(link)) {
            state
                .waiting
                .remove(&account)
                .map(|set| set.into_iter().collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    /// A body not here reads "offline" whenever the account's link cannot run a request, whatever
    /// the frontend passed.
    fn settle(&self, account: AccountId, message: MessageId, body: Stored) -> Stored {
        let mut state = self.state.lock().expect("never poisoned");
        let link = state.links.get(&account).copied();
        let body = match body {
            Stored::Absent(Absent::Partial) => {
                Stored::Absent(postio_ui::reader::document::unfetched(
                    false,
                    !postio_ui::reader::document::reaches_server(link),
                ))
            }
            other => other,
        };
        if matches!(body, Stored::Absent(Absent::Offline)) {
            state.waiting.entry(account).or_default().insert(message);
        }
        body
    }
}

pub fn wire_body(stored: Stored) -> Body {
    match stored {
        Stored::Ready {
            body,
            encoding_problems,
            ..
        } => Body::Ready {
            body,
            encoding_problems,
        },
        Stored::Absent(Absent::Partial) => Body::Partial,
        Stored::Absent(Absent::Offline) => Body::Offline,
        Stored::Absent(Absent::Missing) => Body::Missing,
        Stored::Absent(Absent::Empty) => Body::Empty,
        Stored::Absent(Absent::ForeignDraft) => Body::ForeignDraft,
        Stored::Absent(Absent::Failed) => Body::Failed,
    }
}

/// The parts are stored metadata, so asking costs a row read and never a fetch.
pub(crate) async fn load(
    connection: &postio_storage::Checkout,
    message: MessageId,
    offline: bool,
    links: &Links,
) -> Reading {
    // A draft this machine owns is answered from its buffer without reading the row, so its row is
    // still read here.
    let (body, fetched) =
        postio_session::reading::load_with_row(connection, message, offline).await;
    let body = match &fetched {
        Some(row) => links.settle(row.account_id, message, body),
        None => body,
    };
    let headers = match &body {
        Stored::Ready { headers, .. } => headers.clone(),
        Stored::Absent(_) => None,
    };
    let row = match fetched {
        Some(row) => Some(row),
        None => MessageRepository::new(connection)
            .get(message)
            .await
            .ok()
            .flatten(),
    };
    let sender_warning = match (&row, &body) {
        (Some(row), Stored::Ready { .. }) => {
            postio_session::reading::sender_warning(connection, row, headers.as_deref()).await
        }
        _ => None,
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
        sender_warning,
    }
}

pub(crate) async fn readings(
    database: &Store,
    messages: &[MessageId],
    offline: bool,
    links: &Links,
) -> Result<Vec<Reading>, StoreError> {
    let reader = database.read().await.map_err(StoreError::from)?;
    let mut read = Vec::with_capacity(messages.len());
    for &message in messages {
        read.push(load(&reader, message, offline, links).await);
    }
    Ok(read)
}

pub(crate) async fn thread_readings(
    database: &Store,
    thread: postio_model::ThreadId,
    limit: usize,
    offline: bool,
    links: &Links,
) -> Result<Vec<Reading>, StoreError> {
    let reader = database.read().await.map_err(StoreError::from)?;
    let members = ThreadRepository::new(&reader)
        .member_ids(thread)
        .await
        .map_err(StoreError::from)?;
    let mut read = Vec::new();
    for message in members.into_iter().take(limit) {
        read.push(load(&reader, message, offline, links).await);
    }
    Ok(read)
}

/// After that the next reading asks again: the fetch failed or the link was down.
const ASKED_PATIENCE: std::time::Duration = std::time::Duration::from_secs(60);

/// Short, because the person opening it again is the retry; not nothing, because the failure itself
/// makes a pane
/// read again and a link that fails at once would loop (#1985).
#[cfg(not(test))]
const ASKED_RETRY: std::time::Duration = std::time::Duration::from_secs(5);
#[cfg(test)]
const ASKED_RETRY: std::time::Duration = std::time::Duration::from_millis(200);

/// A pane drawn as "Downloading this message" is a promise: reading it is the person opening it,
/// the one thing allowed to reach
/// for its words ahead of the backfill (`ARCHITECTURE.md` §11). A pane reads again on every repaint
/// and a second ask while the
/// first is on the wire is a second fetch, so each message is asked for once until its body lands
/// (#1788).
#[derive(Default)]
pub(crate) struct AskedBodies {
    asked: std::sync::Mutex<std::collections::HashMap<MessageId, Asked>>,
}

struct Asked {
    when: std::time::Instant,
    account: Option<AccountId>,
}

impl AskedBodies {
    /// Returns the messages to read again, so their panes ask the new engine (#1986).
    pub(crate) fn forget_account(&self, account: AccountId) -> Vec<MessageId> {
        let mut asked = self.asked.lock().expect("never poisoned");
        let mut forgotten = Vec::new();
        asked.retain(|message, ask| {
            let theirs = ask.account == Some(account);
            if theirs {
                forgotten.push(*message);
            }
            !theirs
        });
        forgotten
    }

    pub(crate) fn failed(&self, message: MessageId, now: std::time::Instant) {
        let mut asked = self.asked.lock().expect("never poisoned");
        if let Some(ask) = asked.get_mut(&message) {
            ask.when = now.checked_sub(ASKED_PATIENCE - ASKED_RETRY).unwrap_or(now);
        }
    }

    /// A body that is here is forgotten, so a message is never remembered past its fetch.
    pub(crate) fn to_ask(&self, readings: &[Reading], now: std::time::Instant) -> Vec<MessageId> {
        let mut asked = self.asked.lock().expect("never poisoned");
        asked.retain(|_, ask| now.saturating_duration_since(ask.when) < ASKED_PATIENCE);
        let mut ask = Vec::new();
        for reading in readings {
            match reading.body {
                // A refused body is still asked for when the person opens it: only the seeds leave
                // it alone (#1918).
                Body::Partial | Body::Failed => {
                    if let std::collections::hash_map::Entry::Vacant(first) =
                        asked.entry(reading.message)
                    {
                        first.insert(Asked {
                            when: now,
                            account: reading.row.as_ref().map(|row| row.account_id),
                        });
                        ask.push(reading.message);
                    }
                }
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
            sender_warning: None,
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
    fn a_fetch_that_failed_is_asked_for_again_soon_not_after_the_patience() {
        let asked = AskedBodies::default();
        let now = std::time::Instant::now();
        let absent = [reading(1, Body::Partial)];
        assert_eq!(asked.to_ask(&absent, now).len(), 1);

        asked.failed(MessageId::new(1), now);

        assert!(
            asked.to_ask(&absent, now).is_empty(),
            "the repaint the failure itself causes does not ask"
        );
        assert_eq!(
            asked.to_ask(&absent, now + ASKED_RETRY).len(),
            1,
            "opening it again does"
        );
    }

    #[test]
    fn a_new_engine_forgets_only_what_its_account_was_asked() {
        let asked = AskedBodies::default();
        let now = std::time::Instant::now();
        let mut mine = reading(1, Body::Partial);
        mine.row = Some(Box::new(row_of(AccountId::new(1))));
        let mut theirs = reading(2, Body::Partial);
        theirs.row = Some(Box::new(row_of(AccountId::new(2))));
        let read = [mine, theirs];
        assert_eq!(asked.to_ask(&read, now).len(), 2);

        assert_eq!(
            asked.forget_account(AccountId::new(1)),
            vec![MessageId::new(1)]
        );

        assert_eq!(
            asked.to_ask(&read, now),
            vec![MessageId::new(1)],
            "asked again at once, and the other account's ask stands"
        );
    }

    fn row_of(account: AccountId) -> postio_model::Message {
        postio_model::Message::new(
            account,
            postio_model::MailboxId::new(1),
            chrono::DateTime::UNIX_EPOCH,
        )
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
            reading(6, Body::Failed),
        ];
        assert_eq!(
            asked.to_ask(&read, now),
            vec![MessageId::new(5), MessageId::new(6)],
            "opening a message the server once refused asks again"
        );
    }
}
