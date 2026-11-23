//! Filing a message into a thread: the rule lives in [`postio_model::threading`]; this is the
//! [`ThreadIndex`] it asks and the writes that follow.
//! O(thread): lookups are against `thread_links`, primary-keyed on `(account_id, rfc_message_id)`,
//! one query for the whole reference chain; a test asserts it by counting statements at two mailbox
//! sizes. A thread claims ids it has never seen because an initial sync walks newest-first and a
//! reply arrives before its parent.
//! A message with no references is placed only by the subject fallback and never revisited at
//! insert; [`ThreadingRepository::reconsider`] and [`ThreadingRepository::rethread_orphans`] repair
//! that.

use chrono::{DateTime, Utc};
use postio_model::{
    AccountId, Assignment, MailboxId, Message, RfcMessageId, Thread, ThreadCue, ThreadId,
    ThreadIndex, assign, claimed_ids,
};

use super::{MessageRepository, ThreadRepository};

use crate::error::Result;
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;

#[derive(Debug)]
pub struct ThreadingRepository<'a> {
    connection: &'a Connection,
    account_id: AccountId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Threaded {
    pub thread_id: ThreadId,
    pub created: bool,
    /// Non-empty means a conversation the user saw as two is now one, so the list redraws more than
    /// one row.
    pub merged: Vec<ThreadId>,
}

impl<'a> ThreadingRepository<'a> {
    /// Threading never crosses accounts: two people can hold the same `Message-ID`.
    pub fn new(connection: &'a Connection, account_id: AccountId) -> Self {
        Self {
            connection,
            account_id,
        }
    }

    /// Writes `messages.thread_id`. Idempotent for a message already filed.
    pub async fn thread(&self, message: &Message) -> Result<Threaded> {
        let cue = ThreadCue::of(message);
        sql::in_scope(self.connection, |scope| async move {
            let index = LoadedIndex::load(&scope, self.account_id, &cue, None).await?;
            let assignment = assign(&cue, &index);

            let result = self
                .apply(
                    &scope,
                    message,
                    &cue,
                    assignment,
                    index.joined_across_a_split(),
                )
                .await?;
            Ok(result)
        })
        .await
    }

    async fn apply(
        &self,
        connection: &Connection,
        message: &Message,
        cue: &ThreadCue,
        assignment: Assignment,
        across_a_split: bool,
    ) -> Result<Threaded> {
        let threads = ThreadRepository::new(connection);

        let (thread_id, created, merged) = match assignment {
            Assignment::New => {
                let mut thread = Thread::new(self.account_id);
                thread.subject = (!cue.subject.is_empty()).then(|| cue.subject.clone());
                (threads.create(&mut thread).await?, true, Vec::new())
            }
            Assignment::Join(id) => (id, false, Vec::new()),
            Assignment::Merge { into, absorb } => {
                for other in &absorb {
                    // Relink before merging: `merge` deletes the absorbed thread and
                    // `thread_links.thread_id` cascades, so the other order drops every id it
                    // claimed.
                    self.relink(connection, *other, into).await?;
                    threads.merge(into, *other).await?;
                }
                (into, false, absorb)
            }
        };

        threads.add_message(thread_id, message.id).await?;
        for id in claimed_ids(cue) {
            if across_a_split {
                // Joined the split thread by its nearest link: the ids its
                // References name in the original stay the original's.
                self.claim_if_free(connection, id, thread_id).await?;
            } else {
                self.claim(connection, id, thread_id).await?;
            }
        }

        Ok(Threaded {
            thread_id,
            created,
            merged,
        })
    }

    /// Last claim wins: only when two threads already merged or a message is re-filed, and the
    /// newer answer is right.
    async fn claim(
        &self,
        connection: &Connection,
        id: &RfcMessageId,
        thread_id: ThreadId,
    ) -> Result<()> {
        // Cached: run once per id every message claims, one of the statements a first sync compiles
        // hundreds of thousands of times (#728).
        sql::statement(
            connection,
            "INSERT INTO thread_links (account_id, rfc_message_id, thread_id)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (account_id, rfc_message_id) DO UPDATE
                SET thread_id = excluded.thread_id",
        )
        .await?
        .execute(bind![self.account_id.get(), id.folded(), thread_id.get()])
        .await?;
        Ok(())
    }

    async fn claim_if_free(
        &self,
        connection: &Connection,
        id: &RfcMessageId,
        thread_id: ThreadId,
    ) -> Result<()> {
        sql::statement(
            connection,
            "INSERT INTO thread_links (account_id, rfc_message_id, thread_id)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (account_id, rfc_message_id) DO NOTHING",
        )
        .await?
        .execute(bind![self.account_id.get(), id.folded(), thread_id.get()])
        .await?;
        Ok(())
    }

