//! Filing a message into a thread.
//!
//! The rule — which thread a message belongs to, and when two threads have to
//! merge — lives in [`postio_model::threading`] and knows nothing about SQL.
//! This is the other half: the [`ThreadIndex`] it asks, and the writes that
//! follow from its answer.
//!
//! # What makes it O(thread)
//!
//! The lookup is against `thread_links`, one row per `Message-ID` a thread has
//! claimed, primary-keyed on `(account_id, rfc_message_id)`. So placing a
//! message costs one indexed query for its whole reference chain, plus the
//! writes for the thread it lands in. Nothing scans the mailbox, and adding the
//! ten-thousandth message costs what adding the tenth did — there is a test
//! that asserts exactly that by counting statements at two mailbox sizes.
//!
//! # Why a thread claims ids it has never seen
//!
//! An initial sync walks newest-first, so a reply routinely arrives before the
//! message it answers. The reply's thread claims the parent's id immediately,
//! and when the parent turns up it finds the thread that was waiting for it.
//! The alternative — searching `messages.reference_ids`, which is a
//! space-separated list — is the mailbox scan this table exists to avoid.
//!
//! # What a claim cannot fix: no reference at all
//!
//! [`ThreadingRepository::thread`] only ever answers "where does this message
//! go" from what the index knows *right now*. A message with no `In-Reply-To`
//! and no `References` at all can only be placed by
//! [`postio_model::subject::is_reply`]'s subject fallback, and if it arrives
//! before the messages that share its subject, there is nothing to fall back
//! to yet — it starts its own thread, alone, and nothing about inserting a
//! *later* message ever revisits that decision.
//! [`ThreadingRepository::reconsider`] and
//! [`ThreadingRepository::rethread_orphans`] are the repair: re-ask the same
//! question later, for exactly the messages that could not have gotten a
//! better answer at insertion time. See postio-tn9.2.

use postio_model::{
    AccountId, Assignment, MailboxId, Message, RfcMessageId, Thread, ThreadCue, ThreadId,
    ThreadIndex, assign, claimed_ids,
};

use super::{MessageRepository, ThreadRepository};

use crate::error::Result;
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;

/// Files messages into threads.
///
/// A thin borrow of a connection, like every repository. Build one inside the
/// transaction that writes the message and the thread lands with it.
#[derive(Debug)]
pub struct ThreadingRepository<'a> {
    connection: &'a Connection,
    account_id: AccountId,
}

/// What filing a message did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Threaded {
    /// The thread the message is now in.
    pub thread_id: ThreadId,
    /// Whether the thread was created for it.
    pub created: bool,
    /// Threads folded into `thread_id` because this message linked them.
    ///
    /// Non-empty means a conversation the user saw as two is now one, which is
    /// worth an event: the list has to redraw more than one row.
    pub merged: Vec<ThreadId>,
}

impl<'a> ThreadingRepository<'a> {
    /// Borrows a connection, scoped to one account.
    ///
    /// Threading never crosses accounts: two people can hold the same
    /// `Message-ID` and they are not in a conversation.
    pub fn new(connection: &'a Connection, account_id: AccountId) -> Self {
        Self {
            connection,
            account_id,
        }
    }

    /// Files `message` into a thread, creating or merging as required.
    ///
    /// Writes `messages.thread_id`, so the caller does not have to. Idempotent
    /// for a message already filed: re-filing it re-derives the same answer
    /// and claims the same ids.
    pub async fn thread(&self, message: &Message) -> Result<Threaded> {
        let cue = ThreadCue::of(message);
        sql::in_scope(self.connection, |scope| async move {
            let index = LoadedIndex::load(&scope, self.account_id, &cue, None).await?;
            let assignment = assign(&cue, &index);

            let result = self.apply(&scope, message, &cue, assignment).await?;
            Ok(result)
        })
        .await
    }

