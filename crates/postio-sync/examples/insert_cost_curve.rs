use std::collections::BTreeSet;
use std::time::Instant;

use postio_model::{Account, EmailAddress, Mailbox, Message, RfcMessageId, Uid, UidValidity};
use postio_storage::test_support;
use postio_sync::commit_batch;

const BATCH: usize = 200;

const CEILING: usize = 30_000;

fn ceiling() -> usize {
    std::env::var("CURVE_CEILING")
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(CEILING)
}

const REPORT_EVERY: usize = 1_000;

const REPRODUCED_MS: f64 = 40.0;

fn synthetic(account: &Account, mailbox: &Mailbox, uid: u32) -> Message {
    let mut message = Message::new(account.id, mailbox.id, chrono::Utc::now());
    // Sized like mail: the fts index eats the subject and sender.
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

async fn run(with_search_index: bool, with_contacts: bool) {
    println!(
        "\n== {}{} ==",
        if with_search_index {
            "with the search_documents fts index (the app's reality)"
        } else {
            "without it (the bisect)"
        },
        if with_contacts {
            ", recording contacts"
        } else {
            ""
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

    let own = [account.address.clone()];
    while written < ceiling() {
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
        if with_contacts {
            let contacts = postio_storage::repository::ContactRepository::new(&connection);
            for message in &batch {
                contacts
                    .record_message(message, &own)
                    .await
                    .expect("the contacts are recorded");
            }
        }
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
    run(true, false).await;
    run(true, true).await;
    run(false, false).await;
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
