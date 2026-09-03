//! Does the second sync lane still earn its place?
//!
//! #32 parallelised the initial sync on a stated premise: "a batch of two
//! hundred headers spent an order of magnitude longer waiting for the server
//! than writing to SQLite", so one mailbox at a time left every one of those
//! waits unused. **That premise is now inverted.** #77's read-ahead put the
//! next `FETCH` on the wire before the local write, and #78 then measured a
//! real account at Σ`fetch_ms` 13.8 s against Σ`write_ms` 165.5 s — 1:12 the
//! other way — with the write phase occupied ~97% of wall clock summed across
//! both lanes. At any instant one lane was inside its write and the other was
//! queued behind SQLite's single write lock rather than overlapping a wait.
//!
//! #726 then ruled out the obvious explanation: per-message write cost is
//! flat from an empty store to 131 MB, so #78's 0.88 → 2.80 ms is not the
//! store filling. What is left is contention.
//!
//! # The question
//!
//! Not "can two lanes keep more than one request in flight" — #32 settled
//! that and `sync_wave.rs` asserts `peak_in_flight > 1`. The question is
//! **whether the second lane adds throughput once the writes it enables have
//! to serialise through one write lock**, and at what round trip the answer
//! changes. A lane that only lengthens each batch's write is worse than no
//! lane at all.
//!
//! So this sweeps latency as well as lane count. At a high enough round trip
//! the extra lane must win; at zero it cannot. What matters is where the
//! crossover sits relative to the 22 ms p50 / 118 ms p90 `fetch_ms` #78 saw
//! in the wild.
//!
//! # What is measured
//!
//! The same shape the engine runs, without reaching into it: `sync_wave` is
//! private and needs an `EngineParts`, so this reproduces what it does —
//! `lanes` concurrent [`sync_mailbox`] passes on a **current-thread** runtime
//! (the engine's, and the reason the write lock is contended rather than
//! merely shared), each holding its own pooled connection, taken up front
//! before anything is concurrent exactly as `sync_wave` takes them.
//!
//! **The work is held constant.** [`TOTAL_MESSAGES`] is split across however
//! many mailboxes there are lanes, so one lane syncing 1,200 messages in one
//! folder is compared against two lanes syncing 600 each. Comparing equal
//! per-lane work would measure "more lanes do more" and answer nothing.
//!
//! The database pool is deliberately oversized ([`POOL`]): this is measuring
//! contention on the *write lock*, and a pool that ran out would measure the
//! pool instead. What the shipped `sync_lanes` may take is a separate
//! question — see #729.
//!
//! # Running
//!
//! ```sh
//! cargo bench -p postio-runtime --bench sync_lanes
//! ```
//!
//! Like `sync_writes`, this is wall clock and therefore worth nothing while
//! another session is building: CLAUDE.md's warning that timing on this
//! machine measures the other build sessions applies with full force. It
//! prints a table rather than asserting a budget — the answer here is a
//! comparison, not a threshold.
//!
//! # What it found
//!
//! Measured 2026-08-29, three runs, median of each cell:
//!
//! ```text
//!    latency     1 lane(s)     2 lane(s)     3 lane(s)
//!       0 ms        269 ms        260 ms        262 ms
//!       5 ms        299 ms        301 ms        302 ms
//!      20 ms        350 ms        357 ms        343 ms
//!      50 ms        485 ms        428 ms        434 ms
//!     120 ms       1036 ms        721 ms        643 ms
//! ```
//!
//! **There is no crossover, and that is the finding.** The hypothesis this
//! was built on — that the second lane had stopped paying for itself, and
//! might be costing, because its writes queue behind one lock — is wrong.
//! Extra lanes never lose:
//!
//! * **At or below 20 ms** they make no measurable difference. Every cell in
//!   those rows is inside the run-to-run spread; lanes neither help nor hurt.
//! * **At 50 ms** the second lane is worth ~11%, and the third adds nothing
//!   over it.
//! * **At 120 ms** the second is worth ~30% and the third a further ~12%.
//!   These rows are the most stable in the table (1 lane 1037/1029/1036,
//!   2 lanes 728/720/721, 3 lanes 643/648/631), so the effect is real.
//!
//! Why contention does not become a penalty: serialising writes does not
//! *add* work, it only fails to remove any. A lane that cannot overlap its
//! write still overlaps its fetch, and the fetch is what grows with latency.
//!
//! Against #78's real account — `fetch_ms` p50 22 ms, p90 118 ms — that puts
//! the median batch in the row where lanes do nothing and the tail in the row
//! where they do a great deal. Which is an argument for keeping them, not for
//! dropping to one.
//!
//! # What this does not license
//!
//! Raising `MAX_SYNC_LANES` to a reachable three. Two caveats bound the table:
//! [`POOL`] is oversized so lanes are never starved, where the shipped
//! database pool yields two lanes (#729); and nothing here reads from the
//! store while the sync runs, which is exactly what `RESERVED_FOR_ELSEWHERE`
//! exists to protect. "Three lanes is faster in isolation" is not "ship three
//! lanes".

#![allow(missing_docs)]

use std::time::{Duration, Instant};

