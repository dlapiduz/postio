# Implementation Plan: The Outbox, and reserved mailboxes every account has

**Branch**: `feature/outbox-and-reserved-mailboxes` | **Date**: 2026-09-11 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/003-outbox-and-reserved-mailboxes/spec.md`

## Summary

41 requirements, and the research turned the shape of the work inside out.

The expensive-looking part is already built. #166 gives every draft a
`messages` row written **in the same transaction as the draft, offline and
always** (`postio-storage/src/repository/drafts.rs:180`, `:733`), so a draft
already has the stable `MessageId` that the list, the selection and every
`MessageTarget` command need. The Outbox is therefore what #1491 guessed it
was: a `ListScope` variant with a `WHERE` clause, the same mechanism as
`Flagged` and `Snoozed`. No second row model, no change to how a draft is
stored.

What is actually expensive is in three other places:

1. **Drafts and the Outbox are two predicates over one mailbox**, not two
   mailboxes — the mirror row sits in the Drafts folder whatever its state.
   Splitting them touches the generic mailbox scope, which is the hot read
   path, and the trigger-maintained count the Drafts badge reads. This is the
   real content of US1 and US2.
2. **`MailBackend` cannot create a mailbox** — no `create`, `rename`, `delete`
   or `subscribe` anywhere in the trait. FR-027 needs a new trait method and
   four implementations. `io-imap` already has RFC 3501 `CREATE`, so no wire
   code is written.
3. **The view rows live inside a widget.** `Flagged` and `Snoozed` are invented
   in `postio-gtk::feed` as negative-id `Mailbox` values, which is why macOS has
   never had them. Adding the Outbox the same way would ship that defect a third
   time.

**The ordering surprise, and it governs the plan:** US1's offline correctness
depends on US3. `list_row` silently does nothing when the account has no Drafts
mailbox, so an account mid-first-sync has an invisible draft *and* an empty
Outbox. FR-026 is what closes that hole. US3 is P3 by user value and a
prerequisite by mechanism, so the reserved-role guarantee is built early even
though it ships later.

## Technical Context

**Language/Version**: Rust, pinned by `rust-toolchain.toml` (1.98.0)

**Primary Dependencies**: `rusqlite` (`postio-storage`); `io-imap` `=0.6.0`
(`postio-account`) — already a dependency, and already carries the `CREATE`
this feature needs; gtk4 / libadwaita (`postio-gtk`). **No new dependency.**

**Storage**: SQLite via `postio-storage`. One migration: a denormalised send
state on `messages`, its partial index, and the count-trigger split. Builds on
the `mailbox_roles` table (migration `0017`), which merged with #1496.

**Testing**: `cargo test --lib` per crate for the pure logic (`postio-model`
role kinds, `postio-ui::sidebar` rows, scope reactions); `cargo nextest run -p
postio-storage` for the scope predicates, the counts and the `counting`
budgets; `gtk_suite` for the sidebar and the row states; `app_suite` for the
wiring; the `MailBackend` mock for folder creation. No test in the default
suite touches the network.

**Target Platform**: Linux, GTK4/libadwaita, Wayland first. Everything placed in
`postio-model`, `postio-ui` and `postio-storage` is inherited by the macOS
frontend and must stay toolkit-free.

**Project Type**: Desktop application, Rust workspace of ~20 crates

**Performance Goals**: interaction < 16 ms; the sidebar's new counts must not
scale with mailbox size; the Drafts/Outbox split must not put a correlated
subquery on the generic mailbox listing

**Constraints**: the UI never awaits the network — including the Outbox, which
must be right offline; folder creation is a network write to the user's account
and is bounded by FR-027's terms; logs carry ids, counts and outcomes only

**Scale/Scope**: the Outbox is bounded by sends in flight (typically zero);
Drafts by drafts; both windowed like any other list

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design.*

| Principle | Gate | Status |
|---|---|---|
| **I. Local-first** | Write, enqueue, emit, repaint; navigation works with the network absent | **PASS, and it repairs one** — the Outbox is correct offline *because* #166's row is local; FR-026 closes the one case where it is not |
| **II. Keyboard is a system** | Every command in `postio-core::registry` with a binding, a palette entry and an accessible action; `docs/keybindings.md` regenerated | **PASS with work** — FR-016 adds one navigation command; the binding table is regenerated, never hand-edited |
| **III. Search / one query language** | — | **N/A** — no query surface is added. The Outbox is a scope, not a saved search |
| **IV. Test-first (NON-NEGOTIABLE)** | Failing test observed before the code; assertions on what a person would see | **PASS with work** — each task names its red test; the offline and duplicate-row cases assert list *contents*, not that a layer was told |
| **V. Performance is functional** | Counts, not timings, where a read path changes | **PASS with care** — this changes the generic mailbox read path and the sidebar refresh. Both carry `counting` assertions. See Complexity Tracking |
| **VI. Privacy is a feature** | Nothing leaves the machine the user did not ask for | **PASS with a recorded exception** — FR-027 writes to the user's server. See below |
| **VII. Boundaries enforced** | `postio-model`/`postio-ui` toolkit-free; `postio-gtk` no SQL or protocol; providers are data | **PASS, and it repairs one** — the view rows *move* from `postio-gtk::feed` into `postio-ui::sidebar`; FR-030 forbids a provider-named constant for a created folder |

### The one gate that needs stating rather than ticking

**Principle VI and FR-027.** "Nothing leaves this machine that the user did not
ask for" — and creating a folder is a write to the user's account that they did
not individually request. The maintainer decided this on 2026-09-11, over a
local-only mailbox and over showing the role as unavailable. The spec bounds it;
this plan makes those bounds tests rather than intentions: creation happens only
as a consequence of the user adding or enabling an account, only for a role that
resolves to nothing, only once, **never for the Inbox**, and is visible
afterwards in the account's mailbox settings where it can be changed. A refusal
is remembered, not retried.

Recorded, not argued. If a confirmation should sit in front of it, that is a
change to FR-027 and to the one task that implements it.

### The ADR this branch writes

One, and only one: **a mailbox row is either a folder or a view, and a view is
never a destination** (FR-037, FR-040, FR-041). That rule outlives this feature
and other work must obey it — every future sidebar row, the FFI, and anything
that asks a scope for somewhere to put a message.

Everything else is recorded in `spec.md`. Per `CLAUDE.md`, a spec and an ADR are
not both needed. **The ADR takes its number when the branch merges, not when it
is drafted** — the lesson `feature/mailbox-roles` paid for twice, at 0025 and
again at 0027.

ADR 0021's "What the user sees" table (`:230-243`) is corrected in the same
branch: it says every send state is reachable from the Drafts list, which FR-001
and FR-020 make false. The decision it records is not reopened.

## Project Structure

### Documentation (this feature)

```text
specs/003-outbox-and-reserved-mailboxes/
├── plan.md              # This file
├── research.md          # Phase 0 — six questions; R1 is the one that resized the feature
├── data-model.md        # Phase 1
├── quickstart.md        # Phase 1
├── contracts/           # Phase 1
│   ├── list-scope.md        # the Outbox scope: predicates, reactions, non-destination
│   ├── mailbox-role.md      # folder vs view, and the reserved set
│   ├── sidebar-rows.md      # the shared row model both frontends consume
│   └── mail-backend.md      # create_mailbox, and what a refusal means
├── checklists/
│   └── requirements.md
└── tasks.md             # Phase 2 — NOT created by /speckit-plan
```

### Source Code (repository root)

```text
crates/
├── postio-model/src/
│   ├── mailbox.rs        # MailboxRole::Outbox, kind(), the reserved set
│   └── scope.rs          # ListScope::Outbox + reaction/is_drawn_from/mailbox
├── postio-storage/src/
│   ├── migrations/00NN_message_send_state.sql   # column, partial index, triggers
│   └── repository/
│       ├── messages.rs   # the Outbox predicate and the Drafts exclusion
│       ├── drafts.rs     # send state written beside drafts.state
│       └── mailboxes.rs  # refuse to store a View role
├── postio-ui/src/
│   └── sidebar.rs        # view rows built here, not in a widget; role_order
├── postio-core/src/
│   ├── registry.rs       # the Outbox navigation command
│   └── event.rs          # DraftStateChanged
├── postio-account/src/
│   ├── backend/mod.rs    # create_mailbox on MailBackend
│   ├── backend/mock.rs
│   └── imap/mailboxes.rs # io-imap CREATE
├── postio-sync/src/
│   └── discover.rs       # create a missing reserved role; remember a refusal
├── postio-gmail/src/backend.rs   # Unsupported
├── postio-jmap/src/backend.rs    # Unsupported
├── postio-gtk/src/
│   ├── feed.rs           # the negative-id sentinels are DELETED here
│   └── sidebar.rs        # renders what postio-ui hands it
└── postio-ffi/src/
    ├── mailbox.rs        # the flagged/snoozed counts the shared rules need
    └── session.rs        # view rows reach macOS

