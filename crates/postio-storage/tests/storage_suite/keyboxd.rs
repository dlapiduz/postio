use postio_storage::keyboxd::openpgp_keyblobs;

async fn pubring(dir: &std::path::Path, rows: &[(u8, i64, i64, &[u8])]) -> std::path::PathBuf {
    let path = dir.join("pubring.db");
    let database = turso::Builder::new_local(path.to_str().expect("utf-8"))
        .build()
        .await
        .expect("a database");
    let connection = database.connect().expect("a connection");
    connection
        .execute(
            "CREATE TABLE pubkey (ubid BLOB NOT NULL PRIMARY KEY, type INTEGER NOT NULL,
             ephemeral INTEGER NOT NULL DEFAULT 0, revoked INTEGER NOT NULL DEFAULT 0,
             keyblob BLOB NOT NULL)",
            (),
        )
        .await
        .expect("the table");
    for (ubid, kind, ephemeral, blob) in rows {
        connection
            .execute(
                "INSERT INTO pubkey (ubid, type, ephemeral, keyblob) VALUES (?1, ?2, ?3, ?4)",
                (vec![*ubid], *kind, *ephemeral, blob.to_vec()),
            )
            .await
            .expect("a key");
    }
    path
}

#[tokio::test]
async fn keyboxd_gives_its_openpgp_keys_and_not_ephemeral_or_x509_ones() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = pubring(
        dir.path(),
        &[
            (1, 1, 0, b"ada's key"),
            (2, 1, 1, b"an ephemeral key"),
            (3, 2, 0, b"an x509 certificate"),
            (4, 1, 0, b"grace's key"),
        ],
    )
    .await;
    let mut blobs = openpgp_keyblobs(&path).await.expect("read");
    blobs.sort();
    assert_eq!(blobs, [b"ada's key".to_vec(), b"grace's key".to_vec()]);
}

#[tokio::test]
async fn a_keyboxd_file_without_its_table_is_an_error_not_an_empty_keyring() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("pubring.db");
    let database = turso::Builder::new_local(path.to_str().expect("utf-8"))
        .build()
        .await
        .expect("a database");
    database
        .connect()
        .expect("a connection")
        .execute("CREATE TABLE other (id INTEGER)", ())
        .await
        .expect("a table");
    drop(database);
    assert!(openpgp_keyblobs(&path).await.is_err());
}
