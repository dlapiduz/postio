//! The [`MailBackend`] implementation.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use io_gmail::v1::client::GmailClientStdError;
use io_gmail::v1::rest::labels::GmailLabel;
use io_gmail::v1::rest::messages::insert::GmailMessageInsert;
use io_gmail::v1::rest::messages::list::GmailMessagesListParams;
use io_gmail::v1::rest::messages::{GmailMessage, GmailMessageFormat, decode_raw, encode_raw};
use postio_account::auth::TokenSource;
use postio_account::backend::{
    AppendMessage, BackendError, BackendResult, BodyPart, BodySink, Capabilities, FetchedBody,
    FetchedMessage, FlagChange, FlagUpdate, MailBackend, MailboxEvent, MailboxFilter,
    MailboxStatus, MailboxSummary, SelectMode, UidMapping, UidSet,
};
use postio_account::cancel::CancelToken;
use postio_account::secret::AccountKey;
use postio_model::{Flag, FlagSet, Generation, ModSeq, RemoteId, Uid, UidValidity};

use crate::connection::GmailConnection;
use crate::convert;

/// A [`MailBackend`] speaking the Gmail REST API v1.
pub struct GmailBackend {
    connection: GmailConnection,
}

impl fmt::Debug for GmailBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GmailBackend").finish_non_exhaustive()
    }
}

fn gmail_error(context: &str, error: GmailClientStdError) -> BackendError {
    let reason = format!("{context}: {error}");
    if reason.contains("401") || reason.contains("403") {
        BackendError::Auth {
            account: String::new(),
            reason,
        }
    } else {
        BackendError::Protocol { reason }
    }
}

impl GmailBackend {
    /// A backend authenticating with a fixed bearer. Tests, mostly.
    pub fn new(token: &str) -> Self {
        Self {
            connection: GmailConnection::new(token),
        }
    }

    /// A backend whose bearer comes from the account's token source.
    pub fn with_token_source(key: AccountKey, tokens: Arc<dyn TokenSource>) -> Self {
        Self {
            connection: GmailConnection::with_token_source(key, tokens),
        }
    }

    /// Point every request at a scripted server on loopback.
    pub fn with_loopback_endpoint(mut self, host: &str, port: u16) -> Self {
        self.connection = self.connection.with_loopback_endpoint(host, port);
        self
    }

    async fn labels(&self) -> BackendResult<Vec<GmailLabel>> {
        self.connection
            .run(&CancelToken::new(), |client| {
                client
                    .labels_list()
                    .map(|out| out.response.labels)
                    .map_err(|error| gmail_error("labels.list", error))
            })
            .await
    }

    /// The label id `path` names, or the archive's `None`.
    async fn resolve(&self, path: &str) -> BackendResult<Option<String>> {
        if path == convert::ARCHIVE_PATH {
            return Ok(None);
        }
        let labels = self.labels().await?;
        convert::label_id(path, &labels)
            .map(Some)
            .ok_or_else(|| BackendError::NoSuchMailbox {
                path: path.to_owned(),
            })
    }

    /// One page of ids for `path`, newest first.
    async fn list_ids(
        &self,
        path: &str,
        max: u32,
        page: Option<String>,
    ) -> BackendResult<(Vec<String>, Option<String>, Option<u64>)> {
        let label = self.resolve(path).await?;
        self.connection
            .run(&CancelToken::new(), move |client| {
                let labels: Vec<String> = label.into_iter().collect();
                let out = client
                    .messages_list(&GmailMessagesListParams {
                        q: labels.is_empty().then_some(convert::ARCHIVE_QUERY),
                        label_ids: &labels,
                        max_results: Some(max),
                        page_token: page.as_deref(),
                        include_spam_trash: false,
                    })
                    .map_err(|error| gmail_error("messages.list", error))?;
                Ok((
                    out.response
                        .messages
                        .into_iter()
                        .map(|message| message.id)
                        .collect(),
                    out.response.next_page_token,
                    out.response.result_size_estimate,
                ))
            })
            .await
    }

    async fn get(&self, id: String, format: GmailMessageFormat) -> BackendResult<GmailMessage> {
        self.connection
            .run(&CancelToken::new(), move |client| {
                client
                    .message_get(&id, format, &[])
                    .map(|out| out.response)
                    .map_err(|error| gmail_error("messages.get", error))
            })
            .await
    }

