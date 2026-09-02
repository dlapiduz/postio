//! Backfilling message bodies: newest first, out of the user's way, and never
//! in front of the message they just opened.
//!
//! # Two lanes and one rule
//!
//! Offline reading needs every body local; a first run that downloaded every
//! body before showing anything would be unusable. So headers land first
//! ([`crate::initial`]) and bodies follow behind — but "behind" must never mean
//! "behind the user". A message being opened is not speculative work, and it is
//! the only fetch anybody is watching a spinner for.
//!
//! Hence two lanes, and the rule that decides every question below: **the
//! interactive lane always wins.** It ignores the size cap, ignores a metered
//! connection, ignores the pause switches and jumps whatever the backlog holds.
//! The longest a user can ever wait behind the backfill is the one body already
//! on the wire when they clicked, which is what
//! [`Backfill::next_body`] being one-at-a-time is for.
//!
//! # Newest first
//!
//! Same reason as [`crate::initial`]: nobody opens a mail client to read the
//! oldest message in it. The backlog is a max-heap on the delivery time, so a
//! backfill interrupted after ten minutes has made the ten minutes of mail the
//! user is most likely to open available offline, not the ten minutes they read
//! years ago.
//!
//! # What holds the backlog
//!
//! Nothing here persists a queue, and there is deliberately no "to backfill"
//! table: `body_state` on the message row already *is* the durable record of
//! which messages have no body, and a second list of the same fact is a second
//! list to keep in step. This type is the in-memory *scheduler* over that fact
//! — the caller enqueues what it has just written headers for, and repopulates
//! the backlog from storage at startup.
//!
//! # No runtime
//!
//! Like [`Supervisor`](crate::connect::Supervisor) and
//! [`Watcher`](crate::watch::Watcher), this owns no task and no timer:
//! [`Backfill::next_body`] hands out one claim, [`fetch_body`] performs it, and
//! [`Backfill::finished`] settles it. A caller that wants the interactive lane
//! to run alongside a background fetch simply drives two.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};

use chrono::{DateTime, Utc};
use postio_imap::backend::{BackendError, BodyPart, MailBackend, VecSink};
use postio_imap::cancel::CancelToken;
use postio_model::{BodyState, MailboxId, MessageId, Uid, mime};
use postio_storage::BlobStore;
use postio_storage::repository::{
    BackfillCandidate, MailboxRepository, MessageRepository, StoredBody,
};
use rusqlite::Connection;

use crate::blob_sink::BlobSink;
use crate::drain::SyncError;

/// This module's result type.
pub type Result<T> = std::result::Result<T, SyncError>;

// ---------------------------------------------------------------------------
// Policy
// ---------------------------------------------------------------------------

/// How eagerly bodies are pulled ahead of being asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackfillPolicy {
    /// Whether the background lane runs at all — `[sync] body_fetch` in
    /// `config.toml`.
    ///
    /// Turning it off does not turn off reading: a message the user opens is
    /// still fetched. It only stops Postio pulling bodies nobody asked for.
    pub background: bool,
    /// The largest body the background lane will pull, if any.
    ///
    /// The interactive lane ignores this: the user asked for that message and
    /// is looking at a spinner, so a cap that refused them would be a bug
    /// wearing a policy's clothes.
    pub max_body_bytes: Option<u64>,
    /// Whether a metered connection pauses the background lane.
    pub pause_on_metered: bool,
    /// Whether the user interacting pauses the background lane.
    pub pause_when_active: bool,
    /// How many bodies one [`seed`] queues for one mailbox.
    ///
    /// A batch, not a horizon. The engine re-seeds whenever the queue drains,
    /// so a mailbox is covered by however many batches it takes; this only
    /// bounds how much of it is held in memory, and in front of an
    /// interactive fetch, at any one moment.
    ///
    /// It was a private constant in the engine, and the queue was seeded once
    /// per folder at startup and never again — so the cap read as a horizon,
    /// and everything below the newest `seed_batch` messages of a folder was
    /// never fetched until somebody opened it (#318).
    pub seed_batch: u32,
    /// The most background requests held in memory at once, across every
    /// mailbox.
    ///
    /// The backlog is a `BinaryHeap` of requests, each carrying an owned
    /// mailbox path. `seed_batch` bounds one folder's contribution, and
    /// nothing bounded the sum — fine at 200 across forty folders, and not
    /// fine the first time something re-seeds a whole folder. ADR 0017's
    /// second axis: no mailbox is ever resident in this process.
    ///
    /// Overflow is *refused*, not evicted. The heap is newest-first, so
    /// evicting would drop the mail most likely to be opened, and there is
    /// nothing to lose by refusing: `body_state` is the durable record of what
    /// still needs fetching, and [`seed`] re-reads it whenever the queue
    /// drains. The heap is a window onto that record, never a second copy of
    /// it.
    ///
    /// It bounds the *background* lane only. The interactive lane is subject
    /// to none of the policy, this included.
    pub max_backlog: usize,
    /// The largest *inline* part the text axis will carry with the body.
    ///
    /// ADR 0017's "inline parts ride with the text": `disposition = 'inline'`
    /// is 2.64 GB of the reference account, almost all of it the CID images
    /// an HTML message references from its own body. A message whose inline
    /// images are missing renders as broken boxes, and the reader blocks
    /// *remote* images by default — so CID parts are the images that are
    /// supposed to appear. Under this cap a part is text; over it, a payload,
    /// so HTML mail reads correctly offline without pulling the forty-megabyte
    /// video somebody embedded.
    ///
    /// `None` turns the rule off and leaves every inline part on the payload
    /// axis, which is what the reader looked like before #751.
    pub max_inline_bytes: Option<u64>,
    /// What to do about attachment payloads — the *other* axis, governed
    /// separately from everything above it.
    pub attachments: AttachmentPolicy,
}

/// When an attachment's bytes are downloaded — ADR 0017's payload axis.
///
/// The fields above this one govern the **text** axis: headers and every
/// `text/*` part, for every message, to completion, because that is the corpus
/// search answers from and the corpus offline reading needs. Payloads are a
/// different question with a different answer. On the reference account they
/// are 11.00 GB of a 12.43 GB mailbox — 88.5% of the weight, and none of it
/// anything FTS5 can index. A PDF contributes its filename to search and
/// nothing else.
///
/// So this is a real choice rather than a tuning knob: it is the difference
/// between ~1.4 GB on disk and ~12.4 GB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AttachmentPolicy {
    /// Fetch a payload when the user opens or saves it, and never before.
    ///
    /// The default, and the one ADR 0016 stays affordable under. Metadata is
    /// still synced for every part, so `has:attachment` and `filename:`
    /// answer completely without a byte having moved.
    #[default]
    OnOpen,
    /// Backfill payloads behind the text, for a genuinely complete offline
    /// archive on a machine with the disk for one.
    Eager,
    /// Never fetch a payload, not even when the user opens it.
    ///
    /// Filename search and nothing more. An explicit refusal rather than a
    /// silent failure: the reader says the bytes are not here and were not
    /// asked for, which is a sentence a person can act on.
    Never,
}

