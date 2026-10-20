//! What backfilling the bodies of one folder costs in round trips and wall
//! clock when every command takes a round trip (#1941).
//!
//! The mock charges its latency once per command, a batched `UID FETCH`
//! included, so `calls()` is the number of round trips the engine paid and
//! the stopwatch is what they cost at 50 ms each. Messages are mixed: plain
//! notes, text-and-HTML alternatives, and a few large newsletters, so the
//! batches are not all the same shape.
//!
//! ```text
//! cargo nextest run -p postio-runtime --test body_backfill_cost --profile nightly --no-capture
//! ```
//!
//! POSTIO-MEASUREMENT: its output is numbers a person reads and compares
//! across changes to the backfill, and at 50 ms a command it takes a minute.

use std::sync::Arc;
use std::time::{Duration, Instant};

use postio_account::backend::{BodyStructure, MockBackend, MockMailbox, MockMessage, PartNode};
use postio_core::bridge::event_channel;
use postio_runtime::engine::{Engine, EngineParts, NetworkSource, SystemClock};
use postio_storage::{BlobStore, test_support};

const MESSAGES: u32 = 300;
const ROUND_TRIP: Duration = Duration::from_millis(50);

/// `POSTIO_MEASURE_RTT_MS` overrides [`ROUND_TRIP`]; zero is the local floor.
fn round_trip() -> Duration {
    std::env::var("POSTIO_MEASURE_RTT_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .map_or(ROUND_TRIP, Duration::from_millis)
}

/// `POSTIO_MEASURE_MESSAGES` overrides [`MESSAGES`] for a quicker look.
fn messages() -> u32 {
    std::env::var("POSTIO_MEASURE_MESSAGES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(MESSAGES)
}

fn sentence(words: usize) -> String {
    "The tide gate interlock report for this week, with the readings. ".repeat(words / 10 + 1)
}

/// Five shapes in rotation: a line, a note, a long note, an alternative, and
/// a large newsletter. Each carries the structure and sections a server
/// reports, because without a `BODYSTRUCTURE` the backfill fetches the whole
/// message and never batches.
fn message(n: u32) -> MockMessage {
    let head = format!(
        "From: Ada Lovelace <ada@example.com>\r\n\
         To: Postio <postio@example.net>\r\n\
         Subject: interlock report {n}\r\n\
         Message-ID: <m-{n}@example.com>\r\n\
         Date: Mon, 1 Jun 2026 09:00:00 +0000\r\n"
    );
    let single = |kind: &str, text: String| {
        let raw = format!("{head}Content-Type: {kind}; charset=utf-8\r\n\r\n{text}\r\n");
        MockMessage::new(raw.into_bytes())
            .with_structure(BodyStructure::from_parts(
                kind,
                [PartNode::new("1", kind, text.len() as u64).with_charset("utf-8")],
            ))
            .with_part("1", text.into_bytes())
    };
    match n % 5 {
        0 => single("text/plain", sentence(10)),
        1 => single("text/plain", sentence(200)),
        2 => single("text/plain", sentence(1500)),
        3 => {
            let (plain, html) = (sentence(300), format!("<p>{}</p>", sentence(300)));
            let raw = format!(
                "{head}MIME-Version: 1.0\r\n\
                 Content-Type: multipart/alternative; boundary=\"b\"\r\n\r\n\
                 --b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{plain}\r\n\
                 --b\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n--b--\r\n"
            );
            MockMessage::new(raw.into_bytes())
                .with_structure(BodyStructure::from_parts(
                    "multipart/alternative",
                    [
                        PartNode::new("1", "text/plain", plain.len() as u64).with_charset("utf-8"),
                        PartNode::new("2", "text/html", html.len() as u64).with_charset("utf-8"),
                    ],
                ))
                .with_part("1", plain.into_bytes())
                .with_part("2", html.into_bytes())
        }
        _ => single("text/html", format!("<p>{}</p>", sentence(30_000))),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn what_a_folders_bodies_cost_at_fifty_milliseconds_a_command() {
    let mut archive = MockMailbox::new("Archive").attributes(["\\Archive"]);
    for n in 1..=messages() {
        archive = archive.message(message(n));
    }
    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(MockMailbox::new("INBOX"))
            .mailbox(archive)
            .build(),
    );
    backend.refuse_creates("no new folders here");
    backend.set_latency(round_trip());

    let database = test_support::temp().await;
    let account = {
        let connection = database.connect().await.expect("a connection");
        test_support::account(&connection).await
    };
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.path().to_path_buf(), &test_support::blob_keys())
        .expect("a blob store");
    let (sink, _events) = event_channel();

    let started = Instant::now();
    let engine = Engine::spawn(EngineParts {
        account: account.id,
        database: database.clone(),
        blobs,
        backend: backend.clone(),
        smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
        tokens: Arc::new(postio_account::auth::StoredPasswordSource::new(Arc::new(
            postio_account::secret::MemorySecretStore::default(),
        ))),
        events: sink,
        retry: Default::default(),
        backfill: Default::default(),
        reconnect: Default::default(),
        watch: Default::default(),
        network: NetworkSource::Ignored,
        mailbox_roles: Default::default(),
        clock: Arc::new(SystemClock),
    })
    .expect("the engine starts");

    let count = |sql: &'static str| {
        let database = database.clone();
        async move {
            let connection = database.connect().await.expect("a connection");
            postio_storage::sql::one(&connection, sql, (), |row| {
                postio_storage::sql::RowExt::col(row, 0)
            })
            .await
            .expect("a count")
        }
    };
    let mut headers_at = None;
    let deadline = Instant::now() + postio_test_support::scaled(Duration::from_secs(600));
    loop {
        let stored: i64 = count("SELECT count(*) FROM messages").await;
        let owed: i64 = count(
            "SELECT count(*) FROM messages WHERE body_state IN ('not_fetched', 'headers_only')",
        )
        .await;
        if headers_at.is_none() && stored >= i64::from(messages()) {
            headers_at = Some((started.elapsed(), backend.calls()));
        }
        if stored >= i64::from(messages()) && owed == 0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the backfill did not finish: {stored} stored, {owed} bodies owed"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let took = started.elapsed();
    let calls = backend.calls();
    let batches = backend.section_batches();
    let named: usize = batches.iter().map(|(_, n)| n).sum();
    eprintln!("batches: {batches:?}");
    drop(engine);

    let (headers_took, headers_calls) = headers_at.unwrap_or_default();
    eprintln!(
        "body backfill, {} messages at {:?} a command: {took:.1?} and {calls} commands \
         in all; headers all local at {headers_took:.1?} and {headers_calls} commands; \
         {} batched section commands naming {named} messages; peak {} in flight",
        messages(),
        round_trip(),
        batches.len(),
        backend.peak_in_flight(),
    );
    let _ = (calls, named);
}
