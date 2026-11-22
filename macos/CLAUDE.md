# The macOS application

Swift over the same Rust engine, through the UniFFI boundary in `crates/postio-ffi`. Read `docs/decisions/0019-macos-frontend.md` before changing the shape of anything here.

## Build loops (the wrong one wastes minutes)

```bash
scripts/macos-build.sh --lib-only    # Rust changed (crates/): cargo, then regenerate bindings
cd macos && swift build              # Swift changed (macos/Sources): seconds, no cargo
scripts/macos-build.sh               # both, or unsure: the whole chain
scripts/macos-bundle.sh              # assemble Postio.app
open macos/build/Postio.app
scripts/macos-test.sh                # Swift tests with the library linked; runs macos-build.sh --lib-only first
```

`scripts/ffi-bindgen.sh` writes `macos/Sources/PostioFFI/` and `macos/Sources/postio_ffiFFI/` on every build; they are gitignored products, never edited (uniffi checksums are verified at startup, so a skew is a launch-time `fatalError`).

## Getting mail into a fresh build

A fresh install opens the first-run wizard (`FirstRunView`); `Settings > Accounts > +` is the longer sheet. Headless, `postio-provision` makes the wizard's two writes (an account row in the encrypted store, a password in the login Keychain). The `Focus` demo needs neither.

```bash
export POSTIO_ADDRESS='you@your-provider.example'
read -rs POSTIO_APP_PASSWORD && export POSTIO_APP_PASSWORD   # not echoed, not in history
cargo run -p postio-session --bin postio-provision
```

- It prints the servers it resolved before writing anything. The app syncs on launch.
- **Never put the password in a file or on the command line** (`argv` is visible via `ps`). It reads one variable and hands it to the Keychain.
- A domain absent from the provider preset table (the same table onboarding reads) is **refused, not guessed**: `imap.<domain>` often is not your mail server. Give the servers instead:

  ```bash
  export POSTIO_IMAP_HOST='imap.example.com'
  export POSTIO_SMTP_HOST='smtp.example.com'
  export POSTIO_USERNAME='...'      # only if the login is not the address
  ```

  `POSTIO_IMAP_PORT` and `POSTIO_SMTP_PORT` override a preset's ports field by field.
- iCloud custom domain: `imap.mail.me.com`, `smtp.mail.me.com`, `POSTIO_USERNAME` the Apple ID address, and an **app-specific password**; revoke it after testing.
- Re-running is inert: an existing address is left alone and a working password is never overwritten.

**Keychain prompts.** The helper and the app have separate code identities, and an item's ACL binds to its creator, so expect two prompts. An unsigned build's identity changes per rebuild, so "Always Allow" does not stick. Sign with a stable identity (the script lists identities if the named one is missing; ad hoc, the default, suits demos, which read no Keychain):

```bash
POSTIO_CODESIGN_IDENTITY="Apple Development: you@example.com (TEAMID)" scripts/macos-bundle.sh
```

## The Focus app: targets, and who decides

