#![allow(unsafe_code)]
//! Autosave writes on the runtime, never on the GTK main thread (#1608).
//!
//! The composer's save handler ran `blocking::now` around an interactive
//! write, so every autosave tick waited on the GTK thread for the writer --
//! behind whatever unit a background sync was committing. Moving it off the
//! thread must not cost what the synchronous version guaranteed: saves in a
//! row update one row rather than inserting two, the composer learns the id
//! its first save assigned, and closing an empty composer still removes the
//! row it autosaved.

use crate::settle_until;

use gtk::{gdk, glib};
use postio_app::{Wiring, feed_the_window};
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_storage::seed::seed_small;
use postio_storage::test_support::counting::counted;
use postio_storage::{BlobStore, Store, test_support};

async fn drafts(database: &Store) -> i64 {
    let connection = database.connect().await.expect("a connection");
    postio_storage::sql::one(&connection, "SELECT count(*) FROM drafts", (), |row| {
        postio_storage::sql::RowExt::col(row, 0)
    })
    .await
    .expect("a count")
}

pub fn autosave_writes_off_the_main_thread_and_keeps_one_row() {
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

        let database = test_support::memory().await;
        let report = seed_small(&database, 9).await;
        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(
            directory.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .expect("a blob store");
        let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
        let (sink, _events) = event_channel();
        let wiring = Wiring::new(
            database.clone(),
            blobs,
            bridge.handle(),
            sink,
            bridge.commands(),
        );
        let window = Window::default();
        window.present();
        while glib::MainContext::default().iteration(false) {}
        let _ = feed_the_window(&window, &wiring).await;
        while glib::MainContext::default().iteration(false) {}
        let before = drafts(&database).await;

        let composer = window.composer();
        composer.open(postio_model::Draft::new(report.account.id));
        composer.test_set_subject("Tide gate interlock");
        let counts = counted(|| {
            composer.save();
            composer.test_set_body("first words");
            composer.save();
        });
        assert_eq!(
            counts.statements, 0,
            "two autosaves ran {} statements on the GTK thread",
            counts.statements
        );

        assert!(
            settle_until(async || composer.draft().id.is_assigned()).await,
            "the composer never learnt the id its first save assigned"
        );
        assert_eq!(
            drafts(&database).await - before,
            1,
            "two saves in a row inserted two rows rather than updating one"
        );

        composer.test_set_subject("");
        composer.test_set_body("");
        composer.close();
        assert!(
            settle_until(async || {
                let now = drafts(&database).await;
                now == before
            })
            .await,
            "closing an emptied composer left its autosaved row behind"
        );
    });
}
