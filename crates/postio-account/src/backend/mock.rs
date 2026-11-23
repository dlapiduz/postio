//! In-memory `MailBackend` that `postio-sync` is developed against, with injectable faults and
//! latency, so no default-suite test touches the network.
//! It has no MIME parser: envelopes come from a few headers and a `BodyStructure` is given, or
//! parser tests would test themselves.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use postio_model::{
    ChangeFeed, EmailAddress, Flag, FlagSet, ModSeq, RemoteId, RfcMessageId, Uid, UidValidity,
};
use tokio::sync::Notify;

use super::identity;
use crate::cancel::CancelToken;

use super::{
    AppendMessage, BackendError, BackendResult, BodyPart, BodySink, BodyStructure, Capabilities,
    Capability, Envelope, FetchedBody, FetchedMessage, FlagChange, FlagUpdate, MailBackend,
    MailboxEvent, MailboxFilter, MailboxStatus, MailboxSummary, SelectMode, UidMapping, UidSet,
};

const DEFAULT_CHUNK: usize = 64 * 1024;

const DEFAULT_HOST: &str = "mock.invalid";

/// Scheduled by call index (`inject_after`) so a retry path lands on a known call.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Fault {
    /// The session closes, so the next call fails with `NotConnected` until something reconnects.
    Disconnect,
    Timeout,
    AuthFailed,
    RateLimited(Option<Duration>),
    Rejected(String),
    Io(String),
}

#[derive(Clone, Debug, Default)]
pub struct MockMessage {
    raw: Vec<u8>,
    flags: FlagSet,
    internal_date: Option<DateTime<Utc>>,
    structure: Option<BodyStructure>,
    parts: HashMap<String, Vec<u8>>,
}

impl MockMessage {
    pub fn new(raw: impl Into<Vec<u8>>) -> Self {
        Self {
            raw: raw.into(),
            ..Self::default()
        }
    }

    pub fn with_flags(mut self, flags: FlagSet) -> Self {
        self.flags = flags;
        self
    }

    /// Part bytes are stated, not parsed: the mock has no MIME parser, or parser tests would test
    /// themselves.
    /// `bytes` are what the wire carries: still encoded, no headers.
    pub fn with_part(mut self, section: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        self.parts.insert(section.into(), bytes.into());
        self
    }

    pub fn with_internal_date(mut self, internal_date: DateTime<Utc>) -> Self {
        self.internal_date = Some(internal_date);
        self
    }

    pub fn with_structure(mut self, structure: BodyStructure) -> Self {
        self.structure = Some(structure);
        self
    }
}

impl From<Vec<u8>> for MockMessage {
    fn from(raw: Vec<u8>) -> Self {
        Self::new(raw)
    }
}

impl From<&[u8]> for MockMessage {
    fn from(raw: &[u8]) -> Self {
        Self::new(raw.to_vec())
    }
}

impl From<&str> for MockMessage {
    fn from(raw: &str) -> Self {
        Self::new(raw.as_bytes().to_vec())
    }
}

#[derive(Clone, Debug)]
pub struct MockMailbox {
    path: String,
    delimiter: Option<char>,
    attributes: Vec<String>,
    subscribed: bool,
    uid_validity: UidValidity,
    highest_mod_seq: ModSeq,
    starting_uid: u32,
    messages: Vec<MockMessage>,
}

impl MockMailbox {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            delimiter: None,
            attributes: Vec::new(),
            subscribed: true,
            uid_validity: UidValidity::new(1),
            highest_mod_seq: ModSeq::new(1),
            starting_uid: 1,
            messages: Vec::new(),
        }
    }

    /// Models a folder in long use: `UIDNEXT` far above the message count, invisible when seeded
    /// from UID 1 (postio-qhz.9).
    pub fn starting_uid(mut self, uid: u32) -> Self {
        self.starting_uid = uid.max(1);
        self
    }

    pub fn delimiter(mut self, delimiter: char) -> Self {
        self.delimiter = Some(delimiter);
        self
    }

    /// Leaving them off covers iCloud, which advertises no `SPECIAL-USE`.
    pub fn attributes<I, S>(mut self, attributes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.attributes = attributes.into_iter().map(Into::into).collect();
        self
    }

    pub fn subscribed(mut self, subscribed: bool) -> Self {
        self.subscribed = subscribed;
        self
    }

    pub fn uid_validity(mut self, uid_validity: UidValidity) -> Self {
        self.uid_validity = uid_validity;
        self
    }

    pub fn highest_mod_seq(mut self, highest_mod_seq: ModSeq) -> Self {
        self.highest_mod_seq = highest_mod_seq;
        self
    }

    pub fn message(mut self, message: impl Into<MockMessage>) -> Self {
        self.messages.push(message.into());
        self
    }

    #[cfg(feature = "test-corpus")]
    pub fn corpus<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for name in names {
            let fixture = postio_model::test_corpus::load(name.as_ref());
            self.messages
                .push(MockMessage::new(fixture.bytes().to_vec()));
        }
        self
    }
}

#[derive(Clone, Debug)]
struct MessageState {
    uid: u32,
    raw: Vec<u8>,
    flags: FlagSet,
    internal_date: DateTime<Utc>,
    mod_seq: u64,
    envelope: Envelope,
    structure: Option<BodyStructure>,
    parts: HashMap<String, Vec<u8>>,
}

#[derive(Debug)]
struct MailboxState {
    summary: MailboxSummary,
    uid_validity: UidValidity,
    uid_next: u32,
    highest_mod_seq: u64,
    messages: Vec<MessageState>,
    pending: Vec<MailboxEvent>,
    stale_since: Option<UidValidity>,
}

impl MailboxState {
    fn seed(seed: MockMailbox) -> Self {
        let summary = MailboxSummary::new(seed.path, seed.delimiter, seed.attributes.clone());
        let mut state = Self {
            summary: MailboxSummary {
                subscribed: seed.subscribed,
                ..summary
            },
            uid_validity: seed.uid_validity,
            uid_next: seed.starting_uid,
            highest_mod_seq: seed.highest_mod_seq.get(),
            messages: Vec::new(),
            pending: Vec::new(),
            stale_since: None,
        };
        for message in seed.messages {
            state.push(message);
        }
        state
    }