`Package.swift` has three Swift targets over the generated `PostioFFI`:
- **PostioKit**: models, plans, policy, the intent applier. **No AppKit** (iOS reach, #1264; `NoAppKitTests` fails on `import AppKit`).
- **PostioAppKit**: tables, panels, popovers, windows, menu bar, key handling. Depends on PostioKit, never the reverse.
- **Postio**: the executable (app, main window, engine start-up).

Rules:
- **Behaviour lives in the Rust `postio-focus` controller**, not here. Keys, menus and clicks go down through `invoke`, `focusPoint`, `focusPick`; answers return on `nextEvent` as `UiEvent`s, applied by `FocusIntents`, which decides nothing: a field it changes that no event named is the Mac keeping an opinion of its own. Change a rule in `crates/postio-focus` with a controller test first.
- **Words come from `postio-ui`** through the FFI (labels, hints, footers, key spellings, toasts), never Swift literals.
- **Intercepted keys.** `Intercepted` (PostioKit/Accessibility.swift) lists commands this frontend presents a window for instead of dispatching. It mirrors `postio_ffi::registry::INTERCEPTED`; `theInterceptedListsAgreeAcrossTheBoundary` keeps them equal, so change both. Keep it short: a command the controller handles (`/`, `g o`, `?`, pickers) does not belong.
- **No echo of controller-driven surfaces.** Do not report the bar, a picker, the key map or the message window back with `focusSurfaceOpened` / `focusSurfaceClosed` (an echo can close a reopened surface). Swift reports only toolkit-only facts: a click on the dimmed area or outside a popover, a click on a row, the list scrolled to top, reader state. A new surface is a new `postio-ffi` export and `UiEvent` case, not a Swift decision.
- **`SecondaryWindowController`** owns the one window over the list (message, digest, composer, capture), one at a time (M4): opening another closes the first and reports it; the same kind keeps its frame. Ask the controller; never open a second from a view.
- **The message body** is the engine's treated document (`focus_reader_document`) in a hardened WebKit view with no network (`ReaderPolicy`). The contrast guard lives in the shared markup, not a Swift colour.
- **Rust decides, Swift only draws and observes.** Which command a key runs, reader contents, paging, what an absent body means are Rust (ADR 0019: "anything a frontend interprets will drift"). Swift owns views and platform observation (`NWPathMonitor`, `UNUserNotificationCenter`), pushed *down* through a setter. A rule written here is on the wrong side.

## Seeing it: the demo and the shot script

`POSTIO_DEMO=<seed>` runs the app over an in-memory store of invented mail (FFI `demo` feature; `scripts/macos-build.sh` builds it for `macos-shot.sh`). No Keychain, store or network; its `config.toml` is a throwaway under `$TMPDIR` (`postio_ffi::demo::config_file`). Seeds: `small` (inbox), `small:<n>` for screen `n` (`small:22` digest, `small:23` a digest email, `small:25` capture with a throwaway vault).

- `POSTIO_DEMO_KEYS='j j x ⏎'` presses keys after the first page lands (`⏎` Return, `⎋` Escape)
- `POSTIO_DEMO_STATE=offline|auth|first-sync|synced`
- `POSTIO_WINDOW_SIZE=1440x900`, `POSTIO_APPEARANCE=light|dark`
- `POSTIO_DEMO_AGENTS=<dir>/agents/mcp.sock` answers agents there; reach it with `postio-mcp` using `POSTIO_STORE=<dir>/postio.db` and `POSTIO_CONFIG` naming a file whose `[mcp]` shares `test@example.com`. Keep `<dir>` short (socket path length, see gotchas).

`scripts/macos-shot.sh <name> --seed small --both` builds, launches, photographs the window and children with `screencapture -l` and quits; pictures go to `Design/review/focus-macos/` (untracked). The terminal needs Screen Recording once. WebKit does not paint a covered window: never photograph one another app covers.

## Running it

The window draws *before* the store's key is requested, so a Keychain prompt appears over a visible Postio. `POSTIO_LOG` works as on GTK; `[logging]` in `config.toml` retunes a running instance. For a stuck launch, `sample Postio 2`: anything but `NSApplication run` on `com.apple.main-thread` is a bug.

## Things that will bite

- A nested `enum State` shadows SwiftUI's `@State`; the error names neither. Rename it.
- Opening a session reads the login Keychain, and an unsigned build re-prompts after every rebuild. Anything that can work without a session (the command registry) should.
- A bare `swift test` will not link: use `scripts/macos-test.sh`.
- `unset RUSTUP_TOOLCHAIN` and `MAKEFLAGS` before any cargo command: the first beats `rust-toolchain.toml`; the second names the Linux workstation's jobserver fifo.
- `check-no-silent-tracking.py` scans `macos/Sources/**/*.swift` and refuses `URLSession`, `NSWorkspace.shared.open` and the like without a `POSTIO-CONSENT:` comment saying how the user asked for it.
