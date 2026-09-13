//! The store's own pages are **not** reclaimed, and this is what says so
//! (#381, and what became of it).
//!
//! `auto_vacuum = INCREMENTAL` is how a SQLite store hands a freed page back
//! to the filesystem, and #381 chose it: without it a mailbox that loses ten
//! thousand messages keeps every page it ever needed. `storage_suite/vacuum.rs`
//! proved the conversion and this case proved the *reach* — that the running
//! application performed it rather than leaving the method tested and
//! uncalled, which is what #416 was three times over.
//!
//! **This engine has neither half.** `PRAGMA auto_vacuum = 2` answers
//! *"Autovacuum is not enabled. Use --experimental-autovacuum flag"*, and the
//! Rust builder exposes no such toggle — so incremental vacuum cannot be
//! turned on from Postio at all. A full `VACUUM` exists behind
//! `Builder::experimental_vacuum`, and a full vacuum rewrites the entire
//! database: on ADR 0017's 868 MB reference store that is not something to do
//! on a startup path, and it is experimental in a pre-1.0 engine besides.
//!
//! So the behaviour is a known regression rather than an oversight: a Postio
//! store grows and does not shrink. What is left to assert is that it is still
//! *known* — this fails the moment the engine grows the pragma, which is the
//! signal to take the conversion back.
//!
//! Deliberately not a GTK case any more. There is nothing for the application
//! to reach, so standing up a window to watch it not happen would be theatre.

use postio_storage::test_support;

/// `auto_vacuum = INCREMENTAL`, as SQLite numbers the modes.
const INCREMENTAL: i64 = 2;

pub fn the_store_still_cannot_be_told_to_reclaim_its_pages() {
    crate::gtk_case(async {
        let database = test_support::temp().await;
        let connection = database.connect().await.expect("a connection");

        let refused = connection
            .execute(&format!("PRAGMA auto_vacuum = {INCREMENTAL}"), ())
            .await;

        let Err(error) = refused else {
            panic!(
                "the engine accepted `auto_vacuum = INCREMENTAL`. That is good \
                 news: a Postio store can hand freed pages back again. Take \
                 #381's conversion out of the archive, call it from \
                 `feed_the_window`, and put back the case that proves the \
                 application reaches it -- `storage_suite/vacuum.rs` and this \
                 file both have the shape in their history."
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
