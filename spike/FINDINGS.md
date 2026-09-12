# Spike: SQLCipher's crypto in Rust, and no OpenSSL

**Result: it works, in both directions, and the store format does not
change.** A SQLCipher built with a Rust crypto provider reads stores written
by the OpenSSL build and the OpenSSL build reads its stores, and the binary
links no libcrypto at all.

This is a spike. It is not proposed for landing as it stands — see *What is
missing* at the end.

## What was measured

    ── linkage ──
    openssl  libcrypto entries: 1
    rust     libcrypto entries: 0

    ── each reads the other's store ──
    wrote  a.db  provider=openssl  cipher=AES-256-CBC  (encrypted)
    read   a.db  provider=rust     cipher=AES-256-CBC  -> "it works"
    wrote  b.db  provider=rust     cipher=AES-256-CBC  (encrypted)
    read   b.db  provider=openssl  cipher=AES-256-CBC  -> "it works"

`spike/run.sh` reproduces it from a clean checkout.

And `openssl-sys` leaves the graph outright:

    $ cargo tree -i openssl-sys      # with bundled-sqlcipher + the provider
    error: package ID specification `openssl-sys` did not match any packages

    $ cargo tree -i openssl-sys      # postio today
    openssl-sys v0.9.117
    └── libsqlite3-sys v0.38.2
        └── rusqlite v0.40.2

## Why this is possible at all

OpenSSL is not wired into SQLCipher; it is the fallback when nothing else is
named. From the amalgamation this build already compiles:

```c
#if !defined (SQLCIPHER_CRYPTO_CC) && !defined (SQLCIPHER_CRYPTO_LIBTOMCRYPT) \
 && !defined (SQLCIPHER_CRYPTO_OPENSSL) && !defined (SQLCIPHER_CRYPTO_CUSTOM)
#define SQLCIPHER_CRYPTO_OPENSSL
#endif
```

A provider is a table of eighteen function pointers. Five do work — `hmac`,
`kdf`, `cipher`, `random`, `add_random` — and the rest report sizes and
names. Every primitive behind those five was **already in this workspace's
dependency graph**, put there by other crates: `aes`, `cbc`, `hmac`, `sha2`,
`pbkdf2`, `getrandom`. The spike added one crate and no new third-party code.

It changes who computes the bytes, not what the bytes are. The format —
AES-256-CBC per page, a random per-page IV, an HMAC over the ciphertext,
PBKDF2 over the passphrase and the file salt — is decided in the amalgamation
above this table. **So there is no migration and no re-encrypt.**

## What it would buy

- **`openssl-src` out of the graph.** `docs/notes/2026-09-03-the-vendored-openssl-is-perl-not-c...`
  measured that at 28 s serial per cold tree, on the critical path into
  `postio-storage`, and proved no compiler cache can touch it: the cost is
  `perl` running `Configure`, not C.
