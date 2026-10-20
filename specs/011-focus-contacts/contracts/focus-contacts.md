# Contract: Contacts intents, rows, popovers and the sheet

What `postio-focus` tells a frontend to show, and what a frontend reports back.
Shapes, not code: the Rust types and their `*Ffi` mirrors are written test-first
from this. Words come from `postio-ui`; Swift adds none. Revised 2026-10-19 to
the handoff `Design/focus-macos-contacts/` (screens by filename).

## Rows (the list's content)

| Kind | Fields shown | Where |
|---|---|---|
| `Contact` | avatar (kind shape, palette index from the id, photo key), name, tag pill (`notify`, `digest weekly`, `filtered`, `always original`), primary address, extra-address count, last-message date, count line ("431 messages", "list · 112 messages", "newsletter · 48 issues", "company · 3 people · 57 messages"), marked | every kind tab |
| `Letter` | the header letter | between contacts |

Rows are paged by the display-name sort key; the strip carries the total, the
five tab counts (from `contact_counts`), and the review summary
(`duplicates`, `bounced`, `frequent_unsaved`; all zero hides the line). The
footer carries total, saved and the sync label.

## The card (the right part of the mode)

One read per cursor move, answered from maintained columns (plan section 6):

| Field | Notes |
|---|---|
| `header` | kind, name, role line parts (title, company, saved, sync label, groups), buttons by kind (`Write`, `All mail`, `Edit` or `All issues`, `Unsubscribe`, `Edit`) |
| `addresses[]` | address, primary, note ("old work · last used March 2025"), `bounced_at`; plus `also_seen_as[]` |
| `arrival` | `Inbox \| Digest \| Filtered out`; for Digest the rule line ("in the weekly digest, Saturday 16:00") |
| `switches` | notify, html_original, remind (days), archive_after (days, newsletters) |
| `notes` | text and the linked path |
| `tiles[4]` | value and caption, `—` when unknown |
| `months[12]` | counts and axis labels |
| `open[]` | kind ("Waiting on you", "Waiting on X", "Task"), text, when/state, an openable target |
| `files[]` | chips |

## The popover

`ShowContactPopover { anchor, known: Known | NewSender }`:

- `Known`: avatar, name, address, role, `tiles[3]`, `open[≤2]`, `arrival`, the
  three actions.
- `NewSender`: "Not saved yet", `reached_via` (a sentence), `signature[]`
  (title, company, phone, website, each with where it was seen), `arrival`, the
  three actions. `signature` is empty when "Read signatures for details" is off.
- A change to `arrival` is a command and emits `Pill { text, undoable }`.
- The popover opens above the row when `room_below` (reported by the frontend as
  a toolkit fact) is too small.

## Intents the controller emits

| Intent | Meaning |
|---|---|
| `ShowContacts { tab, rows_from, card }` | enter or redraw the mode; no transition |
| `ShowContactPopover { … }` / `CloseContactPopover` | the popover, anchored to a row or a name |
| `EditContact { id }` / `LeaveEdit` | the form in place of the card, or the card |
| `ShowMergeSheet { candidates, kept, evidence[], preview }` | the sheet |
| `Pill { text, undoable }` | the toast |
| `Refuse { text }` | a one-line refusal |
| `FocusFilter` | the strip's filter field |
| `PresentPanel { kind: Open \| Save, suggested_name }` | import and export; the frontend shows the system panel |
| `ShowSyncPlan { report }` | the dry-run sheet (S12); the report carries names and is never logged |

## What a frontend reports

Only toolkit facts (009): a click on a row or a sender name, a click outside a
popover, room below an anchor, the panel's chosen URLs or its cancel, the list
scrolled to its top, a focus change in a text field (so single keys do not fire
in it). Surfaces the controller opens or closes are not echoed back.

## Failure

A write that fails reverts the visible change, shows `Pill { "Couldn't save.
Nothing changed. <reason>" }`, and keeps the intent so that the same key retries.
`Edit`'s `Save` keeps the form open with its values.

## Panels and files

`PresentPanel` is the only way a file is read or written; cancel returns none and
the controller shows nothing; every call site carries a `POSTIO-CONSENT:` comment
(`check-no-silent-tracking.py`). Import is not undoable (M4).
