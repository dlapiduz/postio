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

## Performance: measured, and it is the one real cost

`cargo run --release -p postio-cipher --example throughput` asks both
providers the same questions through the same vtable, back to back in one
process, and reports the floor of five rounds. A ratio measured a microsecond
apart survives a shared machine in a way an absolute number would not.

| | openssl | rust | rust/openssl |
|---|---:|---:|---:|
| cipher encrypt (4 KiB page) | 3825 ns | 3965 ns | **1.04x** |
| cipher decrypt (4 KiB page) | 1166 ns | 971 ns | **0.83x** |
| hmac sha256 — what Postio uses | 9010 ns | 16801 ns | **1.86x** |
| hmac sha512 — older stores | 6827 ns | 9326 ns | 1.37x |
| pbkdf2 sha512, raw key — what Postio pays | 2777 ns | 1700 ns | **0.61x** |
| pbkdf2 sha512, 256k — *not* Postio's path | 127 ms | 150 ms | 1.18x |
| **full scan of a 26.4 MB store** | **141 ms** | **167 ms** | **1.18x** |

**AES is a non-issue, and I had it wrong.** The first write-up said `aes` is
"software unless the AES-NI features are on". It is not: `aes` 0.8 dispatches
to AES-NI at run time on x86-64 through `cpufeatures`, this machine has it,
and CBC *decrypt* — the direction that matters, since reading is what Postio
does — is 17% **faster** than OpenSSL.

**The 256,000-iteration PBKDF2 is not Postio's cost either.** `db.rs` keys
with the raw `x'…'` form, because the key is high-entropy material from the
keyring rather than something a person typed, and a raw key skips the main KDF
for `FAST_PBKDF2_ITER`, which is 2. That path is 39% faster in Rust. #1479's
30.8 ms `store` phase was already telling me the 127 ms path was not being
taken.

**The real cost is HMAC, and it is 1.86x.** It is also the dominant per-page
cost — the MAC over a 4 KiB page costs more than the AES over it, either way
round. End to end, a full scan of a real store is **1.18x**, because crypto is
only part of what a page read does.

### The caveat that decides whether that number means anything

**This machine has no SHA-NI.** `/proc/cpuinfo` has `aes` and `avx2` and not
`sha_ni` — so for SHA-256, OpenSSL uses AVX2 assembly and `sha2` falls back to
portable Rust. On a machine with the extension both would use it: `sha2`
0.10.9 autodetects it (`cpufeatures::new!(shani_cpuid, "sha", …)` in
`sha256/x86.rs`), as does OpenSSL.

Which matters twice over, because `PageMac::CURRENT` is `Sha256` *precisely
because* of SHA-NI — `postio_storage::db` says so, and cites a profile
putting 45.9% of samples in `sha512_block_data_order_avx2`. So this
measurement is the **pessimistic** case for the algorithm Postio picked, taken
on hardware that algorithm was not picked for. Worth noticing separately: on
this box SHA-512 is faster than SHA-256 for *both* providers, which is exactly
what that same doc comment predicts for a CPU without the extensions.

Nobody should conclude from 1.86x without re-running this on a SHA-NI
machine. The command is one line and the harness is committed.

## What is missing before this could land

1. **Upstream, or a vendored `libsqlite3-sys`.** A patched build script in a
   `[patch.crates-io]` is what the spike used and is not what should ship.
2. **The same measurement on a SHA-NI machine**, per above. It is the
   difference between "1.18x end to end" and "no measurable difference", and
   it is the only open question that could still sink this.
3. **The differential test against real stores, in CI.** `tests/differential.rs`
   compares the two providers primitive by primitive and across whole stores,
   but only in a process that has both. A build with no OpenSSL cannot run
   it, so the gate has to be a corpus of stores written by the OpenSSL build
   and committed, or a CI job that builds both.
4. **A decision about SHA-1.** The provider implements it because the vtable
   has three algorithm arms and a store written under an older
   `PRAGMA cipher_kdf_algorithm` is still a store somebody has. It is not
   used by anything this build writes.

## Can the gap be closed? Two answers, depending on what is fixed

### Inside SQLCipher's format: yes, and with something already here

`examples/hmac_backends.rs`. The reason `sha2` loses is narrow and worth
naming: its SHA-256 backends are `soft`, `soft_compact`, `aarch64`,
`loongarch64_asm` and `x86` — and the x86 one is **SHA-NI only**. There is no
AVX2 path. So on a CPU with AVX2 and no SHA-NI, OpenSSL runs hand-written
assembly and `sha2` runs portable Rust. That is the entire 1.86x, and it is
not a statement about the language.

| HMAC-SHA256, 4 KiB page | | vs openssl |
|---|---:|---:|
| openssl | 8926 ns | 1.00x |
| rustcrypto `sha2` | 16735 ns | 1.87x |
| `sha2` with the `asm` feature | 14688 ns | 1.48x |
| `sha2`, `-C target-cpu=native` | 13320 ns | 1.44x |
| **`ring`** | **9044 ns** | **1.01x** |

`ring` is parity, and it is **already in this workspace's dependency graph** —
rustls pulls it in for every TLS connection Postio makes, so it would add no
third-party code at all. It is not pure Rust: it is BoringSSL's assembly in a
Rust wrapper. Which is the honest description of how you get parity at
SHA-256 on a machine without the instruction.

### Outside it: pure Rust is not on par, it is several times faster

