//! The local store, over real SQLite.
//!
//! The half of [`super`] that owns a database. Behind the `runtime` feature
//! because `postio-gtk` depends on `postio-core` and must not have `rusqlite`
//! anywhere in its dependency graph; whatever assembles the running
//! application turns the feature on, and the view layer never does.

use postio_model::ids::{AccountId, MessageId};
use postio_model::mailbox::Mailbox;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use postio_storage::repository::{
    ListCursor, ListQuery, MailboxRepository, MessageListRow, MessageRepository, ThreadCursor,
    ThreadListQuery, ThreadListRow, ThreadRepository,
};
use postio_storage::{Database, Pool};

use crate::store::{
    ListPage, ListScope, MailStore, MessagePage, MessageSummary, PageRequest, Read, StoreError,
    ThreadPage, ThreadSummary,
};

impl From<postio_storage::Error> for StoreError {
    fn from(error: postio_storage::Error) -> Self {
        StoreError::new(error.to_string())
    }
}

/// The local store, read from a blocking pool.
#[derive(Debug, Clone)]
pub struct SqliteStore {
    pool: Pool,
    /// Where each page boundary starts, so a page does not have to be
    /// counted to from the top of the folder every time. See [`Marks`].
    marks: Arc<Mutex<Marks<ListCursor>>>,
    /// The same, for the threaded list. Its own, because a folder that
    /// threads and the same folder listed by message are different windows
    /// with different row counts -- one set of marks would have each read
    /// clearing the other's.
    thread_marks: Arc<Mutex<Marks<ThreadCursor>>>,
}

/// Remembered page boundaries for one scope.
///
/// `page_at` is an `OFFSET` query and SQLite walks the rows it skips, so a
/// page halfway down a 100,000-message folder costs 24ms against a 16ms
/// budget — measured, in `benches/store_reads.rs`. The fix is to seek instead
/// of skip: `MessageRepository::page` takes a [`ListCursor`] and its where
/// clause is a row value, which SQLite turns into a range constraint.
///
/// The frontend's windowed model asks for *page N*, which a cursor cannot
/// answer on its own — so the store remembers where each page it has already
/// read began. Scrolling is sequential, so the next page's start is nearly
/// always known and the read seeks straight to it. A jump to a page nobody
/// has visited falls back to the nearest mark below it and skips the
/// remainder, which is at worst what it costs today.
///
/// # When they are wrong
///
/// New mail shifts every row down, and a mark then points one row off. The
/// total is checked on every read — it is a column lookup now, not a count —
/// and a change drops every mark. That misses the case where one message
/// arrives and another is deleted between two reads, leaving the total equal
/// and the order shifted; the consequence is one page off by a row, which is
/// exactly what a plain `OFFSET` does in the same situation and what
/// `page_at`'s own documentation warns about.
#[derive(Debug)]
struct Marks<C> {
    /// The row count the marks were taken against.
    total: Option<u32>,
    /// Offset of a page boundary, and the cursor the row before it left.
    at: BTreeMap<u32, C>,
}

// Derived `Default` would demand `C: Default`, and a cursor has no sensible
// zero -- an empty map does.
impl<C> Default for Marks<C> {
    fn default() -> Self {
        Marks {
            total: None,
            at: BTreeMap::new(),
        }
    }
}

impl<C: Copy> Marks<C> {
    /// Forget everything if the list is not the length it was.
    fn check(&mut self, total: u32) {
        if self.total != Some(total) {
            self.at.clear();
            self.total = Some(total);
        }
    }

    /// The nearest remembered boundary at or before `offset`.
    fn nearest(&self, offset: u32) -> Option<(u32, C)> {
        self.at
            .range(..=offset)
            .next_back()
            .map(|(at, cursor)| (*at, *cursor))
    }

    /// Remember where the page after `offset` begins.
    fn remember(&mut self, offset: u32, cursor: C) {
        // Bounded: a folder read end to end at 50 a page leaves 2,000 marks
        // for 100,000 messages, and each is two integers. Worth the memory to
        // never walk the folder again.
        self.at.insert(offset, cursor);
    }
}

impl SqliteStore {
    /// Read `database` through its own pool.
    ///
    /// Cloning a [`SqliteStore`] is cheap and gives another handle to the same pool,
    /// which is how each blocking read gets a connection of its own.
    pub fn new(database: &Database) -> Self {
        SqliteStore {
            pool: database.pool().clone(),
            marks: Arc::new(Mutex::new(Marks::default())),
            thread_marks: Arc::new(Mutex::new(Marks::default())),
        }
    }

