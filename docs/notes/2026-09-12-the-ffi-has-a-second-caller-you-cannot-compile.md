# The FFI has a second caller you cannot compile (2026-09-12)

Read this before changing anything in `crates/postio-ffi/src/`.

## What happened

`specs/003-outbox-and-reserved-mailboxes` changed three things on the
boundary: `RowFfi.draft: bool` became `send_state: Option<String>`,
`MailboxFfi` gained `flagged` and `snoozed`, and `MailboxRoleFfi` gained
`outbox`. Every Rust gate was green — clippy, `cargo check --workspace
--all-targets`, the sanity tier, the integration suites, `check.sh` — and the
branch landed.

Then `main`'s macOS job went red, and stayed red across **three** landings:

1. Two `switch folder.role` statements in `FolderRow.swift` became
   non-exhaustive.
2. `MessageTableTests.swift` built a `RowFfi` with `draft: false`.
3. `WindowStateTests.swift` built a `MailboxFfi` without the two new counts.

Each was a one-line fix and each cost a ~13-minute CI round trip, found one at
a time because the Swift compiler stops at the first file it cannot build.

## Why no local gate can catch this

There is no Swift toolchain on this workstation, and there is not meant to be.
`macos/` is compiled on exactly one machine, in CI, thirteen minutes away.
Everything the Rust side knows how to check had already passed — the
information simply is not here.

Note what this is *not*. It is not a missing invariant: Swift's exhaustive
`switch` is the same guard as `MailboxRole::kind()`'s exhaustive match, and it
worked exactly as designed. ADR 0036's claim that "a new role is a compile
error until it is classified" held in both languages. The gap is only in
*when* you find out.

## What to do instead

Before landing a change that touches `crates/postio-ffi/src/`, list what you
altered and grep `macos/` for it:

```bash
git diff origin/main...HEAD -- crates/postio-ffi/src/ \
  | grep -E '^[-+] *pub (fn|struct|enum|[a-z_]+:)'
grep -rn 'RowFfi(\|MailboxFfi(\|switch .*\.role' macos/
```

The first command names every boundary field and function the branch changed.
The second finds the Swift that builds those types by hand or switches over
them — which, on this repository today, is a handful of call sites in two test
files and one view. All three failures above were in that grep's output before
the first landing.

One pass costs a minute and collapses three CI round trips into none.

## Where the detail is

`docs/decisions/0036-a-sidebar-row-is-a-folder-or-a-view.md` for why the
frontends share a layer at all, and the "two things CI proves that a local run
cannot" section of
`specs/003-outbox-and-reserved-mailboxes/quickstart.md`.
