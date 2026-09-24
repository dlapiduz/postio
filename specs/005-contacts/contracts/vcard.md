# Contract: vCard import and export

Crate `postio-vcard` (new leaf; research R9) over Pimalaya `vcard-rs =0.4.0`.
No database engine, gtk4 or tokio — a `scripts/checks/` boundary rule says so.

## Surface

```text
parse(bytes) -> Import { cards: Vec<ParsedCard>, skipped: Vec<Skip { index, reason }> }
ParsedCard::Person { uid, name, emails: [(EmailAddress, preferred)], organization, note, raw }
ParsedCard::Group  { uid, name, members: [uid or mailto:] , raw }
export(person, raw: Option<&str>) -> String      // one 4.0 card
export_group(group, member_uids, raw) -> String
```

The app does the store half (matching addresses to people, joining, writing
`contacts.vcard`), so the crate stays a pure mapping.

## Import rules

| Input | Result | Spec |
|---|---|---|
| 3.0 or 4.0 card (2.1 read too) | parsed | FR-050 |
| several `EMAIL` on one card | one person, several addresses; `PREF=1` / `TYPE=pref` marks preferred, else the first | FR-052 |
| `KIND:group` + `MEMBER` | a group; members resolved by `urn:uuid:` UID or `mailto:` | FR-052 |
| an address already owned by a person | card and that person are one person | FR-053 |
| addresses owned by several people | all joined; the summary lists the join | R9 |
| card `FN` vs a user-set name | user's name stands; summary counts it | R9 |
| `CATEGORIES` | kept verbatim, not mapped to groups | R9 |
| unreadable card | skipped, counted, reason given; the rest import | FR-054 |
| any other property | kept verbatim in `contacts.vcard` | FR-051 |

Summary shape: `{ people_created, people_updated, joins: [(names…)],
groups_created, name_conflicts, skipped: [(index, reason)] }`. Shown to the
user; logged as counts only (FR-061).

## Export rules

- Always `VERSION:4.0`.
- Modelled properties (`UID`, `FN`, `N`, `EMAIL`, `ORG`, `NOTE`, `REV`,
  `KIND`, `MEMBER`) are written from the store, edited in place in the stored
  card so their position and group prefix survive.
- Every other property: byte-identical from a 4.0 source. From a 3.0 source,
  only the spellings 4.0 forbids change: `VERSION`, `ENCODING=b` → `data:` URI,
  `TYPE=PREF` → `PREF=1`, `CHARSET` dropped. (Sharpens FR-051 / SC-006.)
- What is exported with nothing selected: exactly the people the list shows
  (FR-050); never a non-`live` person.
- A person with no stored card gets a fresh minimal card with a new `UID`.

## Fixtures

Under `crates/postio-vcard/tests/corpus/`, reserved domains only
(`check-no-personal-data.py` applies): a 3.0 export with `item1.` groups,
`PHOTO;ENCODING=b`, `TEL`, `X-` vendor properties and two `EMAIL`s; a 4.0
export with the same; a group card; a file with one malformed card among good
ones.
