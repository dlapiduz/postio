//! The [`MailBackend`] implementation.
//!
//! One rule shapes everything here: the engine addresses messages by
//! [`RemoteId`], and for JMAP that id is the server's `Email` id verbatim.
//! The `uid` this adapter reports is a *synthetic enumeration position* —
//! where the email sat in the `receivedAt`-ascending order of its mailbox
//! at fetch time — because the engine's pull machinery counts a uid range
//! (ADR 0018 Q3 keeps that IMAP-shaped until the native delta seam). A
//! position is not an identity: rows are matched by `remote_id`, so a
//! position shifting between passes costs a re-fetch, never a mix-up.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use io_jmap::rfc8620::filter::JmapFilter;

use io_jmap::rfc8621::email::get::JmapEmailGetOptions;
use io_jmap::rfc8621::email::import::JmapEmailImportArgs;
use io_jmap::rfc8621::email::query::{JmapEmailComparator, JmapEmailFilter, JmapEmailQueryOptions};
use io_jmap::rfc8621::email::set::{JmapEmailPatch, JmapEmailSetArgs};
use io_jmap::rfc8621::mailbox::JmapMailbox;
use io_jmap::rfc8621::mailbox::get::JmapMailboxGetOptions;
use postio_account::backend::{
    AppendMessage, BackendError, BackendResult, BodyPart, BodySink, Capabilities, FetchedBody,
    FetchedMessage, FlagChange, FlagUpdate, MailBackend, MailboxEvent, MailboxFilter,
    MailboxStatus, MailboxSummary, SelectMode, UidMapping, UidSet,
};
use postio_account::cancel::CancelToken;
use postio_model::{FlagSet, Generation, ModSeq, RemoteId, Uid, UidValidity};
use url::Url;

use crate::convert;
use crate::error::backend_error;
use crate::session::JmapConnection;

/// A [`MailBackend`] speaking RFC 8620/8621.
pub struct JmapBackend {
    connection: JmapConnection,
}

impl fmt::Debug for JmapBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JmapBackend")
            .field("connection", &self.connection)
            .finish()
    }
}

impl JmapBackend {
    /// A backend against `session_url`, authenticating with `token`.
    pub fn new(session_url: Url, token: &str) -> Self {
        Self {
            connection: JmapConnection::new(session_url, token),
        }
    }

    /// A backend whose bearer comes from the account's token source.
    pub fn with_token_source(
        session_url: Url,
        key: postio_account::secret::AccountKey,
        tokens: std::sync::Arc<dyn postio_account::auth::TokenSource>,
    ) -> Self {
        Self {
            connection: JmapConnection::with_token_source(session_url, key, tokens),
        }
    }

    /// The mailbox object whose assembled path is `path`.
    async fn mailbox(&self, path: &str) -> BackendResult<JmapMailbox> {
        let wanted = path.to_owned();
        let (mailbox, _) = self
            .connection
            .run(&CancelToken::new(), move |client| {
                let out = client
                    .mailbox_get(JmapMailboxGetOptions::default())
                    .map_err(|error| backend_error("Mailbox/get", error))?;
                let by_id: BTreeMap<String, JmapMailbox> = out
                    .mailboxes
                    .iter()
                    .filter_map(|mailbox| Some((mailbox.id.clone()?, mailbox.clone())))
                    .collect();
                let found = out
                    .mailboxes
                    .iter()
                    .find(|mailbox| convert::summary(mailbox, &by_id).path == wanted)
                    .cloned()
                    .ok_or_else(|| BackendError::NoSuchMailbox {
                        path: wanted.clone(),
                    })?;
                Ok((found, by_id))
            })
            .await?;
        Ok(mailbox)
    }

    /// `path`'s server id, or [`BackendError::NoSuchMailbox`].
    async fn mailbox_id(&self, path: &str) -> BackendResult<String> {
        self.mailbox(path)
            .await?
            .id
            .ok_or_else(|| BackendError::Protocol {
                reason: format!("the server listed `{path}` without an id"),
            })
    }

    /// Every mailbox, with its id, for path assembly.
    async fn mailboxes(&self) -> BackendResult<Vec<JmapMailbox>> {
        self.connection
            .run(&CancelToken::new(), |client| {
                client
                    .mailbox_get(JmapMailboxGetOptions::default())
                    .map(|out| out.mailboxes)
                    .map_err(|error| backend_error("Mailbox/get", error))
            })
            .await
    }

