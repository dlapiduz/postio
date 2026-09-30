# Implementation Plan: Postio in the terminal

**Branch**: `feature/tui-frontend` | **Date**: 2026-09-23 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/005-tui-frontend/spec.md`

## Summary

A second Linux frontend, `postio-tui`, built on ratatui and crossterm, with
the same commands, keys, store and behaviour as the GTK app. It reads mail as
Markdown-styled text, writes it in Markdown, works with the mouse, and takes
dropped and pasted files and images.

The terminal is the smaller half of the work. The larger half is sharing
one store. The store-side logic that lived in `postio-app` moves into
`postio-host`, and every frontend -- GTK, the terminal, macOS -- runs that host
inside its own process and reaches mail through `postio-client`. That move is
FR-004's "logic once" as well as FR-040's shared store. Only one app has the
store open at a time; the other says so (FR-041).

> **Revised 2026-09-24.** This plan first made both apps open at once true
> with a headless `postio-daemon` owning the store and every frontend its
> client over a Unix socket. It was built, and the maintainer withdrew it as
> too much complexity (spec Clarifications, 2026-09-24). What follows keeps
> the daemon's sections as the record of that design; where they disagree
> with the paragraph above, the paragraph above is what holds, and ADR 0041
> is the rule. research.md R1 says the same at its top.

## Technical Context

**Language/Version**: Rust 1.98.0, edition 2024 (pinned by `rust-toolchain.toml`)

**Primary Dependencies**:
- New, terminal side: `ratatui 0.30`, `crossterm 0.29` (features:
  `bracketed-paste`, `osc52`), `ratatui-textarea 0.9`, `tui-input 0.15`,
  `tui-markdown 0.3`, `linkify`, `arboard 3.6` (`wayland-data-control`,
  `image-data`), `terminal-colorsaurus 1.0`.
- New, `postio-body`: `pulldown-cmark 0.13` and `htmd`.
- New, dev: `insta 1.48`.
- Existing, reused: `postio-session`, `postio-ui`, `postio-core`,
  `postio-body`, `postio-config`, `tokio`, `serde`.
- All MIT, Apache-2.0 or BSD. No GPL or AGPL code is copied (research R3).

**Storage**: The existing encrypted Turso store and blob directory, now opened
only by `postio-daemon`. One schema change: `drafts.body_markdown TEXT`,
NULL by default (research R6). The store's canonical location becomes
`$XDG_DATA_HOME/postio` for every package (research R10). Existing flatpak
stores resync rather than migrate.

**Testing**: `cargo nextest`. Terminal screens go through ratatui
`TestBackend` with `insta` buffer snapshots and synthetic crossterm events.
No tty and no compositor are needed, so these run in the default suite.
Engine-backed tests use the in-process transport over the `MailBackend` mock.
A two-client suite runs against a real daemon on a temporary runtime
directory.

**Target Platform**: Linux terminals (local and over SSH): 16-colour to
true-colour, with or without the kitty keyboard protocol and mouse
reporting.

**Project Type**: A desktop application's second frontend inside a Cargo
workspace (20 crates, becoming 23).

**Performance Goals**: The constitution's, unchanged: usable screen < 500 ms
from a populated store; < 16 ms per interaction; local search < 100 ms. Cold
start includes spawning the daemon, which opens the store in parallel with
the terminal's own setup.

**Constraints**:
- The UI never awaits the network.
- A mailbox is never loaded into memory. The terminal list draws only the
  `ListWindow` slice.
- No message content in logs, and none on the protocol's diagnostic output.
- No GTK, WebKit, store engine or display server in `postio-tui`'s dependency
  graph, enforced by `check-crate-boundaries.py`.
- Nothing may be removed from the GTK or macOS frontends (FR-005).

**Scale/Scope**: About 78 direct store and blob uses in `postio-app` move
behind the client API, drawing on ~6,000 production lines in `reading.rs`,
`compose.rs`, `onboarding.rs`, `search.rs`, `notifications.rs`, `export.rs`
and `settings_*.rs` (research R1). Every command in `postio-core::registry`
must be reachable in the terminal frontend. Reference mailbox: ~81,000
messages.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design.*

| Principle | Gate | Status |
|---|---|---|
| I. Local-first; the UI never awaits the network | Writes land locally first; no visible outcome waits on a server | **Pass.** The daemon runs the same `store → enqueue → emit` order. A frontend waits on a *local* socket round trip (tens of µs), never on the network. Watch: no client call may block the terminal's draw loop; client calls are async and the loop keeps drawing. |
| II. The keyboard is a system | Every command has a key, a palette entry and an accessible action, all from the registry | **Pass by construction.** The terminal keymap *is* `postio_ui::keymap::Resolver` over `registry::all()`; the palette and cheat sheet are `postio_ui::palette`/`cheatsheet`. Legacy-terminal chords are *added* as `alternate_bindings`, a registry change GTK also sees (research R4). SC-001 is an enumeration test. |
| III. Search is navigation; one query language | Same query, same meaning, everywhere | **Pass.** Queries are parsed by `postio-search` and executed in the daemon by the existing `search::execute`. The terminal adds no syntax. |
| IV. Test-first | Every task's test seen red first | **Pass.** Tasks are written that way. Terminal screens are asserted as rendered text, which is what a person sees. |
| V. Performance, gated as counts | Read paths carry statement/row assertions | **At risk → action.** A process boundary adds round trips, which counting at the storage seam cannot see. So `postio-client` gets a round-trip counter, and each keystroke path asserts a bound on it (research R11). Cold start with a daemon to spawn must be measured early (Phase 0 spike T0.3), not assumed. |
| VI. Privacy is a feature | Nothing leaves the machine unasked; no content in logs | **Pass, with two new surfaces handled.** (1) Terminal escape sequences in mail are this medium's script injection; one sanitising chokepoint plus a corpus test (R5). (2) The clipboard is read only on paste (R7). The socket is `0600` under `$XDG_RUNTIME_DIR`, never TCP. The key never leaves the daemon. `$EDITOR` temp files are `0600` and deleted. The daemon's lifetime default (exit when unused, R1b) keeps "syncs only while open". |
| VII. Boundaries are enforced | New crates have checked rules | **Action required.** `check-crate-boundaries.py` gains `postio-tui` and `postio-client` (banned: gtk4\*, libadwaita\*, gdk4\*, gsk4\*, webkit6\*, turso, rusqlite, io-imap). It also gains the missing `postio-ui` entry: `postio-ui/src/lib.rs:11` claims the check enforces it, and it does not. |
| Additional Constraints: Scope ("v1 is Linux only: GTK4 and libadwaita") | Work outside scope belongs on the roadmap | **Amendment required, maintainer-directed.** The maintainer asked for this frontend (2026-09-23). The Scope paragraph is amended on this branch to name the terminal frontend beside GTK (a MINOR bump, since it expands a section), with the Sync Impact Report updated. `PRODUCT.md` §2 and §23 follow. |
| Development Workflow | Spec-driven work lands once on one feature branch | **Pass.** Clarified: one landing with all seven stories. |

**No unjustified violations.** The Scope amendment and the new crates are
listed under Complexity Tracking.

**Re-check after Phase 1**: unchanged. The design adds no second query
language, keymap or composer generator. The protocol
([contracts/protocol.md](./contracts/protocol.md)) carries commands and events
the registry already defines, plus typed reads.

## Project Structure

### Documentation (this feature)

```text
specs/005-tui-frontend/
├── spec.md              # Written, clarified
├── plan.md              # This file
├── research.md          # Phase 0
├── data-model.md        # Phase 1
├── quickstart.md        # Phase 1
├── contracts/
│   ├── protocol.md      # client ↔ daemon
│   ├── markdown.md      # Markdown ↔ Document ↔ HTML mapping
│   └── tui-surface.md   # commands, CLI, screens, input, paste
├── checklists/
│   └── requirements.md  # Written, passing
└── tasks.md             # /speckit-tasks
```

### Source Code

```text
crates/
├── postio-client/           # NEW  what every frontend holds; no store engine
│   ├── src/protocol.rs      #   wire types, framing, version handshake
│   ├── src/socket.rs        #   Unix-socket transport; spawns the daemon
│   ├── src/api.rs           #   typed facade: commands, events, queries, writes
│   └── src/counting.rs      #   round-trip counter for budget tests
├── postio-host/             # NEW  the one owner of the store
│   ├── src/lib.rs           #   Host: Wiring + serve(); in-process transport
│   ├── src/reading.rs       #   ← postio-app/src/reading.rs store half
│   ├── src/compose.rs       #   ← postio-app/src/compose.rs store half
│   ├── src/onboarding.rs    #   ← postio-app/src/onboarding.rs persistence, OAuth loopback
│   ├── src/search.rs        #   ← postio-app/src/search.rs store half
│   ├── src/settings.rs      #   ← settings_*.rs writes
│   ├── src/allowlist.rs     #   ← postio-gtk/src/reader/allowlist.rs (format → TOML)
│   ├── src/clients.rs       #   per-client undo stacks, notifier election
│   └── src/bin/postio-daemon.rs
├── postio-tui/              # NEW  the terminal frontend
│   ├── src/main.rs          #   startup: query terminal, connect, raw mode
│   ├── src/app.rs           #   update(App, Event) -> Effects; no tty
│   ├── src/input.rs         #   crossterm → keymap::Chord; mouse → commands
│   ├── src/layout.rs        #   panes by width (ADR 0024's rule)
│   ├── src/view/            #   sidebar, list, reader, composer, palette, cheatsheet, settings, onboarding
│   ├── src/term.rs          #   suspend/resume, $EDITOR handoff, restore on panic
│   ├── src/clipboard.rs     #   arboard → wl-paste → xclip; OSC 52 out
│   └── tests/               #   TestBackend + insta suites
├── postio-body/             # + markdown.rs: from_html, to_document, from_document
├── postio-ui/               # + terminal.rs (text sanitiser), paste.rs (classify), colours role table
├── postio-core/             # + alternate_bindings for legacy-terminal chords
├── postio-storage/          # + drafts.body_markdown
├── postio-app/              # store/blob uses → postio-client; in-process host in tests
├── postio-ffi/              # Session wired over postio-host in-process (gains writes)
└── postio-session/          # unchanged API; the host composes it
flatpak/
├── dev.postio.Postio.json   # + postio-daemon; shared xdg-data/postio, xdg-run/postio
└── dev.postio.PostioTui.json# NEW  org.freedesktop.Platform
.github/workflows/release.yml# + tui-flatpak and tui-tarball jobs, same suite gate
scripts/checks/check-crate-boundaries.py  # + postio-tui, postio-client, postio-ui
docs/decisions/0041-one-app-opens-the-store-at-a-time.md  # NEW (Accepted 2026-09-25)
```

**Structure Decision**: Three new crates. `postio-client` and `postio-host`
are split so that a frontend's dependency graph contains no store engine: the
terminal binary stays small (SC-004), and the boundary check can prove it.
`postio-tui` is a binary crate on the pattern of `postio-app`.

### Order of work (for `/speckit-tasks`)

The branch lands once, but it is built in an order where every commit keeps
GTK and macOS green:

0. **Spikes** (each settles a Phase 0 risk with a number):
   - T0.1 `htmd` over the whole corpus, against the SC-005 assertions.
   - T0.2 Legacy-terminal chord coverage of the registry.
   - T0.3 Cold start: spawn the daemon, then keyring, then store open, in
     parallel with terminal setup, against 500 ms on the reference mailbox.
1. **Host and client, in-process only.** Move store logic into `postio-host`,
   put `postio-app` and `postio-ffi` on `postio-client` over the in-process
   transport. GTK behaviour is unchanged and `app_suite` is unchanged. This is
   the largest step and the riskiest for FR-005.
2. **Socket transport and `postio-daemon`.** GTK becomes a socket client.
   Then the two-client suite (US6).
3. **Terminal P1**: shell, list, reader, triage (US1), Markdown reading (US2),
   Markdown writing with drop and paste (US3).
4. **Terminal P2/P3**: search and palette (US4), mouse (US5), onboarding and
   settings (US7).
5. **Packaging**, the constitution amendment, `PRODUCT.md`, ADR 0041, and
   the boundary rules.

## Decisions taken with a default — the maintainer may override

These are recorded in research.md and are one-line changes if overridden:

- **The desktop flatpak's store moves** to `~/.local/share/postio` so both
  flatpaks and the standalone download share it (R10). An existing flatpak
  store resyncs.
- **The terminal text part is `format=fixed`**, not flowed (R6).

## Complexity Tracking

| Cost accepted | Why | What was rejected |
|---|---|---|
| One app at a time | Turso refuses a second opener and its multi-process mode is not ready (R1); a daemon to share it was built and withdrawn as too much complexity | A daemon owning the store with frontends as its clients (the first version of ADR 0041) |
| GTK app becomes an in-process client (~78 call sites) | One implementation of every store operation for three frontends (FR-004) | Each frontend keeping its own store code |
| Three new crates | Keeps the store engine out of frontend graphs (size, boundary check) | One `postio-client` crate that includes the host |
| Constitution Scope amendment | Maintainer-directed new frontend | Leaving the constitution saying "GTK only" while shipping a second frontend |
| A second Markdown path (`to_document` + the text part as written) | FR-021: the maintainer asked for Markdown writing, and ADR 0003's rejection was a GTK-editor decision | Sending `to_flowed_text` from the terminal, which would not be what the user wrote |
| `drafts.body_markdown` column | A terminal draft reopens exactly as typed; GTK ignores it | Re-serialising from HTML every time, which loses what the Document cannot hold |
