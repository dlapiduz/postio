# FR-005 audit: what this branch did to the desktop's and macOS's tests

Final pass after Phase 11 (one app at a time); earlier passes predate it.

FR-005 says no desktop or macOS function may be removed or degraded, and the
tests that prove them stay unchanged except for import paths. This is the
record T100 asks for: every test file under `postio-app`, `postio-gtk`,
`postio-ffi` and `macos/` that differs from the merge base, and why.

Rerun it before landing. The same commands give the same answer, with the
branch head in place of the one named here:

```bash
base=$(git merge-base HEAD origin/main)
git diff --stat "$base" HEAD -- crates/postio-app/tests crates/postio-gtk/tests crates/postio-ffi/tests macos/
git diff "$base" HEAD -- crates/postio-gtk/src crates/postio-app/src crates/postio-ffi/src \
  | grep -E '^-\s+fn [a-z_0-9]+\(' | sed -E 's/^-\s+fn ([a-z_0-9]+).*/\1/' | sort -u
```

For each function name the second command prints, compare its body at the
merge base with its body at `HEAD`, wherever it now lives.

## Integration suites: additions only

`macos/` has no changes at all. The integration suites gained four modules
and changed none:
- `postio-app/tests/app_suite/store_in_use_window.rs`: the desktop says the
  store is open elsewhere, and opens it once it is free (Phase 11);
- `postio-ffi/tests/ffi_suite/host.rs`: a command through the macOS
  boundary reaches the store, which the no-op bus it replaced never did
  (T019);
- `gtk_banner_keys.rs`: the reader's banner keys;
- `gtk_composer_markdown.rs`: a draft's Markdown survives the desktop
  composer.

The only other changes are their rows in each suite's `main.rs`.

## Unit tests that moved with their code

Three desktop modules moved with their tests: two to the shared,
toolkit-free layer (FR-004), and the notification decision to the host,
where both frontends ask it. Every body is byte-for-byte identical but the
one noted below the table.

| Tests | Were in | Now in |
|---|---|---|
| 15, the status line: sync, backfill, connection state, age | `postio-gtk/src/feed.rs` | `postio-ui/src/status.rs` |
| 10, the remote-image allow list | `postio-gtk/src/reader/allowlist.rs` | `postio-ui/src/allowlist.rs` |
| 3, deciding a new-mail notification | `postio-app/src/notifications.rs` | `postio-host/src/notify.rs` |

The desktop's folder tree (`folder_rows`, `ancestors_of`, `FolderRow`,
`MAX_DEPTH`) moved to `postio-ui/src/sidebar.rs` without its tests:
`postio-gtk::sidebar` re-exports the names, so its tree tests stay where they
were and did not change at all.

Two of the three notification tests are identical; the third,
`mail_landing_in_the_open_mailbox_of_the_active_window_is_not_posted`,
differs by one import path (`notify::Suppressed` is imported as
`Suppressed`), which FR-005 allows.

## One test changed in place, and why

`postio-gtk/src/composer.rs`,
`a_command_with_no_key_left_shows_no_hint_rather_than_a_blank_one`, from
T017 (`c2f9126b`, "give every command a key a terminal sends").

The test proves a rule: a composer command whose key was taken by a `[keys]`
override shows no hint, rather than an empty one.
- **Before:** it built that case by giving `save_draft` the key `send` had.
  Send had no other key, so Send was left without one.
- **Why that stopped working:** T017 gave Send a second default,
  `alt+Return`, because a legacy terminal cannot deliver `Ctrl+Return`. So
  the same override no longer leaves Send keyless, and there was no longer a
  keyless command to check.
- **After:** the test builds the same case the other way round. It gives
  `send` the key `save_draft` has. Save draft has no alternate, so it is
  left keyless, and the test checks that it shows no hint.

The rule and the assertion's strength are unchanged. For the desktop the
change is an addition: `Alt+Return` now also sends from the composer, and
nothing it could do before is gone.