    /// The write side of [`ThreadingRepository::thread`] and
    /// [`ThreadingRepository::reconsider`] alike: given an already-decided
    /// [`Assignment`], create or merge threads as it requires, then file
    /// `message` into the result and claim its ids.
    async fn apply(
        &self,
        connection: &Connection,
        message: &Message,
        cue: &ThreadCue,
        assignment: Assignment,
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
                    // Relink *before* merging. `merge` deletes the absorbed
                    // thread, and `thread_links.thread_id` cascades on that
                    // delete — so the other order silently drops every id the
                    // absorbed thread had claimed, and the conversation comes
                    // apart again on the next reply.
                    self.relink(connection, *other, into).await?;
                    threads.merge(into, *other).await?;
                }
                (into, false, absorb)
            }
        };

        threads.add_message(thread_id, message.id).await?;
        for id in claimed_ids(cue) {
            self.claim(connection, id, thread_id).await?;
        }

        Ok(Threaded {
            thread_id,
            created,
            merged,
        })
    }

    /// Records that `thread_id` claims `id`.
    ///
    /// Last claim wins. That only happens when two threads have already been
    /// merged, or when a message is re-filed, and in both cases the newer
    /// answer is the right one.
    async fn claim(
        &self,
        connection: &Connection,
        id: &RfcMessageId,
        thread_id: ThreadId,
    ) -> Result<()> {
        // Cached: a sync pass runs this once per id every message claims, so
        // it is one of the handful of statements a first sync compiles
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

    /// Moves every id `absorbed` claimed onto `into`.
    ///
    /// `OR REPLACE` because the two threads may well claim the same id — that
    /// is often *why* they merged — and the surviving row is the one that
    /// points at the thread that survived.
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

    /// Every id a thread claims, for diagnostics and tests.
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

    /// The thread claiming `id`, if any.
    pub async fn thread_of(&self, id: &RfcMessageId) -> Result<Option<ThreadId>> {
        sql::first(
            self.connection,
            // Binary equality against the folded key, deliberately: the
            // comparison used to be `COLLATE NOCASE`, and Turso's planner
            // will not bind an equality through a collated index column —
            // it seeks to `account_id` and walks every link the account
            // has, once per message, which is #1587's linear curve. The
            // case-insensitivity lives in `RfcMessageId::folded` now.
            "SELECT thread_id FROM thread_links
              WHERE account_id = ?1 AND rfc_message_id = ?2",
            bind![self.account_id.get(), id.folded()],
            |row| Ok(ThreadId::new(row.col(0)?)),
        )
        .await
    }

    /// Re-derives where `message` belongs against the index as it stands now,
    /// and merges it into a better thread if one has since appeared.
    ///
    /// Only acts when `message` is the sole member of its current thread —
    /// the signature of a subject-only orphan that started alone. A thread
    /// with other members got them by the same rule this message did, and
    /// moving them on a match *this* message's own cue cannot vouch for would
    /// risk merging two threads that only coincidentally share a subject,
    /// which is exactly what [`postio_model::threading`]'s `is_reply` guard
    /// exists to prevent. A no-op, cheaply, whenever that guard applies, or
    /// when re-deriving lands back on the thread `message` is already in.
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
            // `current` was created *for* this exact message, so its own thread
            // row necessarily matches `cue`'s subject and would answer every
            // question `assign` asks right back with itself — a self-match, not
            // a better one. Excluding it is what makes "nothing better than what
            // it already has" ([`Assignment::New`]) distinguishable from "still
            // itself".
            let index = LoadedIndex::load(&scope, self.account_id, &cue, Some(current)).await?;
            let assignment = assign(&cue, &index);
            if matches!(assignment, Assignment::New) {
                return Ok(None);
            }

            let mut result = self.apply(&scope, message, &cue, assignment).await?;
            // This message was `current`'s only member, so `apply` (via
            // `add_message`) just emptied it. Fold the now-empty thread away
            // properly — claims included — rather than leaving a zero-message row
            // behind for the thread list to have to filter out forever after.
            self.relink(&scope, current, result.thread_id).await?;
            ThreadRepository::new(&scope)
                .merge(result.thread_id, current)
                .await?;
            result.merged.push(current);

            Ok(Some(result))
        })
        .await
    }

    /// Runs [`reconsider`](Self::reconsider) over every message in
    /// `mailbox_id` that carries no usable reference at all — the subject-only
    /// orphans arrival order can strand — and returns how many moved.
    ///
    /// Meant to run when a mailbox's sync goes idle, not on every insert:
    /// unlike [`ThreadingRepository::thread`], this scans, and the design this
    /// crate follows is that adding a message never costs more than its own
    /// reference chain. See postio-tn9.2.
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

