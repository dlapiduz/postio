//! A maildir behind [`MailBackend`].
//!
//! Everything above the seam — threading, the windowed list, incremental
//! passes, the operation queue's replay — is written against this trait, so a
//! maildir that implements it needs no code of its own anywhere else. What
//! this module does is translate: folders on disk to mailboxes, filenames to
//! identities, info letters to flags, and the seam's IMAP-shaped questions
//! into filesystem answers.
//!
//! # What it says it cannot do
//!
//! There is no CONDSTORE and no QRESYNC here, and no pretending otherwise: a
//! maildir has no modification sequence, so [`MailboxStatus::highest_mod_seq`]
//! is `None`, every [`FetchedMessage::mod_seq`] is `None`, and the sync engine
//! falls back to comparing listings — which for a local tree is a directory
//! read, not a round trip. [`MailBackend::existing_uids`] *is* implemented,
//! because a listing is the cheapest thing a maildir does and it saves the
//! engine walking a UID range full of holes.

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

/// A [`MailBackend`] over a maildir on this machine.
#[derive(Debug)]
pub struct MaildirBackend {
    store: LocalStore,
    /// The UIDVALIDITY handed to a folder that has never been numbered.
    ///
    /// One per backend rather than one per folder, and drawn once at
    /// construction: two folders numbered in the same session belong to the
    /// same generation, and a folder that already has a list keeps the
    /// generation written in it.
    fresh_validity: u32,
    /// Whether [`MailBackend::connect`] has been called.
    ///
    /// A maildir has no session, but the seam's contract does: a command
    /// before a connect is [`BackendError::NotConnected`], and callers rely
    /// on that being true of every backend.
    connected: Mutex<bool>,
}

