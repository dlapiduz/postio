//! The store half of writing mail: saving, discarding and queueing drafts, and what the composer
//! reads.
//! Every function logs and answers `None` or an error rather than panicking.

use chrono::{DateTime, Utc};
use postio_client::protocol::Queued;
use postio_model::contact_group::RecipientCandidate;
use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_model::listing::StoreError;
use postio_model::{
    Account, Attachment, Draft, DraftId, EmailAddress, Message, OperationTarget, SignatureId,
    signature_default,
};
use postio_storage::repository::{
    AccountRepository, CancelSendOutcome, ContactGroupRepository, ContactRepository,
    CorrespondentRepository, DraftRepository, MailboxRepository, MessageRepository,
    OperationQueueRepository,
};
use postio_storage::{BlobStore, Store};

pub const SUGGESTION_LIMIT: u32 = 8;

/// From the clock seam, so a storyboard that stops the clock dates the Outbox by its own day.
fn now() -> DateTime<Utc> {
    postio_model::clock::now().with_timezone(&Utc)
}

/// Autosave: the draft and its Drafts-folder upload are queued in one transaction.
/// Uses `interactive_write` so the typing person's save goes ahead of a backfill's bulk writes
/// (#425).
pub async fn save_draft(database: &Store, draft: &mut Draft) -> postio_storage::Result<()> {
    let (connection, _permit) = database.interactive_write().await?;
    DraftRepository::new(&connection)
        .save_and_sync(draft, now())
        .await?;
    Ok(())
}

pub async fn delete_draft(database: &Store, id: DraftId) -> postio_storage::Result<()> {
    let (connection, _permit) = database.interactive_write().await?;
    DraftRepository::new(&connection).discard(id, now()).await?;
    Ok(())
}

/// With `at`, the drainer leaves the draft alone until then.
pub async fn queue_send(
    database: &Store,
    draft: &mut Draft,
    at: Option<DateTime<Utc>>,
) -> postio_storage::Result<()> {
    let (connection, _permit) = database.interactive_write().await?;
    expand_groups(&connection, draft).await?;
    let drafts = DraftRepository::new(&connection);
    match at {
        Some(at) => {
            drafts.queue_send_at(draft, now(), at).await?;
        }
        None => {
            drafts.queue_send(draft, now()).await?;
        }
    }
    Ok(())
}

/// A group named in To, Cc or Bcc becomes its members' primary addresses. An unknown group is left
/// for the send to refuse.
async fn expand_groups(
    connection: &postio_storage::Connection,
    draft: &mut Draft,
) -> postio_storage::Result<()> {
    use postio_storage::repository::ContactGroupRepository;
    let names_a_group = |entry: &postio_model::EmailAddress| {
        !entry.address.contains('@') && !entry.address.trim().is_empty()
    };
    if ![&draft.to, &draft.cc, &draft.bcc]
        .iter()
        .any(|list| list.iter().any(names_a_group))
    {
        return Ok(());
    }
    let groups = ContactGroupRepository::new(connection);
    let known = groups.list().await?;
    let mut already: std::collections::HashSet<String> = [&draft.to, &draft.cc, &draft.bcc]
        .iter()
        .flat_map(|list| list.iter())
        .filter(|entry| !names_a_group(entry))
        .map(postio_model::EmailAddress::normalized)
        .collect();
    for list in [&mut draft.to, &mut draft.cc, &mut draft.bcc] {
        let mut expanded = Vec::with_capacity(list.len());
        for entry in std::mem::take(list) {
            if !names_a_group(&entry) {
                expanded.push(entry);
                continue;
            }
            let typed = entry.address.trim();
            let name = typed
                .strip_prefix("to:")
                .or_else(|| typed.strip_prefix("TO:"))
                .unwrap_or(typed)
                .trim();
            match known
                .iter()
                .find(|group| group.name.eq_ignore_ascii_case(name))
            {
                Some(group) => {
                    for member in groups.expand(group.id).await? {
                        if already.insert(member.normalized()) {
                            expanded.push(member);
                        }
                    }
                }
                None => expanded.push(entry),
            }
        }
        *list = expanded;
    }
    Ok(())
}

