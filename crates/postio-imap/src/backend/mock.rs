//! An in-memory [`MailBackend`], with injectable faults and latency.
//!
//! This is not test-only scaffolding tucked behind `#[cfg(test)]`: it is the
//! implementation `postio-sync` is *developed* against. The whole sync engine
//! — operation queue, resync, backoff, conflict handling — can be written and
//! tested before a socket is opened, and CLAUDE.md's rule that no test in the
//! default suite touches the network is a consequence rather than a chore.
//!
//! ```
//! # use postio_imap::backend::{Fault, MailBackend, MockBackend, MockMailbox};
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let backend = MockBackend::builder()
//!     .capabilities(["IMAP4rev1", "CONDSTORE", "QRESYNC", "IDLE", "UIDPLUS"])
//!     .mailbox(MockMailbox::new("INBOX"))
//!     .build();
//!
//! backend.connect().await?;
//! backend.inject(Fault::Disconnect);
//!
//! assert!(backend.status("INBOX").await.unwrap_err().is_transient());
//! # Ok(())
//! # }
//! ```
//!
//! # What it does not do
//!
//! The mock has no MIME parser. It reads a handful of headers to build an
//! [`Envelope`] — enough for threading and list rendering — and takes a
//! [`BodyStructure`] as given rather than deriving one. Parsing bytes into the
//! model is `postio-model`'s job, and describing a MIME tree is the real
//! backend's; duplicating either here would mean testing them against
//! themselves.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use postio_model::{EmailAddress, Flag, FlagSet, ModSeq, RemoteId, RfcMessageId, Uid, UidValidity};
use tokio::sync::Notify;

use super::identity;
use crate::cancel::CancelToken;

use super::{
    AppendMessage, BackendError, BackendResult, BodyPart, BodySink, BodyStructure, Capabilities,
    Capability, Envelope, FetchedBody, FetchedMessage, FlagChange, FlagUpdate, MailBackend,
    MailboxEvent, MailboxFilter, MailboxStatus, MailboxSummary, SelectMode, UidMapping, UidSet,
};

/// How much of a body the mock hands to a sink at a time.
const DEFAULT_CHUNK: usize = 64 * 1024;

/// The host name the mock claims to be, so error messages read like real ones.
const DEFAULT_HOST: &str = "mock.invalid";

// ---------------------------------------------------------------------------
// Faults
// ---------------------------------------------------------------------------

/// A failure to make the backend produce on demand.
///
/// Every one of these is a thing a real server does and a thing the sync
/// engine has to survive. Scheduling them by call index
/// ([`MockBackend::inject_after`]) is what makes a retry path testable: the
/// failure lands on a known call and the recovery is observable.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Fault {
    /// The connection dies mid-command. The session is closed, so the *next*
    /// call fails with [`BackendError::NotConnected`] until something
    /// reconnects.
    Disconnect,
    /// The server never answers.
    Timeout,
    /// The credentials are refused. Retrying will not help.
    AuthFailed,
    /// The server asks us to slow down.
    RateLimited(Option<Duration>),
    /// The server understood the command and said no.
    Rejected(String),
    /// The transport failed.
    Io(String),
}

// ---------------------------------------------------------------------------
// Seed types
// ---------------------------------------------------------------------------

/// A message to seed a [`MockMailbox`] with.
#[derive(Clone, Debug, Default)]
pub struct MockMessage {
    raw: Vec<u8>,
    flags: FlagSet,
    internal_date: Option<DateTime<Utc>>,
    structure: Option<BodyStructure>,
    parts: HashMap<String, Vec<u8>>,
}

impl MockMessage {
    /// A message from its raw RFC 5322 bytes.
    pub fn new(raw: impl Into<Vec<u8>>) -> Self {
        Self {
            raw: raw.into(),
            ..Self::default()
        }
    }

    /// Sets the flags the message already carries.
    pub fn with_flags(mut self, flags: FlagSet) -> Self {
        self.flags = flags;
        self
    }

    /// Seeds the bytes a `BODY[<section>]` fetch returns for one MIME part.
    ///
    /// The mock has no MIME parser and deliberately does not grow one: if it
    /// did, every parser test would be testing the parser against itself (see
    /// the [`sketch`] module's own warning). So a part's bytes are stated,
    /// exactly as its envelope and its `BODYSTRUCTURE` are.
    ///
    /// `bytes` are what the wire carries -- still base64 or quoted-printable,
    /// with no headers of their own -- because that is what a real server
    /// hands back and what the code under test has to cope with.
    pub fn with_part(mut self, section: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        self.parts.insert(section.into(), bytes.into());
        self
    }

