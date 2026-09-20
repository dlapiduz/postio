# Quickstart: proving the rebuilt store works

How to check this feature is real, in the order the work lands. Each section
is runnable on its own and corresponds to one user story in the spec.

## Prerequisites

```sh
cd ~/src/postio-worktrees/turso-store      # the feature worktree
cargo build -p postio-storage              # pulls turso; first build is slow
```

No system dependency is added and one is removed: `openssl-src` leaves the
graph. Confirm with:

```sh
cargo tree -i openssl-sys -e normal        # expect: did not match any packages
cargo tree -i rusqlite    -e normal        # expect: did not match any packages
```

## US1 — a mailbox that opens, encrypted

```sh
cargo run -p postio-session --example prove_cipher
```

Expect the store's own opening path to report `cipher = "aes256gcm"`, a schema
at head, a header that is not `SQLite format 3`, no table name or message text
in the raw bytes, and another key refused.

**This is the one that must never be skipped.** A mail store that is not
encrypted at rest is a defect, not a slower feature.

## US2 — mail arrives and can be read

```sh
cargo nextest run -p postio-storage                 # every repository
cargo nextest run -p postio-sync -p postio-session  # the write path
cargo test -p postio-app --test app_suite           # the window, end to end
```

Then, against a real account — on a **fresh store**, never the live one.

```sh
scripts/run-isolated.sh HEAD
```

`run-isolated.sh` is the safe way and the only one worth using here. It sets
its own `XDG_DATA_HOME`, `XDG_CONFIG_HOME` and `XDG_STATE_HOME`, so the store
it writes is `~/scratch/postio-run/state/data/postio/postio.db` and nothing it
does can reach the real mailbox, the real `config.toml`, or the
remote-images allowlist that #215 was about. It builds `--release` from a
pinned commit in a worktree of its own, so it is not whatever happened to be
on disk.

`POSTIO_LOG=debug scripts/run-isolated.sh` for a readable trace of the sync;
`scripts/run-isolated.sh --clean` to throw the store away afterwards.

**If you point it at the live store anyway, it refuses rather than damages
it.** A store the old engine wrote cannot be read by this one, and
`Store::open` says so in a sentence naming the remedy — sync again — and
leaves the file byte for byte (`storage/tests/refuses_a_foreign_store.rs`
proves both halves). That is worth knowing but is not a reason to try: the
engine is pre-1.0 and its encryption is unaudited, which is why the store
under test is a scratch one.

Add the account, let the first sync run, and check the four things a test
cannot:

1. the list fills, and scrolling deep into it stays instant;
2. a message opens and reads correctly — including one with accents, which is
   what `postio_model::fold` now has to get right on both sides;
3. searching for a word that is only in a body finds it, and the best match is
   at the top rather than merely the newest;
4. archiving one feels immediate.

## US3 — search that finds what it finds today

```sh
cargo nextest run -p postio-index
cargo run -p postio-index --example search_equivalence
```

The example is the acceptance for SC-002: one corpus, both engines, the same
query strings, and any message today's search returns that the new one does not
is a failure. It must include a query whose term differs from the text only by
diacritics — that is the case the engine does not handle and the application
must.

## US4 — it still feels instant

```sh
cargo nextest run -p postio-app --test app_suite -E 'test(startup_reads)'
cargo nextest run -p postio-storage -E 'test(list_statement_count)'
```

Both are counted rather than timed, so they say the same thing on any machine.
They must fail when a read becomes proportional to the mailbox — check that by
reading their failure messages, not by trusting the green.

## Where it is allowed to be worse than today

Two things, both recorded in the spec and neither a surprise if they show up:

- **Store size.** Bodies are text, not zstd: ~2.19x on the text axis by the
  figure `body.rs` records, plus the folded column for mail that carries
  accents.
- **What the cost gate can see.** It counts at the storage seam, not inside the
  engine, so it can see rows *returned* and not rows *examined*. The aggregate
  that hid a full scan in #1479 would not be caught by it.