enum DraftOp {
    Save {
        generation: u64,
        draft: Draft,
        reply: tokio::sync::oneshot::Sender<Result<DraftId, StoreError>>,
    },
    /// Answers the Drafts folder, whose list just changed, once the queue write landed.
    Send {
        generation: u64,
        draft: Draft,
        at: Option<DateTime<Utc>>,
        reply: tokio::sync::oneshot::Sender<Result<Queued, StoreError>>,
    },
    Discard {
        generation: u64,
        known: Option<DraftId>,
        reply: tokio::sync::oneshot::Sender<()>,
    },
}

/// Writes one composer's drafts in the order it made them, off the GTK thread (#1608).
/// A `generation` names a composition; its first save's id is remembered, so later saves update the
/// same row and a discard deletes it.
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

fn gone() -> StoreError {
    StoreError::new("the draft writer has stopped")
}

impl DraftWriter {
    pub fn spawn(database: Store, runtime: &tokio::runtime::Handle) -> Self {
        let (ops, requests) = async_channel::unbounded::<DraftOp>();
        runtime.spawn(async move {
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
                        // Taken on failure too: a failed queue write leaves the autosaved row
                        // recoverable, and the close after a send must not delete it.
                        if current.is_some_and(|(saved_for, _)| saved_for == generation) {
                            current = None;
                        }
                        let moved = match queue_send(&database, &mut draft, at).await {
                            Ok(()) if at.is_none() => Ok(Queued {
                                drafts: drafts_mailbox(&database, draft.account_id).await,
                                draft: draft.id,
                            }),
                            Ok(()) => Ok(Queued {
                                drafts: None,
                                draft: draft.id,
                            }),
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

    pub fn send(
        &self,
        generation: u64,
        draft: Draft,
        at: Option<DateTime<Utc>>,
    ) -> impl Future<Output = Result<Queued, StoreError>> + Send + 'static {
        let (reply, answer) = tokio::sync::oneshot::channel();
        self.hand_over(DraftOp::Send {
            generation,
            draft,
            at,
            reply,
        });
        async move { answer.await.unwrap_or_else(|_| Err(gone())) }
    }

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
        // Unbounded: the composer never waits on the writer.
        let _ = self.ops.try_send(op);
    }
}

/// The Outbox is a predicate over this folder, not a second one.
/// `None` before the first sync has found one.
pub async fn drafts_mailbox(database: &Store, account: AccountId) -> Option<MailboxId> {
    let connection = database.read().await.ok()?;
    MailboxRepository::new(&connection)
        .by_role(account, postio_model::MailboxRole::Drafts)
        .await
        .ok()
        .flatten()
        .map(|mailbox| mailbox.id)
}

pub async fn cancel_queued_send(database: &Store, id: DraftId) -> Option<Draft> {
    let connection = database
        .connect()
        .await
        .map_err(|error| tracing::warn!(%error, "could not open the store to cancel a send"))
        .ok()?;
    let drafts = DraftRepository::new(&connection);
    match drafts.cancel_send(id, now()).await {
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

/// Read from the queue row, where the drainer writes it, so no second copy can disagree (#1487).
pub async fn why_the_send_failed(database: &Store, id: DraftId) -> Option<String> {
    let connection = database
        .read()
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

pub async fn draft_behind(database: &Store, message: MessageId) -> Option<Draft> {
    let connection = database
        .read()
        .await
        .map_err(|error| tracing::warn!(%error, "could not open the store to resume a draft"))
        .ok()?;
    DraftRepository::new(&connection)
        .by_message(message)
        .await
        .map_err(|error| tracing::warn!(%error, "could not read the draft behind a row"))
        .ok()?
}

pub async fn draft_by_id(database: &Store, id: DraftId) -> Option<Draft> {
    let connection = database
        .read()
        .await
        .map_err(|error| tracing::warn!(%error, "could not open the store to resume a draft"))
        .ok()?;
    DraftRepository::new(&connection)
        .get(id)
        .await
        .map_err(|error| tracing::warn!(%error, "could not read a draft by id"))
        .ok()?
}

pub async fn drafts_of(database: &Store, account: AccountId) -> postio_storage::Result<Vec<Draft>> {
    let connection = database.read().await?;
    DraftRepository::new(&connection)
        .list_for_account(account)
        .await
}

/// Records that a session began; answers the draft `account` was writing if the last session died
/// uncleanly (#491). Only after a crash: `Editing` alone is not evidence (Esc parks drafts so), and
/// an untouched buffer is not recovered, since reopening it autosaves another empty row. Asked once
/// per process: `begin_session` marks this one open.
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

/// Blocking: run on a blocking thread.
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

pub async fn account(database: &Store, account: AccountId) -> Option<Account> {
    let connection = database.read().await.ok()?;
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

/// The selected mailbox's override, else the account's default, else none.
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
    // A closure cannot await.
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

/// Bounds a pathological store; not a page size.
const CORRESPONDENT_LIMIT: u32 = 50_000;

/// People are shared across accounts; `_account` stays only for the wire protocol's shape. Empty,
/// and logged, when the store cannot be read.
pub async fn correspondents(database: &Store, _account: AccountId) -> Vec<postio_model::Contact> {
    let found = async {
        let connection = database.read().await?;
        ContactRepository::new(&connection)
            .people(CORRESPONDENT_LIMIT)
            .await
    };
    found.await.unwrap_or_else(|error| {
        tracing::warn!(%error, "could not read the correspondents");
        Vec::new()
    })
}

/// Held in memory by the desktop composer so each keystroke is a lookup. Empty, and logged, when
/// the store cannot be read.
pub async fn recipient_directory(
    database: &Store,
    _account: AccountId,
) -> postio_client::protocol::RecipientDirectory {
    let found = async {
        let connection = database.read().await?;
        let groups = ContactGroupRepository::new(&connection);
        let mut named = Vec::new();
        for group in groups.list().await? {
            let members = groups.members(group.id).await?;
            let addresses: Vec<EmailAddress> = members
                .iter()
                .filter_map(postio_ui::recipients::preferred_address)
                .collect();
            if !addresses.is_empty() {
                named.push((group.name, addresses));
            }
        }
        let contacts = ContactRepository::new(&connection)
            .people(CORRESPONDENT_LIMIT)
            .await?;
        let written = CorrespondentRepository::new(&connection)
            .sent_counts()
            .await?;
        Ok::<_, postio_storage::Error>(postio_client::protocol::RecipientDirectory {
            groups: named,
            contacts: with_letters(contacts, &written),
        })
    };
    found.await.unwrap_or_else(|error| {
        tracing::warn!(%error, "could not read the recipient directory");
        Default::default()
    })
}

/// Empty, and logged, when the store cannot be read.
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

/// Nothing, logged, when the store cannot answer.
pub async fn label_counts(
    database: &Store,
    account: AccountId,
) -> Vec<(postio_model::LabelId, u32)> {
    let found = async {
        let connection = database.read().await?;
        postio_storage::repository::LabelRepository::new(&connection)
            .counts(account)
            .await
    };
    found.await.unwrap_or_else(|error| {
        tracing::warn!(%error, "could not count the labels");
        Vec::new()
    })
}

/// Case-insensitive get-or-create; reaches the server as a keyword on the first message it goes on.
pub async fn create_label(
    database: &Store,
    account: AccountId,
    name: &str,
) -> Option<postio_model::Label> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let made = async {
        let (connection, _permit) = database.interactive_write().await?;
        let labels = postio_storage::repository::LabelRepository::new(&connection);
        let existing = labels
            .list(account)
            .await?
            .into_iter()
            .find(|label| label.name.to_lowercase() == name.to_lowercase());
        if let Some(existing) = existing {
            return Ok::<_, postio_storage::Error>(existing);
        }
        let mut label = postio_model::Label::new(account, name);
        labels.create(&mut label).await?;
        Ok(label)
    };
    made.await
        .map_err(|error| tracing::warn!(%error, "could not make a label"))
        .ok()
}

/// One statement for the whole page. Nothing, logged, when the store cannot answer.
pub async fn thread_labels(
    database: &Store,
    threads: &[postio_model::ThreadId],
) -> Vec<(postio_model::ThreadId, postio_model::Label)> {
    let found = async {
        let connection = database.read().await?;
        postio_storage::repository::LabelRepository::new(&connection)
            .for_threads(threads)
            .await
    };
    found.await.unwrap_or_else(|error| {
        tracing::warn!(%error, "could not read a page's labels");
        Vec::new()
    })
}

/// Counted over Focus's own membership, not the per-folder trigger counts, which cannot see held
/// mail.
pub async fn focus_counts(
    database: &Store,
) -> Result<postio_client::protocol::FocusCounts, postio_model::listing::StoreError> {
    let counted = async {
        let connection = database.read().await?;
        let threads = postio_storage::repository::ThreadRepository::new(&connection);
        let inboxes = threads.unified_inboxes().await?;
        // Local midnight, like `filter_decisions.decided_at`.
        let midnight = chrono::Local::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|midnight| {
                chrono::TimeZone::from_local_datetime(&chrono::Local, &midnight).earliest()
            })
            .map_or_else(chrono::Utc::now, |midnight| {
                midnight.with_timezone(&chrono::Utc)
            });
        Ok::<_, postio_storage::Error>(postio_client::protocol::FocusCounts {
            conversations: threads.focus_count(&inboxes).await?,
            unread: threads.focus_unread(&inboxes).await?,
            has_action: threads.focus_marked(&inboxes).await?.len(),
            filtered_today: postio_storage::repository::FilterDecisionRepository::new(&connection)
                .count_since(midnight)
                .await?,
        })
    };
    counted.await.map_err(|error| {
        tracing::warn!(%error, "could not count Focus's inbox");
        postio_model::listing::StoreError::new(error.to_string())
    })
}

/// Ranked by `postio_ui::recipients::suggest`, the rule the desktop applies to its directory in
/// memory.
pub async fn recipients(
    database: &Store,
    _account: AccountId,
    prefix: &str,
) -> Vec<RecipientCandidate> {
    let found = async {
        let connection = database.read().await?;
        let groups = ContactGroupRepository::new(&connection);
        let mut named = Vec::new();
        for group in groups.list().await? {
            let members = groups.members(group.id).await?;
            let addresses: Vec<EmailAddress> = members
                .iter()
                .filter_map(postio_ui::recipients::preferred_address)
                .collect();
            if !addresses.is_empty() {
                named.push((group.name, addresses));
            }
        }
        // Wide enough that the store's own order cannot push out an address the user writes to
        // before the rule ranks it.
        let contacts = ContactRepository::new(&connection)
            .complete(prefix, PREFIX_POOL)
            .await?;
        let written = CorrespondentRepository::new(&connection)
            .sent_counts()
            .await?;
        Ok::<_, postio_storage::Error>(postio_ui::recipients::suggest(
            &named,
            &with_letters(contacts, &written),
            prefix,
            SUGGESTION_LIMIT as usize,
        ))
    };
    found.await.unwrap_or_else(|error| {
        tracing::warn!(%error, "could not search contacts");
        Vec::new()
    })
}

const PREFIX_POOL: u32 = 500;

fn with_letters(
    contacts: Vec<postio_model::Contact>,
    written: &std::collections::HashMap<String, u32>,
) -> Vec<postio_ui::recipients::Correspondent> {
    contacts
        .into_iter()
        .map(|contact| postio_ui::recipients::Correspondent {
            sent_count: contact
                .addresses
                .iter()
                .map(|owned| {
                    written
                        .get(&owned.address.normalized())
                        .copied()
                        .unwrap_or_default()
                })
                .sum(),
            contact,
        })
        .collect()
}

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

/// Blocking: run on a blocking thread. The caller supplies the type because sniffing differs by
/// frontend;
/// [`guess_mime_type`] is the fallback without `gio`.
pub fn attach_file(
    blobs: &BlobStore,
    path: &std::path::Path,
    mime_type: String,
) -> Option<Attachment> {
    let size = std::fs::metadata(path).ok()?.len();
    let file = std::fs::File::open(path)
        // The path is not logged: an attachment's name is the user's.
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

/// The rule is `postio_session::attaching::inline_image`'s, so every frontend agrees on the
/// `Content-ID` and refuses non-images and oversized ones.
pub fn inline_attachment(blobs: &BlobStore, bytes: Vec<u8>, mime_type: &str) -> Option<Attachment> {
    postio_session::attaching::inline_image(blobs, &bytes, mime_type)
        .map_err(|error| tracing::warn!(%error, "could not store the pasted image"))
        .ok()
}

/// For a process without shared-mime-info. Unknown types are the generic "some bytes": a type that
/// claims too much is the only harmful answer.
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

/// Why Download and attach left an image a link, as every frontend's row
/// says it.
pub fn remote_refusal(
    refusal: postio_session::attaching::FetchRefusal,
) -> postio_ui::compose::RemoteRefusal {
    use postio_session::attaching::FetchRefusal;
    use postio_ui::compose::RemoteRefusal;
    match refusal {
        FetchRefusal::Unreachable { reason } => RemoteRefusal::Unreachable(reason),
        FetchRefusal::TooLarge => RemoteRefusal::TooLarge,
        FetchRefusal::NotAnImage => RemoteRefusal::NotAnImage,
        FetchRefusal::Offline => RemoteRefusal::Offline,
    }
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
