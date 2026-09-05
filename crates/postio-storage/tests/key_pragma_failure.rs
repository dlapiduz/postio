//! What `PRAGMA key requires a key of one or more characters` actually means
//! (#710).
//!
//! `storage_suite::concurrent_open` pins one half of this: an empty key string
//! produces that message. The conclusion drawn from it — that the message
//! means a zero-length key **and nothing else**, so a run reporting it must
//! have executed a statement Postio did not write — does not follow, and this
//! is the counter-example.
//!
//! The pragma handler in the vendored amalgamation
//! (`libsqlite3-sys/sqlcipher/sqlite3.c`) reads:
//!
//! ```c
//! rc = sqlite3_key_v2(db, zDb, zKey, n);
//! ...
//! if( rc==SQLITE_OK && n!=0 ) { ...ok... }
//! else { sqlite3ErrorMsg(pParse, "An error occurred with PRAGMA key or rekey. ...") }
//! ```
//!
//! Two ways in, not one. `n == 0` is the empty key; `rc != SQLITE_OK` is
//! everything else, and `sqlcipherCodecAttach` has several — including
//! `return sqlcipher_init_error` when SQLCipher's one-time initialization
//! (`sqlcipher_extra_init`: static mutexes, a private heap, the crypto
//! provider, and its first draw of randomness) has failed. That one matters
//! for #710's shape: `sqlite3_initialize` retries it on the next call, so a
//! transient failure fails the opens inside its window and lets later ones
//! through — which is the cluster-then-recover the issue keeps recording.
//!
//! **Its own binary** because the lever is `cipher_default_page_size`, which
//! is a SQLCipher process global. `storage_suite`'s module doc is explicit
//! that a case needing one has to move back out rather than change what its
//! neighbours see.

/// The reported text, matched on the sentence that names the key.
const REPORTED: &str = "PRAGMA key requires a key of one or more";

/// A perfectly good key: 32 bytes of hex, the shape `db::configure` writes.
const GOOD_KEY: &str =
    "PRAGMA key = \"x'5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a'\";";

fn probe(before: &str) -> Option<String> {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let connection = rusqlite::Connection::open(directory.path().join("probe.db")).expect("open");
    connection
        .execute_batch("PRAGMA cipher_memory_security = OFF;")
        .expect("the pragma that goes before the key");
    connection.execute_batch(before).expect("the lever");
    connection
        .execute_batch(GOOD_KEY)
        .err()
        .map(|error| error.to_string())
}

#[test]
fn the_key_pragma_error_also_means_the_codec_would_not_start() {
    // `cipher_default_page_size` is stored with `atoi` and validated nowhere;
    // `sqlcipher_codec_ctx_init` reads it and calls
    // `sqlcipher_codec_ctx_set_pagesize`, which refuses anything that is not
    // a power of two between 512 and 65536. So this is a codec that cannot
    // start, reached without going anywhere near the key.
    let text = probe("PRAGMA cipher_default_page_size = 3;");
    assert!(
        text.as_deref().is_some_and(|text| text.contains(REPORTED)),
        "a valid 32-byte key still reported #710's message when the codec \
         could not start -- so the message does not mean the key was empty. \
         Got {text:?}"
    );

    // And the control, so the case is about the codec rather than about the
    // key having been broken along the way: put the default back and the same
    // key is accepted.
    let text = probe("PRAGMA cipher_default_page_size = 4096;");
    assert!(
        text.is_none(),
        "the same key must be accepted once the codec can start: {text:?}"
    );
}

#[test]
fn a_codec_that_will_not_start_is_not_reported_as_a_key_problem() {
    // `db.rs` says of this pragma: "`PRAGMA key` cannot fail: SQLCipher
    // accepts any key and only discovers a wrong one when a page will not
    // decrypt." The case above is that sentence being wrong, and this is what
    // it costs: `Database::open` hands the caller SQLCipher's own text, which
    // says the key needed "one or more characters" — so three passes at #710
    // went looking at key derivation, which is the one place the fault
    // cannot be.
    let directory = tempfile::tempdir().expect("a temporary directory");
    let lever = rusqlite::Connection::open(directory.path().join("lever.db")).expect("open");
    lever
        .execute_batch("PRAGMA cipher_default_page_size = 3;")
        .expect("the lever");

    let opened = postio_storage::Database::open(
        directory.path().join("store.db"),
        &postio_storage::test_support::key(),
    );

    // Put the global back before asserting, so a failure here cannot leave
    // the rest of this binary running against a codec that cannot start.
    lever
        .execute_batch("PRAGMA cipher_default_page_size = 4096;")
        .expect("the lever, put back");

    let error = match opened {
        Ok(_) => panic!("a codec that cannot start must not open a database"),
        Err(error) => error.to_string(),
    };
    assert!(
        !error.contains("requires a key of one or more"),
        "the error a caller sees must not be SQLCipher's text, which names \
         the key: {error}"
    );
    assert!(
        error.contains("sqlcipher") || error.contains("SQLCipher"),
        "and it must say what did fail, so the log lines SQLCipher already \
         printed can be found: {error}"
    );
}
