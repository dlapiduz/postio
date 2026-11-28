# ADR 0019 — A native macOS frontend over `postio-session`

- **Status:** Accepted, maintainer-directed (2026-08-27); built (`macos/`, `crates/postio-ffi`). Since spec 009 the Mac app is a Focus interface: Focus's behaviour lives in `postio-focus` ([ADR 0045](0045-focus-behaviour-lives-in-postio-focus.md)).
- **Decision:** a native Swift (SwiftUI + AppKit) frontend over the same Rust engine, through a UniFFI boundary in `postio-ffi`. Toolkit-free presentation logic is shared through `postio-ui` and `postio-focus`. The model is Ghostty's.

## Rules
- **Native, not GTK on macOS.** `webkitgtk` has no macOS build, and a GTK app there has the wrong menu bar, shortcuts and chrome.
- **Seam shape.** `postio-ffi` exposes a floor of about fifteen functions plus an append-only `UiEvent` surface that a frontend may ignore variant by variant. The rule for adding one sits at the definition. The Swift dispatcher ends in a `default:` that logs and returns false.
- **UniFFI**, not a hand-written C ABI: `async fn` becomes Swift `async`, and `Result<_, SessionError>` stays distinguishable (`SecretError::Locked` must reach the unlock surface). It compiles under `unsafe_code = "forbid"`, so no lint exception. `postio-ffi` is private to the macOS app and promises no stability.
- **Commands cross as name strings.** Swift calls `commands()` once and derives palette, cheat sheet, menu bar and key hints from the registry. A new command needs no boundary or Swift change. The `Command` enum is not mirrored.
- **The list is pull, never materialised.** `row_count()`, `request_page(generation, page)`, and a synchronous `take_page()` over a bounded resident cache with no I/O; a miss returns a placeholder and `UiEvent::PageReady` reloads that range. No API lets `await` appear in a table delegate.
- **The reader gets a finished document** from the one Rust function both frontends call (CSP, font faces, tokens, sanitised body, scroll markers). Swift never composes reader HTML. Faces are served over `postio-font:` ([ADR 0023](0023-reader-fonts-are-served-not-inlined.md)); the Swift reader registers both scheme handlers. The CSP is asserted byte-for-byte in `postio-ui`. A loopback-listener test asserts zero connections with remote images blocked and one when allowed. WebKit exposes no WebRTC, WebGL or WebAudio toggles; script off plus `default-src 'none'` closes them, and the call site says so.
- **Links** open in the user's browser, carry `POSTIO-CONSENT:`, and `check-no-silent-tracking.py` scans `macos/Sources/**/*.swift`.
- **The core owns the keyboard.** One binding table; frontends send a reduced key event, ask `trigger_for_command(id)` and render the accelerator (`<Ctrl>N`, `⌘N`). `[keys]` has the `mod` token, resolved at keymap build against a `Platform` (Control on freedesktop, Command on Apple); `ctrl` stays literal. Dispatch is a window-level `NSEvent` monitor, not `.keyboardShortcut`; menu items get no key equivalent.
- **`postio-ui`** holds toolkit-free logic (selection, tokens, palette, keymap, `ListWindow<T: ListRow>`, reader document assembly). `ListWindow` owns the paging bookkeeping and resident rows; the toolkit owns row identity, change notification and re-entrancy. Test for what is shared: would a second frontend re-derive it; does it exist for a toolkit contract; would a subtle error fail loudly (if not, share it).
- **Linux stays green.** `postio-ffi` builds and tests on Linux. `issue-land.sh` refuses to land a crate the host cannot build (probes `pkg-config`); dependents land with `needs-linux-verify`. macOS issues carry `ready-mac`, not `ready`. A crate the host never compiled has not passed.
- **Build.** `macos/` uses SwiftPM, no `.xcodeproj` (merge-hostile for agents). cargo to staticlib to `.xcframework` as a SwiftPM binary target. Release asset `postio-<version>-macos-arm64.zip` is ad-hoc signed, not notarized. Two entitlement files. Build loops are in `macos/CLAUDE.md`.
- The OAuth loopback redirect stays on both platforms (ADR 0006 rejected a custom scheme for freedesktop reasons only).

## Why
- The three things Postio must win at (speed, search, keyboard) are engine properties. Thirteen of fifteen crates built and tested on macOS unchanged.
- One implementation of the privacy-critical reader beats two that agree until they do not.

## Revisit when
- Developer ID signing and notarization are wanted. The abstraction makes starting cheap, not finishing; a stalled frontend must leave the Linux app better off.
