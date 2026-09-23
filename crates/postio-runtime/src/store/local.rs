//! The local store, read directly off the database engine.
//!
//! The half of [`super`] that owns a database. Behind the `runtime` feature
//! because `postio-gtk` depends on `postio-core` and must not have the engine
//! anywhere in its dependency graph; whatever assembles the running
//! application turns the feature on, and the view layer never does.

use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_model::mailbox::Mailbox;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use postio_storage::repository::{
    ListCursor, ListQuery, MailboxRepository, MessageListRow, MessageRepository, ThreadCursor,
    ThreadListQuery, ThreadListRow, ThreadRepository, UnifiedThreadListQuery,
};
use postio_storage::{Checkout, Store};

use crate::store::{
    ListPage, ListRows, ListScope, MailStore, MessagePage, MessageSummary, PageRequest, Read,
    StoreError, ThreadPage, ThreadSummary,
};

impl From<postio_storage::Error> for StoreError {
    fn from(error: postio_storage::Error) -> Self {
        StoreError::new(error.to_string())
    }
}

/// The local store, read directly.
#[derive(Debug, Clone)]
pub struct LocalStore {
    store: Store,
    /// Where each page boundary starts, so a page does not have to be
    /// counted to from the top of the folder every time. See [`Marks`].
    marks: Arc<Mutex<Marks<ListCursor>>>,
    /// The same, for the threaded list. Its own, because a folder that
    /// threads and the same folder listed by message are different windows
    /// with different row counts -- one set of marks would have each read
    /// clearing the other's.
    thread_marks: Arc<Mutex<Marks<ThreadCursor>>>,
    /// The same again, for the unified list. Its own for the reason the
    /// threaded marks have their own: the unified window folds conversations
    /// across accounts, so it is a different list in a different order from
    /// any one account's, and two windows sharing marks would each clear the
    /// other's -- or, where their totals happened to match, seek with one.
    unified_marks: Arc<Mutex<Marks<ThreadCursor>>>,
    /// The last threaded count of a folder, and the cheap number it was taken
    /// against. See [`CountedFolder`].
    folder_counts: Arc<Mutex<HashMap<MailboxId, CountedFolder>>>,
    /// Removals the app has told us about and no read has settled yet:
    /// which folder, which messages. Drained by [`counted_total`] before it
    /// compares its witness, so the count it holds is adjusted rather than
    /// discarded (#1607). See [`MailStore::note_removed`].
    removals: Arc<Mutex<Vec<(MailboxId, Vec<MessageId>)>>>,
}

/// A folder's thread count, and how to tell whether it still holds.
///
/// The count itself is expensive — a correlated subquery, one indexed probe
/// per message, 786 ms on a real 60,907-message Archive. `witness` is the
/// trigger-maintained `mailboxes.total_count` for the same folder: a column
/// lookup, and the number the expensive one follows. When the cheap number is
/// unchanged the expensive one is reused.
///
/// **What this trades, stated plainly.** Messages arriving and being removed
/// between two reads can leave `total_count` equal while the thread count has
/// moved — the same window [`Marks`] already documents itself as having, for
/// the same reason, and with a smaller consequence: a row count briefly off
/// by one, corrected by the next change to the folder. Against that: without
/// it, every page of a large folder pays the full count, which is what made
/// such a folder impossible to open at all (#1534).
#[derive(Debug, Clone, Copy)]
struct CountedFolder {
    /// What the folder looked like when the count was taken — see
    /// [`witness_of`].
    witness: Witness,
    /// What counting the threads answered then.
    threads: u32,
}

/// The cheap facts a folder's thread count is allowed to outlive.
///
/// **Both, and the second one is the lesson.** `total_count` alone misses a
/// sync that re-threads without changing how many messages are in the folder:
/// the count would be stale, and — worse — [`Marks::check`] would see an
/// unchanged total and keep seek marks that no longer point where they say.
/// A mark that is wrong seeks past the end and the page comes back **empty**,
/// which is a list showing nothing on a folder that has 36,000 rows.
///
/// `highest_mod_seq` moves on any server-side change to the folder, which is
/// when threading changes. Together they are strictly stronger than the
/// recomputed total this replaced, rather than merely cheaper than it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Witness {
    /// `mailboxes.total_count`, maintained by the counting triggers.
    messages: u32,
    /// How far the folder has been synced.
    mod_seq: Option<u64>,
}

/// What a folder looks like cheaply: one row, no scan.
fn witness_of(mailbox: &Mailbox) -> Witness {
    Witness {
        messages: mailbox.counts.total,
        mod_seq: mailbox.highest_mod_seq.map(|seq| seq.get()),
    }
}

