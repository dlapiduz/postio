# The macOS application

Swift over the same Rust engine, through the UniFFI boundary in
`crates/postio-ffi`. Read `docs/decisions/0019-macos-frontend.md` before
changing the shape of anything here.

## Two build loops, and using the wrong one wastes minutes

**Rust changed** — anything under `crates/`:

```bash
scripts/macos-build.sh --lib-only    # cargo, then regenerate the bindings
```

**Swift changed** — anything under `macos/Sources`:

```bash
cd macos && swift build              # seconds, no cargo at all
```

**Both, or you are not sure:**

```bash
scripts/macos-build.sh               # the whole chain
scripts/macos-bundle.sh              # assemble Postio.app
open macos/build/Postio.app
```

`scripts/macos-test.sh` runs the Swift tests with the library linked.

## Getting mail into a fresh build

A fresh install opens the first-run wizard (`FirstRunView`, address and
password, then `Connect`); `Settings > Accounts > +` is the longer sheet
(browser sign-in, a folder of mail on this Mac). For a headless setup, or an
agent with no one to click, `postio-provision` makes the same two writes the
wizard makes: an account row in the encrypted store, and a password in the
login Keychain. The `Focus` demo (below) needs neither.

```bash
export POSTIO_ADDRESS='you@your-provider.example'
read -rs POSTIO_APP_PASSWORD && export POSTIO_APP_PASSWORD   # not echoed, not in history
cargo run -p postio-session --bin postio-provision
```

It prints the servers it resolved before it writes anything, so a mistyped
host is visible then rather than as a failed sync later. Then open the app and
it syncs on launch.

**Do not put the password in a file, and do not pass it on the command line.**
`argv` is readable by every process on the machine through `ps`. The helper
reads exactly one variable and hands it to the Keychain; it is never printed,
never logged, and there is no field on the account row that could hold it.

**Not in a GTK crate.** The helper lives in `postio-session` because
`postio-gtk` and `postio-widgets` link GTK, which a Mac cannot compile
(ADR 0019).

### What to expect from the Keychain

Two prompts, not one, and possibly more on later runs. The helper and the app
are separate binaries with separate code identities, and a Keychain item's ACL
is bound to whatever created it — so granting access to the helper says nothing
about the app. An unsigned build's identity also changes on every rebuild, for
the reason under *Things that will bite* below, which is why "Always Allow"
stops sticking as soon as you rebuild.

**Sign with a stable identity and "Always Allow" survives rebuilds:**

```bash
POSTIO_CODESIGN_IDENTITY="Apple Development: you@example.com (TEAMID)" \
    scripts/macos-bundle.sh
```

The script lists the identities it can see if the one named is missing. Ad hoc
signing (the default) is right for demos, which read no Keychain.

### When it will not resolve the servers

