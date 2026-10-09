# Gotchas

These are the traps in this repository that are still live and that no check, hook or line of CLAUDE.md stops you from walking into. The incidents and measurements behind them are in `docs/archive/engineering-notes.md` and `docs/archive/notes/`.

## Tests and the headless compositor

**Waiting for a total in an integration test measures every other row too.** If any other row moves, the total is wrong while the software is right. Wait for the specific message, row or widget by its identity. Use a count only when the count is the property, and settle it against the store.

**`unwrap_or_default()` on a test's own observation turns breakage into a false accusation.** If the query breaks, it returns the same default as "not ready yet", and the test times out blaming the code it covers. Unwrap in the observation path. A precondition that something else can satisfy is a race in the same way: `queued > 0` counts what *this call* added, not whether there is work.

**Tests that fail under load have known fixes.** Assert order and causality by recording the sequence in the mock, never wall-clock overlap. Liveness deadlines are minutes and go through `postio_test_support::scaled`/`patience`. A fault meaning "the server refuses X" is persistent, not `inject_after` a call count. `tokio::time::pause` misfires when real I/O is in the loop.

**A negative assertion needs a duration and a positive control.** "Nothing was marked read" is just as true when the mechanism never armed. The control waits on its condition. Without one, the test passes with the feature deleted.

**Locals drop in reverse order, so a `TempDir` bound last goes first.** A running engine then recreates the directory, and nothing owns it. Stop the engine in a `Drop` before the directory goes, as `BlobDir` does in `crates/postio-runtime/tests/runtime_suite/harness/mod.rs`.

**Test binaries share `$TMPDIR`.** A path named after something that restarts at 1 in every store, such as a draft id, collides across processes; add the process id and a per-session counter. `.cargo/config.toml` points `TMPDIR` at `target/tmp`, so in a tree without that directory every `tempfile::tempdir()` fails with NotFound.

**A fixture must not do the application's job.** If a test goes red after a fixture loses a shortcut, such as a recount or an invented INBOX, the red is the bug. Do not put the shortcut back. `crates/postio-storage/tests/storage_suite/seed_is_honest.rs` guards the known cases.

**A tested `pub fn`, or an event the runtime emits, is not proof that anything uses it.** For every event, ask who repaints. For every `Job` with a `oneshot` reply, ask who asks. Put the assertion at the far end: "a store this application opened has had X done to it", not "X works".

**`has_focus()` is false on a focused widget in a headless window.** It requires an active toplevel, and a headless window never is one. Use `is_focus()`, `GtkWindowExt::focus`, or `state_flags().contains(gtk::StateFlags::FOCUSED)`.

**To assert on pixels, count real frames, then wait until two renders agree.** `pump()` returns at once when nothing is pending, so it does not wait for anything. Treat stability as the precondition, never as the assertion. Before trusting "no change", push a loud CSS override to prove the harness can see one.

**The test display lays nothing out for WebKit, which still renders the composer.** Assert on the cascade, such as `getComputedStyle(el).color` or DOM attributes. Never assert on geometry, and `getComputedStyle(el).width` counts as geometry. Reader layout belongs to `postio-render` and is asserted headlessly in its own tests (`notes/2026-09-27-the-reader-renders-without-a-display.md`).

**A `GtkWindow` joins the toplevel list as soon as it is constructed.** Dropping the Rust handle does not release it, so a test that builds windows must `destroy()` them. A teardown segfault is usually a race you cannot reproduce, so test the leak instead: hold a `WeakRef`, drop the window, turn the loop, and require `upgrade()` to be `None`.

**One clean run of a GUI suite on a loaded machine proves nothing.** Bisect by running each side two or three times. If a test is red for one person on every commit and a bisect finds nothing, the cause is machine state (a state file, the keyring), not the tree.

## The store and the search index

The store is Turso (ADR 0038). Its planner and defaults are not SQLite's, and most of what follows fails silently.

