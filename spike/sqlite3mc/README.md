# Postio on SQLite3 Multiple Ciphers

A spike branch: the application, with its store encrypted by
**ChaCha20-Poly1305** through SQLite3 Multiple Ciphers instead of AES-CBC +
HMAC through SQLCipher and OpenSSL.

## Running it

```sh
spike/sqlite3mc/setup.sh          # fetches and verifies the amalgamation
cargo run -p postio-app
```

**The branch does not build until `setup.sh` has run.** That is deliberate.
The alternative is a 13 MB amalgamation committed to git; upstream publishes
signed `SHA256SUMS`, which the script checks, so fetching is better provenance
as well as a smaller diff. It drops the result into `vendor/libsqlite3-sys`,
which the workspace's `[patch.crates-io]` points at and `.gitignore` ignores.

## The encryption, proved through the door the application uses

```sh
cargo run -p postio-session --example prove_cipher
```

```
cipher         = "chacha20"
journal_mode   = "wal"
schema objects = 113
header         = [34, 28, ac, 80, d8, 2a, b4, b0, 74, be, 5a, cf, d6, 44, 3a, 9d]
plaintext scan = no table names in 458752 bytes
another key    = refused

OK — the application's store is ChaCha20-Poly1305 and encrypted
```

That calls `postio_session::open_store_at` — what `postio-app`'s startup
calls — and then asks the file. The migrations ran, the header is the random
salt rather than `SQLite format 3`, no table name appears anywhere in the raw
bytes, and a different key is refused.

## What it is worth

**1.84x on the page path.** Same workload, same journal mode, same machine,
floor of five fresh-connection full scans: 5.20 ms/MB for SQLCipher and
OpenSSL against **2.82 ms/MB** here. One pass instead of two is where it comes
from — ChaCha20-Poly1305 authenticates the page as part of encrypting it,
where SQLCipher encrypts and then MACs.

**`openssl-sys` leaves the dependency graph.** `cargo tree -i openssl-sys`
answers *did not match any packages*, so `openssl-src`'s 28 s of serial,
uncacheable `perl` per cold tree is gone, and so is the `atexit` workaround
in `db.rs` and the #794/#699 crash class behind it — this build's store
crypto never enters libcrypto.

Be precise about that last one: `ldd` on the binary still finds
`libcrypto.so.3`, because GTK and WebKit's network stack load it. What
changed is that **Postio's mail no longer goes through it**, and that nothing
in this workspace builds it.

## What was removed, and why that is allowed

ADR 0014 Q4's plaintext-to-encrypted migration, and its tests. It existed to
carry a store written before encryption was added; this build cannot read a
SQLCipher store at all, and does not try — it opens chacha20 stores and
refuses others rather than opening some and mis-reading others.

That is on the maintainer's own instruction for this branch, and it is what
`postio_storage::db::PageMac`'s doc comment already said a format change here
means: the store is a cache of the server, so a store in another format is
rebuilt by resyncing.

Also gone: `page_mac`'s two cases and `hmac_cost`, which chose SHA-256 over
SHA-512 as the page MAC. Under an AEAD there is no separate MAC to choose.

## The one surprise, which no feature comparison would have told you

> Setting key not supported for in-memory or temporary databases.

**sqlite3mc refuses `PRAGMA key` on an in-memory database, where SQLCipher
accepted it.** Three pool tests failed on it. `Location::open` now skips
keying for `Location::Memory`, which is safe on its own terms — there is no
file to protect and nothing outlives the process — and every real store is a
file. But it is a behavioural difference that had to be found by running the
suite.

## Where it stands

| suite | |
|---|---|
| `postio-storage` | 573 passed, 0 failed |
| `postio-app`, `postio-session`, `postio-runtime`, `postio-index`, `postio-sync`, `postio-storage` | 1404 passed, 0 failed |

## What this is not

Ready to land. `[patch.crates-io]` at a vendored path is a spike's
arrangement; shipping wants sqlite3mc support upstream in `libsqlite3-sys`
(rusqlite#1726 is the open PR) or a published fork. And the format change is
a decision about everybody's store, not a dependency bump.
