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
//! The leading hypothesis for the growth, which this bench exists to confirm
//! or kill: `PRAGMA cache_size` is 16 MiB against a store measured at 172 MB,
//! and `thread_links` is a `WITHOUT ROWID` b-tree keyed on a text
//! `Message-ID` — so it takes inserts in effectively random key order, and
//! every page miss pays an SQLCipher decrypt. If that is the mechanism, the
//! per-message cost climbs with size and the threading phase is where it
//! climbs. ADR 0014 names `cache_size` as the first lever if a bench trips.
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
// `criterion_group!` expands to a `pub fn`, and the workspace lint floor
// reaches bench targets. A bench is not public API.

use std::collections::BTreeSet;
use std::time::Instant;

use criterion::{Criterion, criterion_group, criterion_main};
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
                &format!("ada{}@example.com", uid % 64),
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

/// The headline: what a batch costs at each store size.
fn bench_commit_batch(c: &mut Criterion) {
    for &stored in SIZES {
        let (database, account, inbox) = filled(stored);
        let mut run = 0;
        c.bench_function(&format!("commit_batch, {stored} already stored"), |b| {
            b.iter(|| {
                run += 1;
                write_one(&database, &account, &inbox, run)
            })
        });
    }
}

/// Where inside a batch the time goes.
///
/// Not a gate — the budget is asserted on the whole path. This exists so that
/// a regression in the total names the phase that caused it, which is the
/// difference between "a first sync got slower" and "threading got slower as
/// `thread_links` grew", and only the second is actionable.
///
/// Each phase is measured against a store that has already been filled, in
/// the order the write path runs them: threading needs the message rows the
/// upsert wrote, so it cannot be measured before one.
fn bench_phases(c: &mut Criterion) {
    for &stored in SIZES {
        let (database, account, inbox) = filled(stored);
        let connection = database.connection().expect("a checked-out connection");
        let mut run = 10_000;

        c.bench_function(&format!("phase: upsert, {stored} already stored"), |b| {
            b.iter(|| {
                run += 1;
                let mut messages = batch(&account, &inbox, run);
                let unit =
                    Transaction::new_unchecked(&connection, TransactionBehavior::Immediate)
                        .expect("a transaction");
                MessageRepository::new(&unit)
                    .upsert_batch(&mut messages)
                    .expect("upsert");
                unit.commit().expect("commit");
            })
        });

        c.bench_function(&format!("phase: threading, {stored} already stored"), |b| {
            b.iter(|| {
                run += 1;
                let written = upserted(&connection, &account, &inbox, run);
                let unit =
                    Transaction::new_unchecked(&connection, TransactionBehavior::Immediate)
                        .expect("a transaction");
                let threading = ThreadingRepository::new(&unit, account.id);
                for message in &written {
                    threading.thread(message).expect("thread");
                }
                unit.commit().expect("commit");
            })
        });

        c.bench_function(&format!("phase: contacts, {stored} already stored"), |b| {
            b.iter(|| {
                run += 1;
                let written = upserted(&connection, &account, &inbox, run);
                let unit =
                    Transaction::new_unchecked(&connection, TransactionBehavior::Immediate)
                        .expect("a transaction");
                let contacts = ContactRepository::new(&unit);
                for message in &written {
                    contacts.record_message(message).expect("record");
                }
                unit.commit().expect("commit");
            })
        });
    }
}

/// A batch already written to the store, ready for a later phase to act on.
fn upserted(connection: &Connection, account: &Account, mailbox: &Mailbox, run: u32) -> Vec<Message> {
    let mut messages = batch(account, mailbox, run);
    let unit = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .expect("a transaction");
    MessageRepository::new(&unit)
        .upsert_batch(&mut messages)
        .expect("upsert");
    unit.commit().expect("commit");
    messages
}

/// The gate. Criterion reports; this fails.
///
/// A budget nobody notices breaking is not a budget, which is why
/// `postio-core`'s own benches assert as well as measure.
///
/// Asserted at the largest size in [`SIZES`], because that is the case that
/// hurts: a first sync of a large account is the slowest thing Postio does,
/// and the cost at the *end* of one is what decides when it finishes.
fn bench_budget(c: &mut Criterion) {
    let _ = c;
    let largest = *SIZES.last().expect("at least one size");
    let (database, account, inbox) = filled(largest);

    // Warm: the first batch into a freshly opened store pays for a cold page
    // cache, which is a real cost but not the steady state a sync spends its
    // time in.
    write_one(&database, &account, &inbox, 1);

    let measured = write_one(&database, &account, &inbox, 2);
    let per_message = measured / BATCH as u32;
    if let Err(exceeded) = check_budget(per_message, SYNC_WRITE_BUDGET) {
        panic!(
            "writing a sync batch into a {largest}-message store costs \
             {per_message:?} per message, over budget: {exceeded:?}"
        );
    }
}

criterion_group!(benches, bench_commit_batch, bench_phases, bench_budget);
criterion_main!(benches);