    /// Sets `INTERNALDATE`; otherwise it is taken from the `Date` header.
    pub fn with_internal_date(mut self, internal_date: DateTime<Utc>) -> Self {
        self.internal_date = Some(internal_date);
        self
    }

    /// Sets the MIME structure the server should report for this message.
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

/// A mailbox to seed a [`MockBackend`] with.
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
    /// An empty mailbox at `path`.
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

    /// Sets the UID the first message is given.
    ///
    /// Models a folder that has been in use for a while: everything below
    /// `uid` was expunged years ago, so `UIDNEXT` is far larger than the
    /// number of messages left. That gap is invisible in a mailbox seeded
    /// from UID 1 — where the UID ceiling and the message count are the same
    /// number — and it is exactly what `postio-qhz.9` was about.
    pub fn starting_uid(mut self, uid: u32) -> Self {
        self.starting_uid = uid.max(1);
        self
    }

    /// Sets the hierarchy delimiter the server reports.
    pub fn delimiter(mut self, delimiter: char) -> Self {
        self.delimiter = Some(delimiter);
        self
    }

    /// Sets the `LIST` attributes, e.g. `\Sent` or `\Noselect`.
    ///
    /// This is how a test covers a server that *does* advertise `SPECIAL-USE`;
    /// leaving them off is how it covers iCloud, which does not.
    pub fn attributes<I, S>(mut self, attributes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.attributes = attributes.into_iter().map(Into::into).collect();
        self
    }

    /// Sets whether the account is subscribed to this mailbox.
    pub fn subscribed(mut self, subscribed: bool) -> Self {
        self.subscribed = subscribed;
        self
    }

    /// Sets the mailbox's UID generation.
    pub fn uid_validity(mut self, uid_validity: UidValidity) -> Self {
        self.uid_validity = uid_validity;
        self
    }

    /// Sets the modification sequence the mailbox starts at.
    pub fn highest_mod_seq(mut self, highest_mod_seq: ModSeq) -> Self {
        self.highest_mod_seq = highest_mod_seq;
        self
    }

    /// Adds a message. UIDs are assigned in insertion order, from
    /// [`starting_uid`](Self::starting_uid).
    pub fn message(mut self, message: impl Into<MockMessage>) -> Self {
        self.messages.push(message.into());
        self
    }

    /// Adds messages from the `.eml` corpus, by fixture name.
    ///
    /// The corpus is the same one every other crate tests against, so a sync
    /// test and a parser test are looking at the same bytes.
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

// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct MessageState {
    uid: u32,
    raw: Vec<u8>,
    flags: FlagSet,
    internal_date: DateTime<Utc>,
    mod_seq: u64,
    envelope: Envelope,
    structure: Option<BodyStructure>,
    /// Bytes a `BODY[<section>]` fetch answers with, as seeded by
    /// [`MockMessage::with_part`].
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
    /// The generation the caller still believes in, once the UID space has
    /// been renumbered underneath it.
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

