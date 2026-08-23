//! [`ImapBackend`]: the `MailBackend` the rest of Postio actually talks to.
//!
//! Everything under `imap` is a free function over a [`ConnectionPool`],
//! because that is the shape the protocol wants — a command, a connection, a
//! priority. Everything above `MailBackend` wants an object with methods and
//! no idea what IMAP is. This is the join, and it is deliberately thin: if a
//! method here does anything more interesting than choose a lane and
//! delegate, the logic is in the wrong place.
//!
//! It lives under `imap` rather than beside the trait because
//! `crates/postio-imap/src/backend/` may not name `io_imap` — a test reads
//! those sources and fails if one does. The seam stays protocol-free; its
//! implementation lives below it.
//!
//! # Priority is not in the trait, so it is in the value
//!
//! The pool serves interactive work before background work, which is what
//! stops a ten-thousand-message backfill from making a keystroke wait. The
//! trait has no way to say which kind a call is, so a backend *is* one:
//! [`ImapBackend::background`] hands back a second view onto the same pool
//! and the same connections, whose commands queue behind interactive ones.
//! The sync engine holds that one; anything a user is waiting on holds the
//! other.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use postio_model::{ModSeq, Uid};

use crate::backend::{
    AppendMessage, BackendResult, BodyPart, BodySink, Capabilities, FetchedBody, FetchedMessage,
    FlagChange, FlagUpdate, MailBackend, MailboxEvent, MailboxFilter, MailboxStatus,
    MailboxSummary, SelectMode, UidMapping, UidSet,
};
use crate::cancel::CancelToken;
use crate::secret::{AccountKey, SecretStore};

use super::{
    ConnectionPool, ConnectionSettings, ImapConnector, PoolConfig, Priority, append, copy_messages,
    expunge, fetch_headers, fetch_part, idle, list_mailboxes, move_messages, select, status,
    store_flags,
};

/// An IMAP server, behind the trait the rest of Postio speaks.
#[derive(Debug)]
pub struct ImapBackend {
    pool: Arc<ConnectionPool>,
    priority: Priority,
}

impl ImapBackend {
    /// Builds a backend and the pool underneath it. No connection is opened
    /// until one is asked for.
    pub fn new(
        settings: ConnectionSettings,
        key: AccountKey,
        store: Arc<dyn SecretStore>,
        connector: Arc<dyn ImapConnector>,
        config: PoolConfig,
    ) -> Self {
        Self::over(Arc::new(ConnectionPool::new(
            settings, key, store, connector, config,
        )))
    }

    /// Builds a backend over a pool somebody else owns.
    pub fn over(pool: Arc<ConnectionPool>) -> Self {
        Self {
            pool,
            priority: Priority::Interactive,
        }
    }

    /// The same server, on the same connections, for work nobody is waiting
    /// on.
    ///
    /// Hand this one to the sync engine and keep the interactive one for
    /// whatever the user is looking at.
    pub fn background(&self) -> Self {
        Self {
            pool: Arc::clone(&self.pool),
            priority: Priority::Background,
        }
    }

    /// The pool underneath, for stats and for shutdown.
    pub fn pool(&self) -> &Arc<ConnectionPool> {
        &self.pool
    }
}

#[async_trait]
impl MailBackend for ImapBackend {
    fn describe(&self) -> &'static str {
        "imap"
    }

    async fn connect(&self) -> BackendResult<Capabilities> {
        // Acquiring opens a connection when none is parked, and the session
        // refuses an empty post-auth capability list on the way (ADR 0001,
        // Q3). The slot is given straight back: this is a probe, not a claim.
        let session = self.pool.acquire(self.priority).await?;
        let capabilities = session.capabilities().clone();
        drop(session);
        Ok(capabilities)
    }

    async fn disconnect(&self) -> BackendResult<()> {
        // Every idle connection goes; anything in flight is left to finish.
        // The backend stays usable, because "disconnected" for a pool means
        // "holding nothing open", not "finished" — that is `close`.
        self.pool.drop_idle();
        Ok(())
    }

    async fn capabilities(&self) -> BackendResult<Capabilities> {
        // Answered from what the last connection reported, never by asking
        // the server: callers use this as a cheap liveness check, and a
        // round trip per check would make watching the link cost more than
        // the work it protects.
        self.pool
            .capabilities()
            .ok_or_else(|| crate::backend::BackendError::NotConnected {
                context: format!("no connection has been opened to {}", self.describe()),
            })
    }

    async fn list_mailboxes(&self, filter: &MailboxFilter) -> BackendResult<Vec<MailboxSummary>> {
        list_mailboxes(&self.pool, filter, self.priority).await
    }

    async fn select(&self, path: &str, mode: SelectMode) -> BackendResult<MailboxStatus> {
        select(&self.pool, path, mode, self.priority).await
    }

    async fn status(&self, path: &str) -> BackendResult<MailboxStatus> {
        status(&self.pool, path, self.priority).await
    }

    async fn fetch_headers(
        &self,
        mailbox: &str,
        uids: &UidSet,
        changed_since: Option<ModSeq>,
        cancel: &CancelToken,
    ) -> BackendResult<Vec<FetchedMessage>> {
        fetch_headers(
            &self.pool,
            mailbox,
            uids,
            changed_since,
            self.priority,
            cancel,
        )
        .await
    }

    async fn fetch_part(
        &self,
        mailbox: &str,
        uid: Uid,
        part: &BodyPart,
        sink: &mut dyn BodySink,
        cancel: &CancelToken,
    ) -> BackendResult<FetchedBody> {
        fetch_part(&self.pool, mailbox, uid, part, sink, self.priority, cancel).await
    }

    async fn store_flags(
        &self,
        mailbox: &str,
        uids: &UidSet,
        change: &FlagChange,
    ) -> BackendResult<Vec<FlagUpdate>> {
        store_flags(&self.pool, mailbox, uids, change, self.priority).await
    }

    async fn move_messages(
        &self,
        from: &str,
        uids: &UidSet,
        to: &str,
    ) -> BackendResult<Vec<UidMapping>> {
        move_messages(&self.pool, from, uids, to, self.priority).await
    }

    async fn copy_messages(
        &self,
        from: &str,
        uids: &UidSet,
        to: &str,
    ) -> BackendResult<Vec<UidMapping>> {
        copy_messages(&self.pool, from, uids, to, self.priority).await
    }

    async fn expunge(&self, mailbox: &str, uids: Option<&UidSet>) -> BackendResult<Vec<Uid>> {
        expunge(&self.pool, mailbox, uids, self.priority).await
    }

    async fn append(
        &self,
        mailbox: &str,
        message: &AppendMessage,
    ) -> BackendResult<Option<UidMapping>> {
        append(&self.pool, mailbox, message, self.priority).await
    }

    async fn idle(
        &self,
        mailbox: &str,
        timeout: Duration,
        cancel: &CancelToken,
    ) -> BackendResult<Vec<MailboxEvent>> {
        // Always the watch lane, whatever this backend's priority: a wait of
        // minutes must never sit in a slot that commands need.
        idle(&self.pool, mailbox, timeout, cancel).await
    }
}
