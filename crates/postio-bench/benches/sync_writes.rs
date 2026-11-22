//! Per-message write cost of a first sync, swept over store size: it does not climb with store size
//! (1.03x from empty to 131 MB), so `cache_size` is not the lever; contention is the open question.
//! Not criterion: the workload is stateful and criterion's resampling would sweep the store size.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use postio_core::perf_budget::{SYNC_WRITE_BUDGET, check_budget};
use postio_model::{
    Account, EmailAddress, Mailbox, MailboxRole, Message, RfcMessageId, Uid, UidValidity,
};
use postio_storage::Connection;
use postio_storage::repository::{ContactRepository, MessageRepository, ThreadingRepository};
use postio_storage::seed::{seed_large, thread_seeded_messages};
use postio_storage::test_support::{self, TempStore};
use postio_sync::{DEFAULT_BATCH_SIZE, commit_batch};

/// Multi-threaded because a store read may reach `block_in_place`.
fn on_runtime<T>(future: impl std::future::Future<Output = T>) -> T {
    use std::sync::OnceLock;
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("a runtime for the benches")
        })
        .block_on(future)
}

/// 20k and 80k bracket what #78's run crossed; 120k is the size ADR 0014's falsification test uses.
const SIZES: &[usize] = &[0, 20_000, 80_000, 120_000];

/// What a sync pass hands the write path: five write units of 25.
const BATCH: usize = DEFAULT_BATCH_SIZE;

/// Threading cost depends on how populated `thread_links` is, so one-message threads would
/// understate it.
const PER_THREAD: usize = 4;

const UID_VALIDITY: UidValidity = UidValidity::new(1);

/// A file rather than memory: WAL, the page cache and SQLCipher's per-page decrypt are what is
/// measured.
/// Seeded and threaded: `seed_large` alone never runs the threading pass, leaving `thread_links`
/// empty.
async fn filled(stored: usize) -> (TempStore, Account, Mailbox) {
    let database = test_support::temp().await;
    let report = seed_large(&database, 7, stored).await;
    if stored > 0 {
        thread_seeded_messages(&database, report.account.id, PER_THREAD).await;
    }
    let inbox = report
        .mailbox(MailboxRole::Inbox)
        .expect("an inbox")
        .clone();
    (database, report.account, inbox)
}

/// `run` makes each batch write new UIDs and `Message-ID`s: an insert and an update are not the
/// same
/// write. Every fourth message replies to the previous, so threading joins a thread as it does in
/// the wild.
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

async fn write_one(
    database: &TempStore,
    account: &Account,
    mailbox: &Mailbox,
    run: u32,
) -> std::time::Duration {
    let connection = database.connect().await.expect("a checked-out connection");
    let mut messages = batch(account, mailbox, run);
    let started = Instant::now();
    on_runtime(commit_batch(
        &connection,
        mailbox,
        Some(account),
        &BTreeSet::new(),
        &mut messages,
    ))
    .expect("the batch commits");
    started.elapsed()
}

const BATCHES: u32 = 10;

/// The first batch into a fresh store pays for cold page and statement caches, not the steady
/// state.
const WARMUP: u32 = 1;

struct Measured {
    stored: usize,
    /// In bytes. Printed so a flat curve can be read against how far the file exceeds `PRAGMA
    /// cache_size`.
    on_disk: u64,
    whole: Duration,
    upsert: Duration,
    threading: Duration,
    contacts: Duration,
}

/// Not criterion: the workload is stateful, a measured batch stays written, and criterion's
/// resampling would sweep the store size this bench holds fixed.
/// Phases are measured after the whole path, in write order: threading and contacts need the
/// upserted rows.
async fn measure(stored: usize) -> Measured {
    let (database, account, inbox) = filled(stored).await;
    let connection = database.connect().await.expect("a checked-out connection");
    let mut run = 0;

    for _ in 0..WARMUP {
        run += 1;
        write_one(&database, &account, &inbox, run).await;
    }

    let mut whole = Duration::ZERO;
    for _ in 0..BATCHES {
        run += 1;
        whole += write_one(&database, &account, &inbox, run).await;
    }

    let mut upsert = Duration::ZERO;
    for _ in 0..BATCHES {
        run += 1;
        let mut messages = batch(&account, &inbox, run);
        let started = Instant::now();
        let unit = connection.clone();
        on_runtime(unit.execute("BEGIN IMMEDIATE", ())).expect("a transaction");
        MessageRepository::new(&unit)
            .upsert_batch(&mut messages)
            .await
            .expect("upsert");
        on_runtime(unit.execute("COMMIT", ())).expect("commit");
        upsert += started.elapsed();
    }

    let mut threading = Duration::ZERO;
    for _ in 0..BATCHES {
        run += 1;
        let written = upserted(&connection, &account, &inbox, run);
        let started = Instant::now();
        let unit = connection.clone();
        on_runtime(unit.execute("BEGIN IMMEDIATE", ())).expect("a transaction");
        let threads = ThreadingRepository::new(&unit, account.id);
        for message in &written {
            threads.thread(message).await.expect("thread");
        }
        on_runtime(unit.execute("COMMIT", ())).expect("commit");
        threading += started.elapsed();
    }

    let mut contacts = Duration::ZERO;
    for _ in 0..BATCHES {
        run += 1;
        let written = upserted(&connection, &account, &inbox, run);
        let started = Instant::now();
        let unit = connection.clone();
        on_runtime(unit.execute("BEGIN IMMEDIATE", ())).expect("a transaction");
        let recorder = ContactRepository::new(&unit);
        for message in &written {
            on_runtime(recorder.record_message(message, std::slice::from_ref(&account.address)))
                .expect("record");
        }
        on_runtime(unit.execute("COMMIT", ())).expect("commit");
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

/// The WAL counts: recent writes live there during a long first sync.
fn on_disk(database: &TempStore) -> u64 {
    let file = database.directory().join("postio.db");
    let wal = database.directory().join("postio.db-wal");
    [file, wal]
        .iter()
        .filter_map(|path| std::fs::metadata(path).ok())
        .map(|meta| meta.len())
        .sum()
}

fn upserted(
    connection: &Connection,
    account: &Account,
    mailbox: &Mailbox,
    run: u32,
) -> Vec<Message> {
    let mut messages = batch(account, mailbox, run);
    let unit = connection.clone();
    on_runtime(unit.execute("BEGIN IMMEDIATE", ())).expect("a transaction");
    on_runtime(MessageRepository::new(&unit).upsert_batch(&mut messages)).expect("upsert");
    on_runtime(unit.execute("COMMIT", ())).expect("commit");
    messages
}

fn main() {
    println!(
        "\n{:>9}  {:>9}  {:>12}  {:>10}  {:>10}  {:>10}",
        "stored", "on disk", "per message", "upsert", "threading", "contacts"
    );

    let mut results = Vec::new();
    for &stored in SIZES {
        let measured = on_runtime(measure(stored));
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

    if let (Some(first), Some(last)) = (results.first(), results.last()) {
        let growth = last.whole.as_secs_f64() / first.whole.as_secs_f64();
        println!(
            "\ngrowth from {} to {} stored: {:.2}x per message",
            first.stored, last.stored, growth
        );
    }

    // The gate is at the largest size: the end of a large first sync decides when it finishes.
    // Criterion reports; this fails.
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
