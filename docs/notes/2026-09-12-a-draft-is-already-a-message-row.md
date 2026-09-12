# A draft is already a message row (2026-09-12)

The thing to know before designing anything that lists drafts, or that adds a
row to the sidebar.

## Every draft has a `messages` row, written offline

`DraftRepository::save` calls `list_row`
(`crates/postio-storage/src/repository/drafts.rs`), which mirrors the draft into
the account's Drafts folder **in the same transaction as the draft itself** —
no network, no waiting. `drafts.message_id` is the link. Sync does not
duplicate it: `upsert_batch` retains an incoming batch against
`own_draft_copies`, and `set_server_copy` attaches the UID to the row that
already exists.

This is #166's work and it is load-bearing in a way its own issue does not
say. The Outbox in `specs/003` was specified as though it might need a second
row model — a list backed by the `drafts` table, with its own identity and its
own paging — and it needed none of that. A draft already has a `MessageId`,
which is what the selection, the cursor, every `MessageTarget` command and the
thread view are keyed on.

**So: the Outbox and Drafts are two predicates over one folder, not two
folders.** The mirror row sits in Drafts whatever the draft's state;
`messages.send_state` says which list draws it. If you are about to write a
query that joins `drafts` to answer what a list shows, check whether the
column already answers it.

## The one hole, and what closed it

`list_row` returns early when the account has no Drafts mailbox — "the ordinary
state of an account that has not finished its first sync". While that is true
the draft is durable but *unlisted*, and the Drafts badge reads 0 because it is
the mailbox's cached count of message rows.

`specs/003` closed it from the other end: discovery now creates a folder for
any reserved role that resolves to nothing, so every account has a Drafts
folder to mirror into. That is why the lowest-priority story in that spec
(US3, reserved roles) was built *before* the highest (US1, the Outbox) — the
dependency runs the other way from the priorities.

## What `total_count` means, and does not

`mailboxes.total_count` counts message rows filed in a folder. For Drafts that
includes the ones in flight, because their mirror row is still there — and that
is deliberate: the column means the same thing for every folder, and teaching
three intricate triggers a fourth condition to make one folder different was
the alternative.

The sidebar therefore does **not** read `total_count` for Drafts. It asks
`MailboxRepository::draft_counts`, which answers three numbers in one read:
what is on its way, what has stopped and needs a person, and what Drafts should
show. If you add a number to that row, add it there.

## Two traps, both paid for

**A sentinel id.** `Flagged` and `Snoozed` used to be `Mailbox` values with
`MailboxId::new(-1)` and `-2`. Views have no id now — see
[ADR 0036](../decisions/0036-a-sidebar-row-is-a-folder-or-a-view.md) — which
means three sidebar rows share the unassigned id. **Anything identifying a
sidebar row by `MailboxId` is wrong.** The keyboard walk did exactly that and
stuck on Snoozed for ever; take a `postio_gtk::sidebar::SidebarChoice` instead.

**A second copy of `LIST_COLUMNS`.** A thread query in `threads.rs` spelled the
same thirteen columns out again and handed them to the same `read_list_row`.
Adding a fourteenth left that copy behind: "Invalid column index: 13", the
unified list unable to page at all, and **every storage-level test passing** —
the failure surfaced two crates away in `postio-ffi`. The copy is gone. If you
add a column to a list row, `LIST_COLUMNS` is the only place it belongs.

## Where the detail is

`specs/003-outbox-and-reserved-mailboxes/` — `research.md` for why each
decision went the way it did, `contracts/` for the rules, and `spec.md` for
what a person sees.
