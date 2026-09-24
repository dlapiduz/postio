//! The store half of writing mail: saving, discarding and queueing drafts,
//! and what the composer reads to fill itself in.
//!
//! Moved out of `postio-app` (specs/005-tui-frontend T015) so the desktop and
//! the terminal save a draft the same way. Both reach them through the
//! Compose [`Req`](postio_client::protocol::Req)s, and each client's draft
//! writes go through a [`DraftWriter`] of its own, in the order it made them
//! (#1608). What stays in a frontend is its own half: the desktop's composer
//! signals, and `gio`'s MIME sniff.
//!
//! Every function here logs and answers `None` or an error rather than
//! panicking: a composer that cannot read a contact still lets the user type
//! the address.

use chrono::{DateTime, Utc};
use postio_model::contact_group::RecipientCandidate;
use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_model::listing::StoreError;
use postio_model::{
    Account, Attachment, Draft, DraftId, EmailAddress, Message, OperationTarget, SignatureId,
    signature_default,
};
use postio_storage::repository::{
    AccountRepository, CancelSendOutcome, ContactGroupRepository, ContactRepository,
    DraftRepository, MailboxRepository, MessageRepository, OperationQueueRepository,
};
use postio_storage::{BlobStore, Store};

/// How many recipient suggestions a prefix offers.
pub const SUGGESTION_LIMIT: u32 = 8;

/// Autosave: the draft is written, and its upload to the server's Drafts
/// folder is queued in the same transaction (`DraftRepository::save_and_sync`).
///
/// `interactive_write` rather than a bare connection: a draft autosave is a
/// write the person typing is waiting on, so it goes ahead of a backfill's
/// bulk writes rather than queueing behind them (#425).
pub async fn save_draft(database: &Store, draft: &mut Draft) -> postio_storage::Result<()> {
    let (connection, _permit) = database.interactive_write().await?;
    DraftRepository::new(&connection)
        .save_and_sync(draft, Utc::now())
        .await?;
    Ok(())
}

/// Discard: the local row goes now, and the server copy is queued for removal.
pub async fn delete_draft(database: &Store, id: DraftId) -> postio_storage::Result<()> {
    let (connection, _permit) = database.interactive_write().await?;
    DraftRepository::new(&connection)
        .discard(id, Utc::now())
        .await?;
    Ok(())
}

/// Send: the draft goes to `Queued` and its `Operation::Send` row is written,
/// in one transaction — see `DraftRepository::queue_send`. With `at`, the
/// drainer leaves it alone until then (`queue_send_at`).
pub async fn queue_send(
    database: &Store,
    draft: &mut Draft,
    at: Option<DateTime<Utc>>,
) -> postio_storage::Result<()> {
    let (connection, _permit) = database.interactive_write().await?;
    let drafts = DraftRepository::new(&connection);
    match at {
        Some(at) => {
            drafts.queue_send_at(draft, Utc::now(), at).await?;
        }
        None => {
            drafts.queue_send(draft, Utc::now()).await?;
        }
    }
    Ok(())
}

/// What one composer asks of the store, in the order it asked (#1608).
enum DraftOp {
    /// Autosave this composition's draft; answer with the id it has.
    Save {
        generation: u64,
        draft: Draft,
        reply: tokio::sync::oneshot::Sender<Result<DraftId, StoreError>>,
    },
    /// Queue it to send -- now, or at `at` -- and answer with the Drafts
    /// folder, whose list just changed, once the queue write landed.
    Send {
        generation: u64,
        draft: Draft,
        at: Option<DateTime<Utc>>,
        reply: tokio::sync::oneshot::Sender<Result<Option<MailboxId>, StoreError>>,
    },
    /// This composition was closed empty: its autosaved row goes.
    Discard {
        generation: u64,
        known: Option<DraftId>,
        reply: tokio::sync::oneshot::Sender<()>,
    },
}

/// Writes one composer's drafts, one request at a time, in the order the
/// composer made them (#1608). Each frontend has its own.
///
/// Every save, send and discard once ran on the GTK thread, each waiting for
/// the interactive write permit, so an autosave tick could stall the thread
/// that draws for as long as a sync's unit took. Here the composer hands the
/// request over and returns; the answer arrives when the write has landed.
///
/// What the synchronous version guaranteed by construction is kept by order
/// and by remembering one id. A composition is named by a `generation` the
/// composer chooses. Its first save assigns an id, which the writer
/// remembers, so a second save made before the first landed updates the same
/// row rather than inserting another, and a send queues the row the saves
/// made. A discard is behind every save its composition made, so it deletes
/// the row they left.
#[derive(Clone, Debug)]
pub struct DraftWriter {
    ops: async_channel::Sender<DraftOp>,
}

