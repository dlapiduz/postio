# Quickstart: building and verifying Focus on the Mac

## Prerequisites

- macOS 14 or later, with Xcode command-line tools and Swift 6.
- `unset RUSTUP_TOOLCHAIN` before cargo. `mise` exports it, and it would
  override `rust-toolchain.toml`.
- Work in `~/src/postio-worktrees/focus-macos`, on `feature/focus-macos`.
- Once per machine:
  - Screen Recording and Accessibility for the terminal, for screenshots.
  - Approval of the Keychain prompt for each ad-hoc signed build.

## The shared controller (any host)

```bash
cargo test -p postio-focus                     # every controller rule, milliseconds
cargo nextest run -p postio-ffi --test ffi_suite
python3 scripts/checks/check-crate-boundaries.py
```

`postio-gtk` does not build on the Mac. A slice's GTK half is proven by CI
(`cargo nextest run -p postio-gtk`, focus_suite) on its pull request to
`main`. Before pushing, grep for orphaned imports
(`docs/notes/2026-09-06-moving-code-out-of-a-crate-you-cannot-compile.md`).

## The Mac app

```bash
scripts/macos-build.sh            # cargo build, bindgen, tokens, swift build
scripts/macos-test.sh             # Swift Testing: keymap layer, view models
scripts/macos-bundle.sh           # macos/build/Postio.app
scripts/macos-shot.sh 01          # NEW: demo store, 1440×900, light and dark → Design/review/focus-macos/
```

`macos-shot.sh <screen>` launches the bundle with `POSTIO_STORE` and
`POSTIO_CONFIG` over a scratch copy of the seeded demo store. It drives the
app to that screen's state through the FFI's scripted inputs, and writes
`<screen>-light.png` and `<screen>-dark.png`.

## What "done" means for a screen (FR-061)

1. Capture it in light and dark at 1440×900.
2. Put each capture beside `Design/focus-macos-design/screens/<nn>-*.png`.
3. Write every difference into the phase's comparison note
   (`docs/notes/2026-10-xx-focus-macos-phase-<n>.md`). Each one is fixed, or
   explained by a decision (C*, M*).
4. For the email window (04, 22, 23), also compare it with every PNG in
   `message-window/screens/`, and capture it with a 1024-wide main window:
   the window is 656 and the plain column is 560.

## Scenarios that prove each phase

| Phase | Scenario | Expected |
|---|---|---|
| 0 | Open the app over the demo store with `[focus]` on | Markers and digest rows appear; `cargo test -p postio-ffi` shows `Frontend::Focus` owed and nothing three-pane |
| 1 | Launch | 01 and 02 match; scrolling a 10k-conversation store drops no frames |
| 2 | Select 3 rows with `x`, press `a`, wait 10 s, choose Edit › Undo | The menu reads "Undo Archive 3 messages"; the rows return; the cursor is on the restored row |
| 3 | Open a newsletter in dark appearance; press ⇧O; press `j` | It opens on paper, dimmed; switches to app colours; the next message opens at the same window size |
| 3 | Press `t` in the email window with a vault configured | Capture opens (#1754) |
| 4 | Reply to a question row, press Esc, reopen the draft, press ⌘↩ | The draft is kept; it is sent through the outbox; the marker clears |
| 5 | Type "from ada last week", then Tab | Chips `from:ada` `after:…`; Tab enters them; ⌘⌫ returns to the words |
| 6 | Press `s`, Tab, type "tue 9am", press ↩ | Snoozed to the same instant Linux computes |
| 7 | Turn Wi-Fi off; archive a row | Offline strip; the row leaves at once; the change is sent on reconnect |
| 8 | Edit `[keys]` in `~/Library/Application Support/Postio/config.toml` | The menu, keycaps and `?` show the new key without a restart |
| 9 | Capture a task, then open its `postio://` link from another app | Postio comes forward on that message |

## Landing

- **Controller slices** go to `main` from their own branches with
  `scripts/issue-land.sh --detach`. They are spec work, so commits end
  `Refs: specs/009-focus-macos` and the task id.
- **The Mac branch** lands once with `scripts/issue-land.sh --detach
  --full-suite`, after phase 9. M3 lands as a follow-up on the same branch
  or a new one, as `tasks.md` orders it.
- **Interactions.** A Mac landing that changes one carries
  `interactions-unreviewed` (FR-063).