impl Default for BackfillPolicy {
    /// Backfill, up to five megabytes a message, out of the way of both the
    /// user and their data plan.
    ///
    /// Five megabytes covers essentially every message that is text and
    /// images, and stops the background lane speculatively pulling the
    /// forty-megabyte attachment nobody may ever open — which would be paid
    /// for twice on a phone tether, once in time and once in money.
    fn default() -> Self {
        Self {
            background: true,
            max_body_bytes: Some(5 * 1024 * 1024),
            pause_on_metered: true,
            pause_when_active: true,
            seed_batch: 200,
            // Twenty batches in flight: enough that a drained queue is
            // re-seeded rather than starved, small enough that the heap and
            // its owned paths stay a rounding error beside one message's
            // bytes.
            max_backlog: 4_000,
            // 256 KiB, ADR 0017's number: comfortably above a signature logo
            // or a screenshot and far below anything a person would call a
            // download.
            max_inline_bytes: Some(256 * 1024),
            attachments: AttachmentPolicy::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Work
// ---------------------------------------------------------------------------

/// Which lane a fetch is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// The user is waiting for this one.
    Interactive,
    /// Nobody has asked; it is being fetched so that they can read it offline
    /// later.
    Background,
}

/// One body worth fetching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyRequest {
    /// The local row the bytes belong to.
    pub message: MessageId,
    /// The mailbox it is in.
    pub mailbox: MailboxId,
    /// That mailbox's path on the server.
    pub path: String,
    /// The server's identifier for the message.
    pub uid: Uid,
    /// The backend's own identity — what the fetch is addressed by (#543).
    pub remote_id: postio_model::RemoteId,
    /// `RFC822.SIZE`, as the header fetch reported it — what the cap is
    /// measured against.
    pub size: u64,
    /// When the server received it. The backlog's sort key.
    pub received_at: DateTime<Utc>,
    /// Which bytes of it to ask the server for.
    pub want: Want,
}

/// Which bytes of a message one request is after — ADR 0017's two axes, plus
/// the fallback for a row that predates them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Want {
    /// The sections holding the message's own words: the text axis, and what
    /// every background request asks for.
    ///
    /// Pulling payloads nobody asked for is exactly what the split exists to
    /// stop, so this is the default and the other two are opted into.
    #[default]
    Text,
    /// Named payload sections, by the `BODYSTRUCTURE` path
    /// `Attachment::part_id` holds — `2`, `2.1`.
    ///
    /// The payload axis. Interactive when the user opened one of them, and
    /// background when [`AttachmentPolicy::Eager`] is draining the backlog.
    Payloads(Vec<String>),
    /// Every byte, headers included.
    ///
    /// Two rows still need this: one whose `BODYSTRUCTURE` was never recorded,
    /// so no section can be named; and one whose payload has no
    /// `Attachment::part_headers`, so a fetched section could not be decoded.
    /// Slower and fatter, and the only answer that is not a guess.
    Whole,
}

impl Want {
    /// Folds another request for the same message into this one.
    ///
    /// Two payload requests union their parts: one round trip, both parts.
    /// Anything else has no smaller answer than every byte — a text request
    /// and a payload request together are the whole message, and a
    /// whole-message fallback on either side already was.
    fn absorb(&mut self, other: &Want) {
        if self == other {
            return;
        }
        match (&mut *self, other) {
            (Want::Payloads(mine), Want::Payloads(theirs)) => {
                for part in theirs {
                    if !mine.contains(part) {
                        mine.push(part.clone());
                    }
                }
            }
            _ => *self = Want::Whole,
        }
    }
}

impl From<BackfillCandidate> for BodyRequest {
    fn from(candidate: BackfillCandidate) -> Self {
        Self {
            message: candidate.message_id,
            mailbox: candidate.mailbox_id,
            path: candidate.mailbox_path,
            uid: candidate.uid,
            remote_id: candidate.remote_id,
            size: candidate.size,
            received_at: candidate.received_at,
            // The text axis is the default; the payload axis opts in.
            want: Want::Text,
        }
    }
}

/// A [`BodyRequest`] the queue has handed out.
///
/// Outstanding until it is reported through [`Backfill::finished`], which is
/// what stops the same body being put on the wire twice.
#[derive(Debug, Clone)]
pub struct Claim {
    /// What to fetch.
    pub request: BodyRequest,
    /// Which lane it came from.
    pub priority: Priority,
    /// Fired by [`Backfill::cancel`]. Pass it to [`fetch_body`].
    pub cancel: CancelToken,
}

/// What became of one body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The bytes are local now.
    Stored {
        /// How many bytes arrived.
        bytes: u64,
    },
    /// There is no local row to attach a body to any more — the message was
    /// deleted, or its mailbox was wiped by a `UIDVALIDITY` reset, while the
    /// request sat in the queue. Settled, not failed: nothing went wrong.
    Gone,
    /// The fetch failed.
    Failed {
        /// Why, for the user or a bug report.
        reason: String,
    },
}

/// What the backfill has done and has left to do.
///
/// Every message that has ever entered the queue is in exactly one of these
/// counts, which is what makes a progress display add up rather than drift.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BackfillProgress {
    /// Queued and not yet started.
    pub pending: usize,
    /// Handed out and not yet reported.
    pub in_flight: usize,
    /// Fetched and written.
    pub stored: usize,
    /// Refused by the size cap without ever being attempted.
    pub skipped: usize,
    /// No longer had a local row.
    pub gone: usize,
    /// Attempted and failed.
    pub failed: usize,
    /// Bytes written by this backfill.
    pub bytes: u64,
}

impl BackfillProgress {
    /// How many messages the queue has finished with, one way or another.
    pub fn settled(&self) -> usize {
        self.stored + self.skipped + self.gone + self.failed
    }
}

// ---------------------------------------------------------------------------
// The queue
// ---------------------------------------------------------------------------

/// Where a message sits right now. Also the lazy-deletion marker: a heap entry
/// whose message is no longer `Background` is stale and skipped on the way out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lane {
    Background,
    Interactive,
    InFlight,
}

/// A backlog entry, ordered newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry(BodyRequest);

impl Ord for Entry {
    fn cmp(&self, other: &Self) -> Ordering {
        // `BinaryHeap` is a max-heap, so "greater" must mean "fetch sooner":
        // the most recently received message, ties broken by the higher UID so
        // the order is total and a test can assert it.
        self.0
            .received_at
            .cmp(&other.0.received_at)
            .then_with(|| self.0.uid.get().cmp(&other.0.uid.get()))
            .then_with(|| other.0.message.get().cmp(&self.0.message.get()))
    }
}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Decides which body to fetch next, and gets out of the user's way.
#[derive(Debug)]
pub struct Backfill {
    policy: BackfillPolicy,
    /// The user's clicks, in the order they made them.
    interactive: VecDeque<BodyRequest>,
    /// Everything else, newest first.
    background: BinaryHeap<Entry>,
    lanes: HashMap<MessageId, Lane>,
    progress: BackfillProgress,
    metered: bool,
    disk_full: bool,
    user_active: bool,
    cancel: CancelToken,
    cancelled: bool,
    /// Messages this session has offered and will not offer again.
    ///
    /// Two kinds: one over [`BackfillPolicy::max_body_bytes`], and one whose
    /// fetch failed or found nothing there. Both stay `body_state <> 'full'`
    /// in the store for ever, so without this every re-seed would offer them
    /// again — re-counting the skip, or re-queueing a failing fetch in a tight
    /// loop against the server. It holds failures and oversized mail only,
    /// never the whole mailbox: a body that lands leaves `needing_backfill`
    /// by becoming `full`.
    ///
    /// In memory, like the rest of the queue, so a restart offers them once
    /// more — which is the retry a transient failure deserves and the only one
    /// it gets.
    set_aside: HashSet<MessageId>,
    /// Requests that arrived while the same message was already on the wire,
    /// re-offered the moment it settles.
    ///
    /// Two attachment chips clicked in quick succession. A fetch already on
    /// the wire cannot grow a part, and the second request has nowhere to go —
    /// [`Backfill`] tracks one lane per message, deliberately, because that is
    /// what bounds how long anything can be stuck behind the backfill. Dropping
    /// it silently would leave the second spinner turning until the reading
    /// pane's own deadline gave up on bytes nobody was fetching.
    deferred: HashMap<MessageId, BodyRequest>,
}

