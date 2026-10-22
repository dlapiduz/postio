//! A host that starts its idle passes gives back the disk nothing is using.
//!
//! `BlobStore::collect_garbage`, `purge_temporary` and `evict_to_fit` were
//! each written, tested and documented, and none had a production caller
//! (#416, #862). `postio-storage` proves what the sweeps do and proved it
//! while they were uncalled, so the assertion here is not "the sweep works".
//! It is **"a store this host opened and started its idle passes over has
//! been reclaimed"**, which is the sentence that was false:
//!
//! - blobs no row names, and debris from an unfinished fetch, are gone;
//! - with `[storage] max_bytes` set, the oldest referenced blob goes and the
//!   newer one that fits stays;
//! - a store with enough free pages to be worth rewriting is rewritten, and
//!   keeps its mail.

use std::time::{Duration, Instant};

use postio_host::Host;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

fn patience() -> Duration {
    postio_test_support::scaled(Duration::from_secs(60))
}

/// Poll `done` until it holds or the deadline passes.
fn settled(rt: &tokio::runtime::Runtime, mut done: impl AsyncFnMut() -> bool) -> bool {
    let deadline = Instant::now() + patience();
    loop {
        if rt.block_on(done()) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a test runtime")
}

fn blob_store(directory: &tempfile::TempDir) -> BlobStore {
    BlobStore::open(directory.path().to_path_buf(), &test_support::blob_keys())
        .expect("a blob store")
}

#[test]
fn starting_the_idle_passes_reclaims_what_nothing_references() {
    let rt = runtime();
    let database = rt.block_on(test_support::memory());
    let report = rt.block_on(seed_small(&database, 11));
    assert!(report.message_count > 0, "the fixture seeded no mail");
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = blob_store(&directory);

    // What a deleted message leaves behind: bytes on disk that no row names.
    let orphan = blobs
        .put(b"the body of a message that is no longer in the database")
        .expect("put an orphan");
    // And what a crash mid-fetch leaves: a part file nothing will finish.
    let debris = blobs.temporary_directory().join("9999-0.part");
    std::fs::write(&debris, b"half a message").expect("stage some debris");
    assert!(blobs.contains(&orphan), "the orphan is there to begin with");

    // Backdated past `BLOB_GRACE_PERIOD` rather than shortening it: a blob is
    // written before the row naming it commits, so inside that window a
    // healthy blob is indistinguishable from an orphan. Ageing the file
    // exercises the real constant and the real case, a blob orphaned an hour
    // ago.
    let aged = std::time::SystemTime::now() - Duration::from_secs(2 * 60 * 60);
    std::fs::File::options()
        .write(true)
        .open(blobs.path_of(&orphan).expect("the orphan's path"))
        .expect("open the orphan")
        .set_times(std::fs::FileTimes::new().set_modified(aged))
        .expect("age the orphan");

    let host = Host::start(database, blobs.clone(), |wiring| wiring).expect("a host");
    host.start_idle_passes();

    assert!(
        settled(&rt, async || !blobs.contains(&orphan)),
        "starting the idle passes left a blob nothing references on disk"
    );
    assert!(
        settled(&rt, async || !debris.exists()),
        "starting the idle passes left debris from a fetch that never finished"
    );
}

/// The third sweep, and the one that carries a policy (#862).
///
/// Both blobs are *referenced*: a row points at each. That makes this about
/// eviction rather than about the garbage collector running a moment
/// earlier -- a sweep that only takes what nothing wants would leave both,
/// and both are seconds old, so the grace period would spare them even if
/// they were orphans.
#[test]
fn starting_the_idle_passes_with_a_ceiling_evicts_down_to_it() {
    let rt = runtime();
    let database = rt.block_on(test_support::memory());
    let seeded = rt.block_on(seed_small(&database, 11));
    let inbox = seeded
        .mailbox(postio_model::MailboxRole::Inbox)
        .expect("the seed makes an inbox")
        .clone();
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = blob_store(&directory);

    // Two messages, each holding its raw source. Different fill bytes: the
    // store is content-addressed, so the same bytes twice would be one blob.
    let written = rt.block_on(async {
        let connection = database.connect().await.expect("checkout");
        let messages = postio_storage::repository::MessageRepository::new(&connection);
        let mut written = Vec::new();
        for (index, second) in [1_000_i64, 9_000].into_iter().enumerate() {
            let blob = blobs
                .put(&vec![b'a' + index as u8; 40_000])
                .expect("put a raw source");
            let received = chrono::TimeZone::timestamp_opt(&chrono::Utc, second, 0)
                .single()
                .expect("a timestamp");
            let mut message = postio_model::Message::new(seeded.account.id, inbox.id, received);
            message.server.uid = Some(postio_model::Uid::new(9_000 + index as u32));
            message.server.uid_validity = Some(postio_model::UidValidity::new(1));
            message.raw_blob_id = Some(blob.clone());
            messages.create(&mut message).await.expect("create");
            written.push(blob);
        }
        written
    });
    let (old, new) = (written[0].clone(), written[1].clone());
    assert!(blobs.contains(&old) && blobs.contains(&new));

    // Room for the newer blob and nothing else.
    let ceiling = blobs.len_of(&new).expect("len") + 16;
    let host = Host::start(database, blobs.clone(), |wiring| {
        wiring.with_storage_ceiling(Some(ceiling))
    })
    .expect("a host");
    host.start_idle_passes();

    assert!(
        settled(&rt, async || !blobs.contains(&old)),
        "starting the idle passes left the store over the ceiling its config asked for"
    );
    assert!(
        blobs.contains(&new),
        "eviction took more than the ceiling required: this week's mail stays"
    );
}

/// Enough holes to clear `RECLAIM_FLOOR`, made the cheap way: written and
/// dropped rather than seeded and deleted. `RECLAIM_FLOOR` and
/// `RECLAIM_FRACTION` are the constants that ship; a test that moved them
/// would prove a configuration nobody runs.
const HOLE_BYTES: usize = 96 * 1024;
const DOUBLINGS: u32 = 10; // one row, doubled ten times: 1024 x 96 KiB = 96 MiB

async fn punch_holes(database: &postio_storage::Store) {
    let connection = database.connect().await.expect("a connection");
    connection
        .execute("CREATE TABLE scratch (v TEXT NOT NULL)", ())
        .await
        .expect("a scratch table");
    connection
        .execute(
            "INSERT INTO scratch (v) VALUES (?1)",
            ("x".repeat(HOLE_BYTES),),
        )
        .await
        .expect("the first row");
    for _ in 0..DOUBLINGS {
        connection
            .execute("INSERT INTO scratch (v) SELECT v FROM scratch", ())
            .await
            .expect("double it");
    }
    connection
        .execute("DROP TABLE scratch", ())
        .await
        .expect("drop it");
    drop(connection);
    // The freelist is only what a checkpoint has settled.
    database.truncate_log().await.expect("checkpoint");
}

#[test]
fn starting_the_idle_passes_rewrites_a_store_full_of_holes_and_keeps_its_mail() {
    let rt = runtime();
    // File-backed: pages are only handed back to a filesystem.
    let database = rt.block_on(test_support::temp());
    let report = rt.block_on(seed_small(&database, 11));
    assert!(report.message_count > 0, "the fixture seeded no mail");
    rt.block_on(punch_holes(&database));

    // The precondition, stated rather than assumed: a pass that never runs
    // and one that correctly declines look identical from below.
    assert!(
        rt.block_on(database.is_worth_reclaiming())
            .expect("ask about the holes"),
        "the fixture did not make enough holes to be worth reclaiming, so the \
         assertion below cannot fail"
    );

    let directory = tempfile::tempdir().expect("a blob directory");
    let host =
        Host::start((*database).clone(), blob_store(&directory), |wiring| wiring).expect("a host");
    host.start_idle_passes();

    assert!(
        settled(&rt, async || database
            .free_bytes()
            .await
            .unwrap_or(u64::MAX)
            == 0),
        "starting the idle passes left 96 MiB of free pages in the file: \
         `is_worth_reclaiming` said yes and nothing acted on it"
    );
    // A reclaim that loses rows would satisfy every byte-counting assertion.
    let survived = rt.block_on(async {
        let connection = database.connect().await.expect("a connection");
        postio_storage::sql::scalar(&connection, "SELECT count(*) FROM messages", ())
            .await
            .expect("count")
    });
    assert_eq!(
        survived as usize, report.message_count,
        "the reclaim lost mail"
    );
}

/// The tripwire on the half of #381 this engine does not have.
///
/// A full vacuum is what it can do; it stalls every writer for its whole
/// duration, which is why the case above needs a policy in front of it. The
/// incremental mode has no such cost, so the day the engine accepts it is the
/// day to step it from the idle passes instead. This is a storage assertion
/// and could live in `postio-storage`; it sits beside the passes it informs.
#[test]
fn the_store_still_cannot_be_told_to_reclaim_its_pages_a_little_at_a_time() {
    let rt = runtime();
    rt.block_on(async {
        let database = test_support::temp().await;
        let connection = database.connect().await.expect("a connection");
        let refused = connection.execute("PRAGMA auto_vacuum = 2", ()).await;
        let Err(error) = refused else {
            panic!(
                "the engine accepted `auto_vacuum = INCREMENTAL`: step it from \
                 `maintenance::reclaim_disk` where the full vacuum is decided now"
            );
        };
        let said = error.to_string();
        assert!(
            said.to_lowercase().contains("autovacuum"),
            "`auto_vacuum` failed for some reason other than being unsupported: {said}"
        );
        let mode = postio_storage::sql::scalar(&connection, "PRAGMA auto_vacuum", ())
            .await
            .expect("the pragma reads");
        assert_eq!(mode, 0, "auto_vacuum is {mode} rather than NONE");
    });
}