impl std::fmt::Debug for DraftOp {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            DraftOp::Save { .. } => "Save",
            DraftOp::Send { .. } => "Send",
            DraftOp::Discard { .. } => "Discard",
        })
    }
}

/// A write the writer did not get to: the runtime it ran on is gone.
fn gone() -> StoreError {
    StoreError::new("the draft writer has stopped")
}

impl DraftWriter {
    /// A writer for one composer, running on `runtime` until the last clone
    /// of it is dropped.
    pub fn spawn(database: Store, runtime: &tokio::runtime::Handle) -> Self {
        let (ops, requests) = async_channel::unbounded::<DraftOp>();
        runtime.spawn(async move {
            // The composition whose draft was saved last, and its id.
            let mut current: Option<(u64, DraftId)> = None;
            let id_for = |current: &Option<(u64, DraftId)>, generation: u64, draft: &mut Draft| {
                if !draft.id.is_assigned()
                    && let Some((saved_for, id)) = *current
                    && saved_for == generation
                {
                    draft.id = id;
                }
            };
            while let Ok(op) = requests.recv().await {
                match op {
                    DraftOp::Save {
                        generation,
                        mut draft,
                        reply,
                    } => {
                        id_for(&current, generation, &mut draft);
                        let saved = match save_draft(&database, &mut draft).await {
                            Ok(()) => {
                                current = Some((generation, draft.id));
                                Ok(draft.id)
                            }
                            Err(error) => {
                                tracing::error!(%error, "could not autosave the draft: {error}");
                                Err(StoreError::from(error))
                            }
                        };
                        let _ = reply.send(saved);
                    }
                    DraftOp::Send {
                        generation,
                        mut draft,
                        at,
                        reply,
                    } => {
                        id_for(&current, generation, &mut draft);
                        // Taken on failure too, and deliberately: a queue write
                        // that fails leaves the autosaved row where it is,
                        // `Editing`, recoverable, and the close that follows a
                        // send must not delete the user's words on the way out.
                        // Losing the send is recoverable, losing the message is
                        // not.
                        if current.is_some_and(|(saved_for, _)| saved_for == generation) {
                            current = None;
                        }
                        let moved = match queue_send(&database, &mut draft, at).await {
                            Ok(()) if at.is_none() => {
                                Ok(drafts_mailbox(&database, draft.account_id).await)
                            }
                            Ok(()) => Ok(None),
                            Err(error) => {
                                tracing::error!(%error, "could not queue the draft for sending: {error}");
                                Err(StoreError::from(error))
                            }
                        };
                        let _ = reply.send(moved);
                    }
                    DraftOp::Discard {
                        generation,
                        known,
                        reply,
                    } => {
                        let id = match current {
                            Some((saved_for, id)) if saved_for == generation => {
                                current = None;
                                Some(id)
                            }
                            _ => known,
                        };
                        if let Some(id) = id
                            && let Err(error) = delete_draft(&database, id).await
                        {
                            tracing::warn!(%error, "could not clear the finished draft");
                        }
                        let _ = reply.send(());
                    }
                }
            }
        });
        DraftWriter { ops }
    }