impl Backfill {
    /// An empty backfill.
    pub fn new(policy: BackfillPolicy) -> Self {
        Self {
            policy,
            interactive: VecDeque::new(),
            background: BinaryHeap::new(),
            lanes: HashMap::new(),
            progress: BackfillProgress::default(),
            metered: false,
            disk_full: false,
            user_active: false,
            cancel: CancelToken::new(),
            cancelled: false,
            set_aside: HashSet::new(),
            deferred: HashMap::new(),
        }
    }

    /// The policy in force.
    pub fn policy(&self) -> BackfillPolicy {
        self.policy
    }

    /// What has happened so far.
    pub fn progress(&self) -> BackfillProgress {
        self.progress
    }

    /// Whether there is nothing queued and nothing on the wire.
    pub fn is_idle(&self) -> bool {
        self.progress.pending == 0 && self.progress.in_flight == 0
    }

    /// Whether [`cancel`](Self::cancel) has been called.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    /// Tells the backfill the connection now costs money.
    pub fn set_metered(&mut self, metered: bool) {
        self.metered = metered;
    }

    /// Tells the backfill the user is doing something.
    pub fn set_user_active(&mut self, active: bool) {
        self.user_active = active;
    }

    /// Tells the backfill the store is at its disk ceiling.
    ///
    /// Set from `[storage] max_bytes` against what the blob store actually
    /// occupies. Unlike the other pauses this one is not about politeness: a
    /// full store with the lane still running is backfill fetching, eviction
    /// freeing, and backfill fetching the same bytes again — a loop that burns
    /// a data plan to stay exactly as full as it was.
    ///
    /// Like the other pauses, it does not apply to the interactive lane. That
    /// is also the only way out: opening mail is what proves which blobs are
    /// worth keeping, and a client that stopped serving reads when its cache
    /// filled would be a client that had stopped working.
    pub fn set_disk_full(&mut self, full: bool) {
        self.disk_full = full;
    }

    /// Whether the background lane is held back right now, for any reason.
    ///
    /// What a status surface reports (#352): "not fetching" and "nothing left
    /// to fetch" look identical from the outside and mean opposite things.
    pub fn is_paused(&self) -> bool {
        !self.background_runs()
    }

    /// Queues a body to fetch when there is nothing better to do. Answers
    /// whether it actually joined the queue.
    ///
    /// A body over [`BackfillPolicy::max_body_bytes`] is counted as skipped and
    /// not queued — it stays on the server until somebody actually wants it, at
    /// which point [`request_now`](Self::request_now) fetches it regardless.
    /// A message already known to the queue is ignored rather than duplicated,
    /// and so is one already [set aside](Self::set_aside).
    ///
    /// The answer is what lets a caller tell "there was more to do" from
    /// "there were more rows, and none of them are worth offering again" —
    /// which is what stops the engine's top-up looping (#318). It is also why
    /// the size skip is counted once rather than on every re-seed.
    pub fn enqueue(&mut self, request: BodyRequest) -> bool {
        if self.cancelled
            || self.lanes.contains_key(&request.message)
            || self.set_aside.contains(&request.message)
        {
            return false;
        }
        if self
            .policy
            .max_body_bytes
            .is_some_and(|cap| request.size > cap)
        {
            self.progress.skipped += 1;
            self.set_aside.insert(request.message);
            return false;
        }
        // Full. Not `set_aside`: this message is perfectly fetchable and will
        // be offered again by the next `seed`, unlike one refused for its
        // size. Recording it as skipped would misreport the backlog as
        // permanently shorter than the work remaining.
        if self.background.len() >= self.policy.max_backlog {
            return false;
        }
        // Same reasoning for a store at its ceiling: nothing is wrong with
        // this message, there is simply nowhere to put it yet. Queueing it
        // would mean fetching bytes that eviction must immediately free.
        if !self.background_runs() && self.disk_full {
            return false;
        }
        self.lanes.insert(request.message, Lane::Background);
        self.background.push(Entry(request));
        self.progress.pending += 1;
        true
    }

    /// Asks for a body now, because the user is looking at it.
    ///
    /// Jumps the whole backlog, and is subject to none of the policy: the size
    /// cap, the metered pause and the active-user pause all exist to keep
    /// speculative work out of the way, and this is not speculative.
    ///
    /// A message already in the backlog is *promoted* rather than queued twice.
    /// One already queued in front absorbs this request's
    /// [`Want`](Want::absorb) rather than dropping it, and one already on the
    /// wire holds it until that fetch settles — a second attachment clicked
    /// while the first is downloading is a part to fetch next, not a part to
    /// forget.
    pub fn request_now(&mut self, request: BodyRequest) {
        if self.cancelled {
            return;
        }
        match self.lanes.get(&request.message) {
            // On the wire. Nothing can be added to a fetch already running, so
            // this waits for it: `finished` offers it again.
            Some(Lane::InFlight) => {
                match self.deferred.entry(request.message) {
                    std::collections::hash_map::Entry::Occupied(mut held) => {
                        held.get_mut().want.absorb(&request.want);
                    }
                    std::collections::hash_map::Entry::Vacant(empty) => {
                        empty.insert(request);
                    }
                }
                return;
            }
            // Queued in front already. One round trip can answer both, so the
            // request that will run absorbs this one's want.
            Some(Lane::Interactive) => {
                for queued in self
                    .interactive
                    .iter_mut()
                    .filter(|queued| queued.message == request.message)
                {
                    queued.want.absorb(&request.want);
                }
                return;
            }
            Some(Lane::Background) => {
                // The stale heap entry is skipped on its way out; the count
                // does not move, because the message did not leave the queue.
                self.lanes.insert(request.message, Lane::Interactive);
            }
            None => {
                self.lanes.insert(request.message, Lane::Interactive);
                self.progress.pending += 1;
            }
        }
        self.interactive.push_back(request);
    }

    /// Hands out the next body to fetch, or `None` when there is nothing to do
    /// right now.
    ///
    /// One at a time, and outstanding until [`finished`](Self::finished)
    /// reports it. That is what bounds how long the user can wait behind the
    /// backfill: one body, the one already on the wire.
    pub fn next_body(&mut self) -> Option<Claim> {
        if self.cancelled {
            return None;
        }

        if let Some(request) = self.take_interactive() {
            return Some(self.claim(request, Priority::Interactive));
        }
        if !self.background_runs() {
            return None;
        }
        let request = self.take_background()?;
        Some(self.claim(request, Priority::Background))
    }

    /// Records what became of a claim.
    pub fn finished(&mut self, message: MessageId, outcome: Outcome) {
        if self.lanes.remove(&message) != Some(Lane::InFlight) {
            return;
        }
        self.progress.in_flight = self.progress.in_flight.saturating_sub(1);
        match outcome {
            Outcome::Stored { bytes } => {
                self.progress.stored += 1;
                self.progress.bytes += bytes;
            }
            // Set aside, both of them. The row stays `body_state <> 'full'`
            // either way, so it comes back from `needing_backfill` on every
            // re-seed for ever: without this, a message the server has no
            // copy of, or one whose fetch keeps failing, would be re-queued
            // the instant the queue drained and fetched again immediately —
            // a retry loop at the speed of the engine's own loop. It is
            // offered once more on the next reconnection, and once more on
            // the next start, which is the retry a transient failure earns.
            Outcome::Gone => {
                self.progress.gone += 1;
                self.set_aside.insert(message);
            }
            Outcome::Failed { .. } => {
                self.progress.failed += 1;
                self.set_aside.insert(message);
            }
        }
        // Whatever was asked for while this was on the wire. Offered even
        // after a failure: it is the user asking a second time, and a request
        // they made by hand earns the attempt a speculative one would not.
        if let Some(held) = self.deferred.remove(&message) {
            self.request_now(held);
        }
    }

