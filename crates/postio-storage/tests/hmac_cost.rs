//! What SQLCipher's per-page MAC costs, and what the alternative costs (#1216).
//!
//! # Why this exists
//!
//! A profile of a real 957 MiB mailbox put **45.9%** of all sampled CPU in
//! `sha512_block_data_order_avx2` and only **2.7%** in `aesni_cbc_encrypt`.
//! That ratio is the finding: the *cipher* is nearly free and the *MAC* is not.
//!
//! SQLCipher 4 authenticates every page with HMAC-SHA512. On x86-64 the SHA
//! extensions (`sha_ni`) accelerate SHA-1 and SHA-256 and **not** SHA-512, and
//! AES-NI accelerates the cipher — and CBC *decryption* parallelises across
//! blocks where encryption cannot. So on this class of CPU the one part of the
//! page path left running in plain software is the part that dominates.
//!
//! Measured on an i7-1160G7 (`sha_ni`, `avx512`, `vaes`), at page size:
//!
//! ```text
//! sha256           567 MB/s        aes-256-cbc   630 MB/s (encrypt; decrypt is far faster)
//! sha512           255 MB/s
//! ```
//!
//! # What this bench measures
//!
//! Two SQLCipher databases, same key, same content, differing only in
//! `cipher_hmac_algorithm`. A cold connection scans every page:
//!
//! ```text
//! HMAC_SHA512 (default): 1.38s to scan 253 MiB   (183 MB/s)
//! HMAC_SHA256:            810ms to scan 253 MiB   (312 MB/s)
//! ```
//!
//! **1.70x**, or 41% off the page-read path. Below the 2.2x the primitives
//! promise, because AES and SQLite's own work do not change.
//!
//! # What it does not say
//!
//! * **That the format should change.** `cipher_hmac_algorithm` is written into
//!   the database: switching it means re-encrypting an existing store through
//!   `sqlcipher_export()`. That is a migration, and ADR 0014 asks for the cache
//!   levers to be exhausted first — see `cache_pressure.rs`.
//! * **That it wins everywhere.** On a CPU *without* SHA-NI, SHA-512 is
//!   normally the faster of the two, being 64-bit work. This is a win on modern
//!   x86-64 and ARMv8, not a universal one, so a fixed choice trades one
//!   machine's gain for another's loss.
//! * **That HMAC-SHA256 is weaker.** It is not, at these sizes, in any sense
//!   that matters for authenticating a page.
//!
//! `#[ignore]`d: it writes two 253 MiB databases.
//!
//! ```text
//! cargo test -p postio-storage --test hmac_cost -- --ignored --nocapture
//! ```

use std::time::Duration;

use rusqlite::Connection;

/// How much data each database holds. Large enough that the scan is the
/// measurement rather than the connection setup.
const MIB: usize = 220;

/// Big enough to land in overflow pages, which is what makes the scan below
/// touch the whole file rather than a b-tree's worth of headers.
const ROW: usize = 4000;

/// The fixture key. Not a secret and not derived: this database exists for
/// eighteen seconds inside a temporary directory.
const KEY: &str = "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a";

fn cpu() -> Duration {
    let stat = std::fs::read_to_string("/proc/self/stat").expect("/proc/self/stat");
    let tail = &stat[stat.rfind(')').expect("the comm field ends") + 1..];
    let fields: Vec<&str> = tail.split_whitespace().collect();
    let utime: u64 = fields[11].parse().expect("utime");
    let stime: u64 = fields[12].parse().expect("stime");
    Duration::from_secs_f64((utime + stime) as f64 / 100.0)
}

/// A connection with the key applied, and the MAC set before anything is read.
fn open(path: &std::path::Path, hmac: Option<&str>) -> Connection {
    let connection = Connection::open(path).expect("open");
    // Before the key, which is what SQLCipher requires of this one.
    connection
        .execute_batch("PRAGMA cipher_memory_security = OFF;")
        .expect("memory security");
    connection
        .execute_batch(&format!("PRAGMA key = \"x'{KEY}'\";"))
        .expect("key");
    if let Some(algorithm) = hmac {
        // After the key and before the first read: it decides how pages are
        // authenticated, so it cannot be changed once one has been.
        connection
            .execute_batch(&format!("PRAGMA cipher_hmac_algorithm = {algorithm};"))
            .expect("hmac algorithm");
    }
    // A deliberately small cache: this measures cold pages, and a cache that
    // held the file would measure nothing.
    connection
        .execute_batch("PRAGMA journal_mode = WAL; PRAGMA cache_size = -2000;")
        .expect("pragmas");
    connection
}

fn build(path: &std::path::Path, hmac: Option<&str>) {
    let connection = open(path, hmac);
    connection
        .execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, data BLOB);")
        .expect("schema");
    let blob = vec![0x5au8; ROW];
    let transaction = connection.unchecked_transaction().expect("a transaction");
    {
        let mut statement = connection
            .prepare("INSERT INTO t (data) VALUES (?1)")
            .expect("prepare");
        for _ in 0..(MIB * 1024 * 1024 / ROW) {
            statement.execute([&blob]).expect("insert");
        }
    }
    transaction.commit().expect("commit");
    // Into the database proper, so the scan reads pages rather than the log.
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .expect("checkpoint");
}

/// Read every page on a cold connection, and return what it cost.
fn scan(path: &std::path::Path, hmac: Option<&str>) -> Duration {
    let connection = open(path, hmac);
    let before = cpu();
    // Reaching into the *last* byte of each blob is what forces the overflow
    // pages to be read, decrypted and verified. `sum(length(data))` reads the
    // record header and skips them: the first version of this bench "scanned"
    // 253 MiB in 150 ms, six times what this CPU can MAC, and the number was
    // meaningless.
    let total: i64 = connection
        .query_row(
            &format!("SELECT sum(unicode(substr(data, {}, 1))) FROM t", ROW - 1),
            [],
            |row| row.get(0),
        )
        .expect("scan");
    let burned = cpu().saturating_sub(before);
    assert!(total > 0, "the scan read nothing");
    burned
}

#[test]
#[ignore = "writes two 253 MiB databases; a bench, not a gate"]
fn hmac_sha256_is_cheaper_than_sha512_on_this_cpu() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let mut costs = Vec::new();

    for (label, hmac) in [
        ("HMAC_SHA512 (default)", None),
        ("HMAC_SHA256", Some("HMAC_SHA256")),
    ] {
        let path = directory
            .path()
            .join(format!("{}.db", label.split(' ').next().expect("a label")));
        build(&path, hmac);
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        // Twice: the first reading warms the file in the OS cache, so the
        // second is the crypto rather than the disk.
        let _ = scan(&path, hmac);
        let burned = scan(&path, hmac);
        eprintln!(
            "{label:>22}: {burned:?} to scan {} MiB ({:.0} MB/s)",
            size / 1024 / 1024,
            (size as f64 / 1_000_000.0) / burned.as_secs_f64()
        );
        costs.push(burned);
    }

    let (sha512, sha256) = (costs[0], costs[1]);
    assert!(
        sha256 < sha512,
        "HMAC_SHA256 was not cheaper ({sha256:?} against {sha512:?}). On a CPU \
         without the SHA extensions this is the expected answer, and the \
         conclusion in this file's docs does not hold there."
    );
}