    fn push(&mut self, message: MockMessage) -> u32 {
        let uid = self.uid_next;
        self.uid_next += 1;
        let envelope = sketch::envelope(&message.raw);
        let internal_date = message
            .internal_date
            .or(envelope.date)
            .unwrap_or_else(Utc::now);
        self.messages.push(MessageState {
            uid,
            raw: message.raw,
            flags: message.flags,
            internal_date,
            mod_seq: self.highest_mod_seq,
            envelope,
            structure: message.structure,
            parts: message.parts,
        });
        uid
    }

    /// Bumps the counter before stamping: RFC 7162 §3.1.2.1 needs a new MODSEQ strictly above the
    /// previous HIGHESTMODSEQ, or `CHANGEDSINCE` would never report the arrival.
    fn push_arriving(&mut self, message: MockMessage) -> u32 {
        self.bump_mod_seq();
        self.push(message)
    }

    fn bump_mod_seq(&mut self) -> u64 {
        self.highest_mod_seq += 1;
        self.highest_mod_seq
    }

    fn find(&self, uid: u32) -> Option<&MessageState> {
        self.messages.iter().find(|message| message.uid == uid)
    }

    fn guard_uid_validity(&self) -> BackendResult<()> {
        match self.stale_since {
            Some(known) => Err(BackendError::UidValidityChanged {
                mailbox: self.summary.path.clone(),
                known,
                observed: self.uid_validity,
            }),
            None => Ok(()),
        }
    }

    fn status(&self, condstore: bool, read_only: bool) -> MailboxStatus {
        MailboxStatus {
            path: self.summary.path.clone(),
            generation: postio_model::Generation::new(self.uid_validity.get()),
            uid_next: Uid::new(self.uid_next),
            exists: self.messages.len() as u32,
            unseen: Some(
                self.messages
                    .iter()
                    .filter(|message| message.flags.is_unread())
                    .count() as u32,
            ),
            highest_mod_seq: condstore.then(|| ModSeq::new(self.highest_mod_seq)),
            permanent_flags: FlagSet::from_iter([
                Flag::Seen,
                Flag::Answered,
                Flag::Flagged,
                Flag::Deleted,
                Flag::Draft,
            ]),
            can_create_keywords: true,
            read_only,
        }
    }
}

#[derive(Debug)]
struct State {
    host: String,
    connected: bool,
    capabilities: Capabilities,
    mailboxes: Vec<MailboxState>,
    faults: Vec<(u64, Fault)>,
    persistent_fault: Option<Fault>,
    body_fault: Option<Fault>,
    latency: Duration,
    calls: u64,
    refuse_creates: Option<String>,
    refuse_select: Vec<(String, String)>,
    created: Vec<String>,
    in_flight: usize,
    peak_in_flight: usize,
    chunk_size: usize,
    header_fetches: Vec<(u64, String)>,
    body_fetches: Vec<(u64, String)>,
    /// A sync pass lists UIDs once per start, so this tells "kept running" from "cancelled and
    /// restarted" (#1593).
    uid_listings: Vec<String>,
    header_fetch_rows: Vec<usize>,
    undecodable: Vec<u32>,
    flag_fetches: Vec<String>,
    change_feed: ChangeFeed,
    section_batches: Vec<(String, usize)>,
    refuse_uid_listing: bool,
    truncate_uid_listing: Option<usize>,
    omit_uid_mappings: bool,
    expunge_keeps_mod_seq: bool,
    holds: Vec<Hold>,
    parked: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Held {
    Headers,
    Bodies,
}

#[derive(Debug)]
struct Hold {
    kind: Held,
    mailbox: String,
    admit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchEvent {
    Header(String),
    Body(String),
}

impl State {
    fn index_of(&self, path: &str) -> BackendResult<usize> {
        self.mailboxes
            .iter()
            .position(|mailbox| mailbox.summary.path == path)
            .ok_or_else(|| BackendError::NoSuchMailbox {
                path: path.to_owned(),
            })
    }

    fn require_connected(&self, context: &str) -> BackendResult<()> {
        if self.connected {
            Ok(())
        } else {
            Err(BackendError::NotConnected {
                context: context.to_owned(),
            })
        }
    }

    fn condstore(&self) -> bool {
        self.capabilities.contains(Capability::CondStore)
    }

    fn uid_plus(&self) -> bool {
        self.capabilities.contains(Capability::UidPlus)
    }

    fn take_fault(&mut self, call: u64) -> Option<Fault> {
        if let Some(fault) = &self.persistent_fault {
            return Some(fault.clone());
        }
        let position = self.faults.iter().position(|(at, _)| *at == call)?;
        Some(self.faults.remove(position).1)
    }
}

/// Cloning shares one set of mailboxes and one fault schedule.
#[derive(Clone)]
pub struct MockBackend {
    state: Arc<Mutex<State>>,
    notify: Arc<Notify>,
    released: Arc<Notify>,
}

impl fmt::Debug for MockBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state();
        f.debug_struct("MockBackend")
            .field("host", &state.host)
            .field("connected", &state.connected)
            .field("mailboxes", &state.mailboxes.len())
            .field("calls", &state.calls)
            .finish()
    }
}

impl Default for MockBackend {
    fn default() -> Self {
        Self::new()
    }
}

struct InFlight(Arc<Mutex<State>>);

impl InFlight {
    fn enter(state: &Arc<Mutex<State>>) -> Self {
        {
            let mut state = state.lock().expect("mock backend mutex");
            state.in_flight += 1;
            state.peak_in_flight = state.peak_in_flight.max(state.in_flight);
        }
        Self(Arc::clone(state))
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.lock() {
            state.in_flight = state.in_flight.saturating_sub(1);
        }
    }
}

struct Parked(Arc<Mutex<State>>);

impl Drop for Parked {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.lock() {
            state.parked = state.parked.saturating_sub(1);
        }
    }
}

impl MockBackend {
    /// An iCloud-shaped capability set and no folders: a fixture must not supply what the wiring
    /// should produce (an invented `INBOX` hid that nothing ever listed folders, postio-755,
    /// postio-bl2).
    pub fn new() -> Self {
        Self::builder().build()
    }

    pub fn builder() -> MockBackendBuilder {
        MockBackendBuilder::new()
    }

    pub fn inject(&self, fault: Fault) {
        self.inject_after(0, fault);
    }

