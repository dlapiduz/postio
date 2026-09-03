# ADR 0007 — The address book: one table, two provenances

- **Status:** Accepted — **GO** (2026-08-24)
- **Date:** 2026-08-24
- **Issue:** [#4 Address book / contact management](https://github.com/dlapiduz/postio/issues/4)
- **Related:** `docs/ARCHITECTURE.md` §6 (one matching language),
  [ADR 0005](0005-multiple-accounts.md) (accounts, and what "shared" means)
- **Decision:** explicit contacts and mail-history sightings are **the same
  row**, distinguished by a `source` column and protected by a **suppression
  tombstone**. Groups are an explicit table, expanded at compose time. vCard
  import/export is hand-written, dependency-free, and **preserves unknown
  properties verbatim** — which is the only way round-tripping is honest.

---

## What is already built

The MVP shortcut turns out to be most of the foundation.

| Piece | State |
|---|---|
| `contacts` table, `account_id NULL` = shared across accounts | Built |
| Identity = **normalised address**, not display name | Built, and correct |
| A user-set `name` overriding header names, never overwritten by a sighting | Built (`repository/contacts.rs:11`) |
| `record`, `record_message`, `get`, `by_address`, `list`, `search`, `set_name`, `delete` | All built |
| Sightings written on genuine insert only, never on re-enumeration | Built (`sync/src/contacts.rs`) — and carefully |
| `@` mode in the finder, ranked, wired | Built (`gtk/src/finder.rs`) |
| Composer recipient completion | Built |
| Creating a contact that has never sent mail | **Absent** |
| Groups, vCard, deletion that stays deleted | **Absent** |

`finder.rs`'s comment — *"Postio has no address book: contacts accumulate from
the addresses that have come through the mailbox"* — describes a missing
feature sitting on a finished data model.

---

## Q1 — Two tables, or one?

The instinct is a second table: `address_book` for real contacts, `contacts`
for the autocomplete cache. It is wrong, and the reason is a single user
gesture.

**A user types an address that has been seen 40 times and gives it a proper
name and a phone number.** With two tables that is a copy, and now two rows
claim the same normalised address, autocomplete has to merge them at query
time, and `times_seen` lives on the row that is *not* the one the user edits.
With one table it is an `UPDATE`.

**Decision: one `contacts` table, plus a provenance column.**

```sql
ALTER TABLE contacts ADD COLUMN source TEXT NOT NULL DEFAULT 'mail'
    CHECK (source IN ('mail', 'user', 'import'));
ALTER TABLE contacts ADD COLUMN suppressed INTEGER NOT NULL DEFAULT 0;
ALTER TABLE contacts ADD COLUMN uid TEXT;      -- vCard UID; also CardDAV's key
ALTER TABLE contacts ADD COLUMN vcard_extra TEXT;   -- see Q4
```

`source` is *how the row first appeared*, not what it is now. A `mail` row the
user edits becomes `user`; that is the promotion, and it is one statement.

---

## Q2 — Deletion has to stay deleted

This is the bug the feature would otherwise ship with, and it is worth stating
plainly because it is invisible until a user hits it.

`ContactRepository::delete` exists and works. On a `mail`-sourced contact it
also does nothing lasting: the next sync pass that inserts a message from that
address calls `record_message`, the unique index finds no row, and the contact
the user just deleted is back — with `times_seen` reset to 1, so it even looks
like a new one.

**Decision: `delete` on a `mail`-sourced contact sets `suppressed = 1` rather
than removing the row, and `record` must not resurrect a suppressed contact.**
It keeps counting sightings — the row is still the place that bookkeeping
lives — and stays out of autocomplete, out of the `@` finder, and out of the
contact list.

A `user`-sourced contact deletes for real; the user created it, so there is
nothing to suppress it against. And "unsuppress" is just creating it again,
which lands on the same row by address.

The test: record a message, delete the contact, record another message from the
same address, assert autocomplete does not offer it. That test fails today.

---

## Q3 — Groups

A group is a **named set of contacts**, not a query. This is the one place
where `ARCHITECTURE.md` §6's "one matching language" does not apply, and the
distinction is worth being explicit about: §6 governs *which messages*; a group
answers *which people*. A saved query cannot express "Ada, Grace and Katherine,
because I said so".

```sql
CREATE TABLE contact_groups (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE,  -- NULL = shared
    name       TEXT NOT NULL,
    uid        TEXT,                    -- vCard KIND:group
    created_at INTEGER NOT NULL
);
CREATE TABLE contact_group_members (
    group_id   INTEGER NOT NULL REFERENCES contact_groups(id) ON DELETE CASCADE,
    contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    PRIMARY KEY (group_id, contact_id)
);
```

**A group is expanded in the composer, at the moment it is picked, into its
member addresses.** There is no `family@` to put in a `To:` header, and
pretending otherwise would mean a draft whose recipients change between saving
and sending because someone edited the group in between. Expanding at pick time
makes the recipient list what the user can see, which is also what makes Bcc
behave.

Where §6 *does* reach: `postio-search` gains a `group:` field, so `group:family`
means "from or to any member" and works in the search bar, in a pinned sidebar
filter and in `[filters]` alike. That is one `Field` row and one arm in the
parser, resolved by `postio-index` to an address set — one language, one
parser, and dry-run for free.

---

## Q4 — vCard, and what "round-trips correctly" has to mean

The acceptance criterion is `vCard import and export round-trip correctly`.
Read strictly, that criterion cannot be met by a parser that models only the
fields Postio uses: a contact exported from another address book carries
`PHOTO`, `BDAY`, `ADR`, `TEL`, `X-`-prefixed vendor properties, and dropping
them on import means the export is lossy.

**Decision: parse the properties Postio understands, and keep every other
property verbatim in `vcard_extra`.**

This is the same move `postio-config` already makes with `Extras` — unknown
TOML keys are preserved rather than discarded, so a config written by a newer
Postio survives an older one. Applying it here means a contact imported from
another tool and exported again is byte-comparable in the properties Postio
never claimed to understand.

Understood: `UID`, `FN`, `N`, `EMAIL` (with `TYPE`/`PREF`), `ORG`, `NOTE`,
`CATEGORIES`, `KIND`, `MEMBER`, `REV`. Everything else is preserved.

**No dependency.** vCard is line-unfolding, then `GROUP.NAME;PARAM=v:value`
with backslash escaping — a few hundred lines, and it parses a file the user
was handed by someone else, which is a supply-chain surface worth not opening
for something this size. It lives in `postio-model::vcard`, which is where
`mail-parser`'s output already gets turned into domain types, and it adds no
crate to the graph — the constraint [ADR 0004](0004-composer-document-model.md)
Q1 imposed on `postio-model` is about *dependencies*, and this brings none.

Both vCard 3.0 and 4.0 are read; 4.0 is written. Most exports in the wild are
3.0, and refusing them would make import a feature that fails on the first real
file anyone tries.

---

## Q5 — Shared or per-account?

`contacts.account_id` is already nullable with `NULL` meaning shared, and both
partial unique indexes already exist. That was foresight and it holds up.

- **Sightings are per-account.** They are evidence about one mailbox, and with
  multi-account (ADR 0005) two accounts genuinely can know different people.
- **Contacts the user creates default to shared.** An address book is a
  person's, not an account's, and the alternative makes the user pick an
  account in a dialogue that has no other reason to mention one.
- **Autocomplete unions both**, which the existing indexes serve.

---

## Q6 — Ranking: explicit beats frequent

Autocomplete must use both sources (the issue's second criterion), and the two
are not commensurable — `times_seen = 400` for a mailing list robot is not
evidence that the user wants to write to it.

**Bands, then score within a band:**

1. `user` and `import` contacts, and group names.
2. `mail` sightings, most recently seen first (`last_seen_at DESC`), with
   `times_seen DESC` breaking a tie on recency.

Suppressed rows appear in neither. Match quality (prefix beats substring, name
beats address) applies within a band, never across one, so a contact the user
deliberately created is never pushed below a robot they have never replied to.

Recency led frequency after #424: a correspondent written to once yesterday
belongs above one written to fifty times last year, and the earlier
`(times_seen DESC, last_seen_at DESC)` ordering said the opposite. Frequency
still settles a tie on the same day — see
`frequency_decides_between_addresses_used_equally_recently` in
`crates/postio-storage/tests/storage_suite/contacts.rs` — which is the whole
of what it is for now.

---

## Q7 — Where the surface lives

- **Finding** stays in the `@` finder mode. It is built, it is ranked, and
  `finder.rs`'s reasoning about why picking a contact *searches their mail*
  rather than composing to them stands.
- **Managing** is a surface that takes over the reading pane, like the
  composer — not a separate window. It gets `Context::Contacts` in
  `postio-core`, which is what makes its commands reachable from the palette
  and printable in the cheat sheet without either learning about the widget
  (the reasoning `Context::Sidebar`'s doc comment already sets out).
- **Editing a contact is local-first like everything else**: SQLite write,
  emit the event, repaint. There is no remote half today, which is exactly why
  the commands must still go through the same path — when CardDAV arrives it
  becomes an operation-queue row and nothing above it changes.

---

## Q8 — What this deliberately does not decide

**CardDAV.** Out of scope, and left possible rather than designed: `uid`,
`REV` and a nullable `etag` are enough for a sync layer to key against later,
and the operation queue's `target_kind` CHECK will need `'contact'` when that
day comes. Designing the sync now would be designing against a protocol nobody
has read yet.

---

## Alternatives

**A separate address-book table.** Rejected in Q1: it duplicates a row per
address the moment a user promotes a correspondent, and splits `times_seen`
away from the row being edited.

**Hard-delete implicit contacts.** Rejected in Q2: sync resurrects them, and
the user cannot tell why.

**Groups as saved searches.** Rejected in Q3: a query cannot express an
arbitrary set of people, and a distribution list defined by a query would change
who a draft goes to between save and send.

**A vCard crate.** Rejected in Q4 on size and on supply-chain surface, against a
parser this small — and none of the obvious candidates preserve unknown
properties, which is the property the acceptance criterion actually needs.

---

## Consequences

- One migration: four columns on `contacts`, two new tables.
- `ContactRepository::delete` changes behaviour for `mail` rows; the test that
  proves it is one that fails today.
- `postio-search` gains `group:`; the shortcut and config references regenerate
  (`ARCHITECTURE.md` §2).
- `postio-model` gains `vcard` and no dependencies.
- `postio-core` gains `Context::Contacts` and its commands, which — per §2 — is
  what makes them exist at all.