    /// The flags each of `ids` carries now, per `Email/get`.
    async fn flags_of(&self, ids: Vec<String>) -> BackendResult<Vec<FlagUpdate>> {
        self.connection
            .run(&CancelToken::new(), move |client| {
                let out = client
                    .email_get(ids, JmapEmailGetOptions::default())
                    .map_err(|error| backend_error("Email/get", error))?;
                Ok(out
                    .emails
                    .iter()
                    .filter_map(|email| {
                        Some(FlagUpdate {
                            remote_id: RemoteId::new(email.id.clone()?),
                            flags: convert::flags(email.keywords.as_ref()),
                            mod_seq: None,
                        })
                    })
                    .collect())
            })
            .await
    }
}

/// The capability names this backend reports, over whatever URNs the
/// session advertised: enough that the engine gates correctly. None of
/// CONDSTORE, UIDPLUS or IDLE is claimed — see the crate docs for what
/// each omission means.
fn capabilities(session_urns: impl IntoIterator<Item = String>) -> Capabilities {
    Capabilities::from_names(session_urns)
}

#[async_trait]
impl MailBackend for JmapBackend {
    fn describe(&self) -> &'static str {
        "jmap"
    }

    async fn connect(&self) -> BackendResult<Capabilities> {
        let session = self.connection.connect(&CancelToken::new()).await?;
        let urns: Vec<String> = session.capabilities.keys().cloned().collect();
        if urns.is_empty() {
            return Err(BackendError::EmptyCapabilities {
                host: self.connection.host(),
            });
        }
        Ok(capabilities(urns))
    }

    async fn disconnect(&self) -> BackendResult<()> {
        Ok(())
    }

    async fn capabilities(&self) -> BackendResult<Capabilities> {
        match self.connection.session() {
            Some(session) => Ok(capabilities(session.capabilities.keys().cloned())),
            None => Err(BackendError::NotConnected {
                context: "no JMAP session has been resolved".to_owned(),
            }),
        }
    }

    async fn list_mailboxes(&self, _filter: &MailboxFilter) -> BackendResult<Vec<MailboxSummary>> {
        let mailboxes = self.mailboxes().await?;
        let by_id: BTreeMap<String, JmapMailbox> = mailboxes
            .iter()
            .filter_map(|mailbox| Some((mailbox.id.clone()?, mailbox.clone())))
            .collect();
        Ok(mailboxes
            .iter()
            .map(|mailbox| convert::summary(mailbox, &by_id))
            .collect())
    }

    async fn select(&self, path: &str, _mode: SelectMode) -> BackendResult<MailboxStatus> {
        self.status(path).await
    }

    async fn status(&self, path: &str) -> BackendResult<MailboxStatus> {
        let mailbox = self.mailbox(path).await?;
        Ok(MailboxStatus {
            path: path.to_owned(),
            // One generation, for ever: JMAP ids never renumber.
            generation: Generation::new(convert::GENERATION),
            // The enumeration ceiling: positions run 1..=total.
            uid_next: Uid::new(mailbox.total_emails + 1),
            exists: mailbox.total_emails,
            unseen: Some(mailbox.unread_emails),
            highest_mod_seq: None,
            permanent_flags: FlagSet::new(),
            // RFC 8621 keywords are free-form.
            can_create_keywords: true,
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
            // Unreachable through the engine: CONDSTORE is never claimed.
            return Err(BackendError::Unsupported {
                capability: postio_account::backend::Capability::CondStore,
            });
        }
        let mut positions = uids.uids();
        let Some(first) = positions.next() else {
            return Ok(Vec::new());
        };
        let last = positions.last().unwrap_or(first);
        let mailbox_id = self.mailbox_id(mailbox).await?;
        let wanted = uids.clone();
        self.connection
            .run(cancel, move |client| {
                let out = client
                    .email_query(JmapEmailQueryOptions {
                        filter: Some(JmapFilter::Condition(JmapEmailFilter {
                            in_mailbox: Some(mailbox_id),
                            ..Default::default()
                        })),
                        sort: Some(vec![JmapEmailComparator {
                            property:
                                io_jmap::rfc8621::email::query::JmapEmailSortProperty::ReceivedAt,
                            is_ascending: Some(true),
                            collation: None,
                            keyword: None,
                        }]),
                        position: Some(u64::from(first.get().saturating_sub(1))),
                        limit: Some(u64::from(last.get() - first.get() + 1)),
                        properties: None,
                    })
                    .map_err(|error| backend_error("Email/query", error))?;
                Ok(out
                    .emails
                    .iter()
                    .enumerate()
                    .filter_map(|(offset, email)| {
                        let position = first.get() + offset as u32;
                        if !wanted.contains(Uid::new(position)) {
                            return None;
                        }
                        convert::fetched(email, position)
                    })
                    .collect())
            })
            .await
    }

    async fn fetch_part(
        &self,
        _mailbox: &str,
        id: &RemoteId,
        part: &BodyPart,
        sink: &mut dyn BodySink,
        cancel: &CancelToken,
    ) -> BackendResult<FetchedBody> {
        if !matches!(part, BodyPart::Whole) {
            // Fetched headers carry no BODYSTRUCTURE, so nothing above the
            // seam ever learns a section to ask for; the backfill's
            // no-sections path fetches the whole message instead.
            return Err(BackendError::Unsupported {
                capability: postio_account::backend::Capability::Binary,
            });
        }

        // The raw blob id, then the blob itself.
        let email_id = id.as_str().to_owned();
        let (blob_id, session) = {
            let wanted = email_id.clone();
            let out = self
                .connection
                .run(cancel, move |client| {
                    client
                        .email_get(vec![wanted], JmapEmailGetOptions::default())
                        .map_err(|error| backend_error("Email/get", error))
                })
                .await?;
            let email = out
                .emails
                .first()
                .ok_or_else(|| BackendError::NoSuchMessage {
                    mailbox: _mailbox.to_owned(),
                    uid: 0,
                })?;
            let blob_id = email
                .blob_id
                .clone()
                .ok_or_else(|| BackendError::Protocol {
                    reason: format!("email `{email_id}` has no blobId"),
                })?;
            let session = self
                .connection
                .session()
                .ok_or_else(|| BackendError::NotConnected {
                    context: "no JMAP session has been resolved".to_owned(),
                })?;
            (blob_id, session)
        };

        let account_id = session.primary_account_id_for(io_jmap::rfc8621::JMAP_MAIL_CAPABILITY);
        let download_url = expand_download_url(&session.download_url, &account_id, &blob_id)?;
        let bytes = self
            .connection
            .run(cancel, move |client| {
                client
                    .blob_download(&download_url)
                    .map_err(|error| backend_error("blob download", error))
            })
            .await?;

        let mut written = 0u64;
        sink.chunk(&bytes).await?;
        written += bytes.len() as u64;
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
        let patch = patch_for(change);
        let updates: BTreeMap<String, JmapEmailPatch> = ids
            .iter()
            .map(|id| (id.as_str().to_owned(), patch.clone()))
            .collect();

        let updated = self
            .connection
            .run(&CancelToken::new(), move |client| {
                let out = client
                    .email_set(JmapEmailSetArgs {
                        update: Some(updates),
                        ..Default::default()
                    })
                    .map_err(|error| backend_error("Email/set", error))?;
                Ok(out.updated.keys().cloned().collect::<Vec<_>>())
            })
            .await?;

        // "Reports what they are now": read the keywords back, so the
        // caller applies the server's truth rather than this adapter's
        // guess. Ids the set did not update are dropped — the seam reads an
        // empty update as "not in this mailbox any more".
        self.flags_of(updated).await
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
        mailbox: &str,
        ids: Option<&[RemoteId]>,
    ) -> BackendResult<Vec<RemoteId>> {
        let targets: Vec<String> = match ids {
            Some(ids) => ids.iter().map(|id| id.as_str().to_owned()).collect(),
            None => {
                // The untargeted form destroys what is marked, exactly like
                // `EXPUNGE`: everything in this mailbox carrying the
                // `$deleted` marker `store_flags` left.
                let mailbox_id = self.mailbox_id(mailbox).await?;
                self.connection
                    .run(&CancelToken::new(), move |client| {
                        let out = client
                            .email_query(JmapEmailQueryOptions {
                                filter: Some(JmapFilter::Condition(JmapEmailFilter {
                                    in_mailbox: Some(mailbox_id),
                                    has_keyword: Some("$deleted".to_owned()),
                                    ..Default::default()
                                })),
                                ..Default::default()
                            })
                            .map_err(|error| backend_error("Email/query", error))?;
                        Ok(out
                            .emails
                            .iter()
                            .filter_map(|email| email.id.clone())
                            .collect::<Vec<_>>())
                    })
                    .await?
            }
        };
        if targets.is_empty() {
            return Ok(Vec::new());
        }

        self.connection
            .run(&CancelToken::new(), move |client| {
                let mut args = JmapEmailSetArgs::default();
                for id in &targets {
                    args.destroy(id.clone());
                }
                let out = client
                    .email_set(args)
                    .map_err(|error| backend_error("Email/set destroy", error))?;
                Ok(out.destroyed.into_iter().map(RemoteId::new).collect())
            })
            .await
    }

    async fn append(
        &self,
        mailbox: &str,
        message: &AppendMessage,
    ) -> BackendResult<Option<UidMapping>> {
        let mailbox_id = self.mailbox_id(mailbox).await?;
        let session = self
            .connection
            .session()
            .ok_or_else(|| BackendError::NotConnected {
                context: "no JMAP session has been resolved".to_owned(),
            })?;
        let account_id = session.primary_account_id_for(io_jmap::rfc8621::JMAP_MAIL_CAPABILITY);
        let upload_url = expand_upload_url(&session.upload_url, &account_id)?;

        let raw = message.raw.clone();
        let keywords: BTreeMap<String, bool> = message
            .flags
            .iter()
            .filter_map(convert::keyword)
            .map(|keyword| (keyword, true))
            .collect();
        let received_at = message.internal_date.map(|date| date.to_rfc3339());

        // Two calls, two streams: each request stands alone, per the
        // session module's connection story.
        let blob_id = self
            .connection
            .run(&CancelToken::new(), move |client| {
                client
                    .blob_upload(&upload_url, "message/rfc822", raw)
                    .map(|uploaded| uploaded.blob_id)
                    .map_err(|error| backend_error("blob upload", error))
            })
            .await?;
        let created = self
            .connection
            .run(&CancelToken::new(), move |client| {
                let mut emails = BTreeMap::new();
                emails.insert(
                    "postio-append".to_owned(),
                    JmapEmailImportArgs {
                        blob_id,
                        mailbox_ids: BTreeMap::from([(mailbox_id, true)]),
                        keywords: (!keywords.is_empty()).then_some(keywords),
                        received_at,
                    },
                );
                let out = client
                    .email_import(emails)
                    .map_err(|error| backend_error("Email/import", error))?;
                Ok(out
                    .created
                    .get("postio-append")
                    .and_then(|email| email.id.clone()))
            })
            .await?;

        Ok(created.map(|id| UidMapping {
            // No uid space to land in: the identity is the whole answer.
            source: Uid::new(0),
            destination: Uid::new(0),
            uid_validity: UidValidity::new(convert::GENERATION),
            destination_remote_id: RemoteId::new(id),
        }))
    }

    async fn idle(
        &self,
        _mailbox: &str,
        timeout: Duration,
        cancel: &CancelToken,
    ) -> BackendResult<Vec<MailboxEvent>> {
        // No IDLE capability is claimed, so the watcher polls `status`
        // instead of calling this; honouring the contract anyway costs a
        // sleep. The Event Source push channel is the native seam's job
        // (ADR 0018 Q3).
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

impl JmapBackend {
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
        let from_id = self.mailbox_id(from).await?;
        let to_id = self.mailbox_id(to).await?;

        let mut patch = JmapEmailPatch::default().add_to_mailbox(to_id);
        if remove_source {
            patch = patch.remove_from_mailbox(from_id);
        }
        let updates: BTreeMap<String, JmapEmailPatch> = ids
            .iter()
            .map(|id| (id.as_str().to_owned(), patch.clone()))
            .collect();

        let moved = self
            .connection
            .run(&CancelToken::new(), move |client| {
                let out = client
                    .email_set(JmapEmailSetArgs {
                        update: Some(updates),
                        ..Default::default()
                    })
                    .map_err(|error| backend_error("Email/set move", error))?;
                Ok(out.updated.keys().cloned().collect::<Vec<_>>())
            })
            .await?;

        // A JMAP move keeps the id: the mapping's destination identity is
        // the same message.
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

/// The flag change as an RFC 7396 patch.
fn patch_for(change: &FlagChange) -> JmapEmailPatch {
    match change {
        FlagChange::Add(flags) => flags
            .iter()
            .filter_map(convert::keyword)
            .fold(JmapEmailPatch::default(), |patch, keyword| {
                patch.set_keyword(keyword)
            }),
        FlagChange::Remove(flags) => flags
            .iter()
            .filter_map(convert::keyword)
            .fold(JmapEmailPatch::default(), |patch, keyword| {
                patch.unset_keyword(keyword)
            }),
        FlagChange::Replace(flags) => {
            let keywords: BTreeMap<String, bool> = flags
                .iter()
                .filter_map(convert::keyword)
                .map(|keyword| (keyword, true))
                .collect();
            JmapEmailPatch::default().replace_keywords(keywords)
        }
    }
}

/// RFC 6570 level-1 expansion of the session's blob templates.
fn expand_download_url(template: &str, account_id: &str, blob_id: &str) -> BackendResult<Url> {
    template
        .replace("{accountId}", account_id)
        .replace("{blobId}", blob_id)
        .replace("{type}", "application/octet-stream")
        .replace("{name}", "message")
        .parse()
        .map_err(|error| BackendError::Protocol {
            reason: format!("the session's downloadUrl does not expand to a URL: {error}"),
        })
}

fn expand_upload_url(template: &str, account_id: &str) -> BackendResult<Url> {
    template
        .replace("{accountId}", account_id)
        .parse()
        .map_err(|error| BackendError::Protocol {
            reason: format!("the session's uploadUrl does not expand to a URL: {error}"),
        })
}
