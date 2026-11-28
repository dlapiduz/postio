//! The local store read directly off the database engine, behind the `runtime` feature so the view
//! layer's graph never has the engine.

use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_model::mailbox::Mailbox;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use postio_model::FocusScope;
use postio_storage::repository::{
    DraftRepository, FocusListQuery, ListCursor, ListQuery, MailboxRepository, MessageListRow,
    MessageRepository, ThreadCursor, ThreadGroup, ThreadListQuery, ThreadListRow, ThreadRepository,
    UnifiedThreadListQuery,
};
use postio_storage::{Checkout, Store};

use crate::store::{
    ListPage, ListRows, ListScope, MailStore, MessagePage, MessageSummary, PageRequest, Read,
    StoreError, ThreadPage, ThreadSummary,
};

type Removals = Mutex<Vec<(MailboxId, Vec<MessageId>)>>;

#[derive(Debug, Clone)]
pub struct LocalStore {
    store: Store,
    marks: Arc<Mutex<Marks<ListCursor>>>,
    /// Its own: a threaded and a flat window of one folder have different row counts, and shared
    /// marks would clear each other's.
    thread_marks: Arc<Mutex<Marks<ThreadCursor>>>,
    /// Its own: the unified window folds conversations across accounts, and shared marks would
    /// clear or wrongly seek with each other.
    unified_marks: Arc<Mutex<Marks<ThreadCursor>>>,
    unified_count: Arc<Mutex<Option<(UnifiedWitness, u32)>>>,
    focus_marks: Arc<Mutex<Marks<ThreadCursor>>>,
    focus_count: Arc<Mutex<Option<FocusCounted>>>,
    folder_counts: Arc<Mutex<HashMap<MailboxId, CountedFolder>>>,
    /// Drained by `counted_total` before it compares its witness, so the held count is adjusted
    /// rather than discarded (#1607).
    removals: Arc<Removals>,
}

/// The count is a correlated subquery (786 ms on a 60,907-message Archive); `witness` is the cheap
/// trigger-maintained column it follows. An arrival plus a removal between two reads can leave
/// `total_count` equal while the thread count moved: a row count briefly off by one, against every
/// page paying the full count (#1534).
#[derive(Debug, Clone, Copy)]
struct CountedFolder {
    witness: Witness,
    threads: u32,
}

/// Both facts, because `total_count` alone misses a sync that re-threads without changing the
/// count: stale marks would seek past the end and return an empty page. `highest_mod_seq` moves on
/// any server-side change, which is when threading changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Witness {
    messages: u32,
    mod_seq: Option<u64>,
    splits: Splits,
}

fn witness_of(mailbox: &Mailbox, splits: Splits) -> Witness {
    Witness {
        messages: mailbox.counts.total,
        mod_seq: mailbox.highest_mod_seq.map(|seq| seq.get()),
        splits,
    }
}

type Splits = (i64, i64, i64);

/// A split regroups messages without moving one, so no folder's counts witness it (#2259).
async fn splits_witness(connection: &Checkout) -> Result<Splits, postio_storage::Error> {
    let splits = postio_storage::sql::first(
        connection,
        "SELECT -1, count(*), coalesce(max(split_at), 0) FROM thread_splits",
        (),
        |row| {
            use postio_storage::sql::RowExt as _;
            Ok((row.col(0)?, row.col(1)?, row.col(2)?))
        },
    )
    .await?;
    Ok(splits.unwrap_or((-1, 0, 0)))
}

/// A thread count is a correlated subquery (786 ms on a 60,907-message Archive), so what matters is
/// how many are issued per folder (#1534). An atomic, not `postio_storage`'s thread-local: these
/// reads run on the blocking pool.
static FOLDERS_COUNTED: AtomicU64 = AtomicU64::new(0);

#[doc(hidden)]
pub fn folders_counted() -> u64 {
    FOLDERS_COUNTED.load(Ordering::Relaxed)
}

#[doc(hidden)]
pub fn unified_counted() -> u64 {
    UNIFIED_COUNTED.load(Ordering::Relaxed)
}

static UNIFIED_COUNTED: AtomicU64 = AtomicU64::new(0);

