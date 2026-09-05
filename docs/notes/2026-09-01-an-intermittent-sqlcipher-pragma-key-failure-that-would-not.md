# An intermittent SQLCipher `PRAGMA key` failure that would not reproduce on demand (2026-09-01, #710/#699)

Two unrelated sessions, weeks apart, each saw a `Database::open` panic once
with SQLCipher's own guard rail:

```
An error occurred with PRAGMA key or rekey. PRAGMA key requires a key of one
or more characters. ...
```

against the crate's fixed test key (`test_support::key()`, `[0x5a; 32]`
through BLAKE3) — never a wrong key, never reproduced twice in the same run.
Both sightings share a shape: a **full-workspace `cargo test` under ordinary
multi-session machine load** (#710) or **many test threads in one release
binary, looping to reproduce an unrelated bug** (#699's comment) — never a
targeted single test.

**What the code cannot do.** `db::configure` builds
`PRAGMA key = "x'<64 hex chars>'";` from `Subkey::to_hex()`, which always
emits exactly `KEY_BYTES * 2` ASCII hex digits from a 32-byte array — there is
no path from a valid key to a shorter string. Reading SQLCipher's own C source
(`sqlite3Pragma`'s `PragTyp_KEY` case, then `sqlcipher_cipher_ctx_key_derive`)
confirms the exact error text fires only when the **parsed pragma value's
length comes back zero** — the SQL text lost content before reaching the
codec, not a wrong key being rejected.

**What was tried.** ~51,000 concurrent `Database::open` calls, two
concurrency shapes, run under this workstation's ordinary multi-session load
(`/proc/loadavg` ~5.5 on 8 cores, several other sessions building and testing
at the time — the condition both sightings were under):

- 64 threads × 200 opens in one release-mode process (12,800 opens) — 0
  failures.
- 8 concurrent release-mode processes × 16 threads × 100 opens each, 3 rounds
  (38,400 opens) — 0 failures.

Neither shape reproduced the error once. `bundled-sqlcipher-vendored-openssl`
statically links a private copy of SQLCipher and libcrypto into *each* test
binary, so a race between two separate `cargo test` binaries cannot corrupt
shared memory the way a race between two threads in one process could — if
this is a real race rather than a one-off (resource exhaustion under a dozen
simultaneous sessions, or something adjacent to #699's own segfault), it is
more likely thread-local to one process than cross-process.

**Left as: reproduce, don't chase further.** Twice in months, both unrepeated
within their own run, and ~51k attempts across both plausible concurrency
shapes came back clean — the environmental branch of this issue's own
acceptance criteria. If it recurs: capture `POSTIO_LOG=debug` *at the moment
of failure* (neither prior sighting had it), and if it becomes reproducible on
demand even once, build under `-fsanitize=thread` rather than guessing from
the outside again.
