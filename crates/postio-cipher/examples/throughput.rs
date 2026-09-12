//! What the Rust provider costs against the one SQLCipher shipped with.
//!
//! The spike's open question, and the one that could sink it: Postio decrypts
//! every page it reads, so a provider that is slower lands on
//! `docs/PRODUCT.md` §18's 16 ms interaction budget rather than on startup.
//!
//! ```sh
//! cargo run --release -p postio-cipher --example throughput
//! ```
//!
//! **`--release` is not optional.** RustCrypto and OpenSSL are both built at
//! the profile's opt level, and a debug build measures neither of them.
//!
//! # Why a ratio rather than a budget
//!
//! This machine routinely runs three build sessions at once, which is why
//! this project gates on counted work rather than on wall-clock. A count
//! cannot see this question at all — the same statements read the same rows
//! whoever decrypts them — so it has to be timed, and the honest way to time
//! it is back to back in one process, reporting the *floor* of several
//! rounds. A ratio between two implementations measured a microsecond apart
//! survives a noisy machine in a way an absolute number does not.
//!
//! Both providers are asked through the same vtable, which is the same door
//! SQLCipher itself uses.

// The whole of this file is asking a C vtable questions; every block says
// which contract it is relying on, the same as the library next door.
#![allow(unsafe_code)]

use std::time::{Duration, Instant};

use postio_cipher::Provider;

/// SQLCipher 4's defaults, read out of the amalgamation this links:
/// `default_page_size = 4096`, `PBKDF2_ITER 256000`, HMAC and KDF SHA-512.
const PAGE: usize = 4096;
const KDF_ITER: i32 = 256_000;
/// `FAST_PBKDF2_ITER`: what a raw `x'…'` key costs instead, which is what
/// Postio actually pays.
const FAST_KDF_ITER: i32 = 2;
const HMAC_SHA512: i32 = 2;
/// What Postio actually authenticates pages with: `PageMac::CURRENT` is
/// `Sha256` (`postio_storage::db`), chosen because SHA-512 has no instruction
/// on x86-64 while SHA-256 has SHA-NI. SHA-512 is measured beside it because
/// stores written before that change carry it and are still read.
const HMAC_SHA256: i32 = 1;

/// Rounds of each measurement; the floor of them is what is reported.
const ROUNDS: usize = 5;

fn floor_of(rounds: usize, mut body: impl FnMut() -> Duration) -> Duration {
    (0..rounds).map(|_| body()).min().expect("a round")
}

fn per_op(total: Duration, ops: usize) -> f64 {
    total.as_secs_f64() * 1e9 / ops as f64
}

fn main() {
    // Force SQLCipher to set itself up, so `current()` is the provider it
    // compiled in rather than null.
    let directory = tempfile::tempdir().expect("a directory");
    {
        let db = rusqlite::Connection::open(directory.path().join("warm.db")).expect("open");
        db.pragma_update(None, "key", "correct horse battery staple")
            .expect("key");
        db.execute_batch("CREATE TABLE t(x)").expect("a schema");
    }

    // SAFETY: the open above initialised SQLCipher, so there is a provider.
    let was = unsafe { postio_cipher::current() };
    let shipped: &Provider = was.as_provider();
    // SAFETY: this crate's comparison table, never registered.
    let ours: &Provider = unsafe { &*postio_cipher::table() };
    println!(
        "shipped provider {:?}   this crate {:?}\n",
        shipped.name(),
        ours.name()
    );

    let key = [0x11u8; 32];
    let iv = [0x22u8; 16];
    let page: Vec<u8> = (0..PAGE).map(|n| n as u8).collect();
    let mut out = vec![0u8; PAGE];

    // ── AES-256-CBC, one page ────────────────────────────────────────────
    //
    // The per-page cost, and the one that multiplies by every row read.
    // Encrypt and decrypt separately: CBC encryption is serial by
    // construction and decryption is not, so a provider can be even on one
    // and behind on the other.
    for (label, encrypting) in [("cipher encrypt", true), ("cipher decrypt", false)] {
        const OPS: usize = 20_000;
        let mut result = [0f64; 2];
        for (slot, provider) in [shipped, ours].into_iter().enumerate() {
            let took = floor_of(ROUNDS, || {
                let started = Instant::now();
                for _ in 0..OPS {
                    // SAFETY: a live table; `out` is as long as `page`.
                    unsafe { provider.transform(encrypting, &key, &iv, &page, &mut out) };
                }
                started.elapsed()
            });
            result[slot] = per_op(took, OPS);
        }
        report(label, result[0], result[1], Some(PAGE));
    }

    // ── HMAC-SHA512 over a page and its page number ──────────────────────
    //
    // Also per page. SHA-512 has no instruction on x86-64 the way AES does —
    // SHA-NI covers SHA-1 and SHA-256 only — so this is assembly against
    // portable Rust, and it is where a difference was most likely to be.
    for (label, algorithm) in [
        ("hmac sha256 (current)", HMAC_SHA256),
        ("hmac sha512 (older stores)", HMAC_SHA512),
    ] {
        const OPS: usize = 20_000;
        let mut digest = [0u8; 64];
        let mut result = [0f64; 2];
        for (slot, provider) in [shipped, ours].into_iter().enumerate() {
            let took = floor_of(ROUNDS, || {
                let started = Instant::now();
                for _ in 0..OPS {
                    // SAFETY: a live table; `digest` is 64 bytes, which is
                    // `get_hmac_sz` for the larger of the two.
                    unsafe {
                        provider.sign(algorithm, &key, &page, b"\x01\x00\x00\x00", &mut digest)
                    };
                }
                started.elapsed()
            });
            result[slot] = per_op(took, OPS);
        }
        report(label, result[0], result[1], Some(PAGE));
    }

    // ── PBKDF2, at both the iteration counts SQLCipher uses ──────────────
    //
    // 256,000 is `PRAGMA key` over a *passphrase*, and Postio never takes
    // that path: `db.rs` keys with the raw `x'…'` form, because the key is
    // high-entropy material out of the keyring rather than something a
    // person typed. A raw key skips the main KDF entirely and runs
    // `FAST_PBKDF2_ITER`, which the amalgamation defines as **2**, to derive
    // the HMAC key.
    //
    // Both are measured because the 256,000 figure is the one that looks
    // alarming and is not Postio's, and saying so is worth more than leaving
    // it out. #1479 measured the whole `store` phase at 30.8 ms on a real
    // store, which is by itself proof the 126 ms path is not being taken.
    for (label, iterations) in [
        ("pbkdf2 sha512 (256k)", KDF_ITER),
        ("pbkdf2 sha512 (raw key)", FAST_KDF_ITER),
    ] {
        let mut derived = [0u8; 32];
        let mut result = [0f64; 2];
        for (slot, provider) in [shipped, ours].into_iter().enumerate() {
            // Many rounds when it is two iterations, three when it is
            // 256,000: the same total time either way.
            let batch = if iterations > 1000 { 1 } else { 20_000 };
            let took = floor_of(3, || {
                let started = Instant::now();
                for _ in 0..batch {
                    // SAFETY: a live table; `derived` is the length passed
                    // as `key_sz`.
                    unsafe {
                        provider.derive(
                            HMAC_SHA512,
                            b"correct horse battery staple",
                            b"0123456789abcdef",
                            iterations,
                            &mut derived,
                        )
                    };
                }
                started.elapsed()
            });
            result[slot] = per_op(took, batch);
        }
        report(label, result[0], result[1], None);
    }

    whole_store(directory.path(), was);
}

