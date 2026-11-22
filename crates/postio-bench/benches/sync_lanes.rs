//! Does the second sync lane still earn its place? Measured: extra lanes never lose; at or below 20
//! ms
//! latency they make no difference, at 50 ms the second is worth ~11%, at 120 ms ~30% and the third
//! ~12% more. A pool larger than the shipped one and no concurrent reads bound the table, so it
//! does
//! not license raising `MAX_SYNC_LANES` (#729).

#![allow(missing_docs)]

use std::time::{Duration, Instant};

use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use postio_account::backend::{MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_account::cancel::CancelToken;
use postio_model::{Account, Mailbox};
use postio_storage::{Checkout, Store, test_support};
use postio_sync::sync_mailbox;

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

/// Enough batches per lane at every lane count (`DEFAULT_BATCH_SIZE` is 200); one batch would
/// measure setup.
const TOTAL_MESSAGES: usize = 1_200;

/// Three is `MAX_SYNC_LANES`, unreachable on the shipped pools (#729); measured anyway.
const LANES: &[usize] = &[1, 2, 3];

/// Zero is pure write contention; 20 ms and 120 ms bracket the p50 and p90 fetch seen in the wild.
const LATENCIES_MS: &[u64] = &[0, 5, 20, 50, 120];

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

fn path(n: usize) -> String {
    if n == 0 {
        "INBOX".to_owned()
    } else {
        format!("Folder{n}")
    }
}

async fn run(lanes: usize, latency: Duration) -> Duration {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let database = Store::open(directory.path().join("postio.db"), &test_support::key())
        .await
        .expect("a database");

    let backend = server(lanes, latency);

    let (account, mailboxes): (Account, Vec<Mailbox>) = {
        let connection = database.connect().await.expect("a connection");
        let account = test_support::account(&connection).await;
        let mut mailboxes = Vec::with_capacity(lanes);
        for n in 0..lanes {
            mailboxes.push(test_support::mailbox(&connection, &account, &path(n)).await);
        }
        (account, mailboxes)
    };
    let _ = &account;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");

    runtime.block_on(async {
        backend.connect().await.expect("connect");

        // Connections are taken up front, sequentially: the pool blocks the OS thread when
        // exhausted and
        // the engine is single threaded, so two waiting passes would deadlock.
        let mut connections: Vec<Checkout> = Vec::with_capacity(lanes);
        for _ in 0..lanes {
            connections.push(database.connect().await.expect("a connection"));
        }

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
            let elapsed = on_runtime(run(lanes, latency));
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