**`fts_score` returns `0.0` unless it is projected bare and shares its parameter with the `fts_match`.** `-fts_score(..)` gives `0.0`. So does `?1` in the score beside `?2` in the match, even with the same value. Score in a subquery, do the arithmetic outside it, and reuse one explicit `?N`. A bare `?` written before an explicit `?N` collides with it. The rows and the count stay right and only the order changes, so a ranking test needs a case where an older, better match must beat a newer one. Pinned in `crates/postio-storage/tests/turso_capabilities.rs`.

**A row-value cursor filters instead of seeking.** `(received_at, id) < (?2, ?3)` walks everything above the cursor. Keep the redundant bare `received_at <= ?2` in front of it. Removing it as a tidy-up turns keyset paging back into a skip.

**The planner will not read through a partial index, but it still enforces a partial `UNIQUE`.** A `WHERE` on an index in `schema.rs` is a constraint, never a size optimisation. If a constrained index is also on a read path, add an unpredicated companion index.

**The planner will not bind an equality through a `COLLATE NOCASE` index column.** It seeks the prefix and walks the rest. Fold the value into the key instead, as `RfcMessageId::folded` does. `idx_labels_account_name` still has this shape. `IN` on a second index column also binds only the prefix, so use a `UNION ALL` of point lookups.

**A `SEARCH` that binds one column of a two-column key is a scan, and a gate that greps for `SCAN` misses it.** Plan gates must name the columns they expect bound. An aggregate is one statement and one row however much it read, so statement and row counts cannot see it: keep `count(*)` off the first-frame path, and read a panel's figures when the panel is mapped. Do not time a statement by wrapping it in `SELECT count(*) FROM (..)`, because the engine prunes columns nothing reads.