use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use postio_account::backend::{MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_account::cancel::CancelToken;
use postio_model::{Account, Mailbox};
use postio_storage::{Database, PooledConnection, test_support};
use postio_sync::sync_mailbox;

/// Total messages synced per run, however many lanes share them.
///
/// Enough to need several batches per lane at every lane count —
/// `DEFAULT_BATCH_SIZE` is 200, so even three lanes get two batches each —
/// because a single-batch pass would measure setup rather than the steady
/// state a first sync spends its time in.
const TOTAL_MESSAGES: usize = 1_200;

/// Lane counts to compare.
///
/// Three is `MAX_SYNC_LANES`, which #729 records as unreachable on the
/// shipped pools. Measured anyway: knowing whether it would help is what
/// decides if making it reachable is worth anything.
const LANES: &[usize] = &[1, 2, 3];

/// Round trips to sweep, in milliseconds.
///
/// Zero is the floor — pure write contention, where an extra lane can only
/// hurt. 20 ms is about #78's measured p50 `fetch_ms` and 120 ms about its
/// p90, so the crossover, wherever it is, should be bracketed by these.
const LATENCIES_MS: &[u64] = &[0, 5, 20, 50, 120];

/// Database connections available. Never the binding constraint here.
const POOL: usize = 8;

/// A server holding `TOTAL_MESSAGES` split evenly across `mailboxes` folders.
fn server(mailboxes: usize, latency: Duration) -> MockBackend {
    let each = TOTAL_MESSAGES / mailboxes;
    let mut builder = MockBackend::builder();
    for folder in 0..mailboxes {
        let mut mailbox = MockMailbox::new(path(folder));
        for n in 0..each {
            mailbox = mailbox.message(MockMessage::new(
                format!(
                    "From: Ada Lovelace <ada{}@example.com>\r\n\
                     Subject: Note {n}\r\n\
                     Message-ID: <lane-{folder}-{n}@example.com>\r\n\r\nBody {n}.\r\n",
                    n % 64
                )
                .into_bytes(),
            ));
        }
        builder = builder.mailbox(mailbox);
    }
    let backend = builder.build();
    backend.set_latency(latency);
    backend
}

/// The folder name for lane `n`. `INBOX` first, as a real account has.
fn path(n: usize) -> String {
    if n == 0 {
        "INBOX".to_owned()
    } else {
        format!("Folder{n}")
    }
}

/// One measurement: `lanes` concurrent passes over `TOTAL_MESSAGES`.
fn run(lanes: usize, latency: Duration) -> Duration {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let database = Database::open_with(
        directory.path().join("postio.db"),
        &test_support::key(),
        POOL,
    )
    .expect("a database");

    let backend = server(lanes, latency);

    // One local mailbox row per folder the server holds.
    let (account, mailboxes): (Account, Vec<Mailbox>) = {
        let connection = database.connection().expect("a connection");
        let account = test_support::account(&connection);
        let mailboxes = (0..lanes)
            .map(|n| test_support::mailbox(&connection, &account, &path(n)))
            .collect();
        (account, mailboxes)
    };
    let _ = &account;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");

    runtime.block_on(async {
        backend.connect().await.expect("connect");

        // Every lane's connection taken up front, sequentially, before
        // anything is concurrent -- `sync_wave` does this because the pool
        // blocks the OS thread when exhausted and the engine is single
        // threaded, so two passes both waiting would deadlock. Same shape
        // here, so the measurement is of the same arrangement.
        let connections: Vec<PooledConnection> = (0..lanes)
            .map(|_| database.connection().expect("a connection"))
            .collect();

        let cancel = CancelToken::new();
        let started = Instant::now();
        let mut running = FuturesUnordered::new();
        for (connection, mailbox) in connections.iter().zip(&mailboxes) {
            running.push(sync_mailbox(connection, &backend, mailbox, &cancel, |_| {}));
        }
        while let Some(outcome) = running.next().await {
            outcome.expect("a pass completes");
        }
        started.elapsed()
    })
}

fn main() {
    println!("\n{TOTAL_MESSAGES} messages, split across the lanes; wall clock per run\n");
    print!("{:>10}", "latency");
    for lanes in LANES {
        print!("{:>14}", format!("{lanes} lane(s)"));
    }
    println!("{:>12}", "best");

    for &milliseconds in LATENCIES_MS {
        let latency = Duration::from_millis(milliseconds);
        print!("{:>10}", format!("{milliseconds} ms"));

        let mut timings = Vec::new();
        for &lanes in LANES {
            let elapsed = run(lanes, latency);
            print!(
                "{:>14}",
                format!("{:.0} ms", elapsed.as_secs_f64() * 1000.0)
            );
            timings.push((lanes, elapsed));
        }
        let best = timings
            .iter()
            .min_by_key(|(_, elapsed)| *elapsed)
            .map(|(lanes, _)| *lanes)
            .unwrap_or(1);
        println!("{:>12}", format!("{best} lane(s)"));
    }

    println!(
        "\nThe column that wins at each latency is the answer. A crossover \
         above the 22 ms p50 #78 measured means the extra lane is not paying \
         for itself on a real account."
    );
}
