//! What one synced message costs to write, against store size (#1587).
//!
//! A live first sync measured ~118 ms per row — a twenty-five row write unit
//! held the lock for ~3 s — and the existing reproductions all came back
//! flat: `commit_cost` (small commits vs store size), `fts_write_cost` (body
//! index share of a header insert), `fts_merge_stall` (worst single insert
//! against a body index). None of them walked the one path the sync actually
//! takes: [`commit_batch`] — upsert, threading, and the `search_documents`
//! metadata trigger with its five-column `USING fts` index — repeated until
//! the store is *large*.
//!
//! So this walks it, printing ms/row as the store grows, twice: once with
//! the search schema installed (the app's reality — `ensure_schema` runs at
//! session open) and once without, which is the bisect. A curve that climbs
//! with the index and stays flat without it is the answer.
//!
//! ```text
//! cargo run -p postio-sync --release --example insert_cost_curve
//! ```
//!
//! Fixtures only: this writes, so it must never be pointed at real mail.

use std::collections::BTreeSet;
use std::time::Instant;

use postio_model::{Account, EmailAddress, Mailbox, Message, RfcMessageId, Uid, UidValidity};
use postio_storage::test_support;
use postio_sync::commit_batch;

/// How many messages one loop iteration commits — the network batch size the
/// real pass uses, which `commit_batch` then cuts into write units itself.
const BATCH: usize = 200;

/// Stop a run once the store holds this many messages.
const CEILING: usize = 30_000;

/// Report every this many rows.
const REPORT_EVERY: usize = 1_000;

/// Stop early once the cost is unambiguous: two consecutive reports over
/// this many milliseconds per row is the live failure reproduced, and the
/// rest of the curve is money spent proving a proven point.
const REPRODUCED_MS: f64 = 40.0;

fn synthetic(account: &Account, mailbox: &Mailbox, uid: u32) -> Message {
    let mut message = Message::new(account.id, mailbox.id, chrono::Utc::now());
    // Sized like mail, not like a fixture: the fts index eats the subject
    // and sender, so a two-word subject would understate it.
    message.subject = Some(format!(
        "Quarterly figures and the {uid} things still open before the review"
    ));
    message.from = vec![EmailAddress::new(
        Some("Ada Lovelace"),
        format!("sender{}@example.com", uid % 50),
    )];
    message.to = vec![EmailAddress::new(Some("Bob"), "bob@example.com")];
    message.server.uid = Some(Uid::new(uid));
    message.server.uid_validity = Some(UidValidity::new(1));
    message.rfc_message_id = Some(RfcMessageId::new(format!("<curve-{uid}@example.com>")));
    message
}

async fn run(with_search_index: bool) {
    println!(
        "\n== {} ==",
        if with_search_index {
            "with the search_documents fts index (the app's reality)"
        } else {
            "without it (the bisect)"
        }
    );
    println!("{:>8}  {:>10}  {:>10}", "rows", "ms/row", "file");

    let store = test_support::temp().await;
    let connection = store.connect().await.expect("checkout");
    if with_search_index {
        postio_index::index::ensure_schema(&connection)
            .await
            .expect("the search schema");
    }
    let account = test_support::account(&connection).await;
    let mailbox = test_support::mailbox(&connection, &account, "INBOX").await;

    let mut written = 0usize;
    let mut uid = 1u32;
    let mut chunk_started = Instant::now();
    let mut over_budget_reports = 0u32;

    while written < CEILING {
        let mut batch: Vec<Message> = (0..BATCH)
            .map(|_| {
                let message = synthetic(&account, &mailbox, uid);
                uid += 1;
                message
            })
            .collect();
        commit_batch(
            &connection,
            &mailbox,
            Some(&account),
            &BTreeSet::new(),
            &mut batch,
        )
        .await
        .expect("the batch commits");
        written += BATCH;

        if written.is_multiple_of(REPORT_EVERY) {
            let per_row = chunk_started.elapsed().as_secs_f64() * 1000.0 / REPORT_EVERY as f64;
            let file = std::fs::metadata(store.directory().join("postio.db"))
                .map(|meta| meta.len())
                .unwrap_or(0);
            println!("{written:>8}  {per_row:>10.2}  {:>10}", bytes(file));
            chunk_started = Instant::now();

            if per_row > REPRODUCED_MS {
                over_budget_reports += 1;
                if over_budget_reports >= 2 {
                    println!("reproduced: two consecutive reports over {REPRODUCED_MS} ms/row");
                    break;
                }
            } else {
                over_budget_reports = 0;
            }
        }
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    run(true).await;
    run(false).await;
}

fn bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}
