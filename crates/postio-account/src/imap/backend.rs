//! The join between `MailBackend` and the free functions over a `ConnectionPool`; thin by design.
//! Lives under `imap` because `backend/` may not name `io_imap`.
//! `ImapBackend::background` is a second view onto the same pool whose commands queue behind
//! interactive ones.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use postio_model::{ChangeFeed, FlagSet, ModSeq, RemoteId, Uid};

use crate::backend::{
    AppendMessage, BackendResult, BodyPart, BodySink, Capabilities, FetchedBody, FetchedMessage,
    FlagChange, FlagUpdate, MailBackend, MailboxEvent, MailboxFilter, MailboxStatus,
    MailboxSummary, SelectMode, UidMapping, UidSet,
};
use crate::cancel::CancelToken;
use crate::secret::{AccountKey, SecretStore};

use super::{
    ConnectionPool, ConnectionSettings, ImapConnector, PoolConfig, Priority, append, copy_messages,
    create_mailbox, expunge, fetch_headers, fetch_headers_for_filing, fetch_part, fetch_sections,
    idle, list_mailboxes, move_messages, select, status, store_flags,
};

#[derive(Debug)]
pub struct ImapBackend {
    pool: Arc<ConnectionPool>,
    priority: Priority,
}

impl ImapBackend {
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

    pub fn over(pool: Arc<ConnectionPool>) -> Self {
        Self {
            pool,
            priority: Priority::Interactive,
        }
    }

    /// A second view onto the same pool whose commands queue behind interactive ones; the sync
    /// engine holds this one.
    pub fn background(&self) -> Self {
        Self {
            pool: Arc::clone(&self.pool),
            priority: Priority::Background,
        }
    }

    pub fn pool(&self) -> &Arc<ConnectionPool> {
        &self.pool
    }
}

#[async_trait]
impl MailBackend for ImapBackend {
    fn describe(&self) -> &'static str {
        "imap"
    }

    fn for_background(&self) -> Option<Arc<dyn MailBackend>> {
        Some(Arc::new(self.background()))
    }

    async fn connect(&self) -> BackendResult<Capabilities> {
        // Acquiring opens a connection if none is parked, and the session refuses an empty
        // post-auth capability list (ADR 0001 Q3). The slot goes straight back.
        let session = self.pool.acquire(self.priority).await?;
        let capabilities = session.capabilities().clone();
        drop(session);
        Ok(capabilities)
    }

    async fn disconnect(&self) -> BackendResult<()> {
        // "Disconnected" for a pool means holding nothing open, not finished; that is `close`.
        self.pool.drop_idle();
        Ok(())
    }

    async fn capabilities(&self) -> BackendResult<Capabilities> {
        // Answered from the last connection's report, not the server: callers use this as a cheap
        // liveness check.
        self.pool
            .capabilities()
            .ok_or_else(|| crate::backend::BackendError::NotConnected {
                context: format!("no connection has been opened to {}", self.describe()),
            })
    }

    async fn list_mailboxes(&self, filter: &MailboxFilter) -> BackendResult<Vec<MailboxSummary>> {
        list_mailboxes(&self.pool, filter, self.priority).await
    }

    async fn create_mailbox(&self, path: &str) -> BackendResult<()> {
        create_mailbox(&self.pool, path, self.priority).await
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

    async fn fetch_headers_for_filing(
        &self,
        mailbox: &str,
        uids: &UidSet,
        changed_since: Option<ModSeq>,
        cancel: &CancelToken,
    ) -> BackendResult<Vec<FetchedMessage>> {
        fetch_headers_for_filing(
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
        id: &RemoteId,
        part: &BodyPart,
        sink: &mut dyn BodySink,
        cancel: &CancelToken,
    ) -> BackendResult<FetchedBody> {
        fetch_part(&self.pool, mailbox, id, part, sink, self.priority, cancel).await
    }

    async fn fetch_sections(
        &self,
        mailbox: &str,
        ids: &[postio_model::RemoteId],
        part: &BodyPart,
        cancel: &CancelToken,
    ) -> BackendResult<Vec<(postio_model::RemoteId, Vec<u8>)>> {
        fetch_sections(&self.pool, mailbox, ids, part, self.priority, cancel).await
    }

    async fn store_flags(
        &self,
        mailbox: &str,
        ids: &[RemoteId],
        change: &FlagChange,
    ) -> BackendResult<Vec<FlagUpdate>> {
        store_flags(&self.pool, mailbox, ids, change, self.priority).await
    }

    async fn move_messages(
        &self,
        from: &str,
        ids: &[RemoteId],
        to: &str,
    ) -> BackendResult<Vec<UidMapping>> {
        move_messages(&self.pool, from, ids, to, self.priority).await
    }

    async fn copy_messages(
        &self,
        from: &str,
        ids: &[RemoteId],
        to: &str,
    ) -> BackendResult<Vec<UidMapping>> {
        copy_messages(&self.pool, from, ids, to, self.priority).await
    }

    async fn expunge(
        &self,
        mailbox: &str,
        ids: Option<&[RemoteId]>,
    ) -> BackendResult<Vec<RemoteId>> {
        expunge(&self.pool, mailbox, ids, self.priority).await
    }

    async fn append(
        &self,
        mailbox: &str,
        message: &AppendMessage,
    ) -> BackendResult<Option<UidMapping>> {
        append(&self.pool, mailbox, message, self.priority).await
    }

    async fn find_by_message_id(
        &self,
        mailbox: &str,
        message_id: &str,
    ) -> BackendResult<Option<RemoteId>> {
        super::mutate::find_by_message_id(&self.pool, mailbox, message_id, self.priority).await
    }

    async fn existing_uids(
        &self,
        mailbox: &str,
        cancel: &CancelToken,
    ) -> BackendResult<Option<Vec<Uid>>> {
        super::fetch::existing_uids(&self.pool, mailbox, self.priority, cancel).await
    }

    async fn fetch_flags(
        &self,
        mailbox: &str,
        cancel: &CancelToken,
    ) -> BackendResult<Option<Vec<(Uid, FlagSet)>>> {
        super::fetch::fetch_flags(&self.pool, mailbox, self.priority, cancel).await
    }

    fn change_feed(&self) -> ChangeFeed {
        crate::discovery::preset_for_imap_host(&self.pool.settings().host)
            .map_or(ChangeFeed::Reliable, |preset| preset.change_feed())
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
