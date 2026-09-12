//! SQLite3 Multiple Ciphers: encryption proved, then timed.
//!
//! The same workload `postio-cipher`'s `throughput.rs` runs against
//! SQLCipher, so the two numbers can be put beside each other: build a store
//! of the same size, then scan it whole from a *fresh* connection, floor of
//! five rounds. Fresh because SQLite caches decrypted pages — a second scan
//! down the same connection decrypts nothing.
use std::time::{Duration, Instant};

const KEY: &str = "x'000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f'";
const ROUNDS: usize = 5;

fn key(db: &rusqlite::Connection) {
    db.execute_batch(&format!("PRAGMA key = \"{KEY}\";"))
        .expect("the key pragma");
}

fn main() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("store.db");

    // ── the encryption, before anything is timed ─────────────────────────
    {
        let db = rusqlite::Connection::open(&path).expect("open");
        key(&db);
        // WAL, because that is what Postio runs and because a VFS-based
        // encryption layer has to get the write-ahead log right too.
        let mode: String = db
            .query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))
            .expect("wal");
        let cipher: String = db
            .query_row("PRAGMA cipher", [], |r| r.get(0))
            .expect("cipher");
        println!("cipher = {cipher:?}   journal_mode = {mode:?}   sqlite = {:?}",
            db.query_row::<String, _, _>("SELECT sqlite_version()", [], |r| r.get(0)).unwrap());
        assert_eq!(mode, "wal", "the WAL did not engage");

        db.execute_batch(
            "CREATE TABLE mail(id INTEGER PRIMARY KEY, subject TEXT, body BLOB);
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 6000)
             INSERT INTO mail(subject, body)
               SELECT 'a subject line ' || i, randomblob(4000) FROM n;",
        )
        .expect("a store worth scanning");
        db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").ok();
    }

    let head = std::fs::read(&path).expect("the file");
    assert!(
        !head.starts_with(b"SQLite format 3"),
        "THE STORE IS PLAINTEXT -- the whole point is the encryption"
    );
    {
        let db = rusqlite::Connection::open(&path).expect("open");
        let wrong = "x'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'";
        let _ = db.execute_batch(&format!("PRAGMA key = \"{wrong}\";"));
        assert!(
            db.query_row::<i64, _, _>("SELECT count(*) FROM mail", [], |r| r.get(0))
                .is_err(),
            "A WRONG KEY OPENED THE STORE"
        );
    }
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    println!("encrypted, wrong key refused, {:.1} MB\n", bytes as f64 / 1048576.0);

    // ── and the scan ─────────────────────────────────────────────────────
    let scan = (0..ROUNDS)
        .map(|_| {
            let db = rusqlite::Connection::open(&path).expect("open");
            key(&db);
            let started = Instant::now();
            let total: i64 = db
                .query_row("SELECT sum(length(body)) FROM mail", [], |r| r.get(0))
                .expect("a full scan");
            let took = started.elapsed();
            assert!(total > 0);
            took
        })
        .min()
        .unwrap_or(Duration::ZERO);

    println!("full scan of {:.1} MB   sqlite3mc/chacha20  {scan:>8.2?}", bytes as f64 / 1048576.0);
}
