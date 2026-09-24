//! A store written by an older schema is refused, not opened and then broken.
//!
//! `refuses_a_foreign_store.rs` covers the cross-engine case: a SQLCipher
//! store cannot be decrypted by this build, so it is caught at the door. This
//! is the case that argument does not reach. A store written by an *earlier
//! build of this engine* opens perfectly — same cipher, same key, same file
//! format — and then fails at query time, once, per statement that names a
//! column added since:
//!
//! ```text
//! ERROR postio_app::commands: command failed
//!   engine: Parse error: no such column: body_parsed_with
//! WARN  postio_runtime::engine: cannot top up the backfill for a folder
//!   error=engine: Parse error: no such column: body_parsed_with
//! ```
//!
//! Met against a real store on 2026-09-17: created 08:48, and
//! `body_parsed_with` was added to the schema at 10:36 the same morning. There
//! are no migrations by design (`schema::HEAD`'s own header says why), so the
//! remedy is to resync — but nothing said so, and the store went on running,
//! spraying the same warning per folder forever.
//!
//! So the schema is stamped when it is applied and checked when it is opened.
//! An older store is refused with a sentence naming the remedy, and left
//! exactly as it was, for the same reason the foreign store is: "rebuilt by
//! resyncing" means the file has to survive being refused.

#![allow(clippy::disallowed_methods)] // the crate's own code prepares through `sql::statement`; a test may reach the engine directly

use postio_storage::{
    Store,
    key::{Purpose, StoreKey},
};

/// Opening a store whose schema is not this build's is refused.
#[tokio::test]
async fn a_store_from_an_older_schema_is_refused_and_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("postio.db");
    let key = StoreKey::from_bytes([0x2a; 32]).derive(Purpose::Database);

    // A store this build wrote, and would open again happily.
    {
        let store = Store::open(&path, &key).await.expect("a fresh store opens");
        let connection = store.connect().await.expect("a connection");
        // Wind the stamp back to what an earlier build left: a schema this
        // file no longer matches. The rows stay exactly where they are, which
        // is the whole point — this is the shape of a store that has been
        // synced for days and is now one column behind.
        connection
            .execute("PRAGMA user_version = 1", ())
            .await
            .expect("the stamp is writable");
    }

    let before = std::fs::read(&path).unwrap();

    let Err(error) = Store::open(&path, &key).await else {
        panic!(
            "a store written by an older schema must be refused. Opening it \
             succeeds and then every statement naming a column added since \
             fails one at a time, which is how an INBOX ends up warning per \
             folder forever with nothing saying to resync"
        );
    };
    assert!(
        matches!(error, postio_storage::Error::SchemaFromAnotherBuild { .. }),
        "the refusal has to name the remedy: there are no migrations, so \
         resyncing is the only way out and a person cannot infer that from \
         \"no such column\". Got: {error}"
    );
    let said = error.to_string();
    assert!(
        said.contains("sync"),
        "the sentence reaches a screen and must say what to do about it, \
         got: {said}"
    );

    let after = std::fs::read(&path).unwrap();
    assert_eq!(
        before, after,
        "the refusal rewrote the file it was refusing to read"
    );
}

/// The control: this build's own store still opens.
///
/// Without it the guard could be "refuse everything", which is a worse bug
/// than the one it fixes — every launch would demand a resync.
#[tokio::test]
async fn a_store_this_build_wrote_still_opens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("postio.db");
    let key = StoreKey::from_bytes([0x2a; 32]).derive(Purpose::Database);

    drop(Store::open(&path, &key).await.expect("a fresh store opens"));
    Store::open(&path, &key)
        .await
        .expect("reopening a store this build wrote must not demand a resync");
}
