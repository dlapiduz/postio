//! What a first sync costs to *write*, and how that changes as the store fills.
//!
//! #78 measured a first sync against a real account and found the assumption
//! the sync engine was built on inverted. `sync_wave`'s doc comment says a
//! batch "spent an order of magnitude longer waiting for the server than
//! writing to SQLite"; the measured run spent 13.8 s fetching and 165.5 s
//! writing, a 1:12 ratio the other way, with the write phase occupied ~97% of
//! wall clock across both sync lanes. Read-ahead (#77) is most of why: the
//! fetch now hides behind the previous write, so what is left in the open is
//! the write.
//!
//! That makes **per-message write cost the number that decides how long a
//! first sync takes**, and nothing measured it. The existing budgets —
//! `store_reads`, `search_budget`, the startup trace — are all *read* budgets
//! at one store size.
//!
//! # The curve is the point
//!
//! The other thing #78 saw: write cost per message roughly tripled over one
//! run, 0.88 ms → 2.80 ms, as the store filled. A single number at one store
//! size would have missed that entirely, so this sweeps [`SIZES`]. The shape
//! is the output — a cost that grows with what is already stored is a
//! different problem from one that is simply too high, and only the sweep
//! tells them apart.
//!
//! # What it found, and what that killed
//!
//! The hypothesis this was built to test: `PRAGMA cache_size` is 16 MiB
//! against a store measured at 172 MB, and `thread_links` is a
//! `WITHOUT ROWID` b-tree keyed on a text `Message-ID` — so it takes inserts
//! in effectively random key order, and every page miss pays an SQLCipher
//! decrypt. If that were the mechanism, per-message cost would climb with
//! size and threading would be where it climbed. ADR 0014 names `cache_size`
//! as the first lever if a bench trips.
//!
//! **It does not climb.** Measured on 2026-08-28:
//!
//! ```text
//!    stored    on disk   per message      upsert   threading    contacts
//!         0       9 MB      0.342 ms    0.104 ms    0.136 ms    0.018 ms
//!     20000      25 MB      0.330 ms    0.115 ms    0.148 ms    0.018 ms
//!     80000      88 MB      0.378 ms    0.117 ms    0.143 ms    0.022 ms
//!    120000     131 MB      0.352 ms    0.112 ms    0.130 ms    0.021 ms
//! ```
//!
//! 1.03x from empty to 131 MB, and 1.11x on a second run — the difference
//! between those two runs is the noise floor, so the growth is nothing. And
//! the hypothesis was genuinely exercised rather than merely missed: 131 MB
//! is eight times `cache_size`, the same order of oversubscription as the
//! real store that prompted the question. Raising `cache_size` is not the
//! lever.
//!
//! That leaves #78's 0.88 → 2.80 ms unexplained by store size, and the gap is
//! large: this writes a message in 0.35 ms where that run's *best* case was
//! 0.88 ms and its worst 2.80 ms. What differs is contention — that run had
//! two sync lanes queueing on SQLite's single write lock (its `write_ms`
//! includes the write-gate and lock wait, and #78 measured the write path
//! ~97% occupied across both lanes) on a machine that was also building.
//! Whichever of those it is, it is not the size of the store, and the next
//! measurement worth making is one lane against two.
//!
//! # What is measured
//!
//! [`postio_sync::commit_batch`] — the real write half of a sync pass, not a
//! reimplementation of it. That function is public for exactly this reason:
//! a bench that assembled its own upsert-thread-record sequence would measure
//! a copy that drifts, and a budget over a copy guards nothing.
//!
//! The three phases are timed separately as well, so a regression names the
//! phase that caused it rather than only the total. Those reach the same
//! repository calls `commit_batch` makes; the budget below is asserted on
//! `commit_batch` itself, and the phase split is diagnosis rather than gate.
//!
//! # Why this one does not use criterion
//!
//! Every other bench in this repository does. This workload is stateful and
//! destructive — a measured batch writes two hundred messages that stay
//! written — and criterion decides how often to run a closure by resampling
//! until its statistics settle. That would sweep the very variable this bench
//! holds fixed. See [`measure`].
//!
//! # Running
//!
//! ```sh
//! cargo bench -p postio-runtime --bench sync_writes
//! ```
//!
//! Seeding 120k messages takes a while before the first measurement — that is
//! the fixture, not the workload. CI compiles this and does not time it: a
//! shared runner is too noisy for a millisecond budget. The measurement
//! asserts its own budget with a real `Instant` as well, so a local run fails
//! loudly rather than only reporting.