    /// Autosave `draft` as composition `generation`; the answer is its id.
    pub fn save(
        &self,
        generation: u64,
        draft: Draft,
    ) -> impl Future<Output = Result<DraftId, StoreError>> + Send + 'static {
        let (reply, answer) = tokio::sync::oneshot::channel();
        self.hand_over(DraftOp::Save {
            generation,
            draft,
            reply,
        });
        async move { answer.await.unwrap_or_else(|_| Err(gone())) }
    }

    /// Queue `draft` to send, now or at `at`; the answer is the Drafts folder
    /// whose list moved, when an immediate send moved one.
    pub fn send(
        &self,
        generation: u64,
        draft: Draft,
        at: Option<DateTime<Utc>>,
    ) -> impl Future<Output = Result<Option<MailboxId>, StoreError>> + Send + 'static {
        let (reply, answer) = tokio::sync::oneshot::channel();
        self.hand_over(DraftOp::Send {
            generation,
            draft,
            at,
            reply,
        });
        async move { answer.await.unwrap_or_else(|_| Err(gone())) }
    }

    /// Drop composition `generation`'s row -- the one its saves made, else
    /// `known`.
    pub fn discard(
        &self,
        generation: u64,
        known: Option<DraftId>,
    ) -> impl Future<Output = ()> + Send + 'static {
        let (reply, answer) = tokio::sync::oneshot::channel();
        self.hand_over(DraftOp::Discard {
            generation,
            known,
            reply,
        });
        async move {
            let _ = answer.await;
        }
    }

    fn hand_over(&self, op: DraftOp) {
        // Unbounded: the composer never waits on the writer, which is the
        // point. A closed channel means the runtime is gone, and the answer
        // says so.
        let _ = self.ops.try_send(op);
    }
}

/// The account's Drafts folder, which is where a draft's row lives whatever
/// its send state — the Outbox is a predicate over that folder, not a second
/// one (spec 003).
///
/// `None` before the first sync has found one, in which case the draft has no
/// row to have moved and there is nothing to announce.
pub async fn drafts_mailbox(database: &Store, account: AccountId) -> Option<MailboxId> {
    let connection = database.connect().await.ok()?;
    MailboxRepository::new(&connection)
        .by_role(account, postio_model::MailboxRole::Drafts)
        .await
        .ok()
        .flatten()
        .map(|mailbox| mailbox.id)
}

/// Takes a queued draft back for editing, if it has not started sending.
pub async fn cancel_queued_send(database: &Store, id: DraftId) -> Option<Draft> {
    let connection = database
        .connect()
        .await
        .map_err(|error| tracing::warn!(%error, "could not open the store to cancel a send"))
        .ok()?;
    let drafts = DraftRepository::new(&connection);
    match drafts.cancel_send(id, Utc::now()).await {
        Ok(CancelSendOutcome::Cancelled) => drafts
            .get(id)
            .await
            .map_err(|error| tracing::warn!(%error, "could not reread a draft after cancelling its send"))
            .ok()
            .flatten(),
        Ok(CancelSendOutcome::NotQueued | CancelSendOutcome::AlreadyInFlight) => None,
        Err(error) => {
            tracing::warn!(%error, "could not cancel a queued draft's send");
            None
        }
    }
}

/// Why this draft's last send attempt gave up, if it did.
///
/// Read from the queue row rather than from the draft, because that is where
/// the drainer writes it and a second copy is one that can come to disagree
/// with the first (#1487).
pub async fn why_the_send_failed(database: &Store, id: DraftId) -> Option<String> {
    let connection = database
        .connect()
        .await
        .map_err(|error| tracing::warn!(%error, "could not open the store to read a send failure"))
        .ok()?;
    OperationQueueRepository::new(&connection)
        .last_failure_for(OperationTarget::Draft(id))
        .await
        .map_err(|error| tracing::warn!(%error, "could not read why a send failed"))
        .ok()
        .flatten()
}

/// The draft a message row is listing, if it is listing one.
pub async fn draft_behind(database: &Store, message: MessageId) -> Option<Draft> {
    let connection = database
        .connect()
        .await
        .map_err(|error| tracing::warn!(%error, "could not open the store to resume a draft"))
        .ok()?;
    DraftRepository::new(&connection)
        .by_message(message)
        .await
        .map_err(|error| tracing::warn!(%error, "could not read the draft behind a row"))
        .ok()?
}

/// Every draft `account` has, for crash recovery.
pub async fn drafts_of(database: &Store, account: AccountId) -> postio_storage::Result<Vec<Draft>> {
    let connection = database.connect().await?;
    DraftRepository::new(&connection)
        .list_for_account(account)
        .await
}

