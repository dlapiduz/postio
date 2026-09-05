# The SQLCipher key error does not mean what it says (2026-09-05, #710)

> **Corrected the same day.** This note first argued that the message means a
> zero-length key *and nothing else*, and concluded from that that a run
> reporting it must have executed a statement Postio did not write. The first
> half was measured and is half of the truth; the second does not follow, and
> it pointed at the most expensive wrong thing available — memory corruption
> at the FFI boundary. The section below marked **the other way in** is the
> counter-example.

#710 is an unreproduced flake: one `PRAGMA key` failure in one
full-workspace run, in `test_support::temp()`, which opens a database with a
constant key.

```
An error occurred with PRAGMA key or rekey. PRAGMA key requires a key of one
or more characters. PRAGMA rekey can only be run on an existing encrypted
database. …
```

The branch's own harness commit, and the issue comment beside it, both record
that this text is "SQLCipher's generic text for *any* failed key pragma,
printed whatever the cause" — and therefore that the title's reading, that the
key was empty, was never the likely one. Measured against the vendored
SQLCipher this workspace links, **both halves of that are wrong**, and the
correction matters more than the original claim did: "generic" is what sends
the next investigation away from the statement text, which is the one place
left to look.

## What actually produces it

| Statement | Result |
|---|---|
| `PRAGMA key = "";` | **the reported error** |
| `PRAGMA key = '';` | **the reported error** |
| `PRAGMA key = "x''";` | accepted |
| `PRAGMA key = "x'5a5a5a5a'";` | accepted |
| a *wrong* key | accepted here, and fails later on a page that will not decrypt |

One shape *of key*: a key pragma whose **value string is zero-length**. An
empty *hex payload* is accepted — SQLCipher takes `x''` as the
four-character passphrase rather than as zero bytes — which is what
`db::configure`'s `format!` would produce from an empty key. So the error is
not reachable from any *value* of the key.

`an_empty_key_string_produces_the_reported_error` in
`storage_suite::concurrent_open` is that table, so nobody has to take this
note's word for it.

## The other way in, which is the one that matters

The table above says what a *key* can do. It is not what the pragma handler
tests. From the vendored amalgamation:

```c
rc = sqlite3_key_v2(db, zDb, zKey, n);
...
if( rc==SQLITE_OK && n!=0 ) { ...ok... }
else { sqlite3ErrorMsg(pParse, "An error occurred with PRAGMA key or rekey. ..."); }
```

`n == 0` is the empty key. **`rc != SQLITE_OK` is everything else**, and
`sqlcipherCodecAttach` has several of those:

| Return | When |
|---|---|
| `sqlcipher_init_error` | SQLCipher's one-time init (`sqlcipher_extra_init`) has failed |
| `SQLITE_MISUSE` | no key, or an invalid database index |
| whatever `sqlcipher_codec_ctx_init` gave | the codec would not start for this connection |

`tests/key_pragma_failure.rs` pins the third with a **valid 32-byte key**:
`PRAGMA cipher_default_page_size = 3` is stored by `atoi` and validated
nowhere, and `sqlcipher_codec_ctx_init` then refuses it — and the pragma
prints the message that names the key. Its own binary, because that pragma is
a SQLCipher process global and `storage_suite` forbids those.

The first row is the one that fits #710's shape. `sqlite3_initialize` retries
`sqlcipher_extra_init` on its next call, so a transient failure — an
allocation, the crypto provider, or its first draw of randomness — fails the
opens inside its window and lets later ones through. That is the
cluster-then-recover the issue keeps recording, and `db.rs` already predicted
it in passing: the `silence_openssl_atexit` comment says a libcrypto that will
not initialise "will fail loudly at the first `PRAGMA key`".

## The reason is already being printed

SQLCipher logs before it returns, at ERROR, and on Linux its default target is
**stderr**. The counter-example above prints:

```text
ERROR CORE cipher_page_size not a power of 2 and between 512 and 65536 inclusive
ERROR CORE sqlcipher_codec_ctx_init: error 1 returned from sqlcipher_codec_ctx_set_pagesize with 3
ERROR CORE sqlcipherCodecAttach: context initialization failed, forcing error state with rc=1
```

So every occurrence of #710 has had its diagnosis a few lines above the panic,
and three passes at the issue quoted the panic without the lines around it.
`cargo test`'s per-test capture does not hold these — they are `fprintf` from
C, not Rust's `eprintln!` — so they land in the run's own output, interleaved.
**Next occurrence: keep the whole run output, not the failing test's block.**

## Which makes it a signature rather than noise

`db::configure` builds the statement as
`format!("PRAGMA key = \"x'{}'\";", *hex)`, and `Subkey` is a fixed-size
`[u8; KEY_BYTES]` array — so the hex has a fixed length and the text has a
compile-time constant shape. `PRAGMA key = ""` is not a statement Postio can
write. Postio also issues no `PRAGMA rekey` anywhere in the workspace, so the
second sentence of the message is not about this codebase either.

A run reporting this is therefore not reporting a key Postio could have
written. What it *is* reporting is the paragraph above: `sqlite3_key_v2`
returned an error. The earlier version of this note concluded instead that
SQLCipher must have parsed a statement Postio did not write, and sent the
reader to the FFI boundary looking for memory corruption. That conclusion
needed the pragma to test only `n`, and it tests `rc` as well.

## Two hypotheses ruled out, and how to tell them apart

The suggested next step on the issue was to run the stress harness on Linux
under file-descriptor and memory pressure. Done, on the Linux box, and
**neither produces the reported error**. Both fail distinguishably, which is
the useful part: a future sighting can be classified from its error code
alone.

| Pressure | How | What it produces |
|---|---|---|
| File descriptors | `ulimit -n 40` | `CannotOpen`, extended code **14**, "unable to open database file" |
| Memory | `ulimit -v 500000` | "out of memory", and "SQL logic error" |
| Neither | `ulimit -n 64`, 16 threads × 12 opens | clean |

`ulimit -n 64` is still clean, so the descriptor threshold is between 40 and
64 for this harness — far below anything a real run approaches, and the
failure it gives is not #710's in any case.

## What is left

Nothing here reproduces the original. What is removed is four wrong places to
look — key derivation, descriptor exhaustion, memory pressure, and a corrupted
statement — and what is added is one right one: SQLCipher's own log lines,
which are printed on every occurrence and have never been read.

`Error::CipherUnavailable` now carries that, so the sentence a caller sees
names the codec rather than the key.

Run the harness with `cargo nextest run -p postio-storage --test storage_suite
concurrent_open`; for pressure, build it and run the binary directly under a
`ulimit`, because the limit has to apply to the test process rather than to
cargo.