- **The `atexit` workaround, and the crash class behind it.**
  `postio_storage::db::silence_openssl_atexit` exists because libcrypto
  registers exit handlers that tear it down under a thread still writing
  (#794, #699). A provider whose crypto is Rust in this binary has no C
  crypto DSO to finalise.
- **One fewer thing to keep hermetic** for the Flatpak and for contributors.

## Memory safety, honestly

The first write-up of this called the ownership problem "a hazard of the
runtime door" and moved on. That was too comfortable, and re-reading the crate
against it turned up three real defects and two lines of dead `unsafe`.

**`install` was a safe function with a safety precondition.** It documented
"call it after SQLite is initialised" and then called `sqlcipher_malloc`,
which reaches for a private heap and a mutex that `sqlite3_initialize` is what
creates. A safe function may not have preconditions — that is the whole
meaning of the word — and nothing stopped a caller from asking first. It
discharges it now: `sqlite3_initialize` is idempotent and thread-safe, it is
what runs SQLCipher's own `sqlcipher_extra_init`, and after it returns there
is no precondition left for a caller to get wrong.

**It leaked the table if registration was refused.** The allocation only
becomes SQLCipher's when `sqlcipher_register_provider` succeeds; on either
failure path this crate is still the only owner and now frees it.

**`postio_cipher_setup` took `&mut *provider` over the caller's memory.** A
reference asserts that what it points at is a valid value of its type, and
most of `Provider` is `Option<fn(..)>`, which has invalid bit patterns. It
is filled through raw pointers now, field by field, which asks nothing of the
memory and costs nothing — and the documented contract says so.

Two caveats on that last one, because overstating it would be its own
dishonesty. It never actually misbehaved: `sqlcipher_malloc` zeroes, so every
`Option` really was a valid `None`. And **Miri does not catch it**, with
default flags or strict ones — reference creation is not where it checks value
validity. So this was UB by the letter of the validity invariant, invisible to
the tool that exists to find such things, and correct in practice by the
allocator's habit rather than by anything the signature promised. Which is the
worst way for this class of bug to behave, and the argument for writing it the
way that needs no habit.

**And two `unsafe impl`s were dead.** `Send`/`Sync` for `Provider` were left
over from the version with a `static mut` table; nothing needed them once that
went. They are gone.

## What is actually checked, and what cannot be

`cargo miri test --lib` is clean over the six unit tests, including the one
that fills a table in over memory from `std::alloc::alloc` — deliberately not
`alloc_zeroed`. It has to be run outside the workspace, because this
repository's `.cargo/config.toml` sets a target runner for the headless
compositor and Miri sets its own; `spike/FINDINGS.md` is where that is written
down rather than a script, since it is a spike.

What no tool checks, and what is irreducible: the five functions that do work
are called by C with raw pointers and lengths. `hmac` writes the digest into a
buffer SQLCipher sized from `get_hmac_sz`; `cipher` writes `in_sz` bytes into
`out` because the amalgamation asserts they are equal on its side. Those are
contracts, not proofs, and they would be contracts for a provider written in
any language — it is the same trust OpenSSL's provider is extended two
branches away in the same file. What Rust buys here is the primitives, not the
boundary.

## What the spike found by doing rather than reading

**A provider registered at run time has to be `sqlcipher_malloc`'d.**
`sqlcipher_extra_shutdown` walks the provider chain and calls
`sqlcipher_free(provider, sizeof(sqlcipher_provider))` on every link, so the
first version — a `static` table in Rust — passed every test and then aborted
the process on the way out:

    test ... ok
    free(): invalid pointer

Worth knowing twice over. It is a hazard of the *runtime* door only: the
shipping path has SQLCipher allocate the table and call
`SQLCIPHER_CRYPTO_CUSTOM` merely to fill it, so ownership is never in
question. And it is a hazard in any language — it is about who allocated,
not about Rust.

**`libsqlite3-sys` is the whole of what is left.** Its build script hard-codes
four branches and has no way to say "the caller supplies the provider". The
patch is in `spike/libsqlite3-sys/` and is twenty lines; it reads one
environment variable and skips the OpenSSL discovery. That is a plausible
upstream PR, and it is the only part of this that is somebody else's code.

## What is missing before this could land

1. **Upstream, or a vendored `libsqlite3-sys`.** A patched build script in a
   `[patch.crates-io]` is what the spike used and is not what should ship.
2. **The differential test against real stores, in CI.** `tests/differential.rs`
   compares the two providers primitive by primitive and across whole stores,
   but only in a process that has both. A build with no OpenSSL cannot run
   it, so the gate has to be a corpus of stores written by the OpenSSL build
   and committed, or a CI job that builds both.
3. **Performance.** Unmeasured, and it matters: `aes` is a software
   implementation unless `aes-armv8`/AES-NI features are on, while OpenSSL
   picks AES-NI at run time. Postio decrypts every page it reads, so a
   regression here lands on the interaction budget rather than on startup.
   `postio_storage::test_support::counting` cannot see this — it is the same
   number of statements either way — so it wants a real bench.
4. **A decision about SHA-1.** The provider implements it because the vtable
   has three algorithm arms and a store written under an older
   `PRAGMA cipher_kdf_algorithm` is still a store somebody has. It is not
   used by anything this build writes.

## Where this branch stands

`scripts/check.sh` is clean on it, which took two entries a reviewer should
see rather than skim past:

- `postio-cipher` is in `check-lint-floor.py`'s `EXCEPTIONS` at `deny`, like
  the five crates already there. It cannot inherit the workspace's `forbid`
   — handing C a table of function pointers is what it is for — but every
  `unsafe` site still carries its own `#[allow(unsafe_code)]` and says which
  of SQLCipher's contracts it is relying on.
- Three `pub fn`s are in `uncalled-pub-fn-baseline.txt`. They exist to ask
  *any* provider the same question so two can be compared, and only the
  differential test has two.

## Files

- `crates/postio-cipher/` — the provider, its unit tests, and
  `tests/differential.rs`, which is the real argument
- `spike/libsqlite3-sys/libsqlite3-sys-crypto-custom.patch` — the build-script
  change
- `spike/prove/` — the two-binary cross-read harness
- `spike/run.sh` — reproduces the measurements above
