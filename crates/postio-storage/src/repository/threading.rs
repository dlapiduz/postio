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

use postio_model::{
    AccountId, Assignment, Message, RfcMessageId, Thread, ThreadCue, ThreadId, ThreadIndex, assign,
    claimed_ids,
};
use rusqlite::{Connection, params};

use super::ThreadRepository;
use crate::error::Result;

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
    pub fn thread(&self, message: &Message) -> Result<Threaded> {
        let cue = ThreadCue::of(message);
        let scope = super::Scope::open(self.connection)?;
        let index = SqlIndex {
            connection: &scope,
            account_id: self.account_id,
        };

        let assignment = assign(&cue, &index);
        let threads = ThreadRepository::new(&scope);

        let (thread_id, created, merged) = match assignment {
            Assignment::New => {
                let mut thread = Thread::new(self.account_id);
                thread.subject = (!cue.subject.is_empty()).then(|| cue.subject.clone());
                (threads.create(&mut thread)?, true, Vec::new())
            }
            Assignment::Join(id) => (id, false, Vec::new()),
            Assignment::Merge { into, absorb } => {
                for other in &absorb {
                    // Relink *before* merging. `merge` deletes the absorbed
                    // thread, and `thread_links.thread_id` cascades on that
                    // delete — so the other order silently drops every id the
                    // absorbed thread had claimed, and the conversation comes
                    // apart again on the next reply.
                    self.relink(&scope, *other, into)?;
                    threads.merge(into, *other)?;
                }
                (into, false, absorb)
            }
        };

        threads.add_message(thread_id, message.id)?;
        for id in claimed_ids(&cue) {
            self.claim(&scope, id, thread_id)?;
        }

        scope.commit()?;
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
    fn claim(&self, connection: &Connection, id: &RfcMessageId, thread_id: ThreadId) -> Result<()> {
        connection.execute(
            "INSERT INTO thread_links (account_id, rfc_message_id, thread_id)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (account_id, rfc_message_id) DO UPDATE
                SET thread_id = excluded.thread_id",
            params![self.account_id.get(), id.as_str(), thread_id.get()],
        )?;
        Ok(())
    }

    /// Moves every id `absorbed` claimed onto `into`.
    ///
    /// `OR REPLACE` because the two threads may well claim the same id — that
    /// is often *why* they merged — and the surviving row is the one that
    /// points at the thread that survived.
    fn relink(&self, connection: &Connection, absorbed: ThreadId, into: ThreadId) -> Result<()> {
        connection.execute(
            "UPDATE OR REPLACE thread_links SET thread_id = ?2 WHERE thread_id = ?1",
            params![absorbed.get(), into.get()],
        )?;
        Ok(())
    }

    /// Every id a thread claims, for diagnostics and tests.
    pub fn claims(&self, thread_id: ThreadId) -> Result<Vec<RfcMessageId>> {
        let mut statement = self.connection.prepare(
            "SELECT rfc_message_id FROM thread_links
              WHERE account_id = ?1 AND thread_id = ?2 ORDER BY rfc_message_id",
        )?;
        let rows = statement.query_map(params![self.account_id.get(), thread_id.get()], |row| {
            row.get::<_, String>(0)
        })?;
        Ok(rows
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .map(RfcMessageId::new)
            .collect())
    }

    /// The thread claiming `id`, if any.
    pub fn thread_of(&self, id: &RfcMessageId) -> Result<Option<ThreadId>> {
        let index = SqlIndex {
            connection: self.connection,
            account_id: self.account_id,
        };
        Ok(index.thread_of(id))
    }
}

/// The model's [`ThreadIndex`], over SQLite.
struct SqlIndex<'a> {
    connection: &'a Connection,
    account_id: AccountId,
}

impl ThreadIndex for SqlIndex<'_> {
    fn thread_of(&self, id: &RfcMessageId) -> Option<ThreadId> {
        // A lookup that fails is not the same as an id nobody claims, but the
        // model's trait has nowhere to put an error and the consequence of
        // treating one as the other is a thread that does not merge — which the
        // next message on the chain will fix. A broken database announces
        // itself on the write that follows.
        self.connection
            .query_row(
                "SELECT thread_id FROM thread_links
                  WHERE account_id = ?1 AND rfc_message_id = ?2 COLLATE NOCASE",
                params![self.account_id.get(), id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .ok()
            .map(ThreadId::new)
    }

    fn threads_with_subject(&self, subject: &str) -> Vec<ThreadId> {
        let Ok(mut statement) = self
            .connection
            .prepare("SELECT id FROM threads WHERE account_id = ?1 AND subject = ?2 ORDER BY id")
        else {
            return Vec::new();
        };
        let Ok(rows) = statement.query_map(params![self.account_id.get(), subject], |row| {
            row.get::<_, i64>(0)
        }) else {
            return Vec::new();
        };
        rows.filter_map(|row| row.ok().map(ThreadId::new)).collect()
    }
}
