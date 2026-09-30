//! What indexing every body leaves resident, under the application's
//! allocator (#1606).
//!
//! `first_sync_memory` measured a first sync through the engine and settled
//! two of the review's suspects: huge pages cost ~14 MB, and the header
//! index's segment cache nothing measurable. The body index was the one it
//! could not see, because the session's indexer writes it and an engine-level
//! run does not include that. This is the indexer's own catch-up pass,
//! `index_local_bodies`, over a store whose bodies are all local:
//!
//! ```text
//! cargo nextest run -p postio-session --test body_index_memory --profile nightly --no-capture
//! ```
//!
//! Three samples: before the pass, when every body is indexed, and after a
//! quiet spell for the allocator to purge. Each reports RSS, the process peak,
//! and the largest anonymous mapping -- the mimalloc arena.
//!
//! POSTIO-MEASUREMENT: its output is numbers a person reads, and indexing
//! twenty thousand bodies takes minutes.

use std::time::{Duration, Instant};

use chrono::Utc;
use postio_model::{BodyState, Message};
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::test_support;

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

const BODIES: usize = 20_000;

/// `POSTIO_MEASURE_BODIES` overrides [`BODIES`] for a quicker look.
fn bodies() -> usize {
    std::env::var("POSTIO_MEASURE_BODIES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(BODIES)
}

/// A body of a few hundred words, varied enough that the index holds a real
/// vocabulary rather than one repeated term.
fn body(n: usize) -> String {
    let words = [
        "interlock",
        "tide",
        "station",
        "reading",
        "channel",
        "gate",
        "survey",
        "valve",
        "pressure",
        "north",
        "basin",
        "schedule",
        "report",
        "weekly",
        "inspection",
    ];
    (0..300)
        .map(|i| format!("{}{}", words[(n * 7 + i * 13) % words.len()], (n + i) % 97))
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug)]
struct Sample {
    rss_kb: u64,
    peak_kb: u64,
    arena_rss_kb: u64,
}

fn sample() -> Sample {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |name: &str| {
        status
            .lines()
            .find(|line| line.starts_with(name))
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    };
    let smaps = std::fs::read_to_string("/proc/self/smaps").unwrap_or_default();
    let (mut arena, mut anonymous, mut rss) = (0u64, false, 0u64);
    for line in smaps.lines() {
        let first = line.split_whitespace().next().unwrap_or_default();
        if first.contains('-') && first.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
            if anonymous {
                arena = arena.max(rss);
            }
            anonymous = line.split_whitespace().count() == 5;
            rss = 0;
        } else if first == "Rss:" {
            rss = line
                .split_whitespace()
                .nth(1)
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
        }
    }
    if anonymous {
        arena = arena.max(rss);
    }
    Sample {
        rss_kb: field("VmRSS:"),
        peak_kb: field("VmHWM:"),
        arena_rss_kb: arena,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn what_indexing_every_body_leaves_resident() {
    let database = test_support::temp().await;
    {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        postio_index::index::ensure_schema(&connection)
            .await
            .expect("the index schema");
        let repository = MessageRepository::new(&connection);
        for n in 0..bodies() {
            let mut message = Message::new(account.id, inbox, Utc::now());
            message.subject = Some(format!("Report {n}"));
            let id = repository.create(&mut message).await.expect("a message");
            repository
                .set_body(
                    id,
                    &StoredBody {
                        text: Some(body(n)),
                        ..StoredBody::default()
                    },
                    BodyState::Full,
                )
                .await
                .expect("a body");
        }
    }

    let before = sample();
    let started = Instant::now();
    let indexed = postio_session::index_local_bodies(&database)
        .await
        .expect("the pass runs");
    let took = started.elapsed();
    let indexed_sample = sample();
    tokio::time::sleep(Duration::from_secs(10)).await;
    let quiet = sample();

    let mb = |kb: u64| kb as f64 / 1024.0;
    for (label, s) in [
        ("before", &before),
        ("indexed", &indexed_sample),
        ("quiet", &quiet),
    ] {
        eprintln!(
            "body-index memory {label:>7}: rss {:7.1} MB, peak {:7.1} MB, arena {:7.1} MB",
            mb(s.rss_kb),
            mb(s.peak_kb),
            mb(s.arena_rss_kb)
        );
    }
    eprintln!(
        "body-index memory: {indexed} bodies indexed in {:.1}s",
        took.as_secs_f64()
    );
    assert_eq!(indexed, bodies(), "every body was indexed");
}