    /// Adds a message that is *arriving now*, rather than being seeded.
    ///
    /// RFC 7162 §3.1.2.1 requires a new message's MODSEQ to be strictly
    /// greater than the mailbox's previous HIGHESTMODSEQ, so the counter moves
    /// before the message is stamped with it. Push first and a
    /// `FETCH (CHANGEDSINCE <the value observed a moment ago>)` — which is
    /// strictly greater-than — would never report the arrival at all.
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
    /// `(call index to fire on, fault)`, in schedule order.
    faults: Vec<(u64, Fault)>,
    /// A fault every call fails with until cleared. See
    /// [`MockBackend::fail_all`].
    persistent_fault: Option<Fault>,
    latency: Duration,
    calls: u64,
    /// Calls currently waiting out [`State::latency`].
    in_flight: usize,
    /// The largest [`State::in_flight`] ever reached.
    peak_in_flight: usize,
    chunk_size: usize,
    /// Which mailbox each served `FETCH` (headers) call was for, in the
    /// order the server handled them. See [`MockBackend::header_fetches`].
    header_fetches: Vec<String>,
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

// ---------------------------------------------------------------------------
// The backend
// ---------------------------------------------------------------------------

/// An in-memory [`MailBackend`].
///
/// Cloning shares one set of mailboxes and one fault schedule, so a clone
/// handed to a spawned task is the same server.
#[derive(Clone)]
pub struct MockBackend {
    state: Arc<Mutex<State>>,
    notify: Arc<Notify>,
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

/// Counts one call as in flight for as long as it is alive.
///
/// See [`MockBackend::peak_in_flight`].
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

impl MockBackend {
    /// A backend with an iCloud-shaped capability set and **no folders**.
    ///
    /// It used to invent an `INBOX`, and that one line is the fixture half of
    /// a shipped bug: with a server that always had an inbox, no test ever
    /// had to say where folders come from, and nothing noticed that the
    /// application never asked for them. `MailBackend::list_mailboxes` was
    /// implemented, tested, and had no production caller for the life of the
    /// project — a live account synced 0 mailboxes and 0 messages and
    /// reported success. See `postio-755` and `postio-bl2`.
    ///
    /// A test that needs folders now says so:
    ///
    /// ```
    /// # use postio_imap::backend::{MockBackend, MockMailbox};
    /// let backend = MockBackend::builder()
    ///     .mailbox(MockMailbox::new("INBOX"))
    ///     .build();
    /// # let _ = backend;
    /// ```
    ///
    /// Which is the point: a fixture must not supply what the wiring is
    /// supposed to produce.
    pub fn new() -> Self {
        Self::builder().build()
    }

    /// Starts describing a backend.
    pub fn builder() -> MockBackendBuilder {
        MockBackendBuilder::new()
    }

    /// Makes the next call fail.
    pub fn inject(&self, fault: Fault) {
        self.inject_after(0, fault);
    }

    /// Makes the call `after` calls from now fail.
    ///
    /// `inject_after(0, …)` is [`inject`](Self::inject); `inject_after(2, …)`
    /// lets two calls succeed and fails the third, which is how a backoff loop
    /// is tested without sleeping.
    pub fn inject_after(&self, after: u64, fault: Fault) {
        let mut state = self.state();
        let at = state.calls + after + 1;
        state.faults.push((at, fault));
    }

    /// Makes **every** call fail with `fault` until [`clear_faults`] is
    /// called.
    ///
    /// Use this — not a counted [`inject_after`] — whenever the test means
    /// "the server refuses this, whoever asks". [`inject_after`] schedules by
    /// absolute call number, and a spawned engine's own loops (the watcher's
    /// polls, the supervisor's dials) also call this backend on their own
    /// schedule: under load their calls interleave with the test's, the
    /// fault lands on the wrong call, and the test flakes (#210). A
    /// persistent fault is order-immune by construction. The positional form
    /// remains for tests that genuinely mean a positional fault — a
    /// connection dying mid-drain.
    ///
    /// [`clear_faults`]: Self::clear_faults
    /// [`inject_after`]: Self::inject_after
    pub fn fail_all(&self, fault: Fault) {
        self.state().persistent_fault = Some(fault);
    }

    /// Clears [`fail_all`](Self::fail_all)'s fault and any scheduled ones —
    /// "the user fixed it", whatever it was.
    pub fn clear_faults(&self) {
        let mut state = self.state();
        state.persistent_fault = None;
        state.faults.clear();
    }

    /// Delays every call by `latency`.
    pub fn set_latency(&self, latency: Duration) {
        self.state().latency = latency;
    }

    /// How many calls the backend has served.
    pub fn calls(&self) -> u64 {
        self.state().calls
    }

    /// Which mailbox each served header `FETCH` was for, oldest first.
    ///
    /// This is the mock's answer to scheduling questions, the same way
    /// [`peak_in_flight`](Self::peak_in_flight) is its answer to concurrency
    /// ones: **order in this log is causal and survives any machine load**,
    /// where "was X still incomplete when Y finished" is a wall-clock overlap
    /// that goes vacuous exactly when the box is slow (#125). A test that
    /// means "INBOX was not queued behind the archive" asserts on positions
    /// here, not on a stopwatch.
    ///
    /// Only *served* fetches are recorded: a call that a fault failed or that
    /// never got past connect does not appear.
    pub fn header_fetches(&self) -> Vec<String> {
        self.state().header_fetches.clone()
    }

