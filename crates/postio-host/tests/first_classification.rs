//! Focus's first classification pass over a 100,000-message store, against
//! SC-011's budgets (specs/007-postio-focus T147).
//!
//! POSTIO-MEASUREMENT: it builds a store of a hundred thousand messages, and
//! its output is numbers a person reads, so it runs nightly
//! (`.config/nextest.toml` excludes it from the default profile). Run it by
//! hand with
//! `cargo nextest run -p postio-host --profile nightly --test first_classification --no-capture`.
//!
//! SC-011 has two budgets, on one core of the reference workstation, with
//! the interface keeping its interaction budget throughout:
//!
//! - the filtering, digest and invitation pass over the store: under five
//!   minutes, reading no body beyond calendar parts;
//! - the built-in needs-action pass over the inbox's last 30 days: under a
//!   minute, reading only bodies already on this machine.
//!
//! Both are what Focus does when it opens, driven through
//! [`Host::enable_focus`] as `postio-focus` drives it, and each is measured
//! on its own, in two opens of one store:
//!
//! 1. **Focus's first open.** No mark yet, so filing sorts nothing (FR-118)
//!    and the body stage's catch-up is the whole of the work: the
//!    needs-action pass.
//! 2. **An open after another app filed the whole store.** The mark is put
//!    back to the start, the worst case of the catch-up over mail filed
//!    while Focus was closed (FR-134), and the bodies are already read: the
//!    filtering, digest and invitation pass.
//!
//! When both have work at once they share the one background connection
//! and take turns, so neither number is the sum; run 2026-09-28 with both at
//! once, each finished at about the other's time.
//!
//! **What it reports rather than gates** (FR-140: "Timings are measured
//! nightly and report without gating"): the two passes' wall-clock times, the
//! CPU the process spent while they ran, and how long a page of the inbox
//! took to read while they did. It gates only that each pass finished and
//! took everything it should have.

use std::time::{Duration, Instant};

use chrono::Utc;
use postio_client::protocol::ClientKind;
use postio_host::{FocusSetup, Host};
use postio_model::listing::{MailStore, PageRequest};
use postio_model::{FocusScope, ListScope};
use postio_storage::repository::{MessageRepository, SettingsRepository};
use postio_storage::sql::{self, RowExt as _};

/// The store's size, as SC-011 names it.
const MESSAGES: usize = 100_000;

/// Of those, the inbox's last 30 days with a body here: what a working
/// mailbox of that size receives in a month, and what the needs-action pass
/// reads.
const RECENT: usize = 3_000;

/// SC-011's budget for the filtering, digest and invitation pass.
const FILING_BUDGET: Duration = Duration::from_secs(5 * 60);

/// SC-011's budget for the built-in needs-action pass.
const BODY_BUDGET: Duration = Duration::from_secs(60);

/// The constitution's interaction budget, which a page read is held to
/// while the passes run.
const INTERACTION_BUDGET: Duration = Duration::from_millis(16);

/// Where Focus keeps how far it has sorted the store.
const FILED_THROUGH: &str = "focus.filed_through";

/// How long this waits before calling a pass stuck.
const PATIENCE: Duration = Duration::from_secs(20 * 60);

/// The CPU time this process has used so far, from `/proc/self/stat`: user
/// and system clock ticks, at the kernel's usual 100 a second.
fn cpu_time() -> Option<Duration> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let ticks: u64 = fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?;
    Some(Duration::from_millis(ticks * 10))
}

