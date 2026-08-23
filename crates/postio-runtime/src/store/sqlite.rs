//! The local store, over real SQLite.
//!
//! The half of [`super`] that owns a database. Behind the `runtime` feature
//! because `postio-gtk` depends on `postio-core` and must not have `rusqlite`
//! anywhere in its dependency graph; whatever assembles the running
//! application turns the feature on, and the view layer never does.

use postio_model::ids::AccountId;
use postio_model::mailbox::Mailbox;
use postio_storage::repository::{
    ListQuery, ListScope as StorageScope, MailboxRepository, MessageListRow, MessageRepository,
    ThreadRepository,
};
use postio_storage::{Database, Pool};

use crate::store::{
    ListScope, MailStore, MessagePage, MessageSummary, PageRequest, Read, StoreError,
};

impl From<ListScope> for StorageScope {
    fn from(scope: ListScope) -> Self {
        match scope {
            ListScope::Mailbox(id) => StorageScope::Mailbox(id),
            ListScope::Account(id) => StorageScope::Account(id),
            ListScope::Flagged(id) => StorageScope::Flagged(id),
        }
    }
}

impl From<postio_storage::Error> for StoreError {
    fn from(error: postio_storage::Error) -> Self {
        StoreError::new(error.to_string())
    }
}

/// The local store, read from a blocking pool.
#[derive(Debug, Clone)]
pub struct SqliteStore {
    pool: Pool,
}

impl SqliteStore {
    /// Read `database` through its own pool.
    ///
    /// Cloning a [`SqliteStore`] is cheap and gives another handle to the same pool,
    /// which is how each blocking read gets a connection of its own.
    pub fn new(database: &Database) -> Self {
        SqliteStore {
            pool: database.pool().clone(),
        }
    }

    async fn read_page(&self, request: PageRequest) -> Result<MessagePage, StoreError> {
        self.read(move |connection| {
            let messages = MessageRepository::new(connection);
            let query = ListQuery {
                scope: request.scope.into(),
                limit: request.limit,
                after: None,
            };
            // Both from one connection and one moment, so the rows and the
            // number of them cannot disagree.
            let total = count(connection, request.scope, &query)?;
            let rows = messages.page_at(&query, request.offset)?;

            let threads = ThreadRepository::new(connection);
            let rows = rows
                .into_iter()
                .map(|row| summarise(row, &threads))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(MessagePage { total, rows })
        })
        .await
    }

    async fn read_count(&self, scope: ListScope) -> Result<u32, StoreError> {
        self.read(move |connection| {
            count(
                connection,
                scope,
                &ListQuery {
                    scope: scope.into(),
                    limit: 0,
                    after: None,
                },
            )
        })
        .await
    }

    async fn read_mailboxes(&self, account: AccountId) -> Result<Vec<Mailbox>, StoreError> {
        self.read(move |connection| {
            Ok(MailboxRepository::new(connection).list_for_account(account)?)
        })
        .await
    }

    /// Run `read` on a blocking thread with a connection of its own.
    ///
    /// `spawn_blocking` rather than a tokio worker: rusqlite blocks, and a
    /// blocked worker is a worker not running everything else. The caller
    /// awaits, so nothing on the calling thread waits either.
    async fn read<T, F>(&self, read: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&postio_storage::PooledConnection) -> Result<T, StoreError> + Send + 'static,
    {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let connection = pool.get().map_err(StoreError::from)?;
            read(&connection)
        })
        .await
        .unwrap_or_else(|error| {
            // A panic in a read is a bug, but the surface it reaches is a
            // mail client that should say something rather than disappear.
            Err(StoreError {
                message: format!("the read did not finish: {error}"),
            })
        })
    }
}

/// How many rows the list would show.
///
/// A single mailbox answers from its own cached count rather than by
/// counting: `count(*)` over a folder is linear in its size, and the message
/// list asks for the total with *every page* — so a 100,000-message mailbox
/// paid 12ms per scroll for a number it already had written down. The
/// `mailboxes.total` column is maintained by `MailboxRepository::recount`
/// against `deleted_locally = 0`, which is exactly the list query's own
/// predicate, so the two cannot mean different things.
///
/// The account-wide and flagged views still count: there is no column for
/// them, and neither is on the scrolling hot path.
fn count(
    connection: &postio_storage::PooledConnection,
    scope: ListScope,
    query: &ListQuery,
) -> Result<u32, StoreError> {
    if let ListScope::Mailbox(mailbox) = scope
        && let Some(counts) = MailboxRepository::new(connection).counts(mailbox)?
    {
        return Ok(counts.total);
    }
    Ok(MessageRepository::new(connection).count(query)?)
}

/// Add the thread count a row's badge needs.
///
/// One lookup per distinct thread on the page — at most a page's worth of
/// indexed point reads, off the UI thread. Counting them in the list query
/// itself would be faster still, and belongs in `postio-storage` when the
/// badge is worth a join.
fn summarise(
    row: MessageListRow,
    threads: &ThreadRepository<'_>,
) -> Result<MessageSummary, StoreError> {
    let thread_count = match row.thread_id {
        Some(id) => threads
            .get(id)?
            .map(|thread| thread.message_count)
            .unwrap_or(1),
        None => 1,
    };
    Ok(MessageSummary {
        id: row.id,
        thread: row.thread_id,
        from: row.from,
        subject: row.subject,
        preview: row.preview,
        received_at: row.received_at,
        seen: row.seen,
        flagged: row.flagged,
        answered: row.answered,
        draft: row.draft,
        has_attachments: row.has_attachments,
        thread_count: thread_count.max(1),
    })
}

impl MailStore for SqliteStore {
    fn message_page(&self, request: PageRequest) -> Read<'_, MessagePage> {
        Box::pin(self.read_page(request))
    }

    fn message_count(&self, scope: ListScope) -> Read<'_, u32> {
        Box::pin(self.read_count(scope))
    }

    fn mailboxes(&self, account: AccountId) -> Read<'_, Vec<Mailbox>> {
        Box::pin(self.read_mailboxes(account))
    }
}