/// The model's [`ThreadIndex`], loaded.
///
/// # Why the lookups happen before the decision, not during it
///
/// [`ThreadIndex`] is synchronous, and it is in `postio-model`, which knows
/// nothing about databases and must go on knowing nothing: the threading rule
/// is pure so that it can be tested exhaustively against two maps, which is
/// the model's own description of what a storage layer is for this purpose.
///
/// The engine is async, so an implementation that queried inside `thread_of`
/// would have to make the model's trait async and the model's purity with it.
/// It does not need to. Everything `assign` can ask about is derivable from
/// the cue before it is called -- the links it will look up, and the one
/// subject it may fall back to -- so this loads exactly those answers first
/// and then answers from memory.
///
/// It is also fewer round trips than the version that queried per reference:
/// one statement for the links, one for the subject, against a lookup per
/// reference on every message a sync pass files (#728).
#[derive(Debug, Default)]
struct LoadedIndex {
    by_id: std::collections::HashMap<String, ThreadId>,
    by_subject: Vec<ThreadId>,
    /// A thread to pretend does not exist. [`ThreadingRepository::reconsider`]
    /// asks "does anything *besides* the thread this message is already in
    /// explain it better", and a thread born from this exact message always
    /// matches its own subject -- so without this, `assign` would find its own
    /// thread first and call the question answered.
    exclude: Option<ThreadId>,
}

impl LoadedIndex {
    /// Read the answers `assign` could ask for, for this cue.
    async fn load(
        connection: &Connection,
        account_id: AccountId,
        cue: &ThreadCue,
        exclude: Option<ThreadId>,
    ) -> Result<Self> {
        // One statement for the whole chain, not one per ancestor.
        //
        // This asked the database once per link, in series. `cue.links()` is
        // the `References`/`In-Reply-To` chain, so its length is the depth of
        // the conversation — and `commit_batch` threads one message at a time,
        // paying that for every message in every batch, inside the
        // transaction, holding the write gate. Counted: 3 statements to thread
        // a message with no ancestors against 43 with forty, which on a real
        // Sent folder at two hundred messages a batch is thousands of
        // sequential round trips, and the 31 s `write_ms` reported against a
        // `fetch_ms` of 610.
        //
        // Binary equality against folded keys — see `thread_of` for the
        // Turso planner rule that retired `COLLATE NOCASE` here. The old
        // gate asserted the planner *seeks*, and it did: a SEARCH that binds
        // one column of a two-column key is a scan wearing a seek's clothes,
        // which is why the gate asserts the bound columns now.
        let links: Vec<&RfcMessageId> = cue.links().collect();
        let mut by_id = std::collections::HashMap::new();
        if !links.is_empty() {
            let mut parameters = vec![turso::Value::from(account_id.get())];
            parameters.extend(links.iter().map(|link| turso::Value::from(link.folded())));
            // `UNION ALL` of point lookups, not `IN`, and the difference is
            // the whole of #1587 in one statement: this planner binds an
            // equality per compound arm — `(account_id=? AND
            // rfc_message_id=?)` each — but hands an `IN` on the index's
            // second column a one-column prefix and walks every link the
            // account has, per message, for the whole of a sync. Still one
            // statement per chain, which is what the statement-count gate
            // holds this to.
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
            // Keyed by what the *cue* spelled: `assign` looks these up by
            // the link it holds, and the stored key is folded — which is why
            // the match here stays case-insensitive.
            for (stored, thread) in found {
                if let Some(link) = links
                    .iter()
                    .find(|link| link.as_str().eq_ignore_ascii_case(&stored))
                {
                    by_id.insert(link.as_str().to_owned(), thread);
                }
            }
        }

        // Only consulted when nothing linked, and only for a usable subject —
        // the same condition `assign` applies, so a subject it would never ask
        // about is never queried for.
        let by_subject = if cue.subject_is_usable() {
            sql::all(
                connection,
                "SELECT id FROM threads WHERE account_id = ?1 AND subject = ?2 ORDER BY id",
                bind![account_id.get(), cue.subject.as_str()],
                |row| Ok(ThreadId::new(row.col(0)?)),
            )
            .await?
        } else {
            Vec::new()
        };

        Ok(Self {
            by_id,
            by_subject,
            exclude,
        })
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

    fn threads_with_subject(&self, _subject: &str) -> Vec<ThreadId> {
        // The subject was fixed when this was loaded -- it is the cue's own,
        // and `assign` asks about no other.
        self.by_subject
            .iter()
            .copied()
            .filter(|found| self.allowed(*found))
            .collect()
    }
}
