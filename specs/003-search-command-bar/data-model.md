# Phase 1: Data Model — Search and Command Bar

**Feature**: [spec.md](./spec.md) | **Plan**: [plan.md](./plan.md)

This feature adds no persisted data and no migration. What it adds is two
enumerations and one resolution step; everything else already exists.

---

## New: `FinderMode` (in `postio-ui`)

The bar's modes, lifted out of `postio-gtk::finder::Mode` so the macOS frontend
shares them rather than re-deriving them (R5).

| Field | Type | Meaning | Rules |
|---|---|---|---|
| `prefix` | `Option<char>` | The character that reaches the mode from an empty box | `None` for search, which is what typing does with no prefix. Every other mode's prefix is unique across the table |
| `name` | `&'static str` | What the mode is called where it is listed | Non-empty |
| `purpose` | `&'static str` | What the mode is for, in a user's words | Non-empty; this is what the hint and the docs both show |

**The set** is what ships today, unchanged: search (no prefix), `>` command,
`#` mailbox, `+` label, `@` contact.

**Invariants** (each becomes a test):

- Prefixes are unique, and no prefix is a character an ordinary query starts with by accident.
- Exactly one mode has no prefix.
- Every mode has a non-empty name and purpose.
- The table is the only place the set is written down: the bar's hint, the cheat sheet and the generated documentation all read it.

---

## New: destination commands (in `postio-core`)

One `CommandId` per destination that gets a sequence (R3, R4). These are
ordinary registry commands and gain nothing new structurally — which is the
point: being ordinary is what makes them discoverable.

| Id | `[keys]` spelling | Title | Default binding | Role targeted |
|---|---|---|---|---|
| `GoToInbox` | `go_to_inbox` | Go to inbox | `g i` | `Inbox` |
| `GoToDrafts` | `go_to_drafts` | Go to drafts | `g d` | `Drafts` |
| `GoToSent` | `go_to_sent` | Go to sent | `g t` | `Sent` |
| `GoToFlagged` | `go_to_flagged` | Go to flagged | `g s` | `Flagged` |

Archive is recommended and unlettered pending the design authority (R4). Junk,
Trash and Snoozed get no command and stay reachable through `#`.

**`CommandSpec` values**, following the shape every other entry uses:

- `contexts`: the list surfaces — the same set `FirstMessage` and `FocusSidebar` use. Not the composer, where `g` is a letter someone is typing.
- `destructive`: `false`, and therefore `recovery: Recovery::None`. Going somewhere destroys nothing.
- `requires`: `None`. A destination that does not exist for the account is reported when the command runs (FR-041), not hidden from the palette — a user who cannot find "Go to drafts" learns less than one who is told this account has none.
- `alternate_bindings`: empty.

**Invariants** (each becomes a test):

- Every new id has a non-empty title and default binding, which `command_registry.rs` already asserts for all commands.
- No new binding collides with an existing one, `g g`, `g f` and `g a` included.
- `docs/keybindings.md` regenerates to include them, which `keybindings_doc.rs` already enforces.
- Each id appears in the palette in a context where it can act.

---

## Resolution: a role becomes a folder

The one piece of behaviour with a decision in it. A destination command names a
**role**; the message list needs a **mailbox id**.

- **Input**: a `MailboxRole`, and the scope currently in view.
- **Source**: the mailbox list the feed already holds — the same list the sidebar draws and the finder searches. No query, no network (Principle I).
- **Output**: a `MailboxId`, or nothing.
- **Nothing found**: the user is told the current account has no such folder (FR-041). This is a normal outcome, not an error state.
- **Scope**: whatever the sidebar's current scope resolves, rather than a second answer invented here (R7).

**State change on success** is exactly what picking the folder in the bar
already does: `sidebar().select(id)` and `show(id)`. There is deliberately no
new path — a second way to arrive would be a second set of bugs about what the
sidebar highlights.

---

## Unchanged

Named because a reader should not have to check: `ParsedQuery` and the operator
vocabulary, `Chip`, the palette `Entry` and its `positions`, `Mailbox`,
`MailboxRole`, saved searches, and every existing `CommandId`. This feature
reads them and adds beside them.
