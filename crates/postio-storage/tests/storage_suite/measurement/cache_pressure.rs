#![allow(clippy::disallowed_methods)] // the crate's own code prepares through `sql::statement`; a test may reach the engine directly

use std::time::{Duration, Instant};

use postio_storage::repository::{ListQuery, MessageRepository};
use postio_storage::seed::seed_large;
use postio_storage::test_support;

/// How many messages to seed. Chosen to be past where 16 MiB suffices.
const MESSAGES: usize = 400_000;

/// Cache sizes in KiB, ending where it began: the repeat is the control.
const SIZES: [i64; 4] = [16_000, 65_536, 262_144, 16_000];

fn cpu() -> Duration {
    let stat = std::fs::read_to_string("/proc/self/stat").expect("/proc/self/stat");
    let tail = &stat[stat.rfind(')').expect("the comm field ends") + 1..];
    let fields: Vec<&str> = tail.split_whitespace().collect();
    let utime: u64 = fields[11].parse().expect("utime");
    let stime: u64 = fields[12].parse().expect("stime");
    Duration::from_secs_f64((utime + stime) as f64 / 100.0)
}

async fn sweep(
    database: &postio_storage::Store,
    mailbox: postio_model::MailboxId,
    kib: i64,
) -> (Duration, usize) {
    let connection = database.connect().await.expect("a connection");
    connection
        .execute(&format!("PRAGMA cache_size = {}", -kib), ())
        .await
        .expect("set the cache");
    // So each size starts from the same place rather than inheriting the last
    // one's pages -- without this the sweep measures the order it ran in.
    let _ = connection.execute("PRAGMA shrink_memory", ()).await;

    let before = cpu();
    let mut seen = 0usize;
    for _ in 0..3 {
        for offset in (0..80_000).step_by(500) {
            seen += MessageRepository::new(&connection)
                .page_at(&ListQuery::mailbox(mailbox), offset)
                .await
                .expect("a page")
                .len();
        }
    }
    (cpu().saturating_sub(before), seen)
}

#[tokio::test]
async fn a_cache_below_the_working_set_costs_cpu() {
    let database = test_support::memory().await;
    let report = seed_large(&database, 11, MESSAGES).await;
    let mailbox = report
        .mailboxes
        .iter()
        .find(|m| m.role == postio_model::MailboxRole::Inbox)
        .or_else(|| report.mailboxes.first())
        .expect("a mailbox")
        .id;
    eprintln!("seeded {} messages", report.message_count);

    let started = Instant::now();
    let mut burned = Vec::new();
    for kib in SIZES {
        let (cost, seen) = sweep(&database, mailbox, kib).await;
        assert!(seen > 0, "the workload read nothing at {kib} KiB");
        eprintln!("cache {kib:>7} KiB -> {cost:?}");
        burned.push(cost);
    }
    eprintln!("(whole sweep {:?})", started.elapsed());

    let small = burned[0];
    let large = burned[1];
    let control = burned[3];

    assert!(
        large < small,
        "a larger cache did not help: {small:?} at 16 MiB against {large:?} at 64 MiB. \
         Either the working set now fits 16 MiB -- raise MESSAGES -- or the lever \
         `db.rs` names has stopped being one."
    );
    // The control is what makes the line above mean anything: if the run simply
    // got faster as it went, this would be fast too.
    assert!(
        control > large,
        "the smallest cache was cheap when it ran last ({control:?} against {small:?} \
         when it ran first), so this measured the order of the sweep rather than \
         the cache."
    );
}
