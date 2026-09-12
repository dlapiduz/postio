---

description: "Task list for the Search and Command Bar"
---

# Tasks: Search and Command Bar

**Input**: Design documents from `specs/003-search-command-bar/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md), [data-model.md](./data-model.md), [contracts/](./contracts/), [quickstart.md](./quickstart.md)

**Tests**: Required, not optional. Constitution IV is NON-NEGOTIABLE — the failing test is written and **observed failing** before the code that satisfies it. Every task below that implements behaviour has a test task above it that must be red first.

**Organization**: By user story. Three of the spec's five stories already ship and are not rebuilt; US4 and US5 are the work.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: US4 or US5, matching [spec.md](./spec.md)

## Path Conventions

A Cargo workspace. Crate sources are `crates/<crate>/src/`, unit tests live beside them, and integration suites are `crates/<crate>/tests/`. Paths below are repository-relative.

---

## Phase 1: Setup

**Purpose**: Establish that the built parts really are built, because everything in this plan rests on that claim.

- [X] T001 Run the built-state baseline and record every count in `specs/003-search-command-bar/quickstart.md`, replacing each `[n]` placeholder: `cargo test -p postio-core --lib`, `cargo nextest run -p postio-core --test core_suite`, `cargo nextest run -p postio-gtk --test gtk_suite gtk_finder`, `cargo test -p postio-app --test app_suite`
- [X] T002 Confirm `scripts/check.sh` is clean before anything moves between crates, so a later crate-boundary failure is known to be this feature's doing

---

## Phase 2: Foundational (Blocking Prerequisites)

**No foundational tasks.** Both stories extend structures that already exist — the command registry for US5, the bar and its generated documentation for US4 — and neither needs the other to start. This phase is empty on purpose rather than by omission.

One shared file needs care and is not a task: `docs/keybindings.md` is generated, and both stories change what it contains. See Dependencies below.

**Checkpoint**: After Phase 1, both user stories may begin, in either order or in parallel.

---

## Phase 3: User Story 4 — Find out what the bar can do (Priority: P1) 🎯 MVP

**Goal**: A person who did not write Postio can discover that the box goes to folders, runs commands, labels a selection and finds a correspondent.

**Independent Test**: Hand the bar to somebody who has not read the source and ask them to go to Drafts without a pointer. They should get there on what the bar and the documentation tell them.

### Tests for User Story 4 ⚠️ red first

- [X] T003 [P] [US4] Unit tests for the mode table's invariants in `crates/postio-ui/src/finder_modes.rs` — prefixes unique, exactly one mode without a prefix, every mode carrying a non-empty name and purpose
- [X] T004 [P] [US4] A `gtk_suite` case in `crates/postio-gtk/tests/gtk_suite/gtk_finder.rs` asserting the bar at rest indicates it does more than search mail, and that an open empty box lists every mode with its prefix and purpose (FR-028, FR-029)
- [X] T005 [P] [US4] A `gtk_suite` case in `crates/postio-gtk/tests/gtk_suite/gtk_finder.rs` asserting an active mode says which mode it is and how to leave it (FR-030)
- [ ] T006 [P] [US4] A `gtk_suite` case in `crates/postio-gtk/tests/gtk_suite/gtk_cheatsheet.rs` asserting the `?` cheat sheet lists the bar's modes
- [X] T007 [P] [US4] A `ui_suite` case in `crates/postio-ui/tests/ui_suite/keybindings_doc.rs` asserting `docs/keybindings.md` carries a modes section generated from the table, and fails when it drifts (FR-031, FR-032)
- [ ] T008 [P] [US4] A `gtk_accessibility` case in `crates/postio-gtk/tests/gtk_accessibility.rs` asserting everything the bar says about its modes is reachable as text (FR-034)

### Implementation for User Story 4

- [X] T009 [US4] Create the mode table in `crates/postio-ui/src/finder.rs` — prefix, name, purpose per [contracts/mode-table.md](./contracts/mode-table.md) — and export it from `crates/postio-ui/src/lib.rs`
- [X] T010 [US4] Re-export the table from `crates/postio-gtk/src/finder.rs` and delete the prefix and description strings it duplicates, so the table is the only place the set is written down
- [X] T011 [US4] Render the hint in `crates/postio-gtk/src/finder.rs` — visible at rest and in an open empty box, out of the way once a query is being typed (FR-035)
- [X] T012 [US4] Show the active mode and how to leave it in `crates/postio-gtk/src/finder.rs`
- [ ] T013 [US4] Suppress a mode that cannot act in the current context, in `crates/postio-gtk/src/finder.rs` (FR-033)
- [X] T014 [US4] Add the modes section to the generator in `crates/postio-ui/tests/ui_suite/keybindings_doc.rs` and regenerate `docs/keybindings.md`
- [ ] T015 [US4] Give the hint its accessible text in `crates/postio-gtk/src/finder.rs` (FR-034)

**Checkpoint**: The bar now says what it can do, and the documentation says the same thing from the same table. US4 is independently shippable without US5.

---

## Phase 4: User Story 5 — Go straight to the places you go most (Priority: P2)

**Goal**: `g i` reaches the inbox, and the destinations that have a sequence are ordinary registry commands — so they arrive in the palette, the cheat sheet and the documentation on the way.

**Independent Test**: With mail in more than one folder, press `g i` from the message list and confirm the inbox is showing. Repeat for `g d`, `g t`, `g s`.

### Tests for User Story 5 ⚠️ red first

- [ ] T016 [P] [US5] A `core_suite` case in `crates/postio-core/tests/core_suite/command_registry.rs` asserting the four destination ids exist with their titles, bindings and list-surface contexts per [contracts/command-ids.md](./contracts/command-ids.md)
- [ ] T017 [P] [US5] A unit test in `crates/postio-gtk/src/feed.rs` (or its module's tests) for role resolution: a role that exists returns its mailbox, a role that does not returns nothing
- [ ] T018 [US5] **The acceptance test**: a new `app_suite` case in `crates/postio-app/tests/app_suite/` — a module plus its row in `CASES` in `crates/postio-app/tests/app_suite/main.rs` — that presses `g i` at the composition root and asserts on the folder then showing. Not that a handler fired: what a person would be looking at ([research.md](./research.md) R8)
- [ ] T019 [P] [US5] An `app_suite` or `gtk_suite` case asserting `g` typed in the composer enters a letter and navigates nowhere (FR-042)
- [ ] T020 [P] [US5] A case asserting a destination absent from the current account is reported to the user rather than silently doing nothing (FR-041)

### Implementation for User Story 5

- [ ] T021 [US5] Add `GoToInbox`, `GoToDrafts`, `GoToSent` and `GoToFlagged` to `command_ids!` in `crates/postio-core/src/command.rs`, with the `[keys]` spellings from [contracts/command-ids.md](./contracts/command-ids.md)
- [ ] T022 [US5] Add the four `CommandSpec` entries in `crates/postio-core/src/registry.rs` — titles, `g i`/`g d`/`g t`/`g s`, list-surface contexts, `destructive: false`, `Recovery::None`, `requires: None`
- [ ] T023 [US5] Add the `Command` variants and their dispatch in `crates/postio-core/src/command.rs` so each id carries its role
- [ ] T024 [US5] Resolve a role to a mailbox from the list the feed already holds, in `crates/postio-gtk/src/feed.rs`, honouring the sidebar's current scope ([research.md](./research.md) R7). No query, no network
- [ ] T025 [US5] Handle the four commands in `crates/postio-gtk/src/window.rs`'s `act`, reaching the folder through the existing `sidebar().select(id)` and `show(id)` rather than a second path
- [ ] T026 [US5] Report an absent destination in `crates/postio-gtk/src/window.rs` (FR-041)
- [ ] T027 [US5] Regenerate `docs/keybindings.md` so the four destinations appear, and confirm `keybindings_doc.rs` passes

**Checkpoint**: `g i` works, and the destinations are in the palette and the cheat sheet without anybody having added them there.

---

## Phase 5: Polish & Cross-Cutting Concerns

- [ ] T028 [P] Confirm `crates/postio-ui` still carries no toolkit dependency by running `python3 scripts/checks/check-crate-boundaries.py`
- [ ] T029 [P] Run `cargo clippy -p postio-core -p postio-ui -p postio-gtk -p postio-app --all-targets -- -D warnings`
- [ ] T030 Run `scripts/check.sh` and confirm every repository invariant is clean
- [ ] T031 Walk [quickstart.md](./quickstart.md) sections 1–5 end to end, and replace any remaining `[n]` with the count the command actually selected — a section is not walked while its own count is a placeholder
- [ ] T032 Walk [quickstart.md](./quickstart.md) section 6 by eye, which is the only step that can judge whether a person can find the thing. Needs a display and a person
- [ ] T033 Land: `scripts/issue-land.sh --detach` from the feature worktree — one pull request reviewed against [spec.md](./spec.md), closing no issue, with `Refs: specs/003-search-command-bar`

---

## Dependencies & Execution Order

### Phase dependencies

- **Phase 1 (Setup)**: no dependencies; do it first so the baseline is known.
- **Phase 2 (Foundational)**: empty. Nothing blocks the stories.
- **Phase 3 (US4)** and **Phase 4 (US5)**: both depend only on Phase 1, and are independent of each other.
- **Phase 5 (Polish)**: depends on whichever stories were taken.

### The one shared file

`docs/keybindings.md` is generated, and both stories change it — US5 adds four rows from the registry, US4 adds a modes section from the mode table. **T014 and T027 both regenerate it.** Done in either order they are fine; done simultaneously in two worktrees they conflict. Since both stories land on this one feature branch, do them in sequence and regenerate once after each.

### Within each story

- Every test task precedes its implementation task and **must be observed failing first**. A test never seen red is tightened until it visibly constrains the behaviour — the bug is never re-injected to prove it (Constitution IV).
- T009 before T010–T013 and T015: the table exists before anything reads it.
- T021 before T022 before T023: an id exists before it has a spec before it is dispatched.
- T024 before T025: resolution exists before the handler calls it.

### Parallel opportunities

- T003–T008 are six different files and may be written together.
- T016, T017, T019 and T020 may be written together; T018 is sequenced after T017 because it needs resolution to exist to be more than a compile error.
- T028 and T029 may run together.
- With two people, US4 and US5 run in parallel end to end, meeting only at `docs/keybindings.md`.

---

## Parallel Example: User Story 4

```bash
# The six red tests, written together, then run together to watch them fail:
cargo test -p postio-ui --lib finder_modes
cargo nextest run -p postio-gtk --test gtk_suite gtk_finder
cargo nextest run -p postio-gtk --test gtk_suite gtk_cheatsheet
cargo nextest run -p postio-core --test core_suite keybindings_doc
cargo nextest run -p postio-gtk --test gtk_accessibility
```

---

## Implementation Strategy

### MVP: User Story 4 alone

US4 is the MVP and it is not the one that was asked for, which is the point. It
makes four shipped capabilities findable, including the folder jump that
prompted this feature. Shipping it alone would have answered the original
request — `#` was always there.

1. Phase 1 → Phase 3 → stop and validate against quickstart section 6 item 1.
2. Ship. The bar now explains itself.

### Then User Story 5

`g i` is what the maintainer asked for and is a smaller, sharper win on top. It
also demonstrates the mechanism US4 relies on: a destination expressed as a
registry command needs no separate discoverability work at all.

### Notes

- Every task is a commit on `feature/search-command-bar`; the branch lands once, as one pull request reviewed against the spec (Constitution, Development Workflow).
- Commit messages end `Refs: specs/003-search-command-bar` and the task id, not `Refs: #<issue>` — this work has no issues by design.
- Archive's sequence is deliberately not a task here. It has no good letter, and [research.md](./research.md) R4 leaves it with the design authority. Work discovered on the way that is not in the spec is still filed through `scripts/issue-file.sh`.
