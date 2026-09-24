# Data Model: Contacts

Phase 1. The store's shape after this feature, and the rules each table
carries. Reasons are in `research.md` (R-numbers); requirements in `spec.md`
(FR-numbers). SQL here is the shape, not the final text — `schema.rs`'s
`HEAD` is the one home, and its comments carry each column's meaning.

## Overview

```text
            ┌──────────────────────────┐
            │ contacts  (a person)     │◄───────────────┐
            │ state live|deleted|merged│                │ merged_into
            └──────────┬───────────────┘────────────────┘
      preferred_address│      ▲ contact_id (0..1 owner per address)
                       ▼      │
 recipients ──► addresses ────┘
                    │
                    ▼  (address_id, account_id)
            contact_sightings        contact_terms (term → contact)
contact_groups ◄── contact_group_members ──► contacts
contact_join_candidates / contact_join_dismissals  (address pairs)
```

## `contacts` — a person

Replaces today's one-row-per-address table. `account_id`, `address`,
`address_name`, `address_normalized`, `suppressed` and `vcard_extra` go.

| Column | Meaning |
|---|---|
| `id` | Local id (`ContactId`). |
| `name` | The name the user set or picked on a join (FR-013, FR-021). NULL until they do. Never written by sync. |
| `organization`, `note` | User-edited (FR-021). |
| `source` | `mail` \| `user` \| `import` — how the person first appeared; `mail` → `user` on the first user edit or join (FR-022). |
| `state` | `live` \| `deleted` \| `merged` (R4). Only `live` people are shown, offered or used for name substitution. |
| `merged_into` | The survivor, when `state = 'merged'`; NULL otherwise. |
| `preferred_address` | `addresses.id` of the preferred address (FR-016); must be one of the person's own. |
| `seen_name` | The display name most recently seen on any of the person's addresses. |
| `sort_key` | Case-folded displayed name: `name`, else `seen_name`, else the preferred address. Ordering key for the list (R5). |
| `name_key` | Case-folded, whitespace-collapsed displayed name; join-suggestion key (R11). |
| `times_seen`, `last_seen_at`, `written` | Aggregates over the person's sightings (R2). `written` = total `times_written`; > 0 puts the person in the default list (FR-005). |
| `uid` | vCard `UID`; kept across export/import. |
| `vcard` | The whole card as last imported, verbatim (R9). NULL for a person never imported. |
| `created_at`, `updated_at` | Epoch millis; `updated_at` is the vCard `REV`. |

**Rules.**
- A `live` or `deleted` person owns at least one address. Detaching the last
  is refused (spec edge case).
- `merged` people own no addresses (they moved to the survivor) and keep
  their fields and memberships for undo.
- Displayed name: `name` → `seen_name` → preferred address (spec edge case
  "an empty name").

**Indexes.** Default list `(state, written, sort_key, id)`; everyone and
deleted `(state, sort_key, id)`; completion rank
`(state, (source = 'mail'), last_seen_at DESC, times_seen DESC, id)`
(the inherited Q6 bands); suggestions `(state, name_key)`.

## `addresses` — gains an owner

| Column | Meaning |
|---|---|
| `contact_id` | The person this address belongs to, or NULL (R1). NULL for the user's own identity addresses and for addresses only ever seen in the user's own drafts. |

Indexed on `contact_id`. The unique normalised index stays; it is what makes
"differ only in case" one address (FR-011).

**Rules.**
- At most one owner per address (FR-010) — a column, so nothing to enforce.
- Sync assigns an owner to a newly seen, non-own address by creating a `mail`
  person for it (the spec's "every address starts as its own contact").
- An address is **suppressed** iff its owner's `state` is not `live` (R4).

## `contact_sightings` — the mail's evidence

Primary key `(address_id, account_id)`.

| Column | Meaning |
|---|---|
| `times_seen` | Messages this address appeared on in this account, counted once per message and only on first insert — the existing double-count rule, unchanged. |
| `last_seen_at` | Monotonic max of the messages' dates. |
| `last_name` | The display name the most recent message carried. |
| `times_written` | Messages *from* one of this account's own addresses with this address in To/Cc/Bcc (R3). |

Rows survive every structural edit: a join, detach or move changes
`addresses.contact_id`, never a sighting (FR-012, FR-014: history travels with
the address).

## `contact_terms` — the filter index

Primary key `(term, contact_id)`. One row for each word of `name`,
`organization`, `seen_name`, and each owned address's local part and domain,
case-folded. Rewritten for a person whenever any of those change. Serves the
list filter, completion and the finder's prefix (R5).

## `contact_groups` and `contact_group_members`

`contact_groups`: `id`, `name`, `uid`, `vcard`, `created_at`. `account_id`
goes — groups are shared, as people are. Name unique case-insensitively (so
`group:<name>` is unambiguous).

`contact_group_members`: `(group_id, contact_id)`, both cascade. Members are
people. A join gives the survivor the union of memberships and records which
it added, so undo removes exactly those (R8). Deleting a person keeps its
memberships (restore returns them, FR-023a); a non-`live` member is skipped
when a group is expanded or searched.

## `contact_join_candidates` and `contact_join_dismissals`

Both keyed by an address pair `(address_low, address_high)`, the lower id first.

- Candidates: `replies` — how many times mail to one was answered from the
  other (R11). Written by sync.
- Dismissals: presence only. A suggestion between two people is hidden when
  any dismissed pair has one address in each (Story 4, scenario 2).

## State transitions of a person

```text
          create / first sighting
                  │
                  ▼
   ┌────────── live ◄──────────┐
   │  delete      │ join (absorbed)
   ▼              ▼            │ unjoin (undo)
deleted ──restore─┘  merged ───┘
   ▲   (or: its address added elsewhere → address moves; FR-024)
```

- `live → deleted`: delete (FR-023). Addresses stay; new mail counts but shows
  nowhere.
- `deleted → live`: undo, or restore from the Deleted filter (FR-023a).
- `live → merged`: absorbed by a join. `merged → live`: undo of that join only.
- An address of a `deleted` person may be moved to another person (FR-015,
  FR-024); a deleted person left with no addresses is removed outright.

## Model types (`postio-model`)

- `Contact` becomes the person: `id`, `name`, `organization`, `note`, `source`,
  `state`, `preferred: AddressId`, `addresses: Vec<ContactAddress>`,
  `seen_name`, `times_seen`, `last_seen_at`, `written`. `display_name()` keeps
  its precedence (name → seen name → address).
- `ContactAddress`: `id: AddressId`, `address: EmailAddress`, per-address
  `times_seen`, `last_seen_at`, `written`.
- `ContactState` { `Live`, `Deleted`, `Merged` } beside `ContactSource`.
- `AddressId` is new in `ids.rs`; `ContactId`, `ContactGroupId` stay.
- `ContactGroup` loses `account_id`.

## What reads it, after

| Reader | Reads |
|---|---|
| Contacts list (new) | `contacts` by view index, keyset pages; `contact_terms` when filtered |
| Detail view (new) | one person + addresses + per-address sightings + groups |
| Composer completion | `contact_terms` → live people by rank → their addresses, preferred first (FR-030) |
| `@` finder | live people with their addresses (as today, reshaped) |
| `group:` / `with:` | `recipients.address_id` via `addresses.contact_id` / by exact address (R6) |
| List, thread rows, reader | sender name via `addresses.contact_id` → live `contacts.name` (R7) |
| Search ranking affinity | `contact_sightings.times_seen` by `address_id` (replaces `executor.rs:1343`'s contacts subquery) |