    /// `inject_after(2, ..)` lets two calls succeed and fails the third, testing a backoff loop
    /// without sleeping.
    pub fn inject_after(&self, after: u64, fault: Fault) {
        let mut state = self.state();
        let at = state.calls + after + 1;
        state.faults.push((at, fault));
    }

    /// Order-immune, unlike `inject_after`: a spawned engine's own loops also call this backend, so
    /// a counted fault lands on the wrong call under load (#210).
    pub fn fail_all(&self, fault: Fault) {
        self.state().persistent_fault = Some(fault);
    }

    /// Narrower than `fail_all`: a spawned engine's other calls would hit the fault first (#1943).
    pub fn fail_body_fetches(&self, fault: Fault) {
        self.state().body_fault = Some(fault);
    }

    /// Narrower than a `Fault`: the case under test is a refused UID listing with everything else
    /// working; a sync then walks the UID space (#727).
    pub fn refuse_uid_listing(&self) {
        self.state().refuse_uid_listing = true;
    }

    /// The call succeeds and under-reports, which costs a mailbox its mail: a pass believing a
    /// short listing stamps `last_full_sync_at`, and every later pass is incremental against a
    /// never-enumerated mailbox.
    pub fn truncate_uid_listing(&self, keep: usize) {
        self.state().truncate_uid_listing = Some(keep);
    }

    /// What io-imap does with an undecodable untagged `FETCH`: drops it and completes `Ok`. A
    /// caller reading absence as "expunged" would delete a live message.
    pub fn drop_from_header_fetches(&self, uid: u32) {
        self.state().undecodable.push(uid);
    }

    /// The download size, which the call count cannot show.
    pub fn header_fetch_rows(&self) -> Vec<usize> {
        self.state().header_fetch_rows.clone()
    }

    /// RFC 4315 §3 lets a UIDPLUS server omit `COPYUID` (`UIDNOTSTICKY`, or an append-only
    /// destination). Reading that as "message gone" is #903.
    pub fn omit_uid_mappings(&self) {
        self.state().omit_uid_mappings = true;
    }

    /// RFC 7162 §3.2 requires the increment on expunge only of QRESYNC servers, not CONDSTORE-only
    /// ones.
    pub fn expunge_keeps_mod_seq(&self) {
        self.state().expunge_keeps_mod_seq = true;
    }

    /// Narrower than a `Fault`: the case under test is refused creation with everything else
    /// working; discovery must finish, leave the role unmapped and not ask again.
    pub fn refuse_creates(&self, reason: impl Into<String>) {
        self.state().refuse_creates = Some(reason.into());
    }

    /// Narrower than a `Fault`: one folder fails its pass while the link and other folders stay
    /// healthy (#1731).
    pub fn refuse_select(&self, path: impl Into<String>, reason: impl Into<String>) {
        self.state()
            .refuse_select
            .push((path.into(), reason.into()));
    }

    pub fn allow_select(&self, path: &str) {
        self.state().refuse_select.retain(|(each, _)| each != path);
    }

    pub fn clear_faults(&self) {
        let mut state = self.state();
        state.persistent_fault = None;
        state.body_fault = None;
        state.faults.clear();
    }

    pub fn set_latency(&self, latency: Duration) {
        self.state().latency = latency;
    }

    /// A parked fetch makes a pass demonstrably mid-flight at any machine load (#2105, #2166). Its
    /// cancel token still ends it, so a cancelled lane does not hang.
    /// A second hold on the same kind and mailbox replaces the first.
    pub fn hold(&self, kind: Held, mailbox: &str, admit: usize) {
        let mut state = self.state();
        state
            .holds
            .retain(|hold| !(hold.kind == kind && hold.mailbox == mailbox));
        state.holds.push(Hold {
            kind,
            mailbox: mailbox.to_owned(),
            admit,
        });
    }

    pub fn release(&self) {
        self.state().holds.clear();
        self.released.notify_waiters();
    }

    /// What a test waits for before interrupting: the pass is inside a call that cannot finish on
    /// its own.
    pub fn parked(&self) -> usize {
        self.state().parked
    }

    async fn gate(&self, kind: Held, mailbox: &str, cancel: &CancelToken) -> BackendResult<()> {
        loop {
            // Enabled before the check, so a release between check and wait still wakes it.
            let released = self.released.notified();
            tokio::pin!(released);
            released.as_mut().enable();
            {
                let mut state = self.state();
                match state
                    .holds
                    .iter_mut()
                    .find(|hold| hold.kind == kind && hold.mailbox == mailbox)
                {
                    None => return Ok(()),
                    Some(hold) if hold.admit > 0 => {
                        hold.admit -= 1;
                        return Ok(());
                    }
                    Some(_) => state.parked += 1,
                }
            }
            let parked = Parked(Arc::clone(&self.state));
            tokio::select! {
                () = released => {}
                () = cancel.cancelled() => return Err(BackendError::Cancelled),
            }
            // Looked at again: a new hold may have been set since.
            drop(parked);
        }
    }

    pub fn calls(&self) -> u64 {
        self.state().calls
    }

    /// Includes paths that already existed, so a test can see a second attempt.
    pub fn created(&self) -> Vec<String> {
        self.state().created.clone()
    }

    /// Order in this log is causal and survives any machine load; assert on positions, not a
    /// stopwatch (#125). Only served fetches appear.
    pub fn header_fetches(&self) -> Vec<String> {
        self.state()
            .header_fetches
            .iter()
            .map(|(_, mailbox)| mailbox.clone())
            .collect()
    }

    /// The count for one mailbox is how many passes over it started: tells a wave that kept its
    /// lanes from one that restarted them (#1593).
    pub fn uid_listings(&self) -> Vec<String> {
        self.state().uid_listings.clone()
    }

    /// Not among `header_fetches`: a flag sweep carries no message data.
    pub fn flag_fetches(&self) -> Vec<String> {
        self.state().flag_fetches.clone()
    }

    /// Mimics iCloud, Yahoo and Exchange when another device sets a flag: neither `CHANGEDSINCE`
    /// nor a watcher can see it.
    pub fn set_flags_silently(&self, mailbox: &str, uid: Uid, flags: FlagSet) {
        let mut state = self.state();
        let index = self
            .locate(&state, mailbox, "STORE")
            .expect("the mailbox exists");
        let message = state.mailboxes[index]
            .messages
            .iter_mut()
            .find(|message| message.uid == uid.get())
            .expect("the message exists");
        message.flags = flags;
    }

