//! Body backfill scheduler: newest first, in the background, never ahead of a message the user
//! opened.
//! The interactive lane always wins: it ignores the size cap, metered and pause switches.
//! No persisted queue: `body_state` on the message row is the durable record; no task or timer
//! here.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};

use chrono::{DateTime, Utc};
use postio_account::backend::{BackendError, BodyPart, MailBackend, VecSink};
use postio_account::cancel::CancelToken;
use postio_model::fetch::{PayloadAsk, Sections};
use postio_model::{BodyState, MailboxId, MessageId, Uid, mime};
use postio_storage::BlobStore;
use postio_storage::repository::{
    BackfillCandidate, MailboxRepository, MessageRepository, StoredBody,
};
use postio_storage::{Checkout, Connection, WritePriority};

use crate::blob_sink::BlobSink;
use crate::drain::SyncError;

pub type Result<T> = std::result::Result<T, SyncError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackfillPolicy {
    /// A message the user opens is still fetched when this is off.
    pub background: bool,
    /// The interactive lane ignores this cap.
    pub max_body_bytes: Option<u64>,
    pub pause_on_metered: bool,
    pub pause_when_active: bool,
    /// A batch, not a horizon: the engine re-seeds whenever the queue drains.
    pub seed_batch: u32,
    /// Most background requests held in memory across all mailboxes (ADR 0017).
    /// Overflow is refused, not evicted: `body_state` is the durable record and a re-seed re-reads
    /// it.
    pub max_backlog: usize,
    /// Largest inline part carried with the text axis, so CID images render offline.
    /// `None` leaves every inline part on the payload axis.
    pub max_inline_bytes: Option<u64>,
    pub attachments: AttachmentPolicy,
    /// Attachment bytes fetched unasked for a just-arrived message; `None` is no limit.
    /// Not applied under `AttachmentPolicy::Never` or when a metered connection pauses the
    /// background lane.
    pub fresh_payload_bytes: Option<u64>,
}

/// When an attachment's bytes are downloaded (ADR 0017's payload axis).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AttachmentPolicy {
    /// The default. Part metadata is still synced, so `has:attachment` and `filename:` search work.
    #[default]
    OnOpen,
    Eager,
    /// The reader says the bytes are not here, rather than failing silently.
    Never,
}

impl Default for BackfillPolicy {
    /// Backfill up to five megabytes a message, so the background lane skips huge attachments.
    fn default() -> Self {
        Self {
            background: true,
            max_body_bytes: Some(5 * 1024 * 1024),
            pause_on_metered: true,
            pause_when_active: true,
            seed_batch: 200,
            max_backlog: 4_000,
            max_inline_bytes: Some(256 * 1024),
            attachments: AttachmentPolicy::default(),
            // 25 MB: the large providers' attachment limit.
            fresh_payload_bytes: Some(25 * 1024 * 1024),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    Interactive,
    Fresh,
    Background,
}

/// Section bytes fetched in one batched command ahead of the messages that want them (#1551).
/// A miss is not an error: the per-message path fetches it.
#[derive(Debug, Default)]
pub struct Prefetched {
    sections: HashMap<(postio_model::RemoteId, String), Vec<u8>>,
    headers: HashMap<postio_model::RemoteId, Vec<u8>>,
}

impl Prefetched {
    fn take(&mut self, id: &postio_model::RemoteId, section: &str) -> Option<Vec<u8>> {
        self.sections.remove(&(id.clone(), section.to_owned()))
    }

    fn take_headers(&mut self, id: &postio_model::RemoteId) -> Option<Vec<u8>> {
        self.headers.remove(id)
    }

    /// Taken before the fetch: the window's fetches share one cache concurrently, and a borrow held
    /// across an await would panic.
    pub fn split_for(&mut self, id: &postio_model::RemoteId) -> Prefetched {
        let mine: Vec<_> = self
            .sections
            .keys()
            .filter(|(owner, _)| owner == id)
            .cloned()
            .collect();
        let mut split = Prefetched::default();
        for key in mine {
            if let Some(bytes) = self.sections.remove(&key) {
                split.sections.insert(key, bytes);
            }
        }
        if let Some(bytes) = self.headers.remove(id) {
            split.headers.insert(id.clone(), bytes);
        }
        split
    }

    pub fn len(&self) -> usize {
        self.sections.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sections.is_empty() && self.headers.is_empty()
    }
}

/// Bytes of fetched-ahead sections held at once; a batch is in memory whole.
/// Measured against `BodyRequest::text_size`, so a short note with a big PDF is not priced as the
/// PDF.
const PREFETCH_BUDGET: u64 = 8 * 1024 * 1024;

/// A set is a command line, and a command line has a length limit.
const PREFETCH_BATCH: usize = 50;

pub const BATCH_LEN: usize = PREFETCH_BATCH;

/// Half of `PREFETCH_BUDGET`, so the batch being written and the one being fetched fit together.
pub const BATCH_BYTES: u64 = PREFETCH_BUDGET / 2;

/// Fetches text sections grouped by `(path, section)`, one `UID FETCH` per group.
/// Best-effort: any miss falls back to the per-message path in `fetch_body`.
pub async fn prefetch_text_sections(
    connection: &Checkout,
    backend: &dyn MailBackend,
    requests: &[BodyRequest],
    cancel: &CancelToken,
) -> Prefetched {
    let mut prefetched = Prefetched::default();
    if requests.len() < 2 {
        return prefetched;
    }

    let messages = MessageRepository::new(connection);
    let mut groups: HashMap<(String, String), Vec<postio_model::RemoteId>> = HashMap::new();
    let mut contents = HashSet::new();
    let mut budget = PREFETCH_BUDGET;
    let mut named = HashSet::new();
    for request in requests {
        if !matches!(request.want, Want::Text) {
            continue;
        }
        if request.text_size > budget {
            continue;
        }
        let Ok(Some(message)) = messages.get(request.message).await else {
            continue;
        };
        if let Some(identity) = &message.server.content_identity
            && (messages
                .has_reusable_content(request.message)
                .await
                .unwrap_or(false)
                || !contents.insert((message.account_id, identity.clone())))
        {
            continue;
        }
        let sections = [message.text_part_id.clone(), message.html_part_id.clone()];
        let mut wanted = false;
        for section in sections.into_iter().flatten() {
            let group = groups.entry((request.path.clone(), section)).or_default();
            if group.len() < PREFETCH_BATCH {
                group.push(request.remote_id.clone());
                wanted = true;
            }
        }
        if wanted {
            budget = budget.saturating_sub(request.text_size);
            named.insert(request.remote_id.clone());
        }
    }

    for ((path, section), ids) in groups {
        if cancel.is_cancelled() || ids.len() < 2 {
            continue;
        }
        let part = BodyPart::Section(section.clone());
        match backend.fetch_sections(&path, &ids, &part, cancel).await {
            Ok(answered) => {
                for (id, bytes) in answered {
                    prefetched.sections.insert((id, section.clone()), bytes);
                }
            }
            // Best-effort: a failed batch must not fail the bodies it was helping.
            Err(error) => {
                tracing::debug!(%path, %section, %error, "a batched section fetch failed");
            }
        }
    }

    // Header blocks of the same messages, one command per mailbox; otherwise one round trip per
    // message.
    let mut by_path: HashMap<&str, Vec<postio_model::RemoteId>> = HashMap::new();
    for request in requests {
        if named.remove(&request.remote_id) {
            by_path
                .entry(request.path.as_str())
                .or_default()
                .push(request.remote_id.clone());
        }
    }
    for (path, ids) in by_path {
        if cancel.is_cancelled() || ids.len() < 2 {
            continue;
        }
        match backend
            .fetch_sections(path, &ids, &BodyPart::Headers, cancel)
            .await
        {
            Ok(answered) => prefetched.headers.extend(answered),
            Err(error) => {
                tracing::debug!(%path, %error, "a batched header fetch failed");
            }
        }
    }
    prefetched
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyRequest {
    pub message: MessageId,
    pub mailbox: MailboxId,
    pub path: String,
    pub uid: Uid,
    pub remote_id: postio_model::RemoteId,
    pub size: u64,
    /// Bytes fetching its text costs: `size` less the attachment parts. The cap is measured against
    /// this for a text request.
    pub text_size: u64,
    /// Mailbox position in the sync queue (`order::sync_priority`); lower is fetched sooner and
    /// outranks `received_at`.
    /// The backlog is one heap across mailboxes, so date alone would let a huge archive outrank the
    /// inbox.
    pub rank: u8,
    pub received_at: DateTime<Utc>,
    pub want: Want,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Want {
    #[default]
    Text,
    Payloads(Vec<String>),
    /// Used when `BODYSTRUCTURE` was never recorded or a payload has no `part_headers`.
    Whole,
    /// Repair lane (#884): fetches `BODY[HEADER]` for rows whose local body has no stored header
    /// block.
    /// Not done at sync time (ADR 0025 Q4).
    HeaderBlock,
}

impl BodyRequest {
    pub fn fetched_bytes(&self) -> u64 {
        match self.want {
            Want::Text => self.text_size,
            _ => self.size,
        }
    }
}

impl Want {
    pub fn sections(&self) -> Sections {
        match self {
            Want::Text => Sections::Text,
            Want::Payloads(parts) => Sections::Parts(parts.clone()),
            Want::Whole => Sections::Whole,
            Want::HeaderBlock => Sections::Headers,
        }
    }

    /// Payload requests union their parts; anything else widens to the whole message.
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
            text_size: candidate.text_size,
            rank: crate::order::sync_priority(candidate.mailbox_role),
            received_at: candidate.received_at,
            want: Want::Text,
        }
    }
}

/// Outstanding until reported through `Backfill::finished`, so the same body is not put on the wire
/// twice.
#[derive(Debug, Clone)]
pub struct Claim {
    pub request: BodyRequest,
    pub priority: Priority,
    pub cancel: CancelToken,
}

/// First backoff before an interrupted body is offered again; doubles per failure up to
/// `RETRY_CAP`.
const RETRY_BASE: std::time::Duration = std::time::Duration::from_secs(5);

const RETRY_CAP: std::time::Duration = std::time::Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Stored {
        bytes: u64,
    },
    /// Settled, not failed: the message or its mailbox (`UIDVALIDITY` reset) is gone.
    Gone,
    Failed {
        reason: String,
    },
    /// Not about the message (dropped link, timeout, cancel, `UIDVALIDITY` moved): offered again
    /// after a backoff, and at once on reconnect (#1985).
    Interrupted {
        reason: String,
    },
}

/// Every message ever queued is in exactly one count.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BackfillProgress {
    pub pending: usize,
    pub in_flight: usize,
    pub stored: usize,
    pub skipped: usize,
    pub gone: usize,
    pub failed: usize,
    pub bytes: u64,
}

