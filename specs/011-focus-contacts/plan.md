# Implementation Plan: Contacts for Postio Focus

**Branch**: `feature/focus-contacts` | **Date**: 2026-10-19 (revised to the
designer's handoff) | **Spec**: [spec.md](spec.md)

**Input**: `specs/011-focus-contacts/spec.md`, the handoff
`Design/focus-macos-contacts/` (never copied in), the maintainer's decisions of
2026-10-19 (spec M1 to M8).

## Summary

Contacts becomes a mode of Focus's main window (list plus an inline card), two
popovers reached with `i`, a form, a merge sheet and a settings pane, built for
the Mac first over a shared engine that Linux keeps building.

The handoff asks for more than 005's person model: every sender and recipient
is a contact; each has a **kind**, a **saved** state, **settings**, **stats**
and, last, **links** to Apple Contacts or CardDAV. So this feature, unlike its
first draft, **does change the schema**, on top of what the 005 port provides.
Behaviour stays in `postio-focus` (ADR 0045); the Mac draws.

The plan has three parts, in this order: what the code says today (the four
questions the handoff's `PROMPT.md` asks first), the engine design, and the
slices. Open questions are in the spec; nothing is built until the maintainer
OKs this.

## 1. What the code says today

Read on `origin/main` at `98cf219b` (2026-10-19). The 005 port
(`feature/contacts-mac`) is **not pushed** and holds, past `main`, the 005
specs, a `postio-vcard` leaf crate and an `AddressId` type
(`crates/postio-model/src/ids.rs`): no schema and no repository yet. What it
will provide is read from its `data-model.md`.

### 1.1 How senders and recipients are stored

On `main`, two layers:

- **The mail's own evidence.** `addresses` (one row per normalised address,
  `crates/postio-storage/src/schema.rs:365`, unique index at `:1112`) and
  `recipients` (`schema.rs:983`: one row per header occurrence, with
  `kind in ('from','sender','reply_to','to','cc','bcc')`, the per-message display
  `name`, and `address_id`; indexed `(address_id, kind)` at `:1254`). A message
  row also carries the header facts a kind needs: `list_id` (`:790`),
  `unsubscribe_offered` and the `automation` bitmask (`:803-804`).
- **A flat contact per address.** `contacts` (`schema.rs:415`) is **one row per
  address** (`account_id`, `address`, `address_normalized`, `name` = a name the
  user set, `source` mail/user/import, `suppressed`, `uid`, `vcard_extra`,
  `times_seen`, `last_seen_at`), written by `ContactRepository::record_message`
  (`repository/contacts.rs:62`) at sync. Identity is the normalised address
  (`contacts.rs:1-12`). Groups exist: `contact_groups` (`schema.rs:406`) and
  `contact_group_members` (`:400`). `correspondents` (`:443`) counts who the user
  wrote to. There is no kind, no saved state, no per-contact settings, no
  stats, no external link.
- **`group:` is already a search filter**, resolved through
  `contact_group_members` (`postio-index/src/executor.rs:2348`).

**On the port branch** (its `specs/005-contacts/data-model.md`): `contacts` becomes a
*person* (`state live|deleted|merged`, `name`, `organization`, `note`,
`preferred_address`, `seen_name`, `sort_key`, `name_key`, aggregates, `uid`,
`vcard`); `addresses` gains `contact_id`; new `contact_sightings`,
`contact_terms`, `contact_join_candidates` and `contact_join_dismissals`;
groups lose `account_id`. 005 R13 would rebuild an existing store by resync.

### 1.2 Where per-sender settings live, and how they are keyed

There is **no table for them**. Each setting has its own home and key:

| Setting | Home | Keyed by |
|---|---|---|
| Digest rule | `[[focus.digests]]` in `config.toml` (`postio-config/src/focus.rs:288`, `DigestRule`; `applicable_digests` `:454`) | **A query**, not an address: "like this" offers `from:<address>`, `from:<domain>` and the list (`postio-host/src/focus/rules.rs:351`, `:355`) |
| Never filter ("pinned", or restored with `R`) | `[focus.filter] never` (`focus.rs:169`, `never_filters` `:245`) | An address or a whole domain (`@example.org`) |
| Stopped marker kinds | `[focus.filter] stop_markers` (`focus.rs:191`) | An address or domain, plus `question` or `todo` |
| HTML "always for this sender" (the rendering override) | A `[Treatment]` section of the **remote-image allowlist file** (`postio-ui/src/allowlist.rs:30-35`, `treatment_for` `:298`, `set_treatment` `:305`) | The sender's address. Read by the Mac through `postio-ffi/src/focus_reader.rs:205,292` and by the GTK reader **directly** (`postio-widgets/src/reader/view.rs:800`) |
| Remote images "always from this sender" | The same file's `[addresses]`/`[domains]` (`allowlist.rs`) | Address or domain; fail-closed by design (a privacy rule) |
| Remind if no reply | `reminders` table (`schema.rs:1018`) | **A conversation** (`thread_id`) and an anchor message. There is **no per-sender default** |
| Notify when they write | Nothing per sender. Notifications are by folder role (`[sync] notify_roles`, `docs/ARCHITECTURE.md:635`; `postio-host/src/notify.rs`) | n/a |
| Filter decisions | `filter_decisions` (`schema.rs:601`), per message, layers `header\|senders\|server\|model` | A message |

So four of the handoff's settings are new (notify, remind default, archive
after N days, notes), "Filtered out" per contact has no counterpart today (the
user can pin to the inbox with `R` or `never`, but cannot force filtering), and
the three that exist are keyed by **address or domain or query**, in files the
GTK app reads. That is the main migration risk (section 4).

### 1.3 How search resolves `from:`

