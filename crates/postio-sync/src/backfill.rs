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
use std::collections::{BinaryHeap, HashMap, VecDeque};

use chrono::{DateTime, Utc};
use postio_imap::backend::{BackendError, MailBackend, VecSink};
use postio_imap::cancel::CancelToken;
use postio_model::{BodyState, MailboxId, MessageId, Uid, mime};
use postio_storage::BlobStore;
use postio_storage::repository::{BackfillCandidate, BodyBlobs, MessageRepository};
use rusqlite::Connection;

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
    /// `RFC822.SIZE`, as the header fetch reported it — what the cap is
    /// measured against.
    pub size: u64,
    /// When the server received it. The backlog's sort key.
    pub received_at: DateTime<Utc>,
}

impl From<BackfillCandidate> for BodyRequest {
    fn from(candidate: BackfillCandidate) -> Self {
        Self {
            message: candidate.message_id,
            mailbox: candidate.mailbox_id,
            path: candidate.mailbox_path,
            uid: candidate.uid,
            size: candidate.size,
            received_at: candidate.received_at,
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
    user_active: bool,
    cancel: CancelToken,
    cancelled: bool,
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
            user_active: false,
            cancel: CancelToken::new(),
            cancelled: false,
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

    /// Queues a body to fetch when there is nothing better to do.
    ///
    /// A body over [`BackfillPolicy::max_body_bytes`] is counted as skipped and
    /// not queued — it stays on the server until somebody actually wants it, at
    /// which point [`request_now`](Self::request_now) fetches it regardless.
    /// A message already known to the queue is ignored rather than duplicated.
    pub fn enqueue(&mut self, request: BodyRequest) {
        if self.cancelled || self.lanes.contains_key(&request.message) {
            return;
        }
        if self
            .policy
            .max_body_bytes
            .is_some_and(|cap| request.size > cap)
        {
            self.progress.skipped += 1;
            return;
        }
        self.lanes.insert(request.message, Lane::Background);
        self.background.push(Entry(request));
        self.progress.pending += 1;
    }

    /// Asks for a body now, because the user is looking at it.
    ///
    /// Jumps the whole backlog, and is subject to none of the policy: the size
    /// cap, the metered pause and the active-user pause all exist to keep
    /// speculative work out of the way, and this is not speculative.
    ///
    /// A message already in the backlog is *promoted* rather than queued twice,
    /// and one already on the wire is left alone — the fetch the user is
    /// waiting for is the one already running.
    pub fn request_now(&mut self, request: BodyRequest) {
        if self.cancelled {
            return;
        }
        match self.lanes.get(&request.message) {
            // Already being fetched, or already at the front. Either way the
            // user is not waiting any less for a second copy.
            Some(Lane::InFlight | Lane::Interactive) => return,
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
            Outcome::Gone => self.progress.gone += 1,
            Outcome::Failed { .. } => self.progress.failed += 1,
        }
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
/// Returns how many requests were queued, for a caller that wants to log or
/// report progress; not an error for there to be none.
pub fn seed(
    connection: &Connection,
    backfill: &mut Backfill,
    mailbox_id: MailboxId,
    limit: u32,
) -> Result<usize> {
    let candidates = MessageRepository::new(connection).needing_backfill(mailbox_id, limit)?;
    let count = candidates.len();
    for candidate in candidates {
        backfill.enqueue(candidate.into());
    }
    Ok(count)
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
/// Attachment payloads are not pulled here. They are fetched per part when the
/// user opens one, which is why the size cap can be measured against
/// `RFC822.SIZE` and still mean something.
pub async fn fetch_body(
    connection: &Connection,
    blobs: &BlobStore,
    backend: &dyn MailBackend,
    request: &BodyRequest,
    cancel: &CancelToken,
) -> Result<Outcome> {
    let messages = MessageRepository::new(connection);
    let Some(mut message) = messages.get(request.message)? else {
        // Deleted, or wiped by a UIDVALIDITY reset, while this sat in the
        // queue. Checked before the fetch so a stale queue costs no bandwidth.
        return Ok(Outcome::Gone);
    };

    let mut sink = VecSink::new();
    backend
        .fetch_body(&request.path, request.uid, &mut sink, cancel)
        .await?;
    if !sink.is_finished() {
        // The sink contract: without `finish` the bytes are a fragment, and a
        // fragment stored as a message is worse than no message.
        return Err(SyncError::Backend(BackendError::Cancelled));
    }

    let raw = sink.into_inner();
    let bytes = raw.len() as u64;
    // The ingest boundary of #277. `mime::parse` contains the panic itself, so
    // this is not what keeps sync alive -- it is what keeps the failure
    // visible. Without it a message whose bytes defeat the parser stores no
    // body blobs and is indistinguishable, in a log, from one that genuinely
    // had none.
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

    let stored = BodyBlobs {
        text: put_text(blobs, parsed.body.text.as_deref())?,
        html: put_text(blobs, parsed.body.html.as_deref())?,
        // The header block has no reader of its own yet: everything that wants
        // headers has the row, and everything that wants all of them has the
        // raw blob. A copy nobody reads is a copy that can go stale.
        headers: None,
    };

    message.raw_blob_id = Some(blobs.put(&raw)?);
    if message.preview.is_none() {
        message.preview = parsed.preview;
    }
    messages.update(&mut message)?;

    // The commit point.
    messages.set_body_blobs(request.message, &stored, BodyState::Full)?;

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

/// Stores one decoded body form, skipping an empty one so a message with no
/// HTML alternative does not get a blob saying so.
pub(crate) fn put_text(
    blobs: &BlobStore,
    text: Option<&str>,
) -> Result<Option<postio_model::BlobId>> {
    match text {
        Some(text) if !text.is_empty() => Ok(Some(blobs.put(text.as_bytes())?)),
        _ => Ok(None),
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
            message: MessageId::new(uid as i64),
            mailbox: MailboxId::new(1),
            path: "INBOX".to_owned(),
            uid: Uid::new(uid),
            size,
            received_at: at(uid as i64),
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
    fn an_empty_body_form_stores_no_blob() {
        let directory =
            std::env::temp_dir().join(format!("postio-backfill-{}", std::process::id()));
        let blobs = BlobStore::open(&directory).expect("a blob store");

        assert_eq!(put_text(&blobs, None).expect("none"), None);
        assert_eq!(put_text(&blobs, Some("")).expect("empty"), None);
        assert!(put_text(&blobs, Some("hello")).expect("some").is_some());

        std::fs::remove_dir_all(&directory).ok();
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