    async fn modify(
        &self,
        ids: Vec<String>,
        adds: Vec<String>,
        removes: Vec<String>,
    ) -> BackendResult<Vec<String>> {
        if adds.is_empty() && removes.is_empty() {
            return Ok(ids);
        }
        self.connection
            .run(&CancelToken::new(), move |client| {
                let mut touched = Vec::new();
                for id in ids {
                    client
                        .message_modify(&id, &adds, &removes)
                        .map_err(|error| gmail_error("messages.modify", error))?;
                    touched.push(id);
                }
                Ok(touched)
            })
            .await
    }
}

#[async_trait]
impl MailBackend for GmailBackend {
    fn describe(&self) -> &'static str {
        "gmail"
    }

    async fn connect(&self) -> BackendResult<Capabilities> {
        let address = self
            .connection
            .run(&CancelToken::new(), |client| {
                client
                    .profile_get()
                    .map(|out| out.response.email_address)
                    .map_err(|error| gmail_error("getProfile", error))
            })
            .await?;
        tracing::debug!(resolved = !address.is_empty(), "gmail profile resolved");
        Ok(Capabilities::from_names(["GMAIL-REST-V1"]))
    }

    async fn disconnect(&self) -> BackendResult<()> {
        Ok(())
    }

    async fn capabilities(&self) -> BackendResult<Capabilities> {
        Ok(Capabilities::from_names(["GMAIL-REST-V1"]))
    }

    async fn list_mailboxes(&self, _filter: &MailboxFilter) -> BackendResult<Vec<MailboxSummary>> {
        Ok(convert::mailboxes(&self.labels().await?))
    }

    async fn select(&self, path: &str, _mode: SelectMode) -> BackendResult<MailboxStatus> {
        self.status(path).await
    }

    async fn status(&self, path: &str) -> BackendResult<MailboxStatus> {
        let (exists, unseen) = match self.resolve(path).await? {
            Some(label_id) => {
                let label = self
                    .connection
                    .run(&CancelToken::new(), move |client| {
                        client
                            .label_get(&label_id)
                            .map(|out| out.response)
                            .map_err(|error| gmail_error("labels.get", error))
                    })
                    .await?;
                (
                    label.messages_total.unwrap_or_default() as u32,
                    label.messages_unread.map(|unread| unread as u32),
                )
            }
            // The archive is a search; the estimate is the only count the
            // API offers. Off-by-some costs a re-fetch next pass, never a
            // mislabel — identity matching absorbs it.
            None => {
                let (_, _, estimate) = self.list_ids(path, 1, None).await?;
                (estimate.unwrap_or_default() as u32, None)
            }
        };
        Ok(MailboxStatus {
            path: path.to_owned(),
            generation: Generation::new(convert::GENERATION),
            uid_next: Uid::new(exists + 1),
            exists,
            unseen,
            highest_mod_seq: None,
            permanent_flags: FlagSet::new(),
            can_create_keywords: false,
            read_only: false,
        })
    }

    async fn fetch_headers(
        &self,
        mailbox: &str,
        uids: &UidSet,
        changed_since: Option<ModSeq>,
        cancel: &CancelToken,
    ) -> BackendResult<Vec<FetchedMessage>> {
        if changed_since.is_some() {
            return Err(BackendError::Unsupported {
                capability: postio_account::backend::Capability::CondStore,
            });
        }
        let mut positions = uids.uids();
        let Some(first) = positions.next() else {
            return Ok(Vec::new());
        };
        let last = positions.last().unwrap_or(first);
        let wanted = uids.clone();

        // The list is newest first; positions count from the oldest. Walk
        // pages until the whole span 1..=last is in hand, then address the
        // requested positions from the end.
        let mut ids_newest_first = Vec::new();
        let mut page = None;
        loop {
            if cancel.is_cancelled() {
                return Err(BackendError::Cancelled);
            }
            let (ids, next, _) = self.list_ids(mailbox, 500, page).await?;
            ids_newest_first.extend(ids);
            match next {
                Some(next) => page = Some(next),
                None => break,
            }
        }
        let total = ids_newest_first.len() as u32;

        let mut fetched = Vec::new();
        for position in first.get()..=last.get() {
            if !wanted.contains(Uid::new(position)) || position == 0 || position > total {
                continue;
            }
            let id = ids_newest_first[(total - position) as usize].clone();
            let message = self.get(id, GmailMessageFormat::Metadata).await?;
            if let Some(message) = convert::fetched(&message, position) {
                fetched.push(message);
            }
        }
        Ok(fetched)
    }

    async fn fetch_part(
        &self,
        _mailbox: &str,
        id: &RemoteId,
        part: &BodyPart,
        sink: &mut dyn BodySink,
        _cancel: &CancelToken,
    ) -> BackendResult<FetchedBody> {
        if !matches!(part, BodyPart::Whole) {
            return Err(BackendError::Unsupported {
                capability: postio_account::backend::Capability::Binary,
            });
        }
        let message = self
            .get(id.as_str().to_owned(), GmailMessageFormat::Raw)
            .await?;
        let raw = message.raw.ok_or_else(|| BackendError::Protocol {
            reason: format!("message `{id:?}` came back without raw content"),
        })?;
        let bytes = decode_raw(&raw).map_err(|error| BackendError::Protocol {
            reason: format!("the raw content is not base64url: {error}"),
        })?;

        sink.chunk(&bytes).await?;
        let written = bytes.len() as u64;
        sink.finish().await?;
        Ok(FetchedBody {
            remote_id: id.clone(),
            part: part.clone(),
            bytes_written: written,
        })
    }

    async fn store_flags(
        &self,
        _mailbox: &str,
        ids: &[RemoteId],
        change: &FlagChange,
    ) -> BackendResult<Vec<FlagUpdate>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let raw_ids: Vec<String> = ids.iter().map(|id| id.as_str().to_owned()).collect();

        // `\Deleted` is the trash (ADR 0018 Q4): the seam's
        // mark-then-expunge becomes trash-then-permanent-delete.
        if let FlagChange::Add(flags) = change
            && flags.contains(&Flag::Deleted)
        {
            let trashed = self
                .connection
                .run(&CancelToken::new(), move |client| {
                    let mut trashed = Vec::new();
                    for id in raw_ids {
                        client
                            .message_trash(&id)
                            .map_err(|error| gmail_error("messages.trash", error))?;
                        trashed.push(id);
                    }
                    Ok(trashed)
                })
                .await?;
            return Ok(trashed
                .into_iter()
                .map(|id| FlagUpdate {
                    remote_id: RemoteId::new(id),
                    flags: FlagSet::from_iter([Flag::Deleted]),
                    mod_seq: None,
                })
                .collect());
        }

        let (adds, removes) = convert::label_changes(change);
        let touched = self.modify(raw_ids, adds, removes).await?;

        // Report what the labels say now, per the trait's contract.
        let mut updates = Vec::new();
        for id in touched {
            let message = self.get(id.clone(), GmailMessageFormat::Minimal).await?;
            updates.push(FlagUpdate {
                remote_id: RemoteId::new(id),
                flags: convert::flags(&message.label_ids),
                mod_seq: None,
            });
        }
        Ok(updates)
    }

    async fn move_messages(
        &self,
        from: &str,
        ids: &[RemoteId],
        to: &str,
    ) -> BackendResult<Vec<UidMapping>> {
        self.transfer(from, ids, to, true).await
    }

    async fn copy_messages(
        &self,
        from: &str,
        ids: &[RemoteId],
        to: &str,
    ) -> BackendResult<Vec<UidMapping>> {
        self.transfer(from, ids, to, false).await
    }

    async fn expunge(
        &self,
        _mailbox: &str,
        ids: Option<&[RemoteId]>,
    ) -> BackendResult<Vec<RemoteId>> {
        // Only ever targeted: an untargeted expunge would need "everything
        // in the trash", and permanently deleting mail this adapter was
        // not handed by id is exactly the overreach the seam's UID EXPUNGE
        // rule exists to prevent.
        let Some(ids) = ids else {
            return Ok(Vec::new());
        };
        let raw_ids: Vec<String> = ids.iter().map(|id| id.as_str().to_owned()).collect();
        self.connection
            .run(&CancelToken::new(), move |client| {
                let mut deleted = Vec::new();
                for id in raw_ids {
                    client
                        .message_delete(&id)
                        .map_err(|error| gmail_error("messages.delete", error))?;
                    deleted.push(RemoteId::new(id));
                }
                Ok(deleted)
            })
            .await
    }

    async fn append(
        &self,
        mailbox: &str,
        message: &AppendMessage,
    ) -> BackendResult<Option<UidMapping>> {
        let label = self.resolve(mailbox).await?;
        let mut label_ids: Vec<String> = label.into_iter().collect();
        if !message.flags.is_seen() {
            label_ids.push("UNREAD".to_owned());
        }
        if message.flags.is_flagged() {
            label_ids.push("STARRED".to_owned());
        }
        let raw = encode_raw(&message.raw);

        let created = self
            .connection
            .run(&CancelToken::new(), move |client| {
                let message = GmailMessage {
                    raw: Some(raw),
                    label_ids,
                    ..Default::default()
                };
                let coroutine = GmailMessageInsert::new(
                    &client.auth,
                    &client.user_id,
                    &message,
                    Some(io_gmail::v1::rest::messages::GmailInternalDateSource::DateHeader),
                    false,
                )
                .map_err(|error| BackendError::Protocol {
                    reason: format!("messages.insert: {error}"),
                })?;
                client
                    .run(coroutine)
                    .map(|out| out.response.id)
                    .map_err(|error| gmail_error("messages.insert", error))
            })
            .await?;

        if created.is_empty() {
            return Ok(None);
        }
        Ok(Some(UidMapping {
            source: Uid::new(0),
            destination: Uid::new(0),
            uid_validity: UidValidity::new(convert::GENERATION),
            destination_remote_id: RemoteId::new(created),
        }))
    }

    async fn find_by_message_id(
        &self,
        _mailbox: &str,
        message_id: &str,
    ) -> BackendResult<Option<RemoteId>> {
        // Gmail's search speaks the header natively. The mailbox is not
        // part of the question: any copy proves arrival, per the trait.
        let query = format!("rfc822msgid:{}", message_id.trim_matches(['<', '>']));
        self.connection
            .run(&CancelToken::new(), move |client| {
                let out = client
                    .messages_list(&GmailMessagesListParams {
                        q: Some(&query),
                        label_ids: &[],
                        max_results: Some(1),
                        page_token: None,
                        include_spam_trash: true,
                    })
                    .map_err(|error| gmail_error("messages.list", error))?;
                Ok(out
                    .response
                    .messages
                    .into_iter()
                    .next()
                    .map(|message| RemoteId::new(message.id)))
            })
            .await
    }

    async fn idle(
        &self,
        _mailbox: &str,
        timeout: Duration,
        cancel: &CancelToken,
    ) -> BackendResult<Vec<MailboxEvent>> {
        // No IDLE claim; the watcher polls `status`. The history-poll
        // coroutine is the native delta seam's job (ADR 0018 Q3).
        let deadline = tokio::time::Instant::now() + timeout;
        while tokio::time::Instant::now() < deadline {
            if cancel.is_cancelled() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Ok(Vec::new())
    }
}

impl GmailBackend {
    async fn transfer(
        &self,
        from: &str,
        ids: &[RemoteId],
        to: &str,
        remove_source: bool,
    ) -> BackendResult<Vec<UidMapping>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        // The archive is the absence of labels (ADR 0018 Q4): moving there
        // adds nothing, moving out of it removes nothing.
        let adds: Vec<String> = self.resolve(to).await?.into_iter().collect();
        let removes: Vec<String> = if remove_source {
            self.resolve(from).await?.into_iter().collect()
        } else {
            Vec::new()
        };
        let raw_ids: Vec<String> = ids.iter().map(|id| id.as_str().to_owned()).collect();
        let moved = self.modify(raw_ids, adds, removes).await?;

        Ok(moved
            .into_iter()
            .map(|id| UidMapping {
                source: Uid::new(0),
                destination: Uid::new(0),
                uid_validity: UidValidity::new(convert::GENERATION),
                destination_remote_id: RemoteId::new(id),
            })
            .collect())
    }
}
