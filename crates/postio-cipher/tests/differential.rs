//! The Rust provider against the one SQLCipher shipped with, in one process.
//!
//! This is the whole argument of the spike, and it is deliberately not a test
//! that the Rust code "works": a provider can round-trip its own output
//! perfectly and still write a store SQLCipher cannot read. What matters is
//! that the two agree, so the question asked here is always *the same
//! question, of both*.
//!
//! Two shapes of it. The primitives are compared directly — same key, same
//! salt, same page, byte for byte — which is deterministic and needs no
//! store. Then real stores are written by one provider and read by the
//! other, in both directions, which is the property a migration would
//! otherwise have to provide.
//!
//! One test function. SQLCipher's default provider is process-global state
//! and these cases swap it; two `#[test]`s would run on separate threads
//! against one pointer.

#![allow(unsafe_code)]

use postio_cipher::Provider;
use rusqlite::Connection;

/// SQLCipher 4's defaults, which is what Postio's store is written under.
const HMAC_SHA512: i32 = 2;
const HMAC_SHA256: i32 = 1;
const HMAC_SHA1: i32 = 0;

const KEY: &str = "correct horse battery staple";

/// Open an encrypted database at `path` and key it.
///
/// Keying is what forces SQLCipher to derive, encrypt and MAC — a connection
/// that is opened and never keyed does no crypto at all and would prove
/// nothing.
fn open(path: &std::path::Path) -> Connection {
    let connection = Connection::open(path).expect("a connection");
    connection
        .pragma_update(None, "key", KEY)
        .expect("the key pragma");
    connection
}