    /// `OR REPLACE`: the two threads may claim the same id, often why they merged, and the survivor
    /// points at the surviving thread.
    async fn relink(
        &self,
        connection: &Connection,
        absorbed: ThreadId,
        into: ThreadId,
    ) -> Result<()> {
        sql::execute(
            connection,
            "UPDATE OR REPLACE thread_links SET thread_id = ?2 WHERE thread_id = ?1",
            bind![absorbed.get(), into.get()],
        )
        .await?;
        Ok(())
    }

    pub async fn claims(&self, thread_id: ThreadId) -> Result<Vec<RfcMessageId>> {
        let mut statement = sql::statement(
            self.connection,
            "SELECT rfc_message_id FROM thread_links
              WHERE account_id = ?1 AND thread_id = ?2 ORDER BY rfc_message_id",
        )
        .await?;
        let rows = sql::mapped(
            &mut statement,
            bind![self.account_id.get(), thread_id.get()],
            |row| row.col::<String>(0),
        )
        .await?;
        Ok(rows.into_iter().map(RfcMessageId::new).collect())
    }

    pub async fn thread_of(&self, id: &RfcMessageId) -> Result<Option<ThreadId>> {
        sql::first(
            self.connection,
            // Binary equality against the folded key: Turso's planner will not bind an equality
            // through a `COLLATE NOCASE` index column, walking every link the account has once per
            // message (#1587). Case-insensitivity lives in `RfcMessageId::folded`.
            "SELECT thread_id FROM thread_links
              WHERE account_id = ?1 AND rfc_message_id = ?2",
            bind![self.account_id.get(), id.folded()],
            |row| Ok(ThreadId::new(row.col(0)?)),
        )
        .await
    }

    /// Only acts when `message` is the sole member of its thread, the signature of a subject-only
    /// orphan: moving a thread's other members on a match this message's cue cannot vouch for could
    /// merge threads that merely share a subject, which `is_reply` exists to prevent. A cheap no-op
    /// otherwise.
    pub async fn reconsider(&self, message: &Message) -> Result<Option<Threaded>> {
        let Some(current) = message.thread_id else {
            return Ok(None);
        };

        let threads = ThreadRepository::new(self.connection);
        let Some(current_thread) = threads.get(current).await? else {
            return Ok(None);
        };
        if current_thread.message_count != 1 {
            return Ok(None);
        }

        let cue = ThreadCue::of(message);
        sql::in_scope(self.connection, |scope| async move {
            // `current` was created for this message, so its own row matches `cue`'s subject and
            // would answer `assign` with itself; excluding it makes "nothing better"
            // ([`Assignment::New`]) distinguishable from "still itself".
            let index = LoadedIndex::load(&scope, self.account_id, &cue, Some(current)).await?;
            let assignment = assign(&cue, &index);
            if matches!(assignment, Assignment::New) {
                return Ok(None);
            }

            let mut result = self
                .apply(
                    &scope,
                    message,
                    &cue,
                    assignment,
                    index.joined_across_a_split(),
                )
                .await?;
            // `current`'s only member just left, so fold the empty thread away, claims included,
            // rather than leaving a zero-message row for the list to filter.
            self.relink(&scope, current, result.thread_id).await?;
            ThreadRepository::new(&scope)
                .merge(result.thread_id, current)
                .await?;
            result.merged.push(current);

            Ok(Some(result))
        })
        .await
    }

    /// Run when a mailbox's sync goes idle, not on every insert: unlike
    /// [`ThreadingRepository::thread`] this scans (postio-tn9.2).
    pub async fn rethread_orphans(&self, mailbox_id: MailboxId) -> Result<usize> {
        let messages = MessageRepository::new(self.connection);
        let mut moved = 0;
        for id in messages.subject_only_orphans(mailbox_id).await? {
            let Some(message) = messages.get(id).await? else {
                continue;
            };
            if self.reconsider(&message).await?.is_some() {
                moved += 1;
            }
        }
        Ok(moved)
    }
}

/// Loaded before the decision because [`ThreadIndex`] is synchronous and lives in `postio-model`,
/// which must stay free of databases so the rule is testable against two maps. Everything `assign`
/// can ask is derivable from the cue, so the links and the one subject are loaded first: fewer
/// round trips than a lookup per reference (#728).
#[derive(Debug, Default)]
struct LoadedIndex {
    by_id: std::collections::HashMap<String, ThreadId>,
    /// Which found threads came from a split (spec 013). Read only when the links found more than
    /// one thread, so the common message pays nothing.
    splits: std::collections::HashSet<ThreadId>,
    by_subject: Option<ThreadId>,
    /// A thread to pretend does not exist: [`ThreadingRepository::reconsider`] asks whether
    /// anything besides the message's own thread explains it better, and a thread born from it
    /// always matches its own subject.
    exclude: Option<ThreadId>,
}