    /// Causal order, like `header_fetches` (#631).
    pub fn body_fetches(&self) -> Vec<String> {
        self.state()
            .body_fetches
            .iter()
            .map(|(_, mailbox)| mailbox.clone())
            .collect()
    }

    /// A batch of twenty messages sharing a section should appear once with twenty, not twenty
    /// times with one (#1551).
    pub fn section_batches(&self) -> Vec<(String, usize)> {
        self.state().section_batches.clone()
    }

    /// Merged by `State::calls`, which is monotonic across every call kind, so "bodies must not
    /// queue behind another folder's headers" is checkable (#631).
    pub fn fetch_order(&self) -> Vec<FetchEvent> {
        let state = self.state();
        let mut merged: Vec<(u64, FetchEvent)> = state
            .header_fetches
            .iter()
            .map(|(seq, mailbox)| (*seq, FetchEvent::Header(mailbox.clone())))
            .chain(
                state
                    .body_fetches
                    .iter()
                    .map(|(seq, mailbox)| (*seq, FetchEvent::Body(mailbox.clone()))),
            )
            .collect();
        merged.sort_by_key(|(seq, _)| *seq);
        merged.into_iter().map(|(_, event)| event).collect()
    }

    /// Zero without a latency set: "on the wire" is the stretch a call spends in `set_latency`, the
    /// only part that yields. Cancelled calls are counted out.
    pub fn peak_in_flight(&self) -> usize {
        self.state().peak_in_flight
    }

    /// Every data operation on the mailbox then fails with `UidValidityChanged` until acknowledged;
    /// a real server says it once at `SELECT`, the mock keeps saying it so a test cannot pass by
    /// missing it.
    pub fn change_uid_validity(&self, mailbox: &str, uid_validity: UidValidity) {
        let mut state = self.state();
        let Ok(index) = state.index_of(mailbox) else {
            return;
        };
        let mailbox = &mut state.mailboxes[index];
        mailbox.stale_since = Some(mailbox.uid_validity);
        mailbox.uid_validity = uid_validity;
    }

    pub fn acknowledge_uid_validity(&self, mailbox: &str) {
        let mut state = self.state();
        if let Ok(index) = state.index_of(mailbox) {
            state.mailboxes[index].stale_since = None;
        }
    }

    pub fn push_event(&self, mailbox: &str, event: MailboxEvent) {
        {
            let mut state = self.state();
            let Ok(index) = state.index_of(mailbox) else {
                return;
            };
            state.mailboxes[index].pending.push(event);
        }
        self.notify.notify_waiters();
    }

    /// Like `append`, with the structure and part bytes an `APPEND` cannot carry.
    pub fn deliver(&self, mailbox: &str, message: MockMessage) {
        {
            let mut state = self.state();
            let Ok(index) = state.index_of(mailbox) else {
                return;
            };
            let folder = &mut state.mailboxes[index];
            folder.push_arriving(message);
            let count = folder.messages.len() as u32;
            self.announce(&mut state, index, MailboxEvent::Exists { count });
        }
        self.notify.notify_waiters();
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("mock backend mutex")
    }

    async fn serve_part(
        &self,
        mailbox: &str,
        id: &RemoteId,
        part: &BodyPart,
        sink: &mut dyn BodySink,
        cancel: &CancelToken,
    ) -> BackendResult<FetchedBody> {
        if cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }

        let (bytes, chunk_size) = {
            let mut state = self.state();
            let seq = state.calls;
            state.body_fetches.push((seq, mailbox.to_owned()));
            let state = state;
            let index = self.locate(&state, mailbox, "FETCH BODY")?;
            let folder = &state.mailboxes[index];
            let uid = identity::wire_uid(mailbox, folder.uid_validity, id)?;
            let message = folder
                .find(uid.get())
                .ok_or_else(|| BackendError::NoSuchMessage {
                    mailbox: mailbox.to_owned(),
                    uid: uid.get(),
                })?;
            (section(message, part)?, state.chunk_size)
        };

        let mut written = 0u64;
        for chunk in bytes.chunks(chunk_size) {
            if cancel.is_cancelled() {
                return Err(BackendError::Cancelled);
            }
            sink.chunk(chunk).await?;
            written += chunk.len() as u64;
        }
        sink.finish().await?;

        Ok(FetchedBody {
            remote_id: id.clone(),
            part: part.clone(),
            bytes_written: written,
        })
    }

    async fn enter(&self, context: &str) -> BackendResult<()> {
        let (latency, fault) = {
            let mut state = self.state();
            state.calls += 1;
            let call = state.calls;
            let fault = state.take_fault(call).or_else(|| {
                (context == "FETCH BODY")
                    .then(|| state.body_fault.clone())
                    .flatten()
            });
            (state.latency, fault)
        };

        // A guard, not a decrement after the await: a future cancelled mid-sleep must not leave the
        // in-flight count high.
        let _in_flight = InFlight::enter(&self.state);

        if !latency.is_zero() {
            tokio::time::sleep(latency).await;
        }

        match fault {
            None => Ok(()),
            Some(fault) => Err(self.raise(fault, context)),
        }
    }

    fn raise(&self, fault: Fault, context: &str) -> BackendError {
        let host = self.state().host.clone();
        match fault {
            Fault::Disconnect => {
                self.state().connected = false;
                BackendError::Disconnected {
                    context: context.to_owned(),
                    reason: "connection reset by peer".to_owned(),
                }
            }
            Fault::Timeout => BackendError::TimedOut {
                context: context.to_owned(),
                after: Duration::from_secs(30),
            },
            Fault::AuthFailed => BackendError::Auth {
                account: host,
                reason: "the server rejected the app-specific password".to_owned(),
            },
            Fault::RateLimited(retry_after) => BackendError::RateLimited {
                retry_after,
                reason: "too many simultaneous connections".to_owned(),
            },
            Fault::Rejected(reason) => BackendError::Rejected {
                command: context.to_owned(),
                reason,
            },
            Fault::Io(reason) => BackendError::Io {
                context: context.to_owned(),
                reason,
            },
        }
    }

    fn locate(&self, state: &State, path: &str, context: &str) -> BackendResult<usize> {
        state.require_connected(context)?;
        let index = state.index_of(path)?;
        state.mailboxes[index].guard_uid_validity()?;
        Ok(index)
    }

    /// Every change made through this backend is announced, the caller's own included; see
    /// `MailBackend::idle` for why that differs from a real server. Use `push_event` for a change
    /// nothing here made.
    fn announce(&self, state: &mut State, index: usize, event: MailboxEvent) {
        state.mailboxes[index].pending.push(event);
    }
}