/// How many times this process has counted a threaded folder.
///
/// A folder's thread count is not a column lookup — it is a correlated
/// subquery, one indexed probe per message — and on a real 60,907-message
/// Archive it measured **786 ms** against a 100 ms interaction budget. What
/// made that fatal rather than merely slow was paying it on *every page*, so
/// the number that matters is not how long a count takes but how many are
/// issued for one folder (#1534).
///
/// An atomic rather than the thread-local `postio_storage` counting uses:
/// these reads happen on the blocking pool, so a thread-local incremented
/// there and read from a test would answer zero.
static FOLDERS_COUNTED: AtomicU64 = AtomicU64::new(0);

/// How many threaded-folder counts this process has issued. For tests.
///
/// See [`FOLDERS_COUNTED`]. Opening and paging one folder should add one.
#[doc(hidden)]
pub fn folders_counted() -> u64 {
    FOLDERS_COUNTED.load(Ordering::Relaxed)
}

/// A folder's thread count, from the cache when the folder has not moved.
///
/// Only for a folder scope. An account-wide or unified count has no single
/// mailbox row to witness it, and is left to be counted as before — those
/// windows are not the ones that cost 786 ms.
async fn counted_total(
    connection: &Checkout,
    cache: &Mutex<HashMap<MailboxId, CountedFolder>>,
    removals: &Mutex<Vec<(MailboxId, Vec<MessageId>)>>,
    scope: ListScope,
    threads: &ThreadRepository<'_>,
    query: &ThreadListQuery,
) -> Result<u32, postio_storage::Error> {
    let ListScope::Mailbox(mailbox) = scope else {
        FOLDERS_COUNTED.fetch_add(1, Ordering::Relaxed);
        return threads.count_of(query).await;
    };
    // The cheap facts, read first: one row, no scan.
    let witness = MailboxRepository::new(connection)
        .get(mailbox)
        .await?
        .as_ref()
        .map(witness_of);

    // What the app said left this folder since the count was taken. An
    // archive moved the witness, and the count would be paid again in
    // front of the first row; instead the rows that left are subtracted
    // -- the conversations with no member left here, and the lone messages
    // -- and the held count moves to the new witness (#1607). Taken out of
    // the queue before any await, and the lock never held across one.
    let pending: Vec<Vec<MessageId>> = {
        let mut removals = removals.lock().expect("not poisoned");
        let (ours, others): (Vec<_>, Vec<_>) = removals
            .drain(..)
            .partition(|(folder, _)| *folder == mailbox);
        *removals = others;
        ours.into_iter().map(|(_, ids)| ids).collect()
    };
    if !pending.is_empty()
        && let Some(witness) = witness
        && cache.lock().expect("not poisoned").contains_key(&mailbox)
    {
        let mut gone = 0u32;
        for ids in &pending {
            gone += threads.conversations_gone_from(mailbox, ids).await?;
        }
        if let Some(held) = cache.lock().expect("not poisoned").get_mut(&mailbox) {
            held.threads = held.threads.saturating_sub(gone);
            held.witness = witness;
        }
    }
    if let Some(witness) = witness
        && let Some(held) = cache.lock().expect("not poisoned").get(&mailbox).copied()
        && held.witness == witness
    {
        return Ok(held.threads);
    }

    FOLDERS_COUNTED.fetch_add(1, Ordering::Relaxed);
    let counted = threads.count_of(query).await?;
    if let Some(witness) = witness {
        cache.lock().expect("not poisoned").insert(
            mailbox,
            CountedFolder {
                witness,
                threads: counted,
            },
        );
    }
    Ok(counted)
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
    /// Forget every mark, keeping the length they were taken against.
    ///
    /// For a caller that has learnt a mark was wrong by using it, rather than
    /// by noticing the list changed length.
    fn forget(&mut self) {
        self.at.clear();
    }

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

impl LocalStore {
    /// Read `store`.
    ///
    /// Cloning a [`LocalStore`] is cheap and gives another handle to the same
    /// store, which is how each read gets a connection of its own.
    pub fn new(store: &Store) -> Self {
        LocalStore {
            store: store.clone(),
            marks: Arc::new(Mutex::new(Marks::default())),
            thread_marks: Arc::new(Mutex::new(Marks::default())),
            unified_marks: Arc::new(Mutex::new(Marks::default())),
            folder_counts: Arc::new(Mutex::new(HashMap::new())),
            removals: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn read_page(&self, request: PageRequest) -> Result<MessagePage, StoreError> {
        let marks = self.marks.clone();
        self.read(move |connection| async move {
            let messages = MessageRepository::new(&connection);
            let query = ListQuery {
                scope: request.scope,
                limit: request.limit,
                after: None,
            };
            // Both from one connection and one moment, so the rows and the
            // number of them cannot disagree.
            let total = count(&connection, request.scope, &query).await?;

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
            let rows = messages.page_at(&query, skip).await?;

            // And remember where the next page begins, so it can seek too.
            if let Some(last) = rows.last() {
                marks
                    .lock()
                    .expect("not poisoned")
                    .remember(request.offset + rows.len() as u32, last.cursor());
            }

            // A loop rather than `map().collect()`: `summarise` reads the
            // thread's participants, so it awaits, and a closure cannot.
            let threads = ThreadRepository::new(&connection);
            let mut summaries = Vec::with_capacity(rows.len());
            for row in rows {
                summaries.push(summarise(row, &threads).await?);
            }
            let rows = summaries;
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
            return Ok(matches!(scope, ListScope::Account(_) | ListScope::Unified));
        };
        self.read(move |connection| async move {
            let folder = MailboxRepository::new(&connection)
                .get(mailbox)
                .await?
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
    /// The same shape as [`LocalStore::read_page`], over the thread window
    /// instead of the message one: count and rows from one connection and one
    /// moment, seek to the nearest boundary anybody has already read, and
    /// remember where this page ended so the next one can seek too.
    async fn read_thread_page(&self, request: PageRequest) -> Result<ThreadPage, StoreError> {
        if matches!(request.scope, ListScope::Unified) {
            return self.read_unified_page(request).await;
        }
        let marks = self.thread_marks.clone();
        let counts = self.folder_counts.clone();
        let removals = self.removals.clone();
        self.read(move |connection| async move {
            let query = thread_query(&connection, request.scope, request.limit).await?;
            let threads = ThreadRepository::new(&connection);
            let total = counted_total(
                &connection,
                &counts,
                &removals,
                request.scope,
                &threads,
                &query,
            )
            .await?;

            let start = {
                let mut marks = marks.lock().expect("not poisoned");
                marks.check(total);
                marks.nearest(request.offset)
            };
            let (seek, skip) = match start {
                Some((at, cursor)) => (Some(cursor), request.offset - at),
                None => (None, request.offset),
            };
            let mut rows = threads
                .page_at(
                    &ThreadListQuery {
                        after: seek,
                        ..query.clone()
                    },
                    skip,
                )
                .await?;

            // An empty page inside a list that says it has rows means the
            // mark we seeked from lied: it claimed a cursor stood at some
            // offset, the rows moved under it, and the seek landed past the
            // end. The page comes back empty and the list shows **nothing**
            // on a folder holding tens of thousands of messages -- which is
            // what "I click into another folder and it just shows empty"
            // turned out to be (#1534).
            //
            // Marks are an optimisation, so the honest response to one that
            // cannot be trusted is to stop trusting all of them and read the
            // way we would have without any. Costly -- this is the deep
            // `OFFSET` the marks exist to avoid -- and it happens once,
            // because the marks are gone afterwards.
            if rows.is_empty() && seek.is_some() && request.offset < total {
                marks.lock().expect("not poisoned").forget();
                rows = threads.page_at(&query, request.offset).await?;
            }

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

    /// One page of the unified list: every account at once, conversations
    /// grouped across them.
    ///
    /// The same seek-mark bargain the account-scoped page makes, and for the
    /// same reason -- the frontend asks for a row offset and the walk can
    /// only resume from a cursor. Absorption is what makes the marks worth
    /// more here than anywhere else: a group is not a fixed number of
    /// threads, so `unified_page_at` has to re-walk from the cursor to find
    /// where row *n* begins, and a remembered mark is what keeps that walk
    /// short.
    async fn read_unified_page(&self, request: PageRequest) -> Result<ThreadPage, StoreError> {
        let marks = self.unified_marks.clone();
        self.read(move |connection| async move {
            let threads = ThreadRepository::new(&connection);
            let total = threads.unified_count().await?;

            let start = {
                let mut marks = marks.lock().expect("not poisoned");
                marks.check(total);
                marks.nearest(request.offset)
            };
            let (seek, skip) = match start {
                Some((at, cursor)) => (Some(cursor), request.offset - at),
                None => (None, request.offset),
            };
            let groups = threads
                .unified_page_at(
                    &UnifiedThreadListQuery {
                        limit: request.limit,
                        after: seek,
                    },
                    skip,
                )
                .await?;
            if let Some(last) = groups.last() {
                marks
                    .lock()
                    .expect("not poisoned")
                    .remember(request.offset + groups.len() as u32, last.cursor());
            }

            let rows = groups
                .into_iter()
                .map(|group| summarise_thread(group.row))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ThreadPage { total, rows })
        })
        .await
    }

    async fn read_thread_count(&self, scope: ListScope) -> Result<u32, StoreError> {
        if matches!(scope, ListScope::Unified) {
            return self
                .read(move |connection| async move {
                    Ok(ThreadRepository::new(&connection).unified_count().await?)
                })
                .await;
        }
        self.read(move |connection| async move {
            let query = thread_query(&connection, scope, 0).await?;
            Ok(ThreadRepository::new(&connection).count_of(&query).await?)
        })
        .await
    }

    /// No seek marks and no count: an explicit id list is not a window into
    /// anything, so there is no position to remember and nothing to be
    /// consistent with.
    /// [`MailStore::rows_in`]: the same scope decision a page makes, then
    /// the same query, with the ids in place of a window. A unified list
    /// pages every account through its own query and has no by-id read
    /// yet; it answers with an error and the list re-reads the page, as it
    /// always did.
    async fn read_rows_in(
        &self,
        scope: ListScope,
        ids: Vec<MessageId>,
    ) -> Result<ListRows, StoreError> {
        if !self.lists_conversations(scope).await? {
            return self.read_rows(ids).await.map(ListRows::Messages);
        }
        if !matches!(scope, ListScope::Mailbox(_) | ListScope::Account(_)) {
            return Err(StoreError::new(
                "a unified list re-reads its page rather than its rows",
            ));
        }
        self.read(move |connection| async move {
            let query = thread_query(&connection, scope, 0).await?;
            ThreadRepository::new(&connection)
                .rows_for(&query, &ids)
                .await?
                .into_iter()
                .map(summarise_thread)
                .collect::<Result<Vec<_>, _>>()
                .map(ListRows::Threads)
        })
        .await
    }

    async fn read_rows(&self, ids: Vec<MessageId>) -> Result<Vec<MessageSummary>, StoreError> {
        self.read(move |connection| async move {
            let rows = MessageRepository::new(&connection).rows_for(&ids).await?;
            let threads = ThreadRepository::new(&connection);
            let mut summaries = Vec::with_capacity(rows.len());
            for row in rows {
                summaries.push(summarise(row, &threads).await?);
            }
            Ok(summaries)
        })
        .await
    }

    async fn read_count(&self, scope: ListScope) -> Result<u32, StoreError> {
        self.read(move |connection| async move {
            count(
                &connection,
                scope,
                &ListQuery {
                    scope,
                    limit: 0,
                    after: None,
                },
            )
            .await
        })
        .await
    }

    async fn read_mailboxes(&self, account: AccountId) -> Result<Vec<Mailbox>, StoreError> {
        self.read(move |connection| async move {
            Ok(MailboxRepository::new(&connection)
                .list_for_account(account)
                .await?)
        })
        .await
    }

    /// Run `read` with a connection of its own.
    ///
    /// # There is no blocking thread any more
    ///
    /// There was: `spawn_blocking`, because `rusqlite` blocked and a blocked
    /// tokio worker is a worker not running everything else. The engine is
    /// async to the bottom now, so the read simply awaits -- no thread pool,
    /// no `T: Send + 'static` on every closure, and no "the read did not
    /// finish" arm for a panic that crossed a thread boundary.
    async fn read<T, F, Fut>(&self, read: F) -> Result<T, StoreError>
    where
        F: FnOnce(Checkout) -> Fut,
        Fut: Future<Output = Result<T, StoreError>>,
    {
        // A turn on a warm reader, not a connection of this read's own: the
        // engine keeps one page cache per connection, so a read that opened
        // its own paid a cold cache and the pragmas every time, and a list
        // page did so twice (#1602). The turn lasts until the read is done.
        let reader = self.store.read().await.map_err(StoreError::from)?;
        let answer = read(reader.checkout()).await;
        drop(reader);
        answer
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
async fn count(
    connection: &Checkout,
    scope: ListScope,
    query: &ListQuery,
) -> Result<u32, StoreError> {
    if let ListScope::Mailbox(mailbox) = scope
        && let Some(counts) = MailboxRepository::new(connection).counts(mailbox).await?
        && counts.total > 0
    {
        return Ok(counts.total);
    }
    Ok(MessageRepository::new(connection).count(query).await?)
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
async fn thread_query(
    connection: &Checkout,
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
                .get(mailbox)
                .await?
                .ok_or_else(|| StoreError::new("That folder is no longer here"))?
                .account_id;
            Ok(ThreadListQuery::in_mailbox(account, mailbox).limit(limit))
        }
        ListScope::Account(account) => Ok(ThreadListQuery::account(account).limit(limit)),
        // The Outbox lists messages, not conversations, like the two views
        // beside it: what is on its way is a set of individual sends, and
        // grouping them into threads would hide two messages going to the
        // same conversation behind one row.
        ListScope::Flagged(_)
        | ListScope::Snoozed(_)
        | ListScope::Outbox(_)
        | ListScope::Thread(_) => Err(StoreError::new(
            "That view lists messages rather than conversations",
        )),
        // Never reached: `read_thread_page` and `read_thread_count` take the
        // unified branch before they get here. It is spelled out rather than
        // left to a catch-all so that a future scope cannot land in this arm
        // by accident.
        ListScope::Unified => Err(StoreError::new(
            "The unified list is not scoped to one account",
        )),
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
            send_state: latest.send_state,
            send_at: latest.send_at,
            has_attachments: latest.has_attachments,
            thread_count: row.message_count.max(1),
        },
    })
}

async fn summarise(
    row: MessageListRow,
    threads: &ThreadRepository<'_>,
) -> Result<MessageSummary, StoreError> {
    let thread_count = match row.thread_id {
        Some(id) => threads
            .get(id)
            .await?
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
        send_state: row.send_state,
        send_at: row.send_at,
        has_attachments: row.has_attachments,
        thread_count: thread_count.max(1),
    })
}

/// The two windows underneath [`MailStore::list_page`], for callers that
/// mean one of them specifically.
///
/// Not part of [`MailStore`]: a frontend asks for "the list, however this
/// scope lists itself" and never chooses a window, so putting these on the
/// trait made every fake implement two reads nothing would call. The tests
/// and benches that measure one window reach the concrete store.
impl LocalStore {
    /// One page of the flat message list, with the count that page was read
    /// against.
    pub fn message_page(&self, request: PageRequest) -> Read<'_, MessagePage> {
        Box::pin(self.read_page(request))
    }

    /// How many rows the flat list would show, without reading any of them.
    pub fn message_count(&self, scope: ListScope) -> Read<'_, u32> {
        Box::pin(self.read_count(scope))
    }

    /// One page of the *threaded* list, with the count it was read against.
    ///
    /// A real folder threads and a query view does not (ADR 0015), so this
    /// answers only [`ListScope::Mailbox`] and [`ListScope::Account`];
    /// anything else is a caller asking the wrong question and comes back as
    /// an error rather than as message rows wearing a hat.
    pub fn thread_page(&self, request: PageRequest) -> Read<'_, ThreadPage> {
        Box::pin(self.read_thread_page(request))
    }

    /// How many conversations the threaded list would show.
    pub fn thread_count(&self, scope: ListScope) -> Read<'_, u32> {
        Box::pin(self.read_thread_count(scope))
    }
}

impl MailStore for LocalStore {
    fn list_page(&self, request: PageRequest) -> Read<'_, ListPage> {
        Box::pin(self.read_list_page(request))
    }

    fn list_count(&self, scope: ListScope) -> Read<'_, u32> {
        Box::pin(self.read_list_count(scope))
    }

    fn rows_in(&self, scope: ListScope, ids: Vec<MessageId>) -> Read<'_, ListRows> {
        Box::pin(self.read_rows_in(scope, ids))
    }

    fn note_removed(&self, mailbox: MailboxId, messages: Vec<MessageId>) {
        self.removals
            .lock()
            .expect("not poisoned")
            .push((mailbox, messages));
    }

    fn message_rows(&self, ids: Vec<MessageId>) -> Read<'_, Vec<MessageSummary>> {
        Box::pin(self.read_rows(ids))
    }

    fn mailboxes(&self, account: AccountId) -> Read<'_, Vec<Mailbox>> {
        Box::pin(self.read_mailboxes(account))
    }

    fn draft_counts(
        &self,
        account: AccountId,
    ) -> Read<'_, postio_storage::repository::DraftCounts> {
        Box::pin(self.read(move |connection| async move {
            Ok(MailboxRepository::new(&connection)
                .draft_counts(account)
                .await?)
        }))
    }
}