`from:ada` is not contact-aware. It is a phrase match on the **`sender` column
of the content-keyed `search_documents` full-text index**
(`postio-index/src/index.rs:1121`): `Filter::From(value) =>
fts_column_condition("sender", value)` (`executor.rs:2129`; as a set
`:2380`; the helper `:2312-2345`). The column holds the header's name and
address text, so `from:ada` finds mail whose sender text contains "ada" and
never finds a second address of the same human under another display name. 005
adds `with:` (exact addresses through `recipients.address_id`,
`contracts/query-with.md`) for "all mail with this contact". `Filter::AnyOf`
(D26, `executor.rs:2367`) already unions sets, so "All mail" can be an `AnyOf` of
`from:` over a contact's addresses with no executor change. The handoff's
"`from:ada` matches all her addresses" needs a contact-aware expansion in the
query compiler; it is an **opt-in executor option** so Linux's results do not
change until it adopts (section 9).

### 1.4 Does anything already talk to CNContactStore or CardDAV?

- **`CNContactStore`: yes, read only, for completion.** `ContactsSource.swift`
  (`macos/Sources/PostioKit/ContactsSource.swift:16-110`): `SystemContactBook`
  fetches full name and email addresses by name predicate; authorization is
  requested **on the first keystroke in a recipient field** and a refusal is
  never asked again (spec 009 T077-T078, `specs/009-focus-macos/tasks.md:439-442`;
  `NSContactsUsageDescription` in `Info.plist`). What it reads is "never stored
  or logged"; it crosses to Rust only as `extra` on
  `recipient_suggestions` (`postio-ffi/src/contacts.rs:56`). **This is FR-051's
  completion**, and it conflicts with M7 (the prompt only when sync is turned on):
  see Q3.
- **CardDAV: nothing.** The only mentions are ADR 0007 (`uid` is "also
  CardDAV's key", "left possible rather than designed",
  `docs/decisions/0007-address-book.md:54,223,232`) and `docs/field-report.md`.
  `main`'s `contacts.uid` and `vcard_extra` columns were put there for it.
- No existing code writes to any external address book.

## 2. What this needs from the 005 port, and what this adds

The port is the base; coordinate on its PRs (comments, not edits to its branch).
Asks of the port, each cheap now and expensive after it lands:

| Ask | Why |
|---|---|
| Write `contacts` as a real **migration** (INSERT…SELECT from the old rows, keeping `name`, `source`, `suppressed`) rather than R13's rebuild by resync | The catch-up in section 4 and SC-005 need the old rows |
| Keep `addresses.contact_id` nullable and **assign lazily** | The migration cannot create 200,000 rows inside the startup transaction (section 4) |
| `contact_sightings` carries `times_seen`, `last_seen_at`, `times_written` per `(address, account)` | Stats section 6 builds on them rather than a parallel table |
| Keep 005's command ids; "join" in code may stay, the word on screen is Merge | Ids are a file format; words come from `postio-ui` |
| `contacts.preferred_address` stays the column; the word on screen is "primary" | Same |