#[derive(Clone, Debug)]
pub struct MockBackendBuilder {
    host: String,
    capabilities: Capabilities,
    mailboxes: Vec<MockMailbox>,
    chunk_size: usize,
    change_feed: ChangeFeed,
}

impl MockBackendBuilder {
    pub fn new() -> Self {
        Self {
            host: DEFAULT_HOST.to_owned(),
            capabilities: Capabilities::from_names([
                "IMAP4rev1",
                "ENABLE",
                "CONDSTORE",
                "QRESYNC",
                "IDLE",
                "UIDPLUS",
                "MOVE",
            ]),
            mailboxes: Vec::new(),
            chunk_size: DEFAULT_CHUNK,
            change_feed: ChangeFeed::Reliable,
        }
    }

    pub fn unreliable_change_feed(mut self) -> Self {
        self.change_feed = ChangeFeed::Unreliable;
        self
    }

    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.host = host.into();
        self
    }

    /// An empty list is legal to build, so a test can prove `connect` refuses it.
    pub fn capabilities<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.capabilities = Capabilities::from_names(names);
        self
    }

    pub fn mailbox(mut self, mailbox: MockMailbox) -> Self {
        self.mailboxes.push(mailbox);
        self
    }

    pub fn chunk_size(mut self, chunk_size: usize) -> Self {
        self.chunk_size = chunk_size.max(1);
        self
    }

    pub fn build(self) -> MockBackend {
        MockBackend {
            state: Arc::new(Mutex::new(State {
                host: self.host,
                connected: false,
                capabilities: self.capabilities,
                mailboxes: self.mailboxes.into_iter().map(MailboxState::seed).collect(),
                faults: Vec::new(),
                persistent_fault: None,
                body_fault: None,
                refuse_uid_listing: false,
                truncate_uid_listing: None,
                omit_uid_mappings: false,
                expunge_keeps_mod_seq: false,
                latency: Duration::ZERO,
                calls: 0,
                refuse_creates: None,
                refuse_select: Vec::new(),
                created: Vec::new(),
                in_flight: 0,
                peak_in_flight: 0,
                header_fetches: Vec::new(),
                body_fetches: Vec::new(),
                uid_listings: Vec::new(),
                header_fetch_rows: Vec::new(),
                undecodable: Vec::new(),
                flag_fetches: Vec::new(),
                change_feed: self.change_feed,
                section_batches: Vec::new(),
                chunk_size: self.chunk_size,
                holds: Vec::new(),
                parked: 0,
            })),
            notify: Arc::new(Notify::new()),
            released: Arc::new(Notify::new()),
        }
    }
}

