# Research: Contacts

Phase 0 of `/speckit-plan`. Every decision the plan rests on, with what was
measured or read to reach it. Paths are at `9c44fd44`.

## What exists, in one paragraph

Only sync and the seeder write contacts in production
(`postio-sync/src/contacts.rs:36` → `ContactRepository::record_message`,
called from `initial.rs:634` and `resync.rs:589` for UIDs not already known,
which is what keeps a re-enumeration from double counting). `create`,
`set_name`, `delete` and every group mutator have test callers only. Both UI
readers (`postio-app/src/search.rs:863` for the `@` finder,
`postio-app/src/compose.rs:937` for completion) pass `Some(account)`, so a
shared contact never reaches any screen. `uid` and `vcard_extra` are columns
nothing reads or writes. The `addresses` table (`schema.rs:131`) is already a
global, unique-by-normalised-form table that every `recipients` row points at.

---

## R1 — Where "an address belongs to at most one person" lives

**Decision: a nullable `contact_id` on the existing `addresses` table.**

**Rationale.** `addresses` is already the one row per normalised address that
every message recipient references (`recipients.address_id`,
`messages.rs:2593` upserts it on every message write). Putting the owner on it
makes the spec's uniqueness rule (FR-010) a column rather than an index to
keep in step; it makes FR-032's name substitution one more primary-key hop on
a join the list query already does (`LIST_COLUMNS`, `messages.rs:672`); and it
makes `group:` and the new `with:` (R6) resolve through `recipients.address_id`
directly instead of comparing normalised strings, as `group:` does today
(`executor.rs:1526`). #384 found `recipients` was a third of the database
because of duplicated strings; a second address table would repeat that.

**Alternatives considered.** A `contact_addresses` table holding its own copy
of each address (rejected: a second home for the normalised address, and every
join from mail to a person goes through a string compare). Keeping one contact
row per address and adding a `person_id` above it (rejected: the name, the
tombstone and the vCard would still have two candidate homes).

## R2 — Where the mail's evidence lives

**Decision: a `contact_sightings` table keyed by `(address_id, account_id)`**
holding `times_seen`, `last_seen_at`, `last_name` (the display name last seen)
and `times_written` (messages the user sent to it). The person row carries
**denormalised aggregates** (`times_seen`, `last_seen_at`, `written`,
`seen_name`) maintained on the same write, and recomputed from sightings on
the rare structural edits (join, detach, move).

**Rationale.** The spec keeps evidence with the address, per account (the
inherited Q5). But every read the budgets care about — the default list, its
ordering, completion's banding — is per *person*; computing them from
sightings at read time would group 20,000 people's rows on every page. The
write path already pays per address; one more `UPDATE … WHERE id = ?` per
address on a primary key is the cheaper side of the trade.

**Cost on the sync write path**, which #728 tuned: today `record_in` is two
cached statements per address (a lookup, then an update or insert). After:
the address row already exists in the same transaction, so a lookup of
`(id, contact_id)`, an upsert of the sighting, and an update of the person —
three, plus two more only the first time an address is ever seen (create the
person, set `contact_id`). The task that changes this carries a
`test_support::counting` assertion on statements per recorded address, and
`benches/sync_writes.rs` reports it nightly.

## R3 — What "written to" means (spec FR-005, left to planning)

**Decision:** an address has been *written to* when a message whose `From` is
one of the account's own addresses (`Account::owns_address`,
`postio-model/src/account.rs:412`) lists it in `To`, `Cc` or `Bcc`. It is
counted at the same recording site, from the same message, before own
addresses are stripped (`postio-sync/src/contacts.rs:41` strips them today).
It does not look at which folder the message is in.

**Rationale.** The sent copy arrives through the Sent folder's sync, and a
message the user sent from another client is just as much "written to". Keying
on the From header rather than the mailbox role means a missing or remapped
Sent role (ADR 0035) does not silently empty the default list.

**Alternative considered.** Counting only messages in the `Sent` role mailbox
(rejected for the reason above).

## R4 — Deletion without a `suppressed` column

**Decision: a person has a `state` — `live`, `deleted`, or `merged` — and an
address is suppressed exactly when its owner is not `live`.**

**Rationale.** The inherited Q2 rule is "the next message from a deleted
address must not bring it back". If deleting a person keeps it (FR-023a needs
it kept anyway, to restore it whole), its addresses stay attached to it, and
new mail from them counts toward a hidden person — which is the tombstone,
without a second flag to keep in step. FR-024 (re-creating lifts it) becomes
"the address belongs to a deleted person; move it", which FR-015 already
defines. Every reader filters `state = 'live'`.

`merged` is the state of a person absorbed by a join: it keeps its fields and
group memberships so the join can be undone exactly (R8), and is never shown.

