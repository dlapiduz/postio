//! Translates folders to mailboxes, filenames to identities and info letters to flags. No CONDSTORE
//! or QRESYNC: `highest_mod_seq` and every `mod_seq` are `None`, so the sync engine compares
//! listings, which for a local tree is a directory read. `existing_uids` is implemented to spare it
//! a UID range full of holes.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use postio_model::{Flag, FlagSet, Generation, ModSeq, RemoteId, Uid, UidValidity};

use super::store::{Entry, Folder, LocalStore, Snapshot};
use crate::backend::{
    AppendMessage, BackendError, BackendResult, BodyPart, BodySink, Capabilities, Capability,
    FetchedBody, FetchedMessage, FlagChange, FlagUpdate, MailBackend, MailboxEvent, MailboxFilter,
    MailboxStatus, MailboxSummary, SelectMode, UidMapping, UidSet, identity,
};
use crate::cancel::CancelToken;

#[derive(Debug)]
pub struct MaildirBackend {
    store: LocalStore,
    /// One per backend, drawn once at construction: folders numbered in the same session share a
    /// generation; a folder with a list keeps its own.
    fresh_validity: u32,
    /// A command before `connect` is `NotConnected`, which callers rely on for every backend.
    connected: Mutex<bool>,
}

impl MaildirBackend {
    /// Fails when `root` is not a maildir, so the Add Account sheet says so rather than showing an
    /// account with no mail.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, String> {
        let root = root.into();
        LocalStore::looks_like_a_maildir(&root)?;
        Ok(Self::at(root))
    }

    /// What a stored account gets at startup: the tree is not checked, so a moved directory is not
    /// a hard error at launch; `connect` reports it.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self {
            store: LocalStore::at(root),
            fresh_validity: fresh_validity(),
            connected: Mutex::new(false),
        }
    }

    pub fn root(&self) -> &std::path::Path {
        self.store.root()
    }

    /// `UIDPLUS` and `MOVE` are true of a filesystem, and claiming them spares the engine the
    /// searches that stand in for them.
    fn advertised() -> Capabilities {
        Capabilities::from_names([
            Capability::UidPlus.as_str(),
            Capability::Move.as_str(),
            Capability::Unselect.as_str(),
        ])
    }

    fn require_session(&self, context: &str) -> BackendResult<()> {
        if *self
            .connected
            .lock()
            .expect("the connect flag is never poisoned")
        {
            Ok(())
        } else {
            Err(BackendError::NotConnected {
                context: context.to_owned(),
            })
        }
    }

    fn look(&self, path: &str) -> BackendResult<(Folder, Snapshot)> {
        let folder = self.store.folder(path)?;
        let snapshot = self.store.snapshot(&folder, self.fresh_validity)?;
        Ok((folder, snapshot))
    }

    /// A stale id is `NoSuchMessage`, not a wrong message: acting on whatever now holds the number
    /// would flag or delete mail nobody asked about.
    fn resolve<'a>(
        &self,
        snapshot: &'a Snapshot,
        mailbox: &str,
        id: &RemoteId,
    ) -> BackendResult<&'a Entry> {
        let (validity, uid) = identity::wire(id).ok_or_else(|| BackendError::NoSuchMessage {
            mailbox: mailbox.to_owned(),
            uid: 0,
        })?;
        if validity.get() != snapshot.validity {
            return Err(BackendError::UidValidityChanged {
                mailbox: mailbox.to_owned(),
                known: validity,
                observed: UidValidity::new(snapshot.validity),
            });
        }
        snapshot
            .entries
            .iter()
            .find(|entry| entry.uid == uid.get())
            .ok_or_else(|| BackendError::NoSuchMessage {
                mailbox: mailbox.to_owned(),
                uid: uid.get(),
            })
    }

    fn status_of(&self, path: &str, read_only: bool) -> BackendResult<MailboxStatus> {
        let (_folder, snapshot) = self.look(path)?;
        Ok(MailboxStatus {
            path: path.to_owned(),
            generation: Generation::new(snapshot.validity),
            uid_next: Uid::new(snapshot.next),
            exists: snapshot.entries.len() as u32,
            unseen: Some(
                snapshot
                    .entries
                    .iter()
                    .filter(|entry| !entry.flags.contains(&Flag::Seen))
                    .count() as u32,
            ),
            highest_mod_seq: None,
            permanent_flags: FlagSet::from_iter([
                Flag::Seen,
                Flag::Answered,
                Flag::Flagged,
                Flag::Deleted,
                Flag::Draft,
            ]),
            // A maildir keyword needs a `dovecot-keywords` sidecar and a free
            // slot in it; Postio does not write one.
            can_create_keywords: false,
            read_only,
        })
    }

    /// One message at a time, read back after each: a maildir mints a new unique name on arrival,
    /// so the landing place cannot be predicted, and claiming `UIDPLUS` means answering exactly.
    fn transfer(
        &self,
        from: &str,
        ids: &[RemoteId],
        to: &str,
        remove_source: bool,
    ) -> BackendResult<Vec<UidMapping>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let (source, snapshot) = self.look(from)?;
        let target = self.store.folder(to)?;
        let mut known: Vec<String> = self
            .store
            .snapshot(&target, self.fresh_validity)?
            .entries
            .iter()
            .filter_map(entry_id)
            .collect();

        let mut landed = Vec::new();
        for id in ids {
            let entry = self.resolve(&snapshot, from, id)?;
            let name = entry_id(entry).ok_or_else(|| BackendError::NoSuchMessage {
                mailbox: from.to_owned(),
                uid: entry.uid,
            })?;
            if remove_source {
                self.store.move_to(&source, &name, &target)?;
            } else {
                self.store.copy_to(&source, &name, &target)?;
            }

            let after = self.store.snapshot(&target, self.fresh_validity)?;
            let arrival = after
                .entries
                .iter()
                .find(|candidate| entry_id(candidate).is_some_and(|name| !known.contains(&name)));
            let Some(arrival) = arrival else {
                // The move did not fail, but the new name could not be picked out: saying nothing
                // is the seam's "cannot tell where it went"; inventing a mapping would not be.
                continue;
            };
            if let Some(name) = entry_id(arrival) {
                known.push(name);
            }
            let validity = UidValidity::new(after.validity);
            landed.push(UidMapping {
                source: Uid::new(entry.uid),
                destination: Uid::new(arrival.uid),
                uid_validity: validity,
                destination_remote_id: identity::remote_id(validity, Uid::new(arrival.uid)),
            });
        }
        Ok(landed)
    }
}