scripts/checks/
└── check-view-roles-are-not-storable.py   # the CHECK list and the Folder set agree
```

**Structure Decision**: no new crate. This is a vertical slice through the
existing workspace, and its one architectural move is *subtractive* — the
Flagged/Snoozed sentinels leave `postio-gtk::feed` for `postio-ui::sidebar`,
where the shared sidebar logic already lives (`role_order`, `primary_within`,
`sections`, moved there by #1155 for exactly this reason).

## Build order

Four builds. Each is independently landable; each starts red.

**Build 0 — the vocabulary (blocks everything).** `MailboxRole::Outbox`,
`kind()`, the reserved set, the storage refusal for a `View` role, and the
`scripts/checks/` invariant that keeps the SQL `CHECK` list and the `Folder` set
in step. Pure `postio-model` and `postio-storage`; unit-testable in
milliseconds.

**Build 1 — reserved roles always exist (US3).** `create_mailbox` on
`MailBackend` and its four implementations, discovery creating what is missing,
the refusal remembered in `mailbox_roles`. Lands before the Outbox because
`list_row`'s early return is the hole underneath US1's offline story.

**Build 2 — the Outbox and the Drafts split (US1, US2).** The migration, the
denormalised send state, the two scope predicates, the counts, the row states,
the navigation command, `DraftStateChanged`. The largest build, and the one the
user actually asked for.

**Build 3 — one definition of the rows (US4).** Move the view rows into
`postio-ui::sidebar`, delete the sentinels from `postio-gtk::feed`, carry the
counts across the FFI. Independently valuable on its own: it gives macOS the
Flagged and Snoozed rows it has never had.

Build 3 could precede Build 2, and doing so would stop the Outbox row being
written twice. It is placed second-to-last anyway because Build 2 is the
user-visible deliverable and Build 3 is a refactor. The cost is one file written
twice, and it is accepted deliberately rather than discovered later.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|---|---|---|
| A denormalised send-state column on `messages`, duplicating `drafts.state` | Drafts and the Outbox are two predicates over one mailbox, and both must be plain indexed predicates — one of them is the generic mailbox scope every folder reads through | Computing the split above SQL breaks cursor paging: `ListQuery`'s cursor is a row value over `(received_at, id)` so SQLite can seek, and a page of 50 that then drops rows is not a page of 50 |
| The sidebar gains a query it did not have, for two counts | The Outbox is not a mailbox and has no row to hold a trigger-maintained column; the Drafts badge must stop counting in-flight rows (FR-022) | A fifth count column on `mailboxes` would need triggers on `drafts` maintaining a column on `mailboxes`, coupling two otherwise independent tables |
| A new `MailBackend` method, implemented four times | FR-027, and the trait has no folder creation at all; it is the seam every backend crosses | Calling `io-imap` from `postio-sync` violates Principle VII: `postio-sync` talks to the trait, never to `io-imap` types |

**What the three share**: each is a cost paid to keep a *read* path cheap or a
*boundary* intact, and each is gated by a test rather than a promise — the first
two by `postio_storage::test_support::counting` assertions on statements and
rows, the third by the crate-boundary check already in `scripts/check.sh`.

## Dependencies and risks

- **`feature/mailbox-roles` merged** in [#1496](https://github.com/dlapiduz/postio/pull/1496)
  on 2026-09-12, closing epic #962. FR-033 and FR-035 are its work, and
  FR-031's refusal record has a home because of it. This branch is rebased onto
  it.
- **The riskiest single task is the Drafts exclusion**, because it edits the
  query every folder in the application reads through. It is sequenced first
  within Build 2 and carries `counting` assertions before and after.
- **`MailboxFfi` carries no flagged/snoozed counts** and there is no FFI
  equivalent of `count_for`, so Build 3 is two fields and a shared count rule
  wider than "move the row list".
- **The macOS frontend is built and tested in CI** (`macOS build and Swift
  tests`, ~13 min), so Build 3's FFI change is gated there and cannot be proven
  on Linux alone.
- **#1491 stays open until this branch lands**, and is not an issue to claim
  alongside it — the spec is the queue.