    /// The most calls that were on the wire at once.
    ///
    /// "On the wire" is precisely the stretch a call spends waiting out
    /// [`set_latency`](Self::set_latency), which is the only part of a mock
    /// call that yields — so this is zero-information without a latency set,
    /// and with one it is exactly what a caller that overlaps its requests
    /// looks like from the server's side. A sequential caller never gets
    /// above one however fast it goes, which is what makes this the assertion
    /// for concurrency rather than a stopwatch.
    ///
    /// A call whose future is dropped mid-flight (a cancelled sync) is
    /// counted out again, so cancellation cannot inflate the peak.
    pub fn peak_in_flight(&self) -> usize {
        self.state().peak_in_flight
    }

    /// Renumbers a mailbox's UID space, as a server does after a restore.
    ///
    /// Every data operation on that mailbox then fails with
    /// [`BackendError::UidValidityChanged`] until
    /// [`acknowledge_uid_validity`](Self::acknowledge_uid_validity) is called.
    /// A real server only says it once, when you `SELECT`; the mock keeps
    /// saying it so that a test cannot pass by having missed it.
    pub fn change_uid_validity(&self, mailbox: &str, uid_validity: UidValidity) {
        let mut state = self.state();
        let Ok(index) = state.index_of(mailbox) else {
            return;
        };
        let mailbox = &mut state.mailboxes[index];
        mailbox.stale_since = Some(mailbox.uid_validity);
        mailbox.uid_validity = uid_validity;
    }

    /// Accepts a renumbered UID space, as a client does once it has resynced.
    pub fn acknowledge_uid_validity(&self, mailbox: &str) {
        let mut state = self.state();
        if let Ok(index) = state.index_of(mailbox) {
            state.mailboxes[index].stale_since = None;
        }
    }

    /// Pushes an event to whatever is idling on `mailbox`.
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

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("mock backend mutex")
    }

    /// Counts the call, waits out the configured latency, and fires any fault
    /// scheduled for it.
    async fn enter(&self, context: &str) -> BackendResult<()> {
        let (latency, fault) = {
            let mut state = self.state();
            state.calls += 1;
            let call = state.calls;
            (state.latency, state.take_fault(call))
        };

        // Dropped at the end of this scope *or* when the caller's future is
        // cancelled mid-sleep, which is a case the sync engine really does
        // produce: a guard rather than a decrement after the await, so a
        // cancelled call cannot leave the in-flight count permanently high
        // and every later peak reading wrong.
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

    /// Resolves a mailbox, checking the session and the UID generation first.
    fn locate(&self, state: &State, path: &str, context: &str) -> BackendResult<usize> {
        state.require_connected(context)?;
        let index = state.index_of(path)?;
        state.mailboxes[index].guard_uid_validity()?;
        Ok(index)
    }

    /// Queues an event for the next [`idle`](MailBackend::idle).
    ///
    /// Every change made through this backend is announced, the caller's own
    /// included — see [`MailBackend::idle`] for why that is deliberately
    /// unlike a real server, and which direction the difference errs in. Use
    /// [`push_event`](Self::push_event) to stage a change that nothing here
    /// made.
    fn announce(&self, state: &mut State, index: usize, event: MailboxEvent) {
        state.mailboxes[index].pending.push(event);
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Describes a [`MockBackend`] before it exists.
#[derive(Clone, Debug)]
pub struct MockBackendBuilder {
    host: String,
    capabilities: Capabilities,
    mailboxes: Vec<MockMailbox>,
    chunk_size: usize,
}

impl MockBackendBuilder {
    /// A builder with iCloud's documented post-auth capability set.
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
        }
    }

    /// Sets the host name the backend claims, for error messages.
    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.host = host.into();
        self
    }

    /// Sets the capabilities the server advertises after authentication.
    ///
    /// An empty list is legal to *build*, so that a test can prove
    /// [`connect`](MailBackend::connect) refuses it.
    pub fn capabilities<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.capabilities = Capabilities::from_names(names);
        self
    }

    /// Adds a mailbox.
    pub fn mailbox(mut self, mailbox: MockMailbox) -> Self {
        self.mailboxes.push(mailbox);
        self
    }

    /// Sets how much of a body reaches the sink at a time.
    pub fn chunk_size(mut self, chunk_size: usize) -> Self {
        self.chunk_size = chunk_size.max(1);
        self
    }