impl BackfillProgress {
    pub fn settled(&self) -> usize {
        self.stored + self.skipped + self.gone + self.failed
    }
}

/// Also the lazy-deletion marker: a heap entry whose message is no longer `Background` is skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lane {
    Background,
    Fresh,
    Interactive,
    InFlight,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry(BodyRequest);

impl Ord for Entry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Max-heap: greater means fetch sooner. Rank is reversed (lower `sync_priority` is sooner)
        // and first,
        // or newer mail in an archive outranks the inbox.
        other
            .0
            .rank
            .cmp(&self.0.rank)
            .then_with(|| self.0.received_at.cmp(&other.0.received_at))
            .then_with(|| self.0.uid.get().cmp(&other.0.uid.get()))
            .then_with(|| other.0.message.get().cmp(&self.0.message.get()))
    }
}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug)]
pub struct Backfill {
    policy: BackfillPolicy,
    interactive: VecDeque<BodyRequest>,
    fresh: VecDeque<BodyRequest>,
    /// Attachment follow-ups, queued on `fresh` once the text is stored.
    followups: HashMap<MessageId, BodyRequest>,
    background: BinaryHeap<Entry>,
    lanes: HashMap<MessageId, Lane>,
    progress: BackfillProgress,
    metered: bool,
    disk_full: bool,
    user_active: bool,
    cancel: CancelToken,
    cancelled: bool,
    /// Messages not to be offered again this session: over the size cap, or failed.
    /// They stay `body_state <> 'full'`, so every re-seed would offer them again.
    set_aside: HashSet<MessageId>,
    /// Interrupted messages: failures in a row, and the earliest they may be offered again.
    /// Kept apart from `set_aside` (session-long): these wait out a backoff, and `reconnected`
    /// forgives them.
    interrupted: HashMap<MessageId, (u32, std::time::Instant)>,
    /// Requests that arrived while the same message was on the wire, re-offered when it settles.
    /// One lane per message bounds how long anything waits behind the backfill; dropping the
    /// request would leave a spinner turning.
    deferred: HashMap<MessageId, BodyRequest>,
}

impl Backfill {
    pub fn new(policy: BackfillPolicy) -> Self {
        Self {
            policy,
            interactive: VecDeque::new(),
            fresh: VecDeque::new(),
            followups: HashMap::new(),
            background: BinaryHeap::new(),
            lanes: HashMap::new(),
            progress: BackfillProgress::default(),
            metered: false,
            disk_full: false,
            user_active: false,
            cancel: CancelToken::new(),
            cancelled: false,
            set_aside: HashSet::new(),
            interrupted: HashMap::new(),
            deferred: HashMap::new(),
        }
    }

    pub fn policy(&self) -> BackfillPolicy {
        self.policy
    }

    pub fn progress(&self) -> BackfillProgress {
        self.progress
    }

