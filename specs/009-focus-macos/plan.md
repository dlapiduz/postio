# Implementation Plan: Postio Focus on macOS

**Branch**: `feature/focus-macos` | **Date**: 2026-10-07 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/009-focus-macos/spec.md`

## Summary

The Mac app becomes a Focus interface over the same engine as Linux.

1. **Move Focus's behaviour out of GTK.** It goes into a new sans-IO crate,
   `postio-focus` (R1), extracted from `postio-gtk/src/window.rs` in twelve
   behaviour-neutral slices. Each slice lands on `main` with GTK adopting it
   in the same PR (R2).
2. **Turn the FFI into a Focus driver.** It registers as `Frontend::Focus`,
   enables the engine's Focus pass, drives the controller, and exports the
   Focus reads, the treated reader document, the undo top, links and
   contact suggestions (R5, R7–R9, R11).
3. **Rebuild the Mac UI.** It is rebuilt in Focus's frames, in the brief's
   order, on `feature/focus-macos`. AppKit holds the list, the windows, the
   panels and the popovers; SwiftUI holds their content; `WKWebView` holds
   bodies (R9, R10). The classic three-pane app is deleted as it is
   replaced.

## Technical Context

**Language/Version**: Rust 1.99.0 (`rust-toolchain.toml`; on this Mac unset
`RUSTUP_TOOLCHAIN` first); Swift tools 6.0 (local compiler 6.4)

**Primary Dependencies**:
- **Rust:** `postio-client`, `postio-ui`, `postio-core`, `postio-config`,
  `postio-body`, `postio-search`; UniFFI 0.32 (proc-macro) in `postio-ffi`
  only.
- **Swift:** AppKit, SwiftUI, WebKit, Contacts, UserNotifications.

**Storage**: The one turso store, through the host (ADR 0038, 0041). No
schema change. Config is `config.toml` in `~/Library/Application
Support/Postio/`.

**Testing**:
- **Rust:** `cargo test -p postio-focus` for the controller, unit tests
  that run on both hosts. `cargo nextest run -p postio-ffi --test ffi_suite`.
  `postio-gtk`'s `focus_suite` runs in CI only, since it cannot build on the
  Mac.
- **Swift:** Swift Testing via `scripts/macos-test.sh`.
- **Screens:** per-screen PNG comparison (R12).

**Target Platform**: macOS 14+ (arm64 release). Linux stays green on every
commit.

**Project Type**: desktop app (Rust engine + native Swift frontend)

**Performance Goals**: startup < 500 ms; key → frame < 16 ms; list scrolls
10k+ conversations at 60 fps; local search < 100 ms (constitution V)

**Constraints**:
- Local-first: the UI never awaits the network.
- No remote content by default.
- `PostioKit` imports no AppKit (#1264).
- The controller does no I/O.
- No worktree path in anything rustc sees.

**Scale/Scope**:
- 25 screens plus 9 message-window screens.
- 12 controller slices.
- About 6 FFI export groups.
- About 40 Swift files rewritten or deleted.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design.*

| Principle | How this plan meets it | Status |
|---|---|---|
| I. Local-first, the UI never awaits the network | The controller emits `Ask` and never awaits. Swift applies intents on the main actor. Every verb is store-write-then-emit in the host, as today | Pass |
| II. One command table | Keys, the menu bar, the palette, the key map and keycaps all come from the registry through the controller's resolver. Mac conventions (M6) are registry defaults or alternates. Three-pane commands are deleted, not hidden | Pass |
| III. One query language | Search chips come from `postio_search::natural` through the existing FFI search, with no Swift parsing | Pass |
| IV. Test-first | Each slice's rules are unit-tested red in `postio-focus` before they move. Behaviour changes (R2) get their own failing test first. Swift view models and the keymap layer are tested without a window | Pass |
| V. Performance | `press` is synchronous and in memory. The list stays windowed (`ListWindow`), with one resident copy. Counts are gated as counts | Pass |
| VI. Privacy | `WKWebView` with JS off, a non-persistent store, CSP plus a content rule list, no base URL. Contacts are read locally after one prompt, and never stored or logged. Logs carry ids and counts only | Pass |
| VII. Boundaries enforced | `postio-focus` gets a rule in `check-crate-boundaries.py` (R1). The FFI stays the only Swift boundary. `PostioKit` has no AppKit import, checked by a test | Pass |
| Workflow: one feature branch, one PR | **Deviation**: the controller slices land on `main` separately. See Complexity Tracking | Justified |

Re-check after Phase 1: no new violations. The contracts keep the controller
free of toolkit and I/O types. The FFI exports mirror existing client
requests, plus three small additions: `UndoTop` (R7), links and contact
suggestions (R11), and the contrast guard (R9).

## ADR

One new ADR, because it is a rule that outlives this feature:

- **ADR 0045: Focus's behaviour lives in `postio-focus`; frontends draw its
  intents.** It records the rule, the crate boundary, and the obligation that
  no Focus rule lives in a window. The reasoning stays in this spec.

ADR 0019 is amended (Status: partly superseded by spec 009). Its boundary,
packaging and Linux-stays-green rules still hold; its three-pane surface and
read-only framing do not. Spec 007's Mac brief (`macos.md`) is folded into
this spec and deleted in the same branch.

## Project Structure

### Documentation (this feature)

```text
specs/009-focus-macos/
├── spec.md
├── plan.md               # this file
├── research.md           # R1–R12
├── data-model.md         # controller state, intents, FFI records
├── quickstart.md         # how to run and verify each phase
├── contracts/
│   ├── focus-controller.md   # postio-focus's public surface
│   ├── ffi-focus.md          # what postio-ffi exports for Focus
│   └── mac-surfaces.md       # each Mac surface: frame, anchor, keys, close and focus rules
├── checklists/requirements.md
└── tasks.md              # /speckit-tasks
```

### Source Code (repository root)

```text
crates/
├── postio-focus/                 # NEW: sans-IO Focus controller (R1)
│   ├── src/lib.rs                # FocusController, Input, Effect, Intent, Request, Reply
│   ├── src/perform.rs            # async perform(&Client, Request) -> Reply
│   ├── src/{feed,cursor,verbs,keys,surfaces,reader,compose,bar,pickers,states,digest,filtered,capture}.rs
│   └── tests/                    # rule tests, both platforms
├── postio-ui/src/
│   ├── selection.rs              # + Selector (R3)
│   └── focus_dialog.rs           # geometry keyed by Platform (M1)
├── postio-body/src/treatment.rs  # + contrast guard for kept colours (R9)
├── postio-core/src/registry.rs   # − Frontend::Macos, − ThreePane; Delete gains alt BackSpace (R5, R6)
├── postio-session/src/actions.rs # + peek_description (R7)
├── postio-client/src/{protocol,api}.rs  # + UndoTop (R7)
├── postio-ffi/src/
│   ├── session.rs                # Focus driver; enable_focus; config watch; − three-pane
│   ├── focus.rs                  # NEW: Focus exports and *Ffi mirrors (contracts/ffi-focus.md)
│   ├── event.rs                  # + SurfacedChanged, BackfillProgress, KeymapChanged, Intents
│   └── links.rs, contacts.rs     # NEW: postio://, recipient_suggestions (R11)
└── postio-gtk/src/window.rs, list/feed.rs, …  # shrink to drivers, slice by slice