    /// Offers everything set aside one more time.
    ///
    /// A fetch that failed while the link was going down is not the same
    /// message as one the server genuinely cannot produce, and there is no way
    /// to tell them apart at the time. So a reconnection forgives them all: the
    /// next seed offers them again, and anything that fails a second time is
    /// set aside a second time.
    pub fn forgive_set_aside(&mut self) {
        self.set_aside.clear();
    }

    /// Stops the backfill and everything it has on the wire.
    ///
    /// The queue is emptied rather than parked: what is worth fetching is
    /// re-derivable from `body_state` at any time, so keeping a stale backlog
    /// across a shutdown buys nothing and risks acting on a mailbox that has
    /// moved.
    pub fn cancel(&mut self) {
        self.cancelled = true;
        self.cancel.cancel();
        self.interactive.clear();
        self.background.clear();
        self.lanes.clear();
        self.deferred.clear();
        self.progress.pending = 0;
        self.progress.in_flight = 0;
    }

    /// Makes a cancelled backfill usable again, with a fresh token.
    ///
    /// The counts are kept: they describe what this session has downloaded, and
    /// a reconnection does not undo that.
    pub fn restart(&mut self) {
        self.cancelled = false;
        self.cancel = CancelToken::new();
    }

    /// Whether the background lane may run right now.
    fn background_runs(&self) -> bool {
        self.policy.background
            && !self.disk_full
            && !(self.policy.pause_on_metered && self.metered)
            && !(self.policy.pause_when_active && self.user_active)
    }

    fn take_interactive(&mut self) -> Option<BodyRequest> {
        while let Some(request) = self.interactive.pop_front() {
            if self.lanes.get(&request.message) == Some(&Lane::Interactive) {
                return Some(request);
            }
        }
        None
    }

    fn take_background(&mut self) -> Option<BodyRequest> {
        while let Some(Entry(request)) = self.background.pop() {
            if self.lanes.get(&request.message) == Some(&Lane::Background) {
                return Some(request);
            }
        }
        None
    }

