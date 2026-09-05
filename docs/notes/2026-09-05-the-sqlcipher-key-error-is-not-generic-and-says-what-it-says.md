# The SQLCipher key error is not generic, and it says what it says (2026-09-05, #710)

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

Exactly one shape: a key pragma whose **value string is zero-length**. An
empty *hex payload* is accepted — SQLCipher takes `x''` as the
four-character passphrase rather than as zero bytes — which is what
`db::configure`'s `format!` would produce from an empty key. So this error is
not reachable from any value of the key.

`the_key_pragma_error_means_an_empty_string_and_nothing_else` in
`storage_suite::concurrent_open` is that table, so nobody has to take this
note's word for it.

## Which makes it a signature rather than noise

`db::configure` builds the statement as
`format!("PRAGMA key = \"x'{}'\";", *hex)`, and `Subkey` is a fixed-size
`[u8; KEY_BYTES]` array — so the hex has a fixed length and the text has a
compile-time constant shape. `PRAGMA key = ""` is not a statement Postio can
write. Postio also issues no `PRAGMA rekey` anywhere in the workspace, so the
second sentence of the message is not about this codebase either.

A run reporting this is therefore reporting that SQLCipher parsed a statement
Postio did not write — which is #710's own first instinct (something at the
FFI boundary under heavy concurrent opens) and not a key-derivation problem.

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

The statement text, not the key. Nothing above reproduces the original, and
this note claims no fix — what it removes is three wrong places to look:
key derivation, descriptor exhaustion, and memory pressure.

Run the harness with `cargo nextest run -p postio-storage --test storage_suite
concurrent_open`; for pressure, build it and run the binary directly under a
`ulimit`, because the limit has to apply to the test process rather than to
cargo.
