# Performance: budgets and how they are gated

Performance is a functional requirement (constitution V). A shared CI runner
cannot defend 16 ms, so the gates count the causes of a budget (statements,
rows, planner scans, trigger firings) through
`postio_storage::test_support::counting`. Those numbers are the same on every
machine. Wall-clock figures report; they never gate.

## Budgets

| Budget | Target | How it is gated |
|---|---|---|
| Startup to usable UI | < 500 ms | Counted reads at startup (`focus_suite`'s `startup_reads`); phase timeline from `POSTIO_STARTUP_TRACE=1`; `store_opening` shows what a slow start waits on |
| Ordinary interaction | < 16 ms | `store_reads` and `list_scroll` benches; per-read-path count assertions |
| Local search | < 100 ms | `search_budget` bench; `counting::scans` fails on an unindexed query |
| Memory | no full-mailbox load | Lists are windowed over the paged store; "select all" is a predicate |
| Transitions | <= 100 ms or none | `focus_suite`'s `motion` case; stylesheets carry no transition, and reduced motion stills them |

Counts miss an aggregate that reads many rows as one statement and one row,
so `counting::scans` asks the planner whether a query can be cheap. A count
assertion goes in with any change to a read path (CLAUDE.md, What the code
must do).

## Tiers

| Tier | What runs | Gates a merge |
|---|---|---|
| Merge path | Count assertions in unit and suite tests | yes |
| Nightly | Wall-clock cases, marked `//! POSTIO-MEASUREMENT:` and selected by `.config/nextest.toml`; `bench.yml` compiles the bench targets and times nothing | no |
| Manual | The recipes below, on a real mailbox | no |

A slow budget case goes to the nightly tier. No merge-path test waits on the
clock (`check-no-clock-waits.py`).

## Measuring by hand

Wall-clock numbers are a regression guard on one machine, not a promise
about anyone's hardware. Take the floor of repeated runs, not the mean. The
store is encrypted (ADR 0014), so every figure includes page decryption.

```sh
cargo run --release -p postio-runtime --example seed_store -- /tmp/postio.db 20000
POSTIO_STORE=/tmp/postio.db POSTIO_STARTUP_TRACE=1 POSTIO_STARTUP_EXIT=1 \
  cargo run --release -p postio-gtk

cargo bench -p postio-bench --bench store_reads    # the page read
cargo bench -p postio-bench --bench search_budget  # the query
cargo bench -p postio-bench --bench list_scroll    # the row draw
cargo bench -p postio-bench --bench event_hub      # the event emit path
cargo run -p postio-gtk --example first_realize -- splash real
```

`seed_store` needs this installation's store key from the OS keyring: run
`postio` once first. The tool never mints a key.

Memory: seed a store, start `postio`, wait 45 s for the catch-up passes, then
read `VmRSS`, `RssAnon` and `RssFile` from `/proc/<pid>/status`. Use
`Pss` from `smaps_rollup` for multi-process totals. The anonymous figure is
the claim: it stays flat as the mailbox grows.

## Startup phases

`POSTIO_STARTUP_TRACE=1` prints one line with these phases. The budget runs
to `first frame`, the paint that has mail in it, not to `shell`.

| Phase | Holds |
|---|---|
| `init`, `fonts`, `styles` | toolkit start-up |
| `window` | the widget tree, built before anything is on screen |
| `shell` | the compositor showing the window; no mail yet |
| `store` | keyring, database, migrations, index, on a thread |
| `account` | the route decided and the account found |
| `feeds` | main-thread work the first frame waits on |
| `first frame` | GTK's paint of the frame the mail is in |

Rules that follow from the measurements:

- Startup does no unindexed aggregate (a `count(*)` over every message cost
  70,000 steps and a second on a real store). Read it when the pane that
  shows it opens (#1479).
- The window is presented before the store opens, so a slow migration or
  keyring prompt never leaves nothing on screen (#1114).
- Nothing is drawn while the store opens for the first second: no spinner or
  skeleton. Past a second the list pane names which of four waits it is on
  (`postio_gtk::list_state::Waiting`). A window with no store offers no verb
  that cannot run (`Requirement::StoreOpen`).
- There is no splash screen (#1109). It pays the same first-realize cost it
  would hide, flashes inside the 100 ms transition limit, duplicates the
  desktop's `StartupNotify` feedback, and would be the app's first animation.
- GTK's first realize costs about 74 ms whatever the window holds
  (the shader compile). The GPU renderer is kept (#790).

## Read paths

- A page read is flat against mailbox size: page one of 100,000 messages costs
  what page one of 1,000 does.
- The four list indexes end in `deleted_locally, snoozed_until`, the two
  columns every list query filters on, so an `OFFSET` jump does not fetch
  every discarded row. `store_reads` asserts it (#638).
- A keyset cursor spelled as a row value is a filter, not a seek, on this
  engine. Use the expanded form
  (`docs/notes/2026-09-12-a-row-value-cursor-is-a-filter-not-a-seek.md`).
- The header index is budgeted per message (ADR 0027).
- Search's worst shape is a word in every message with facet counts. Watch it
  when an index changes.
- Memory-mapping is off: pages are encrypted, so each is decrypted into the
  page cache (ADR 0014). Message text is zstd per row
  (`postio_storage::body_codec`).

## Open items

- Wall-clock figures predate the engine swap to Turso (ADR 0038) and the
  removal of the classic app; re-take them on a real mailbox with the recipes
  above.
- The unified (multi-account) page read measured 18.7 ms against 16 ms
  (#619).