#[test]
fn the_rust_provider_and_openssl_agree_on_every_primitive_and_on_whole_stores() {
    let directory = tempfile::tempdir().expect("a directory");

    // ── the provider SQLCipher compiled in ───────────────────────────────
    //
    // Opening and keying one database first, because `default_provider` is
    // null until SQLCipher sets itself up on first use — and because
    // registration below takes a mutex that does not exist until SQLite has
    // initialised.
    let first = directory.path().join("written-by-openssl.db");
    {
        let connection = open(&first);
        connection
            .execute_batch("CREATE TABLE mail(id INTEGER PRIMARY KEY, subject TEXT)")
            .expect("a schema");
        connection
            .execute("INSERT INTO mail(subject) VALUES (?1)", ["a first message"])
            .expect("a row");
    }

    // SAFETY: SQLCipher has been initialised by the open above, so there is
    // a provider; it owns the table and keeps it for the life of the process.
    let was = unsafe { postio_cipher::current() };
    let shipped: &Provider = was.as_provider();
    // SAFETY: a `'static` in this crate, not yet registered.
    let ours: &Provider = unsafe { &*postio_cipher::table() };

    eprintln!(
        "  shipped provider: {:?}   this crate: {:?}",
        shipped.name(),
        ours.name()
    );
    assert_ne!(
        shipped.name(),
        ours.name(),
        "the provider in force is already this crate's, so the comparison \
         below would be a provider agreeing with itself"
    );

    // ── the sizes, which decide the shape of every buffer ────────────────
    assert_eq!(
        shipped.sizes(),
        ours.sizes(),
        "a key, IV or block size that disagrees is a store laid out \
         differently, whatever the cryptography does"
    );
    for algorithm in [HMAC_SHA1, HMAC_SHA256, HMAC_SHA512] {
        assert_eq!(
            shipped.hmac_sz(algorithm),
            ours.hmac_sz(algorithm),
            "digest size for algorithm {algorithm}"
        );
    }

    // ── the key derivation ───────────────────────────────────────────────
    //
    // Every algorithm, not only SQLCipher 4's default: a store written under
    // an older `PRAGMA cipher_kdf_algorithm` is still a store somebody has,
    // and a provider that quietly got one of the other two wrong would open
    // it and produce nonsense rather than refuse it.
    for algorithm in [HMAC_SHA1, HMAC_SHA256, HMAC_SHA512] {
        let (mut theirs, mut mine) = ([0u8; 32], [0u8; 32]);
        // SAFETY: both tables are live, and both buffers are the length
        // passed as `key_sz`.
        unsafe {
            assert_eq!(
                shipped.derive(
                    algorithm,
                    KEY.as_bytes(),
                    b"0123456789abcdef",
                    4096,
                    &mut theirs
                ),
                0
            );
            assert_eq!(
                ours.derive(
                    algorithm,
                    KEY.as_bytes(),
                    b"0123456789abcdef",
                    4096,
                    &mut mine
                ),
                0
            );
        }
        assert_eq!(
            mine, theirs,
            "PBKDF2 under algorithm {algorithm} derived a different key, so \
             every page of a store written this way is encrypted under a key \
             the other provider cannot reproduce"
        );
    }

    // ── the page MAC ─────────────────────────────────────────────────────
    //
    // Two inputs, because that is how SQLCipher calls it: the page
    // ciphertext, then the page number. A provider that concatenated them
    // the other way round would pass a one-input test and corrupt every
    // page.
    for algorithm in [HMAC_SHA1, HMAC_SHA256, HMAC_SHA512] {
        let size = ours.hmac_sz(algorithm) as usize;
        let (mut theirs, mut mine) = (vec![0u8; size], vec![0u8; size]);
        // SAFETY: both tables are live, and both buffers are `hmac_sz` long.
        unsafe {
            assert_eq!(
                shipped.sign(
                    algorithm,
                    &[0x5au8; 32],
                    b"page ciphertext",
                    b"\x01\x00\x00\x00",
                    &mut theirs
                ),
                0
            );
            assert_eq!(
                ours.sign(
                    algorithm,
                    &[0x5au8; 32],
                    b"page ciphertext",
                    b"\x01\x00\x00\x00",
                    &mut mine
                ),
                0
            );
        }
        assert_eq!(
            mine, theirs,
            "HMAC under algorithm {algorithm} differs, so every page would \
             fail its integrity check under the other provider"
        );
    }

    // ── the cipher itself ────────────────────────────────────────────────
    let key = [0x11u8; 32];
    let iv = [0x22u8; 16];
    let page = (0..4064u32).map(|n| n as u8).collect::<Vec<u8>>();
    let (mut theirs, mut mine) = (vec![0u8; page.len()], vec![0u8; page.len()]);
    // SAFETY: both tables are live; `out` is as long as `input` in both.
    unsafe {
        assert_eq!(shipped.transform(true, &key, &iv, &page, &mut theirs), 0);
        assert_eq!(ours.transform(true, &key, &iv, &page, &mut mine), 0);
    }
    assert_eq!(
        mine, theirs,
        "AES-256-CBC over a page-sized buffer differs between the two \
         providers, which is the store itself disagreeing"
    );

    // And each decrypts the other's ciphertext back to the plaintext.
    let mut back = vec![0u8; page.len()];
    // SAFETY: as above.
    unsafe {
        assert_eq!(ours.transform(false, &key, &iv, &theirs, &mut back), 0);
    }
    assert_eq!(back, page, "Rust could not decrypt OpenSSL's ciphertext");
    // SAFETY: as above.
    unsafe {
        assert_eq!(shipped.transform(false, &key, &iv, &mine, &mut back), 0);
    }
    assert_eq!(back, page, "OpenSSL could not decrypt Rust's ciphertext");

    // ── and now whole stores, which is what a person would actually lose ──
    postio_cipher::install().expect("the provider registers");
    // SAFETY: as above.
    let now = unsafe { postio_cipher::current() };
    let now: &Provider = now.as_provider();
    assert_eq!(
        now.name(),
        ours.name(),
        "registration did not take, so everything below is still OpenSSL \
         proving things about itself"
    );

    // The store written moments ago by OpenSSL, read through Rust.
    {
        let connection = open(&first);
        let subject: String = connection
            .query_row("SELECT subject FROM mail WHERE id = 1", [], |row| {
                row.get(0)
            })
            .expect("the row OpenSSL wrote, decrypted by Rust");
        assert_eq!(subject, "a first message");
    }

    // A store written through Rust...
    let second = directory.path().join("written-by-rust.db");
    {
        let connection = open(&second);
        connection
            .execute_batch("CREATE TABLE mail(id INTEGER PRIMARY KEY, subject TEXT)")
            .expect("a schema");
        connection
            .execute("INSERT INTO mail(subject) VALUES (?1)", ["a reply"])
            .expect("a row");
    }

    // ...and read back by the provider SQLCipher shipped with. Re-registering
    // the original elevates it to default again; it is already on the list,
    // so it is moved rather than re-initialised.
    // Safe: `Shipped` is SQLCipher's own table by construction, which is
    // what makes putting it back a safe call.
    postio_cipher::restore(was).expect("the shipped provider goes back");
    // SAFETY: as above.
    let restored = unsafe { postio_cipher::current() };
    let restored: &Provider = restored.as_provider();
    assert_eq!(
        restored.name(),
        shipped.name(),
        "the swap back did not take"
    );

    {
        let connection = open(&second);
        let subject: String = connection
            .query_row("SELECT subject FROM mail WHERE id = 1", [], |row| {
                row.get(0)
            })
            .expect("the row Rust wrote, decrypted by OpenSSL");
        assert_eq!(
            subject, "a reply",
            "a store written by the Rust provider is not readable by the one \
             every existing install has, which is the only result that would \
             make this unshippable"
        );
    }
}