#[doc(hidden)]
pub fn focus_counted() -> u64 {
    FOCUS_COUNTED.load(Ordering::Relaxed)
}

static FOCUS_COUNTED: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
struct FocusCounted {
    witness: UnifiedWitness,
    inboxes: Vec<(AccountId, MailboxId)>,
    total: u32,
}

async fn inbox_witness(connection: &Checkout) -> Result<UnifiedWitness, postio_storage::Error> {
    postio_storage::sql::all(
        connection,
        "SELECT m.id, m.total_count, coalesce(s.highest_mod_seq, 0)
           FROM accounts a JOIN mailboxes m
             ON m.account_id = a.id AND m.role = 'inbox'
           LEFT JOIN sync_state s ON s.mailbox_id = m.id
          WHERE a.enabled = 1 AND a.pending_deletion = 0 AND m.selectable = 1
         UNION ALL
         SELECT -1, (SELECT count(*) FROM thread_splits),
                (SELECT coalesce(max(split_at), 0) FROM thread_splits)
          ORDER BY 1",
        (),
        |row| {
            use postio_storage::sql::RowExt as _;
            Ok((row.col(0)?, row.col(1)?, row.col(2)?))
        },
    )
    .await
}

/// The unified witness plus a row for what Focus holds back, which moves no folder: a count kept
/// on folder state alone would outlive its rows and seek marks (spec 007).
async fn focus_witness(connection: &Checkout) -> Result<UnifiedWitness, postio_storage::Error> {
    postio_storage::sql::all(
        connection,
        "SELECT m.id, m.total_count, coalesce(s.highest_mod_seq, 0)
           FROM accounts a JOIN mailboxes m
             ON m.account_id = a.id AND m.role = 'inbox'
           LEFT JOIN sync_state s ON s.mailbox_id = m.id
          WHERE a.enabled = 1 AND a.pending_deletion = 0 AND m.selectable = 1
         UNION ALL
         SELECT 0, (SELECT count(*) FROM digest_holds),
                (SELECT count(*) FROM digest_deliveries WHERE archived_at IS NOT NULL)
         UNION ALL
         SELECT -1, (SELECT count(*) FROM thread_splits),
                (SELECT coalesce(max(split_at), 0) FROM thread_splits)
          ORDER BY 1",
        (),
        |row| {
            use postio_storage::sql::RowExt as _;
            Ok((row.col(0)?, row.col(1)?, row.col(2)?))
        },
    )
    .await
}

async fn focus_inbox(
    connection: &Checkout,
    cache: &Mutex<Option<FocusCounted>>,
    threads: &ThreadRepository<'_>,
) -> Result<(Vec<(AccountId, MailboxId)>, u32), postio_storage::Error> {
    let witness = focus_witness(connection).await?;
    if let Some(held) = &*cache.lock().expect("not poisoned")
        && held.witness == witness
    {
        return Ok((held.inboxes.clone(), held.total));
    }
    FOCUS_COUNTED.fetch_add(1, Ordering::Relaxed);
    let inboxes = threads.unified_inboxes().await?;
    let total = threads.focus_count(&inboxes).await?;
    *cache.lock().expect("not poisoned") = Some(FocusCounted {
        witness,
        inboxes: inboxes.clone(),
        total,
    });
    Ok((inboxes, total))
}

/// Per inbox, because the list is the inboxes (#1692): an archive moves a message out of an inbox
/// and leaves the summed totals unchanged, and turning an account off moves no folder at all.
type UnifiedWitness = Vec<(i64, i64, i64)>;

async fn unified_total(
    connection: &Checkout,
    cache: &Mutex<Option<(UnifiedWitness, u32)>>,
    threads: &ThreadRepository<'_>,
) -> Result<u32, postio_storage::Error> {
    let witness = inbox_witness(connection).await?;
    if let Some((held, total)) = &*cache.lock().expect("not poisoned")
        && *held == witness
    {
        return Ok(*total);
    }
    UNIFIED_COUNTED.fetch_add(1, Ordering::Relaxed);
    let total = threads.unified_count().await?;
    *cache.lock().expect("not poisoned") = Some((witness, total));
    Ok(total)
}