/// Record that a session began, and answer the draft `account` was still
/// writing if the last session died without ending cleanly (#491).
///
/// Only after a crash: `DraftState::Editing` alone is not evidence of one --
/// Esc parks a draft in exactly that state on purpose -- and a client that
/// opens into a stale compose buffer instead of the inbox reads as broken.
/// `begin_session` is what knows how the last session ended; asking it also
/// marks this one open, so this is asked once per process.
///
/// The most recently edited draft worth keeping, by the rule Esc uses
/// ([`closing`](postio_model::draft::closing)): an untouched buffer is not
/// work, and recovering one is self-perpetuating -- the composer it reopens
/// autosaves another empty `Editing` row, and every unclean stop after the
/// first would open the client into it.
pub async fn recover(database: &Store, account: AccountId) -> Option<Draft> {
    if !postio_session::begin_session(database).await {
        return None;
    }
    let drafts = match drafts_of(database, account).await {
        Ok(drafts) => drafts,
        Err(error) => {
            tracing::error!(%error, "could not read drafts to recover: {error}");
            return None;
        }
    };
    drafts.into_iter().find(|draft| {
        draft.state == postio_model::DraftState::Editing
            && postio_model::draft::closing(draft) == postio_model::draft::Closing::Keep
    })
}

/// The bytes stored under `blob`, or none when they cannot be read.
///
/// Blocking: callers run it on a blocking thread. A composer's inline image
/// is drawn from these, the same cost the reader pays per inline image.
pub fn blob_bytes(blobs: &BlobStore, blob: &postio_model::ids::BlobId) -> Option<Vec<u8>> {
    let mut file = blobs
        .reader(blob)
        .map_err(|error| tracing::warn!(%error, "could not read an inline image blob"))
        .ok()?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut bytes)
        .map_err(|error| tracing::warn!(%error, "could not read an inline image blob"))
        .ok()?;
    Some(bytes)
}

/// The account a composer sends from: its identities, signatures and size
/// limit.
pub async fn account(database: &Store, account: AccountId) -> Option<Account> {
    let connection = database.connect().await.ok()?;
    match AccountRepository::new(&connection).get(account).await {
        Ok(found) => {
            if found.is_none() {
                tracing::warn!("the composer's account is not in the database");
            }
            found
        }
        Err(error) => {
            tracing::warn!(%error, "could not read the account's identities");
            None
        }
    }
}

/// The signature a new draft for `account` starts with: the selected
/// mailbox's override, else the account's default, else none.
pub async fn default_signature(
    database: &Store,
    account: AccountId,
    selected: Option<MailboxId>,
) -> Option<SignatureId> {
    let connection = database
        .read()
        .await
        .map_err(|error| tracing::warn!(%error, "could not resolve a default signature"))
        .ok()?;
    let account_default = AccountRepository::new(&connection)
        .get(account)
        .await
        .ok()
        .flatten()?
        .default_signature_id;
    // Spelled out rather than chained: `and_then` takes a closure, and a
    // closure cannot await.
    let mailbox_signature = match selected {
        Some(id) => MailboxRepository::new(&connection)
            .get(id)
            .await
            .ok()
            .flatten()
            .and_then(|mailbox| mailbox.signature_id),
        None => None,
    };
    signature_default::resolve(mailbox_signature, account_default)
}

/// How many correspondents the finder's `@` can offer: the desktop's bound,
/// there to stop a pathological store rather than to page a normal one.
const CORRESPONDENT_LIMIT: u32 = 50_000;

/// The account's correspondents, as the desktop's `@` reads them. Empty,
/// and logged, when the store cannot be read.
pub async fn correspondents(database: &Store, account: AccountId) -> Vec<postio_model::Contact> {
    let found = async {
        let connection = database.read().await?;
        ContactRepository::new(&connection)
            .search(Some(account), "", CORRESPONDENT_LIMIT)
            .await
    };
    found.await.unwrap_or_else(|error| {
        tracing::warn!(%error, "could not read the correspondents");
        Vec::new()
    })
}

/// Everything recipient completion can offer `account`, on one warm reader:
/// what the desktop's composer holds in memory so each keystroke is a lookup
/// rather than a query. Empty, and logged, when the store cannot be read.
pub async fn recipient_directory(
    database: &Store,
    account: AccountId,
) -> postio_client::protocol::RecipientDirectory {
    let found = async {
        let connection = database.read().await?;
        let groups = ContactGroupRepository::new(&connection);
        let mut named = Vec::new();
        for group in groups.list(Some(account)).await? {
            let members = groups.members(group.id).await?;
            if !members.is_empty() {
                named.push((group.name, members.iter().map(resolved_address).collect()));
            }
        }
        let contacts = ContactRepository::new(&connection)
            .search(Some(account), "", CORRESPONDENT_LIMIT)
            .await?;
        Ok::<_, postio_storage::Error>(postio_client::protocol::RecipientDirectory {
            groups: named,
            contacts,
        })
    };
    found.await.unwrap_or_else(|error| {
        tracing::warn!(%error, "could not read the recipient directory");
        Default::default()
    })
}