    /// Builds the backend.
    pub fn build(self) -> MockBackend {
        MockBackend {
            state: Arc::new(Mutex::new(State {
                host: self.host,
                connected: false,
                capabilities: self.capabilities,
                mailboxes: self.mailboxes.into_iter().map(MailboxState::seed).collect(),
                faults: Vec::new(),
                persistent_fault: None,
                latency: Duration::ZERO,
                calls: 0,
                in_flight: 0,
                peak_in_flight: 0,
                header_fetches: Vec::new(),
                chunk_size: self.chunk_size,
            })),
            notify: Arc::new(Notify::new()),
        }
    }
}

impl Default for MockBackendBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// MailBackend
// ---------------------------------------------------------------------------

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

        Ok(state
            .mailboxes
            .iter()
            .map(|mailbox| mailbox.summary.clone())
            .filter(|summary| !filter.subscribed_only || summary.subscribed)
            .filter(|summary| matches_pattern(&filter.pattern, &summary.path))
            .collect())
    }

    async fn select(&self, path: &str, mode: SelectMode) -> BackendResult<MailboxStatus> {
        self.enter("SELECT").await?;
        let state = self.state();
        state.require_connected("SELECT")?;
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
        self.enter("FETCH").await?;
        if cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }

        let mut state = self.state();
        state.header_fetches.push(mailbox.to_owned());
        let state = state;
        let index = self.locate(&state, mailbox, "FETCH")?;
        let condstore = state.condstore();
        let folder = &state.mailboxes[index];

        Ok(folder
            .messages
            .iter()
            .filter(|message| uids.contains(Uid::new(message.uid)))
            .filter(|message| match changed_since {
                // RFC 7162 CHANGEDSINCE is strictly greater than.
                Some(floor) => message.mod_seq > floor.get(),
                None => true,
            })
            .map(|message| FetchedMessage {
                remote_id: identity::remote_id(folder.uid_validity, Uid::new(message.uid)),
                uid: Uid::new(message.uid),
                uid_validity: folder.uid_validity,
                mod_seq: condstore.then(|| ModSeq::new(message.mod_seq)),
                flags: message.flags.clone(),
                internal_date: message.internal_date,
                size: message.raw.len() as u64,
                envelope: Some(message.envelope.clone()),
                structure: message.structure.clone(),
            })
            .collect())
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
        if cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }

        let (bytes, chunk_size) = {
            let state = self.state();
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
            folder.bump_mod_seq();

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
        // Newest match wins, as the trait says: any copy proves arrival.
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
    /// The shared body of `MOVE` and `COPY`.
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
                // A moved message is the same message: it keeps whatever
                // sections were seeded for it, or a text fetch after a move
                // would fail where the same fetch before it succeeded.
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

        Ok(if uid_plus { mapping } else { Vec::new() })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Matches an IMAP list pattern: `*` crosses hierarchy levels, `%` does not.
fn matches_pattern(pattern: &str, path: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    match pattern.strip_suffix('*') {
        Some(prefix) => path.starts_with(prefix),
        None => pattern == path,
    }
}

/// The bytes of one section of a raw message.
fn section(message: &MessageState, part: &BodyPart) -> BackendResult<Vec<u8>> {
    let raw = &message.raw;
    let split = sketch::body_offset(raw);
    match part {
        BodyPart::Whole => Ok(raw.to_vec()),
        BodyPart::Headers => Ok(raw[..split.headers_end].to_vec()),
        BodyPart::Text => Ok(raw[split.body_start..].to_vec()),
        // Rejected rather than empty when nothing was seeded. An empty answer
        // is indistinguishable from a part that is genuinely empty, and a test
        // asserting "no body" would then pass for the wrong reason.
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

// ---------------------------------------------------------------------------
// Header sketch
// ---------------------------------------------------------------------------

/// Just enough header reading to give the mock a plausible [`Envelope`].
///
/// **Not a MIME parser and not a substitute for one.** It does not decode
/// RFC 2047 encoded words, does not handle group addresses, and gives up on
/// anything exotic rather than guessing. Real parsing belongs to the code that
/// consumes the corpus for real; if this module tried to be that parser, every
/// parser test would be testing it against itself.
mod sketch {
    use super::*;

    pub(super) struct Split {
        pub headers_end: usize,
        pub body_start: usize,
    }

    /// Where the header block ends and the body begins.
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

    /// Reads the headers Postio threads and lists on.
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

    /// Splits a header block into `(name, value)`, joining folded lines.
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

    /// Splits an address list on commas that are not inside quotes or angle
    /// brackets, then reads `Display Name <local@domain>` or a bare addr-spec.
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

    /// Every `<…>` in a `Message-ID`, `In-Reply-To` or `References` value.
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