`examples/page_schemes.rs`, per 4 KiB page, sealed and opened again. The unit
is the *page*, not the primitive, because SQLCipher's path is two passes —
cipher, then MAC over the ciphertext — and an AEAD is one. A scheme can win by
doing less work rather than the same work faster, and these do.

| scheme | open (a page read) | vs today |
|---|---:|---:|
| aes-cbc + hmac-sha256 (openssl) — **today** | 10494 ns | 1.00x |
| aes-cbc + hmac-sha256 (rustcrypto) | 17747 ns | 1.69x |
| aes-cbc + blake3 keyed (rustcrypto) | 2487 ns | **0.24x** |
| chacha20-poly1305 (rustcrypto) | 3497 ns | **0.33x** |
| xchacha20-poly1305 (rustcrypto) | 3602 ns | **0.34x** |
| aes-256-gcm (rustcrypto) | 1802 ns | **0.17x** |

All pure Rust, no C anywhere, on the machine with no SHA-NI. AES-256-GCM is
**5.8x faster** than what Postio pays today; ChaCha20-Poly1305 is 3x.

**The one to want is probably XChaCha20-Poly1305, not the fastest.** AES-GCM's
96-bit nonce has to be unique per encryption under a key, and a page store
rewrites the same page indefinitely — random nonces there have a birthday
bound, and a repeat does not merely leak a page, it leaks the authentication
key. The 192-bit nonce is what makes a random one safe with no counter to
keep, which is exactly the reasoning `postio-storage`'s own manifest already
records for sealing blobs with it. 0.34x with a nonce discipline a page store
can actually hold beats 0.17x with one it cannot.

### And the catch, which is the whole of the difficulty

**None of that is reachable through SQLCipher.** The provider vtable supplies
primitives; the *format* — CBC, a per-page IV, a separate MAC over the
ciphertext — is decided above it in the amalgamation. Changing the scheme
means leaving SQLCipher, and then the options are a different C library
(SQLite3 Multiple Ciphers has ChaCha20-Poly1305 as its default, but its
crypto is C, so it answers the speed question and not the Rust one) or a
page-encrypting VFS written here, which is the WAL, the journal, page one's
salt and atomic writes — a much larger and more dangerous surface than a
vtable of five functions.

The store is a cache of the server and the project has no backwards
compatibility to keep, so the *migration* is a resync rather than a problem —
`postio_storage::db::PageMac` already says as much for its own format change.
The risk is all in the page layer, not in the data.

## If the store were being chosen from scratch

**Page-level, not column-level.** Encrypting values instead of pages would
take FTS5 with it, and search over subjects and bodies is most of what this
application is. It also leaks the shape of the mailbox — row counts, sizes,
timestamps — which is the thing an encrypted store is for.

**An AEAD with a per-page one-time key, not a cipher plus a separate MAC.**
One pass rather than two is where the 3-6x measured above comes from, and the
per-page subkey is what makes a *random* nonce safe on data that is rewritten
indefinitely. A page store cannot keep a counter.

**And that scheme already exists, with a maintained implementation.**
SQLite3 Multiple Ciphers' default is sqleet's ChaCha20-Poly1305: a one-time
key per page derived from the encryption key, the page number and a 16-byte
nonce, with a 16-byte Poly1305 tag — 32 reserved bytes per page, which is 0.8%
of a 4 KiB one. That is the construction this spike would have specified, and
specifying it is not the hard part.

It is also a **VFS over unmodified SQLite** rather than a patched fork: SQLite
removed the codec API in 3.32, so this is the only architecture left, and
sqlite3mc is its reference implementation. Its crypto is self-contained C —
`rijndael.c`, `chacha20poly1305.c`, `fastpbkdf2.c` — a couple of thousand
lines rather than OpenSSL, MIT licensed, and with no libcrypto to link or to
tear down at exit.

**What not to do is write the page layer here.** The crypto in that layer is
five functions; the layer itself is the WAL's frame format, the rollback
journal, page one's salt, reserved-byte accounting, and torn writes. sqlite3mc
is what re-deriving that looks like when it is done carefully, and it is not a
dependency swap. Pure Rust would be the fastest of the options measured — and
that is the wrong axis to optimise for a mailbox.

**Keep the key handling exactly as it is.** A 256-bit key from the OS keyring,
handed over raw, with no passphrase KDF on the open path. It is why the
`store` phase is tens of milliseconds and not the 127 ms a 256,000-iteration
PBKDF2 costs, and it is the one part of the current design this spike found
nothing to improve.

## One more footgun, walked into while measuring

`restore` used to take a bare `*mut Provider`, and the crate also handed out
`table()`, a `Box::leak` for comparing against. Nothing but a doc comment
stood between them, and `examples/throughput.rs` put the second into the
first — which splices an allocation SQLCipher did not make onto the chain it
frees, and aborts at exit with `free(): invalid pointer`. The measurements
printed perfectly first.

Two pointers of one type meaning different things about ownership is the
whole of that bug, so there is a type each now: `current()` returns a
`Shipped`, `restore` takes one, and `restore` is a **safe** function because
a `Shipped` cannot be built out of anything else. The runtime door is the
part of this crate that needs the care, and this is the second time it has
proved it.

## Files

- `crates/postio-cipher/` — the provider, its unit tests, and
  `tests/differential.rs`, which is the real argument
- `spike/libsqlite3-sys/libsqlite3-sys-crypto-custom.patch` — the build-script
  change
- `spike/prove/` — the two-binary cross-read harness
- `spike/run.sh` — reproduces the measurements above