impl MaildirBackend {
    /// A backend over the maildir at `root`.
    ///
    /// Fails when `root` is not a maildir, with the message a person reads in
    /// the Add Account sheet — checking here rather than at the first sync is
    /// the difference between "that is not a maildir" and an account that
    /// appears to have no mail in it.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, String> {
        let root = root.into();
        LocalStore::looks_like_a_maildir(&root)?;
        Ok(Self::at(root))
    }

    /// A backend over `root`, whatever is there.
    ///
    /// What a stored account gets at startup. The tree is not checked here on
    /// purpose: a launch must not be the moment a moved directory becomes a
    /// hard error, and [`MailBackend::connect`] is where the seam already
    /// says "this account cannot be reached" — with the directory named.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self {
            store: LocalStore::at(root),
            fresh_validity: fresh_validity(),
            connected: Mutex::new(false),
        }
    }

    /// The tree this reads.
    pub fn root(&self) -> &std::path::Path {
        self.store.root()
    }

    /// What a maildir can do, in the seam's vocabulary.
    ///
    /// `UIDPLUS` and `MOVE` are simply true of a filesystem — a stored message
    /// is at a path this backend just chose, and a move is one rename — and
    /// claiming them is what stops the engine paying for the searches that
    /// stand in for them. Nothing else is claimed.
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

    /// The folder at `path`, and everything in it.
    fn look(&self, path: &str) -> BackendResult<(Folder, Snapshot)> {
        let folder = self.store.folder(path)?;
        let snapshot = self.store.snapshot(&folder, self.fresh_validity)?;
        Ok((folder, snapshot))
    }

    /// The entry a [`RemoteId`] names, refusing one from another generation.
    ///
    /// A stale id is [`BackendError::NoSuchMessage`] rather than a wrong
    /// message: the numbers were reassigned, and acting on whatever now holds
    /// that number would flag or delete mail nobody asked about.
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
            // No modification sequence exists on disk to report.
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

    /// Moves or copies `ids` from one folder to another.
    ///
    /// One message at a time, and the destination is read back after each:
    /// a maildir mints a *new* unique name when a message arrives in a
    /// folder — that is what makes the name unique, and what a delivery to
    /// that folder would have done — so the landing place cannot be
    /// predicted from the source name. Claiming `UIDPLUS` means answering
    /// this exactly, and looking is the only way to be exact.
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
                // The message is in the target — the move did not fail — but
                // its new name could not be picked out. Saying nothing is the
                // seam's "this backend cannot tell you where it went", which
                // the caller already handles; inventing a mapping would not
                // be.
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
        // Not a handshake — a check that the tree is still there. A maildir
        // on a disk that has been unplugged must fail here, where the caller
        // shows "could not connect", rather than reading as an empty account.
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
                // A maildir does not name its own special folders, so the
                // role is resolved from the path — the same rule that keeps
                // provider knowledge out of the code.
                if folder.path == "INBOX" {
                    attributes.push("\\Inbox".to_owned());
                }
                MailboxSummary::new(&folder.path, Some('/'), attributes)
            })
            .collect())
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
        // `changed_since` is deliberately ignored rather than honoured
        // approximately: with no modification sequence on disk there is no
        // answer to "changed since", and returning fewer messages than were
        // asked for would silently drop changes. Reporting all of them costs
        // a reparse and is always correct. The engine does not ask, because
        // `highest_mod_seq` is `None`.
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
                remote_id: identity::remote_id(validity, Uid::new(entry.uid)),
                uid: Uid::new(entry.uid),
                uid_validity: validity,
                mod_seq: None,
                flags: entry.flags.iter().cloned().collect(),
                internal_date: delivered_at(&entry.file, &parsed),
                size: raw.len() as u64,
                envelope: Some(envelope_of(&parsed)),
                structure: None,
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
            // A section specifier addresses the MIME tree, and this backend
            // reports no BODYSTRUCTURE for anything to address. The whole
            // message is right here, so the caller loses nothing by taking
            // it and parsing locally — which is what it does when a backend
            // says it cannot decode server-side.
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
            // Read back rather than report what was asked for: the filename
            // is the record, and this is what the next listing will see.
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

        // Untargeted means every message marked `\Deleted`, which on a shared
        // tree includes ones another client marked. That is exactly what the
        // seam's UID EXPUNGE rule exists to prevent, and a maildir can always
        // be targeted, so decline instead of widening.
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
        // Unlike an IMAP server, this one knows exactly which messages went:
        // it removed them by name.
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

        // Newest first, because any copy proves arrival and the trait asks
        // for the newest when there is more than one.
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
        // No push: a filesystem can be watched, but not through this call's
        // shape, and a watcher that returned immediately with nothing would
        // turn the engine's wait into a spin. Waiting out the timeout is what
        // "nothing happened" looks like, and the engine polls `status`
        // afterwards — which for a local tree is a directory read.
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

/// The name every maildir operation keys on: the filename without its flags.
fn entry_id(entry: &Entry) -> Option<String> {
    let name = entry.file.file_name()?.to_str()?;
    Some(match name.rsplit_once(":2,") {
        Some((id, _flags)) => id.to_owned(),
        None => name.to_owned(),
    })
}

/// When the message was delivered here.
///
/// The file's modification time is the maildir's `INTERNALDATE`: it is when
/// this machine received the message, which is what the list sorts on, and it
/// survives the `Date` header being absent or a lie. The header is the
/// fallback, and the epoch the last resort — never `now`, which would make
/// every message in an old archive look like it arrived today.
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

/// The addressing headers, in the shape the seam expects from a server.
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

/// Where the header block ends: the first blank line, either line ending.
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

/// An IMAP `LIST` pattern, applied to one path.
///
/// `*` crosses the hierarchy delimiter and `%` does not — the distinction the
/// engine relies on to list one level at a time.
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

/// A UIDVALIDITY for a folder that has never been numbered.
///
/// The clock, as every maildir-aware client uses: it only has to be larger
/// than the last one this tree saw, and a folder that already has a list
/// keeps what is written in it rather than taking this.
fn fresh_validity() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as u32)
        .unwrap_or(1)
        .max(1)
}