    async fn read_page(&self, request: PageRequest) -> Result<MessagePage, StoreError> {
        let marks = self.marks.clone();
        self.read(move |connection| {
            let messages = MessageRepository::new(connection);
            let query = ListQuery {
                scope: request.scope,
                limit: request.limit,
                after: None,
            };
            // Both from one connection and one moment, so the rows and the
            // number of them cannot disagree.
            let total = count(connection, request.scope, &query)?;

            // Seek to the nearest boundary anybody has already read, and skip
            // only what is left. For sequential scrolling that is nothing.
            let start = {
                let mut marks = marks.lock().expect("not poisoned");
                marks.check(total);
                marks.nearest(request.offset)
            };
            let (seek, skip) = match start {
                Some((at, cursor)) => (Some(cursor), request.offset - at),
                None => (None, request.offset),
            };
            let query = ListQuery {
                after: seek,
                ..query
            };
            let rows = messages.page_at(&query, skip)?;

            // And remember where the next page begins, so it can seek too.
            if let Some(last) = rows.last() {
                marks
                    .lock()
                    .expect("not poisoned")
                    .remember(request.offset + rows.len() as u32, last.cursor());
            }

            let threads = ThreadRepository::new(connection);
            let rows = rows
                .into_iter()
                .map(|row| summarise(row, &threads))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(MessagePage { total, rows })
        })
        .await
    }

    /// Whether `scope` lists conversations, which only the store can answer.
    ///
    /// Folders thread and query views list messages (ADR 0015) — with one
    /// exception the ADR did not have to name because it was writing about
    /// reading mail: **Drafts does not thread.** A draft is a document you
    /// are writing, not a conversation you are triaging, and two drafts
    /// answering the same thread would collapse into one row with no way to
    /// open the other. Sent does thread, because a sent message really is
    /// part of the conversation it belongs to.
    async fn lists_conversations(&self, scope: ListScope) -> Result<bool, StoreError> {
        let ListScope::Mailbox(mailbox) = scope else {
            return Ok(matches!(scope, ListScope::Account(_)));
        };
        self.read(move |connection| {
            let folder = MailboxRepository::new(connection)
                .get(mailbox)?
                .ok_or_else(|| StoreError::new("That folder is no longer here"))?;
            Ok(folder.role != postio_model::mailbox::MailboxRole::Drafts)
        })
        .await
    }

    async fn read_list_page(&self, request: PageRequest) -> Result<ListPage, StoreError> {
        if self.lists_conversations(request.scope).await? {
            self.read_thread_page(request).await.map(ListPage::Threads)
        } else {
            self.read_page(request).await.map(ListPage::Messages)
        }
    }

    async fn read_list_count(&self, scope: ListScope) -> Result<u32, StoreError> {
        if self.lists_conversations(scope).await? {
            self.read_thread_count(scope).await
        } else {
            self.read_count(scope).await
        }
    }