/// The account's labels, by name. Empty, and logged, when the store cannot
/// be read.
pub async fn labels(database: &Store, account: AccountId) -> Vec<postio_model::Label> {
    let found = async {
        let connection = database.read().await?;
        postio_storage::repository::LabelRepository::new(&connection)
            .list(account)
            .await
    };
    found.await.unwrap_or_else(|error| {
        tracing::warn!(%error, "could not read the labels");
        Vec::new()
    })
}

/// Recipient completion: contact groups whose name matches `prefix`, then
/// contacts ranked by [`ContactRepository::search`] — groups first, since a
/// group is a deliberate choice the user is more likely typing towards.
pub async fn recipients(
    database: &Store,
    account: AccountId,
    prefix: &str,
) -> Vec<RecipientCandidate> {
    let connection = match database.connect().await {
        Ok(connection) => connection,
        Err(error) => {
            tracing::warn!(%error, "could not search contacts");
            return Vec::new();
        }
    };

    let mut candidates: Vec<RecipientCandidate> = Vec::new();
    let groups = ContactGroupRepository::new(&connection);
    match groups.list(Some(account)).await {
        Ok(list) => {
            let prefix_lower = prefix.to_lowercase();
            for group in list {
                if !group.name.to_lowercase().starts_with(&prefix_lower) {
                    continue;
                }
                match groups.members(group.id).await {
                    // A group with no members yet expands to nothing, so
                    // offering it would be a suggestion that does nothing
                    // when accepted.
                    Ok(members) if !members.is_empty() => {
                        candidates.push(RecipientCandidate::Group {
                            name: group.name,
                            members: members.iter().map(resolved_address).collect(),
                        });
                    }
                    Ok(_) => {}
                    Err(error) => tracing::warn!(%error, "could not read group members"),
                }
            }
        }
        Err(error) => tracing::warn!(%error, "could not search contact groups"),
    }

    match ContactRepository::new(&connection)
        .search(Some(account), prefix, SUGGESTION_LIMIT)
        .await
    {
        Ok(contacts) => candidates.extend(
            contacts
                .iter()
                .map(resolved_address)
                .map(RecipientCandidate::Contact),
        ),
        Err(error) => tracing::warn!(%error, "could not search contacts"),
    }

    candidates.truncate(SUGGESTION_LIMIT as usize);
    candidates
}

/// The address a contact offers: the name the user set, or the last one seen
/// on the address, over the addr-spec `record` accumulated sightings under.
fn resolved_address(contact: &postio_model::Contact) -> EmailAddress {
    let name = contact
        .name
        .clone()
        .or_else(|| contact.address.name.clone());
    EmailAddress::new(name, contact.address.address.clone())
}

/// The message `id` with its body, and the account it belongs to: what a
/// reply or a forward is built from.
pub async fn reply_source(database: &Store, id: MessageId) -> Option<(Message, Account)> {
    let connection = database
        .read()
        .await
        .map_err(|error| tracing::warn!(%error, "could not open a reply source"))
        .ok()?;
    let mut message = MessageRepository::new(&connection)
        .get(id)
        .await
        .ok()
        .flatten()?;
    message.body = postio_session::reading::load_body(&connection, id).await;
    let account = AccountRepository::new(&connection)
        .get(message.account_id)
        .await
        .ok()
        .flatten()?;
    Some((message, account))
}

/// Reads `path` and writes its bytes into `blobs` as an attachment of type
/// `mime_type`.
///
/// Blocking throughout: callers run it on a blocking thread. The type is the
/// caller's, because how to sniff one differs by frontend — the desktop asks
/// shared-mime-info through `gio`; [`guess_mime_type`] is the fallback for a
/// process that has no `gio`.
pub fn attach_file(
    blobs: &BlobStore,
    path: &std::path::Path,
    mime_type: String,
) -> Option<Attachment> {
    let size = std::fs::metadata(path).ok()?.len();
    let file = std::fs::File::open(path)
        // The path is deliberately not logged: an attachment's name is the
        // user's, and a log line is a thing people paste into bug reports.
        .map_err(|error| tracing::warn!(%error, "could not read the file to attach"))
        .ok()?;
    let blob_id = blobs
        .put_reader(file)
        .map_err(|error| tracing::warn!(%error, "could not store the attachment"))
        .ok()?;

    let mut attachment = Attachment::new(MessageId::UNASSIGNED, mime_type, size);
    attachment.filename = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    attachment.blob_id = Some(blob_id);
    Some(attachment)
}

