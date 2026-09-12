# Contract: a mailbox role is either a folder or a view

**Consumers**: `postio-storage`, `postio-sync`, `postio-ui`, `postio-gtk`,
`postio-ffi`, and the macOS frontend through it.

This is the contract the branch's one ADR records, because it outlives the
feature and other work must obey it.

## The rule

Every `MailboxRole` has a **kind**, and the kind decides what may be done with a
mailbox wearing it.

| Kind | Roles | Names a server folder | Storable in `mailboxes.role` | A destination |
|---|---|---|---|---|
| `Folder` | `Inbox` `Archive` `Sent` `Drafts` `Trash` `Junk` `Regular` | yes | yes | yes |
| `View` | `Flagged` `Snoozed` `Outbox` | **no** | **no** | **no** |

A **view** is a saved question about messages filed elsewhere. It has a name, a
position and a count, and nothing else: no path, no UIDVALIDITY, no sync state,
no ability to receive a moved message.

## What each consumer must guarantee

**`postio-model`**
- `MailboxRole::kind()` is total and exhaustive; adding a role is a compile
  error until it is classified.
- `MailboxRole::RESERVED` is exactly the six roles FR-026 guarantees. `Regular`
  is a `Folder` but is not reserved.

**`postio-storage`**
- `MailboxRepository::create` and `update` **refuse** a `View` role with a typed
  error naming the role. Not a panic, not a silent skip — a caller that tries
  has a bug and must be told which.
- The `CHECK` on `mailboxes.role` lists exactly the `Folder` spellings.

**`scripts/checks/`**
- One invariant fails when the `CHECK` list and the `Folder` set disagree. This
  is what the contract rests on: without it, the next role gets added to one and
  not the other, which is how `Snoozed` came to be unstorable by accident rather
  than by decision.

**Every scope consumer**
- `ListScope::mailbox()` answers `None` for a view. Callers that need somewhere
  to put a message must take that `None` as "not a destination" rather than
  treating it as "unknown". This is already the documented rule; the kind is
  what makes it checkable rather than remembered.

## What this replaces

Today "is this a real folder" is answered three different ways, each in the
caller that remembered to ask:

- `id.get() > 0` (`postio-gtk/src/feed.rs:1190`),
- an empty `path`,
- and, for `Snoozed`, a SQL `CHECK` that happens not to list it.

The negative-id sentinels (`FLAGGED_ROW = -1`, `SNOOZED_ROW = -2`) exist only
because there was no way to say "view" — and because they live in a widget, the
macOS frontend has never had those rows at all.

## Tests that hold it up

1. Every role is `Folder` or `View`, and `RESERVED` contains exactly six.
2. Creating a mailbox with each `View` role is refused, and the error names the
   role.
3. The `CHECK` list and the `Folder` set agree — the repository check, run in
   CI.
4. `ListScope::mailbox()` is `None` for every view scope, asserted per variant
   rather than in a loop, so a new variant fails to compile rather than being
   silently skipped.
5. No `Mailbox` with a negative id exists anywhere after Build 3 — a grep-level
   invariant, because the sentinel is exactly the thing being removed.
