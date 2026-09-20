# Quickstart: validating the Outbox and reserved mailboxes

**Feature**: `specs/003-outbox-and-reserved-mailboxes` | **Date**: 2026-09-11

How to prove this feature works, cheapest layer first. Every scenario below is
runnable without a network; the live-server checks at the end are `#[ignore]`
and are not part of any gate.

## Prerequisites

```bash
cd ~/src/postio-worktrees/outbox-and-reserved-mailboxes
scripts/install-shims.sh          # the claim/land/test scripts do this too
```

The branch is cut from `main` after [#1496](https://github.com/dlapiduz/postio/pull/1496),
so `mailbox_roles` (migration `0017`) and ADR 0035 are present. Confirm:

```bash
ls crates/postio-storage/src/migrations/ | grep mailbox_roles
```

## The loop while building

```bash
scripts/test-fast.sh                     # between edits: changed crates, --lib
cargo nextest run -p postio-storage      # the scope predicates and the counts
cargo nextest run -p postio-ui           # the shared sidebar rows: microseconds
scripts/test-sanity.sh                   # before landing
```

The filters below name tests that exist. They were rewritten after the build:
the first draft of this guide predicted names and crates before anything was
written, and three of its five scenarios selected nothing — nextest says
`error: no tests to run`, which is a failure that looks like a typo rather
than like a gap. A validation guide whose commands run nothing is worse than
no guide.

Iterate at the cheapest layer that can fail. Most of this feature's logic is
pure — role kinds in `postio-model`, row assembly in `postio-ui`, scope
reactions — and those run in milliseconds. Keep the GTK and app suites for
confirming the wiring at the end of a build.

## Scenario 1 — a sent message is visible while it is on its way (US1)

The headline. Drive it through the store and the session, with the drainer
paused so the in-flight window is observable.

```bash
cargo nextest run -p postio-storage -E 'test(/send|outbox|draft_state/)'
cargo nextest run -p postio-app --test app_suite resume_queued_draft
```

**Expected**

1. Before sending: the Outbox scope is empty, and `postio-ui::sidebar` returns
   no Outbox row.
2. After queueing: the message is in `ListScope::Outbox`, **not** in
   `ListScope::Mailbox(drafts)`, and the sidebar has an Outbox row with a count
   of 1.
3. After the drainer accepts it: the message is in neither Drafts nor the
   Outbox, it is in Sent, and the Outbox row is gone.
4. Cancelling before the drainer runs returns it to Drafts as editable.
5. Cancelling once in flight is refused, with a reason, and the row stays.

## Scenario 2 — it is right with no network at all (US1, and the point of the feature)

The one that would have failed if the Outbox had been built over `drafts`
instead of over the mirror row.

```bash
cargo nextest run -p postio-storage -E 'test(/does_not_need_a_drafts_mailbox|enqueues_the_operation|never_saved/)'
```

**Expected**: with no backend configured, queue a send. The message is listed in
the Outbox immediately, its state is shown, and nothing awaited a connection.
Navigate away and back: still there. This is Principle I stated as a test.

## Scenario 3 — Drafts means unfinished, and says when one needs you (US2)

```bash
cargo nextest run -p postio-storage -E 'test(/needs_a_person|counts_nothing|retrying_a_failed|keeps_the_reason/)'
cargo nextest run -p postio-gtk -E 'test(/sidebar|gtk_row/)'
```

**Expected**: with one `editing`, one `failed` and one `unconfirmed` draft,
Drafts lists all three and each row states which it is; the sidebar's Drafts row
shows a total of 3 and an attention count of 2. Retry the failed one: it moves
to the Outbox and the attention count falls to 1. With nothing needing
attention, no attention marker is drawn.

## Scenario 4 — every account has every reserved role (US3)

Against the mock backend; no server is contacted.

```bash
cargo nextest run -p postio-sync -E 'test(/reserved_role|created|refus|never_created/)'
```

**Expected**

1. A mock listing only `INBOX` ends discovery with one selectable mailbox per
   reserved role, one `create_mailbox` call per missing role, and **none for the
   Inbox**.
2. A second pass creates nothing (SC-007).
3. A mock that refuses leaves the role unmapped **and shown**, records the
   server's reason, and makes no second attempt on the next pass.
4. A refusal for one role does not stop the others resolving.

## Scenario 5 — the rows are defined once (US4)

```bash
cargo nextest run -p postio-ui
cargo nextest run -p postio-ffi -E 'binary(mailboxes)'   # binary(), not test(): it is the suite's name
```

**Expected**: the same account yields the same rows in the same order from both
the GTK path and the FFI — **view rows included**. This assertion would have
failed for Flagged and Snoozed every day since #1155, which is the point of
writing it.

## Performance, gated as counts

Principle V gates causes, not milliseconds — a shared runner cannot defend
16 ms, but statements and rows are the same number on every machine.

```bash
cargo nextest run -p postio-storage -E 'test(/costs_the_same|costs_a_fixed/)'
```

**Expected**

- Listing an ordinary mailbox issues the same statements and touches the same
  rows as before the Drafts exclusion was added. This is the assertion that
  protects the hot path, and it is the reason the exclusion is a column rather
  than a subquery.
- The sidebar's two new counts are bounded and do not grow with mailbox size
  (SC-008).

## Before landing

```bash
cargo clippy --workspace --all-targets -- -D warnings
scripts/test-sanity.sh
scripts/check.sh            # includes the new view-roles invariant
scripts/issue-land.sh --detach
```

Run the integration suites the diff actually touches — `postio-storage`,
`postio-sync`, `postio-ui`, `postio-gtk`, `postio-ffi`, `postio-app` — and let
CI run the rest. `--full` needs a specific reason.

**Two things CI proves that a local run cannot:**

- **the macOS frontend** (`macOS build and Swift tests`, ~13 min) — Build 3's
  FFI change is gated there and cannot be verified on Linux;
- **the combination** — this branch and whatever else has landed.

**And one thing no compiler will catch**, learned on #1496 the same week:
`shell.css` is the only file here nothing type-checks. If a resolution or an
edit breaks it, the failure surfaces somewhere unrelated — last time, as two
composer focus tests. After touching it, check the brace balance and run
`cargo nextest run -p postio-gtk` in full.

## Against a live account (never in a gate)

```bash
cargo test -p postio-sync -- --ignored reserved_roles_live
```

Requires credentials in the keyring and a real server. Worth running once
against a provider with no Junk folder — the iCloud shape from #943 is what
started all of this.
