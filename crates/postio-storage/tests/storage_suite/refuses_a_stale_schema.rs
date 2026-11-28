#![allow(clippy::disallowed_methods)] // the crate's own code prepares through `sql::statement`; a test may reach the engine directly

use postio_storage::{
    Store,
    key::{Purpose, StoreKey},
};

#[tokio::test]
async fn a_store_from_an_older_schema_is_refused_and_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("postio.db");
    let key = StoreKey::from_bytes([0x2a; 32]).derive(Purpose::Database);

    {
        let store = Store::open(&path, &key).await.expect("a fresh store opens");
        let connection = store.connect().await.expect("a connection");
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
        "the refusal has to name the remedy: no migration leads from this \
         stamp, so a fresh store is the only way out and a person cannot \
         infer that from \"no such column\". Got: {error}"
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
