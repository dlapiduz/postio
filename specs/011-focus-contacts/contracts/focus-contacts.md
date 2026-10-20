# Contract: Contacts intents, rows and panels

What `postio-focus` tells a frontend to show, and what a frontend reports
back. Shapes, not code: the Rust types and their `*Ffi` mirrors are written
test-first from this. Words come from `postio-ui`; Swift adds none.

## Rows (the list's content)

A row has a kind. Commands act on the kind under the cursor (spec, command
table).

| Kind | Fields shown | Tabs |
|---|---|---|
| `Person` | initials, name, organisation, preferred address, extra-address count, provenance, last in touch, message count, marked | 1, 2, 5, a group's members |
| `Group` | name, member count | 3 |
| `Pair` | the two names, the two addresses, evidence ("same name"), each address's message count | 4 |
| `Letter` | the header letter | all but 4 |

Rows are paged and keyed by the displayed name's case-folded key; the
header strip carries the title count, the tab counts and the Duplicates
count.

## Intents the controller emits

| Intent | Meaning |
|---|---|
| `ShowContacts { tab, rows_from }` | enter or redraw the mode; no transition |
| `ShowContactCard { id, mode }` | open or step the page window (`View`, `New`, `Deleted`) |
| `EditField { field }` / `LeaveEdit` | the page's one field in edit, or none |
| `OpenPicker { kind, anchor_row }` | `JoinName`, `JoinOrganisation`, `Group`, `FindOther`; contents from the controller |
| `ShowMove { address, owner_name }` | the Move popover |
| `ShowImportSummary { made, groups, joined, skipped }` | the sheet |
| `PresentPanel { kind: Open | Save, suggested_name }` | the frontend shows the system panel |
| `Pill { text, undoable }` | the toast |
| `Refuse { text }` | a one-line refusal in the footer |
| `FocusBox { mode: People, text }` | the one box in people mode |

## What a frontend reports

Only toolkit facts (009): a click on a row, a click outside a popover, the
panel's chosen URLs or its cancel, a dropped file list, the list scrolled to
its top. Surfaces the controller opens or closes are not echoed back.

## Panels and files

- `PresentPanel` is the only way a file is read or written. The frontend
  returns URLs only after the person chose them; cancel returns none and the
  controller shows nothing.
- Open accepts several `.vcf`; Save suggests `contacts.vcf`.
- Dropped `.vcf` files are the same request as the open panel's answer.
- Each panel call site carries a `POSTIO-CONSENT:` comment naming the
  person's choice (`check-no-silent-tracking.py`).

## Failure

A write that fails reverts the visible change, shows `Pill { "Couldn't save.
Nothing changed. <reason>" }`, and keeps the intent so that the same key
retries. An unreadable import file ends in `ShowImportSummary` with the
skipped count and reasons; an unreadable *file* ends in a refusal naming it.
