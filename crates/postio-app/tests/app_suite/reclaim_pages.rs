//! The store's own pages, and the application reaching them (#381, and what
//! became of it).
//!
//! `auto_vacuum = INCREMENTAL` is how a SQLite store hands a freed page back
//! to the filesystem, and #381 chose it: without it a mailbox that loses ten
//! thousand messages keeps every page it ever needed. `storage_suite`'s own
//! `reclaim_pages.rs` proved the conversion, and this file proved the *reach*
//! — that the running application performed it rather than leaving the method
//! tested and uncalled, which is what #416 was three times over.
//!
//! **This engine has neither half of #381's answer.** `PRAGMA auto_vacuum = 2`
//! answers *"Autovacuum is not enabled. Use --experimental-autovacuum flag"*,
//! the Rust builder exposes no such toggle, and there is no
//! `incremental_vacuum` step to drive it with. What it has is a full `VACUUM`,
//! which is a different proposition and not a drop-in for the other: it
//! rewrites the entire database and blocks every writer while it does, at
//! roughly 25 MiB/s.
//!
//! So the shape changed and the reach question did not. A vacuum that costs
//! that much is *decided* rather than stepped —
//! `Store::is_worth_reclaiming` refuses unless the holes are both large in
//! themselves and a real share of the file — and the thing worth asserting
//! here is the same sentence as ever: **a store this application opened, with
//! enough of it empty to be worth rewriting, has been reclaimed.** That is the
//! sentence a policy nobody calls makes false.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. Set before the app under test starts, which is the one
// moment it is sound -- the same reasoning `wiring.rs` records.

use crate::settle_until;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_session::Wiring;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

use postio_gtk::{app, fonts, style};

/// Enough holes to clear `RECLAIM_FLOOR`, made the cheap way.
///
/// Written and dropped rather than seeded and deleted: 64 MiB of free pages is
/// tens of thousands of messages, which is a minute of fixture for a test that
/// is not about mail. How the pages came to be free is not what is under test
/// — that a store holding this many of them gets rewritten is.
///
/// Deliberately not a lowered threshold. `RECLAIM_FLOOR` and
/// `RECLAIM_FRACTION` are the constants that ship, `store::reclaim_policy`
/// proves what they decide at every magnitude that matters, and a test that
/// moved them would be proving a configuration nobody runs.
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
            (("x".repeat(HOLE_BYTES)),),
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
    // The freelist is only what a checkpoint has settled; without this the
    // store reports the holes it had before the drop.
    database.truncate_log().await.expect("checkpoint");
}

pub fn a_store_full_of_holes_is_reclaimed_by_the_application() {
    crate::gtk_case(async {
        let state_dir = tempfile::tempdir().expect("a state directory");
        // SAFETY: first statement of a single-threaded test.
        unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

        if adw::init().is_err() || gdk::Display::default().is_none() {
            eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
            return;
        }
        let display = gdk::Display::default().unwrap();
        fonts::install().expect("the embedded fonts should install");
        style::install(&display);
        app::install_icons(&display);

        // File-backed, not `memory()`: pages are only handed back to a
        // filesystem, and this test is about the handing back.
        let database = test_support::temp().await;
        let report = seed_small(&database, 11).await;
        assert!(report.message_count > 0, "the fixture seeded no mail");
        punch_holes(&database).await;

        // The precondition, stated rather than assumed. If this is false the
        // test below passes while proving nothing: a worker that never runs
        // and a worker that correctly declines look identical from here.
        assert!(
            database
                .is_worth_reclaiming()
                .await
                .expect("ask about the holes"),
            "the fixture did not make enough holes to be worth reclaiming, so \
             the assertion below cannot fail and is not testing anything"
        );

        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(
            directory.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .expect("a blob store");

        let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
        let (sink, _events) = event_channel();
        let wiring = Wiring::new(
            (*database).clone(),
            blobs,
            bridge.handle(),
            sink,
            bridge.commands(),
        );

        let window = Window::default();
        window.present();
        while glib::MainContext::default().iteration(false) {}

        // The same call `run` makes.
        let feeds = feed_the_window(&window, &wiring)
            .await
            .expect("the seeded store has an account")
            .feeds;
        let _ = feeds;

        assert!(
            settle_until(async || database.free_bytes().await.unwrap_or(u64::MAX) == 0).await,
            "opening a store with 96 MiB of free pages left them in the file: \
             `is_worth_reclaiming` said yes and nothing acted on it"
        );

        // And the mail survived the rewrite. A reclaim that loses rows would
        // satisfy every byte-counting assertion above.
        let connection = database.connect().await.expect("a connection");
        let survived =
            postio_storage::sql::scalar(&connection, "SELECT count(*) FROM messages", ())
                .await
                .expect("count");
        assert_eq!(
            survived as usize, report.message_count,
            "the reclaim the application performed lost mail"
        );
    });
}

/// `auto_vacuum = INCREMENTAL`, as SQLite numbers the modes.
const INCREMENTAL: i64 = 2;

/// The tripwire on the half that is still missing.
///
/// A full vacuum is what this engine can do, not what #381 wanted: it stalls
/// every writer for its whole duration, which is why the case above needs a
/// policy in front of it at all. The incremental mode has no such cost, so the
/// day the engine grows it is the day to take #381's conversion back out of
/// the archive — and this is what notices.
pub fn the_store_still_cannot_be_told_to_reclaim_its_pages_a_little_at_a_time() {
    crate::gtk_case(async {
        let database = test_support::temp().await;
        let connection = database.connect().await.expect("a connection");

        let refused = connection
            .execute(&format!("PRAGMA auto_vacuum = {INCREMENTAL}"), ())
            .await;

        let Err(error) = refused else {
            panic!(
                "the engine accepted `auto_vacuum = INCREMENTAL`. That is good \
                 news: a Postio store can hand freed pages back a few at a time \
                 again, instead of rewriting itself behind a write gate. Take \
                 #381's conversion out of the archive, step it from \
                 `feed_the_window` where the full vacuum is decided now, and \
                 keep the case above -- the reach is the part that was ever \
                 in doubt."
            );
        };
        let said = error.to_string();
        assert!(
            said.to_lowercase().contains("autovacuum"),
            "`auto_vacuum` failed for some reason other than being unsupported, \
             which means this test has stopped measuring what it says: {said}"
        );

        // And the other half: the mode reads as NONE, so nothing anywhere has
        // quietly turned it on by another route.
        let mode = postio_storage::sql::scalar(&connection, "PRAGMA auto_vacuum", ())
            .await
            .expect("the pragma reads");
        assert_eq!(
            mode, 0,
            "auto_vacuum is {mode} rather than NONE, so something set it after \
             all and the claim above is stale"
        );
    });
}
