# Contract: one sidebar row model, two frontends

**Producer**: `postio-ui::sidebar` — toolkit-free, no GTK, no SQL.
**Consumers**: `postio-gtk::sidebar` and `postio-ffi::session` (and the macOS
frontend through it).

## The rule

**A frontend renders sidebar rows; it does not decide what they are.** Which
rows exist, what each is called, what number it shows, and in what order, are
all answered once in `postio-ui::sidebar`.

## What is wrong today

`postio-ui::sidebar` has three functions — `role_order`, `primary_within`,
`sections` — moved out of GTK by #1155 so macOS would stop sorting folders
alphabetically. But the **view** rows never came with them: `Flagged` and
`Snoozed` are built inside `postio-gtk::feed` (`:1004-1044`) as `Mailbox` values
with negative ids and empty paths, their counts summed across the account's real
folders in `Folders::arrived` (`:955-966`).

Two consequences:

- **macOS has never had Flagged or Snoozed.** `Session::mailboxes`
  (`postio-ffi/src/session.rs:2057`) lists real folders only.
- **Adding the Outbox the same way ships that defect a third time**, and puts
  the definition of "what is on its way" inside a widget.

## The shape

`postio-ui::sidebar` returns the account's rows, each carrying what a frontend
needs and nothing it must decide:

- **kind** — a folder or a view (see [mailbox-role.md](./mailbox-role.md))
- **display name** — the role's name for a primary, the server's name for a
  twin or an ordinary folder
- **count**, and **attention count** where the row has one
- **position** — from the shared `role_order`
- **whether it is drawn at all** — the Outbox is absent when empty (FR-012)

Order is fixed and shared: Inbox, Flagged, Snoozed, Drafts, **Outbox**, Sent,
Archive, Junk, Trash, then ordinary folders as a tree.

## The counting rules move too

`count_for` lives in `postio-gtk/src/sidebar.rs:323` and is the reason the two
frontends disagree: Drafts shows a total, Flagged shows flagged, Snoozed shows
snoozed, Sent/Archive/Trash/Junk show nothing, Inbox and ordinary folders show
unread, and zero is never drawn. That table is a product decision, not a widget
detail, and it moves into the shared layer with the rows.

Two additions (FR-013, FR-022): the Outbox shows how many messages it holds, and
Drafts shows a total **and** an attention count for what needs a person.

## What the FFI still owes

`MailboxFfi` (`postio-ffi/src/mailbox.rs:50`) carries `unread` and `total` only.
The shared counting rules need `flagged` and `snoozed` as well, and there is no
FFI equivalent of `count_for` at all. **So "both frontends show the same rows"
costs two fields and one shared count rule, not just the row list** — worth
knowing before US4 is estimated.

## Invariants

1. Neither frontend constructs a `Mailbox` of its own. After this, `feed.rs` has
   no `flagged_folder`, no `snoozed_folder`, and no negative `MailboxId`.
2. No row's identity is a sentinel. A view row is a view because its role says
   so, not because its id is negative.
3. The same account produces the same rows in the same order on both frontends.
4. A view row never reaches a query or a `Command::Move` — the constraint
   `feed.rs:990-1003` states in prose today becomes the type's answer.

## Tests

1. `postio-ui::sidebar` unit tests (no display, microseconds): an account with
   queued sends yields an Outbox row with the right count; with none, no Outbox
   row; order matches the canvas; a role with two folders yields one row.
2. The Drafts row carries a total and an attention count, and no attention
   marker when nothing needs one.
3. `gtk_suite`: the rendered sidebar matches the shared model's answer, rather
   than re-deriving it.
4. `postio-ffi/tests/mailboxes.rs`: the FFI returns the same rows in the same
   order, view rows included — the assertion that would have failed for Flagged
   and Snoozed every day since #1155.
5. A grep-level invariant: no negative `MailboxId` is constructed anywhere.
