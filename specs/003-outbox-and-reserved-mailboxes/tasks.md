---

description: "Task list for the Outbox and reserved mailboxes"
---

# Tasks: The Outbox, and reserved mailboxes every account has

**Input**: Design documents from `/specs/003-outbox-and-reserved-mailboxes/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md), [data-model.md](./data-model.md), [contracts/](./contracts/)

**Tests**: Included and **not optional**. Constitution Principle IV is
NON-NEGOTIABLE: the failing test is written and **observed failing** before the
code that satisfies it. Where a phase lists tests as a block, that is for
reading — in practice each test is taken red before its own implementation task,
not all tests before all code.

**Organization**: One phase per user story, but **not in priority order**. The
plan's build order is 0 → US3 → US4 → US1 → US2, for mechanical reasons stated
in each phase's header. `plan.md` § Build order has the argument.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to
- Paths are relative to the repository root

## Path Conventions

Rust workspace, ~20 crates under `crates/`. Unit tests live beside the code in
`src/`; anything needing a display lives in `tests/` (a second `adw::init()` in
a unit-test binary kills the process). Integration suites run under
`cargo nextest run -p <crate>`.

---

## Phase 1: Setup

**Purpose**: The few facts the rest of the work needs pinned down.

- [X] T001 Confirm the branch is rebased on `main` after #1496 and that `crates/postio-storage/src/migrations/0017_mailbox_roles.sql` is present
- [X] T002 Claimed: `0018_role_creation_refused.sql` (T028) and `0019_message_send_state.sql` (T059); highest on `main` is `0017_mailbox_roles.sql`
- [X] T003 [P] Note in `specs/003-outbox-and-reserved-mailboxes/plan.md` the ADR number is deliberately unassigned until merge — do **not** create `docs/decisions/00NN-*.md` yet

---

## Phase 2: Foundational — the vocabulary (Build 0)

**Purpose**: The role kinds every later phase speaks. Pure logic in
`postio-model` and `postio-storage`; runs in milliseconds.

**⚠️ CRITICAL**: No user story work begins until this phase is complete.

**Contract**: [contracts/mailbox-role.md](./contracts/mailbox-role.md)

### Tests

- [X] T004 [P] Unit test in `crates/postio-model/src/mailbox.rs`: every `MailboxRole` answers `Folder` or `View`, and `RESERVED` contains exactly the six reserved roles
- [X] T005 [P] Unit test in `crates/postio-model/src/mailbox.rs`: `Outbox` and `Snoozed` are `View`; the six reserved roles, `Regular` **and `Flagged`** are `Folder` — RFC 6154 defines `\Flagged`, so a server can really have that folder
- [X] T006 [P] Test in `crates/postio-storage/tests/storage_suite/mailboxes.rs`: creating a mailbox with each `View` role is refused, and the error names the role
- [X] T007 [P] Unit test in `crates/postio-ui/src/sidebar.rs`: `role_order` places Outbox between Drafts and Sent

### Implementation

- [X] T008 Add the `Outbox` variant and `RoleKind`/`kind()`/`RESERVED` to `crates/postio-model/src/mailbox.rs`
- [X] T009 Extend `MailboxRole::as_str`/`from_name` for `outbox` in `crates/postio-model/src/mailbox.rs`, keeping the stable lowercase spelling
- [X] T010 Refuse a `View` role in `MailboxRepository::create` and `update` in `crates/postio-storage/src/repository/mailboxes.rs`, with a typed error added to `crates/postio-storage/src/error.rs`
- [X] T011 [P] Add `Outbox` to `role_order` in `crates/postio-ui/src/sidebar.rs`
- [X] T012 [P] Add the `Outbox` variant to `MailboxRoleFfi` and its conversion in `crates/postio-ffi/src/mailbox.rs`
- [X] T013 Write `scripts/checks/check-view-roles-are-not-storable.py` comparing the SQL `CHECK` list in `crates/postio-storage/src/migrations/0001_initial_schema.sql` against the `Folder` set, and register it in `scripts/check.sh`

**Checkpoint**: `cargo test -p postio-model -p postio-ui --lib` and `scripts/check.sh` green. The vocabulary exists; nothing uses it yet.

---

## Phase 3: User Story 3 — every account has every reserved role (Priority: P3, built first)

**Goal**: An account whose server lacks a reserved folder gets one, so `!`
works and every account has the same sidebar shape.

**Why first, despite being P3**: `list_row` does nothing when the account has no
Drafts mailbox (`crates/postio-storage/src/repository/drafts.rs:734`), so an
account mid-first-sync has an invisible draft *and* an empty Outbox. This phase
is the floor under US1's offline story.

**Independent Test**: Point an account at a mock listing only `INBOX`; after
discovery it has exactly one selectable mailbox per reserved role, each created
once.

**Contract**: [contracts/mail-backend.md](./contracts/mail-backend.md)

### Tests

- [X] T014 [P] [US3] Test in `crates/postio-account/src/backend/mock.rs`: `MockBackend` records `create_mailbox` calls so a suite can assert on them
- [X] T015 [P] [US3] Test in `crates/postio-sync/tests/sync_suite/discover.rs`: a server listing only `INBOX` ends discovery with one selectable mailbox per reserved role, one create per missing role
- [X] T016 [P] [US3] Test in `crates/postio-sync/tests/sync_suite/discover.rs`: a second pass over the same account issues **zero** creates (SC-007, FR-028)
- [X] T017 [P] [US3] Test in `crates/postio-sync/tests/sync_suite/discover.rs`: `Inbox` is never created, even when the server does not list it (FR-029)
- [X] T018 [P] [US3] Test in `crates/postio-sync/tests/sync_suite/discover.rs`: a refused create leaves the role unmapped and shown, records the server's reason, and the next pass makes no second attempt (FR-031)
- [X] T019 [P] [US3] Test in `crates/postio-sync/tests/sync_suite/discover.rs`: a refusal for one role does not stop the others resolving
- [X] T020 [P] [US3] Test in `crates/postio-sync/tests/sync_suite/discover.rs`: a server reporting the mailbox already exists is treated as success
- [X] T021 [P] [US3] Test in `crates/postio-sync/tests/sync_suite/discover.rs`: the created name is the role's own, identical on every provider (FR-030). **Not** "two presets, two names" as first written — the preset table holds server settings and carries no folder names, and `spec.md` § Out of Scope excludes filling it (#959's durable answer). What FR-030 forbids *today* is a provider-specific branch, and that is what is asserted
- [X] T022 [P] [US3] Test in `crates/postio-storage/tests/storage_suite/mailbox_roles.rs`: a refusal is recorded per account-and-role and cleared when the map changes or the folder appears

### Implementation

- [X] T023 [US3] Add `async fn create_mailbox(&self, path: &str) -> BackendResult<()>` to the trait in `crates/postio-account/src/backend/mod.rs`, with no default implementation
- [X] T024 [US3] Implement it for IMAP in `crates/postio-account/src/imap/mailboxes.rs` over `io-imap`'s RFC 3501 `CREATE`, subscribing where the protocol separates the two, and wire it in `crates/postio-account/src/imap/backend.rs`
- [X] T025 [P] [US3] Implement it in `crates/postio-account/src/backend/mock.rs`, recording calls and allowing a scripted refusal
- [X] T026 [P] [US3] Return `Unsupported` in `crates/postio-gmail/src/backend.rs`
- [X] T027 [P] [US3] Return `Unsupported` in `crates/postio-jmap/src/backend.rs`
- [X] T028 [US3] Migration `crates/postio-storage/src/migrations/0018_role_creation_refused.sql`: the refusal time and the server's reason in **its own table**, not a column on `mailbox_roles` — that table's `path` is `CHECK (length(path) > 0)` and a refusal has no path; registered in `crates/postio-storage/src/migrations/mod.rs`
- [X] T029 [US3] Read and write the refusal in `crates/postio-storage/src/repository/mailbox_roles.rs`, including clearing it
- [X] T030 [US3] In `crates/postio-sync/src/discover.rs`, create a reserved role's folder when every tier resolves to nothing — never for `Inbox`, never when a refusal is recorded, never twice
- [X] T031 [US3] Record a refusal rather than failing the pass in `crates/postio-sync/src/discover.rs`; the other roles still resolve and the account stays usable
- [X] T032 [US3] Show an unmapped role and the server's reason in the account's Mailboxes rows in `crates/postio-gtk/src/settings.rs`
- [X] T033 [US3] Log the account id, role and outcome only in `crates/postio-sync/src/discover.rs` — never the folder name a server rejected, never its message verbatim (Principle VI)

**Checkpoint**: `cargo nextest run -p postio-sync -p postio-account -p postio-storage`. Every account has every reserved role, offline drafts have somewhere to be listed.

---

## Phase 4: User Story 4 — the sidebar's rows are defined once (Priority: P2, built second)

**Goal**: The rows a sidebar shows are decided in `postio-ui`, not invented in a
widget — so both frontends get the same ones.

**Why before the Outbox**: building the Outbox row while the view rows still
live in `postio-gtk::feed` means writing it as a third negative-id sentinel and
then moving it. Rows first means the Outbox is defined once, the first time it
is written.

**Independent Test**: Ask the shared model for one account's rows; both
frontends render that same answer, view rows included.

**Contract**: [contracts/sidebar-rows.md](./contracts/sidebar-rows.md)

### Tests

- [X] T034 [P] [US4] Unit test in `crates/postio-ui/src/sidebar.rs`: the shared model returns reserved rows, view rows and ordinary folders, each carrying its kind
- [X] T035 [P] [US4] Unit test in `crates/postio-ui/src/sidebar.rs`: row order matches the canvas — Inbox, Flagged, Snoozed, Drafts, Outbox, Sent, Archive, Junk, Trash
- [X] T036 [P] [US4] Unit test in `crates/postio-ui/src/sidebar.rs`: the count rules move intact — Drafts a total, Flagged flagged, Snoozed snoozed, Sent/Archive/Trash/Junk nothing, Inbox and ordinary unread, zero never drawn
- [X] T037 [P] [US4] Test in `crates/postio-ffi/tests/mailboxes.rs`: the FFI returns the same rows in the same order **including view rows** — the assertion that would have failed since #1155
- [ ] T038 [P] [US4] Test in `crates/postio-gtk/tests/gtk_suite/gtk_sidebar.rs`: the rendered sidebar matches the shared model rather than re-deriving it

### Implementation

- [X] T039 [US4] Build the view rows in `crates/postio-ui/src/sidebar.rs` — no id, no path, kind `View` — and return them with the folder rows
- [X] T040 [US4] Move the count rules from `count_for` in `crates/postio-gtk/src/sidebar.rs` into `crates/postio-ui/src/sidebar.rs` — **and `display_name` with it**: the FFI sent `mailbox.name` raw, which is empty for a view row, so macOS got two rows with no label until the naming rule moved too
- [X] T041 [US4] Delete `flagged_folder`, `snoozed_folder`, `FLAGGED_ROW` and `SNOOZED_ROW` from `crates/postio-gtk/src/feed.rs`, and the `id.get() > 0` filtering they forced
- [X] T042 [US4] Resolve a scope from a view row by its role rather than by a sentinel id in `crates/postio-gtk/src/feed.rs` — **`Folders::scope_of(MailboxId)` has to take the row, not the id**: every view row is unassigned, so three of them now share id 0 and cannot be told apart by it. Reaches the sidebar's selection signal, the drop handler and the context menus
- [X] T043 [US4] Render what the shared model hands over in `crates/postio-gtk/src/sidebar.rs`, deciding nothing locally
- [X] T044 [P] [US4] Add `flagged` and `snoozed` counts to `MailboxFfi` in `crates/postio-ffi/src/mailbox.rs`
- [X] T045 [US4] Emit the shared model's rows, view rows included, from `Session::mailboxes` in `crates/postio-ffi/src/session.rs`
- [X] T046 [P] [US4] Add a `scripts/checks/` invariant that no negative `MailboxId` is constructed anywhere, and register it in `scripts/check.sh`

**Checkpoint**: `cargo nextest run -p postio-ui -p postio-gtk -p postio-ffi`. macOS gains Flagged and Snoozed; the Outbox now has one place to be defined.

---

## Phase 5: User Story 1 — a message on its way is visible (Priority: P1) 🎯 the deliverable

**Goal**: Pressing Send puts the message somewhere it can be found, and stops
putting it somewhere that implies it is unfinished.

**Independent Test**: Queue a send with the drainer paused — the message is in
the Outbox and not in Drafts; release the drainer — it leaves the Outbox for
Sent and the row disappears.

**Contract**: [contracts/list-scope.md](./contracts/list-scope.md)

### Tests

- [X] T047 [P] [US1] Test in `crates/postio-storage/tests/storage_suite/counting.rs`: listing an ordinary mailbox issues the same statements and touches the same rows **before** the Drafts exclusion exists — the baseline the riskiest task is measured against
- [X] T048 [P] [US1] Test in `crates/postio-storage/tests/storage_suite/messages.rs`: a queued draft is in `ListScope::Outbox` and not in `ListScope::Mailbox(drafts)`; FR-004 as a property over all five states
- [X] T049 [P] [US1] Test in `crates/postio-storage/tests/storage_suite/messages.rs`: `messages.send_state` equals `drafts.state` after every draft verb
- [ ] T050 [P] [US1] Test in `crates/postio-session/tests/session_suite/outbox.rs`: **with no backend at all**, a queued send is listed in the Outbox and nothing awaited a connection (US1 scenario 4)
- [ ] T051 [P] [US1] Test in `crates/postio-session/tests/session_suite/outbox.rs`: on acceptance the message leaves the Outbox and appears in Sent
- [ ] T052 [P] [US1] Test in `crates/postio-session/tests/session_suite/outbox.rs`: cancelling before the drainer returns it to Drafts as editable; cancelling in flight is refused with a reason and the row stays
- [ ] T053 [P] [US1] Test in `crates/postio-session/tests/session_suite/outbox.rs`: a scheduled send is listed with its due time, distinguishable from one waiting only on the drainer (FR-007)
- [ ] T054 [P] [US1] Unit test in `crates/postio-model/src/scope.rs`: `ListScope::Outbox(_).mailbox()` is `None`, asserted per variant so a new one fails to compile rather than being skipped (FR-010)
- [ ] T055 [P] [US1] Test in `crates/postio-storage/tests/storage_suite/messages.rs`: a paged Outbox resumes by cursor rather than refiltering
- [ ] T056 [P] [US1] Test in `crates/postio-gtk/tests/gtk_suite/gtk_sidebar.rs`: no Outbox row when empty; a row with a count when not (FR-012, FR-013)
- [ ] T057 [P] [US1] Test in `crates/postio-gtk/tests/gtk_suite/gtk_list.rs`: every Outbox row states which state it is in (FR-015)
- [ ] T058 [P] [US1] Test in `crates/postio-app/tests/app_suite/outbox_wiring.rs`: the Outbox is reachable by keyboard and its command is in the registry (FR-016)

### Implementation

- [X] T059 [US1] Migration `crates/postio-storage/src/migrations/0019_message_send_state.sql`: the nullable `send_state` column with its `CHECK`, the partial index, the count-trigger split, and a one-pass backfill from `drafts`; registered in `crates/postio-storage/src/migrations/mod.rs`
- [X] T060 [US1] Write `send_state` beside `drafts.state` in `crates/postio-storage/src/repository/drafts.rs` — `save`, `set_state`, `queue_send`, `queue_send_at`, `cancel_send` — in the same transaction, and nowhere else
- [X] T061 [US1] Add `ListScope::Outbox(AccountId)` to `crates/postio-model/src/scope.rs` with its arms in `reaction`, `is_drawn_from` and `mailbox()`
- [X] T062 [US1] Add the Outbox predicate and the Drafts exclusion to `where_clause` and `scope_arguments` in `crates/postio-storage/src/repository/messages.rs` — **the riskiest change in the feature**, measured against T047
- [ ] T063 [US1] Add `ListQuery::outbox` beside the existing constructors in `crates/postio-storage/src/repository/messages.rs`
- [X] T064 [US1] Replace `MessageListRow::draft: bool` with `send_state: Option<DraftState>` in `crates/postio-storage/src/repository/messages.rs` and update every consumer the compiler names
- [X] T065 [US1] **No new event.** `Event::MessageListChanged` already means what happened — the row leaves Drafts and joins the Outbox, which is a membership change in both — and both scopes already answer `Reload` to it, where `MessagesChanged` would make the Drafts scope `Refetch` and keep drawing a row that is no longer a member. What was missing was that the compose send path emitted *nothing*: `crates/postio-app/src/compose.rs` now announces through a callback seam. **The drainer's own transitions (Sending/Failed/Unconfirmed in `postio-sync`) still announce nothing** — see the note below
- [X] T066 [US1] Add the sidebar's per-account count query (Outbox count, Drafts total, attention count) in `crates/postio-storage/src/repository/mailboxes.rs`, and carry it through `crates/postio-runtime/src/store/sqlite.rs`
- [X] T067 [US1] Feed the Outbox row's real count into the shared model in `crates/postio-ui/src/sidebar.rs`, hidden when zero
- [X] T068 [US1] **No new command.** FR-016 as first written asked for a registry entry and a binding; no folder in the application has either, so the Outbox is reached by the sidebar walk like every other row. What that needed was a *fix*, not a command: the walk keyed on `MailboxId` and three view rows share id 0, so `j` past Flagged stuck on Snoozed for ever. `crates/postio-gtk/src/sidebar.rs` now selects the row the walk is holding, and `gtk_sidebar_keys::the_keyboard_walks_onto_the_outbox_and_opens_it` guards it
- [X] T065b [US1] The drainer's transitions reach the screen too: `crates/postio-sync/src/send.rs` marks Sending, Failed and Unconfirmed and announces nothing, because `postio-sync` has no event sink and reports through the runtime. A send that fails while the Outbox is open leaves a stale row until something else redraws. Find where sync outcomes already become `Event`s in `crates/postio-runtime/` and add this to that path — **do not** give `postio-sync` an event sink of its own (Principle VII: it talks to the backend trait and the store, not to the UI's bus)
- [X] T069 [US1] Render each row's send state — in `crates/postio-gtk/src/row.rs`, where the marks are drawn, not `list.rs`. **A scheduled send's due time is not drawn**; that half is T069b
- [X] T070 [US1] Announce the Outbox count to assistive technology in `crates/postio-gtk/src/sidebar.rs`, saying what the number means (FR-017)
- [ ] T069b [US1] A scheduled send shows its due time, not just "Waiting to send" (FR-007). The time is `operation_queue.next_attempt_at`, not on the draft or its message row, so the list has nothing to draw it from — carrying it to `Row` is a wider change than the word was
- [X] T071 [US1] Re-run T047's budget in `crates/postio-storage/tests/storage_suite/counting.rs` after T062 and assert the generic mailbox listing is unchanged in statements and rows

**Checkpoint**: The reported defect is fixed. A sent message is visible while it is on its way, offline included, and is never listed among unfinished work.

---

## Phase 6: User Story 2 — Drafts means unfinished, and says when one needs me (Priority: P2)

**Goal**: Drafts holds what you are writing and what has stopped, and the
sidebar distinguishes them.

**Independent Test**: One draft in each of editing, failed and unconfirmed —
Drafts lists all three, each row says which, and the sidebar shows a total of 3
with an attention count of 2.

### Tests

- [ ] T072 [P] [US2] Test in `crates/postio-storage/tests/storage_suite/messages.rs`: Drafts holds `editing`, `failed` and `unconfirmed` and nothing in flight (FR-020)
- [X] T073 [P] [US2] Test in `crates/postio-storage/tests/storage_suite/mailboxes.rs`: the attention count is `failed` plus `unconfirmed`, and the Drafts total excludes in-flight rows (FR-022)
- [X] T074 [P] [US2] Test in `crates/postio-gtk/tests/gtk_suite/gtk_sidebar.rs`: no attention marker when nothing needs one (FR-023)
- [ ] T075 [P] [US2] Test in `crates/postio-gtk/tests/gtk_suite/gtk_list.rs`: each Drafts row states which state it is in (FR-021)
- [X] T076 [P] [US2] Test in `crates/postio-session/tests/session_suite/outbox.rs`: retrying a failed draft moves it to the Outbox and lowers the attention count by one (FR-024)

### Implementation

- [X] T077 [US2] Surface the attention count through the shared model in `crates/postio-ui/src/sidebar.rs`, separate from the total
- [X] T078 [US2] Draw the total and the attention marker on the Drafts row in `crates/postio-gtk/src/sidebar.rs`, and announce what each number means
- [X] T079 [US2] Retry already reaches the Outbox: reopening a failed draft resumes it in the composer and sending goes through the same `queue_send` the first send did — which announces since T065. No new code; the test is what was missing
- [X] T080 [US2] Opening a failed draft already says why (`compose.rs:306`, #1487). **Nothing asserted it** — no test in the workspace mentioned the reason at all; one does now

**Checkpoint**: Drafts means what the word means, and a failed send is countable from the sidebar without opening a folder.

---

## Phase 7: Polish & Cross-Cutting

- [X] T081 ADR **0036** — *a sidebar row is a folder or a view, and a view is never a destination* — `docs/decisions/0036-a-sidebar-row-is-a-folder-or-a-view.md`. Number checked against `origin/main` (highest was 0035) on the day it was written; **re-check before landing** if `main` moves
- [X] T082 [P] Correct ADR 0021's "What the user sees" table in `docs/decisions/0021-exactly-once-send.md`: `Queued` and `Sending` are reachable from the Outbox, not the Drafts list. Do not reopen the decision
- [X] T083 [P] Describe the Outbox in the sidebar section of `docs/PRODUCT.md` §9
- [X] T084 [P] Record the reserved-role guarantee and what a refused creation means in `docs/config.md` beside `[mailboxes]`
- [ ] T085 [P] Add a `docs/notes/` entry on the mirror row (#166) as the thing that made the Outbox cheap, listed in `docs/engineering-notes.md`
- [ ] T086 Run every scenario in [quickstart.md](./quickstart.md) end to end
- [ ] T087 Check `crates/postio-gtk/data/shell.css` brace balance and run `cargo nextest run -p postio-gtk` in full — CSS is the one file here nothing type-checks, and a break in it surfaces somewhere unrelated
- [ ] T088 `cargo clippy --workspace --all-targets -- -D warnings`, `scripts/test-sanity.sh`, `scripts/check.sh`, then `scripts/issue-land.sh --detach`

---

## Dependencies & Execution Order

### Phase dependencies

- **Phase 1 (Setup)**: no dependencies
- **Phase 2 (Foundational)**: after Setup — **blocks every user story**
- **Phase 3 (US3)**: after Phase 2. Blocks US1's offline correctness
- **Phase 4 (US4)**: after Phase 2. Independent of US3; blocks US1's row definition
- **Phase 5 (US1)**: after Phases 3 and 4
- **Phase 6 (US2)**: after Phase 5 — it shares the count query and the row states
- **Phase 7 (Polish)**: after the stories it documents

### The two real cross-story dependencies

Most spec-kit stories are independent. Two here are not, and pretending
otherwise would produce a broken increment:

1. **US1 needs US3.** Without a guaranteed Drafts mailbox, `list_row` writes no
   mirror row and the Outbox is empty for an account mid-first-sync — the
   offline case US1 exists for.
2. **US1 needs US4.** Not for correctness, but for cost: building the Outbox row
   before the rows move means writing it twice, once as a sentinel.

US2 depends on US1 for the count query and row-state rendering it extends.

### Within each story

- The test is observed **red** before the code that satisfies it
- Model before storage, storage before session, session before frontend
- `postio-model` / `postio-ui` changes before the crates that consume them

### Parallel opportunities

- T004–T007 (Phase 2 tests) in parallel
- T014–T022 (US3 tests) in parallel; T025–T027 (the three non-IMAP backends) in parallel
- T034–T038 (US4 tests) in parallel
- T047–T058 (US1 tests) in parallel — the largest block
- T072–T076 (US2 tests) in parallel
- T082–T085 (documentation) in parallel

**Phases 3 and 4 can run in parallel** if two sessions are available: US3 is
`postio-account` / `postio-sync` / `postio-storage`, US4 is `postio-ui` /
`postio-gtk` / `postio-ffi`. They meet only at Phase 5.

---

## Parallel Example: User Story 1 tests

```bash
# Take the whole US1 test block red at once, then implement against it:
cargo nextest run -p postio-storage -E 'test(outbox) or test(send_state)'
cargo nextest run -p postio-session -E 'test(outbox)'
cargo nextest run -p postio-model  --lib -E 'test(scope)'
cargo nextest run -p postio-gtk    -E 'test(sidebar) or test(list)'
```

---

## Implementation Strategy

### This branch lands once

Spec-driven work is one branch and one pull request, so there is no partial
delivery to optimise for and no issue per task. `tasks.md` is the queue,
`spec.md` is the acceptance, and commits end
`Refs: specs/003-outbox-and-reserved-mailboxes` plus the task id.

### The order, and why it is not priority order

1. **Phase 2** — the vocabulary. Cheap, blocks everything
2. **Phase 3 (US3)** — the floor under US1's offline story
3. **Phase 4 (US4)** — so the Outbox row is written once
4. **Phase 5 (US1)** — the deliverable
5. **Phase 6 (US2)** — the refinement that shares US1's machinery
6. **Phase 7** — documents, and the ADR's number

If the branch has to stop early, the natural stopping point is after Phase 5:
the reported defect is fixed and US2's refinement is the only user-visible thing
missing. Stopping after Phase 4 leaves the refactor and the role guarantee
landed and the Outbox absent — real value, but not the thing that was asked for.

### The riskiest task

**T062**, the Drafts exclusion, because it edits the query every folder in the
application reads through. It is bracketed by T047 and T071, which measure the
generic mailbox listing before and after in statements and rows. Principle V
gates causes, not milliseconds, and this is the cause it is gating.

---

## Notes

- `[P]` means different files and no dependency on an incomplete task
- Work discovered here that is *not* in the spec is still filed through
  `scripts/issue-file.sh` — the exemption is for planned work, not for
  everything the branch touches
- Commit as you go; a work-in-progress commit beats loose files
- `docs/keybindings.md` and `crates/postio-core/tests/golden/linux-bindings.txt`
  are generated — regenerate them, never hand-edit
- The ADR's number is claimed at merge, not at draft (T081)