## R5 — The Contacts list: windowed, keyset-paged, filtered by a term index

**Decision.**
- The list is windowed like the message list. *Revised during
  implementation:* `postio-ui::list::ListWindow` was to be generalised over
  its key type, and reading it closely showed it carries message-only
  meaning beyond the key — the thread a verb aims at (`ListRow::thread`,
  the blanket `postio_core::aim::RowFacts`), the selection model built on
  it. Bending that to hold people would have reached into `postio-core`
  for nothing the contacts list needs. So the contacts list has a small
  window of its own, `postio_ui::contacts::ContactsWindow` — pages,
  generation, LRU bound, abandon, evictions — and `postio-gtk`'s
  `ContactsModel` wraps it the way `MessageList` wraps `ListWindow`,
  keeping GTK's two rules (one object per position, never change while
  answering `item()`).
- Pages are read by keyset on `(sort_key, id)` — never `OFFSET` — the way
  `read_page` seeks with marks (`postio-runtime/src/store/local.rs:303`).
- `sort_key` is the person's displayed name, case-folded, maintained on
  write. The three views each have a covering index: default
  (`state, written, sort_key, id`), everyone (`state, sort_key, id`), deleted
  (the same index with `state = 'deleted'`).
- Typing to filter (FR-004) goes through a **`contact_terms` table**: one row
  per `(term, contact_id)` for each word of the user-set name, organisation,
  seen name, and each address's local part and domain. A filter is an index
  range `term >= ?1 AND term < ?1 || x'FF'`. The filtered view is capped
  (first 500 people by `sort_key`) with a "keep typing" row beyond it — typing
  is how a person narrows, and the cap is what keeps the rows a keystroke
  materialises bounded.

**Rationale.** Principle V gates budgets as counts. A `LIKE '%ada%'` over
people and addresses is a full scan (`counting::scans` fails it); a term range
is not. The same term index serves completion and the `@` finder's prefix.

**Alternatives considered.** The engine's `USING fts` index (rejected: a
second full-text surface for a few thousand short strings, and its tokeniser
does not split addresses the way people type them). Loading every person into
memory like the `@` finder does today (rejected for the screen: the list is
the thing that must never be loaded whole).

## R6 — "Show mail" needs a field the language does not have

The clarification chose written-out addresses over a person field. **The
query language is a flat conjunction** (`executor.rs:491`, `:963` join
conditions with `AND`; `postio-search` has no `OR`), so "from or to *any* of
these addresses" is not expressible today, and the finder currently writes
`from:<address>` (`finder.rs:392`) — which misses mail the user sent them.

**Decision: add one field, `with:`, meaning "from, to, cc or bcc is exactly
one of these addresses"**, taking a comma-separated list:
`with:ada@work.example,ada@home.example`. Resolved like `group:` — by exact
normalised address through `recipients` — not by full text. Picking a person
in the finder and "show mail" both write it; a one-address person writes
`with:ada@example.org`.

**Rationale.** It honours the clarification literally: the addresses are in
the query, visible and editable, and a pinned search does not follow later
joins. It is one `Field` row and one executor arm, which is how Principle III
expects the language to grow; `group:` is the precedent. A general `OR` would
be a much larger change to a parser whose flat shape the chips depend on
(`query.rs` module doc).

**Alternatives considered.** Several `from:`/`to:` tokens (rejected: ANDed,
they mean the opposite). A general boolean `OR` (rejected: out of proportion).
`contact:` resolved at search time (rejected by the maintainer, 2026-09-23).

## R7 — FR-032, the name in the list and the reader