#![allow(missing_docs)]
// A bench is not public API, and the workspace lint floor reaches bench
// targets.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use postio_core::perf_budget::{SYNC_WRITE_BUDGET, check_budget};
use postio_model::{
    Account, EmailAddress, Mailbox, MailboxRole, Message, RfcMessageId, Uid, UidValidity,
};
use postio_storage::repository::{ContactRepository, MessageRepository, ThreadingRepository};
use postio_storage::seed::{seed_large, thread_seeded_messages};
use postio_storage::test_support::{self, TempDatabase};
use postio_sync::{DEFAULT_BATCH_SIZE, commit_batch};
use rusqlite::{Connection, Transaction, TransactionBehavior};

/// Store sizes to sweep, in messages already stored before the batch lands.
///
/// Empty is the control. 20k and 80k bracket what #78's run actually crossed
/// — it wrote ~82k messages and the cost tripled somewhere inside that. 120k
/// is the size ADR 0014 writes its own falsification test against, so a
/// SQLCipher regression should be visible at the same point.
const SIZES: &[usize] = &[0, 20_000, 80_000, 120_000];

/// How many messages one measured batch writes.
///
/// [`DEFAULT_BATCH_SIZE`], because the thing being measured is what a sync
/// pass actually hands to the write path — five write units of 25.
const BATCH: usize = DEFAULT_BATCH_SIZE;

/// How many seeded messages share a conversation.
///
/// Four is a real conversation rather than a degenerate one, and it matters
/// more here than in a read bench: threading cost depends on how populated
/// `thread_links` already is, and a store of one-message threads would
/// understate it.
const PER_THREAD: usize = 4;

/// The UID space the written batch belongs to.
const UID_VALIDITY: UidValidity = UidValidity::new(1);

/// A store holding `stored` messages, threaded, plus the account and inbox to
/// write into.
///
/// A file rather than memory: WAL, the page cache and SQLCipher's per-page
/// decrypt are most of what is being measured here, and none of them applies
/// the same way to an in-memory database. Measuring the wrong storage engine
/// would be worse than not measuring.
///
/// Seeded *and threaded*: `seed_large` alone inserts and records
/// correspondents but never calls the threading pass, which would leave
/// `thread_links` empty — the one table this bench most needs populated.
fn filled(stored: usize) -> (TempDatabase, Account, Mailbox) {
    let database = test_support::temp();
    let report = seed_large(&database, 7, stored);
    if stored > 0 {
        thread_seeded_messages(&database, report.account.id, PER_THREAD);
    }
    let inbox = report
        .mailbox(MailboxRole::Inbox)
        .expect("an inbox")
        .clone();
    (database, report.account, inbox)
}

/// A batch of headers to write, shaped like mail rather than like a loop.
///
/// `run` distinguishes one generated batch from the next so that repeated
/// iterations write new UIDs and new `Message-ID`s rather than re-upserting
/// the same rows — an insert and an update are not the same write, and
/// measuring the second while claiming the first is the way this bench could
/// quietly lie.
///
/// Every fourth message is a reply carrying the previous one's id in
/// `References`, so the threading phase does the work it does in the wild:
/// a `thread_of` lookup that hits, and a thread joined rather than created.
/// Senders are drawn from a small pool so contact sightings are mostly
/// repeats, which is also what real mail looks like.
fn batch(account: &Account, mailbox: &Mailbox, run: u32) -> Vec<Message> {
    let base = run * BATCH as u32;
    (0..BATCH as u32)
        .map(|n| {
            let uid = base + n + 1;
            let mut message = Message::new(account.id, mailbox.id, chrono::Utc::now());
            message.subject = Some(format!("Bench note {}", uid / 4));
            message.from = vec![EmailAddress::new(
                Some("Ada Lovelace"),
                format!("ada{}@example.com", uid % 64),
            )];
            message.to = vec![EmailAddress::new(Some("Bob"), "bob@example.com")];
            message.server.uid = Some(Uid::new(uid));
            message.server.uid_validity = Some(UID_VALIDITY);
            message.rfc_message_id = Some(RfcMessageId::new(format!("<bench-{uid}@example.com>")));
            if n % 4 != 0 {
                message.in_reply_to = Some(RfcMessageId::new(format!(
                    "<bench-{}@example.com>",
                    uid.saturating_sub(1)
                )));
            }
            message
        })
        .collect()
}