#[async_trait]
impl MailBackend for MaildirBackend {
    fn describe(&self) -> &'static str {
        "maildir"
    }

    async fn connect(&self) -> BackendResult<Capabilities> {
        // A check the tree is still there: an unplugged disk must fail here as "could not connect",
        // not read as an empty account.
        LocalStore::looks_like_a_maildir(self.store.root()).map_err(|reason| BackendError::Io {
            context: format!("opening the maildir at {}", self.store.root().display()),
            reason,
        })?;
        *self
            .connected
            .lock()
            .expect("the connect flag is never poisoned") = true;
        Ok(Self::advertised())
    }

    async fn disconnect(&self) -> BackendResult<()> {
        *self
            .connected
            .lock()
            .expect("the connect flag is never poisoned") = false;
        Ok(())
    }

    async fn capabilities(&self) -> BackendResult<Capabilities> {
        self.require_session("CAPABILITY")?;
        Ok(Self::advertised())
    }

    async fn list_mailboxes(&self, filter: &MailboxFilter) -> BackendResult<Vec<MailboxSummary>> {
        self.require_session("LIST")?;
        let folders = self.store.folders()?;
        let paths: Vec<String> = folders.iter().map(|folder| folder.path.clone()).collect();

        Ok(folders
            .iter()
            .filter(|folder| matches_pattern(&filter.pattern, &folder.path))
            .map(|folder| {
                let prefix = format!("{}/", folder.path);
                let has_children = paths.iter().any(|path| path.starts_with(&prefix));
                let mut attributes = vec![if has_children {
                    "\\HasChildren".to_owned()
                } else {
                    "\\HasNoChildren".to_owned()
                }];
                if folder.path == "INBOX" {
                    attributes.push("\\Inbox".to_owned());
                }
                MailboxSummary::new(&folder.path, Some('/'), attributes)
            })
            .collect())
    }

    async fn create_mailbox(&self, path: &str) -> BackendResult<()> {
        self.require_session("CREATE")?;
        // Creating a folder that exists is the same call, so IMAP's "ask whether it exists after a
        // refusal" has no counterpart.
        self.store.create(path)?;
        Ok(())
    }

    async fn select(&self, path: &str, mode: SelectMode) -> BackendResult<MailboxStatus> {
        self.require_session("SELECT")?;
        self.status_of(path, matches!(mode, SelectMode::ReadOnly))
    }

    async fn status(&self, path: &str) -> BackendResult<MailboxStatus> {
        self.require_session("STATUS")?;
        self.status_of(path, false)
    }

    async fn fetch_headers(
        &self,
        mailbox: &str,
        uids: &UidSet,
        _changed_since: Option<ModSeq>,
        cancel: &CancelToken,
    ) -> BackendResult<Vec<FetchedMessage>> {
        // `changed_since` is ignored, not approximated: with no modification sequence, returning
        // fewer messages would silently drop changes, and returning all is always correct.
        self.require_session("FETCH")?;
        let (_folder, snapshot) = self.look(mailbox)?;
        let validity = UidValidity::new(snapshot.validity);

        let mut fetched = Vec::new();
        for entry in snapshot
            .entries
            .iter()
            .filter(|entry| uids.contains(Uid::new(entry.uid)))
        {
            if cancel.is_cancelled() {
                return Err(BackendError::Cancelled);
            }
            let raw = self.store.read(&entry.file)?;
            let parsed = postio_model::mime::parse_headers(&raw);
            fetched.push(FetchedMessage {
                content_identity: None,
                remote_id: identity::remote_id(validity, Uid::new(entry.uid)),
                uid: Uid::new(entry.uid),
                uid_validity: validity,
                mod_seq: None,
                flags: entry.flags.iter().cloned().collect(),
                internal_date: delivered_at(&entry.file, &parsed),
                size: raw.len() as u64,
                envelope: Some(envelope_of(&parsed)),
                structure: None,
                // The headers are already parsed, so the filing pass's fields cost nothing (spec
                // 007, R8).
                promoted: Some(postio_model::promoted::PromotedHeaders::from_headers(
                    &parsed.headers,
                )),
            });
        }
        Ok(fetched)
    }

    async fn fetch_part(
        &self,
        mailbox: &str,
        id: &RemoteId,
        part: &BodyPart,
        sink: &mut dyn BodySink,
        cancel: &CancelToken,
    ) -> BackendResult<FetchedBody> {
        self.require_session("FETCH BODY")?;
        let (_folder, snapshot) = self.look(mailbox)?;
        let raw = {
            let entry = self.resolve(&snapshot, mailbox, id)?;
            self.store.read(&entry.file)?
        };
        if cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }

        let bytes = match part {
            BodyPart::Whole => raw.as_slice(),
            BodyPart::Headers => header_block(&raw),
            BodyPart::Text => body_block(&raw),
            // No BODYSTRUCTURE is reported, so a section cannot be addressed; the caller takes the
            // whole message and parses locally.
            BodyPart::Section(_) => {
                return Err(BackendError::Unsupported {
                    capability: Capability::Binary,
                });
            }
        };

        sink.chunk(bytes).await?;
        let bytes_written = bytes.len() as u64;
        sink.finish().await?;
        Ok(FetchedBody {
            remote_id: id.clone(),
            part: part.clone(),
            bytes_written,
        })
    }

    async fn store_flags(
        &self,
        mailbox: &str,
        ids: &[RemoteId],
        change: &FlagChange,
    ) -> BackendResult<Vec<FlagUpdate>> {
        self.require_session("STORE")?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let (folder, snapshot) = self.look(mailbox)?;

        let mut updates = Vec::new();
        for id in ids {
            let entry = self.resolve(&snapshot, mailbox, id)?;
            let name = entry_id(entry).ok_or_else(|| BackendError::NoSuchMessage {
                mailbox: mailbox.to_owned(),
                uid: entry.uid,
            })?;
            let now: FlagSet = change.apply(&entry.flags.iter().cloned().collect());
            self.store
                .set_flags(&folder, &name, &now.iter().cloned().collect::<Vec<_>>())?;
            updates.push(FlagUpdate {
                remote_id: id.clone(),
                flags: self.store.flags(&folder, &name)?.into_iter().collect(),
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
        self.require_session("MOVE")?;
        self.transfer(from, ids, to, true)
    }

    async fn copy_messages(
        &self,
        from: &str,
        ids: &[RemoteId],
        to: &str,
    ) -> BackendResult<Vec<UidMapping>> {
        self.require_session("COPY")?;
        self.transfer(from, ids, to, false)
    }

    async fn expunge(
        &self,
        mailbox: &str,
        ids: Option<&[RemoteId]>,
    ) -> BackendResult<Vec<RemoteId>> {
        self.require_session("EXPUNGE")?;
        let (folder, snapshot) = self.look(mailbox)?;

        // Untargeted would expunge every `\Deleted` message, another client's included; a maildir
        // can always be targeted, so decline instead of widening.
        let Some(ids) = ids else {
            return Err(BackendError::Rejected {
                command: "EXPUNGE".to_owned(),
                reason: "a maildir is only ever expunged by id, so that mail another \
                         client marked deleted is left alone"
                    .to_owned(),
            });
        };

        let mut gone = Vec::new();
        for id in ids {
            let entry = self.resolve(&snapshot, mailbox, id)?;
            if !entry.flags.contains(&Flag::Deleted) {
                continue;
            }
            let name = entry_id(entry).ok_or_else(|| BackendError::NoSuchMessage {
                mailbox: mailbox.to_owned(),
                uid: entry.uid,
            })?;
            self.store.delete(&folder, &name)?;
            gone.push(id.clone());
        }
        Ok(gone)
    }

    async fn append(
        &self,
        mailbox: &str,
        message: &AppendMessage,
    ) -> BackendResult<Option<UidMapping>> {
        self.require_session("APPEND")?;
        let folder = self.store.folder(mailbox)?;
        let flags: Vec<Flag> = message.flags.iter().cloned().collect();
        let name = self.store.store(&folder, message.raw.clone(), &flags)?;

        let landed = self.store.snapshot(&folder, self.fresh_validity)?;
        let Some(entry) = landed
            .entries
            .iter()
            .find(|entry| entry_id(entry).as_deref() == Some(name.as_str()))
        else {
            return Ok(None);
        };
        let validity = UidValidity::new(landed.validity);
        Ok(Some(UidMapping {
            source: Uid::new(0),
            destination: Uid::new(entry.uid),
            uid_validity: validity,
            destination_remote_id: identity::remote_id(validity, Uid::new(entry.uid)),
        }))
    }

    async fn find_by_message_id(
        &self,
        mailbox: &str,
        message_id: &str,
    ) -> BackendResult<Option<RemoteId>> {
        self.require_session("SEARCH")?;
        let (_folder, snapshot) = self.look(mailbox)?;
        let wanted = message_id.trim().trim_matches(['<', '>']);
        let validity = UidValidity::new(snapshot.validity);

        for entry in snapshot.entries.iter().rev() {
            let raw = self.store.read(&entry.file)?;
            let parsed = postio_model::mime::parse_headers(&raw);
            let found = parsed
                .rfc_message_id
                .as_ref()
                .is_some_and(|id| id.as_str().trim_matches(['<', '>']) == wanted);
            if found {
                return Ok(Some(identity::remote_id(validity, Uid::new(entry.uid))));
            }
        }
        Ok(None)
    }

    async fn existing_uids(
        &self,
        mailbox: &str,
        cancel: &CancelToken,
    ) -> BackendResult<Option<Vec<Uid>>> {
        self.require_session("SEARCH ALL")?;
        if cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }
        let (_folder, snapshot) = self.look(mailbox)?;
        Ok(Some(
            snapshot
                .entries
                .iter()
                .map(|entry| Uid::new(entry.uid))
                .collect(),
        ))
    }

    async fn idle(
        &self,
        _mailbox: &str,
        timeout: Duration,
        cancel: &CancelToken,
    ) -> BackendResult<Vec<MailboxEvent>> {
        // No push: a watcher returning immediately would turn the engine's wait into a spin, so
        // wait out the timeout; the engine then polls `status`, a directory read.
        self.require_session("IDLE")?;
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

fn entry_id(entry: &Entry) -> Option<String> {
    let name = entry.file.file_name()?.to_str()?;
    Some(match name.rsplit_once(":2,") {
        Some((id, _flags)) => id.to_owned(),
        None => name.to_owned(),
    })
}

/// File mtime is the `INTERNALDATE`, then the `Date` header, then the epoch, never `now`, which
/// would make an old archive look like it arrived today.
fn delivered_at(
    file: &std::path::Path,
    parsed: &postio_model::mime::ParsedMessage,
) -> chrono::DateTime<chrono::Utc> {
    std::fs::metadata(file)
        .and_then(|metadata| metadata.modified())
        .map(chrono::DateTime::<chrono::Utc>::from)
        .ok()
        .or(parsed.date)
        .unwrap_or_default()
}

fn envelope_of(parsed: &postio_model::mime::ParsedMessage) -> crate::backend::Envelope {
    crate::backend::Envelope {
        date: parsed.date,
        subject: parsed.subject.clone(),
        from: parsed.from.clone(),
        sender: parsed.sender.clone(),
        reply_to: parsed.reply_to.clone(),
        to: parsed.to.clone(),
        cc: parsed.cc.clone(),
        bcc: parsed.bcc.clone(),
        in_reply_to: parsed.in_reply_to.clone(),
        message_id: parsed.rfc_message_id.clone(),
        references: parsed.references.clone(),
        list_id: parsed.list_id.clone(),
    }
}

fn split_at_blank_line(raw: &[u8]) -> usize {
    raw.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|at| at + 4)
        .or_else(|| {
            raw.windows(2)
                .position(|window| window == b"\n\n")
                .map(|at| at + 2)
        })
        .unwrap_or(raw.len())
}

fn header_block(raw: &[u8]) -> &[u8] {
    &raw[..split_at_blank_line(raw)]
}

fn body_block(raw: &[u8]) -> &[u8] {
    &raw[split_at_blank_line(raw)..]
}

fn matches_pattern(pattern: &str, path: &str) -> bool {
    fn walk(pattern: &[u8], path: &[u8]) -> bool {
        match pattern.first() {
            None => path.is_empty(),
            Some(b'*') => (0..=path.len()).any(|at| walk(&pattern[1..], &path[at..])),
            Some(b'%') => (0..=path.len())
                .take_while(|at| !path[..*at].contains(&b'/'))
                .any(|at| walk(&pattern[1..], &path[at..])),
            Some(byte) => path.first() == Some(byte) && walk(&pattern[1..], &path[1..]),
        }
    }
    pattern.is_empty() || walk(pattern.as_bytes(), path.as_bytes())
}

/// The clock: it only has to exceed the last one this tree saw; a folder with a list keeps its own.
fn fresh_validity() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as u32)
        .unwrap_or(1)
        .max(1)
}
