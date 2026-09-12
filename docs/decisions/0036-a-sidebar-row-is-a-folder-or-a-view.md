# ADR 0036 — A sidebar row is a folder or a view, and a view is never a destination

- **Status:** Accepted (2026-09-12)
- **Numbered 0036 at merge, not at draft.** ADR 0035 paid for the other habit
  twice — drafted as 0025 while 0025 was landing, renumbered to 0027 while 0027
  was landing. A number is claimed when a branch merges. This one was left
  unassigned in `specs/003-outbox-and-reserved-mailboxes/plan.md` until the day
  it landed, and checked against `main` immediately before.
- **Date:** 2026-09-12
- **Decision by:** a spec-driven session working
  `specs/003-outbox-and-reserved-mailboxes`, on a rule the feature could not
  be built without and that outlives it.
- **Issue:** [#1491](https://github.com/dlapiduz/postio/issues/1491) is what
  surfaced it; the rule is not about the Outbox.
- **Related:** [ADR 0005](0005-multiple-accounts.md) Q4 (the unified scope is a
  view, never a destination — this generalises it),
  [ADR 0019](0019-macos-frontend.md) (the second frontend, which is who this
  cost), [ADR 0035](0035-mailbox-roles-are-mapped-per-account.md) (roles per
  account), `PRODUCT.md` §9.
- **Decision:** **Every `MailboxRole` is a *folder* role or a *view* role. A
  view names no folder on any server, can never be stored in `mailboxes`, and
  is never somewhere a message can be put. Which one a role is, is a property
  of the role — not something each reader infers.**

---

## The rule

| Kind | Roles | Names a server folder | Storable | A destination |
|---|---|---|---|---|
| `Folder` | `Inbox` `Archive` `Sent` `Drafts` `Trash` `Junk` `Regular` `Flagged` | yes | yes | yes |
| `View` | `Snoozed` `Outbox` | **no** | **no** | **no** |

A **view** is a saved question about messages filed elsewhere. It has a name, a
position and a count, and nothing else: no path, no UIDVALIDITY, no sync state,
and no id — because there is no row.

## Why this needed deciding

It was already true, and enforced three different ways, none of which said so.

`postio-gtk::feed` invented the Flagged and Snoozed rows as `Mailbox` values
with `MailboxId::new(-1)` and `-2`. A negative id is an id that means "not an
id", and it fails in two ways that are really one:

- **A reader that forgets gets silence, not an error.** A sentinel travels
  everywhere a real id does. `MessageSet::InMailbox { mailbox: -1 }` matches no
  rows and reports success; `Command::Move` to `-1` is a foreign key that does
  not resolve. The comment on `FLAGGED_ROW` listed the places the value must
  never reach — a rule no compiler checks.
- **A frontend that never knew has no rows at all.** The convention lived in one
  widget, so the macOS sidebar — reading the same store through the same shared
  layer — simply never had Flagged or Snoozed. #1155 moved the sidebar's
  *ordering* into `postio-ui` for exactly this reason and left the rows behind.

Meanwhile `MailboxRole::Snoozed` could never be stored, because the `CHECK` on
`mailboxes.role` happened not to list it. That worked, for three years, for the
wrong reason: whoever wrote the constraint listed the roles that existed that
day.

## `Flagged` is a folder, and it is the case that makes the rule precise

The split is by whether RFC 6154 defines a `SPECIAL-USE` attribute — **not** by
whether the sidebar draws a synthetic row.

`\Flagged` *is* defined, and `MailboxRole::from_special_use` honours it, so a
server really can have that folder; Gmail's "Starred" is one, and `flagged` has
always been in the schema's `CHECK` for that reason. The sidebar synthesises a
Flagged row only for accounts whose server advertises none.

This was got wrong while building. Classifying `Flagged` as a view — because
the sidebar draws one — would have made a real Gmail folder unstorable. The
invariant below caught it on its first run, which is the argument for having
written it.

## What enforces it

Three things, because no single one is sufficient:

1. `MailboxRole::kind()` in `postio-model`, exhaustive with no fallback arm: a
   new role is a compile error until it is classified.
2. `MailboxRepository::create`/`update` refuse a `View` with a typed error that
   **names the role**. A `CHECK` violation says "constraint failed" and does not
   say which — and this is a caller bug worth naming.
3. `scripts/checks/check-view-roles-are-not-storable.py`, which fails when the
   SQL `CHECK` list and the `Folder` set disagree. It reads both out of the
   source, so it holds no third copy to drift from the other two.

And for the ids: `scripts/checks/check-no-sentinel-mailbox-ids.py` refuses a
negative `MailboxId` anywhere under `crates/`. A view row is *unassigned*, which
is already this codebase's word for "not a row in the database".

`postio_gtk::sidebar::SidebarChoice` — `Folder(id)` or `View(role)` — is how a
reader says which kind it has rather than remembering a number. The id decides,
not the role: a server with a real `\Flagged` folder gives a row with **both**
an id and the Flagged role, and that is a folder, the one holding the account's
own mail.

## Consequences

**Good.** The view rows are built once, in `postio-ui::sidebar`, so macOS has
Flagged and Snoozed for the first time. `ListScope::mailbox()` answering `None`
for a view becomes checkable rather than remembered. Adding the Outbox was a
variant and a `WHERE` clause rather than a fourth sentinel.

**The cost, stated plainly.** Three view rows share the unassigned id, so
anything keying on `MailboxId` cannot tell them apart. That is not theoretical:
the sidebar's keyboard walk did exactly that, and stuck on Snoozed for ever —
`j` could not reach the Outbox at all. The fix was for the walk to use the row
it was holding rather than look one up. **Any new code that identifies a
sidebar row by its id is wrong for the same reason**, and should take a
`SidebarChoice`.

**Not decided here.** Whether saved searches become views of this kind. They
are a separate mechanism in a separate section today, and folding them in is a
larger question than this rule answers.

## Alternatives rejected

**A distinct `SidebarRow` type.** Cleaner on paper. But the feed, the FFI, the
drop handler and the context menus all take `&[Mailbox]`, and changing that is
a far larger diff than the rule needs — for a distinction three call sites
actually make.

**Keeping the sentinels and documenting them harder.** The documentation
existed. It was a comment listing the places a value must not reach, and the
frontend that needed it most never read it.