impl Default for MockBackendBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MailBackend for MockBackend {
    fn describe(&self) -> &'static str {
        "mock"
    }

    async fn connect(&self) -> BackendResult<Capabilities> {
        self.enter("CONNECT").await?;

        let mut state = self.state();
        if state.capabilities.is_empty() {
            return Err(BackendError::EmptyCapabilities {
                host: state.host.clone(),
            });
        }
        state.connected = true;
        Ok(state.capabilities.clone())
    }

    async fn disconnect(&self) -> BackendResult<()> {
        self.enter("LOGOUT").await?;
        self.state().connected = false;
        Ok(())
    }

    async fn capabilities(&self) -> BackendResult<Capabilities> {
        self.enter("CAPABILITY").await?;
        let state = self.state();
        state.require_connected("CAPABILITY")?;
        Ok(state.capabilities.clone())
    }

    async fn list_mailboxes(&self, filter: &MailboxFilter) -> BackendResult<Vec<MailboxSummary>> {
        self.enter("LIST").await?;
        let state = self.state();
        state.require_connected("LIST")?;

        let mut listed: Vec<MailboxSummary> = state
            .mailboxes
            .iter()
            .map(|mailbox| mailbox.summary.clone())
            .filter(|summary| !filter.subscribed_only || summary.subscribed)
            .filter(|summary| matches_pattern(&filter.pattern, &summary.path))
            .collect();
        // The IMAP edge settles roles on a listing, so the mock does too: no real backend yields
        // two folders with one role.
        super::resolve_roles(&mut listed);
        Ok(listed)
    }

    async fn create_mailbox(&self, path: &str) -> BackendResult<()> {
        self.enter("CREATE").await?;
        let mut state = self.state();
        state.require_connected("CREATE")?;
        state.created.push(path.to_owned());
        if let Some(reason) = state.refuse_creates.clone() {
            return Err(BackendError::Rejected {
                command: "CREATE".to_owned(),
                reason,
            });
        }
        // Already there is success: the caller wants the folder to exist.
        if state.index_of(path).is_ok() {
            return Ok(());
        }
        state
            .mailboxes
            .push(MailboxState::seed(MockMailbox::new(path.to_owned())));
        Ok(())
    }

    async fn select(&self, path: &str, mode: SelectMode) -> BackendResult<MailboxStatus> {
        self.enter("SELECT").await?;
        let state = self.state();
        state.require_connected("SELECT")?;
        if let Some((_, reason)) = state.refuse_select.iter().find(|(each, _)| each == path) {
            return Err(BackendError::Rejected {
                command: "SELECT".to_owned(),
                reason: reason.clone(),
            });
        }
        let index = state.index_of(path)?;
        Ok(state.mailboxes[index].status(state.condstore(), mode == SelectMode::ReadOnly))
    }

    async fn status(&self, path: &str) -> BackendResult<MailboxStatus> {
        self.enter("STATUS").await?;
        let state = self.state();
        state.require_connected("STATUS")?;
        let index = state.index_of(path)?;
        Ok(state.mailboxes[index].status(state.condstore(), true))
    }

    async fn fetch_headers(
        &self,
        mailbox: &str,
        uids: &UidSet,
        changed_since: Option<ModSeq>,
        cancel: &CancelToken,
    ) -> BackendResult<Vec<FetchedMessage>> {
        self.fetched(mailbox, uids, changed_since, false, cancel)
            .await
    }

    /// Reads the promoted fields with the same parser the IMAP adapter uses on the block it is
    /// sent.
    async fn fetch_headers_for_filing(
        &self,
        mailbox: &str,
        uids: &UidSet,
        changed_since: Option<ModSeq>,
        cancel: &CancelToken,
    ) -> BackendResult<Vec<FetchedMessage>> {
        self.fetched(mailbox, uids, changed_since, true, cancel)
            .await
    }

    async fn fetch_flags(
        &self,
        mailbox: &str,
        cancel: &CancelToken,
    ) -> BackendResult<Option<Vec<(Uid, FlagSet)>>> {
        self.enter("FETCH FLAGS").await?;
        if cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }
        let mut state = self.state();
        state.flag_fetches.push(mailbox.to_owned());
        let state = state;
        let index = self.locate(&state, mailbox, "FETCH FLAGS")?;
        Ok(Some(
            state.mailboxes[index]
                .messages
                .iter()
                .map(|message| (Uid::new(message.uid), message.flags.clone()))
                .collect(),
        ))
    }

    fn change_feed(&self) -> ChangeFeed {
        self.state().change_feed
    }

    async fn fetch_sections(
        &self,
        mailbox: &str,
        ids: &[RemoteId],
        part: &BodyPart,
        cancel: &CancelToken,
    ) -> BackendResult<Vec<(RemoteId, Vec<u8>)>> {
        // Recorded before anything can fail: the question is whether the caller asked for a set.
        if let BodyPart::Section(section) = part {
            self.state()
                .section_batches
                .push((section.clone(), ids.len()));
        }
        // One command, one wait, as on the wire: charging latency per message would make a batch
        // look no cheaper than a walk.
        self.enter("FETCH BODY").await?;
        self.gate(Held::Bodies, mailbox, cancel).await?;
        let mut answered = Vec::with_capacity(ids.len());
        for id in ids {
            let mut sink = crate::backend::sink::VecSink::new();
            match self.serve_part(mailbox, id, part, &mut sink, cancel).await {
                Ok(_) if sink.is_finished() => answered.push((id.clone(), sink.into_inner())),
                Ok(_) => continue,
                Err(BackendError::NoSuchMessage { .. }) => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(answered)
    }

    async fn fetch_part(
        &self,
        mailbox: &str,
        id: &RemoteId,
        part: &BodyPart,
        sink: &mut dyn BodySink,
        cancel: &CancelToken,
    ) -> BackendResult<FetchedBody> {
        self.enter("FETCH BODY").await?;
        self.gate(Held::Bodies, mailbox, cancel).await?;
        self.serve_part(mailbox, id, part, sink, cancel).await
    }

    async fn store_flags(
        &self,
        mailbox: &str,
        ids: &[RemoteId],
        change: &FlagChange,
    ) -> BackendResult<Vec<FlagUpdate>> {
        self.enter("STORE").await?;

        let updates = {
            let mut state = self.state();
            let index = self.locate(&state, mailbox, "STORE")?;
            let condstore = state.condstore();
            let validity = state.mailboxes[index].uid_validity;
            let uids = identity::wire_set(mailbox, validity, ids)?;
            let mod_seq = state.mailboxes[index].bump_mod_seq();

            let mut updates = Vec::new();
            let mut events = Vec::new();
            for message in &mut state.mailboxes[index].messages {
                if !uids.contains(Uid::new(message.uid)) {
                    continue;
                }
                message.flags = change.apply(&message.flags);
                message.mod_seq = mod_seq;
                events.push(MailboxEvent::FlagsChanged {
                    uid: Some(Uid::new(message.uid)),
                    flags: message.flags.clone(),
                });
                updates.push(FlagUpdate {
                    remote_id: identity::remote_id(validity, Uid::new(message.uid)),
                    flags: message.flags.clone(),
                    mod_seq: condstore.then(|| ModSeq::new(mod_seq)),
                });
            }
            for event in events {
                self.announce(&mut state, index, event);
            }
            updates
        };

        self.notify.notify_waiters();
        Ok(updates)
    }

    async fn move_messages(
        &self,
        from: &str,
        ids: &[RemoteId],
        to: &str,
    ) -> BackendResult<Vec<UidMapping>> {
        self.enter("MOVE").await?;
        let mapping = self.transfer(from, ids, to, true, "MOVE")?;
        self.notify.notify_waiters();
        Ok(mapping)
    }

    async fn copy_messages(
        &self,
        from: &str,
        ids: &[RemoteId],
        to: &str,
    ) -> BackendResult<Vec<UidMapping>> {
        self.enter("COPY").await?;
        let mapping = self.transfer(from, ids, to, false, "COPY")?;
        self.notify.notify_waiters();
        Ok(mapping)
    }

    async fn expunge(
        &self,
        mailbox: &str,
        ids: Option<&[RemoteId]>,
    ) -> BackendResult<Vec<RemoteId>> {
        self.enter("EXPUNGE").await?;

        let expunged = {
            let mut state = self.state();
            let keeps_mod_seq = state.expunge_keeps_mod_seq;
            let index = self.locate(&state, mailbox, "EXPUNGE")?;
            let folder = &mut state.mailboxes[index];
            let validity = folder.uid_validity;
            let targeted = ids
                .map(|ids| identity::wire_set(mailbox, validity, ids))
                .transpose()?;

            let expunged: Vec<Uid> = folder
                .messages
                .iter()
                .filter(|message| message.flags.is_deleted())
                .filter(|message| {
                    targeted
                        .as_ref()
                        .is_none_or(|set| set.contains(Uid::new(message.uid)))
                })
                .map(|message| Uid::new(message.uid))
                .collect();

            folder
                .messages
                .retain(|message| !expunged.contains(&Uid::new(message.uid)));
            if !keeps_mod_seq {
                folder.bump_mod_seq();
            }

            for uid in &expunged {
                let event = MailboxEvent::Vanished { uids: vec![*uid] };
                self.announce(&mut state, index, event);
            }
            expunged
                .into_iter()
                .map(|uid| identity::remote_id(validity, uid))
                .collect::<Vec<_>>()
        };

        self.notify.notify_waiters();
        Ok(expunged)
    }

    async fn find_by_message_id(
        &self,
        mailbox: &str,
        message_id: &str,
    ) -> BackendResult<Option<RemoteId>> {
        self.enter("SEARCH").await?;
        let state = self.state();
        let index = self.locate(&state, mailbox, "SEARCH")?;
        let folder = &state.mailboxes[index];
        Ok(folder
            .messages
            .iter()
            .rev()
            .find(|message| {
                message
                    .envelope
                    .message_id
                    .as_ref()
                    .is_some_and(|id| id.as_str() == message_id)
            })
            .map(|message| identity::remote_id(folder.uid_validity, Uid::new(message.uid))))
    }

    /// Counted as a call but not recorded as a header fetch: it carries no message data.
    async fn existing_uids(
        &self,
        mailbox: &str,
        cancel: &CancelToken,
    ) -> BackendResult<Option<Vec<Uid>>> {
        self.enter("SEARCH").await?;
        if cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }
        let mut state = self.state();
        state.uid_listings.push(mailbox.to_owned());
        let state = state;
        if state.refuse_uid_listing {
            return Err(BackendError::Protocol {
                reason: "SEARCH is not available".to_owned(),
            });
        }
        let index = self.locate(&state, mailbox, "SEARCH")?;
        let mut uids: Vec<Uid> = state.mailboxes[index]
            .messages
            .iter()
            .map(|message| Uid::new(message.uid))
            .collect();
        // The newest are kept, so the truncation does not look like an empty mailbox.
        if let Some(keep) = state.truncate_uid_listing {
            uids.sort_unstable_by_key(|uid| std::cmp::Reverse(uid.get()));
            uids.truncate(keep);
        }
        Ok(Some(uids))
    }

    async fn append(
        &self,
        mailbox: &str,
        message: &AppendMessage,
    ) -> BackendResult<Option<UidMapping>> {
        self.enter("APPEND").await?;

        let mapping = {
            let mut state = self.state();
            let index = self.locate(&state, mailbox, "APPEND")?;
            let uid_plus = state.uid_plus();
            let folder = &mut state.mailboxes[index];

            let seed = MockMessage {
                raw: message.raw.clone(),
                flags: message.flags.clone(),
                internal_date: message.internal_date,
                structure: None,
                parts: HashMap::new(),
            };
            let uid = folder.push_arriving(seed);
            let uid_validity = folder.uid_validity;
            let count = folder.messages.len() as u32;

            let event = MailboxEvent::Exists { count };
            self.announce(&mut state, index, event);

            uid_plus.then_some(UidMapping {
                // An appended message has no source; it is its own origin.
                source: Uid::new(uid),
                destination: Uid::new(uid),
                uid_validity,
                destination_remote_id: identity::remote_id(uid_validity, Uid::new(uid)),
            })
        };

        self.notify.notify_waiters();
        Ok(mapping)
    }

    async fn idle(
        &self,
        mailbox: &str,
        timeout: Duration,
        cancel: &CancelToken,
    ) -> BackendResult<Vec<MailboxEvent>> {
        self.enter("IDLE").await?;
        {
            let state = self.state();
            self.locate(&state, mailbox, "IDLE")?;
        }

        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);

        loop {
            // Subscribe before looking, so an event that lands between the
            // two is not missed.
            let notified = self.notify.notified();
            tokio::pin!(notified);

            {
                let mut state = self.state();
                let index = state.index_of(mailbox)?;
                let pending = std::mem::take(&mut state.mailboxes[index].pending);
                if !pending.is_empty() {
                    return Ok(pending);
                }
            }

            tokio::select! {
                _ = cancel.cancelled() => return Ok(Vec::new()),
                _ = &mut deadline => return Ok(Vec::new()),
                _ = &mut notified => continue,
            }
        }
    }
}

