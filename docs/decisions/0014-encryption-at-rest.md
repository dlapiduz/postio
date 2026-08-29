# ADR 0014 — The local store encrypts itself

- **Status:** Accepted — **GO** (2026-08-25)
- **Date:** 2026-08-25
- **Issue:** [#143](https://github.com/dlapiduz/postio/issues/143), decided by
  the maintainer: Postio encrypts at rest itself — relying on OS disk
  encryption is not enough. This ADR chooses the mechanism and prices its
  consequences; [#297](https://github.com/dlapiduz/postio/issues/297) tracked
  writing it.
- **Related:** the keyring posture in `postio-imap/src/secret.rs` (no
  plaintext fallback, ever), the permissions hardening of #142,
  `docs/PRODUCT.md`'s privacy commitments, the perf budgets in CLAUDE.md.
- **Decision:** **SQLCipher for the database, per-blob AEAD for the blob
  store, one master key in the Secret Service keyring, no plaintext
  fallback.** Blob ids become *keyed* BLAKE3 hashes so deduplication
  survives without cross-store content correlation. New stores encrypt from
  first open; the pre-release migration path is drain-and-reencrypt. The
  README's mmap-backed memory numbers are a casualty and are re-measured.

---

## What "encrypt at rest" must mean here, stated honestly

Threat model first, because at-rest encryption is routinely oversold:

**Protected:** a stolen or discarded disk; a backup, rsync copy, or cloud
sync of `$XDG_DATA_HOME` that wanders; another user on a multi-user machine
who defeats or predates the `0700` permissions (#142); any reading of the
files while the keyring is locked or absent.

**Not protected, and the docs must say so:** an attacker running as the
user while the keyring is unlocked — they can read the key exactly as
Postio does; root on a live system; RAM, swap and hibernation images
(full-disk encryption remains *complementary*, not replaced — the privacy
page recommends both). SQLite's temp spill is closed off with
`PRAGMA temp_store = MEMORY`, which the store sets alongside its existing
pragmas.

## Q1 — The database: SQLCipher

The database is where the metadata, the threading, the FTS5 index and the
sync state live, and FTS5 is what rules most alternatives out: index
content is derived from message content, so an "encrypt the bodies, leave
the index" design leaks what it claims to protect.

**Decision: SQLCipher, via rusqlite's `bundled-sqlcipher-vendored-openssl`
feature.**

- Page-level encryption below SQLite's own machinery, so **FTS5, WAL, the
  migrations and every repository work unchanged** — the encryption is
  invisible above `Location::open`.
- Keying is one `PRAGMA key` with a **raw 32-byte key**
  (`x'…'` form) issued immediately after open, before any other statement,
  in the same `configure()` every pooled connection already passes through.
  Raw key on purpose: the keyring already stores high-entropy key material,
  so SQLCipher's passphrase KDF would be a thousand iterations of nothing.
- The vendored build keeps CI, the Flatpak and contributor machines
  hermetic. The `bundled-sqlcipher` + system-libcrypto variant is the
  recorded alternative if the vendored OpenSSL's build cost ever earns its
  removal; both are Apache-2.0-compatible for `deny.toml`.

**Rejected:**

- **fscrypt / filesystem-level** — the development box itself runs btrfs,
  which does not support fscrypt; it requires per-machine setup Postio
  cannot perform for the user; and "Postio encrypts at rest" would then be
  true only on some filesystems, which is the accidental posture #143
  existed to end.
- **Encrypting bodies but not the index** — leaks via FTS5, above.
- **Hand-rolled page or file encryption over SQLite** — reimplementing
  SQLCipher with new mistakes.
- **SQLite SEE** — proprietary; a licence cannot be a dependency of an MIT
  mail client.

## Q2 — The blob store: per-blob AEAD, and keyed ids

Raw messages and attachments live as content-addressed files. Two changes:

- **Content:** each blob is encrypted with **XChaCha20-Poly1305** (the
  RustCrypto `chacha20poly1305` crate — pure Rust, small, no new native
  dependency), a fresh random 24-byte nonce per blob, stored as a
  versioned header (`magic ‖ nonce ‖ ciphertext‖tag`). Streaming writes
  keep the 64 KiB chunking; the format field is what lets a future
  algorithm arrive without a flag day.
- **Ids:** a blob's name is currently the BLAKE3 hash of its plaintext.
  That preserves dedup but leaks *content equality*: anyone with the
  directory listing can confirm whether this mailbox contains a known
  file. BLAKE3 is a keyed hash natively, so ids become
  **BLAKE3 keyed by a store-specific subkey**. Dedup within the store is
  untouched (same content, same key, same id); cross-store correlation and
  known-file confirmation are gone. `BlobId` itself does not change shape.

## Q3 — One master key, in the keyring, with no plaintext fallback

One 32-byte master key per store, generated from the OS RNG on first open
and kept in the Secret Service keyring beside the account credentials —
the same `SecretStore` seam, the same locked-keyring UX, the same
**no-plaintext-fallback** rule `secret.rs` already enforces for passwords.
Subkeys are derived with BLAKE3's `derive_key` (contexts `"postio db"`,
`"postio blob content"`, `"postio blob id"`), so the database, the blob
contents and the blob ids are cryptographically separated without three
keyring entries.

Consequences taken deliberately:

- **A locked keyring means the mail does not open.** Startup reads the key
  through the runtime before the store opens; `SecretError::Locked` routes
  to the same unlock-and-retry surface a refused credential uses. There is
  no "open read-only anyway" — that would be the plaintext fallback this
  repository has refused everywhere else.
- **The keyring entry is part of the mailbox.** Copying the store
  directory to another machine without the key copies ciphertext. The
  privacy page documents this, and it is a *feature* of the posture, not a
  caveat — it is what "a backup that wanders is protected" means. (The
  store is also a cache: everything but drafts and the operation queue can
  be re-synced from the server, which is the practical recovery path.)
- **Tests run the real path.** `test_support` passes a fixed key, so every
  existing test exercises the encrypted store; nothing tests a plaintext
  configuration that no longer ships.

## Q4 — Migration, pre-release

New stores encrypt from first open. For the handful of existing
development stores: drain the operation queue, `sqlcipher_export()` the
database into an encrypted sibling, re-encrypt blobs streamwise, swap
directories atomically, and delete the plaintext only after the encrypted
store passes its integrity read. Drafts and the queue — the only local
truths — are what the drain-first ordering protects; everything else is
refetchable. No mail may be lost by a migration that dies half-way, which
the swap-last ordering is for.

## Q5 — What it costs, and the gate that decides

- **The mmap story dies.** `PRAGMA mmap_size` is meaningless over
  encrypted pages, so the README's "file-backed 256 MiB map" memory
  narrative goes with it; pages come through the page cache with a
  decrypt on read. The README numbers get re-measured, not hand-adjusted.
- **Per-page decrypt overhead** lands exactly where the budgets watch:
  startup < 500 ms, interaction < 16 ms, search < 100 ms. The existing
  benches are the gate — `store_reads`, `search_budget`, the startup
  trace — and the budgets do not move for this feature. First levers if a
  bench trips: `cache_size`, `cipher_memory_security = OFF` (its
  memory-wiping defence is not part of this threat model).
- **Build cost:** vendored OpenSSL is the heaviest new compile in the
  graph. sccache absorbs it machine-wide after the first build (#178) —
  *since #736*: as first landed, sccache was wired in as a rustc wrapper
  only and never saw the C compiler inside the openssl-src and
  libsqlite3-sys build scripts, so every fresh worktree recompiled OpenSSL
  and SQLCipher from source (~4 minutes at the pinned `jobs = 2`, 77% of a
  postio-storage build) — and sccache *cannot* absorb it, because
  openssl-src builds inside each target dir and sccache does no C path
  normalization. `scripts/cc-wrapper.sh`, wired in as `[env] CC` and
  fronting **ccache**, is what makes this bullet true.

## What would falsify this

- A bench showing SQLCipher cannot meet the 100 ms search budget on the
  120k-message index after the cache levers — that reopens Q1 toward the
  system-crypto build first and the fscrypt-where-available posture
  second, not toward shipping a blown budget.
- The keyed-id change breaking an assumption that blob names are
  recomputable from content alone — nothing in the tree does this today
  (`BlobStore` is the only namer), and the boundary check keeps outside
  crates from acquiring the habit.

---

## Consequences

- `postio-storage` gains the SQLCipher feature, the keyed-id and AEAD blob
  format, `temp_store = MEMORY`, and a store-key parameter threaded from
  the composition root; `postio-session` fetches the key through the
  existing `SecretStore` before `Database::open`.
- Implementation lands as three sequenced `ready` issues — key service,
  encrypted database + bench re-baseline, blob format + migration — plus a
  docs issue for the privacy page. Filed with this ADR.
- `deny.toml` inherits OpenSSL via `openssl-src`; licences already allowed.
- The README's performance section gets re-measured numbers and loses the
  mmap paragraph; CLAUDE.md's budgets are unchanged.