/// Write one batch through the real path, and report what it cost per message.
fn write_one(
    database: &TempDatabase,
    account: &Account,
    mailbox: &Mailbox,
    run: u32,
) -> std::time::Duration {
    let connection = database.connection().expect("a checked-out connection");
    let mut messages = batch(account, mailbox, run);
    let started = Instant::now();
    commit_batch(
        &connection,
        mailbox,
        Some(account),
        &BTreeSet::new(),
        &mut messages,
    )
    .expect("the batch commits");
    started.elapsed()
}

/// How many batches are measured at each store size.
///
/// Ten batches is 2,000 messages: enough that one slow commit does not decide
/// the answer, few enough that the store barely moves while being measured.
/// See [`measure`] for why that second property is the whole design.
const BATCHES: u32 = 10;

/// One warm-up batch, not counted.
///
/// The first batch into a freshly opened store pays for a cold page cache and
/// a cold statement cache. Both are real costs, and neither is the steady
/// state a first sync spends its minutes in.
const WARMUP: u32 = 1;

/// What one store size cost, per message, split by phase.
struct Measured {
    stored: usize,
    /// The database file's size once seeded, in bytes.
    ///
    /// Reported because the growth hypothesis this bench tests is about the
    /// *page cache*, not about the row count: `PRAGMA cache_size` is 16 MiB,
    /// and what matters is how far the file exceeds it. A flat curve measured
    /// against a store that never grew past the cache would say nothing about
    /// a real one that did, so the number a reader needs in order to know
    /// which of those they are looking at is printed alongside.
    on_disk: u64,
    whole: Duration,
    upsert: Duration,
    threading: Duration,
    contacts: Duration,
}

/// Measure one store size.
///
/// # Why this is not criterion
///
/// Every other bench here uses it, and this one deliberately does not. The
/// workload is **stateful and destructive**: a measured batch writes 200
/// messages that stay written. Criterion decides how many times to run a
/// closure by resampling it until its statistics converge, so a measurement
/// labelled "0 already stored" would run its body eighty-odd times and end up
/// averaging over a store holding sixteen thousand — sweeping the exact
/// variable this bench exists to hold fixed, and reporting the mean of a
/// curve as though it were a point.
///
/// So the count is fixed instead: [`BATCHES`] batches after [`WARMUP`], timed
/// with a real `Instant`. The store still grows by 2,000 messages while being
/// measured, which is unavoidable when the operation under test is an insert
/// — it is bounded, stated, and small against every size in [`SIZES`] except
/// the empty control, where it is the point of the control anyway.
///
/// The phases are measured after the whole-path number, in the order the
/// write path runs them: threading and contact recording both need the rows
/// the upsert wrote, so neither can be measured before one.
fn measure(stored: usize) -> Measured {
    let (database, account, inbox) = filled(stored);
    let connection = database.connection().expect("a checked-out connection");
    let mut run = 0;

    for _ in 0..WARMUP {
        run += 1;
        write_one(&database, &account, &inbox, run);
    }

    let mut whole = Duration::ZERO;
    for _ in 0..BATCHES {
        run += 1;
        whole += write_one(&database, &account, &inbox, run);
    }

    let mut upsert = Duration::ZERO;
    for _ in 0..BATCHES {
        run += 1;
        let mut messages = batch(&account, &inbox, run);
        let started = Instant::now();
        let unit = Transaction::new_unchecked(&connection, TransactionBehavior::Immediate)
            .expect("a transaction");
        MessageRepository::new(&unit)
            .upsert_batch(&mut messages)
            .expect("upsert");
        unit.commit().expect("commit");
        upsert += started.elapsed();
    }

    let mut threading = Duration::ZERO;
    for _ in 0..BATCHES {
        run += 1;
        let written = upserted(&connection, &account, &inbox, run);
        let started = Instant::now();
        let unit = Transaction::new_unchecked(&connection, TransactionBehavior::Immediate)
            .expect("a transaction");
        let threads = ThreadingRepository::new(&unit, account.id);
        for message in &written {
            threads.thread(message).expect("thread");
        }
        unit.commit().expect("commit");
        threading += started.elapsed();
    }

    let mut contacts = Duration::ZERO;
    for _ in 0..BATCHES {
        run += 1;
        let written = upserted(&connection, &account, &inbox, run);
        let started = Instant::now();
        let unit = Transaction::new_unchecked(&connection, TransactionBehavior::Immediate)
            .expect("a transaction");
        let recorder = ContactRepository::new(&unit);
        for message in &written {
            recorder.record_message(message).expect("record");
        }
        unit.commit().expect("commit");
        contacts += started.elapsed();
    }

    let per_message = |total: Duration| total / (BATCHES * BATCH as u32);
    Measured {
        stored,
        on_disk: on_disk(&database),
        whole: per_message(whole),
        upsert: per_message(upsert),
        threading: per_message(threading),
        contacts: per_message(contacts),
    }
}