impl MockBackend {
    fn transfer(
        &self,
        from: &str,
        ids: &[RemoteId],
        to: &str,
        remove_source: bool,
        context: &str,
    ) -> BackendResult<Vec<UidMapping>> {
        let mut state = self.state();
        let source = self.locate(&state, from, context)?;
        let destination = self.locate(&state, to, context)?;
        let uid_plus = state.uid_plus();
        let uids = identity::wire_set(from, state.mailboxes[source].uid_validity, ids)?;

        let moving: Vec<MessageState> = state.mailboxes[source]
            .messages
            .iter()
            .filter(|message| uids.contains(Uid::new(message.uid)))
            .cloned()
            .collect();

        let mut mapping = Vec::new();
        for message in &moving {
            let folder = &mut state.mailboxes[destination];
            let uid = folder.push_arriving(MockMessage {
                raw: message.raw.clone(),
                flags: message.flags.clone(),
                internal_date: Some(message.internal_date),
                structure: message.structure.clone(),
                // A moved message keeps its seeded sections, or a text fetch after a move would
                // fail.
                parts: message.parts.clone(),
            });
            mapping.push(UidMapping {
                source: Uid::new(message.uid),
                destination: Uid::new(uid),
                uid_validity: folder.uid_validity,
                destination_remote_id: identity::remote_id(folder.uid_validity, Uid::new(uid)),
            });
        }

        let count = state.mailboxes[destination].messages.len() as u32;
        self.announce(&mut state, destination, MailboxEvent::Exists { count });

        if remove_source {
            let removed: Vec<u32> = moving.iter().map(|message| message.uid).collect();
            state.mailboxes[source]
                .messages
                .retain(|message| !removed.contains(&message.uid));
            state.mailboxes[source].bump_mod_seq();
            for uid in removed {
                let event = MailboxEvent::Vanished {
                    uids: vec![Uid::new(uid)],
                };
                self.announce(&mut state, source, event);
            }
        }

        Ok(if uid_plus && !state.omit_uid_mappings {
            mapping
        } else {
            Vec::new()
        })
    }
}

fn matches_pattern(pattern: &str, path: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    match pattern.strip_suffix('*') {
        Some(prefix) => path.starts_with(prefix),
        None => pattern == path,
    }
}

fn section(message: &MessageState, part: &BodyPart) -> BackendResult<Vec<u8>> {
    let raw = &message.raw;
    let split = sketch::body_offset(raw);
    match part {
        BodyPart::Whole => Ok(raw.to_vec()),
        BodyPart::Headers => Ok(raw[..split.headers_end].to_vec()),
        BodyPart::Text => Ok(raw[split.body_start..].to_vec()),
        // Rejected rather than empty: an empty answer is indistinguishable from a genuinely empty
        // part, so a "no body" test would pass for the wrong reason.
        BodyPart::Section(section) => {
            message
                .parts
                .get(section)
                .cloned()
                .ok_or_else(|| BackendError::Rejected {
                    command: format!("FETCH BODY[{section}]"),
                    reason: "the mock has no MIME parser; seed the part's bytes explicitly"
                        .to_owned(),
                })
        }
    }
}