    pub fn is_idle(&self) -> bool {
        self.progress.pending == 0 && self.progress.in_flight == 0
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    pub fn set_metered(&mut self, metered: bool) {
        self.metered = metered;
    }

    pub fn set_user_active(&mut self, active: bool) {
        self.user_active = active;
    }

    /// A full store with the lane running would loop: backfill fetches, eviction frees, backfill
    /// fetches again.
    /// The interactive lane is exempt; opening mail is how the store learns which blobs to keep.
    pub fn set_disk_full(&mut self, full: bool) {
        self.disk_full = full;
    }

    /// Whether the background lane is held back for any reason (#352): "not fetching" and "nothing
    /// left" differ.
    pub fn is_paused(&self) -> bool {
        !self.background_runs()
    }

    /// Queues a body for background fetch; answers whether it joined the queue.
    /// A body over the size cap is counted as skipped once and not queued. A known or set-aside
    /// message is ignored.
    /// The answer lets the engine's top-up stop looping (#318).
    pub fn enqueue(&mut self, request: BodyRequest) -> bool {
        self.enqueue_at(request, std::time::Instant::now())
    }

    pub fn enqueue_at(&mut self, request: BodyRequest, now: std::time::Instant) -> bool {
        if self.cancelled
            || self.lanes.contains_key(&request.message)
            || self.set_aside.contains(&request.message)
            || self
                .interrupted
                .get(&request.message)
                .is_some_and(|(_, retry_at)| now < *retry_at)
        {
            return false;
        }
        if self
            .policy
            .max_body_bytes
            .is_some_and(|cap| request.fetched_bytes() > cap)
        {
            self.progress.skipped += 1;
            self.set_aside.insert(request.message);
            return false;
        }
        // Full, not `set_aside`: the message is fetchable and the next `seed` offers it again.
        if self.background.len() >= self.policy.max_backlog {
            return false;
        }
        // At the disk ceiling: queueing would fetch bytes that eviction must immediately free.
        if !self.background_runs() && self.disk_full {
            return false;
        }
        self.lanes.insert(request.message, Lane::Background);
        self.background.push(Entry(request));
        self.progress.pending += 1;
        true
    }

    /// Asks for a body now, because the user is looking at it; subject to none of the policy.
    /// A queued message is promoted, a queued interactive one absorbs this request's `Want`,
    /// and one on the wire holds it until that fetch settles.
    pub fn request_now(&mut self, request: BodyRequest) {
        if self.cancelled {
            return;
        }
        match self.lanes.get(&request.message) {
            // On the wire: a running fetch cannot grow a part, so wait; `finished` offers it again.
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
            Some(Lane::Background | Lane::Fresh) => {
                // The stale entry is skipped on its way out; the count does not move.
                self.lanes.insert(request.message, Lane::Interactive);
            }
            None => {
                self.lanes.insert(request.message, Lane::Interactive);
                self.progress.pending += 1;
            }
        }
        self.interactive.push_back(request);
    }

    /// Queues a just-arrived message: its text, then its attachments once the text is stored.
    /// Skips the user-active pauses (new mail is wanted then) but honours `background` off and a
    /// full disk.
    /// The text is exempt from the size cap; a whole-message fallback is not.
    pub fn arrived(&mut self, text: Option<BodyRequest>, payloads: Option<BodyRequest>) -> bool {
        if self.cancelled || !self.policy.background || self.disk_full {
            return false;
        }
        let Some(message) = text
            .as_ref()
            .or(payloads.as_ref())
            .map(|request| request.message)
        else {
            return false;
        };
        let text = text.filter(|request| {
            request.want == Want::Text
                || !self
                    .policy
                    .max_body_bytes
                    .is_some_and(|cap| request.fetched_bytes() > cap)
        });
        let Some(text) = text else {
            return payloads.is_some_and(|payloads| self.queue_fresh(payloads));
        };
        match self.lanes.get(&message) {
            Some(Lane::InFlight | Lane::Interactive | Lane::Fresh) => {
                if let Some(payloads) = payloads {
                    self.followups.insert(message, payloads);
                }
                false
            }
            Some(Lane::Background) | None => {
                self.set_aside.remove(&message);
                self.interrupted.remove(&message);
                if let Some(payloads) = payloads {
                    self.followups.insert(message, payloads);
                }
                self.queue_fresh(text)
            }
        }
    }

    fn queue_fresh(&mut self, request: BodyRequest) -> bool {
        match self.lanes.get(&request.message) {
            Some(Lane::InFlight | Lane::Interactive | Lane::Fresh) => return false,
            Some(Lane::Background) => {}
            None => self.progress.pending += 1,
        }
        self.lanes.insert(request.message, Lane::Fresh);
        self.fresh.push_back(request);
        true
    }

    /// One at a time, outstanding until `finished`: the user never waits behind more than the body
    /// already on the wire.
    pub fn next_body(&mut self) -> Option<Claim> {
        if self.cancelled {
            return None;
        }

        if let Some(claim) = self.next_interactive_body() {
            return Some(claim);
        }
        if !self.background_runs() {
            return None;
        }
        let request = self.take_background()?;
        Some(self.claim(request, Priority::Background))
    }

    /// A claim for a message the user is waiting on (then fresh mail), or `None`.
    /// The engine tops up its background window with this so an opened message starts at the next
    /// free slot (#1551).
    pub fn next_interactive_body(&mut self) -> Option<Claim> {
        if self.cancelled {
            return None;
        }
        if let Some(request) = self.take_interactive() {
            return Some(self.claim(request, Priority::Interactive));
        }
        let request = self.take_fresh()?;
        Some(self.claim(request, Priority::Fresh))
    }

    /// The next batch to put on the wire together (#1941).
    /// A batch is one kind of work: if the interactive or fresh lanes have anything, only that, so
    /// an opened message does not wait behind background bodies.
    /// Otherwise background bodies until `BATCH_LEN` or `BATCH_BYTES`, and always at least one.
    pub fn next_batch(&mut self) -> Vec<Claim> {
        let mut batch = Vec::new();
        while batch.len() < BATCH_LEN
            && let Some(claim) = self.next_interactive_body()
        {
            batch.push(claim);
        }
        if !batch.is_empty() {
            return batch;
        }
        let mut bytes = 0u64;
        while batch.len() < BATCH_LEN && bytes < BATCH_BYTES {
            let Some(claim) = self.next_body() else {
                break;
            };
            bytes = bytes.saturating_add(claim.request.fetched_bytes());
            batch.push(claim);
        }
        batch
    }

    /// Hands a claim back unfetched, to the front of its lane, so "outstanding" keeps meaning "on
    /// the wire or about to be".
    pub fn release(&mut self, claim: Claim) {
        let message = claim.request.message;
        if self.lanes.get(&message) != Some(&Lane::InFlight) {
            return;
        }
        self.progress.in_flight = self.progress.in_flight.saturating_sub(1);
        self.progress.pending += 1;
        if self.cancelled {
            self.lanes.remove(&message);
            self.progress.pending = self.progress.pending.saturating_sub(1);
            return;
        }
        match claim.priority {
            Priority::Background => {
                self.lanes.insert(message, Lane::Background);
                self.background.push(Entry(claim.request));
            }
            Priority::Interactive => {
                self.lanes.insert(message, Lane::Interactive);
                self.interactive.push_front(claim.request);
            }
            Priority::Fresh => {
                self.lanes.insert(message, Lane::Fresh);
                self.fresh.push_front(claim.request);
            }
        }
    }

    pub fn finished(&mut self, message: MessageId, outcome: Outcome) {
        self.finished_at(message, outcome, std::time::Instant::now());
    }

    pub fn finished_at(&mut self, message: MessageId, outcome: Outcome, now: std::time::Instant) {
        if self.lanes.remove(&message) != Some(Lane::InFlight) {
            return;
        }
        self.progress.in_flight = self.progress.in_flight.saturating_sub(1);
        match outcome {
            Outcome::Stored { bytes } => {
                self.interrupted.remove(&message);
                self.progress.stored += 1;
                self.progress.bytes += bytes;
                if let Some(payloads) = self.followups.remove(&message) {
                    self.queue_fresh(payloads);
                }
            }
            // Set aside: the row stays `body_state <> 'full'`, so without this every re-seed would
            // re-queue it in a tight retry loop.
            Outcome::Gone => {
                self.followups.remove(&message);
                self.progress.gone += 1;
                self.set_aside.insert(message);
            }
            Outcome::Failed { .. } => {
                self.followups.remove(&message);
                self.progress.failed += 1;
                self.set_aside.insert(message);
            }
            Outcome::Interrupted { .. } => {
                self.followups.remove(&message);
                self.progress.failed += 1;
                let tries = self
                    .interrupted
                    .get(&message)
                    .map_or(0, |(tries, _)| *tries);
                let wait = RETRY_BASE.saturating_mul(1 << tries.min(16)).min(RETRY_CAP);
                self.interrupted.insert(message, (tries + 1, now + wait));
            }
        }
        // Offered even after a failure: a request made by hand earns an attempt a speculative one
        // would not.
        if let Some(held) = self.deferred.remove(&message) {
            self.request_now(held);
        }
    }

    /// The queue is emptied, not parked: it is re-derivable from `body_state`.
    pub fn cancel(&mut self) {
        self.cancelled = true;
        self.cancel.cancel();
        self.interactive.clear();
        self.fresh.clear();
        self.followups.clear();
        self.background.clear();
        self.lanes.clear();
        self.deferred.clear();
        self.progress.pending = 0;
        self.progress.in_flight = 0;
    }

    /// Link is up: interrupted messages are offered again now, whatever is left of their backoff.
    pub fn reconnected(&mut self) {
        self.interrupted.clear();
    }

    /// The counts are kept.
    pub fn restart(&mut self) {
        self.cancelled = false;
        self.cancel = CancelToken::new();
    }

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

    /// Attachments stay queued while a metered connection pauses the background lane; text goes
    /// regardless.
    fn take_fresh(&mut self) -> Option<BodyRequest> {
        let held_back = self.policy.pause_on_metered && self.metered;
        let position = self.fresh.iter().position(|request| {
            self.lanes.get(&request.message) == Some(&Lane::Fresh)
                && !(held_back && matches!(request.want, Want::Payloads(_)))
        })?;
        let request = self.fresh.remove(position)?;
        self.fresh
            .retain(|queued| self.lanes.get(&queued.message) == Some(&Lane::Fresh));
        Some(request)
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

/// Queues what in `mailbox_id` still lacks a body, newest first, rebuilding the in-memory backlog
/// from `body_state`.
/// Returns how many requests joined the queue, not how many rows were read: the engine's top-up
/// repeats until it gets zero (#318).
/// Queues nothing for a folder excluded from backfill (ADR 0016, #350); on-open fetches still work
/// there.
pub async fn seed(
    connection: &Connection,
    backfill: &mut Backfill,
    mailbox_id: MailboxId,
    limit: u32,
) -> Result<usize> {
    if MailboxRepository::new(connection)
        .backfill_excluded(mailbox_id)
        .await?
    {
        return Ok(0);
    }
    let messages = MessageRepository::new(connection);
    let mut offset = 0;
    loop {
        let candidates = messages
            .needing_backfill_from(mailbox_id, limit, offset)
            .await?;
        let read = candidates.len();
        let queued = candidates
            .into_iter()
            .filter(|candidate| backfill.enqueue(candidate.clone().into()))
            .count();
        // A batch that queued nothing (over the cap, or set aside) stays `body_state <> 'full'`;
        // walk past it to reach older mail (#318).
        if queued > 0 || read < limit as usize {
            return Ok(queued);
        }
        offset += limit;
    }
}

/// Asks the backfill for one message right now. Returns `false` when there is nothing to fetch.
pub async fn request_body(
    connection: &Connection,
    backfill: &mut Backfill,
    message_id: MessageId,
) -> Result<bool> {
    let Some(candidate) = MessageRepository::new(connection)
        .backfill_candidate(message_id)
        .await?
    else {
        return Ok(false);
    };
    backfill.request_now(candidate.into());
    Ok(true)
}

/// As `request_body`, but asks for every byte, for callers needing the original bytes (drag out,
/// view source).
/// The text axis stores no raw source (ADR 0017), so whether the raw blob is stored decides, not
/// `body_state` (#1984).
/// Returns `false` only when it is stored, or the message is gone or has no `UID`.
pub async fn request_whole(
    connection: &Connection,
    backfill: &mut Backfill,
    message_id: MessageId,
) -> Result<bool> {
    let Some(candidate) = MessageRepository::new(connection)
        .whole_candidate(message_id)
        .await?
    else {
        return Ok(false);
    };
    let mut request = BodyRequest::from(candidate);
    request.want = Want::Whole;
    backfill.request_now(request);
    Ok(true)
}

/// Asks for named payload parts of one message now, by section, because the user opened one (ADR
/// 0017).
/// The answer says why nothing was queued, because each reason is a different sentence for the user
/// (#1996).
/// A part without recorded `part_headers` escalates to `Want::Whole`: a section without them cannot
/// be decoded.
pub async fn request_payloads(
    connection: &Connection,
    backfill: &mut Backfill,
    message_id: MessageId,
    parts: &[String],
) -> Result<PayloadAsk> {
    if backfill.policy().attachments == AttachmentPolicy::Never {
        return Ok(PayloadAsk::Never);
    }
    let messages = MessageRepository::new(connection);
    let Some(message) = messages.get(message_id).await? else {
        return Ok(PayloadAsk::Gone);
    };
    if message.sync.deleted_locally {
        return Ok(PayloadAsk::Gone);
    }

    let mut wanted = Vec::new();
    let mut needs_every_byte = false;
    let mut found = false;
    for part_id in parts {
        let Some(attachment) = message
            .attachments
            .iter()
            .find(|attachment| attachment.part_id.as_deref() == Some(part_id.as_str()))
        else {
            continue;
        };
        found = true;
        if attachment.blob_id.is_some() {
            continue;
        }
        if attachment.part_headers.is_none() {
            needs_every_byte = true;
        }
        wanted.push(part_id.clone());
    }
    if !found {
        return Ok(PayloadAsk::Gone);
    }
    if wanted.is_empty() {
        return Ok(PayloadAsk::Local);
    }
    let Some(candidate) = messages.backfill_candidate(message_id).await? else {
        return Ok(
            if message.server.uid.is_none() || message.server.remote_id.is_none() {
                PayloadAsk::NotOnServer
            } else {
                PayloadAsk::Local
            },
        );
    };

    let mut request = BodyRequest::from(candidate);
    request.want = if needs_every_byte {
        Want::Whole
    } else {
        Want::Payloads(wanted)
    };
    backfill.request_now(request);
    Ok(PayloadAsk::Queued)
}

/// Queues a just-arrived message on the fresh lane: its text, then the attachments that fit
/// `fresh_payload_bytes`.
/// For incremental-pass arrivals, not a first sync's backlog. An excluded folder is excluded here
/// too.
pub async fn request_arrived(
    connection: &Connection,
    backfill: &mut Backfill,
    message_id: MessageId,
) -> Result<bool> {
    let messages = MessageRepository::new(connection);
    let Some(candidate) = messages.backfill_candidate(message_id).await? else {
        return Ok(false);
    };
    if MailboxRepository::new(connection)
        .backfill_excluded(candidate.mailbox_id)
        .await?
    {
        return Ok(false);
    }
    let Some(message) = messages.get(message_id).await? else {
        return Ok(false);
    };
    let policy = backfill.policy();
    let text_local = matches!(
        message.sync.body_state,
        postio_model::BodyState::Partial | postio_model::BodyState::Full
    );

    let mut payloads = None;
    if policy.attachments != AttachmentPolicy::Never {
        let mut room = policy.fresh_payload_bytes.unwrap_or(u64::MAX);
        let mut wanted = Vec::new();
        for attachment in &message.attachments {
            if attachment.blob_id.is_some() || attachment.part_headers.is_none() {
                continue;
            }
            // An inline part under the cap rides with the text (ADR 0017); asking again would fetch
            // it twice.
            let rides_with_text = !text_local
                && attachment.disposition == postio_model::Disposition::Inline
                && policy
                    .max_inline_bytes
                    .is_some_and(|cap| attachment.size <= cap);
            let Some(part) = attachment.part_id.clone() else {
                continue;
            };
            if rides_with_text || attachment.size > room {
                continue;
            }
            room -= attachment.size;
            wanted.push(part);
        }
        if !wanted.is_empty() {
            let mut request = BodyRequest::from(candidate.clone());
            request.want = Want::Payloads(wanted);
            payloads = Some(request);
        }
    }
    let text = (!text_local).then(|| {
        let mut request = BodyRequest::from(candidate);
        if message.content_type.is_none() {
            request.want = Want::Whole;
        }
        request
    });
    Ok(backfill.arrived(text, payloads))
}

/// Queues a header-block fetch for rows that can get one no other way: `partial` rows and rows
/// whose raw source was evicted (#884).
/// Background priority; it stops finding work once the blocks are stored.
pub async fn seed_header_blocks(
    connection: &Connection,
    backfill: &mut Backfill,
    mailbox_id: MailboxId,
    limit: u32,
) -> Result<usize> {
    if MailboxRepository::new(connection)
        .backfill_excluded(mailbox_id)
        .await?
    {
        return Ok(0);
    }
    let messages = MessageRepository::new(connection);
    let mut queued = 0;
    for candidate in messages
        .messages_needing_a_header_fetch(mailbox_id, limit)
        .await?
    {
        let mut request = BodyRequest::from(candidate);
        request.want = Want::HeaderBlock;
        if backfill.enqueue(request) {
            queued += 1;
        }
    }
    Ok(queued)
}

/// Queues payloads for up to `limit` text-backfilled messages; only called under
/// `AttachmentPolicy::Eager`.
/// Walks past an unusable batch as `seed` does (#318).
pub async fn seed_payloads(
    connection: &Connection,
    backfill: &mut Backfill,
    mailbox_id: MailboxId,
    limit: u32,
) -> Result<usize> {
    if MailboxRepository::new(connection)
        .backfill_excluded(mailbox_id)
        .await?
    {
        return Ok(0);
    }
    let messages = MessageRepository::new(connection);
    let mut offset = 0;
    loop {
        let candidates = messages
            .needing_payloads_from(mailbox_id, limit, offset)
            .await?;
        let read = candidates.len();
        let mut queued = 0;
        for candidate in candidates {
            let message_id = candidate.message_id;
            let Some(message) = messages.get(message_id).await? else {
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

/// Sections whose bytes are not local. A part with no `part_id` or no `part_headers` is skipped,
/// not escalated: nobody waits for it.
fn pending_payloads(message: &postio_model::Message) -> Vec<String> {
    message
        .attachments
        .iter()
        .filter(|attachment| attachment.blob_id.is_none() && attachment.part_headers.is_some())
        .filter_map(|attachment| attachment.part_id.clone())
        .collect()
}

/// Fetches one message's body and stores it (ADR 0017).
/// Raw bytes go to the blob store verbatim so a later parser fix applies to mail already
/// downloaded.
/// `body_state` is written last: a crash earlier leaves orphan blobs (swept) and a message that is
/// fetched again.
/// Fetches text sections when the header sync recorded them, else the whole message.
/// A refusal about the message marks it failed (#1918) and returns `Outcome::Failed`;
/// a dropped link, timeout or cancel is returned as an error, unmarked.
pub async fn fetch_body(
    connection: &Checkout,
    blobs: &BlobStore,
    backend: &dyn MailBackend,
    request: &BodyRequest,
    inline_cap: Option<u64>,
    prefetched: Option<&mut Prefetched>,
    cancel: &CancelToken,
) -> Result<Outcome> {
    let fetched = fetch_body_unmarked(
        connection, blobs, backend, request, inline_cap, prefetched, cancel,
    )
    .await;
    let Err(SyncError::Backend(error)) = &fetched else {
        return fetched;
    };
    // The body is what failed; a payload or header block that would not come leaves a good body
    // behind.
    if !matches!(request.want, Want::Text | Want::Whole) || !refusal_about_the_message(error) {
        return fetched;
    }
    let reason = error.to_string();
    {
        let _permit = connection
            .write_gate()
            .acquire(WritePriority::Background)
            .await;
        MessageRepository::new(connection)
            .mark_body_failed(request.message)
            .await?;
    }
    tracing::debug!(
        message = request.message.get(),
        "the server would not give up a body; it is marked until the message changes"
    );
    Ok(Outcome::Failed { reason })
}

/// Whether the server is answering about this message, not the link, login or a cancel.
fn refusal_about_the_message(error: &BackendError) -> bool {
    matches!(
        error,
        BackendError::NoSuchMessage { .. }
            | BackendError::Rejected { .. }
            | BackendError::Protocol { .. }
    )
}

async fn fetch_body_unmarked(
    connection: &Checkout,
    blobs: &BlobStore,
    backend: &dyn MailBackend,
    request: &BodyRequest,
    inline_cap: Option<u64>,
    prefetched: Option<&mut Prefetched>,
    cancel: &CancelToken,
) -> Result<Outcome> {
    let messages = MessageRepository::new(connection);
    let Some(message) = messages.get(request.message).await? else {
        // Deleted or wiped by a UIDVALIDITY reset while queued; checked first so a stale queue
        // costs no bandwidth.
        return Ok(Outcome::Gone);
    };

    if matches!(request.want, Want::Text)
        && message.server.content_identity.is_some()
        && messages.has_reusable_content(request.message).await?
        && !message.attachments.iter().any(|part| {
            part.blob_id.is_none()
                && part.is_inline()
                && inline_cap.is_some_and(|cap| part.size <= cap)
        })
    {
        return Ok(Outcome::Stored { bytes: 0 });
    }

    match &request.want {
        // Native whole-source adapters have no section map, so a parsed MIME path cannot be fetched
        // on its own.
        Want::Payloads(parts)
            if message.server.content_identity.is_some() && message.content_type.is_none() =>
        {
            let missing = message.attachments.iter().any(|part| {
                part.blob_id.is_none() && part.part_id.as_ref().is_some_and(|id| parts.contains(id))
            });
            if !missing {
                return Ok(Outcome::Stored { bytes: 0 });
            }
        }
        Want::Payloads(parts) => {
            let parts = parts.clone();
            return fetch_payloads(
                connection, blobs, backend, request, &message, &parts, cancel,
            )
            .await;
        }
        // `content_type` says a `BODYSTRUCTURE` was parsed; a message that is only an attachment
        // must still take this path.
        Want::Text if message.content_type.is_some() => {
            return fetch_text_parts(
                connection, blobs, backend, request, message, inline_cap, prefetched, cancel,
            )
            .await;
        }
        // Does not touch the body: re-storing the words would risk them for nothing.
        Want::HeaderBlock => {
            let block = fetch_header_block(backend, request, cancel).await;
            let Some(block) = block else {
                // Set aside like any failed fetch; an empty block would claim the message has no
                // such header.
                return Ok(Outcome::Failed {
                    reason: "the header block could not be fetched".to_owned(),
                });
            };
            let bytes = block.text.len() as u64;
            let _permit = connection
                .write_gate()
                .acquire(WritePriority::Background)
                .await;
            messages.set_headers(request.message, Some(&block)).await?;
            index_the_header_block(connection, request.message, Some(&block)).await;
            return Ok(Outcome::Stored { bytes });
        }
        Want::Text | Want::Whole => {}
    }

    // Straight to disk, not a doubling `Vec` (ADR 0017): the interactive lane ignores
    // `max_body_bytes`, so this path sees the oversized messages.
    let mut sink = BlobSink::new(blobs)?;
    backend
        .fetch_body(&request.path, &request.remote_id, &mut sink, cancel)
        .await?;
    let Some(blob) = sink.finished_blob() else {
        // Without `finish` the bytes are a fragment, and a fragment stored as a message is worse
        // than none.
        return Err(SyncError::Backend(BackendError::Cancelled));
    };
    let bytes = sink.bytes();

    // Read back to parse: one exact-size allocation, and the bytes are durable before the parser
    // sees them.
    let raw = blobs.get(&blob)?;
    // The ingest boundary (#277): without it a message that defeats the parser stores no body and
    // looks like one that had none.
    // Ids and sizes only: the bytes are somebody's mail.
    let parsed = mime::try_parse(&raw).unwrap_or_else(|_| {
        tracing::warn!(
            message = request.message.get(),
            bytes,
            "a fetched body could not be parsed; storing it with no body parts"
        );
        mime::parse(&raw)
    });

    // Not read back off `raw_blob_id` at query time: eviction takes raw source first (PRODUCT.md
    // §6), which would silently break `header:` on old mail.
    let block = postio_model::headers::block_of(&raw);
    let stored = StoredBody {
        text: stored_text(parsed.body.text.as_deref()),
        html: stored_text(parsed.body.html.as_deref()),
        headers: block.as_ref().map(|block| block.text.clone()),
        headers_truncated: block.as_ref().is_some_and(|block| block.truncated),
        encoding_problems: parsed.encoding_problems,
    };

    // Everything below writes and nothing touches the wire, so the gate is taken after the fetch
    // and held to the end.
    let _permit = connection
        .write_gate()
        .acquire(WritePriority::Background)
        .await;

    // Native adapters may give an immutable identity without BODYSTRUCTURE: keep the parsed part
    // map.
    // Read current location state under the gate; flags may have changed while the body was on the
    // wire.
    if message.server.content_identity.is_some() && message.content_type.is_none() {
        let Some(mut current) = messages.get(request.message).await? else {
            return Ok(Outcome::Gone);
        };
        // `content_type` marks a complete text section map; keep it absent so an evicted inline
        // part falls back to the whole source.
        current.text_is_flowed = parsed.text_is_flowed;
        current.read_receipt_requested = parsed.read_receipt_requested;
        current.attachments = parsed
            .parts
            .iter()
            .map(|part| part.attachment.clone())
            .collect();
        messages.update(&mut current).await?;
    }

    messages
        .set_fetched(request.message, parsed.preview.as_deref(), Some(&blob))
        .await?;

    // Record where each payload landed, matched by MIME path: the attachment rows came from
    // `BODYSTRUCTURE`, this parse is an independent reading.
    // The blob is content-addressed, so the same part fetched by section is the same blob.
    for part in &parsed.parts {
        let Some(part_id) = part.attachment.part_id.as_deref() else {
            continue;
        };
        let blob = blobs.put(&part.content)?;
        messages
            .set_attachment_blob(request.message, part_id, &blob)
            .await?;
    }

    // The commit point. `Full` unconditionally: parts not matched to a row are still in the raw
    // blob.
    messages
        .set_body(request.message, &stored, BodyState::Full)
        .await?;

    // Not indexed here: a body without a `message_search_bodies` row is the indexer's queue, and
    // `spawn_body_indexer` drains it in batches.
    // Indexing per body put a segment merge on the sync lane (#327).
    index_the_header_block(connection, request.message, block.as_ref()).await;
    Ok(Outcome::Stored { bytes })
}

/// Indexes a stored header block into `message_headers`, after the commit point and never fatally.
/// This is where blocks arrive, so `header:` works for mail that arrives while the app is open.
async fn index_the_header_block(
    connection: &Checkout,
    message: MessageId,
    block: Option<&postio_model::headers::Block>,
) {
    let Some(block) = block else {
        // No block fetched: an empty row set would claim the message has no such header.
        return;
    };
    let headers = postio_model::headers::parse_block(&block.text);
    if let Err(error) =
        postio_index::index::index_headers(connection, message.get(), &headers).await
    {
        tracing::debug!(
            message = message.get(),
            %error,
            "a fetched header block did not reach the search index"
        );
    }
}

/// Fetches only the sections holding a message's own text, and decodes them. No raw blob is kept;
/// the text is a column (ADR 0020).
/// `BODY[1.1]` returns encoded bytes without headers, so the part headers kept on the row are
/// prepended to rebuild a decodable entity.
#[allow(clippy::too_many_arguments)]
async fn fetch_text_parts(
    connection: &Checkout,
    blobs: &BlobStore,
    backend: &dyn MailBackend,
    request: &BodyRequest,
    mut message: postio_model::Message,
    inline_cap: Option<u64>,
    mut prefetched: Option<&mut Prefetched>,
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
    // Accumulated across sections: a message is a guess if any of its parts was (#901).
    let mut encoding_problems = false;
    let mut bytes = 0u64;

    for (section, headers) in wanted {
        let Some(section) = section else { continue };
        // Fetched ahead with its batch, if it was (#1551); a miss costs only the round trip.
        let batched = prefetched
            .as_mut()
            .and_then(|cache| cache.take(&request.remote_id, &section));
        let raw = match batched {
            Some(bytes) => bytes,
            None => {
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
                    // Without `finish` the bytes are a fragment.
                    return Err(SyncError::Backend(BackendError::Cancelled));
                }
                sink.into_inner()
            }
        };
        bytes += raw.len() as u64;

        let mut entity = headers.unwrap_or_default().into_bytes();
        entity.extend_from_slice(b"\r\n");
        entity.extend_from_slice(&raw);
        // Same ingest boundary as the whole-message path (#277): ids and sizes only.
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
        encoding_problems |= parsed.encoding_problems;
    }

    // Inline parts ride with the text (ADR 0017, #751): `cid:` images must appear since remote
    // images are blocked; a small calendar part feeds the invitation row.
    // Before the commit point: `needing_backfill` stops at `partial`, so a half-done inline pass
    // committed there would leave broken boxes for good.
    for part_id in inline_with_the_text(&message, inline_cap)
        .into_iter()
        .chain(calendar_with_the_text(&message))
    {
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
        messages
            .set_attachment_blob(request.message, &part_id, &blob)
            .await?;
        if let Some(attachment) = message
            .attachments
            .iter_mut()
            .find(|part| part.part_id.as_deref() == Some(part_id.as_str()))
        {
            attachment.blob_id = Some(blob);
        }
    }

    // This path stores no raw blob, so nothing else would bring the block and `header:` would miss
    // mail that is here.
    // Body-fetch time, unlike header-sync time (ADR 0025 Q4), already pays the round trip.
    let block = match prefetched
        .as_mut()
        .and_then(|cache| cache.take_headers(&request.remote_id))
    {
        Some(bytes) => postio_model::headers::block_of(&bytes),
        None => fetch_header_block(backend, request, cancel).await,
    };
    let stored = StoredBody {
        text: stored_text(body.text.as_deref()),
        html: stored_text(body.html.as_deref()),
        headers: block.as_ref().map(|block| block.text.clone()),
        headers_truncated: block.as_ref().is_some_and(|block| block.truncated),
        encoding_problems,
    };

    // Write phase: all store from here, so the gate is taken once and held to the end.
    let _permit = connection
        .write_gate()
        .acquire(WritePriority::Background)
        .await;

    messages
        .set_fetched(request.message, preview.as_deref(), None)
        .await?;

    // `partial` means text local, payloads not; the reader offers "download" rather than "open".
    let state = state_for(&message.attachments);

    messages.set_body(request.message, &stored, state).await?;

    index_the_header_block(connection, request.message, block.as_ref()).await;
    Ok(Outcome::Stored { bytes })
}

/// Fetches one named MIME section and stores its decoded bytes (shared by both axes, #751).
/// Returns the blob and bytes on the wire, or `None` when there was nothing to store.
/// `BODY[2]` returns encoded bytes with no headers, so the stored `part_headers` are prepended to
/// make a decodable entity.
/// The blob id is taken on the decoded bytes so the same file wrapped differently dedups, and eager
/// and on-open fetches land identically.
async fn fetch_header_block(
    backend: &dyn MailBackend,
    request: &BodyRequest,
    cancel: &CancelToken,
) -> Option<postio_model::headers::Block> {
    let mut sink = VecSink::new();
    let fetched = backend
        .fetch_part(
            &request.path,
            &request.remote_id,
            &BodyPart::Headers,
            &mut sink,
            cancel,
        )
        .await;
    if let Err(error) = fetched {
        tracing::debug!(
            message = request.message.get(),
            %error,
            "the header block did not come with the payloads"
        );
        return None;
    }
    if !sink.is_finished() {
        return None;
    }
    postio_model::headers::block_of(&sink.into_inner())
}

pub(crate) async fn fetch_section(
    blobs: &BlobStore,
    backend: &dyn MailBackend,
    request: &BodyRequest,
    attachment: &postio_model::Attachment,
    part_id: &str,
    cancel: &CancelToken,
) -> Result<Option<(postio_model::BlobId, u64)>> {
    if attachment.blob_id.is_some() {
        // Already local: no round trip for bytes on disk.
        return Ok(None);
    }
    let Some(headers) = attachment.part_headers.clone() else {
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
        // Without `finish` the bytes are a fragment, and half a file under a whole file's name is
        // worse than none.
        return Err(SyncError::Backend(BackendError::Cancelled));
    }
    let raw = sink.into_inner();
    let bytes = raw.len() as u64;

    let mut entity = headers.into_bytes();
    entity.extend_from_slice(b"\r\n");
    entity.extend_from_slice(&raw);
    let Some(decoded) = mime::decode_entity(&entity) else {
        // Ingest boundary (#277): ids and sizes only.
        tracing::warn!(
            message = request.message.get(),
            bytes = entity.len(),
            "a fetched part could not be decoded; leaving it on the server"
        );
        return Ok(None);
    };

    Ok(Some((blobs.put(&decoded)?, bytes)))
}

/// Fetches named payload sections and records where each landed in `attachments.blob_id` (ADR
/// 0017's payload axis).
/// No raw blob is kept: the message around the part is never fetched.
async fn fetch_payloads(
    connection: &Checkout,
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
            // The structure moved while queued: settled, not failed.
            continue;
        };
        let Some((blob, moved)) =
            fetch_section(blobs, backend, request, attachment, part_id, cancel).await?
        else {
            continue;
        };
        bytes += moved;
        {
            let _permit = connection
                .write_gate()
                .acquire(WritePriority::Background)
                .await;
            messages
                .set_attachment_blob(request.message, part_id, &blob)
                .await?;
        }
    }

    let _permit = connection
        .write_gate()
        .acquire(WritePriority::Background)
        .await;

    // Commit point for this axis: re-read, since what matters is whether every part is local,
    // including ones not asked for.
    if let Some(settled) = messages.get(request.message).await? {
        let state = state_for(&settled.attachments);
        if settled.sync.body_state != state {
            messages.set_body_state(request.message, state).await?;
        }
    }

    Ok(Outcome::Stored { bytes })
}

/// Sections that ride down the text axis: inline parts of at most `cap` bytes (declared
/// `BODYSTRUCTURE` size); `None` disables.
/// Named attachments are payloads however small.
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

/// Largest `text/calendar` part the text axis carries; larger is a payload.
pub(crate) const CALENDAR_PART_CAP: u64 = 256 * 1024;

/// `text/calendar` parts carried with the text, up to `CALENDAR_PART_CAP`: the invitation row reads
/// its times from them with no network.
fn calendar_with_the_text(message: &postio_model::Message) -> Vec<String> {
    message
        .attachments
        .iter()
        .filter(|part| {
            part.mime_type.eq_ignore_ascii_case("text/calendar")
                && !part.is_downloaded()
                && part.size <= CALENDAR_PART_CAP
        })
        .filter_map(|part| part.part_id.clone())
        .collect()
}

/// `full` means every byte; `partial` means the words are here and some part is not (#352).
pub(crate) fn state_for(attachments: &[postio_model::Attachment]) -> BodyState {
    if attachments.iter().all(|part| part.blob_id.is_some()) {
        BodyState::Full
    } else {
        BodyState::Partial
    }
}

/// Folds an empty body form to absent: an empty HTML part would tell the reader there is an HTML
/// alternative.
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
            text_size: size,
            rank: crate::order::sync_priority(postio_model::MailboxRole::Inbox),
            received_at: at(uid as i64),
            want: Want::Text,
        }
    }

    fn from_archive(uid: u32, received: i64) -> BodyRequest {
        let mut request = request(uid, 1);
        request.mailbox = MailboxId::new(2);
        request.path = "Archive".to_owned();
        request.rank = crate::order::sync_priority(postio_model::MailboxRole::Archive);
        request.received_at = at(received);
        request
    }

    #[test]
    fn an_inbox_body_is_fetched_before_a_newer_one_from_the_archive() {
        // Seeding walks mailboxes in `sync_priority` order but the backlog is one heap: by date
        // alone an archive outranks the inbox.
        let mut heap = BinaryHeap::new();
        heap.push(Entry(from_archive(9, 500))); // newer, but archived
        heap.push(Entry(request(1, 1))); // older, but in the inbox

        let first = heap.pop().expect("a body").0;

        assert_eq!(
            first.path, "INBOX",
            "the archive outranked the inbox on date, which is how an inbox \
             stays unreadable while a large folder backfills"
        );
    }

    #[test]
    fn within_one_folder_the_newest_body_still_comes_first() {
        let mut heap = BinaryHeap::new();
        heap.push(Entry(from_archive(1, 100)));
        heap.push(Entry(from_archive(2, 900)));

        let first = heap.pop().expect("a body").0;

        assert_eq!(first.uid.get(), 2, "the older archived body came first");
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

    #[tokio::test]
    async fn seed_queues_everything_a_mailbox_is_missing_newest_first() {
        let database = postio_storage::test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection).await;
        let messages = MessageRepository::new(&connection);

        for (seconds, uid) in [(1, 1), (2, 2), (3, 3)] {
            messages
                .create(&mut headers_only(account.id, inbox, seconds, uid))
                .await
                .expect("create");
        }

        let mut backfill = Backfill::new(BackfillPolicy::default());
        let queued = seed(&connection, &mut backfill, inbox, 10)
            .await
            .expect("seed");

        assert_eq!(queued, 3);
        assert_eq!(backfill.progress().pending, 3);
        let claim = backfill.next_body().expect("a claim");
        assert_eq!(claim.request.uid, Uid::new(3), "newest first");
        assert_eq!(claim.priority, Priority::Background);
    }

    #[tokio::test]
    async fn seed_leaves_the_backlog_empty_when_nothing_is_missing() {
        let database = postio_storage::test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let (_account, inbox) = postio_storage::test_support::account_with_inbox(&connection).await;

        let mut backfill = Backfill::new(BackfillPolicy::default());
        assert_eq!(
            seed(&connection, &mut backfill, inbox, 10)
                .await
                .expect("seed"),
            0
        );
        assert!(backfill.next_body().is_none());
    }

    #[tokio::test]
    async fn request_body_jumps_the_queue_for_whatever_the_reading_pane_opened() {
        let database = postio_storage::test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection).await;
        let messages = MessageRepository::new(&connection);

        let mut older = headers_only(account.id, inbox, 1, 1);
        messages.create(&mut older).await.expect("create");
        let mut newer = headers_only(account.id, inbox, 2, 2);
        messages.create(&mut newer).await.expect("create");

        let mut backfill = Backfill::new(BackfillPolicy::default());
        seed(&connection, &mut backfill, inbox, 10)
            .await
            .expect("seed");

        assert!(
            request_body(&connection, &mut backfill, older.id)
                .await
                .expect("lookup")
        );

        let claim = backfill.next_body().expect("a claim");
        assert_eq!(claim.priority, Priority::Interactive);
        assert_eq!(claim.request.message, older.id);
    }

