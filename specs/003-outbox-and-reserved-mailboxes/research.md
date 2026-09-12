# Phase 0 Research: The Outbox, and reserved mailboxes every account has

**Feature**: `specs/003-outbox-and-reserved-mailboxes` | **Date**: 2026-09-11

Six questions the spec left to the plan. R1 is the one that decides how big
this feature is, and the answer is: smaller than it looks.

---

## R1 — What is the Outbox a view *over*?

**A draft already has a `messages` row, written offline, in the same
transaction as the draft itself.** `DraftRepository::save` calls `list_row`
(`crates/postio-storage/src/repository/drafts.rs:180`, defined at `:733`),
which mirrors the draft into the account's Drafts mailbox with `\Draft` and
`\Seen`, `received_at = updated_at`, and the link written to
`drafts.message_id`. Its doc is explicit about why (#166): the message list is
a windowed query over `messages`, so a draft that is only a `drafts` row cannot
appear in the folder the sidebar sends people to — and *"This row is written in
the same transaction as the draft, offline and always."*

Sync does not duplicate it. `MessageRepository::upsert_batch` reads
`own_draft_copies` (`messages.rs:2514`, used at `:756`) and retains the
incoming batch against the rows the store already owns;
`DraftRepository::set_server_copy` (`drafts.rs:493`) attaches the UID to *that*
row and deletes any stray synced twin (#51).

**Decision: the Outbox is a `ListScope` variant over `messages`, exactly the
mechanism `Flagged` and `Snoozed` use and exactly what #1491 proposed.** No new
row source, no second identity, no change to how a draft is stored.

**Rationale.** The hard part — giving a draft a stable `MessageId` that the
selection, the cursor, `MessageTarget` and the thread view all already
understand — was done by #166. Building the Outbox over the `drafts` table
instead would introduce a second row model for one small, usually-empty folder
and would have to re-solve identity in every caller.

**Alternative considered and rejected: an Outbox backed by a `drafts` query.**
It looks independent of the message list and therefore safer. It is not: it
needs a row identity, and `ListScope::mailbox()`'s own doc records what happens
when a scope's identity is load-bearing and absent.

**The one hole, and US3 is its fix.** `list_row` returns early when the account
has no Drafts mailbox (`drafts.rs:734-737`) — "the ordinary state of an account
that has not finished its first sync". While that is true the draft is durable
but unlisted, and the Drafts badge reads 0 because it is the mailbox's cached
count of message rows. FR-026 — every account has one mailbox per reserved role
— closes it. **This is a real ordering dependency: US1's offline correctness
rests on US3, even though US3 is the lower-priority story.** The plan sequences
them accordingly.

---

## R2 — How do Drafts and the Outbox divide rows that are all in one mailbox?

The mirror row's `mailbox_id` is the Drafts mailbox, whatever the draft's
state. So "in the Outbox" and "in Drafts" are not two mailboxes; they are two
predicates over one.

Three consequences, and they are the substance of US1 and US2:

1. `ListScope::Outbox(AccountId)` selects rows whose draft is `Queued` or
   `Sending`.
2. `ListScope::Mailbox(drafts_id)` must *exclude* those (FR-004), and it is the
   generic mailbox scope — a subquery there would land on every folder listing,
   which is the hot read path Principle V gates.
3. The Drafts badge is `mailboxes.total_count`, maintained by triggers over
   `messages` (`0001_initial_schema.sql:657-720`). Left alone it counts the
   in-flight rows too, which FR-022 forbids.

**Decision: denormalise the draft's send state onto the `messages` row** — one
nullable column, written by the same code that writes `drafts.state`, indexed
partially. Both scopes then become plain indexed predicates rather than
subqueries, the exclusion costs the generic mailbox scope one `AND` on a column
that is `NULL` for every ordinary message, and the triggers can be taught the
split.

**Rationale.** It is the same shape the schema already uses: `messages.draft`,
`seen`, `flagged`, `answered` are denormalised from the `FlagSet` for exactly
this reason (`messages.rs:974-981`) — a list query must not join to answer what
a row is. Doing it any other way puts a correlated subquery on the path every
folder in the application reads through.

**Alternative rejected: compute the split in the repository above SQL.** It
breaks cursor paging — a page of 50 that then drops rows is not a page of 50 —
and `ListQuery`'s cursor is a row value over `(received_at, id)` precisely so
SQLite can seek rather than filter.

**Counts.** The Drafts total and the Outbox count come from one query per
account, run where the sidebar's data is assembled, rather than from a fifth
trigger-maintained column: the Outbox is not a mailbox and has no row to hold
one. The Drafts badge stops reading `total_count`. This adds a read to the
sidebar refresh path, so it carries `postio_storage::test_support::counting`
assertions — bounded statements and rows, independent of mailbox size (SC-008).

---

## R3 — How does a role get a folder the server does not have?

**`MailBackend` cannot create a mailbox at all.** The trait
(`crates/postio-account/src/backend/mod.rs:107`) has `list_mailboxes`,
`select`, `status`, `fetch_*`, `store_flags`, `move_messages`, `copy_messages`,
`expunge`, `append`, `find_by_message_id`, `existing_uids`, `idle` — and no
`create`, `rename`, `delete` or `subscribe`.

**Pimalaya has it**, so no wire code is written: `io-imap` (pinned `=0.6.0`,
`crates/postio-account/Cargo.toml:63`) implements RFC 3501 `CREATE` at
`src/rfc3501/create.rs`, exposed as `client.rs:559`'s `fn create(mailbox)`.
This is what the constitution's "Pimalaya first" asks, and the survey is
current.

**Decision:** one new trait method, `create_mailbox(&self, path: &str)`, with
four implementations — `imap/backend.rs` (a thin addition beside the existing
`list`/`select`/`status` in `imap/mailboxes.rs`), `backend/mock.rs`, and
`postio-jmap` / `postio-gmail`, which answer `Unsupported` until those adapters
are real.

**Called from discovery, not the operation queue.** Creating a role folder is
discovery reconciling an account against its server — already a network context
holding a connection (`crates/postio-sync/src/discover.rs`) — not a user's
mutating action needing an undo. A queue route would need a new `Operation`, an
inverse nobody wants (deleting a folder), and a queue row no keystroke asked
for.

**Remembering a refusal (FR-031):** the server's reason and the time, stored
per account-and-role in the `mailbox_roles` table that `feature/mailbox-roles`
adds — the one place that already answers "what do we know about this account's
roles". Cleared when the map changes or the folder appears.

**Alternative rejected:** creating folders during account setup. It covers only
accounts added after this ships, and says nothing about a server that loses a
folder later.

---

## R4 — Where does a role that is a *view* live, given the schema forbids it?

`mailboxes.role`'s `CHECK` lists
`inbox, archive, sent, drafts, trash, junk, flagged, regular`
(`0001_initial_schema.sql:119`). `MailboxRole::Snoozed` exists in Rust and in
`from_name` but **can never be stored**. The `Flagged` and `Snoozed` rows are
`Mailbox` values invented in `postio-gtk::feed` with negative ids
(`FLAGGED_ROW = -1`, `SNOOZED_ROW = -2`, `feed.rs:1004-1044`) and empty paths —
which is exactly why the macOS frontend has never had them
(`postio-ffi/src/session.rs:2057` lists real folders only).

So the schema already enforces FR-041 for `Snoozed`, by accident and without a
name for what it is doing. `Outbox` joins that category.

**Decision:** name it. `MailboxRole::kind()` answers `Folder` for the six
reserved roles, `Regular` **and `Flagged`** — RFC 6154 defines `\Flagged`, so a
server can really have that folder — and `View` for `Snoozed` and `Outbox`; the
storage repository refuses to write a `View` role with a typed error rather
than leaving SQLite's `CHECK` as the only guard; and a `scripts/checks/`
invariant keeps the `CHECK` list and the `Folder` set from drifting apart.

**And the view rows move to `postio-ui::sidebar`**, which is the toolkit-free
layer both frontends already consume — three functions today (`role_order`,
`primary_within`, `sections`), re-exported verbatim by `postio-gtk::sidebar`
and called by the FFI. Building the view rows there is what satisfies FR-018
and FR-038, and it is what finally gives macOS its Flagged and Snoozed rows.

**Alternative rejected:** a distinct `SidebarRow` type. Cleaner on paper, but
the feed, the FFI, the drop handler and the context menus all take
`&[Mailbox]`; changing that is a far larger diff than this feature needs.

**Two FFI gaps this exposes**, both worth knowing before US4 is estimated:
`MailboxFfi` carries no `flagged`/`snoozed` count, and there is no FFI
equivalent of `count_for`. So "both frontends show the same rows" costs two
fields and one shared count rule, not just the row list.

---

## R5 — How does the list learn a send moved?

Lists react through `Arrival` → `Reaction` (`postio-model/src/scope.rs:161`),
and `ListScope` has arms in `reaction`, `is_drawn_from` and `mailbox()`. The
send drainer emits no events itself (`postio-sync/src/send.rs` holds no
`Event::`); compose events exist (`DraftSaved`, `MessageSent { draft }`) but
none reports the drainer moving a draft to `Sending`, `Sent`, `Failed` or
`Unconfirmed`.

**Decision:** one new event, `DraftStateChanged { account, draft, state }`,
emitted wherever the send path sets state. Outbox and Drafts scopes react
`Reload`; everything else ignores it. `ListScope::Outbox` answers `None` from
`mailbox()`, which is what makes FR-010 checkable rather than remembered.

**Alternative rejected:** reusing `MailboxesChanged`, which means "the folder
list changed" and would reload the whole sidebar on every send transition.

---

## R6 — What does this feature owe the documents?

- **ADR 0021's "What the user sees" table** (`:230-243`) says every send state
  is "reachable from the Drafts list and from the composer". FR-001 and FR-020
  make that false for `Queued` and `Sending`. The table is corrected in this
  branch; the decision it records is not reopened.
- **One new ADR**, and only one: the rule from R4 — *a mailbox row is either a
  folder or a view, and a view is never a destination* — outlives this feature
  and other work must obey it. Everything else is recorded in `spec.md`, per
  `CLAUDE.md`. **Number it when the branch merges, not now** — that is the
  lesson `feature/mailbox-roles` just paid for twice.
- **`docs/keybindings.md`** regenerates for the Outbox's navigation command
  (FR-016).
- **`docs/PRODUCT.md` §9** describes the sidebar and does not mention an
  Outbox.