/// How much of the disk the store occupies: the database plus its WAL.
///
/// The WAL counts. During a long first sync it is where recent writes live,
/// and a page read back out of it is as real as one read from the database.
fn on_disk(database: &TempDatabase) -> u64 {
    let file = database.directory().join("postio.db");
    let wal = database.directory().join("postio.db-wal");
    [file, wal]
        .iter()
        .filter_map(|path| std::fs::metadata(path).ok())
        .map(|meta| meta.len())
        .sum()
}

/// A batch already written to the store, ready for a later phase to act on.
fn upserted(
    connection: &Connection,
    account: &Account,
    mailbox: &Mailbox,
    run: u32,
) -> Vec<Message> {
    let mut messages = batch(account, mailbox, run);
    let unit = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .expect("a transaction");
    MessageRepository::new(&unit)
        .upsert_batch(&mut messages)
        .expect("upsert");
    unit.commit().expect("commit");
    messages
}

fn main() {
    println!(
        "\n{:>9}  {:>9}  {:>12}  {:>10}  {:>10}  {:>10}",
        "stored", "on disk", "per message", "upsert", "threading", "contacts"
    );

    let mut results = Vec::new();
    for &stored in SIZES {
        let measured = measure(stored);
        println!(
            "{:>9}  {:>9}  {:>12}  {:>10}  {:>10}  {:>10}",
            measured.stored,
            format!("{} MB", measured.on_disk / 1_000_000),
            format!("{:.3} ms", measured.whole.as_secs_f64() * 1000.0),
            format!("{:.3} ms", measured.upsert.as_secs_f64() * 1000.0),
            format!("{:.3} ms", measured.threading.as_secs_f64() * 1000.0),
            format!("{:.3} ms", measured.contacts.as_secs_f64() * 1000.0),
        );
        results.push(measured);
    }

    // The shape, said out loud: a cost that grows with what is already stored
    // is a different problem from one that is merely too high, and the ratio
    // is what tells them apart at a glance.
    if let (Some(first), Some(last)) = (results.first(), results.last()) {
        let growth = last.whole.as_secs_f64() / first.whole.as_secs_f64();
        println!(
            "\ngrowth from {} to {} stored: {:.2}x per message",
            first.stored, last.stored, growth
        );
    }

    // The gate, at the largest size: that is the case that hurts, since a
    // first sync of a large account is the slowest thing Postio does and the
    // cost at the *end* of one decides when it finishes.
    //
    // Criterion reports; this fails. A budget nobody notices breaking is not a
    // budget, which is why `postio-core`'s own benches assert as well as
    // measure.
    let largest = results.last().expect("at least one size");
    match check_budget(largest.whole, SYNC_WRITE_BUDGET) {
        Ok(()) => println!(
            "\nwithin budget: {:?} per message at {} stored, budget {:?}",
            largest.whole, largest.stored, SYNC_WRITE_BUDGET
        ),
        Err(exceeded) => panic!(
            "writing a sync batch into a {}-message store costs {:?} per \
             message, over budget: {exceeded:?}",
            largest.stored, largest.whole
        ),
    }
}