#[test]
fn focus_s_first_classification_pass_over_a_hundred_thousand_messages() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime to seed and poll on");

    // The store: a hundred thousand messages, a month of them recent, with
    // bodies, in the inbox, sent to the user.
    //
    // In memory, as `postio-storage`'s `cache_pressure` seeds its 400,000:
    // seeding a hundred thousand rows into an encrypted file in a debug
    // build took longer than the nightly profile's clock before the pass
    // began. So the numbers below leave out the disk -- they are the
    // passes' own work, which is what one core pays for.
    let seeding = Instant::now();
    let database = rt.block_on(postio_storage::test_support::memory());
    let inbox_mail = rt.block_on(async {
        let report = postio_storage::seed::seed_large(&database, 11, MESSAGES - RECENT).await;
        postio_storage::seed::seed_recent_with_bodies(&database, &report, RECENT, Utc::now(), 11)
            .await;
        count(
            &database,
            "SELECT count(*) FROM messages m JOIN mailboxes b ON b.id = m.mailbox_id \
             WHERE b.role = 'inbox'",
        )
        .await
    });
    eprintln!(
        "seeded {MESSAGES} messages ({inbox_mail} in the inbox, {RECENT} recent with bodies) in {:.1}s",
        seeding.elapsed().as_secs_f64()
    );
    let newest = rt
        .block_on(async {
            let reader = database.read().await?;
            MessageRepository::new(&reader).newest_id().await
        })
        .expect("the newest id")
        .expect("a message")
        .get()
        .to_string();
    let blob_dir = tempfile::tempdir().expect("a blob directory");

    // 1. Focus's first open: no mark, so the needs-action pass alone.
    let body = open_focus(&rt, &database, blob_dir.path(), |focus| focus.caught_up());
    let (read, marked) = rt.block_on(async {
        (
            count(
                &database,
                "SELECT count(*) FROM focus_classified WHERE stage = 'body'",
            )
            .await,
            count(&database, "SELECT count(*) FROM markers").await,
        )
    });
    eprintln!(
        "needs-action pass: {read} bodies read, {marked} marked, in {:.1}s (budget {}s){}",
        body.took.as_secs_f64(),
        BODY_BUDGET.as_secs(),
        over(body.took, BODY_BUDGET)
    );
    body.report();

    // 2. An open after another app filed everything: the mark back at the
    // start, so the filtering, digest and invitation pass over the store.
    rt.block_on(async {
        let connection = database.connect().await.expect("a connection");
        SettingsRepository::new(&connection)
            .set(FILED_THROUGH, "0")
            .await
            .expect("the mark");
    });
    let filing = open_focus(&rt, &database, blob_dir.path(), |_| {
        rt.block_on(async {
            let reader = database.read().await?;
            SettingsRepository::new(&reader).get(FILED_THROUGH).await
        })
        .expect("the mark")
        .as_deref()
            == Some(newest.as_str())
    });
    let sorted = rt.block_on(count(
        &database,
        "SELECT count(*) FROM focus_classified WHERE stage = 'filing'",
    ));
    eprintln!(
        "filing pass: {sorted} messages sorted in {:.1}s (budget {}s){}",
        filing.took.as_secs_f64(),
        FILING_BUDGET.as_secs(),
        over(filing.took, FILING_BUDGET)
    );
    filing.report();

    assert_eq!(read, RECENT as u64, "the body pass read every recent body");
    assert!(marked > 0, "the recent mail's asks were marked");
    assert_eq!(
        sorted, inbox_mail,
        "the filing pass sorted every inbox message past the mark"
    );
}

/// What one open cost.
struct Measured {
    took: Duration,
    cpu: Option<Duration>,
    reads: Vec<Duration>,
}

impl Measured {
    fn report(&self) {
        if let Some(cpu) = self.cpu {
            eprintln!(
                "  CPU: {:.1}s, against {:.1}s of wall clock",
                cpu.as_secs_f64(),
                self.took.as_secs_f64()
            );
        }
        let mut reads = self.reads.clone();
        reads.sort_unstable();
        let median = reads.get(reads.len() / 2).copied().unwrap_or_default();
        let slowest = reads.last().copied().unwrap_or_default();
        eprintln!(
            "  a page of the inbox meanwhile: median {} ms, slowest {} ms, over {} reads (budget {} ms){}",
            median.as_millis(),
            slowest.as_millis(),
            reads.len(),
            INTERACTION_BUDGET.as_millis(),
            over(median, INTERACTION_BUDGET)
        );
    }
}

/// Start a host over `database`, switch Focus on as `postio-focus` does,
/// and time it until `done` -- reading a page of Focus's inbox every quarter
/// second meanwhile, as a person would. The host stops before this answers.
fn open_focus(
    rt: &tokio::runtime::Runtime,
    database: &postio_storage::Store,
    blob_dir: &std::path::Path,
    mut done: impl FnMut(&postio_host::FocusHandle) -> bool,
) -> Measured {
    let blobs = postio_storage::BlobStore::open(
        blob_dir.to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let host = Host::start(database.clone(), blobs, |wiring| wiring).expect("a host");
    let client = host.connect(ClientKind::Focus);
    let cpu_before = cpu_time();
    let start = Instant::now();
    let focus = host.enable_focus(FocusSetup::default());
    let mut reads = Vec::new();
    let took = loop {
        assert!(start.elapsed() < PATIENCE, "a pass is stuck");
        if done(&focus) {
            break start.elapsed();
        }
        let read = Instant::now();
        rt.block_on(client.list_page(PageRequest {
            scope: ListScope::Focus(FocusScope::Inbox),
            offset: 0,
            limit: 60,
        }))
        .expect("a page while the pass runs");
        reads.push(read.elapsed());
        std::thread::sleep(Duration::from_millis(250));
    };
    let cpu = cpu_before
        .zip(cpu_time())
        .map(|(before, after)| after.saturating_sub(before));
    drop(client);
    host.stop();
    Measured { took, cpu, reads }
}

/// `sql`'s one number.
async fn count(database: &postio_storage::Store, sql: &str) -> u64 {
    let reader = database.read().await.expect("a reader");
    let counted: i64 = sql::one(&reader, sql, (), |row| row.col(0))
        .await
        .expect("a count");
    counted as u64
}

/// What to say after a number that is over its budget.
fn over(took: Duration, budget: Duration) -> &'static str {
    if took > budget { "  OVER BUDGET" } else { "" }
}