impl MockBackend {
    async fn fetched(
        &self,
        mailbox: &str,
        uids: &UidSet,
        changed_since: Option<ModSeq>,
        promoted: bool,
        cancel: &CancelToken,
    ) -> BackendResult<Vec<FetchedMessage>> {
        self.enter("FETCH").await?;
        self.gate(Held::Headers, mailbox, cancel).await?;

        if cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }

        let mut state = self.state();
        let seq = state.calls;
        state.header_fetches.push((seq, mailbox.to_owned()));
        let index = self.locate(&state, mailbox, "FETCH")?;
        let condstore = state.condstore();
        let folder = &state.mailboxes[index];

        let served: Vec<FetchedMessage> = folder
            .messages
            .iter()
            .filter(|message| uids.contains(Uid::new(message.uid)))
            .filter(|message| !state.undecodable.contains(&message.uid))
            .filter(|message| match changed_since {
                // RFC 7162 CHANGEDSINCE is strictly greater than.
                Some(floor) => message.mod_seq > floor.get(),
                None => true,
            })
            .map(|message| FetchedMessage {
                content_identity: None,
                remote_id: identity::remote_id(folder.uid_validity, Uid::new(message.uid)),
                uid: Uid::new(message.uid),
                uid_validity: folder.uid_validity,
                mod_seq: condstore.then(|| ModSeq::new(message.mod_seq)),
                flags: message.flags.clone(),
                internal_date: message.internal_date,
                size: message.raw.len() as u64,
                envelope: Some(message.envelope.clone()),
                structure: message.structure.clone(),
                promoted: promoted.then(|| {
                    postio_model::promoted::PromotedHeaders::from_headers(
                        &postio_model::mime::parse_headers(&message.raw).headers,
                    )
                }),
            })
            .collect();
        let rows = served.len();
        state.header_fetch_rows.push(rows);
        Ok(served)
    }
}

/// Not a MIME parser: no RFC 2047 decoding or group addresses. Parsing belongs to the code under
/// test, or parser tests would test themselves.
mod sketch {
    use super::*;

    pub(super) struct Split {
        pub headers_end: usize,
        pub body_start: usize,
    }

    pub(super) fn body_offset(raw: &[u8]) -> Split {
        if let Some(at) = find(raw, b"\r\n\r\n") {
            return Split {
                headers_end: at + 2,
                body_start: at + 4,
            };
        }
        if let Some(at) = find(raw, b"\n\n") {
            return Split {
                headers_end: at + 1,
                body_start: at + 2,
            };
        }
        Split {
            headers_end: raw.len(),
            body_start: raw.len(),
        }
    }

    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    pub(super) fn envelope(raw: &[u8]) -> Envelope {
        let split = body_offset(raw);
        let headers = unfold(&String::from_utf8_lossy(&raw[..split.headers_end]));
        let get = |name: &str| -> Option<&str> {
            headers
                .iter()
                .find(|(field, _)| field.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
        };

        Envelope {
            date: get("date").and_then(|value| {
                DateTime::parse_from_rfc2822(value)
                    .ok()
                    .map(|date| date.with_timezone(&Utc))
            }),
            subject: get("subject").map(str::to_owned),
            from: get("from").map(addresses).unwrap_or_default(),
            sender: get("sender").and_then(|value| addresses(value).into_iter().next()),
            reply_to: get("reply-to").map(addresses).unwrap_or_default(),
            to: get("to").map(addresses).unwrap_or_default(),
            cc: get("cc").map(addresses).unwrap_or_default(),
            bcc: get("bcc").map(addresses).unwrap_or_default(),
            message_id: get("message-id").and_then(|value| message_ids(value).into_iter().next()),
            in_reply_to: get("in-reply-to").and_then(|value| message_ids(value).into_iter().next()),
            references: get("references").map(message_ids).unwrap_or_default(),
            list_id: get("list-id").and_then(postio_model::mime::list_id_from_text),
        }
    }

    fn unfold(block: &str) -> Vec<(String, String)> {
        let mut headers: Vec<(String, String)> = Vec::new();
        for line in block.split('\n') {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }
            if line.starts_with([' ', '\t']) {
                if let Some((_, value)) = headers.last_mut() {
                    value.push(' ');
                    value.push_str(line.trim());
                }
                continue;
            }
            if let Some((name, value)) = line.split_once(':') {
                headers.push((name.trim().to_owned(), value.trim().to_owned()));
            }
        }
        headers
    }

    fn addresses(value: &str) -> Vec<EmailAddress> {
        let mut out = Vec::new();
        let mut current = String::new();
        let mut quoted = false;
        let mut angled = false;

        for character in value.chars() {
            match character {
                '"' => {
                    quoted = !quoted;
                    current.push(character);
                }
                '<' if !quoted => {
                    angled = true;
                    current.push(character);
                }
                '>' if !quoted => {
                    angled = false;
                    current.push(character);
                }
                ',' if !quoted && !angled => {
                    push_address(&mut out, &current);
                    current.clear();
                }
                _ => current.push(character),
            }
        }
        push_address(&mut out, &current);
        out
    }

    fn push_address(out: &mut Vec<EmailAddress>, raw: &str) {
        let raw = raw.trim();
        if raw.is_empty() {
            return;
        }
        match raw.split_once('<') {
            Some((name, rest)) => {
                let address = rest.trim_end_matches('>').trim();
                let name = name.trim().trim_matches('"').trim();
                out.push(EmailAddress::new(
                    (!name.is_empty()).then(|| name.to_owned()),
                    address,
                ));
            }
            None => out.push(EmailAddress::new(None::<String>, raw)),
        }
    }

    fn message_ids(value: &str) -> Vec<RfcMessageId> {
        let mut out = Vec::new();
        let mut rest = value;
        while let Some(start) = rest.find('<') {
            let Some(end) = rest[start..].find('>') else {
                break;
            };
            out.push(RfcMessageId::new(&rest[start..start + end + 1]));
            rest = &rest[start + end + 1..];
        }
        if out.is_empty() {
            out.extend(
                value
                    .split_whitespace()
                    .filter(|token| token.contains('@'))
                    .map(RfcMessageId::new),
            );
        }
        out
    }
}