impl LoadedIndex {
    async fn load(
        connection: &Connection,
        account_id: AccountId,
        cue: &ThreadCue,
        exclude: Option<ThreadId>,
    ) -> Result<Self> {
        // One statement for the whole chain: one per ancestor made `commit_batch` pay the
        // conversation's depth per message inside the write gate (3 statements with no ancestors
        // against 43 with forty).
        // Binary equality against folded keys (see `thread_of`). The gate asserts the bound
        // columns: a SEARCH binding one column of a two-column key is a scan wearing a seek's
        // clothes.
        let links: Vec<&RfcMessageId> = cue.links().collect();
        let mut by_id = std::collections::HashMap::new();
        if !links.is_empty() {
            let mut parameters = vec![turso::Value::from(account_id.get())];
            parameters.extend(links.iter().map(|link| turso::Value::from(link.folded())));
            // `UNION ALL` of point lookups, not `IN` (#1587): this planner binds an equality per
            // compound arm, but gives an `IN` on the index's second column a one-column prefix and
            // walks every link the account has, per message.
            let arms: Vec<String> = (0..links.len())
                .map(|n| {
                    format!(
                        "SELECT rfc_message_id, thread_id FROM thread_links \
                          WHERE account_id = ?1 AND rfc_message_id = ?{}",
                        n + 2
                    )
                })
                .collect();
            let found = sql::all(connection, &arms.join(" UNION ALL "), parameters, |row| {
                Ok((row.col::<String>(0)?, ThreadId::new(row.col(1)?)))
            })
            .await?;
            // Keyed by what the cue spelled, since `assign` looks up by that; the stored key is
            // folded, so the match stays case-insensitive.
            for (stored, thread) in found {
                if let Some(link) = links
                    .iter()
                    .find(|link| link.as_str().eq_ignore_ascii_case(&stored))
                {
                    by_id.insert(link.as_str().to_owned(), thread);
                }
            }
        }

        // A split thread claims the id of a message the person split off. Only asked when the links
        // name several threads, where `assign` chooses between joining and merging.
        let mut found: Vec<ThreadId> = by_id.values().copied().collect();
        found.sort_unstable();
        found.dedup();
        let splits = if found.len() > 1 {
            let mut parameters = vec![turso::Value::from(account_id.get())];
            parameters.extend(found.iter().map(|thread| turso::Value::from(thread.get())));
            // Driven from the splits (a handful of rows) into the links by their unique key, never
            // from the threads' links.
            let placeholders = (0..found.len())
                .map(|n| format!("?{}", n + 2))
                .collect::<Vec<_>>()
                .join(", ");
            sql::all(
                connection,
                &format!(
                    "SELECT l.thread_id FROM thread_splits s
                       CROSS JOIN thread_links l
                         ON l.account_id = s.account_id AND l.rfc_message_id = s.rfc_message_id
                      WHERE s.account_id = ?1 AND l.thread_id IN ({placeholders})"
                ),
                parameters,
                |row| Ok(ThreadId::new(row.col(0)?)),
            )
            .await?
            .into_iter()
            .collect()
        } else {
            Default::default()
        };

        // Only when nothing linked and the subject is usable, as `assign` does.
        // Bounded to threads whose latest message is in the window, and to the newest (spec 013,
        // research R2): the model joins one thread, never merging here.
        // No `id DESC` tie-break: it makes the planner prefer `idx_threads_account_last_at`, which
        // binds one column fewer and walks every thread in the window. The thread being
        // reconsidered is excluded here so it cannot crowd out the second-best.
        let by_subject = if cue.subject_is_usable() {
            let (from, until) = cue.subject_window();
            sql::first(
                connection,
                "SELECT id FROM threads
                  WHERE account_id = ?1 AND subject = ?2 AND last_at >= ?3 AND first_at <= ?4 AND id IS NOT ?5
                  ORDER BY last_at DESC LIMIT 1",
                bind![
                    account_id.get(),
                    cue.subject.as_str(),
                    super::to_millis(from),
                    super::to_millis(until),
                    exclude.map(ThreadId::get)
                ],
                |row| Ok(ThreadId::new(row.col(0)?)),
            )
            .await?
        } else {
            None
        };

        Ok(Self {
            by_id,
            splits,
            by_subject,
            exclude,
        })
    }

    /// Then the message must not take over the ids its References name in the other thread.
    fn joined_across_a_split(&self) -> bool {
        !self.splits.is_empty()
    }

    fn allowed(&self, thread: ThreadId) -> bool {
        self.exclude != Some(thread)
    }
}

impl ThreadIndex for LoadedIndex {
    fn thread_of(&self, id: &RfcMessageId) -> Option<ThreadId> {
        self.by_id
            .get(id.as_str())
            .copied()
            .filter(|found| self.allowed(*found))
    }

    fn threads_with_subject(
        &self,
        _subject: &str,
        _from: DateTime<Utc>,
        _until: DateTime<Utc>,
    ) -> Option<ThreadId> {
        self.by_subject.filter(|found| self.allowed(*found))
    }

    fn is_split(&self, thread: ThreadId) -> bool {
        self.splits.contains(&thread)
    }
}