macos/
├── Package.swift                 # + PostioAppKit target
├── Resources/Info.plist          # + postio scheme, NSContactsUsageDescription
├── Sources/PostioKit/            # view models, intent policy, no AppKit
├── Sources/PostioAppKit/         # NEW: list table, windows, panel, popovers, undo manager, menu, keys
├── Sources/Postio/               # app, scenes, composition root
└── Tests/PostioKitTests/ (+ PostioAppKitTests/)

scripts/
├── macos-shot.sh                 # NEW: launch over the demo store, capture 1440×900 light/dark
└── checks/check-crate-boundaries.py  # + postio-focus rule
```

**Structure Decision**: as above. Rust layering follows ADR 0043 rule 1:
logic goes in the lowest layer that can hold it. Swift layering follows
#1264.

## Phases of delivery

The order is the brief's, and FR-060. Each phase is a runnable app and one or
more commits on `feature/focus-macos`. The controller slices ride ahead of it
on `main`, and the branch is rebased onto each as it lands.

| Phase | Mac surfaces | Controller slices (land on `main` first) | FFI |
|---|---|---|---|
| 0 | none | 1 skeleton and geometry | `Frontend::Focus`, `enable_focus`, config watch |
| 1 | Inbox 01, 02: toolbar, header strip, `FocusListTable`, day headers, markers. Three-pane shell deleted | 2 feed | Focus scopes, `FocusRow`, surfaced, counts, `Intents` event |
| 2 | 03: keys, selection, action bar, Has action; undo via ⌘Z and the Edit menu | 3 cursor and selection, 4 verbs and undo | `undo_description`, verb exports |
| 3 | 04: the email window, both treatments | 5 keys and surfaces (+5b, #1754), 6 email | treated document, raw source, contrast guard |
| 4 | 05, 06: compose and reply | 7 compose | `recipient_suggestions`, Contacts |
| 5 | 07–10: bar, search, go-to, folders | 8 bar and places | — |
| 6 | 11–15: pickers, undo pill | 9 pickers and toast | — |
| 7 | 16–19: states | 10 states | `start_over`, typed backfill |
| 8 | 20: key map; menu bar from the registry | 11 keymap | `KeymapChanged` |
| 9 | 21–25: filtered, digest, rule, capture, `postio://` | 12 filtered, digest, capture | digest, vault and capture exports; links |
| 10 | M3 extras, on both apps | in the controller and the shared crates | — |

## Complexity Tracking

| Violation | Why needed | Simpler alternative rejected because |
|---|---|---|
| Controller slices land on `main` as separate PRs, not in the feature branch's one PR | `window.rs` changes under other sessions daily. A second copy of its rules on a long branch would diverge, and the merge is where the bugs would be (#901). Each slice is behaviour-neutral for Linux and invisible to the Mac until the branch lands | Keeping them on the branch means one large PR rewriting GTK's window. It could not be verified locally, since GTK does not build on the Mac, and it would be reviewed against a moving `main` |
| A new crate (`postio-focus`) | `postio-ui` cannot depend on `postio-client` (a cycle), and GTK cannot depend on the FFI | A module in `postio-ui` would invert the client–UI dependency, a larger change than one new crate |
| One new engine request (`UndoTop`) | Edit › Undo must name the engine's top entry (FR-040) | A Swift-side mirror of undo drifts on expiry and on undo from the pill |
