# ADR 0050 — A folder is created locally first

- **Status:** Accepted (2026-11-28)
- **Decision:** Creating a folder writes its row, marked pending, and queues `Operation::CreateMailbox`: it can be opened and moved into at once, and the drainer creates it on the server.

## Rules
- `Command::CreateFolder` is the one way in, for a person and for an agent (`create_folder`, ADR 0010). It writes and enqueues in one transaction, then emits `MailboxesChanged`.
- The role is always `Regular`, set explicitly: "Projects/Sent" is never guessed into a role; mapping one is ADR 0035's.
- The path is built with the parent's delimiter, else the inbox's, else `/`. A name that already exists, ignoring case, or a role's name is refused.
- Discovery never retires a pending row: a folder made offline must survive a reconnect that does not list it yet.
- An operation whose destination is pending waits (`Resolved::Later`) instead of failing.
- Success clears the mark and emits `MailboxesChanged`; a refusal is said as "Folder not created".
- Undo while pending and empty deletes the row; its queued operation goes with it. Undo after the sync needs a delete the backends lack, and is not offered.

## Why
- Local-first is the constitution's rule for every write; a create that waited on the network would be the one exception.
- A pending mark instead of `selectable = false`: that flag already means "the server stopped listing it", and conflating the two hides real loss.

## Revisit when
- Rename or delete folders is built (#2275): undo after the sync becomes possible.
