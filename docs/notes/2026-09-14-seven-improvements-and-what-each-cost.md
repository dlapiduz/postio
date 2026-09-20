# Seven improvements, and what each cost

2026-09-14, on `feature/turso-store`. Asked for ten things worth doing from
an architectural, quality, engineering and performance standpoint, the
maintainer picked seven to do now. This is what each became, with the
number that says it worked, so the next reader knows what changed and what
was deliberately not done.

## 1. The body search index left the sync write path

`set_body` wrote the body's full-text row inside the body's transaction and
the backfill wrote it once more just after -- every body commit updated the
tantivy index on the lane that was syncing, where whichever commit came next
could inherit a segment merge measured in seconds while every folder queued
behind it waited (`2026-09-13-a-slow-pass-stops-every-folder-behind-it.md`).
Neither writes the row now. A stored body with no row is the queue
(`messages_missing_body_text`), and `postio_session::spawn_body_indexer`
drains it: a catch-up pass at start, then one pass half a second after each
burst of `BodyLoaded`, hundreds of bodies under one background permit and one
transaction. `EventSink::subscribe` lets a component wired from a `Wiring`
listen on the hub behind its sink, which is what let the macOS boundary --
which had no body indexer at all and relied on the fetch's write -- run the
same task. A body is searchable a moment after it lands rather than in the
same instant.

## 2. Bodies carry a parser version, and are zstd again

A body is fetched once and its raw bytes are not kept, so a parser fix cannot
reach a stored body: the three messages the lenient quoted-printable reading
fixed stayed empty on the account that found them. `messages.body_parsed_with`
stamps every body with `postio_model::mime::PARSER_VERSION`, and the backfill
seed selects again the rows a lower version left carrying the decode caveat
-- and only those. Bumping the constant is the whole of "re-process what this
fix changes".

The columns are zstd per row again (`body_codec`): a frame when it is
smaller, the text when it is not, told apart by the frame's magic so a store
written before it reads without a migration. 38,000 bytes of newsletter
markup store under a quarter of that; the spec's accepted 2.19x growth on the
text axis is gone.

## 5. Interaction under load is a counted gate

`test_support::gate_log` records every request and grant the write gate
makes, and `interactive_under_load.rs` reads it back after a real first sync
and a real resync with a flag written mid-pass: zero background units between
the keystroke's request and its grant, at least one unit after it, at least
one permit per 25-message unit. Counts, so the same answer on a shared runner
as on a workstation -- the stopwatch version (`resync_interactive_write.rs`)
stays for the loss it reproduces.

## 6. A dead web process fails a test at once, and the WebKit suites run apart

`postio_gtk::web_process` hears `web-process-terminated` on every view the
crate builds; the suites' wait helpers take the pending death and fail
naming it, in one turn of the loop rather than at nextest's 240 s cap. The
reader binary proves it by killing its own web process. `Tests` runs the
workspace without the two WebKit-heavy crates; `Tests (GTK widgets)` and
`Tests (application)` run them apart, each with its own build cache, so a
runner flake re-runs one job. **The two new jobs are not required checks
until the ruleset names them.**

## 8. Diagnostics are built in

`postio-diag census | encoding | shape | pending`, built with the
application, key from the keyring: what a store holds and costs, the bodies
carrying the decode caveat and what their parts declared, rows per page with
and without the bodies, and what the background lanes still owe. Read-only by
promise, counts and header tokens only. The three examples it folds are
gone. The engine's `sync finished` line and the indexer's pass carry
`elapsed_ms`.

## 9. The uncalled-pub-fn baseline shrank, and one entry was wired

An audit of all 113 entries (every caller in tests, benches, doc examples and
prose, with a class per row) found eleven with no caller anywhere and a
sibling doing the job; they are deleted. `prune_settled` -- "the sweep that
stops the table growing without bound", written, tested, never run -- is
wired into housekeeping with a thirty-day retention. 113 becomes 101. The
rest are test-support in production code (83), public API used by benches
and doc examples (4), and seams the ADRs document (13); the baseline header's
rule stands: retire one per landing, wire it or delete it.

## 10. The dependency graph is guarded

`cargo machete` found two declared-and-unused dependencies in 20 crates;
both are gone, and `check-unused-deps.py` refuses the next one under
`check.sh`, with the boundaries job installing the pinned tool. The 661-crate
graph's duplicate versions (hashbrown x4, rand x3, ~20 x2) are ecosystem
transitions upstream crates resolve, not this workspace's to force; feature
gating the D-Bus watcher was judged not worth a feature nobody would turn off.

## Not taken, and why

- **3** (empty section fetches fall back and are surfaced) and **4** (engine
  risk instruments: integrity check, upgrade canary, schema stamp) and **7**
  (frontend parity as a check) were not asked for this pass. The two empty
  sections observed on 2026-09-14 (messages 213 and 238 on the scratch store,
  67 and 65 bytes) are still "left on the server" silently.
