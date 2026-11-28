# Implementation Plan: The native composer, over the document

**Branch**: `feature/native-composer` | **Date**: 2026-11-22 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/015-native-composer/spec.md`

## Summary

ADR 0039's four steps, with its spike gate between the first and the third,
and spec 014's held tasks as the last phase.

1. **The algebra first.** `EditIntent`, `apply`, `project` and the preedit
   hole in `postio-body`, proven with no UI (Phase 2). Independently
   landable.
2. **The old surface, routed.** `beforeinput` goes to Rust on both
   frontends; the JavaScript copies of the rules go (Phase 3). The Mac
   needs a host half here because it shares `editor.js`.
3. **The new surface, beside it.** A `GtkTextView` behind a developer switch
   until the composer suites pass (Phase 4).
4. **One surface per platform.** GTK deletes the WebView, `webkit6` and the
   dialect test in one commit (Phase 5). The Mac follows and deletes the
   shared script (Phase 6, Q1 = B).
5. **Spec 014 unblocked** on the native surface (Phase 7).

## Technical Context

**Language/Version**: Rust 1.99.0 (`rust-toolchain.toml`); GTK 4.20 /
libadwaita (the crate builds with `v4_20`); Swift tools 6.0.

**Primary Dependencies**: `postio-body`, `postio-ui`, `postio-core`,
`postio-widgets`, `postio-gtk`, `postio-ffi` (UniFFI), `postio-session`
(unchanged). New: `libspelling` (Q3, optional, system library). Removed:
`webkit6`. No new crates.

**Storage**: none. The draft stores `body_html` and `body_markdown`, as now.

**Testing**:
- **Rust, fast**: `scripts/test-fast.sh`; `cargo nextest run -p postio-body
  --test body_suite apply projection`.
- **Widgets**: `cargo nextest run -p postio-widgets --test widgets_suite
  composer_ editor_` on the headless compositor (`POSTIO_HEADLESS=0` to
  watch).
- **GTK composition**: `-p postio-gtk --test focus_suite`.
- **Swift**: `scripts/macos-test.sh`.
- **Storyboards**: `storyboards/compose/native-*.toml`.
- **Doctests**: `cargo test -p postio-body --doc`.

**Target Platform**: Linux (GTK Focus and classic), macOS 14+.

**Project Type**: desktop app, Rust engine, GTK and Swift frontends.

**Performance Goals**: typing under 16 ms in a 5 000-word draft, gated as
counts (blocks cloned, buffer operations, tag applications per keystroke);
composer open under 500 ms; no web process.

**Constraints**:
- `postio-body` stays a pure leaf; `check-crate-boundaries.py` enforces it.
- `postio-widgets` depends on no app, store, protocol or host.
- The composer reaches no network, loads no URL, runs no script.
- The surface decides nothing the engine can decide (ADR 0045, FR-023).
- `PostioKit` imports no AppKit (#1264); `NSTextView` lives in
  `PostioAppKit`.
- Another build may be running on this machine; none runs while this spec is
  drafted.

**Scale/Scope**: about 12 `EditIntent` arms on `main`'s document (plus 014's
about 10), 1 new `postio-body` module family, about 9 new files under
`composer/native/`, about 6 files deleted on GTK and 3 on the Mac.

## Constitution Check

| Principle | Check |
|---|---|
| I. Local-first | An edit is `apply`, then history, then patch. No network is awaited; the spell dictionary is local. |
| II. The keyboard is a system | Commands stay registry ids in `Context::Composer`. Caret motion stays the toolkit's. A coverage case fails on an unbound key the surface handles (FR-023). Storyboards for every keyboard path. |
| III. Search | Not touched. `to_search_text` is unchanged. |
| IV. Test-first | Every task names its red case. The algebra is `--lib` tier; surfaces assert composition only (ADR 0049). |
| V. Performance | Gated as counts (T013, T032, T044); no real waits (T040 uses a paused clock). |
| VI. Privacy | Removing the browser engine makes "no network from compose" structural (US5). Logs carry no text (FR-025). Paste is narrowed by `parse` (FR-007). |
| VII. Boundaries | Rules in `postio-body` and `postio-ui`; widget in `postio-widgets`; Swift decides nothing; `postio-ffi` is the only Swift boundary. No new crate. |

Re-check after Phase 1 design: no violations. The one tension is
`EditStep` gaining selections (`postio-body`'s `edit.rs`), which is a
change to an existing public type; it is a leaf and there are no installs to
protect.

## No ADR

D1 to D7 are this feature's and live in `spec.md`. The rule other work must
obey, that **nothing mutates the composer's text except `apply`'s patch**, is
recorded in `postio-body`'s module doc and in `docs/ARCHITECTURE.md`'s entry
for `postio-widgets`. It is a boundary note. ADR 0039 is amended (T076); no
parallel ADR (CLAUDE.md).

## Design by crate

### `postio-body`

- New: `position.rs` (`DocPosition`, `Selection`), `projection.rs`
  (`project`, `Projection`, `Run`, `Preedit`, `Patch`), `edit_ops/`
  (`text`, `marks`, `blocks`, `lists`, `paste`, `convert`, and `mod.rs` with
  `EditIntent`, `Edited`, `BlockRange`, `Join`, `apply`).
- `edit.rs`: `EditStep` gains the two selections; `record` takes `Join`.
- `lib.rs`: exports. Module doc names the rule above.
- Tests: `body_suite/apply.rs`, `projection.rs`, `conformance.rs`;
  `document.rs` gains the dialect contract (T049).

### `postio-ui`

- `editor/markdown.rs`: `conversion_before` over the existing table.
  `markdown_table_js` and `compose::mark_script`, `link_script`,
  `image_script` are deleted in Phase 3. The table is data and stays here.
- `editor/input.rs`: `intent_of` for step 2's bridge.
- A shared `format_state(selection, document)` for the toolbar, used by GTK,
  Mac and the terminal's reflection if it wants it.
- `editor/document.rs` loses its stylesheet and CSP (Phases 4 and 6); tokens
  move to the widget CSS.

### `postio-widgets`

- Phase 3: `editor.rs` gains the `postioIntent` handler; the bridge shrinks.
- Phase 4: `composer/surface.rs` (trait), `composer/native/{mod, render,
  patch, input, preedit, selection, clipboard, image, quoted, history,
  spelling, a11y, css}.rs`. The `Composer` widget in `mod.rs` holds a
  `Rc<dyn ComposerSurface>`.
- Phase 5: deletes `editor.rs`, `editor.js` GTK half, `scheme.rs`,
  `web_process.rs`, the WebKit suites; drops `webkit6`.

### `postio-ffi`

- `editor_doc.rs`: a UniFFI object holding one `Document`: `apply`,
  `project`, `patch`, `html`, `load`. The Mac holds the object, not HTML
  strings, so an edit is block-local rather than a parse of the whole
  document.

### Swift (`macos/Sources`)

- Phase 3: `ComposeEditor.swift` answers `postioIntent`.
- Phase 6: `PostioKit/ComposeSurfaceModel.swift` (logic, no AppKit),
  `PostioAppKit/NativeComposeTextView.swift`; `ComposeEditor.swift` loses
  `WebKit`.

### The step 2 bridge

`editor.js` after Phase 3 does three things: forwards `beforeinput` as
`{ inputType, data, start: DocPosition, end: DocPosition }`; tells the host
when a composition starts and ends; and applies `{ range, html, selection }`
by replacing the changed top-level blocks and placing the caret from the
block path. `intent_of` returns `Native` for composition and plain
insertion (research R5), and the DOM keeps those until `compositionend`.

## Build order

Each phase ends runnable, and ends with a commit.

| Phase | Lands as | User-visible |
|---|---|---|
| 2 | `feat(body,ui)`: the algebra | none |
| 3 | `refactor(widgets,ui)`: the WebView routed; Mac host half | none |
| 4 | `feat(widgets)`: the native surface behind a switch | none (switch off) |
| 5 | `refactor(widgets)`: one surface on Linux | the composer is native |
| 6 | `feat(macos)`: the Mac composes natively | the Mac's composer is native |
| 7 | spec 014's held half | live formatting, pickers, tables |

## Performance plan

- `apply` is linear in the edited block, not the document (FR-003). Its
  property test counts blocks.
- `Projection::patch` returns operations for the changed range only;
  `PatchStats` is asserted per keystroke.
- No `Document::to_html` per keystroke. Autosave serialises on its own
  timer, as now.
- Open: no web process, so the cold path is a widget build and a `project`
  of the draft. A count asserts it.

## Test plan

| Layer | Asks | Where |
|---|---|---|
| Rule | what each intent does, the generated-document property, the preedit hole | `postio-body` `body_suite` |
| Rule | which text triggers a conversion; key to intent | `postio-ui` units |
| Wire | the Rust object and the Swift model agree | `ffi_suite`, `PostioKitTests` |
| Composition | the surface turns inputs into intents and patches, one case per path a person takes | `widgets_suite`, `focus_suite`, Swift |
| Behaviour | the keyboard paths | `storyboards/compose/` |
| Conformance | the same input script gives the same document on both toolkits | `composer-conformance.toml` |
| Structure | no browser engine | `check-no-browser-engine.py` |

## Risks

1. **The caret** (ADR 0039's first falsifier). Gated by T015 to T017.
2. **The Mac shares `editor.js`.** Step 2 changes it, so the Mac host half
   (T025, T026) ships in the same PR as T020. If no Mac session is
   available, Phase 3 waits; Phases 2 and 4 are unaffected, and Phase 3's
   Linux half can be held on a branch.
3. **Spec 014 widens `Document`** under this work. Rebases are mechanical:
   every `match` on `Block` and `Inline` in `edit_ops` fails to compile
   until it handles the new kind.
4. **`Quoted` is rich.** Q2.
5. **`libspelling` as a system dependency.** Q3.
6. **A `GtkTextView` re-render is expensive on a long draft.** D4's patching
   is the mitigation; T044 measures it.

## Project Structure

### Documentation (this feature)

```text
specs/015-native-composer/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── tasks.md
├── contracts/
│   ├── edit-intent.md
│   ├── surface.md
│   └── spec-014-unblock.md
└── checklists/requirements.md
```

### Source Code (repository root)

```text
crates/postio-body/src/{position.rs, projection.rs, edit_ops/}   # new
crates/postio-body/src/edit.rs                                    # EditStep selections
crates/postio-ui/src/editor/{markdown.rs, input.rs}               # matcher, bridge intents
crates/postio-widgets/src/composer/{surface.rs, native/}          # new
crates/postio-widgets/src/composer/{editor.rs, scheme.rs, web_process.rs}  # deleted, Phase 5
crates/postio-ffi/src/editor_doc.rs                               # new
crates/postio-body/tests/{body_suite/apply.rs, projection.rs, conformance.rs, data/}
macos/Sources/{PostioKit/ComposeSurfaceModel.swift, PostioAppKit/NativeComposeTextView.swift}
storyboards/compose/native-*.toml
scripts/checks/check-no-browser-engine.py
```

**Structure Decision**: no new crate. The algebra goes in `postio-body` as
ADR 0039 says, the surface in `postio-widgets` at the path spec 014's
tasks already name.

## Complexity Tracking

| Item | Why | Simpler alternative rejected because |
|---|---|---|
| A projection type with runs, in `postio-body` | The offset map is the one new mechanism and must be testable without a window | Doing it in the widget makes the Mac redo it and leaves the falsifier untestable |
| Two surfaces behind a developer switch for Phases 4 to 5 | The WebView composer works; ADR 0039's own sequencing keeps it until the new one passes | A branch that replaces it at once cannot be merged in pieces |
| The step 2 `Native` exception for composition | A composition event cannot be cancelled | Breaking IME on the WebView to be tidy |
| `EditorDoc` as an FFI object | Holding the document avoids a parse per keystroke on the Mac | Passing HTML each call is linear in the draft |