The provider preset table is consulted by domain, and it is the same table the
onboarding screen reads rather than a second copy — a provider added for the
screen is available here on the same commit (#69 is what two copies cost). A
domain the table does not publish settings for is **refused rather than
guessed**: `imap.<your-domain>` resolves for a great many hosts that are not
your mail server, and pointing an account at one of those means typing a
password into somebody else's machine. Give the servers instead:

```bash
export POSTIO_IMAP_HOST='imap.example.com'
export POSTIO_SMTP_HOST='smtp.example.com'
export POSTIO_USERNAME='...'      # only if the login is not the address
```

`POSTIO_IMAP_PORT` and `POSTIO_SMTP_PORT` override a preset's ports the same
way, field by field — overriding one setting keeps the rest of the row.

An iCloud custom domain wants `imap.mail.me.com` and `smtp.mail.me.com`, with
`POSTIO_USERNAME` set to the Apple ID address rather than the custom one. And
iCloud needs an **app-specific password** — an Apple ID password will not
authenticate. Create one at appleid.apple.com under Sign-In and Security, with
two-factor authentication on, and revoke it there when you are done testing;
that takes effect immediately.

### Re-running it

Safe, and deliberately inert: an address already in the store is reported and
left alone. It will not write a second row for one address, and it will not
overwrite a password that is already working — a re-run from a shell whose
environment had drifted would otherwise break an account that was syncing
perfectly well. Repairing an account is onboarding's job, where there is a
person to confirm it.

## The Focus app: targets, and who decides

`Package.swift` has three Swift targets over the generated `PostioFFI`:

- **PostioKit**: models, plans, policy, the intent applier. **No AppKit**, so
  the package can reach iOS (#1264); `NoAppKitTests` fails if an `import
  AppKit` appears. Anything a view needs to know that is not a view goes here.
- **PostioAppKit**: tables, panels, popovers, windows, the menu bar, key
  handling. Depends on PostioKit, never the reverse.
- **Postio**: the executable: app, main window, engine start-up.

**Behaviour lives in the Rust `postio-focus` controller** (the one GTK's
window also drives), not here. Keys, menus and clicks go down through
`invoke`, `focusPoint` and `focusPick`; its answers come back on `nextEvent`
as `UiEvent`s, and `FocusIntents` is the one switch that applies them.
`FocusIntents` decides nothing: a field it changes that an event did not name
is the Mac keeping an opinion of its own, which is the bug the controller
exists to end. A rule you want to change is changed in `crates/postio-focus`,
with a controller test first, and both apps get it.

**Words come from `postio-ui`** (through the FFI), never Swift literals:
labels, hints, footers, key spellings, toasts.

**Intercepted keys.** `Intercepted` (PostioKit/Accessibility.swift) lists the
commands this frontend presents a window for instead of dispatching. It
mirrors `postio_ffi::registry::INTERCEPTED`, and
`theInterceptedListsAgreeAcrossTheBoundary` keeps the two equal; change both.
Keep it short: a command the controller handles (`/`, `g o`, `?`, the pickers)
does not belong on it.

**Events, exports, and what is not echoed.** Surfaces the controller opens or
closes (the bar, a picker, the key map, the message window by `Return`) are
not reported back with `focusSurfaceOpened` / `focusSurfaceClosed`: an echo
can land after a reopening and close the new surface. Swift reports only
toolkit-only facts the controller cannot know: a click on the dimmed area or
outside a popover (a surface the toolkit closed), a click on a row, the list
scrolled to its top, and reader state. A new surface is a new export on
`postio-ffi` and a new `UiEvent` case, not a new decision in Swift.

**`SecondaryWindowController`** owns the one window over the list (message,
digest, composer, capture). **One at a time** (M4): opening another kind
closes the first and reports it closed, and the same kind again keeps the
frame. Do not open a second secondary window from a view; ask the controller.

**The message body** is the engine's treated document
(`focus_reader_document`), rendered in a hardened WebKit view with no network
(`ReaderPolicy`). The contrast guard lives in the shared markup, not in a
Swift colour.

## Seeing it: the demo, and the shot script

`POSTIO_DEMO=<seed>` starts the app over an in-memory store of invented mail
(a build with the FFI `demo` feature; `scripts/macos-build.sh` does it for
`macos-shot.sh`). It reads no Keychain and touches neither disk nor network.
Seeds: `small` (the inbox), and `small:<n>` for the screen `n` needs
(`small:22` the digest, `small:23` a digest email, `small:25` capture with a
throwaway vault).

- `POSTIO_DEMO_KEYS='j j x ⏎'` presses keys once the first page has landed,
  through the resolver as a person would (`⏎` Return, `⎋` Escape).
- `POSTIO_DEMO_STATE=offline|auth|first-sync|synced` makes the account's sync
  state what the engine would have reported.
- `POSTIO_WINDOW_SIZE=1440x900`, `POSTIO_APPEARANCE=light|dark`.

`scripts/macos-shot.sh <name> --seed small --both` builds, launches over the
seed, photographs the window and its children with `screencapture -l`, and
quits; pictures go to `Design/review/focus-macos/` (untracked). It needs
Screen Recording for the terminal once. WebKit does not paint a web view in a
window it thinks is covered, so the demo opens its windows in front; do not
work around that by photographing a window another app covers.

`scripts/macos-test.sh` runs the Swift tests with the library linked (a bare
`swift test` will not link). It runs `macos-build.sh --lib-only` first.

## The bindings are generated, never edited

`macos/Sources/PostioFFI/` and `macos/Sources/postio_ffiFFI/` are **build
products** and are gitignored. `scripts/ffi-bindgen.sh` writes them from the
Rust crate on every build, using a generator built from this same workspace —
so the generator and the `uniffi` runtime cannot skew. They can: uniffi writes
a per-function checksum into the Swift and verifies it at startup, so a
mismatch is a `fatalError` on launch, a long way from the change that caused it.

Editing them is always wrong. The next build overwrites it.

## What belongs on which side

**Rust**, always: anything that decides something. Which command a key runs,
what a reader document contains, how a list pages, what a body's absence means.
Both frontends share those, and *"shared core does not mean shared behaviour —
anything a frontend interprets will drift"* (ADR 0019).

**Swift**, only: views, and platform observation the platform's own language is
better at. `NWPathMonitor` and `UNUserNotificationCenter` are Swift's, and they
push *down* into the engine through a setter rather than being asked for by it.

If you find yourself writing a rule here, it is on the wrong side.

## Running it, and what a blank window means

`scripts/macos-bundle.sh` then `open macos/build/Postio.app`. The application
draws its window *before* it asks for the store's key, so a Keychain prompt
arrives over a visible Postio — it did not before #1146, and the symptom was an
icon in the Dock and nothing else, forever.

**`POSTIO_LOG` works here**, the same `EnvFilter` the GTK build takes
(`POSTIO_LOG=debug`, or `postio_sync=debug`), and `[logging]` in `config.toml`
retunes a running instance. `PostioSession.startLogging()` installs it as the
first thing `Engine.init` does — before opening the store, because opening the
store is the call most worth tracing.

To see what a stuck launch is stuck on, `sample Postio 2`. It is very good at
this: one command named the blocked call, its caller and the SwiftUI entry
point that made it happen. Anything under `NSApplication run` is the ordinary
event loop; anything else on `com.apple.main-thread` is a bug.

## Things that will bite

- **A nested `enum State` shadows SwiftUI's `@State`.** The error names
  neither. Call it something else.
- **Opening a session reads the login Keychain.** An unsigned build has a new
  code identity on every rebuild, so macOS asks again each time. Anything that
  can work without a session — the command registry, for one — should.
- **`swift test` needs the library on the linker path.** Use
  `scripts/macos-test.sh` rather than a bare `swift test`.
- **`unset RUSTUP_TOOLCHAIN` (and `MAKEFLAGS`) before any cargo command.** `RUSTUP_TOOLCHAIN` in the
  environment beats `rust-toolchain.toml` — the caveat that file names — and
  `MAKEFLAGS` points at the Linux workstation's jobserver fifo, which does not
  exist here. Both have been observed set on a development Mac.
- **The privacy check reads this directory.** `check-no-silent-tracking.py`
  scans `macos/Sources/**/*.swift` and refuses `URLSession`,
  `NSWorkspace.shared.open` and friends without a `POSTIO-CONSENT:` comment
  saying how the user asked for it. That is not bureaucracy: the reader's whole
  claim is that its web view has no network.