What this feature adds on top (shape, not final SQL; `schema.rs` `HEAD` is the
one home, with the `tests/schemas/<fingerprint>.sql` snapshot and a
`Migration`, as `schema.rs`'s header requires):

```sql
-- contacts (005's person) gains
kind            TEXT NOT NULL DEFAULT 'person'
                CHECK (kind IN ('person','company','list','newsletter','automated')),
kind_source     TEXT NOT NULL DEFAULT 'detected' CHECK (kind_source IN ('detected','user')),
kind_version    INTEGER NOT NULL DEFAULT 0,          -- the detector that decided it
saved_at        INTEGER,                              -- NULL = not saved (H3)
ignored_at      INTEGER,                              -- "Ignore sender"
given_name      TEXT,  family_name TEXT,
show_as         TEXT NOT NULL DEFAULT 'full' CHECK (show_as IN ('full','first','last_first','custom')),
title           TEXT,                                 -- organisation is 005's `organization`
photo_blob_id   TEXT,                                 -- blob store; local only
note_path       TEXT,                                 -- linked vault note, relative
sync_rev        INTEGER NOT NULL DEFAULT 0            -- bumped only when a synced field changes
-- indexes: (state, kind, sort_key, id)  tab lists
--          (saved_at, kind, written)    "frequent correspondents aren't saved"

-- addresses gains
label           TEXT CHECK (label IN ('work','personal','old')),
bounced_at      INTEGER                                -- a delivery failure naming it

CREATE TABLE contact_settings (                       -- H7, stored once
  contact_id         INTEGER PRIMARY KEY REFERENCES contacts(id) ON DELETE CASCADE,
  arrival            TEXT CHECK (arrival IN ('inbox','digest','filtered')),  -- NULL = default rules
  digest_cadence     TEXT, digest_day TEXT, digest_at TEXT,                  -- as DigestRule's due
  stop_markers       INTEGER NOT NULL DEFAULT 0,                              -- bitmask: question, todo
  notify             INTEGER NOT NULL DEFAULT 0,
  html_original      INTEGER NOT NULL DEFAULT 0,        -- written through to the allowlist file until S13 (Q1)
  remind_days        INTEGER,                           -- NULL = off
  archive_after_days INTEGER                            -- newsletters
);

CREATE TABLE contact_details (                         -- phones, websites
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('phone','url')), label TEXT, value TEXT NOT NULL,
  source TEXT NOT NULL CHECK (source IN ('user','signature','sync')), added_at INTEGER NOT NULL
);

-- the maintained evidence (section 6): per ADDRESS, so merge and split rewrite nothing
CREATE TABLE address_stats (
  address_id PRIMARY KEY REFERENCES addresses(id) ON DELETE CASCADE,
  total INTEGER NOT NULL DEFAULT 0, sent_to INTEGER NOT NULL DEFAULT 0,
  seen_count INTEGER NOT NULL DEFAULT 0,               -- read messages
  last_at INTEGER,
  with_list_id INTEGER NOT NULL DEFAULT 0, with_list_post INTEGER NOT NULL DEFAULT 0,
  with_unsub INTEGER NOT NULL DEFAULT 0, with_bulk INTEGER NOT NULL DEFAULT 0,
  with_auto_submitted INTEGER NOT NULL DEFAULT 0,      -- kind signals
  reply_median_secs INTEGER, reply_samples INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE address_months  (address_id, month, n,  PRIMARY KEY (address_id, month));
CREATE TABLE address_recent  (address_id, message_id, at, direction, PRIMARY KEY (address_id, message_id));
                                                       -- ring of the 50 newest per address
CREATE TABLE contact_counts  (kind PRIMARY KEY, total, saved);   -- tab counts, trigger-maintained

-- review queue (005 supplies join candidates and dismissals)
-- bounces and "frequent, unsaved" are queries over addresses.bounced_at and contacts

-- sync (section 8, S12 only)
CREATE TABLE contact_sync_accounts (...);  CREATE TABLE contact_external_links (...);
CREATE TABLE contact_sync_conflicts (...);
```

`filter_decisions.layer` gains `'contact'` so a contact set to Filtered out is
recorded as the person's own word, as `restored_at` records `R`.

## 3. Design by crate

| Crate | Gains |
|---|---|
| `postio-model` | `ContactKind`, `Saved`, `ShowAs`, `Arrival`, `ContactSettings`, `Detail`, `display_name()` (one function: name, show-as style, seen name, address); the sort key rule |
| `postio-classify` | `kind::detect(&AddressSignals) -> (ContactKind, Confidence)` as **pure functions with rules as data** (a table, as `Senders` is; "Providers are data" applies in spirit); `signature::parse(&str) -> Details` (title, company, phone, website); `bounce::detect(headers, body)` (DSN); `duplicates::score(a, b)` and its `Evidence`. A leaf: no I/O, no network, cannot send (ADR 0009's boundary) |
| `postio-storage` | the schema above; repositories (`contact_settings`, `contact_details`, `address_stats`, review reads); stats maintained in the message-insert transaction beside `contact_sightings`; the catch-up reads. Every read path gets a `counting` assertion and a plan assertion naming its bound columns |
| `postio-config` | reading digest/`never`/`stop_markers` entries that the migration moves; the migration's file rewrite is the one write to `config.toml` it makes |
| `postio-host` | the settings engine (applies `arrival` to the digest, filtering and notify paths; Section 5); the catch-up job; review-queue reads; sync runner (S12) |
| `postio-core` | `Context::Contacts`, `ContactEdit`, `ContactPopover`, `ContactSheet` added to every exhaustive match (`postio-ui/src/keymap.rs`, `cheatsheet.rs::heading`, `postio-ffi/src/registry.rs`, `keybindings_doc.rs::where_available`, `postio-ui/src/focus.rs`); the command rows; `offered_on` hides them on Freedesktop; `add_account` loses `Contacts` |
| `postio-ui` | words (the empty states, toasts, role lines, "Mail from X arrives in …", tag pills, stat captions, the frequency phrase); `contacts_words`; the card's and popover's view-model *shapes* (rows of text, no toolkit); `SenderTreatments` interface over the allowlist file |
| `postio-focus` | `contacts.rs` (mode, tab, filter, cursor, selection, History, Esc ladder), `contact_card.rs` (card state, edit state, arming "Esc again"), `contact_popover.rs`, `contact_merge.rs` (sheet state, candidate order, kept record); `Policy.caps.contacts_view`; intents and requests (`contracts/focus-contacts.md`) |
| `postio-ffi` | `focus_contacts.rs` (reads, events, exports), `ExternalAddressBook` foreign trait (S12), `INTERCEPTED` gains the panels (`contacts_import`, `contacts_export`) |
| `postio-vcard` (the port's) | consumed by import/export and by CardDAV |
| `postio-contact-sync` (new pure leaf, S12) | matching, field ownership, three-way diff, plan, conflicts: no I/O |

**Swift**

| Target | Gains |
|---|---|
| `PostioKit` (no AppKit) | `ContactsModel` (rows, tabs, strip counts), `ContactCardModel`, `ContactPopoverModel`, `ContactEditModel`, `MergeSheetModel`, `ContactsSettingsModel`, `ContactsIntents` (cases of `FocusIntents`), the Apple adapter `AppleAddressBook` (S12; the one file that imports `Contacts`, as `ContactsSource.swift` does) |
| `PostioAppKit` | `ContactsView` (the list table over `FocusListTable`'s machinery; the card in SwiftUI, hosted), `ContactAvatar`, `ContactPopover` (`NSPopover`), `ContactEditForm`, `MergeSheet` (`FocusSheet`), `ContactsSettingsPane`, `ContactsPanels` for import/export (`POSTIO-CONSENT:` comments), menu bar |
| `Postio` | `MainWindow` hosts the mode; `Engine` carries events |

Neutral rule unchanged: Swift draws; every rule it would otherwise hold is a
function in `postio-ui`, `postio-classify` or `postio-focus`.

### 3.1 Shared code first (maintainer, 2026-10-19)

The maintainer asked that as much of Contacts as possible be one code
path for Linux and the Mac. This tightens the neutral rule:

- **View models are Rust.** Each surface's rows, tabs, strip counts, role
  line, tag pills, stat tiles and captions, the 12-month strip's bars and
  axis labels, open-item rows, file chips, the edit form's fields and
  validation, and the merge sheet's candidates, evidence and result
  preview are built in `postio-ui` (shapes and words) and `postio-focus`
  (state). `postio-ffi` exports them whole. The PostioKit models
  (`ContactsModel`, `ContactCardModel` and the rest) only adapt FFI records
  to SwiftUI/AppKit: no sorting, grouping, formatting, counting or
  enabling logic in Swift. A Swift test fails if a model computes
  something an FFI record already carries.
- **Every key, verb, intent and Esc ladder goes through `postio-focus`**,
  as spec 009's slices do. The Mac sends `Input`s and draws `Intent`s.
- **Avatar initials, the palette hash and kind glyphs are one Rust
  function** (`postio-ui`). Each toolkit only paints the circle or the
  rounded square.
- **Sync logic is Rust, and CardDAV is Rust, so Linux gets CardDAV sync
  from the same code.** The only platform code is the `ExternalAddressBook`
  adapter over `CNContactStore` (Swift) and, at Linux adoption, the file
  chooser for import and export.
- **No Apple-only engine paths.** `Policy.caps.contacts_view` hides the
  surface on Linux only until GTK draws it. Contact-aware `from:`, kind
  ranking in completion and the per-contact settings engine behave the
  same on both platforms once enabled, and each is tested once, in Rust.
- **The HTML treatment moves into `contact_settings` in S3**, not S13. The
  GTK reader reads the shared `SenderTreatments` interface, verified on
  the Linux box (the maintainer's offer for Linux-side work), so there is
  one home from the start rather than a write-through.
- **Every UI slice (S6 to S10) ships a controller test in
  `crates/postio-focus/tests/`** that walks the slice's keys against a
  fake list, so GTK adoption (S13) is drawing only: no rule is written
  twice. Where a slice's Rust view model is complete, the GTK drawing may
  land in the same slice, verified on the Linux box, at most two agents
  there at once.

## 4. The migration

Goals: create contacts from existing mail; move per-sender settings onto
contacts; **change no behaviour** (SC-005); never block startup (< 500 ms); leave
GTK building and showing what it showed.

**4.1 The store step (fast, DDL and cheap copies only).** One `Migration` from
the previous fingerprint: create the tables and indexes above; add the columns
with defaults (`kind='person'`, `saved_at NULL`, `kind_version 0`); keep the old
`contacts` rows through the port's own step (`name`, `source`, `suppressed`,
`uid`, `vcard_extra`); set `saved_at` for `source in ('user','import')` rows
(the user asked for those, 005 FR-005). It runs in the open transaction, so it
copies no row per message.

**4.2 The catch-up (background, resumable, newest first).** As Focus's own
classifier catch-up does (`focus_classified`, FR-141): a job in `postio-host`
that walks `addresses WHERE contact_id IS NULL` by keyset in batches of 500 per
transaction, for each address: create the contact (name left NULL, `seen_name`
from the latest `recipients.name`), fold its `recipients` and `messages` rows into
`address_stats`, `address_months`, `address_recent` and the kind signals, then run
`kind::detect`. Progress is a `settings` row (`contacts.catchup`); the footer
shows "Building contacts, n of m". Meanwhile every reader falls back as today: an
address with no contact reads its header name, and **`i` or the card on one
creates its contact on demand**, so nothing waits on the job. New mail is
handled at insert, in the same transaction as `contact_sightings`, so the job and
live sync never race (the job skips addresses that already have a contact).
Reply-time samples are computed in the job from `thread_id` order (the one
expensive statement per address; bounded by the 50-message ring).

**4.3 Settings, without changing behaviour** (Q1 is the question; this is the
recommended answer, A):

- **Moved, one-shot, in the host on first run after the store step**, each
  with the old entry kept in `config.toml.pre-contacts` (written beside the file
  the first time) so a person can see what moved:
  - `[[focus.digests]]` rules whose `match` is exactly one `from:<address>`
    query → `contact_settings(arrival='digest', cadence, day, at)` on that
    address's contact. The digest engine then reads **both**: config rules as
    before, and contact-derived rules synthesised as `from:` queries per address,
    so `postio-index`'s digest matcher and `postio_ui::schedule::next_due` are
    unchanged. The `g r` rules page lists both, the contact's name on the
    derived rows.
  - `[focus.filter] never` entries that are **addresses** →
    `arrival='inbox'` (pinned). Domain entries stay in the file (a domain is not
    a contact).
  - `stop_markers` address entries → the `stop_markers` bitmask. Domain entries
    stay.
- **Not moved**: rules with other queries (`list:`, domains, `subject:`,
  several addresses), the remote-image allowlist (a fail-closed privacy rule; a
  second home for it is the "two answers" risk its own header names), and the
  **HTML treatment**: the card's "Always show HTML as sent" switch is written
  *through* to the allowlist file's `[Treatment]` per address (`SenderTreatments`
  in `postio-ui`), because the GTK reader reads that file directly
  (`view.rs:800`). It moves into `contact_settings.html_original` in the slice
  that adopts Contacts on Linux (S13), together with a host call for the GTK
  reader.
- **Behaviour proof (SC-005)**: a fixture store plus a `config.toml` holding one
  of each entry above, run through the *old* decision functions and the *new*
  ones over the same corpus of messages; the digest holds, filter decisions, pin
  and marker checks and treatment answers must be equal. This test is written red
  first (it fails until the engine reads `contact_settings`).
- **A merge of contacts with different arrivals** keeps the kept record's
  settings; the other's per-address digest rule (if it was derived) is dropped
  with the preview naming it (Q8). Rules that stayed in the file are
  address-keyed and keep applying either way.
- **New "Filtered out"** writes `contact_settings.arrival='filtered'`; the
  filing pass consults it as a layer before the classifier and records
  `filter_decisions.layer='contact'` with the reason the classifier would
  have given (default `notification` for automated, `promotion` for
  newsletters and lists), so Filtered's tabs and `R` work unchanged.

**4.4 Does it change how Linux shows senders?** See section 9. In short: the
store step and catch-up change nothing visible. What can: a contact whose `name`
was already set (only `source user` rows), 005's name substitution (it replaces
the header name wherever `contacts.name` is set), person-shaped completion, and,
from S3, where digest and pin rules are read from. **The maintainer is told
before S3 and S4 merge.**

## 5. Kind detection

One pure function in `postio-classify`: `detect(signals: &AddressSignals) -> Kind`.

- **Inputs**, accumulated per address at insert (`address_stats` columns), so
  detection never rereads mail: counts of messages with `List-Id`, `List-Post`,
  `List-Unsubscribe` (+ `List-Unsubscribe-Post`), `Precedence: bulk|list|junk`,
  `Auto-Submitted ≠ no`, `auto-replied`; the local part and domain; the display
  name; the number of distinct person-like addresses at the same non-freemail
  domain among contacts. The message columns exist already (`list_id`,
  `unsubscribe_offered`, `automation` bits: 1 bulk, 2 list, 4 junk, 8
  auto-generated, 16 auto-replied, `schema.rs:790-804`), NULL until the headers
  are known, so a NULL is "no evidence", never "no".
- **Order, first match wins**, and a user's kind (`kind_source='user'`) always
  wins:
  1. `List-Post`, or `List-Id` without newsletter evidence → **List**;
  2. `List-Unsubscribe` plus bulk evidence (`Precedence: bulk`, a one-click
     unsubscribe, `List-Id` from a sending service, or a bulk display-name
     pattern) → **Newsletter**;
  3. `Precedence: auto_reply|junk`, `Auto-Submitted`, or a role local part that
     is a robot (`no-reply`, `noreply`, `donotreply`, `notifications`, `mailer-daemon`,
     `postmaster`, `bounce*`) → **Automated**;
  4. a role local part (`info`, `office`, `hello`, `contact`, `team`, `sales`,
     `support`, `billing`, `accounts`) on a non-freemail domain with two or more
     person contacts → **Company**;
  5. otherwise **Person**.
- **Precision before recall**: a person's correspondence must never read as
  Automated; a message with a human `In-Reply-To` chain or a reply from the
  user to the address floors the kind at Person unless List or Newsletter. A
  later message that no longer looks bulk does not flip a kind detected from
  many; confidence is the signal ratio and needs a margin to change.
- **Data, not code**: the role-local-part list, newsletter patterns and
  freemail domains are tables in `postio-classify/src/senders`-style data files.
- **Versioned**: `kind_version` lets a better detector re-decide every
  `kind_source='detected'` row in the catch-up. A merged contact's kind is the
  kept record's (Q11).
- **Tests**: a corpus of header sets (`postio-classify/tests/kind_corpus/*.toml`,
  one per sender, invented, `example.*` domains, hand-labelled) with SC-003's
  95% floor and a "never Automated for a person" assertion; a few real-shaped
  `.eml` added through `/add-fixture` for the header edge cases (a list that
  also replies, a newsletter with `List-Post`).

## 6. Stats, within 50 ms and the counting budgets

The rule from ADR 0040 (and `docs/notes/…an-aggregate-hides-from-a-row-count`):
a per-page aggregate is a **maintained column** or a **range over an index whose
every leading column is bound**, gated by an `EXPLAIN QUERY PLAN` assertion that
names its columns, because `counting::scans` alone cannot see a one-column
`SEARCH` walking a wide index.

- **Per address, never per contact**, so merge, split and move rewrite nothing:
  a contact's numbers are `SUM`/`MAX` over its (one to ten) addresses, one
  statement each, all by `address_id IN (…)` on primary keys.
- **Tiles** (`messages`, `from you`, `last`): from `address_stats`, maintained in
  the message-insert transaction with the sightings' double-count rule (a message
  with an address in To and Cc counts once). **Replies in**: `reply_median_secs`,
  recomputed from a small sample table only when a reply lands (the ring holds the
  last 25). **Opened**: `seen_count`, kept in step by the write that flips
  `messages.seen`, the way `mailboxes` counts are (ADR 0040).
- **The 12-month strip**: `address_months`, one row per `(address, month)`; the
  card reads `WHERE address_id IN (…) AND month >= ?` and sums by month.
- **Open between you and Recent issues/files**: from `address_recent`, the ring
  of the 50 newest `(message_id, at, direction)` per address: markers by
  `idx_markers_dismissed (dismissed_at, message_id)` over the ring's ids;
  reminders by a new `idx_reminders_anchor (anchor_message_id)` over the ring's
  *sent* ids; attachments by `idx_attachments_message`; "frequency" and "last
  issue" from the ring's dates. Bounded by 50 ids per address.
- **Tab counts and the footer** ("1,533 contacts · 640 saved"): `contact_counts`,
  kept by triggers on `contacts` (the ADR 0040 rule for per-page counts).
- **The list**: keyset pages over `(state, kind, sort_key, id)`; the filter over
  005's `contact_terms`.
- **Budgets** (written red first, in the storage suite, `counting`):
  open card ≤ 7 statements, no `SCAN`, rows proportional to the contact's
  addresses and not to its mail (asserted at two store sizes, the `startup_reads`
  shape); first list page unchanged by kind-tab count and by 20,000 vs 200
  contacts; filter keystroke one statement; popover ≤ 4 statements; message insert
  adds a fixed number of statements per distinct address (so a first sync's
  per-row cost, the #1587 number, is measured on `insert_cost_curve` before
  S5 lands, with the fts column as the ceiling).

## 7. The review queue and signature parsing

- **Duplicates**: 005's candidates (replies between addresses, same name key)
  plus `duplicates::score` adding the handoff's evidence: the same signature,
  an address that stopped when another began in the same threads, forwarding
  headers (`Resent-From`). The sheet lists `Evidence` lines from the scorer; no
  evidence line, no candidate. `n` writes 005's dismissals.
- **Bounced**: `bounce::detect` over `multipart/report; report-type=delivery-status`
  and a `5.x.x` status naming the failed recipient sets `addresses.bounced_at`.
- **Frequent, unsaved**: `contacts WHERE saved_at IS NULL AND ignored_at IS NULL
  AND kind='person' AND written >= 3`, on the `(saved_at, kind, written)` index.
- **Signature parsing** (`signature::parse`): pure, local, over the plain text of
  the sender's recent messages' last block; a small set of patterns (title ·
  company line, phone number with international or national shape, `https://`
  site); returns candidates with where they were seen; **stores nothing**;
  the popover shows them, "Added only if you save".

## 8. Sync design (the last step)

**What is Rust and what is Swift.** Rust owns every decision and all state;
Swift only moves bytes to and from `CNContactStore`.

- **Rust**: matching, field ownership, three-way diff, plan, conflicts, the link
  tables, scheduling, the dry-run, the gate, CardDAV (transport and sync tokens),
  vCard mapping (`postio-vcard`).
- **Swift**: `AppleAddressBook`, one class over `CNContactStore` implementing the
  UniFFI foreign trait `ExternalAddressBook { authorization, request_access,
  containers, snapshot(container, since) -> [ExternalRecord], apply(plan) }`.
  Notifications (`CNContactStoreDidChange`) are pushed down through a setter, as
  `NWPathMonitor` is (`macos/CLAUDE.md`). `request_access` is called **only** by
  the Rust runner when the person enables the account.

**Tables** (S12): `contact_sync_accounts` (kind apple|carddav, mode two_way|
read_only, scope, container or URL, `writes` always 0 until the gate, token or
ctag, last synced; CardDAV credentials in the OS keyring only);
`contact_external_links (contact_id, account_id, external_id, etag, base JSON,
synced_at)`; `contact_sync_conflicts`. `base` is the synced fields as of the
last sync, which is what makes a three-way merge possible.

**Matching** (`postio-contact-sync`): by any shared normalised email; then by
name plus phone (digits only, compared on the trailing nine). More than one
candidate on either side → no automatic link; the pair is queued in the review
queue as an ambiguity. A match creates a link and never a contact (Q7); an
unmatched Postio contact is a *create* in the plan (two-way only, writes
gated).

**Field ownership**: synced: given and family name, photo, emails, phones,
organisation, title; Postio-only: kind, `show_as`, settings, notes, stats, Postio
groups (mapped to external groups for linked contacts when the target has them,
by name). Notes are not synced; this also avoids the Contacts notes entitlement.

**Conflicts**: per synced field, `base` vs mine vs theirs: only one changed →
take it; both changed to different values → a conflict row, Keep mine / Keep
theirs; nothing is written for that field meanwhile. An address list merges as a
set.

**Deletes**: gone externally (and the link was live) → `saved_at` cleared, link
marked gone, card and stats kept. Deleted in Postio → if linked, the sheet asks
"Delete from Contacts too?"; "no" unlinks, "yes" puts a delete in the plan.

**Safety (M7)**: (1) **Read only first**: `apply` is never called while
`writes=0`, and the Swift adapter's `apply` throws `.notEnabled` independently of
Rust (two keys, as the privacy checks do). (2) **The gate is a reviewed commit**:
the maintainer reviews the dry-run output, then a one-line commit flips
`CONTACT_SYNC_WRITES` from `false`; until then the Settings control reads "Read
only until writes are reviewed". (3) **Dry-run**: the planner returns `Plan
{ creates, updates, deletes, conflicts, ambiguous }`; the runner stores counts
and hands the *report* (which contains names) to the Settings sheet or to a file
the person asked for; **it is never logged** (logs carry ids and counts, CLAUDE.md
privacy). (4) **Tests**: `postio-contact-sync` has no I/O and is tested over fake
snapshots in both directions, with conflicts and deletes; the Rust runner and the
Swift adapter are tested against in-memory fakes of the foreign trait; the real
store is touched only by a hand-run `#[ignore]` test that refuses to start unless
`POSTIO_SCRATCH_CONTACTS_CONTAINER` names a local container the maintainer made
for this; it never enumerates any other container.
(5) **Permission**: first requested when the account is enabled; denied is a
shown, final state until the person re-enables.

**CardDAV**: "Pimalaya first" (CLAUDE.md): before any wire code, check
whether a Pimalaya CardDAV or address-book I/O crate exists and fits (the way
`io-imap` and `io-smtp` were used); otherwise RFC 6352 over the existing
transport with RFC 6764 discovery, `sync-token` (RFC 6578) where offered and
ctag/etag otherwise, vCard 3.0/4.0 through `postio-vcard`. Each connection
writes an `egress_log` row (the person's consent is adding the account). The
`io` or `Contacts` dependency is a recorded decision in the slice's commit.

**Settings › Contacts** (`08-settings-sync.png`) shows per row: matched count,
"waiting" (saved, in scope, `sync_rev` ahead of the link) and last synced. The
four Suggestions switches are engine settings read by the catch-up, the popover
and `save-on-write`.

## 9. Keys: collisions and resolutions

The full table is in the spec ("Keys"); the summary, 2026-10-19:

| Handoff | Collides with | Resolution |
|---|---|---|
| `g c` | `next_scope` `g c` (Linux; not offered on Apple) | `g c` for Contacts on Apple; `next_scope` → `g n` at Linux adoption |
| `g a` | **Go to archive** `g a` | Review is **`g w`** |
| `i` | `i i`, `i a` (show images) | images → `alt+i`, `alt+I` (Linux-visible; Q2), or the popover on `@` |
| `U` | `toggle_read` alt `U` (other contexts); Unsubscribe is `M` | Contacts-only `contact_unsubscribe` on `U` |
| `x` | none | `toggle_selection` gains `Contacts` |
| `m` | `set_default_account` `m` (Accounts only); Move is `v` | none |
| `⌘⇧N` | **`add_account` `mod+shift+n`, every context** | `add_account` excludes `Contacts` |
| `⌘E`, `⌘S`, `⌘↩`, `⌘⌫`, `⌘I`, `n`, `c`, `↩`, `/`, `d`, `⌘Z` | other contexts only | none |

## 10. Slices

Each is runnable, committed on `feature/focus-contacts` with storyboards and red
tests first, and each engine slice is **additive** so `postio-gtk` keeps
building (CI is the proof; this Mac cannot build GTK; see the notes "Moving code
out of a crate you cannot compile" and "The FFI has a second caller you cannot
compile"). The handoff's seven steps map as marked.

| Slice | Delivers | Handoff step | postio-focus | postio-ffi | PostioKit | PostioAppKit |
|---|---|---|---|---|---|---|
| S0 | Gate: confirm section 2 with the port; storyboards `contacts/` red; demo seed `small:30` with invented contacts | | | | | |
| S1 | Schema, migration, lazy contact creation, catch-up skeleton, `display_name` | 1 | | `contact_for_address` | | |
| S2 | `kind::detect`, signals at insert, the header corpus, SC-003 | 1 | | kind in rows | | |
| S3 | `contact_settings`; the settings engine; the one-shot move of address-keyed digest, `never`, `stop_markers`, and the per-address HTML treatment; `SenderTreatments`, read by the GTK reader too (verified on the Linux box); SC-005's fixture. **ADR**: per-contact settings are the one home of sender-keyed settings. **Linux-visible: tell the maintainer before merging** | 1 | | settings reads and writes | | |
| S4 | Resolution everywhere: Show-as name in list, conversation and reader; All mail as an `AnyOf` of `from:`; the opt-in contact-aware `from:`/`to:`/`from:@Group`; person-shaped completion (still with 009's system read) | 1 | | `recipient_suggestions` shape | | |
| S5 | `address_stats`, months, ring, counts, budgets; `insert_cost_curve` measured | 1 | | stats read | | |
| S6 | **Popover, known sender** (04): `i`, click, arrival control with undo | 2 | `contact_popover.rs`, `Context::ContactPopover`, intents | popover read, arrival write, `UiEvent`s | `ContactPopoverModel` | `ContactPopover` (`NSPopover`) |
| S7 | **Contacts view** (01, 02, 03): mode, kind tabs, filter field, list, card with stats, open items, files; `g c`; Worth a look hidden (queue empty) | 3 | `contacts.rs`, `contact_card.rs`, `contacts_view` | list pages, counts, card read | `ContactsModel`, `ContactCardModel` | `ContactsView`, `ContactAvatar` |
| S8 | **Edit and new contact** (06): the form, Save/Cancel, arming Esc, kinds, show-as, work, addresses, details, groups as chips; `to:Group`, `from:@Group` | 4 | `ContactEdit` context and state | save as one command | `ContactEditModel` | `ContactEditForm` |
| S9 | **New-sender popover and signature parsing** (05): `signature::parse`, Add contact, Not now, Ignore | 5 | popover mode | details read, add, ignore | | `ContactPopover` variant |
| S10 | **Review queue, merge and split** (07): `duplicates::score`, `bounce::detect`, Worth a look, `g w`, the sheet, `m`, `n`, split from the card | 6 | `contact_merge.rs`, `ContactSheet` | candidates, merge, split | `MergeSheetModel` | `MergeSheet` |
| S11 | vCard import and export (File menu), no undo | | panels intents | import/export, `INTERCEPTED` | | `ContactsPanels` |
| S12 | **Sync** (08): S12a Settings pane and the four switches (no sync); S12b `postio-contact-sync`, link tables, dry-run, fakes; S12c Apple adapter read only, permission at enable; S12d the reviewed gate lifts (maintainer's commit); S12e CardDAV. **ADR**: an external store is written only through a reviewed plan | 7 | | `ExternalAddressBook`, sync events | `ContactsSettingsModel`, `AppleAddressBook` | `ContactsSettingsPane` |
| S13 | Linux adoption: GTK draws the Rust view models and turns `contacts_view` on (pieces may land earlier, per section 3.1); `next_scope` → `g n`; GTK file chooser for import and export; CardDAV sync is already shared; storyboards filmed; the `interactions-unreviewed` label retires | | none by design | | | (GTK) |

Ordering: S1 to S5 are engine and ship without any Mac UI (the first visible
change is S4's names, only where a name was set); S6 can start once S2 and S3
land; S7 once S5; S8 to S10 follow S7; S11 anywhere after S8; S12 last.

## Technical Context

**Language/Version**: Rust 1.99.0 (`rust-toolchain.toml`; on this Mac `unset
RUSTUP_TOOLCHAIN MAKEFLAGS` first); Swift tools 6.0

**Primary dependencies**: existing workspace crates; spec 005's `postio-vcard`
(Pimalaya `vcard-rs`); in S12 the `Contacts` framework (Swift) and a CardDAV
transport (Pimalaya first). No other new dependency.

**Storage**: the encrypted Turso store; `schema.rs` `HEAD` changes with a
snapshot and a `Migration` per slice that touches it (S1, S3, S5, S12).

**Testing**: `cargo nextest` per crate and suite; `scripts/test-fast.sh` between
edits; `ffi_suite` for exports; `scripts/macos-test.sh` for Swift; storyboards
red first; `counting` plus plan assertions for every read.

**Target platform**: macOS 14+ (arm64); Linux stays green on every commit.

**Performance goals**: popover and card < 50 ms, `g c` first rows and every key
< 16 ms, filter < 100 ms, startup < 500 ms with the catch-up in the background.

**Scale**: 20,000 contacts, 500,000 messages; about 45 commands, 4 contexts, 1
mode, 2 popovers, 1 form, 1 sheet.

## Constitution Check

| Principle | How this plan meets it | Status |
|---|---|---|
| I. Local-first | Every verb is a store write, an event and a repaint with an undo entry; sync is the only network path, opt-in, in the background, and never awaited by the UI | Pass |
| II. One command table | Every command a registry row with a palette entry and a menu section; `offered_on` keeps Linux's keymap; collisions resolved in the Keys table and asserted by `bindings_do_not_collide_within_a_context` | Pass |
| III. One query language | All mail is an `AnyOf` of `from:` or 005's `with:`; the contact-aware expansion is an executor option, not a second language | Pass |
| IV. Test-first | Each slice lists red tests first; the behaviour-preservation fixture (SC-005) and the kind corpus are written before the code | Pass |
| V. Performance | Maintained columns and bound ranges; plan assertions; the catch-up never blocks startup; insert cost measured before S5 | Pass |
| VI. Privacy | No network except sync the person turned on; signature parsing on this Mac and stored only on save; the dry-run report is never logged; the Contacts permission only on enable; fixtures use `example.*` | Pass |
| VII. Boundaries | `postio-classify` and `postio-contact-sync` are pure leaves; `CNContactStore` stays in one Swift file behind a foreign trait; `PostioKit` has no AppKit; `check-no-silent-tracking.py` sees every panel and any sync call with a `POSTIO-CONSENT:` comment | Pass, checked by `scripts/check.sh` per slice |

## 11. Linux: what keeps building, and what a Linux user would see change

**Building.** Every shared change is additive or behind `offered_on` or
`Policy.caps.contacts_view` (false on Linux); `Context` gains variants in every
exhaustive match (listed in the design table), which `cargo check` on a machine
that can build GTK, and CI, prove; run `scripts/check.sh` and `cargo check -p
postio-tui` locally. No GTK code is changed before S13 except where S3 must
(below). The store migration is the same engine on both.

**What a Linux user would see change** (the maintainer is told before each
merges):

| Slice | Visible on Linux | Merge note |
|---|---|---|
| S1, S2, S5 | Nothing visible. First launch after the upgrade runs the migration and a background catch-up (CPU and disk for minutes on a large store); no UI on Linux | tell: the catch-up |
| S3 | Single-address digest rules and pinned-address entries **leave `config.toml`** (kept in `config.toml.pre-contacts`); the digest rules page (`g r`, and the TUI's rules view) lists contact-derived rules, named by the contact; behaviour unchanged (SC-005). `postio-ui/src/filtering.rs` and `postio-tui/src/rules.rs`, `config_file.rs` change | **the one that changes how Linux gets its per-sender rules; needs the maintainer's OK** |
| S4 | A name the user set replaces the header name in the list, conversation and reader (005 R7), **only for contacts that have one** (migrated `name` overrides; nothing else sets one until Linux has an edit surface, S13). Recipient completion and the `@` finder become person-shaped (one entry per contact, addresses beneath), as 005 FR-030 specifies; ranking by kind is **not** applied on Linux. Contact-aware `from:` stays **off** on Linux | tell: completion shape |
| S6 | `i i` and `i a` (images) become `alt+i`, `alt+I` if Q2 is A | **keybinding change, needs OK** |
| S6-S12 | Nothing: new commands are Apple-only; `keybindings.md` Linux rows unchanged except the images pair | CI |
| S13 | Everything, by design | its own review |

Net answer to the handoff's step 7: **the migration (S1) does not change how
Linux shows senders.** The changes that do are S3 (where rules are read), S4 (a
set name, completion shape) and the images keys, each flagged above.

## Test plan

| Layer | What | Where |
|---|---|---|
| Registry | rows, titles, no collisions in the four contexts, `offered_on`, `add_account` excludes `Contacts`, regenerated `keybindings.md` | `postio-core`, `postio-ui` |
| Detection | the kind corpus (SC-003); signature fixtures; bounce DSN fixtures; duplicate scoring | `postio-classify/tests` |
| Storage | migration from the previous schema; catch-up idempotence and resume; stats maintenance with the double-count rule; plan assertions and budgets | `postio-storage`, `postio-host` |
| Behaviour proof | SC-005 old-vs-new decisions over one corpus | `postio-host` |
| Words | empty states, toasts, role lines, frequency phrase | `postio-ui` |
| Controller | tabs and cursors; Esc ladder and arming; marks-else-cursor; the cursor after a merge and its undo; popover modes; History | `postio-focus/tests/contacts.rs` and siblings |
| FFI | a typed sequence over the demo store emits intents in order; a failed write keeps the intent; the foreign trait's refusal of writes | `ffi_suite/focus_contacts.rs` |
| Sync | planner both directions, conflicts, deletes, ambiguity; runner with fakes; dry-run lists every change and writes none; the gate | `postio-contact-sync`, `postio-host` |
| Swift | models follow intents; keyboard paths; popover anchoring; `AppleAddressBook` over a fake `ContactBook`; `Contacts` imported in one file | `macos/Tests/…` |
| Privacy | `check-no-silent-tracking.py`; no permission request before enable (a Swift test on the fake); no log line carries a name | `scripts/check.sh`, `logging_privacy` |
| Storyboards | popover, contacts-open, edit-save, merge, settings; red on the base | `storyboards/contacts/` |
| Screens | each handoff PNG vs a 1440×900 capture, light and dark for 01 and 02 | `specs/011-focus-contacts/screens.md` |
| Linux | `postio-gtk` suites unchanged; keymap docs diff limited to the images pair | CI |

## Project Structure

```text
specs/011-focus-contacts/
├── spec.md
├── plan.md                  # this file
├── contracts/
│   ├── focus-contacts.md    # intents, rows, popover and sheet shapes
│   └── contact-sync.md      # the foreign trait, the plan, the gate
├── checklists/requirements.md
├── screens.md               # written as screens land (FR-042)
└── tasks.md                 # /speckit-tasks

crates/
├── postio-core/src/{registry,command,context}.rs
├── postio-model/src/contact*.rs
├── postio-classify/src/{kind,signature,bounce,duplicates}.rs   # NEW, pure
├── postio-storage/src/{schema.rs,repository/contact_*.rs}
├── postio-host/src/contacts/{catchup,settings,review,sync}.rs
├── postio-ui/src/{contacts_words,sender_treatments}.rs
├── postio-focus/src/{contacts,contact_card,contact_popover,contact_merge}.rs
├── postio-ffi/src/focus_contacts.rs
└── postio-contact-sync/                                          # NEW, pure, S12
macos/Sources/{PostioKit,PostioAppKit,Postio}/…
macos/Tests/…
storyboards/contacts/*.toml
docs/keybindings.md                                               # regenerated
```

## Complexity Tracking

| Violation | Why needed | Simpler alternative rejected because |
|---|---|---|
| Stats per address, not per contact | Merge, split and move must rewrite nothing and be exactly undoable | Per contact: every merge recomputes and every undo must restore the old numbers |
| A ring of 50 recent messages per address | Open items, recent issues, files and frequency in bounded statements | Querying a contact's mail: a scan that grows with the mailbox, which the 50 ms card cannot afford |
| Four contexts | The list, the form, the popovers and the sheet each give `Return`, `n` and `Escape` different meanings | One context: the collision test cannot say it, and `Return` would be All mail and Merge at once |
| The hybrid settings move (Q1 A) | The address-keyed rules are what merge needs to follow the contact; the file-backed treatment is read by GTK directly | Moving everything now: changes a GTK reader this Mac cannot build; moving nothing: sender settings stay in four homes |
| Two keys on sync writes (Rust gate, Swift refusal) | The maintainer's rule: never write to the real address book unreviewed | One gate: a bug in one side writes |
| Mac-only for now (`offered_on`) | GTK cannot be built here; keys that do nothing are worse | Offering on Linux with no surface (010 D25) |