    #[tokio::test]
    async fn an_interactive_top_up_takes_interactive_work_and_nothing_else() {
        let database = postio_storage::test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection).await;
        let messages = MessageRepository::new(&connection);

        let mut created = Vec::new();
        for (seconds, uid) in [(1, 1), (2, 2), (3, 3)] {
            let mut message = headers_only(account.id, inbox, seconds, uid);
            messages.create(&mut message).await.expect("create");
            created.push(message.id);
        }

        let mut backfill = Backfill::new(BackfillPolicy::default());
        seed(&connection, &mut backfill, inbox, 10)
            .await
            .expect("seed");

        assert!(
            backfill.next_interactive_body().is_none(),
            "a background queue is not interactive work"
        );

        assert!(
            request_body(&connection, &mut backfill, created[0])
                .await
                .expect("lookup")
        );
        let claim = backfill
            .next_interactive_body()
            .expect("the opened message is interactive work");
        assert_eq!(claim.priority, Priority::Interactive);
        assert_eq!(claim.request.message, created[0], "the one the user opened");

        assert!(
            backfill.next_interactive_body().is_none(),
            "the same interactive claim must not be handed out twice"
        );
    }

    #[tokio::test]
    async fn request_body_for_a_message_with_nothing_to_fetch_is_a_no_op() {
        let database = postio_storage::test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection).await;
        let messages = MessageRepository::new(&connection);

        let mut fully_fetched = headers_only(account.id, inbox, 1, 1);
        fully_fetched.sync.body_state = BodyState::Full;
        messages.create(&mut fully_fetched).await.expect("create");

        let mut backfill = Backfill::new(BackfillPolicy::default());

        assert!(
            !request_body(&connection, &mut backfill, fully_fetched.id)
                .await
                .expect("lookup")
        );
        assert!(
            !request_body(&connection, &mut backfill, MessageId::new(404))
                .await
                .expect("lookup"),
            "no local row at all is the same answer, not an error"
        );
        assert!(backfill.next_body().is_none());
    }

    fn payloads(uid: u32) -> BodyRequest {
        let mut request = request(uid, 6_000_000);
        request.want = Want::Payloads(vec!["2".to_owned()]);
        request
    }

    #[test]
    fn an_arrival_goes_behind_a_click_and_ahead_of_the_backlog() {
        let mut backfill = Backfill::new(BackfillPolicy::default());
        backfill.enqueue(request(1, 10));
        backfill.request_now(request(2, 10));
        assert!(backfill.arrived(Some(request(3, 10)), None));

        let order: Vec<_> = std::iter::from_fn(|| backfill.next_body())
            .map(|claim| (claim.request.uid.get(), claim.priority))
            .collect();
        assert_eq!(
            order,
            [
                (2, Priority::Interactive),
                (3, Priority::Fresh),
                (1, Priority::Background),
            ]
        );
    }

    #[test]
    fn an_arrival_is_not_held_back_by_the_user_being_active() {
        let mut backfill = Backfill::new(BackfillPolicy::default());
        backfill.set_user_active(true);
        backfill.arrived(Some(request(1, 10)), None);
        assert_eq!(
            backfill.next_body().map(|claim| claim.priority),
            Some(Priority::Fresh)
        );
    }

    #[test]
    fn the_cap_is_measured_against_the_text_that_is_fetched() {
        let mut note = request(1, 6_000_000);
        note.text_size = 2_000;
        let mut backfill = Backfill::new(BackfillPolicy::default());
        assert!(backfill.enqueue(note.clone()), "its words are small");

        note.want = Want::Whole;
        let mut backfill = Backfill::new(BackfillPolicy::default());
        assert!(
            !backfill.enqueue(note),
            "fetched whole it is six megabytes, and the cap stands"
        );
    }

    #[test]
    fn an_arrival_s_attachments_follow_its_text() {
        let mut backfill = Backfill::new(BackfillPolicy::default());
        backfill.arrived(Some(request(1, 6_000_000)), Some(payloads(1)));

        let text = backfill.next_body().expect("the text first");
        assert_eq!(text.request.want, Want::Text);
        assert!(
            backfill.next_body().is_none(),
            "the attachments wait for the words"
        );
        backfill.finished(text.request.message, Outcome::Stored { bytes: 10 });

        let attachments = backfill.next_body().expect("then the attachments");
        assert_eq!(attachments.priority, Priority::Fresh);
        assert_eq!(
            attachments.request.want,
            Want::Payloads(vec!["2".to_owned()])
        );
    }

    #[test]
    fn an_arrival_s_attachments_wait_out_a_metered_connection() {
        let mut backfill = Backfill::new(BackfillPolicy::default());
        backfill.set_metered(true);
        backfill.arrived(None, Some(payloads(1)));
        backfill.arrived(Some(request(2, 10)), None);
        assert_eq!(
            backfill.next_body().map(|claim| claim.request.uid.get()),
            Some(2),
            "text goes on a metered connection; attachments do not"
        );
        assert!(backfill.next_body().is_none());

        backfill.set_metered(false);
        assert_eq!(
            backfill.next_body().map(|claim| claim.request.uid.get()),
            Some(1)
        );
    }

    #[test]
    fn an_arrival_fetches_nothing_when_background_fetching_is_off() {
        let policy = BackfillPolicy {
            background: false,
            ..BackfillPolicy::default()
        };
        let mut backfill = Backfill::new(policy);
        assert!(!backfill.arrived(Some(request(1, 10)), Some(payloads(1))));
        assert!(backfill.next_body().is_none());
    }

    #[test]
    fn a_failed_text_drops_the_attachments_that_were_to_follow_it() {
        let mut backfill = Backfill::new(BackfillPolicy::default());
        backfill.arrived(Some(request(1, 10)), Some(payloads(1)));
        let text = backfill.next_body().expect("the text");
        backfill.finished(
            text.request.message,
            Outcome::Failed {
                reason: "gone".to_owned(),
            },
        );
        assert!(backfill.next_body().is_none());
    }

    #[test]
    fn an_interrupted_body_waits_out_a_backoff_and_is_then_offered_again() {
        let mut backfill = Backfill::new(BackfillPolicy::default());
        let start = std::time::Instant::now();
        assert!(backfill.enqueue_at(request(1, 10), start));
        let claim = backfill.next_body().expect("the body");
        backfill.finished_at(
            claim.request.message,
            Outcome::Interrupted {
                reason: "timed out".to_owned(),
            },
            start,
        );

        assert!(
            !backfill.enqueue_at(request(1, 10), start + RETRY_BASE / 2),
            "a link that fails every fetch fast is not hammered"
        );
        assert!(
            backfill.enqueue_at(request(1, 10), start + RETRY_BASE),
            "and the wait ends"
        );
    }

    #[test]
    fn each_interruption_in_a_row_waits_longer() {
        let mut backfill = Backfill::new(BackfillPolicy::default());
        let start = std::time::Instant::now();
        for _ in 0..2 {
            assert!(backfill.enqueue_at(request(1, 10), start + RETRY_CAP));
            let claim = backfill.next_body().expect("the body");
            backfill.finished_at(
                claim.request.message,
                Outcome::Interrupted {
                    reason: "timed out".to_owned(),
                },
                start,
            );
        }
        assert!(!backfill.enqueue_at(request(1, 10), start + RETRY_BASE));
    }

    #[test]
    fn a_reconnection_offers_interrupted_bodies_again_but_not_refused_ones() {
        let mut backfill = Backfill::new(BackfillPolicy::default());
        for uid in 1..=3 {
            assert!(backfill.enqueue(request(uid, 10)));
        }
        let outcomes = [
            Outcome::Interrupted {
                reason: "dropped".to_owned(),
            },
            Outcome::Failed {
                reason: "refused".to_owned(),
            },
            Outcome::Gone,
        ];
        let mut lost = None;
        for outcome in outcomes {
            let claim = backfill.next_body().expect("a body");
            if matches!(outcome, Outcome::Interrupted { .. }) {
                lost = Some(claim.request.uid.get());
            }
            backfill.finished(claim.request.message, outcome);
        }
        let queued = |backfill: &mut Backfill| {
            (1..=3)
                .filter(|uid| backfill.enqueue(request(*uid, 10)))
                .collect::<Vec<_>>()
        };
        assert!(queued(&mut backfill).is_empty(), "nothing yet");

        backfill.reconnected();

        assert_eq!(
            queued(&mut backfill),
            vec![lost.expect("one was interrupted")],
            "only the one the link lost"
        );
    }

    #[test]
    fn a_body_that_lands_after_an_interruption_forgets_it() {
        let mut backfill = Backfill::new(BackfillPolicy::default());
        let start = std::time::Instant::now();
        backfill.request_now(request(1, 10));
        let claim = backfill.next_interactive_body().expect("the body");
        backfill.finished_at(
            claim.request.message,
            Outcome::Interrupted {
                reason: "dropped".to_owned(),
            },
            start,
        );
        backfill.request_now(request(1, 10));
        let claim = backfill
            .next_interactive_body()
            .expect("asked again by hand");
        backfill.finished_at(claim.request.message, Outcome::Stored { bytes: 1 }, start);
        assert!(backfill.interrupted.is_empty());
    }
}
