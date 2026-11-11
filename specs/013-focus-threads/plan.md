# Implementation Plan: Threads in Postio Focus, Mac and Linux

**Branch**: `feature/focus-threads` | **Date**: 2026-11-11 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/013-focus-threads/spec.md`

## Summary

Threads become visible again (#2099) without giving up one message at a
time, on both Focus apps at once (T1).

1. **The model first.** Grouping learns a 7-day subject window, leaves list
   mail alone, and stops merging same-subject threads (R2). Mute and split
   are new, keyed by Message-ID so they survive a rethread (R3). Muted
   replies are archived in Focus's filing stage (R4). The body stage stores
   each message's own line (R5).
2. **One read for a thread.** `Req::Thread` returns every member with its own
   line, the message it answers, and a trailing draft, in a constant number
   of statements (R6).
3. **Rules in `postio-ui`, behaviour in `postio-focus`.** The scope table,
   the list splice, chip fitting and every word are pure functions. The
   controller holds one expansion per list and a walk per open window.
4. **Both frontends draw it.** AppKit through `postio-ffi`, GTK directly.
   Only the chrome differs.

Built in the design's seven steps. Each step is runnable on both platforms
and committed on its own, with its storyboards red first.

## Technical Context

**Language/Version**: Rust 1.99.0 (`rust-toolchain.toml`; on this Mac the
SessionStart hook unsets `RUSTUP_TOOLCHAIN`); Swift tools 6.0; GTK 4 /
libadwaita (Linux only).

**Primary Dependencies**: `postio-model`, `postio-storage`, `postio-sync`
(arrival order only), `postio-host`, `postio-client`, `postio-session`,
`postio-body`, `postio-search` (unchanged), `postio-index` (unchanged
requests reused), `postio-core`, `postio-ui`, `postio-focus`, `postio-ffi`
(UniFFI 0.32), `postio-gtk`, `postio-demo`. No new crates, and no new
third-party dependencies.

**Storage**: one migration in `postio-storage/src/schema.rs`. It adds
`thread_mutes`, `thread_splits`, `messages.own_line` and
`idx_threads_subject_recent`, with a new fingerprint in `tests/schemas/`.
There is no rethread migration (R2).

**Testing**:
- **Rust**:
  - `scripts/test-fast.sh` between edits.
  - `cargo nextest run -p postio-storage --test storage_suite`,
    `-p postio-focus --test controller_suite` and
    `-p postio-ffi --test ffi_suite`.
  - `postio-gtk --test focus_suite` on CI and the Linux box.
  - Counts via `postio_storage::test_support::counting`.
- **Swift**: `scripts/macos-test.sh`.
- **Storyboards**: `storyboards/threads/` through `scripts/storyboards.sh`,
  then `/ux-review`.
- **Screens**: `scripts/macos-shot.sh` and `scripts/run-isolated.sh --shot`.

**Target Platform**: macOS 14+ and Linux (GTK Focus). The terminal app and
the classic GTK reader keep building and take the model changes (D4).

**Project Type**: desktop app (Rust engine, Swift and GTK frontends).

**Performance Goals**:
- Expand, collapse and step under 16 ms.
- `Thread` constant in statements at any length.
- A list page with mutes costs today's statements + 1.
- Inbox startup still under 500 ms (SC-002, SC-003).

**Constraints**:
- This Mac cannot build `postio-gtk`: GTK is proven on CI and the Linux box.
- The Linux box takes at most two agents, and no back-to-back full-suite
  loops.
- `PostioKit` imports no AppKit (#1264).
- The controller does no I/O; Swift and GTK decide nothing.
- Nothing is fetched to draw a row, chip or outline, or to forward.
- No worktree path in anything rustc sees.

**Scale/Scope**:
- 11 screens and 7 user stories.
- About 14 new registry commands; 1 removed (`archive_thread`).
- 5 new client requests and 1 retired (`Conversation`).
- About 6 new `postio-ui` modules.
- About 15 Swift files and 8 GTK files new or changed.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design.*

| Principle | How this plan meets it | Status |
|---|---|---|
| I. Local-first | Mute, unmute, split, expand and every walk are store reads/writes then emit. A muted arrival is archived locally and synced like any archive. Whole-thread forward refuses members that are not local rather than fetching them (R12) | Pass |
| II. The keyboard is a system | Every key is a `CommandId` with contexts, a palette entry and a registry clash test. The designer's keys are translated onto the one keymap (D1, T2, T3), not added beside it. Keycaps in footers and banners come from the live keymap | Pass |
| III. One query language | Search reuses spec 010's request and `conversation_matches`. Nothing new is parsed | Pass |
| IV. Test-first | Each task starts from a red test: rule unit tests, controller suite, storyboards red on `main`, then frontend cases per path (ADR 0049) | Pass |
| V. Performance, gated as counts | `Thread`, list pages, the subject lookup and the filing stage's mute check are each gated as statement counts. Child rows and the outline are windowed (R6, R7) | Pass |
| VI. Privacy | No new network path. Logs carry thread and message ids and counts only (FR-029). Mute and split never leave the machine. The demo uses reserved domains | Pass |
| VII. Boundaries | SQL stays in `postio-storage`. Bodies stay in `postio-body`/`postio-session`. Words go in `postio-ui`, behaviour in `postio-focus`, and FFI is the only Swift boundary. No new crate edges beyond `postio-focus → postio-ui` (existing) | Pass |

Re-check after Phase 1: no violations. The contracts keep scope in
`postio-ui::scope`, the splice in `postio-ui::thread_list`, and arrival
order in the host's filing stage.

## No ADR

D1–D4 and T1–T3 are this feature's and live in spec.md. The one rule other
work must obey is that **scope is resolved by `postio_ui::scope::target`
and nowhere else**. It is recorded in the module's doc and in
`docs/ARCHITECTURE.md`'s `postio-ui` entry. That is a boundary note, not a
new ADR. ADR 0032's stacked conversation stays superseded for Focus by
007 C2.

## Design by crate

### `postio-model` (pure)

- `threading.rs`: `ThreadCue.date` and `is_list`; `ThreadIndex`'s bounded
  `threads_with_subject` and `is_split`; `assign`'s split rule and its
  join-never-merge subject fallback; `descendants`; `SUBJECT_WINDOW`.
- Tests: the rule table in data-model, plus each R2 failure (two "Re:
  Hello", a weekly digest, an 8-day gap).

### `postio-storage`

- `schema.rs`: one migration (data-model) and its fingerprint.
- `repository/threading.rs`: `LoadedIndex` loads split flags and bounds the
  subject lookup.
- `repository/threads.rs`: `members`, `muted`, `mute`/`unmute`, `split`.
- `repository/messages.rs`: `set_own_line`; the list query reads `own_line`
  only for members, never for rows.
- Threading corpus: new `.eml` fixtures via `/add-fixture` for forks,
  cross-folder, subject reuse, lists and digests.

### `postio-sync` and the host's filing stage

- Arrival order per the engine contract. The mute check runs first in the
  filing stage (`postio-host/src/focus/…`).
- The body stage writes `own_line` beside the markers.

### `postio-body`

- `own_line`, `forward_thread` and `forward_as_attachments` (R12).

### `postio-client`, `postio-host`, `postio-session`

- `Thread`, `Mute`/`Unmute`, `Split`, `MutedOf`, `ForwardThread`;
  `ThreadChanged` event.
- The composer's `ReplyToLatest` and `SetForwardForm`. `Conversation` is
  retired after both frontends move.

### `postio-core`

- Registry: the commands of R10. `Right`/`Left` are removed from Open and
  Back in list contexts. `ArchiveThread` is deleted.
- `docs/keybindings.md` is regenerated, and the in-app key map follows from
  the registry.

### `postio-ui` (words and pure rules)

- New modules: `scope.rs`, `thread_list.rs`, `thread_bar.rs`,
  `thread_outline.rs`.
- Changed:
  - `focus_row.rs` gains participants, latest line, new badge, child tag
    and `open_at`.
  - `search_view.rs` gains result children and gaps.
  - `focus_target.rs` resolves through `scope::target`.
  - `focus_dialog.rs` drops its own position line and thread chip in favour
    of `thread_bar`.

### `postio-focus`

- `Expansion` per list surface. `Walk` in the reader route.
- `ReaderVerb` additions. `Outline` and `ThreadMenu` surfaces.
- The forward chooser in `compose.rs`.
- `perform` gains `ReadThread`.
- A new controller-suite file, `threads.rs`.

### `postio-ffi`

- `focus_threads.rs` (new) holds the views of the frontend contract.
- `focus_list.rs` gains splice events and the thread row's new fields.
- `focus_message.rs` reads `Thread`, not `Conversation`, and emits the bar.

### Swift (`macos/Sources`)

- **PostioKit**:
  - New: `ThreadBar.swift`, `ThreadOutline.swift`, `ThreadMenu.swift`,
    `ChildRow.swift`, `ForwardChooser.swift`.
  - Changed: `FocusList.swift`, `MessageWindowView.swift`, `ComposeModel.swift`.
- **PostioAppKit**: `FocusListTable.swift` (row insertion, chevron),
  `MessageWindowChrome.swift` (bar below the action row), `ResultsTable.swift`
  (children and gaps).
- **Tests**: expansion focus, window keyboard paths, chooser keys.

### GTK (`crates/postio-gtk`)

- `window.rs`/`list/` (splice, chevron, child row widget), `open.rs` and
  `open_header.rs` (bar, kicker, banners, Reply to), `results.rs` (children,
  gaps), a new `thread_popovers.rs` (menu, outline), `compose/` (chooser,
  older-reply bar).
- `focus_suite/threads.rs`: one case per path a person takes.
- `/gtk-design` before styling.

### `postio-demo`

- `Seed::Threads` (R14) and a live-reply key for screen 08.

## Build order (the design's steps; each runnable on both platforms)

| Step | Delivers | Screens | Stories |
|---|---|---|---|
| 1 | Model: grouping rules, mute, split, `own_line`, `Thread` request, demo seed | — | groundwork for all |
| 2 | Collapsed thread rows | 01 | US1 (rows) |
| 3 | Expand in place, child rows, scope footer | 02, 11 | US1, US2 |
| 4 | Window: bar, `[` `]` `{` `}` `g u`, banners, Reply to; live arrival | 03, 08 | US3, US4 |
| 5 | Thread menu, outline, mute, split, move or delete one | 04, 05 | US5 |
| 6 | Search grouped by thread, open at match, `mod+g` across messages | 06, 07 | US6 |
| 7 | Compose: older reply, `alt+e`, the forward chooser | 09, 10 | US7 |

Step 3 closes #2099's reproduction on both platforms. If the maintainer
wants the issue closed sooner, steps 1–3 can land as their own PR with
`--full-suite`, and the branch continues.

## Performance plan

1. **Step 1.** Count gates for `Thread` (2 vs 500 members), the bounded
   subject lookup, and the filing stage's mute check.
2. **Step 3.** Expand and collapse are measured on the Mac (`ReaderTiming`
   style signposts) and in the GTK suite's frame clock. Each must be
   < 16 ms on the long-thread seed.
3. **Step 4.** Stepping the window is measured the same way; the bar's
   `chips` is a pure function, benchmarked at 500 members.
4. **Stop rule.** If a budget fails, the step stops and the maintainer is
   asked, with the measurements and the option of capping the read at
   `THREAD_READ_CAP` members with "… N earlier" in the bar.

## Test plan

| Layer | What | Where |
|---|---|---|
| Rule | `assign` table, subject window, list mail, split join, `descendants` | `postio-model` unit |
| Corpus | Real header shapes group as expected (SC-005) | `storage_suite/threading.rs` + `.eml` fixtures |
| Storage | `members`, mute through merges and renumbering, split relinks, budgets | `storage_suite` |
| Words | participants, latest line, badges, chips that fit, kicker, banners, Reply to, gaps | `postio-ui` unit |
| Scope | Each cell of D2 | `postio-ui::scope` unit |
| Controller | Expand/collapse, one-at-a-time, cursor through children, anchor leaving, new member while expanded, walk keys, no move on arrival, outline, matches across messages, chooser | `controller_suite/threads.rs` |
| Body | Whole-thread forward, attachments once, `.eml` set, not-local refusal | `postio-body` unit |
| FFI | Views over `Seed::Threads` | `ffi_suite/focus_threads.rs` |
| Swift | Expansion focus, window keyboard, chooser | `macos/Tests/…` |
| GTK | One case per path, real keys | `focus_suite/threads.rs` |
| Storyboards | Each keyboard path, red on `main` | `storyboards/threads/` |
| Screens | 01–11, 02 dark, both platforms | quickstart |

## Project Structure

```text
specs/013-focus-threads/
├── spec.md, plan.md, research.md, data-model.md, quickstart.md
├── checklists/requirements.md
└── contracts/
    ├── engine-threads.md      # requests, arrival order, budgets
    └── frontend-threads.md    # inputs, intents, views, chrome differences

