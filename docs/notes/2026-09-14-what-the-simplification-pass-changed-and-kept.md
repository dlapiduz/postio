# What the simplification pass changed, and what it kept

2026-09-14, on `feature/turso-store`. The maintainer asked three things
after the Turso rebuild landed on its branch: what in the codebase could
be simplified to make it more predictable, whether the three GTK
decoupling moves from the frontend audit could be done, and what in the
engineering practices and dependencies was worth improving. This is the
answer as landed, and — as important — what was looked at and left
alone, with the reason, so the next pass does not re-open it.

## Simplified, and landed

- **One paging policy behind both message lists.** `postio_ui::paging::
  Paging` is what a page number means (an offset read of the scope, or a
  slice of the search ranking), what each scope does with each runtime
  event, and how often a failed page is re-asked. GTK's `Feed` and the
  FFI's `fetch`/`react` are adapters over it. The macOS side used to
  flatten every event into "count again, reset if the count moved", so a
  flag change never reached the row on screen; it refetches in place now.
- **One notification decision.** `postio_ui::notify::decide` is the
  suppression, the coalescing id and the click target for both frontends.
  GTK gained the already-on-screen suppression it lacked. The *wording*
  is a documented per-platform choice (`Wording::Counts` for a lock
  screen, `Wording::Newest` for a desktop banner), not two drifting copies.
- **One pane cycle, one way back.** `postio_ui::focus::next_pane` is Tab's
  table on both frontends; `Returns` replaces the four `before_*` cells the
  GTK window kept for the folders, the parts panel and the settings lists.
- **`MailStore` is the five methods a frontend calls.** The two windows
  underneath `list_page` (`message_page`, `thread_page`, and their counts)
  are `LocalStore`'s own methods for tests and benches; every fake used to
  implement them for nothing.
- **An engine starts from the `Wiring`.** `engine::start`, `start_all` and
  `start_joining` took the same eight installation-wide choices as
  arguments under three `too_many_arguments` allows; they take `&Wiring`.
- **One crossing per reading-pane fill.** `Fill::read_then`, with
  `BodyFetcher` and `Painter` for what a closure needs after it has
  outlived the `Fill`; five hand-spelled copies of the same shape are gone.
- **`SqliteStore` is `LocalStore`.** The type is the local store whatever
  engine holds the file, and the name said rusqlite.
- **The body search index has its own table** (`message_search_bodies`),
  so the fts index no longer rides on `messages`: header inserts at 1,200
  indexed bodies went from 14.56 ms mean / 528 ms worst to 1.34 ms / 31.6
  ms — the no-index baseline — and search is unchanged, gated by its
  statement and row budgets rather than timed.
- **Docs that named rusqlite, SQLCipher or `spawn_blocking` as current**
  were corrected where they were found (runtime, session, app, ffi, gtk,
  index, storage), with the history kept as history.
- **The lint floor is closed.** `postio-storage` is back on the workspace
  lints, the four crates that cannot inherit them deny
  `let_underscore_future` explicitly, and `check-lint-floor.py` verifies
  the clippy lines rather than only the `[lints]` table.

## Looked at, and kept — with the reason

- `MailboxRepository::{recount, recount_account, set_counts}` have no
  production caller. They are documented as the trigger-independent ground
  truth the count tests check against, and as the repair tool for a store
  the triggers were not there for. Moving them behind the test-support
  feature would gate a repair tool behind a test flag. Kept.
- `EngineThread` / `RETAINED` in `postio-runtime`: kept while the engine
  is pre-1.0 and its WAL recovery on an abrupt drop is not something to
  lean on. The doc says so now, instead of citing rusqlite.
- `zbus` (~35 crates) for the one NetworkManager watcher: online/offline
  detection has no cheaper honest source. Kept.
- `postio-body/src/styles.rs` over `cssparser`: complements `ammonia`,
  which filters attributes and does not parse CSS. Kept.
- `scripts/cc-wrapper.sh`: in use — `install-shims.sh` installs it as
  `postio-cc`, the bare-name shim the shared compile cache depends on.
- `blake3` in storage and session: blob ids and the store-key derivation.
  Not measured; not worth measuring against a ~470-crate workspace.
- The GTK source traits (`MessageSource`, `ResultSource`, `MailboxSource`)
  stay in `postio-gtk`. They are the *crossing* — futures pollable on the
  GTK main context — and the macOS side crosses on tokio with no trait at
  all; a trait generic over both would describe nothing either needs.

## Practices, as observed on this branch

- The default landing tier plus CI's full `Tests` job was enough: the one
  failure CI found that the branch's own runs did not was a measurement
  flake (`a_whole_thread_costs_one_web_process` compared process-list
  lengths; it compares pids as sets now).
- The macOS tooling self-test hit `ffi-bindgen.sh`'s 900 s cap once, on
  the push that changed `postio-storage`'s schema — a cold runner rebuilding
  most of the workspace, not a wrong answer. If it repeats on a warm push,
  the cap is the thing to change, not the script.
- Still open, and known: `[sync] notify_roles` does not cross the FFI, so
  macOS notifies for every folder; spec task T046 (a real-account run on a
  fresh store) is the maintainer's to do.
