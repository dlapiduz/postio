use std::time::{Duration, Instant};

use chrono::Utc;
use postio_client::protocol::ClientKind;
use postio_host::{FocusSetup, Host};
use postio_model::listing::{MailStore, PageRequest};
use postio_model::{FocusScope, ListScope};
use postio_storage::repository::{MessageRepository, SettingsRepository};
use postio_storage::sql::{self, RowExt as _};

const MESSAGES: usize = 100_000;

const RECENT: usize = 3_000;

const FILING_BUDGET: Duration = Duration::from_secs(5 * 60);

const BODY_BUDGET: Duration = Duration::from_secs(60);

const INTERACTION_BUDGET: Duration = Duration::from_millis(16);

const FILED_THROUGH: &str = "focus.filed_through";

const PATIENCE: Duration = Duration::from_secs(20 * 60);

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

    // In memory: seeding a hundred thousand rows into an encrypted file in a debug build took
    // longer than the nightly profile's clock, so the numbers leave out the disk.
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

    // What step 2 filed away has left the inbox, so this sorts what is still there.
    let inbox_now = rt.block_on(count(
        &database,
        "SELECT count(*) FROM messages m JOIN mailboxes b ON b.id = m.mailbox_id \
         WHERE b.role = 'inbox'",
    ));
    rt.block_on(async {
        let connection = database.connect().await.expect("a connection");
        SettingsRepository::new(&connection)
            .set(FILED_THROUGH, "0")
            .await
            .expect("the mark");
        for forget in [
            "DELETE FROM focus_classified",
            "DELETE FROM markers",
            "DELETE FROM digest_holds",
            "DELETE FROM filter_decisions",
        ] {
            sql::execute(&connection, forget, ())
                .await
                .expect("the passes' records forgotten");
        }
    });
    let mut needs_action = None;
    let both = open_focus(&rt, &database, blob_dir.path(), |focus| {
        if needs_action.is_none() && focus.caught_up() {
            needs_action = Some(Instant::now());
        }
        needs_action.is_some()
            && rt
                .block_on(async {
                    let reader = database.read().await?;
                    SettingsRepository::new(&reader).get(FILED_THROUGH).await
                })
                .expect("the mark")
                .as_deref()
                == Some(newest.as_str())
    });
    let body_took = needs_action
        .expect("the needs-action pass finished")
        .duration_since(both.started);
    eprintln!(
        "both at once: needs-action in {:.1}s (budget {}s){}, filing in {:.1}s (budget {}s){}",
        body_took.as_secs_f64(),
        BODY_BUDGET.as_secs(),
        over(body_took, BODY_BUDGET),
        both.took.as_secs_f64(),
        FILING_BUDGET.as_secs(),
        over(both.took, FILING_BUDGET)
    );
    both.report();
    let (read_again, sorted_again) = rt.block_on(async {
        (
            count(
                &database,
                "SELECT count(*) FROM focus_classified WHERE stage = 'body'",
            )
            .await,
            count(
                &database,
                "SELECT count(*) FROM focus_classified WHERE stage = 'filing'",
            )
            .await,
        )
    });
    assert_eq!(read_again, RECENT as u64, "both at once, every body read");
    eprintln!("  {read_again} bodies read and {sorted_again} of {inbox_now} inbox messages sorted");
    assert_eq!(
        sorted_again, inbox_now,
        "both at once, every message sorted"
    );
    assert!(
        body_took <= BODY_BUDGET,
        "with the filing pass running, needs-action took {:.1}s",
        body_took.as_secs_f64()
    );
    let median = both.median();
    assert!(
        median <= INTERACTION_BUDGET,
        "with both passes running, a page of the inbox took {} ms at the median",
        median.as_millis()
    );
}

struct Measured {
    started: Instant,
    took: Duration,
    cpu: Option<Duration>,
    reads: Vec<Duration>,
}

impl Measured {
    fn median(&self) -> Duration {
        let mut reads = self.reads.clone();
        reads.sort_unstable();
        reads.get(reads.len() / 2).copied().unwrap_or_default()
    }

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
    Measured {
        started: start,
        took,
        cpu,
        reads,
    }
}

async fn count(database: &postio_storage::Store, sql: &str) -> u64 {
    let reader = database.read().await.expect("a reader");
    let counted: i64 = sql::one(&reader, sql, (), |row| row.col(0))
        .await
        .expect("a count");
    counted as u64
}

fn over(took: Duration, budget: Duration) -> &'static str {
    if took > budget { "  OVER BUDGET" } else { "" }
}