crates/
├── postio-model/src/threading.rs
├── postio-storage/src/{schema.rs, repository/{threading,threads,messages}.rs}, tests/{schemas,storage_suite}/…
├── postio-model/tests/corpus/…                 # new threading fixtures
├── postio-body/src/{own_text.rs, forward.rs}
├── postio-host/src/focus/…                     # filing-stage mute, body-stage own_line
├── postio-client/src/{protocol,api}.rs
├── postio-session/src/…                        # forward forms
├── postio-core/src/registry.rs, command.rs
├── postio-ui/src/{scope,thread_list,thread_bar,thread_outline}.rs + focus_row, search_view, focus_target, focus_dialog
├── postio-focus/src/{lib,surfaces,cursor,verbs,compose,results,perform}.rs, tests/controller_suite/threads.rs
├── postio-ffi/src/{focus_threads.rs, focus_list.rs, focus_message.rs}, tests/ffi_suite/…
├── postio-gtk/src/{window,open,open_header,results,thread_popovers}.rs, compose/, list/, tests/focus_suite/threads.rs
└── postio-demo/src/lib.rs                      # + Seed::Threads

macos/Sources/{PostioKit,PostioAppKit}/…, macos/Tests/…
storyboards/threads/…
docs/keybindings.md (generated), docs/ARCHITECTURE.md (scope note)
```

## Complexity Tracking

| Violation | Why needed | Simpler alternative rejected because |
|---|---|---|
| Two new tables keyed by Message-ID rather than thread id | Thread ids are renumbered by merges and resyncs (the schema says so for reminders) | A column on `threads`: mute and split would silently vanish on the next rethread |
| A stored `own_line` column | Child rows must show own text without parsing HTML per row on the main thread (16 ms) or fetching (privacy) | `preview`: it includes the quote, which is exactly what the design forbids |
| Changing a long-standing grouping rule without a rethread migration | A rethread scans the mailbox; new mail follows the rule and a resync applies it to old mail | A migration that rethreads every account: unbounded work at upgrade, for a few over-merged subjects |