    /// One page of the threaded list.
    ///
    /// The same shape as [`SqliteStore::read_page`], over the thread window
    /// instead of the message one: count and rows from one connection and one
    /// moment, seek to the nearest boundary anybody has already read, and
    /// remember where this page ended so the next one can seek too.
    async fn read_thread_page(&self, request: PageRequest) -> Result<ThreadPage, StoreError> {
        let marks = self.thread_marks.clone();
        self.read(move |connection| {
            let query = thread_query(connection, request.scope, request.limit)?;
            let threads = ThreadRepository::new(connection);
            let total = threads.count_of(&query)?;

            let start = {
                let mut marks = marks.lock().expect("not poisoned");
                marks.check(total);
                marks.nearest(request.offset)
            };
            let (seek, skip) = match start {
                Some((at, cursor)) => (Some(cursor), request.offset - at),
                None => (None, request.offset),
            };
            let rows = threads.page_at(
                &ThreadListQuery {
                    after: seek,
                    ..query.clone()
                },
                skip,
            )?;
            if let Some(last) = rows.last() {
                marks
                    .lock()
                    .expect("not poisoned")
                    .remember(request.offset + rows.len() as u32, last.cursor());
            }

            let rows = rows
                .into_iter()
                .map(summarise_thread)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ThreadPage { total, rows })
        })
        .await
    }

    async fn read_thread_count(&self, scope: ListScope) -> Result<u32, StoreError> {
        self.read(move |connection| {
            let query = thread_query(connection, scope, 0)?;
            Ok(ThreadRepository::new(connection).count_of(&query)?)
        })
        .await
    }

    /// No seek marks and no count: an explicit id list is not a window into
    /// anything, so there is no position to remember and nothing to be
    /// consistent with.
    async fn read_rows(&self, ids: Vec<MessageId>) -> Result<Vec<MessageSummary>, StoreError> {
        self.read(move |connection| {
            let rows = MessageRepository::new(connection).rows_for(&ids)?;
            let threads = ThreadRepository::new(connection);
            rows.into_iter()
                .map(|row| summarise(row, &threads))
                .collect::<Result<Vec<_>, _>>()
        })
        .await
    }

    async fn read_count(&self, scope: ListScope) -> Result<u32, StoreError> {
        self.read(move |connection| {
            count(
                connection,
                scope,
                &ListQuery {
                    scope,
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
/// `mailboxes.total` column is maintained by triggers on `messages` against
/// `deleted_locally = 0`, which is exactly the list query's own predicate, so
/// the two cannot mean different things.
///
/// The account-wide and flagged views still count: there is no column for
/// them, and neither is on the scrolling hot path.
///
/// # Why zero is not taken at its word
///
/// This number is the list model's `n_items`, and a `GtkListView` over a model
/// of length zero asks for no pages at all — so a cached count that is wrong
/// *low* does not show a wrong number, it shows an empty mailbox. That is
/// `postio-qhz.7`: a live account with 81,716 messages, every cached count
/// still at its `DEFAULT 0` because nothing maintained the column, and a store
/// handing the list fifty real rows and a total of zero in the same read.
///
/// The column has an owner now, so this should never fire. It is here because
/// the two failure modes are not comparable: counting an empty folder is free,
/// counting a full one costs milliseconds off the UI thread, and getting it
/// wrong the other way costs the user their mail with nothing on screen to say
/// so. Any future drift degrades to slow rather than to invisible.
fn count(
    connection: &postio_storage::PooledConnection,
    scope: ListScope,
    query: &ListQuery,
) -> Result<u32, StoreError> {
    if let ListScope::Mailbox(mailbox) = scope
        && let Some(counts) = MailboxRepository::new(connection).counts(mailbox)?
        && counts.total > 0
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
/// The thread query a scope asks for, or why it cannot have one.
///
/// Folders thread; query views list messages (ADR 0015). Flagged and a thread
/// drill-in are not folders, and answering them with conversations would be
/// the wrong answer rather than a missing one — so this refuses instead of
/// quietly picking a scope.
fn thread_query(
    connection: &postio_storage::PooledConnection,
    scope: ListScope,
    limit: u32,
) -> Result<ThreadListQuery, StoreError> {
    match scope {
        ListScope::Mailbox(mailbox) => {
            // One row read to learn whose folder it is. The account is not
            // decoration: `threads.account_id` is the leading column of
            // `idx_threads_account_last_at`, so without it the window has no
            // index to seek and the whole flat-paging argument collapses.
            let account = MailboxRepository::new(connection)
                .get(mailbox)?
                .ok_or_else(|| StoreError::new("That folder is no longer here"))?
                .account_id;
            Ok(ThreadListQuery::in_mailbox(account, mailbox).limit(limit))
        }
        ListScope::Account(account) => Ok(ThreadListQuery::account(account).limit(limit)),
        ListScope::Flagged(_) | ListScope::Snoozed(_) | ListScope::Thread(_) => Err(
            StoreError::new("That view lists messages rather than conversations"),
        ),
    }
}

/// One thread row, as the frontend needs it.
fn summarise_thread(row: ThreadListRow) -> Result<ThreadSummary, StoreError> {
    // A thread with no visible message in scope cannot be drawn, and the
    // query does not produce one -- the representative is what makes the row
    // exist. Reported rather than unwrapped, because "cannot happen" is how
    // panics get shipped.
    let latest = row.latest.ok_or_else(|| {
        StoreError::new("A conversation in this folder has no message to show for it")
    })?;
    Ok(ThreadSummary {
        id: row.id,
        subject: row.subject,
        participants: row.participants,
        message_count: row.message_count.max(1),
        unread_count: row.unread_count,
        flagged: row.is_flagged,
        has_attachments: row.has_attachments,
        last_at: row.last_at,
        representative: MessageSummary {
            id: latest.id,
            thread: latest.thread_id,
            from: latest.from,
            subject: latest.subject,
            preview: latest.preview,
            received_at: latest.received_at,
            seen: latest.seen,
            flagged: latest.flagged,
            answered: latest.answered,
            draft: latest.draft,
            has_attachments: latest.has_attachments,
            thread_count: row.message_count.max(1),
        },
    })
}

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

    fn list_page(&self, request: PageRequest) -> Read<'_, ListPage> {
        Box::pin(self.read_list_page(request))
    }

    fn list_count(&self, scope: ListScope) -> Read<'_, u32> {
        Box::pin(self.read_list_count(scope))
    }

    fn thread_page(&self, request: PageRequest) -> Read<'_, ThreadPage> {
        Box::pin(self.read_thread_page(request))
    }

    fn thread_count(&self, scope: ListScope) -> Read<'_, u32> {
        Box::pin(self.read_thread_count(scope))
    }

    fn message_rows(&self, ids: Vec<MessageId>) -> Read<'_, Vec<MessageSummary>> {
        Box::pin(self.read_rows(ids))
    }

    fn mailboxes(&self, account: AccountId) -> Read<'_, Vec<Mailbox>> {
        Box::pin(self.read_mailboxes(account))
    }
}