**Decision:** substitute in **storage**, at the three read functions every
surface already goes through: `LIST_COLUMNS`' sender subquery
(`messages.rs:672`), `ThreadRepository::participants_for` (`threads.rs:1327`),
and `read_recipients` (`messages.rs:2611`). Each resolves a sender's name as
`coalesce(<owner's user-set name, if the owner is live>, recipients.name)`
through `addresses.contact_id`. `read_recipients` additionally returns the raw
header name beside it so the reader's header details can show the mail's own
words (FR-032's second clause).

**Rationale.** Substituting in the app mappers (`feed.rs::row`,
`reading.rs::Envelope`) would mean every surface — list, thread rows,
conversation rail, reader, notifications, the FFI list — re-learning the rule,
and a missed one is exactly the between-layers defect Principle IV warns of.
In storage it is one rule applied three times, and it is keyed by address
by construction, which is what defeats a forged display name.

**Cost:** one extra primary-key lookup per listed row (`addresses.id` →
`contacts.id`), no extra statement. The list's existing
`list_statement_count.rs` assertions must keep passing unchanged, and a new
one pins that renaming a person does not change the statement or row count of
a page.

**Known limit, stated rather than solved:** `from:Ada` is full text over the
sender column the index was built from (`executor.rs:1492`), so it matches
what the mail said, not a name the user set. `with:` is the way to find a
person's mail.

A rename emits a change event; the list and the open conversation refetch
their visible page (the ordinary refetch path), so the new name appears
without re-reading any message.

## R8 — Undo that restores exactly

**Decision:** each contacts mutation is a `Command` with its payload, handled
in `postio-session/src/actions.rs` like `map_mailbox_role` (`:1685`), whose
`Applied.inverse` is a command carrying the prior state explicitly:

| Command | Inverse |
|---|---|
| `JoinContacts { into, others, name, organization }` | `UnjoinContacts { into, restore: [(ContactId, [AddressId])], prior fields, added memberships }` — the absorbed people are `merged`, not deleted, so this is a state flip plus moving addresses back |
| `DetachAddress { address }` → new person | `MoveAddress { address, to: previous }` + discard the empty new person |
| `MoveAddress { address, to }` | `MoveAddress { address, to: previous }` |
| `EditContact { id, fields }` | `EditContact { id, prior fields }` |
| `DeleteContact { id }` / `RestoreContact { id }` | each other |
| `DeleteGroup { id }` | `RestoreGroup { id, name, uid, members }` |
| group commands | their obvious opposites |

Each is registered `Recovery::Undo` with a new `UndoKind` and toast text.
Import is not undoable (it only adds, and says what it did); it is not
destructive, so the registry's rule does not require it.

## R9 — vCard: adopt Pimalaya's `vcard-rs`, in a crate of its own

**The earlier decision (ADR 0007 Q4, hand-written, no dependency) is
superseded.** Its two premises no longer hold, which is why the constitution
says a Pimalaya survey more than a few weeks old is stale.

Survey, 2026-09-23:

- **`vcard-rs` 0.4.0** (lib `vcard`, <https://github.com/pimalaya/vcard>,
  2026-08-31, MIT OR Apache-2.0, `no_std`+alloc, no `unsafe`). Its stated
  purpose is a byte-faithful round trip: unknown properties decode to an
  `Unknown` value and are written back as they came, group prefixes
  (`item1.EMAIL`) are kept, line endings included. Reads 2.1, 3.0 and 4.0 in
  one model. The parser's only hard dependency is `memchr`; `base64`,
  `quoted_printable` and `encoding_rs` are optional features. No serde, chrono
  or tokio. Tests include RFC examples, proptest, and cross-checks against
  three other parsers. **It does not upgrade 3.0 to 4.0**; that step is
  Postio's (below).
- `io-addressbook` 0.0.1 — frozen, superseded. `io-vdir` — local storage, not
  needed. **`io-webdav` 0.3.0** is the CardDAV client for later (it depends on
  the `io-http` 0.5 Postio already uses) — recorded for the day CardDAV
  arrives, not used here.
- `calcard` 0.3.14 (Stalwart) re-serialises rather than round-tripping bytes,
  and pulls `mail-builder`, `jiff`, `serde_json`, `uuid`. `vcard4`, `ical`,
  `vcard` (magiclen), `vparser` are either stale, builders, or tokenisers.

**Decision.** Depend on `vcard-rs` pinned exactly (`=0.4.0`,
`default-features = false`, `parser` plus the encodings), from a **new leaf
crate `postio-vcard`** that maps between cards and Postio's contact model and
does the 3.0 → 4.0 upgrade. Only `postio-app` depends on it.

Why a new crate rather than `postio-model::vcard`: `postio-model` is the crate
the whole workspace waits on to compile (its boundary rule exists for that
reason), and 22,000 lines of parser used by one import command do not belong
on that path. The same boundary check pattern
(`scripts/checks/`) is extended so `postio-vcard` stays a leaf: no database
engine, no gtk4, no tokio.

**Storage:** the person keeps the **whole card as last imported** in
`contacts.vcard` (text), replacing `vcard_extra`. Export edits the properties
Postio models (`UID`, `FN`, `N`, `EMAIL`, `ORG`, `NOTE`, `REV`, `KIND`,
`MEMBER`) in place in that card and leaves every other byte alone; a person
with no card gets a fresh one.

**What "verbatim" means across versions (sharpens FR-051 and SC-006).**
From a 4.0 card, every unmodelled property is byte-identical on export. From
a 3.0 card, the export is 4.0, so the few 3.0 spellings 4.0 forbids are
rewritten and nothing else: the `VERSION` line, `ENCODING=b` binary values to
`data:` URIs, `TYPE=PREF` to `PREF=1`, and `CHARSET` parameters. The test
fixture exercises both.

**Mapping choices.** Several `EMAIL` lines → one person with several
addresses, `PREF`/`TYPE=pref` → preferred (FR-052). `KIND:group` + `MEMBER`
→ a group (FR-052). `CATEGORIES` is **not** mapped to groups — kept verbatim —
because the largest exporters use it for their own labels ("myContacts") and
mapping it would invent groups nobody made. An imported address that already
belongs to a person joins the card into that person (FR-053); a card whose
addresses belong to *several* existing people joins them, because the file is
the user's assertion that they are one person — the import summary lists
every such join. A card's `FN` fills the name only when the person has no
user-set name; otherwise the user's name stands and the summary counts it.

## R10 — Where the screen lives, and how it is reached

**Decision.**
- **A new pane occupant.** `ReaderOccupant::Contacts` joins the pane owner
  (`postio-gtk/src/shell.rs:79`). It takes the pane with the same
  `take_pane` / `release_pane` contract the composer uses
  (`composer.rs:2607`, `:2629`): remember `(context, focused pane)`, restore
  both on close, clear focus so a hidden entry cannot swallow keys. In
  `fallback()` it ranks above the composer while open; "compose to" closes it
  first, so the composer is never hidden behind it.
- **Inside the pane**, a list and a detail side by side when the pane is wide
  enough and stacked (list, then detail on `Enter`) when not, using the
  toolkit's breakpoint. The visual design follows `/gtk-design` and the
  canvas (which has no contacts artboard yet; the settings account list,
  `Design/screens/13-settings-accounts.png`, is the nearest list+detail).
  The design task renders it to PNG and checks it, per the skill.
- **`Context::Contacts`**, list-scoped, entered and left by a focus controller
  like `Context::Accounts` (`window.rs:3345`); appended to `Context::ALL`.
  Every exhaustive match it must join is listed in the plan.
- **The sidebar gets a "Contacts" row** below the saved searches, in a
  single-row list of its own — the saved-search section (`sidebar.rs:458`) is
  the precedent for a row that is neither a folder nor a view. ADR 0036
  governs folder and view rows; this is neither, and does not reopen it.
- **One context, dispatch on the focused row.** Rows in the pane have a
  kind — person, address, group, suggestion — and a command acts on the
  focused kind where it has a meaning, else does nothing and says which row it
  wants (contracts/commands.md). Sub-contexts per row kind were considered and
  rejected: `ContextSet` has six free bits, each context must join seven
  exhaustive matches, and the cheat sheet would split one surface into four
  headings.
- **`g c` opens Contacts** from anywhere (`ContextSet::ANY`, free today beside
  `g i`/`g d`/`g s`/`g t`/`g f`/`g a`).

## R11 — Join suggestions

**Decision.**
- **Shared display name:** each live person has a `name_key` (its displayed
  name, case-folded, whitespace-collapsed, indexed). Two live people with the
  same non-empty `name_key` and no dismissal between them are a suggestion.
  Computed on demand when the suggestions view opens — a self-join on an index,
  bounded.
- **Reply from another address:** recorded at sync time. When a newly
  recorded message's `In-Reply-To` parent was sent by the user to exactly one
  address X, and this message's `From` is Y ≠ X, the pair (X, Y) is written to
  `contact_join_candidates` with a count. Threading already resolves the
  parent in that transaction, so this adds a lookup only for replies to the
  user's own mail.
- **Dismissals** are stored per address pair (`contact_join_dismissals`, the
  lower id first). A suggestion between two people is hidden when any
  dismissed pair spans them — so a dismissal survives joins on either side
  (spec Story 4, scenario 2).

## R12 — ADR 0007 is folded, not amended

Per the Development Workflow, ADR 0007 is about this feature alone. What still
holds is in `spec.md`'s Context; what is superseded (address as identity, the
hand-written parser, `vcard_extra`, `suppressed`) is superseded by this spec
and this research. The ADR file is deleted in this branch, its row removed
from `docs/decisions/README.md`, and its 36 citations in code and docs
(`git grep 0007`) re-pointed to `specs/005-contacts`. `docs/PRODUCT.md`'s
"Out, deliberately" loses the contacts surface and vCard, and its §6 note
about the address book is updated. No new ADR is written: the one rule that
outlives the feature — an address has at most one owner — lives in the schema
and its comment.

## R13 — Schema change and existing stores

`schema.rs` has one `HEAD` and a fingerprint; any change makes an existing
store refuse to open with `SchemaFromAnotherBuild` and be rebuilt by resync
(`store.rs:557`). No production path ever created a user contact or group
(research above), so a rebuild loses nothing the user made; mail-derived
people are rebuilt from the mail.