**Without statistics the planner takes the seek that binds more columns, even inside a join.** A per-row lookup on a one-column key, such as `idx_messages_content (content_id=?)`, loses to any two-column range the same `WHERE` offers, such as `idx_messages_account_list (account_id=? AND received_at<=?)`, and the range is then walked once per outer row. Only a rowid seek is never outbid. When a join has to look rows up through a secondary index, name it with `INDEXED BY`, as `HITS_JOIN` in `crates/postio-index/src/executor.rs` does (#1809).

**A bench only sees a cost shaped like `rows × other_table` if both tables are seeded.** Populate both at real-mailbox scale.

**Never run `ANALYZE`.** With statistics, the planner turns hot sync statements into scans. Fix a plan with an index the planner cannot misjudge. `crates/postio-sync/tests/sync_suite/scan_audit.rs` asserts that the store has no `sqlite_stat1`.

**The engine keeps no connection pool, so every new connection starts with a cold page cache.** Use `Store::read` for reads that come in numbers, `Store::connect` for a writer, and `Store::connect_background` for background work. Never open a raw connection: Turso's defaults (`busy_timeout` 0, `foreign_keys` off, a 2 MB cache) are wrong here, and `store::PER_CONNECTION` sets them.

**`WriteGate`: take the connection first, then the permit, and hold one permit at a time.** The gate is not re-entrant. A writer without a permit is invisible to the gate and starves during a sync. `let permit = gate.acquire(p);` without `.await` compiles and does nothing; clippy cannot see that shape, so only a test can.

**The outermost write transaction must be `BEGIN IMMEDIATE`.** A deferred transaction that reads first fails to promote. It fails with `Busy` within milliseconds, without waiting out the busy timeout. Go through the storage crate's scope (`crates/postio-storage/src/sql.rs`), not a hand-opened transaction.

**Triggers in `schema.rs` maintain the mailbox counts.** Do not add a recount to a new write path. A `total_count` that is too low renders an empty mailbox, not a wrong number.

**A list-row column belongs only in `LIST_COLUMNS`.** A second spelling of that list once broke paging two crates away while every storage test passed.

**Postio folds accents itself, on both sides.** The tokenizer folds case only. Indexed text and query text both go through `postio_model::fold`, and any new path that skips it cannot find `José` by typing `jose`.

**Exclude negated free text in the `WHERE`, outside the match.** Inside the metadata match, `NOT "spam"` admits a message whose "spam" is in its body. Adding a column to the broad path's candidate statement can lose its plan.

**Index a body through `postio_index::index::index_body_of`, which uses `Document::to_search_text`.** `to_text` keeps link addresses and `[image]` placeholders, so a message would match words it never showed.

**`Engine::request_body` queues the fetch rather than doing it.** The bytes land when the engine's loop gets to the job. Wait with `postio_host::parts::wait_for_body`, and treat a busy read as "look again".

**A body fetch replaces the message's attachment rows.** An `AttachmentId` does not survive the fetch it triggers, so resolve it to the MIME part path first. Blob ids hash the decoded payload, not the base64.

**The schema stamp hashes `HEAD`'s text, SQL comments and spacing included.** A search-and-replace across the tree that touches a comment inside `HEAD` moves the stamp exactly as a new column would, and `a_schema_change_comes_with_the_migration_that_reaches_it` fails. Leave `HEAD`'s comments out of mechanical rewrites, or put them back.

**A schema change edits `schema::HEAD` and adds a `schema::MIGRATIONS` step.** Keep the replaced `HEAD` in `crates/postio-storage/tests/schemas/`. A store that no migration reaches is refused with `Remedy::StartOver` (`notes/2026-10-01-store-migrations-and-starting-over.md`). The engine has no read-only open, so point any diagnostic at a copy of the store.

**Only a keyring answer of `NotFound` mints a store key.** `Locked`, `Timeout` and `Backend` mean the keyring did not answer, and treating them as a first run destroys the store. Never `#[derive(Debug)]` on anything that holds key material.

**Grace periods on reclamation are load-bearing.** A blob is written before the row that names it commits. The file-transfer portal hands the receiver paths, not bytes, so an export reclaimed too early becomes a drop that delivers nothing, with no error (`DRAG_EXPORT_GRACE_PERIOD`). To test either, back-date mtimes. Never shorten the period.

## Sync, IMAP and accounts

**A pending local change outranks the server until it settles.** Resync hides messages that have an undrained move or delete, and replays queued flag changes over the server's flags. Only `pending` and `in_flight` operations count. A new operation type must decide what an unacknowledged one does to a resync. To test it, have a second client change a *different* flag; otherwise the pass fetches nothing and the test cannot fail.

**A bulk verb whose effect depends on row state enqueues only the rows that disagree.** The queued rows are the undo set, so a row that already agreed would let `u` undo something the action never did.

**io-imap keeps only the last untagged `* SEARCH` line.** `existing_uids` guards against this by checking the listing against `EXISTS`. `SORT` and `THREAD` have the same bug, so check them before calling either. When `crates/postio-account/tests/io_imap_search_defect.rs` fails, upstream has fixed it; delete the workarounds then.

**Decide QRESYNC from the `CAPABILITY` list after login, never from the `* ENABLED` echo.** Take expunges from `SELECT`'s `vanished_earlier`, not from `FETCH (VANISHED)`. An empty capability list after login is an error. Do not use `watch::ImapMailboxWatch`.

**Install tracing with `set_global_default`, never `.init()`.** `.init()` takes the process's only `log` slot and leaves io-imap's skip counter inert, so a fetch that dropped responses reports success (`crates/postio-session/src/logging.rs`).

**Build one `TokenSource` per account, at the composition root.** A second source of the same type compiles, then splits rejections and refresh-token rotation between IMAP and SMTP. Only `postio_account::auth::with_credential` decides what a refused credential means.

**An OAuth account has two expiry timestamps.** `#oauth-expiry` is the hourly access token. Only `#oauth-refresh-deadline`, the grant, means "sign in again". Record the deadline again after every successful refresh.

**Write the credential before the account row.** An account counts only when `postio_host::startup::route` finds a row whose secret the keyring will give up.

## GTK

**`connect_*` on a process-wide object outlives your widget.** For `adw::StyleManager::default()`, the display, settings or the application, the closure and its strong references live as long as the process. Disconnect when the last clone goes.

**`set_content` on an `adw::Window` gives you no titlebar.** The content has to provide it, with a `ToolbarView` and a `HeaderBar`. A widget test cannot see the missing titlebar, so render with `shot` or a storyboard.

**To move a surface, reparent it and then restore its focus.** Unparent from the actual parent. Inside `close-request`, call `destroy()`, not `close()`, which emits the signal again.

**Whether a surface is open and whether it has the keyboard are separate questions.** A surface left open while focus is elsewhere pins the resolver's context and silently drops bare keys.

**Cursor, selection and activation are three facts.** State which one a surface follows. A verb with an empty selection acts on the cursor row. `SingleSelection` autoselects row 0, which is not the user landing there. Push state into `AppState` through `postio_core::aim::mirror`, not with a signal.

**Changes the app makes for the user stay off the undo stack, with no toast** (`Recording::Incidental`).

**`GtkListView` builds about 205 rows ahead.** A test model near 200 cannot tell recycling from no recycling, so use 1,000 rows or more.

**An app-priority CSS provider resolves `prefers-color-scheme` against its own setting.** `postio_gtk::style::install` keeps that setting in step with `AdwStyleManager`.

## macOS and the FFI

**A change under `crates/postio-ffi/src/` has Swift callers that Linux cannot compile.** Before landing, grep `macos/` for every boundary type and field the diff changed (`notes/2026-09-12-the-ffi-has-a-second-caller-you-cannot-compile.md`).

**Pass a platform difference as a `Platform` parameter, not a `#[cfg]`.** Assert both values from either host. That includes compositions across crates: what `expand_mod` writes must parse in `Modifiers::parse`. A `cfg!` in an assertion only ever checks the host.

**`Engine.swift`, `Shell.swift` and `FolderRow.swift` live in the executable target, which has no tests.** Put the rule in `PostioKit` as a small type and leave one call behind. Then launch the bundle; `sample` names a blocked main thread.

**`KeyMonitor` sees keys before the responder chain.** A `.onKeyPress` on a key the resolver claims never runs. Swallow only keys that something acted on.

**Do not read `\.openSettings` from a view inside the `WindowGroup`.** The main window never finishes its first layout, and the app runs, logs and draws nothing. The shell opens Settings with `openWindow(id:)`.

**`ScenePhase.background` means closing or hiding the window, not quitting.** Shutdown belongs in `applicationWillTerminate`.

**`LazyVStack` hides and pools platform views instead of dismantling them.** A `WKWebView` inside one is freed only when the stack's identity changes. Capture platform views weakly in work that can outlive them.

**A `@MainActor` test must suspend with `Task.sleep`, not spin `RunLoop.main`.** Spinning starves main-actor work and looks like a leak.

**SwiftUI's `.tag()` compiles against any type, and a `WKNavigationDelegate` method that "nearly matches" is never called.** When a selection type changes, grep every `.tag(`. Use the delegate's `async` form, and treat Swift 6's warning about a near match as an error.

**On macOS, a test that reaches the platform keyring raises a prompt and hangs.** Hand it a `MemorySecretStore` and check that the code under test uses it. If every test reports `ok` but no summary prints, a test thread is blocked: run `sample <pid>`.

**`ReaderEgressTests.swift` opens a loopback listener on purpose.** It is the only test that fails when the reader starts fetching from the network. Do not delete it for touching a socket, and do not create an `NSWindow` in a test.

**Moving code out of a crate this host cannot compile leaves stale imports behind.** For each moved symbol, split its remaining uses at the last `mod tests`. Rustdoc cannot link upward from a dependency to its dependent, so name such an item in prose.

## Builds, caches and worktrees

**`RUSTUP_TOOLCHAIN` overrides `rust-toolchain.toml`, and mise exports it.** The land and test scripts unset it; your shell does not. When the pin moves, move the mise pin too and sweep stale `target/` directories. `fuzz/` pins its own dated nightly; run it through `scripts/fuzz.sh`, which changes into that directory.

**`refs/stash` is shared by the whole repository, not kept per worktree.** A stash inside your worktree goes onto every session's stack. Commit work in progress instead.

**`git commit --only <path>` skips untracked files under that path and still exits 0.** `git add` new files by name first.

**Registries that only grow at the end conflict on every rebase.** These are `CommandId`, the command registry, `CASES` and the generated `docs/keybindings.md`. Resolve one item at a time, because concatenating hunks fuses struct literals and functions. Run `cargo check` before `rebase --continue`.

**sccache reads its settings when its server starts.** A bare `sccache --show-stats` after `--stop-server` starts a server at the 10 GiB default, which evicts the cache; restart with `scripts/sccache-restart.sh`. `SCCACHE_ERROR_LOG` records nothing unless `SCCACHE_LOG` is also set. A wedged daemon shows idle, minutes-old `rustc` processes and an executed count that does not move. Stopping it kills other sessions' compiles.

**A change to a profile or linker flag evicts the shared compile cache for everyone.** Make such changes rarely and in one commit. For line tables in a debugger, set `CARGO_PROFILE_DEV_DEBUG` for that one build.

**Only `readelf -p .comment` tells you which linker ran.** gcc obeys the last `-fuse-ld`, and rustc appends its own. `scripts/linker.sh` puts mold last.

**`.cargo/config.toml` is copied into the Flatpak sandbox.** If you add a tool by bare name, a path or a machine-specific setting there, neutralise it in `flatpak/dev.postio.Postio.json`'s `build-options.env`. `gh workflow run Release --ref main` builds the bundle without a tag.

**A CI job that compiles uses `.github/actions/build-cache` and `build-cache-save`, never `actions/cache` on `target/`.** If a codegen setting lives outside the cache key's inputs, bump the action's `epoch` when it changes.

**GitHub path filters are not shell globs.** `*` stops at `/`, so `'*.md'` matches only top-level prose.

**`issue-land.sh` rebases the tree it is running from, and bash reads a script by byte offset as it goes.** When editing it, keep the re-run logic inside the `if [ "$BEHIND" -gt 0 ]` block and before any push.

**A coredump names a worktree, and that worktree's code may never have landed.** Check `git log` before treating it as evidence about `main`. Run any load generator under `timeout`.

**`check-crate-boundaries.py` checks only the crates in its `RULES` table.** A new crate has no boundary rules until someone writes one.

## Logging, privacy and hostile input

**A `POSTIO_LOG` made only of per-target directives turns every other target off.** Start with a bare level, as in `POSTIO_LOG=info,postio_sync=debug`. If a run does not begin with `postio starting`, the filter is hiding everything.

**Redact an error that may name an account at the log call site, with `postio_model::address::redact_addresses`.** The screen keeps the address and the log does not. Never log search query text.

**Secrets escape through the buffers around `Zeroizing`.** On failure, `String::from_utf8` drops the bytes unzeroed. `SecretString::from(String)` reallocates when the string has spare capacity and frees the old buffer without zeroing it. Hand copies over with a single `str::to_owned`.

**`is_secret_key` matches substrings.** Run it only on unknown keys gathered by a `#[serde(flatten)]` catch-all, never on a whole document whose schema has fields like `password_help_url` or `token`.

**Do not paste fuzz crash inputs or io-imap traces into issues.** Both carry data shaped like real mail. Describe the input, and add a fixture through `/add-fixture`.

**A fuzz panic inside a dependency may be a `debug_assert!` that release builds never hit.** Open the panic site before judging severity. `fuzz_target!` aborts even on a panic you caught; `postio_fuzz::allow_contained_panics` exists for this. A fuzz property must be a promise the function under test actually makes.

**On the ingest path, use a worklist instead of recursion.** Input-controlled depth is input-controlled stack, and a stack overflow cannot be caught.

**`address::parse_list` parses what a person is typing, so it stays lenient.** A decoded header value can legitimately contain CR and LF, so anything that writes one back into a header must fold it. Change `docs/rfc-compliance.md` and `crates/postio-model/tests/rfc5322.rs` together.