/// Writes pasted image bytes into `blobs` as an inline part the body can
/// point at by `cid:`.
pub fn inline_attachment(blobs: &BlobStore, bytes: Vec<u8>, mime_type: &str) -> Option<Attachment> {
    let size = bytes.len() as u64;
    let blob_id = blobs
        .put(&bytes)
        .map_err(|error| tracing::warn!(%error, "could not store the pasted image"))
        .ok()?;

    let extension = mime_type.strip_prefix("image/").unwrap_or("png");
    let mut attachment = Attachment::new(MessageId::UNASSIGNED, mime_type, size);
    attachment.filename = Some(format!("inline-image.{extension}"));
    attachment.disposition = postio_model::attachment::Disposition::Inline;
    attachment.content_id = Some(format!("{}@postio.invalid", blob_id.as_str()));
    attachment.blob_id = Some(blob_id);
    Some(attachment)
}

/// A MIME type for `path` from its first bytes, then its extension, for a
/// process without shared-mime-info.
///
/// Only the types mail commonly carries; anything else is the generic "some
/// bytes", which is what the desktop falls back to as well. Wrong-but-generic
/// is safe here: the recipient's client sniffs again, and a type that claims
/// too much (an image that is not one) is the only harmful answer.
pub fn guess_mime_type(path: &std::path::Path) -> String {
    let mut head = [0u8; 16];
    let read = std::fs::File::open(path)
        .and_then(|mut file| std::io::Read::read(&mut file, &mut head))
        .unwrap_or(0);
    let head = &head[..read];
    let sniffed = [
        (&b"\x89PNG\r\n\x1a\n"[..], "image/png"),
        (&b"\xff\xd8\xff"[..], "image/jpeg"),
        (&b"GIF8"[..], "image/gif"),
        (&b"%PDF-"[..], "application/pdf"),
        (&b"PK\x03\x04"[..], "application/zip"),
    ]
    .iter()
    .find(|(magic, _)| head.starts_with(magic))
    .map(|(_, mime)| *mime);
    if let Some(mime) = sniffed {
        // A zip is also every OOXML and ODF document; the extension says which.
        if mime != "application/zip" {
            return mime.to_owned();
        }
    }
    if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        return "image/webp".to_owned();
    }
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase());
    let by_extension = match extension.as_deref() {
        Some("txt" | "log") => "text/plain",
        Some("md") => "text/markdown",
        Some("csv") => "text/csv",
        Some("html" | "htm") => "text/html",
        Some("ics") => "text/calendar",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        Some("odt") => "application/vnd.oasis.opendocument.text",
        Some("ods") => "application/vnd.oasis.opendocument.spreadsheet",
        Some("zip") => "application/zip",
        Some("tar") => "application/x-tar",
        Some("gz") => "application/gzip",
        _ => sniffed.unwrap_or("application/octet-stream"),
    };
    by_extension.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("a directory");
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).expect("write");
        (dir, path)
    }

    #[test]
    fn content_outranks_a_misleading_extension() {
        let (_dir, path) = file("screenshot.txt", b"\x89PNG\r\n\x1a\n rest");
        assert_eq!(guess_mime_type(&path), "image/png");
    }

    #[test]
    fn a_document_zip_is_named_by_its_extension() {
        let (_dir, path) = file("minutes.docx", b"PK\x03\x04 rest");
        assert!(guess_mime_type(&path).contains("wordprocessingml"));
        let (_dir, path) = file("bundle.zip", b"PK\x03\x04 rest");
        assert_eq!(guess_mime_type(&path), "application/zip");
    }

    #[test]
    fn what_nothing_recognises_is_some_bytes() {
        let (_dir, path) = file("blob.bin", b"\x00\x01\x02");
        assert_eq!(guess_mime_type(&path), "application/octet-stream");
    }
}