/// For tests: rows skipped with `OFFSET` after the seek; a scrollbar jump should skip under a page
/// (#1610).
#[doc(hidden)]
pub fn last_thread_skip() -> u64 {
    LAST_THREAD_SKIP.load(Ordering::Relaxed)
}

static LAST_THREAD_SKIP: AtomicU64 = AtomicU64::new(0);

/// Only for a folder scope: an account-wide or unified count has no single mailbox row to witness
/// it.
async fn counted_total(
    connection: &Checkout,
    cache: &Mutex<HashMap<MailboxId, CountedFolder>>,
    removals: &Removals,
    scope: ListScope,
    threads: &ThreadRepository<'_>,
    query: &ThreadListQuery,
) -> Result<u32, postio_storage::Error> {
    let ListScope::Mailbox(mailbox) = scope else {
        FOLDERS_COUNTED.fetch_add(1, Ordering::Relaxed);
        return threads.count_of(query).await;
    };
    let splits = splits_witness(connection).await?;
    let witness = MailboxRepository::new(connection)
        .get(mailbox)
        .await?
        .as_ref()
        .map(|mailbox| witness_of(mailbox, splits));

    // Subtract what the app said left this folder (an archive moved the witness) instead of paying
    // the count again (#1607); taken out of the queue before any await.
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

/// `OFFSET` walks the rows it skips (24 ms halfway down 100,000 messages), so the store remembers
/// where each read page began and seeks. A jump to an unvisited page falls back to the nearest mark
/// below it. The total is checked on every read and a change drops every mark; one arrival plus one
/// deletion leaves it equal and a page off by a row, as plain `OFFSET` would.
#[derive(Debug)]
struct Marks<C> {
    total: Option<u32>,
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
    fn forget(&mut self) {
        self.at.clear();
    }

    fn check(&mut self, total: u32) {
        if self.total != Some(total) {
            self.at.clear();
            self.total = Some(total);
        }
    }

    fn nearest(&self, offset: u32) -> Option<(u32, C)> {
        self.at
            .range(..=offset)
            .next_back()
            .map(|(at, cursor)| (*at, *cursor))
    }

    /// Remember where the page after `offset` begins.
    fn remember(&mut self, offset: u32, cursor: C) {
        // Bounded: 2,000 marks for 100,000 messages at 50 a page, two integers each.
        self.at.insert(offset, cursor);
    }
}

impl LocalStore {
    pub fn new(store: &Store) -> Self {
        LocalStore {
            store: store.clone(),
            marks: Arc::new(Mutex::new(Marks::default())),
            thread_marks: Arc::new(Mutex::new(Marks::default())),
            unified_marks: Arc::new(Mutex::new(Marks::default())),
            unified_count: Arc::new(Mutex::new(None)),
            focus_marks: Arc::new(Mutex::new(Marks::default())),
            focus_count: Arc::new(Mutex::new(None)),
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
            // One connection and one moment, so the rows and their count cannot disagree.
            let total = count(&connection, request.scope, &query).await?;

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

            if let Some(last) = rows.last() {
                marks
                    .lock()
                    .expect("not poisoned")
                    .remember(request.offset + rows.len() as u32, last.cursor());
            }

            let mut rows: Vec<MessageSummary> = rows.into_iter().map(summarise).collect();
            // A draft or queued message is from the person themselves, so
            // its row names who it is to instead.
            let writing: Vec<_> = rows
                .iter()
                .filter(|row| row.send_state.is_some())
                .map(|row| row.id)
                .collect();
            if !writing.is_empty() {
                let mut to = DraftRepository::new(&connection)
                    .recipients_of(&writing)
                    .await?;
                for row in &mut rows {
                    row.to = to.remove(&row.id).unwrap_or_default();
                }
            }
            Ok(MessagePage { total, rows })
        })
        .await
    }

    /// Drafts does not thread: a draft is a document being written, and two drafts answering one
    /// thread would collapse into one row (ADR 0015).
    async fn lists_conversations(&self, scope: ListScope) -> Result<bool, StoreError> {
        let ListScope::Mailbox(mailbox) = scope else {
            return Ok(match scope {
                ListScope::Focus(FocusScope::Snoozed | FocusScope::Flagged) => false,
                _ => matches!(
                    scope,
                    ListScope::Account(_) | ListScope::Unified | ListScope::Focus(_)
                ),
            });
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

    async fn read_thread_page(&self, request: PageRequest) -> Result<ThreadPage, StoreError> {
        if matches!(request.scope, ListScope::Unified) {
            return self.read_unified_page(request).await;
        }
        if let ListScope::Focus(scope) = request.scope {
            return self.read_focus_page(scope, request).await;
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
            let (mut seek, mut skip) = match start {
                Some((at, cursor)) => (Some(cursor), request.offset - at),
                None => (None, request.offset),
            };
            // A scrollbar jump deep into a folder would be a linear `OFFSET` walk over the
            // correlated window (638 ms for
            // 17,804 conversations); one pass over the index finds every page start (#1610).
            let stride = request.limit.max(1);
            if skip > stride && matches!(request.scope, ListScope::Mailbox(_)) {
                let boundaries = threads.boundaries(&query, stride).await?;
                let mut marks = marks.lock().expect("not poisoned");
                marks.check(total);
                for (offset, cursor) in boundaries {
                    marks.remember(offset, cursor);
                }
                if let Some((at, cursor)) = marks.nearest(request.offset) {
                    seek = Some(cursor);
                    skip = request.offset - at;
                }
            }
            LAST_THREAD_SKIP.store(u64::from(skip), Ordering::Relaxed);
            let mut rows = threads
                .page_at(
                    &ThreadListQuery {
                        after: seek,
                        ..query.clone()
                    },
                    skip,
                )
                .await?;

            // An empty page in a list that says it has rows means the seek mark lied (#1534); marks
            // are only an optimisation, so stop trusting all of them and read without them, once.
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

    /// A group is not a fixed number of threads, so `unified_page_at` re-walks from the cursor and
    /// a remembered mark keeps that short (#1692).
    async fn read_unified_page(&self, request: PageRequest) -> Result<ThreadPage, StoreError> {
        let marks = self.unified_marks.clone();
        let cache = self.unified_count.clone();
        self.read(move |connection| async move {
            let threads = ThreadRepository::new(&connection);
            let total = unified_total(&connection, &cache, &threads).await?;

            let start = {
                let mut marks = marks.lock().expect("not poisoned");
                marks.check(total);
                marks.nearest(request.offset)
            };
            let (seek, skip) = match start {
                Some((at, cursor)) => (Some(cursor), request.offset - at),
                None => (None, request.offset),
            };
            let mut groups = threads
                .unified_page_at(
                    &UnifiedThreadListQuery {
                        limit: request.limit,
                        after: seek,
                    },
                    skip,
                )
                .await?;
            // A mark the rows moved under (#1534): read from the top once.
            if groups.is_empty() && seek.is_some() && request.offset < total {
                marks.lock().expect("not poisoned").forget();
                groups = threads
                    .unified_page_at(
                        &UnifiedThreadListQuery {
                            limit: request.limit,
                            after: None,
                        },
                        request.offset,
                    )
                    .await?;
            }
            if let Some(last) = groups.last() {
                marks
                    .lock()
                    .expect("not poisoned")
                    .remember(request.offset + groups.len() as u32, last.cursor());
            }

            let rows = groups
                .into_iter()
                .map(summarise_group)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ThreadPage { total, rows })
        })
        .await
    }

    async fn read_focus_page(
        &self,
        scope: FocusScope,
        request: PageRequest,
    ) -> Result<ThreadPage, StoreError> {
        if scope == FocusScope::HasAction {
            return self.read_has_action_page(request).await;
        }
        let marks = self.focus_marks.clone();
        let cache = self.focus_count.clone();
        self.read(move |connection| async move {
            let threads = ThreadRepository::new(&connection);
            let (inboxes, total) = focus_inbox(&connection, &cache, &threads).await?;

            let start = {
                let mut marks = marks.lock().expect("not poisoned");
                marks.check(total);
                marks.nearest(request.offset)
            };
            let (seek, skip) = match start {
                Some((at, cursor)) => (Some(cursor), request.offset - at),
                None => (None, request.offset),
            };
            let query = FocusListQuery {
                inboxes,
                limit: request.limit,
                after: seek,
            };
            let mut groups = threads.focus_page_at(&query, skip).await?;
            if groups.is_empty() && seek.is_some() && request.offset < total {
                marks.lock().expect("not poisoned").forget();
                groups = threads
                    .focus_page_at(
                        &FocusListQuery {
                            after: None,
                            ..query
                        },
                        request.offset,
                    )
                    .await?;
            }
            if let Some(last) = groups.last() {
                marks
                    .lock()
                    .expect("not poisoned")
                    .remember(request.offset + groups.len() as u32, last.cursor());
            }

            let rows = groups
                .into_iter()
                .map(summarise_group)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ThreadPage { total, rows })
        })
        .await
    }

    /// Read from the markers, not by walking the inbox (spec 007).
    async fn read_has_action_page(&self, request: PageRequest) -> Result<ThreadPage, StoreError> {
        self.read(move |connection| async move {
            let threads = ThreadRepository::new(&connection);
            let inboxes = threads.unified_inboxes().await?;
            let marked = threads.focus_marked(&inboxes).await?;
            let groups = threads
                .focus_marked_page(&inboxes, &marked, request.offset, request.limit)
                .await?;
            let rows = groups
                .into_iter()
                .map(summarise_group)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ThreadPage {
                total: marked.len(),
                rows,
            })
        })
        .await
    }

    async fn read_thread_count(&self, scope: ListScope) -> Result<u32, StoreError> {
        if let ListScope::Focus(FocusScope::HasAction) = scope {
            return self
                .read(move |connection| async move {
                    let threads = ThreadRepository::new(&connection);
                    let inboxes = threads.unified_inboxes().await?;
                    Ok(threads.focus_marked(&inboxes).await?.len())
                })
                .await;
        }
        if let ListScope::Focus(FocusScope::Inbox) = scope {
            let cache = self.focus_count.clone();
            return self
                .read(move |connection| async move {
                    let threads = ThreadRepository::new(&connection);
                    Ok(focus_inbox(&connection, &cache, &threads).await?.1)
                })
                .await;
        }
        if matches!(scope, ListScope::Unified) {
            let cache = self.unified_count.clone();
            return self
                .read(move |connection| async move {
                    let threads = ThreadRepository::new(&connection);
                    Ok(unified_total(&connection, &cache, &threads).await?)
                })
                .await;
        }
        self.read(move |connection| async move {
            let query = thread_query(&connection, scope, 0).await?;
            Ok(ThreadRepository::new(&connection).count_of(&query).await?)
        })
        .await
    }

    /// An explicit id list is no window, so no seek marks and no count. A unified list has no by-id
    /// read yet and answers with an error.
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
            Ok(rows.into_iter().map(summarise).collect())
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

    async fn read<T, F, Fut>(&self, read: F) -> Result<T, StoreError>
    where
        F: FnOnce(Checkout) -> Fut,
        Fut: Future<Output = Result<T, StoreError>>,
    {
        // A turn on a warm reader: the engine keeps one page cache per connection, so a read with
        // its own paid a cold cache every time (#1602).
        let reader = self.store.read().await.map_err(StoreError::from)?;
        let answer = read(reader.checkout()).await;
        drop(reader);
        answer
    }
}

/// A single mailbox answers from its trigger-maintained count, not `count(*)`, which is linear and
/// was paid on every page; Flagged and Snoozed sum per-folder counts (#1614). Zero is not taken at
/// its word: a `GtkListView` over a model of length zero asks for no pages, so a count wrong low
/// shows an empty mailbox. Counting an empty folder is free; being wrong shows nothing on screen.
async fn count(
    connection: &Checkout,
    scope: ListScope,
    query: &ListQuery,
) -> Result<u32, StoreError> {
    let folders = MailboxRepository::new(connection);
    let cached = match scope {
        ListScope::Mailbox(mailbox) => folders.counts(mailbox).await?.map(|counts| counts.total),
        ListScope::Flagged(account) => Some(folders.account_counts(account).await?.flagged),
        ListScope::Snoozed(account) => Some(folders.account_counts(account).await?.snoozed),
        // Account and unified views list conversations; the Outbox is a predicate over Drafts and a
        // thread drill-in is one conversation: no column.
        ListScope::Account(_)
        | ListScope::Unified
        | ListScope::Outbox(_)
        | ListScope::Thread(_)
        | ListScope::Focus(_) => None,
    };
    if let Some(total) = cached.filter(|total| *total > 0) {
        return Ok(total);
    }
    Ok(MessageRepository::new(connection).count(query).await?)
}

/// The thread query a scope asks for, or why it cannot have one. Folders thread, query views list
/// messages (ADR 0015);
/// answering Flagged or a drill-in with conversations would be wrong, so this refuses.
async fn thread_query(
    connection: &Checkout,
    scope: ListScope,
    limit: u32,
) -> Result<ThreadListQuery, StoreError> {
    match scope {
        ListScope::Mailbox(mailbox) => {
            // The account leads `idx_threads_account_last_at`; without it the window has no index
            // to seek.
            let account = MailboxRepository::new(connection)
                .get(mailbox)
                .await?
                .ok_or_else(|| StoreError::new("That folder is no longer here"))?
                .account_id;
            Ok(ThreadListQuery::in_mailbox(account, mailbox).limit(limit))
        }
        ListScope::Account(account) => Ok(ThreadListQuery::account(account).limit(limit)),
        // The Outbox lists individual sends; threading them would hide two messages to one
        // conversation behind one row.
        ListScope::Flagged(_)
        | ListScope::Snoozed(_)
        | ListScope::Outbox(_)
        | ListScope::Thread(_) => Err(StoreError::new(
            "That view lists messages rather than threads",
        )),
        // Never reached (`read_thread_page` takes the unified branch first); spelled out so a
        // future scope cannot land here by accident.
        ListScope::Unified => Err(StoreError::new(
            "The unified list is not scoped to one account",
        )),
        ListScope::Focus(_) => Err(StoreError::new(
            "Focus's lists are not scoped to one account",
        )),
    }
}

fn summarise_group(group: ThreadGroup) -> Result<ThreadSummary, StoreError> {
    let head = group.row.id;
    let mut summary = summarise_thread(group.row)?;
    summary.copies = group
        .members
        .into_iter()
        .map(|(_, thread)| thread)
        .filter(|thread| Some(*thread) != head)
        .collect();
    Ok(summary)
}

fn summarise_thread(row: ThreadListRow) -> Result<ThreadSummary, StoreError> {
    // The query never yields a thread with no visible message; reported rather than unwrapped.
    let latest = row
        .latest
        .ok_or_else(|| StoreError::new("A thread in this folder has no message to show for it"))?;
    Ok(ThreadSummary {
        id: row.id,
        subject: row.subject,
        participants: row.participants,
        message_count: row.message_count.max(1),
        unread_count: row.unread_count,
        flagged: row.is_flagged,
        has_attachments: row.has_attachments,
        last_at: row.last_at,
        marker: row.marker,
        copies: Vec::new(),
        muted: row.muted,
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
            to: Vec::new(),
        },
    })
}

fn summarise(row: MessageListRow) -> MessageSummary {
    let thread_count = row.thread_count.unwrap_or(1);
    MessageSummary {
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
        to: Vec::new(),
    }
}

/// Not on `MailStore`: a frontend never chooses a window, and putting these on the trait made every
/// fake implement two reads.
impl LocalStore {
    pub fn message_page(&self, request: PageRequest) -> Read<'_, MessagePage> {
        Box::pin(self.read_page(request))
    }

    pub fn message_count(&self, scope: ListScope) -> Read<'_, u32> {
        Box::pin(self.read_count(scope))
    }

    /// Answers only `Mailbox` and `Account` scopes (ADR 0015); anything else is an error, not
    /// message rows.
    pub fn thread_page(&self, request: PageRequest) -> Read<'_, ThreadPage> {
        Box::pin(self.read_thread_page(request))
    }

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