/// A real store, read end to end by each provider.
///
/// The primitives above are the honest comparison and the wrong unit: what
/// decides whether this is shippable is what a *page read* costs, and a page
/// read is one decrypt plus one MAC plus everything SQLite does around them.
///
/// A fresh connection per measurement, because SQLite caches decrypted pages
/// — a second scan down the same connection decrypts nothing and would
/// measure the two providers as identical.
fn whole_store(directory: &std::path::Path, was: postio_cipher::Shipped) {
    let path = directory.join("scan.db");
    let key = "correct horse battery staple";

    // ~24 MB, which is thousands of pages and more than the default cache.
    {
        let db = rusqlite::Connection::open(&path).expect("open");
        db.pragma_update(None, "key", key).expect("key");
        // What `postio_storage::db::configure_as` sets, so the scan below is
        // the page path Postio actually runs.
        db.execute_batch("PRAGMA cipher_hmac_algorithm = HMAC_SHA256;")
            .expect("the page mac Postio uses");
        db.execute_batch(
            "CREATE TABLE mail(id INTEGER PRIMARY KEY, subject TEXT, body BLOB);
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 6000)
             INSERT INTO mail(subject, body)
               SELECT 'a subject line ' || i, randomblob(4000) FROM n;",
        )
        .expect("a store worth scanning");
    }
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

    /// Switching to Rust goes through `install`, not through `restore` of
    /// the comparison table.
    ///
    /// This is the footgun the runtime door carries, walked into while
    /// writing this very function: `table()` is a `Box::leak`, and handing it
    /// to `sqlcipher_register_provider` splices a non-SQLCipher allocation
    /// onto the chain SQLCipher frees at shutdown. The measurements printed
    /// fine and the process aborted on the way out with `free(): invalid
    /// pointer` — the same abort as the first version of the crate, from the
    /// other direction. `install` allocates the table SQLCipher's way.
    enum Use {
        Shipped,
        Rust,
    }
    let scan = |which: &Use| -> Duration {
        floor_of(ROUNDS, || {
            match which {
                // Safe by construction: a `Shipped` is SQLCipher's own.
                Use::Shipped => postio_cipher::restore(was).expect("the shipped provider takes"),
                Use::Rust => postio_cipher::install().expect("the rust provider takes"),
            }
            let db = rusqlite::Connection::open(&path).expect("open");
            db.pragma_update(None, "key", key).expect("key");
            db.execute_batch("PRAGMA cipher_hmac_algorithm = HMAC_SHA256;")
                .expect("the page mac Postio uses");
            let started = Instant::now();
            let total: i64 = db
                .query_row("SELECT sum(length(body)) FROM mail", [], |r| r.get(0))
                .expect("a full scan");
            let took = started.elapsed();
            assert!(total > 0);
            took
        })
    };

    let theirs = scan(&Use::Shipped);
    let mine = scan(&Use::Rust);
    // Leave the shipped provider in force, so nothing after this measures
    // something else by accident.
    postio_cipher::restore(was).expect("back");

    println!(
        "\nfull scan of {:.1} MB   openssl {:>8.2?}   rust {:>8.2?}   rust/openssl {:>5.2}x",
        bytes as f64 / 1024.0 / 1024.0,
        theirs,
        mine,
        mine.as_secs_f64() / theirs.as_secs_f64()
    );
}

/// One line: what each provider cost, and what the ratio means.
fn report(label: &str, shipped: f64, ours: f64, bytes: Option<usize>) {
    let ratio = ours / shipped;
    let rate = |ns: f64| match bytes {
        Some(n) => format!("{:>7.0} MB/s", n as f64 / ns * 1000.0),
        None => "            ".to_string(),
    };
    println!(
        "{label:<22} openssl {shipped:>9.1} ns {}   rust {ours:>9.1} ns {}   rust/openssl {ratio:>5.2}x",
        rate(shipped),
        rate(ours),
    );
}