    fn claim(&mut self, request: BodyRequest, priority: Priority) -> Claim {
        self.lanes.insert(request.message, Lane::InFlight);
        self.progress.pending = self.progress.pending.saturating_sub(1);
        self.progress.in_flight += 1;
        Claim {
            request,
            priority,
            cancel: self.cancel.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Filling the queue from storage
// ---------------------------------------------------------------------------

/// Queues everything in `mailbox_id` still missing a body, newest first.
///
/// `Backfill` is deliberately in-memory (see the [module docs](self)): nothing
/// here persists a backlog, because `body_state` on the message row already is
/// the durable record of what is missing. This is what turns that record back
/// into a backlog — at startup, so a cold start does not sit at zero forever,
/// and again after a mailbox finishes an initial sync or a resync, so whatever
/// it just wrote headers for gets backfilled too.
///
/// Returns how many requests actually **joined** the queue — not how many
/// candidate rows were read.
///
/// The difference matters to the engine's top-up (#318), which repeats this
/// until it stops producing work: a folder whose remaining candidates are all
/// over the size cap, or all set aside after failing this session, answers
/// rows for ever and must answer zero here, or the top-up would never stop.
/// Not an error for there to be none.
///
/// Queues nothing at all for a folder marked
/// [`backfill_excluded`](postio_storage::repository::MailboxRepository::backfill_excluded)
/// (ADR 0016, #350) — the background lane only. [`request_body`] and
/// [`request_whole`] answer an interactive, on-open fetch exactly as they
/// would for any other folder; only the unattended top-up this feeds skips
/// an excluded one.
pub fn seed(
    connection: &Connection,
    backfill: &mut Backfill,
    mailbox_id: MailboxId,
    limit: u32,
) -> Result<usize> {
    if MailboxRepository::new(connection).backfill_excluded(mailbox_id)? {
        return Ok(0);
    }
    let messages = MessageRepository::new(connection);
    let mut offset = 0;
    loop {
        let candidates = messages.needing_backfill_from(mailbox_id, limit, offset)?;
        let read = candidates.len();
        let queued = candidates
            .into_iter()
            .filter(|candidate| backfill.enqueue(candidate.clone().into()))
            .count();
        // A batch that queued nothing was a batch of messages the scheduler
        // cannot use — over the size cap, or set aside after failing this
        // session — and those stay `body_state <> 'full'` for ever. Walking
        // past them is what lets the backfill reach the older mail behind
        // them; without it a folder whose newest `limit` messages are all
        // oversized would never be covered at all (#318).
        if queued > 0 || read < limit as usize {
            return Ok(queued);
        }
        offset += limit;
    }
}

/// Asks the backfill for one message right now, looking up what it needs from
/// storage so the caller only has to know the message it just opened.
///
/// Returns `false` when there is nothing to fetch — the message has no local
/// row any more, has no `UID` yet, or already has its full body — in which
/// case the reading pane already has everything it is going to get from this.
pub fn request_body(
    connection: &Connection,
    backfill: &mut Backfill,
    message_id: MessageId,
) -> Result<bool> {
    let Some(candidate) = MessageRepository::new(connection).backfill_candidate(message_id)? else {
        return Ok(false);
    };
    backfill.request_now(candidate.into());
    Ok(true)
}

/// As [`request_body`], but asks for every byte of the message.
///
/// Not the way an attachment is opened — that is [`request_payloads`], which
/// asks for the one section somebody clicked. This is for the callers that
/// genuinely need the *original bytes*: dragging a message out as
/// `message/rfc822`, "view source", and eventually verifying a signature over
/// the bytes it was made across.
///
/// Under ADR 0017 the text axis stores no raw source, so those bytes are not
/// on this machine for any message the background lane fetched, and there is
/// nowhere to get them but the server.
pub fn request_whole(
    connection: &Connection,
    backfill: &mut Backfill,
    message_id: MessageId,
) -> Result<bool> {
    let Some(candidate) = MessageRepository::new(connection).backfill_candidate(message_id)? else {
        return Ok(false);
    };
    let mut request = BodyRequest::from(candidate);
    request.want = Want::Whole;
    backfill.request_now(request);
    Ok(true)
}

/// Asks for named payload parts of one message, now, because the user opened
/// one of them.
///
/// The interactive half of ADR 0017's payload axis. The background lane
/// fetches text and leaves payloads on the server, so this is the only thing
/// that ever asks for them on demand — and it asks by section, not by
/// message: `BODY.PEEK[2]` for the part somebody clicked, not `BODY.PEEK[]`
/// for the forty megabytes it happens to sit beside.
///
/// Returns `false` when there is nothing to fetch, and each way of meaning
/// that is a different sentence the reading pane can say:
///
/// * [`AttachmentPolicy::Never`] — the bytes were never going to be fetched.
/// * every named part already has its bytes — read the blob instead.
/// * the message has no local row, or no `UID`, or is already `full`.
///
/// A part whose [`Attachment::part_headers`](postio_model::Attachment) were
/// never recorded escalates the request to [`Want::Whole`]: `BODY[2]` comes
/// back encoded with nothing to say how, and storing base64 as though it were
/// a PDF is worse than the extra bandwidth.
pub fn request_payloads(
    connection: &Connection,
    backfill: &mut Backfill,
    message_id: MessageId,
    parts: &[String],
) -> Result<bool> {
    if backfill.policy().attachments == AttachmentPolicy::Never {
        return Ok(false);
    }
    let messages = MessageRepository::new(connection);
    let Some(candidate) = messages.backfill_candidate(message_id)? else {
        return Ok(false);
    };
    let Some(message) = messages.get(message_id)? else {
        return Ok(false);
    };

    let mut wanted = Vec::new();
    let mut needs_every_byte = false;
    for part_id in parts {
        let Some(attachment) = message
            .attachments
            .iter()
            .find(|attachment| attachment.part_id.as_deref() == Some(part_id.as_str()))
        else {
            continue;
        };
        if attachment.blob_id.is_some() {
            continue;
        }
        if attachment.part_headers.is_none() {
            needs_every_byte = true;
        }
        wanted.push(part_id.clone());
    }
    if wanted.is_empty() {
        return Ok(false);
    }

    let mut request = BodyRequest::from(candidate);
    request.want = if needs_every_byte {
        Want::Whole
    } else {
        Want::Payloads(wanted)
    };
    backfill.request_now(request);
    Ok(true)
}

/// Queues the payloads of up to `limit` text-backfilled messages in
/// `mailbox_id`, and answers how many requests joined the queue.
///
/// The background half of the payload axis, and only ever called under
/// [`AttachmentPolicy::Eager`]: for someone who wants a genuinely complete
/// offline archive and has the disk for it. On the reference account that is
/// the difference between ~1.4 GB and ~12.4 GB, which is why it is a choice
/// and not a default.
///
/// Walks past a batch it cannot use for the same reason [`seed`] does (#318):
/// a message set aside after a failed fetch stays `partial` for ever, so a
/// folder whose newest batch is all such messages would answer the same rows
/// on every seed and never reach the mail behind them.
pub fn seed_payloads(
    connection: &Connection,
    backfill: &mut Backfill,
    mailbox_id: MailboxId,
    limit: u32,
) -> Result<usize> {
    if MailboxRepository::new(connection).backfill_excluded(mailbox_id)? {
        return Ok(0);
    }
    let messages = MessageRepository::new(connection);
    let mut offset = 0;
    loop {
        let candidates = messages.needing_payloads_from(mailbox_id, limit, offset)?;
        let read = candidates.len();
        let mut queued = 0;
        for candidate in candidates {
            let message_id = candidate.message_id;
            let Some(message) = messages.get(message_id)? else {
                continue;
            };
            let wanted = pending_payloads(&message);
            if wanted.is_empty() {
                continue;
            }
            let mut request = BodyRequest::from(candidate);
            request.want = Want::Payloads(wanted);
            if backfill.enqueue(request) {
                queued += 1;
            }
        }
        if queued > 0 || read < limit as usize {
            return Ok(queued);
        }
        offset += limit;
    }
}

/// The sections of `message` whose bytes are not on this machine yet.
///
/// A part with no `part_id` is not one of them: there is no section to name in
/// a `FETCH`, so nothing can be asked for. Neither is one with no
/// `part_headers` — see [`request_payloads`] for why a headerless section
/// cannot be decoded — which the eager lane skips rather than escalating to a
/// whole-message fetch, because nobody is waiting for it.
fn pending_payloads(message: &postio_model::Message) -> Vec<String> {
    message
        .attachments
        .iter()
        .filter(|attachment| attachment.blob_id.is_none() && attachment.part_headers.is_some())
        .filter_map(|attachment| attachment.part_id.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// Doing one
// ---------------------------------------------------------------------------

/// Fetches one message's body and writes it where the reader will look.
///
/// # What lands where
///
/// The raw RFC 5322 bytes go to the blob store verbatim, and the decoded
/// `text/plain` and `text/html` forms go beside them. Keeping the raw bytes is
/// not redundancy: it is what lets a later parser fix — a charset guess, an
/// encoded word nobody handled — be applied to mail already downloaded, without
/// going back to a server that may no longer have it.
///
/// # Why the writes are in this order
///
/// `body_state` is what everything else reads to decide whether a body is
/// local, so it is written *last*, by
/// [`set_body_blobs`](MessageRepository::set_body_blobs). A crash anywhere
/// before that leaves orphaned blobs — which the blob store's garbage
/// collection sweeps — and a message that still says it has no body, so the
/// next pass simply fetches it again. The opposite order would leave a message
/// that claims a body it does not have, which nothing can detect and nothing
/// can repair.
///
/// # Two axes, and why this fetches sections rather than the message
///
/// ADR 0017. `BODY.PEEK[]` pulls the whole message, attachments included: on
/// the reference account that is 12.43 GB fetched to index the 1.43 GB of it
/// that is words, because ~90% of a mailbox by weight is payloads FTS5 cannot
/// index. So when the header sync recorded where the text lives
/// ([`Message::text_part_id`]), this fetches exactly those sections and leaves
/// the payloads on the server until somebody opens one.
///
/// A row whose sections are unknown — synced before migration 0008, or from a
/// server that reported no `BODYSTRUCTURE` — falls back to the whole-message
/// fetch. Slower and fatter, but a body that silently never arrives would be a
/// hole in search, and guessing section `1` would be wrong for every multipart
/// message.
pub async fn fetch_body(
    connection: &Connection,
    blobs: &BlobStore,
    backend: &dyn MailBackend,
    request: &BodyRequest,
    inline_cap: Option<u64>,
    cancel: &CancelToken,
) -> Result<Outcome> {
    let messages = MessageRepository::new(connection);
    let Some(mut message) = messages.get(request.message)? else {
        // Deleted, or wiped by a UIDVALIDITY reset, while this sat in the
        // queue. Checked before the fetch so a stale queue costs no bandwidth.
        return Ok(Outcome::Gone);
    };

    match &request.want {
        // The payload axis: named sections, nothing around them.
        Want::Payloads(parts) => {
            let parts = parts.clone();
            return fetch_payloads(
                connection, blobs, backend, request, &message, &parts, cancel,
            )
            .await;
        }
        // The text axis, when the header sync left us a map of where the words
        // are. `content_type` is what says a `BODYSTRUCTURE` was parsed at all —
        // a message that is nothing but an attachment has no text sections and
        // must still take this path, or the fallback would fetch the payload this
        // exists to avoid.
        Want::Text if message.content_type.is_some() => {
            return fetch_text_parts(
                connection, blobs, backend, request, message, inline_cap, cancel,
            )
            .await;
        }
        // Every byte: asked for, or the only answer left for a row whose
        // structure was never recorded.
        Want::Text | Want::Whole => {}
    }

    // Straight to disk as it arrives, rather than into a `Vec` that doubles
    // its way up to the message's size (ADR 0017, axis 2). This is the path
    // the interactive lane takes for an oversized message, and the
    // interactive lane ignores `max_body_bytes` by design -- so it is exactly
    // the path where the buffer would have been worst.
    let mut sink = BlobSink::new(blobs)?;
    backend
        .fetch_body(&request.path, &request.remote_id, &mut sink, cancel)
        .await?;
    let Some(blob) = sink.finished_blob() else {
        // The sink contract: without `finish` the bytes are a fragment, and a
        // fragment stored as a message is worse than no message. Nothing was
        // published, so there is nothing to undo.
        return Err(SyncError::Backend(BackendError::Cancelled));
    };
    let bytes = sink.bytes();

    // Read back to parse. The parser needs the message whole, so this is one
    // exact-size allocation -- against the `Vec`'s doubling-and-copying, and
    // against a second full pass to `put` it afterwards. The bytes are also
    // durable before anything looks at them, so a parse that dies takes
    // nothing with it.
    let raw = blobs.get(&blob)?;
    // The ingest boundary of #277. `mime::parse` contains the panic itself, so
    // this is not what keeps sync alive -- it is what keeps the failure
    // visible. Without it a message whose bytes defeat the parser stores no
    // body and is indistinguishable, in a log, from one that genuinely had
    // none.
    //
    // Ids and sizes only: the bytes that caused this are somebody's mail.
    let parsed = mime::try_parse(&raw).unwrap_or_else(|_| {
        tracing::warn!(
            message = request.message.get(),
            bytes,
            "a fetched body could not be parsed; storing it with no body parts"
        );
        mime::parse(&raw)
    });

    let stored = StoredBody {
        text: stored_text(parsed.body.text.as_deref()),
        html: stored_text(parsed.body.html.as_deref()),
        // The header block has no reader of its own yet: everything that wants
        // headers has the row, and everything that wants all of them has the
        // raw blob. A copy nobody reads is a copy that can go stale.
        headers: None,
    };

    message.raw_blob_id = Some(blob);
    if message.preview.is_none() {
        message.preview = parsed.preview;
    }
    messages.update(&mut message)?;

    // Every payload arrived with the message, so record where each one landed
    // — the same content-addressed blob the payload axis would have written,
    // because `ParsedPart::content` is decoded and `BlobStore::put` names a
    // blob after its plaintext. A part fetched here and the same part fetched
    // by section are one blob, not two.
    //
    // Matched by MIME path rather than by position: the attachment rows came
    // from `BODYSTRUCTURE` at header-sync time and this parse is a second,
    // independent reading of the same message.
    for part in &parsed.parts {
        let Some(part_id) = part.attachment.part_id.as_deref() else {
            continue;
        };
        let blob = blobs.put(&part.content)?;
        messages.set_attachment_blob(request.message, part_id, &blob)?;
    }

    // The commit point. `Full` unconditionally, and honestly: whatever the
    // parse could not match to a row is still in the raw blob, which is what
    // `postio_app::reading::part_bytes` falls back to.
    messages.set_body(request.message, &stored, BodyState::Full)?;

    // And into the search index, *after* the commit point.
    //
    // This is the one place every body arrives — background backfill and the
    // interactive fetch of whatever the user just opened both settle through
    // here — which is why the call belongs here and not in a scheduler that
    // sees only some of them. `index_body` had existed and been tested since
    // the index was written and nothing in the workspace ever called it, so
    // the `body` column was empty on every message ever synced (#327).
    //
    // After rather than before: an index entry for a body that is not local
    // yet would let search answer for a corpus it does not have, and nothing
    // could detect it. The other order — indexed but not committed — is a row
    // the maintenance pass simply picks up again.
    //
    // Never fatal. A store whose search schema was never created is a real
    // state (a headless sync, a test that only wants mail), and trading a
    // fetched message for an unavailable index would be the wrong way round.
    // `postio_session::index_local_bodies` sweeps up whatever this misses.
    if let Err(error) =
        postio_index::index::index_body_of(connection, request.message.get(), &parsed.body)
    {
        tracing::debug!(
            message = request.message.get(),
            %error,
            "a fetched body did not reach the search index"
        );
    }
    Ok(Outcome::Stored { bytes })
}

/// Fetches only the sections holding a message's own text, and decodes them.
///
/// The other half of [`fetch_body`]; see its documentation for why this exists.
///
/// # Rebuilding an entity from a section
///
/// `BODY[1.1]` returns a part's *encoded* bytes and none of its headers, so
/// nothing in the response says whether they are base64 or what charset they
/// are in. Both were reported by `BODYSTRUCTURE` and kept on the row
/// ([`Message::text_part_headers`]), so prepending them turns the fetched
/// section back into a self-contained entity the parser can decode — without
/// spending a second round trip on `BODY[1.1.MIME]`.
///
/// # No blob store at all
///
/// There is no raw blob to keep — the whole message was never fetched, which
/// is the point, and is what retires the 1.3x duplication of storing the raw
/// bytes and their decoded forms side by side. "View source" and
/// forward-as-`message/rfc822` refetch on demand. And since ADR 0020 the
/// decoded text is a column rather than a file, so this path touches the blob
/// store nowhere: it is text in, row out.
async fn fetch_text_parts(
    connection: &Connection,
    blobs: &BlobStore,
    backend: &dyn MailBackend,
    request: &BodyRequest,
    mut message: postio_model::Message,
    inline_cap: Option<u64>,
    cancel: &CancelToken,
) -> Result<Outcome> {
    let messages = MessageRepository::new(connection);
    let wanted = [
        (
            message.text_part_id.clone(),
            message.text_part_headers.clone(),
        ),
        (
            message.html_part_id.clone(),
            message.html_part_headers.clone(),
        ),
    ];

    let mut body = postio_model::MessageBody::default();
    let mut preview = None;
    let mut bytes = 0u64;

    for (section, headers) in wanted {
        let Some(section) = section else { continue };
        let mut sink = VecSink::new();
        backend
            .fetch_part(
                &request.path,
                &request.remote_id,
                &BodyPart::Section(section),
                &mut sink,
                cancel,
            )
            .await?;
        if !sink.is_finished() {
            // The sink contract, same as the whole-message path: without
            // `finish` the bytes are a fragment, and a fragment stored as a
            // body is worse than no body.
            return Err(SyncError::Backend(BackendError::Cancelled));
        }
        let raw = sink.into_inner();
        bytes += raw.len() as u64;

        let mut entity = headers.unwrap_or_default().into_bytes();
        entity.extend_from_slice(b"\r\n");
        entity.extend_from_slice(&raw);
        // Same ingest boundary as the whole-message path (#277): ids and sizes
        // only, because the bytes that defeated the parser are somebody's mail.
        let parsed = mime::try_parse(&entity).unwrap_or_else(|_| {
            tracing::warn!(
                message = request.message.get(),
                bytes = entity.len(),
                "a fetched text part could not be parsed; storing what it yielded"
            );
            mime::parse(&entity)
        });
        if body.text.is_none() {
            body.text = parsed.body.text;
        }
        if body.html.is_none() {
            body.html = parsed.body.html;
        }
        preview = preview.or(parsed.preview);
    }

    // ADR 0017's "inline parts ride with the text". A `cid:` image is not an
    // attachment the reader offers to download, it is part of the sentence the
    // message is making: without it the pane draws a broken box, and since the
    // reader blocks remote images by default these are the images that are
    // *supposed* to appear (#751).
    //
    // Before the commit point on purpose. `needing_backfill` stops at
    // `partial`, so a message whose text landed is never offered again — and a
    // half-finished inline pass committed as `partial` would leave those boxes
    // broken for good. Failing here instead leaves the message `headers_only`,
    // and the next seed fetches the whole text axis again, images included.
    for part_id in inline_with_the_text(&message, inline_cap) {
        let Some(attachment) = message
            .attachments
            .iter()
            .find(|part| part.part_id.as_deref() == Some(part_id.as_str()))
        else {
            continue;
        };
        let Some((blob, moved)) =
            fetch_section(blobs, backend, request, attachment, &part_id, cancel).await?
        else {
            continue;
        };
        bytes += moved;
        messages.set_attachment_blob(request.message, &part_id, &blob)?;
        // Kept in step with the row, so `state_for` below reads the truth
        // rather than the structure as the header sync left it.
        if let Some(attachment) = message
            .attachments
            .iter_mut()
            .find(|part| part.part_id.as_deref() == Some(part_id.as_str()))
        {
            attachment.blob_id = Some(blob);
        }
    }

    let stored = StoredBody {
        text: stored_text(body.text.as_deref()),
        html: stored_text(body.html.as_deref()),
        headers: None,
    };

    if message.preview.is_none() {
        message.preview = preview;
    }
    messages.update(&mut message)?;

    // `partial` means text local, payloads not — the variant the schema
    // declared and nothing had ever written until ADR 0017 gave it a meaning.
    // It is what tells the reader to offer "download" rather than "open".
    let state = state_for(&message.attachments);

    // The commit point, after which the body is local as far as anything else
    // is concerned. See `fetch_body` for why it is last.
    messages.set_body(request.message, &stored, state)?;

    if let Err(error) = postio_index::index::index_body_of(connection, request.message.get(), &body)
    {
        tracing::debug!(
            message = request.message.get(),
            %error,
            "a fetched body did not reach the search index"
        );
    }
    Ok(Outcome::Stored { bytes })
}

/// Fetches one named MIME section and stores its decoded bytes.
///
/// The step both axes share since #751: the payload axis has always done this,
/// and the text axis now does it too for the inline parts an HTML body
/// references. Returns the blob and how many bytes crossed the wire, or `None`
/// when there was nothing to store.
///
/// # Rebuilding an entity from a section
///
/// `BODY[2]` returns a part's *encoded* bytes and none of its headers, so
/// nothing in the response says whether they are base64. `BODYSTRUCTURE` said
/// so at header-sync time and `Attachment::part_headers` kept the answer, so
/// prepending it makes a self-contained entity — no `[2.MIME]` round trip.
///
/// # Why the id is taken on the decoded bytes
///
/// Content addressing gives dedup for free, but only over a form two copies of
/// the same file actually share. Base64 does not qualify: the same PDF wrapped
/// at a different line length is different bytes. Decoded, it is the same file,
/// and on the reference account 22,878 named parts collapse to 13,099 distinct
/// — 10.96 GB to 7.69 GB.
///
/// It is also what makes an eager fetch and an on-open fetch land *identically*
/// rather than giving one attachment two blob ids.
async fn fetch_section(
    blobs: &BlobStore,
    backend: &dyn MailBackend,
    request: &BodyRequest,
    attachment: &postio_model::Attachment,
    part_id: &str,
    cancel: &CancelToken,
) -> Result<Option<(postio_model::BlobId, u64)>> {
    if attachment.blob_id.is_some() {
        // It landed while this was queued — a second chip clicked on the same
        // part, an eager pass that got there first, or the text axis having
        // already carried an inline part down with the body. Not worth a round
        // trip for bytes already on the disk.
        return Ok(None);
    }
    let Some(headers) = attachment.part_headers.clone() else {
        // Refused rather than guessed. See `request_payloads`: this should
        // have been escalated to a whole-message fetch before it got here.
        tracing::debug!(
            message = request.message.get(),
            "a part with no recorded encoding was not fetched"
        );
        return Ok(None);
    };

    let mut sink = VecSink::new();
    backend
        .fetch_part(
            &request.path,
            &request.remote_id,
            &BodyPart::Section(part_id.to_owned()),
            &mut sink,
            cancel,
        )
        .await?;
    if !sink.is_finished() {
        // The sink contract, same as every other path here: without `finish`
        // the bytes are a fragment, and half a file written to disk under a
        // name that says it is whole is worse than no file.
        return Err(SyncError::Backend(BackendError::Cancelled));
    }
    let raw = sink.into_inner();
    let bytes = raw.len() as u64;

    let mut entity = headers.into_bytes();
    entity.extend_from_slice(b"\r\n");
    entity.extend_from_slice(&raw);
    let Some(decoded) = mime::decode_entity(&entity) else {
        // The ingest boundary of #277 again: ids and sizes only, because the
        // bytes that defeated the decoder are somebody's mail.
        tracing::warn!(
            message = request.message.get(),
            bytes = entity.len(),
            "a fetched part could not be decoded; leaving it on the server"
        );
        return Ok(None);
    };

    Ok(Some((blobs.put(&decoded)?, bytes)))
}

/// Fetches named payload sections and records where each one landed.
///
/// The payload axis of ADR 0017, and the first thing in this codebase ever to
/// write `attachments.blob_id` on the receive path. Before it that column was
/// filled only on the way *out*, by a composer attaching a file, so
/// `Attachment::is_downloaded` was false for every message that had ever
/// arrived from a server and `postio_app::reading::part_bytes` re-parsed the
/// whole raw message to cut one part out of it.
///
/// # Rebuilding an entity from a section
///
/// The same trick [`fetch_text_parts`] uses, for the same reason: `BODY[2]`
/// returns a part's *encoded* bytes and none of its headers, so nothing in the
/// response says whether they are base64. `BODYSTRUCTURE` said so at
/// header-sync time and `Attachment::part_headers` kept the answer, so
/// prepending it makes a self-contained entity — no `[2.MIME]` round trip.
///
/// # Why the id is taken on the decoded bytes
///
/// Content addressing gives dedup for free, but only over a form two copies of
/// the same file actually share. Base64 does not qualify: the same PDF wrapped
/// at a different line length is different bytes. Decoded, it is the same file,
/// and on the reference account 22,878 named parts collapse to 13,099 distinct
/// — 10.96 GB to 7.69 GB.
///
/// It is also what makes an eager fetch and an on-open fetch land *identically*
/// rather than giving one attachment two blob ids.
///
/// # No raw blob
///
/// Nothing to keep: the message around the part was never fetched. That is the
/// point.
async fn fetch_payloads(
    connection: &Connection,
    blobs: &BlobStore,
    backend: &dyn MailBackend,
    request: &BodyRequest,
    message: &postio_model::Message,
    parts: &[String],
    cancel: &CancelToken,
) -> Result<Outcome> {
    let messages = MessageRepository::new(connection);
    let mut bytes = 0u64;

    for part_id in parts {
        let Some(attachment) = message
            .attachments
            .iter()
            .find(|attachment| attachment.part_id.as_deref() == Some(part_id.as_str()))
        else {
            // The structure moved under the request while it sat in the queue.
            // Settled rather than failed, as `Outcome::Gone` is for a message.
            continue;
        };
        let Some((blob, moved)) =
            fetch_section(blobs, backend, request, attachment, part_id, cancel).await?
        else {
            continue;
        };
        bytes += moved;
        messages.set_attachment_blob(request.message, part_id, &blob)?;
    }

    // The commit point for this axis. Re-read rather than reasoned about: the
    // rows above were written one at a time, and what matters here is whether
    // *every* part is local now — including ones this request never asked for.
    if let Some(settled) = messages.get(request.message)? {
        let state = state_for(&settled.attachments);
        if settled.sync.body_state != state {
            messages.set_body_state(request.message, state)?;
        }
    }

    Ok(Outcome::Stored { bytes })
}

/// The sections of `message` that ride down the text axis with its body.
///
/// ADR 0017's rule, and the whole of #751's first cause: an inline part under
/// `cap` is text, a larger one is a payload. `None` disables the rule, leaving
/// every part on the payload axis.
///
/// Inline *and* nothing else. A named attachment is a payload however little
/// it weighs, or `attachment_fetch = "on_open"` would stop meaning anything;
/// what qualifies here is a part the sender marked for display inside the
/// body, which in practice is a `cid:` image the HTML points at.
///
/// The size is the server's declared one, taken from `BODYSTRUCTURE` at header
/// sync — the only figure available before the bytes move, which is the whole
/// point of having a cap.
fn inline_with_the_text(message: &postio_model::Message, cap: Option<u64>) -> Vec<String> {
    let Some(cap) = cap else {
        return Vec::new();
    };
    message
        .attachments
        .iter()
        .filter(|part| part.is_inline() && !part.is_downloaded() && part.size <= cap)
        .filter_map(|part| part.part_id.clone())
        .collect()
}

/// How much of a message is local, given what its parts have.
///
/// `full` means every byte; `partial` means the words are here and something
/// hanging off them is not. That distinction is what #352 needs in order to
/// tell the user when search is answering for an incomplete corpus, and what
/// the attachment chip needs in order to say "download" rather than "open".
fn state_for(attachments: &[postio_model::Attachment]) -> BodyState {
    if attachments.iter().all(|part| part.blob_id.is_some()) {
        BodyState::Full
    } else {
        BodyState::Partial
    }
}

/// One decoded body form as it goes into the row, folding an empty one to
/// absent.
///
/// `Some("")` and `None` are different facts to `StoredBody`, and here they
/// are not: a `text/plain`-only message parses to an HTML part of zero length,
/// and storing that would tell the reading pane the message has an HTML
/// alternative that renders to nothing.
pub(crate) fn stored_text(text: Option<&str>) -> Option<String> {
    match text {
        Some(text) if !text.is_empty() => Some(text.to_owned()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(second: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 1, 9, 0, 0).unwrap() + chrono::TimeDelta::seconds(second)
    }

    fn request(uid: u32, size: u64) -> BodyRequest {
        BodyRequest {
            remote_id: postio_model::RemoteId::new(format!("1:{uid}")),
            message: MessageId::new(uid as i64),
            mailbox: MailboxId::new(1),
            path: "INBOX".to_owned(),
            uid: Uid::new(uid),
            size,
            received_at: at(uid as i64),
            want: Want::Text,
        }
    }

    #[test]
    fn the_default_cap_is_generous_enough_for_ordinary_mail() {
        let cap = BackfillPolicy::default()
            .max_body_bytes
            .expect("a default cap");

        assert!(
            cap >= 1024 * 1024,
            "a cap that refuses an ordinary message with images is a cap that \
             leaves the mailbox unreadable offline"
        );
    }

    #[test]
    fn the_heap_orders_newest_first() {
        let mut heap = BinaryHeap::new();
        for uid in [2, 5, 1] {
            heap.push(Entry(request(uid, 1)));
        }

        let order: Vec<u32> = std::iter::from_fn(|| heap.pop())
            .map(|Entry(request)| request.uid.get())
            .collect();

        assert_eq!(order, vec![5, 2, 1]);
    }

    #[test]
    fn messages_received_in_the_same_instant_still_have_a_total_order() {
        let mut first = request(1, 1);
        let mut second = request(2, 1);
        first.received_at = at(0);
        second.received_at = at(0);

        assert_ne!(Entry(first).cmp(&Entry(second)), Ordering::Equal);
    }

    #[test]
    fn an_empty_body_form_is_stored_as_absent() {
        // A `text/plain`-only message parses to an HTML part of zero length.
        // Storing that would tell the reading pane there is an HTML
        // alternative which renders to nothing.
        assert_eq!(stored_text(None), None);
        assert_eq!(stored_text(Some("")), None);
        assert_eq!(stored_text(Some("hello")).as_deref(), Some("hello"));
    }

    #[test]
    fn reporting_an_outcome_for_something_never_claimed_changes_nothing() {
        let mut backfill = Backfill::new(BackfillPolicy::default());

        backfill.finished(MessageId::new(404), Outcome::Stored { bytes: 10 });

        assert_eq!(backfill.progress(), BackfillProgress::default());
    }

    #[test]
    fn a_cancelled_backfill_accepts_no_new_work() {
        let mut backfill = Backfill::new(BackfillPolicy::default());
        backfill.cancel();

        backfill.enqueue(request(1, 1));
        backfill.request_now(request(2, 1));

        assert!(backfill.next_body().is_none());
        assert_eq!(backfill.progress().pending, 0);
    }

    // -----------------------------------------------------------------------
    // Filling the queue from storage
    // -----------------------------------------------------------------------

    fn headers_only(
        account: postio_model::AccountId,
        mailbox: MailboxId,
        seconds: i64,
        uid: u32,
    ) -> postio_model::Message {
        let mut message = postio_model::Message::new(account, mailbox, at(seconds));
        message.server.uid = Some(Uid::new(uid));
        message.server.remote_id = Some(postio_model::RemoteId::new(format!("1:{uid}")));
        message.sync.body_state = BodyState::HeadersOnly;
        message
    }

    #[test]
    fn seed_queues_everything_a_mailbox_is_missing_newest_first() {
        let database = postio_storage::test_support::memory();
        let connection = database.connection().expect("checkout");
        let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection);
        let messages = MessageRepository::new(&connection);

        for (seconds, uid) in [(1, 1), (2, 2), (3, 3)] {
            messages
                .create(&mut headers_only(account.id, inbox, seconds, uid))
                .expect("create");
        }

        let mut backfill = Backfill::new(BackfillPolicy::default());
        let queued = seed(&connection, &mut backfill, inbox, 10).expect("seed");

        assert_eq!(queued, 3);
        assert_eq!(backfill.progress().pending, 3);
        let claim = backfill.next_body().expect("a claim");
        assert_eq!(claim.request.uid, Uid::new(3), "newest first");
        assert_eq!(claim.priority, Priority::Background);
    }

    #[test]
    fn seed_leaves_the_backlog_empty_when_nothing_is_missing() {
        let database = postio_storage::test_support::memory();
        let connection = database.connection().expect("checkout");
        let (_account, inbox) = postio_storage::test_support::account_with_inbox(&connection);

        let mut backfill = Backfill::new(BackfillPolicy::default());
        assert_eq!(
            seed(&connection, &mut backfill, inbox, 10).expect("seed"),
            0
        );
        assert!(backfill.next_body().is_none());
    }

    #[test]
    fn request_body_jumps_the_queue_for_whatever_the_reading_pane_opened() {
        let database = postio_storage::test_support::memory();
        let connection = database.connection().expect("checkout");
        let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection);
        let messages = MessageRepository::new(&connection);

        let mut older = headers_only(account.id, inbox, 1, 1);
        messages.create(&mut older).expect("create");
        let mut newer = headers_only(account.id, inbox, 2, 2);
        messages.create(&mut newer).expect("create");

        let mut backfill = Backfill::new(BackfillPolicy::default());
        seed(&connection, &mut backfill, inbox, 10).expect("seed");

        // The user opened the OLDER message; the interactive lane must still
        // put it ahead of the newer one the background lane would fetch first.
        assert!(request_body(&connection, &mut backfill, older.id).expect("lookup"));

        let claim = backfill.next_body().expect("a claim");
        assert_eq!(claim.priority, Priority::Interactive);
        assert_eq!(claim.request.message, older.id);
    }

    #[test]
    fn request_body_for_a_message_with_nothing_to_fetch_is_a_no_op() {
        let database = postio_storage::test_support::memory();
        let connection = database.connection().expect("checkout");
        let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection);
        let messages = MessageRepository::new(&connection);

        let mut fully_fetched = headers_only(account.id, inbox, 1, 1);
        fully_fetched.sync.body_state = BodyState::Full;
        messages.create(&mut fully_fetched).expect("create");

        let mut backfill = Backfill::new(BackfillPolicy::default());

        assert!(!request_body(&connection, &mut backfill, fully_fetched.id).expect("lookup"));
        assert!(
            !request_body(&connection, &mut backfill, MessageId::new(404)).expect("lookup"),
            "no local row at all is the same answer, not an error"
        );
        assert!(backfill.next_body().is_none());
    }
}
