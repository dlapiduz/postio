# Data model: the conversation reading pane

**Feature**: `specs/001-conversation-reading-pane` | **Date**: 2026-09-08

Most of what this feature needs already exists in `postio-model`, whose types
*"would survive a second protocol without changing shape"*. What follows is
what this feature adds or changes, and the rules attached to each.

## Existing, consumed unchanged

| Entity | Where | Used for |
|---|---|---|
| `Thread` | `postio-model`, `postio-storage::repository::threads` | The conversation the pane presents (FR-012) |
| `Message` | `postio-model` | Sender, recipients, date, flags, body state |
| `EmailAddress` | `postio-model::address` | Header rendering via `postio_ui::conversation::participants` |
| `Attachment` | `postio-model` | Indicated in the message header |

**A thread belongs to one account**, so no conversation the pane presents ever
spans accounts.

## Added

### Message length

**What it is**: a line count for a message, used by the rail to show which
messages are long enough to be worth knowing about before scrolling into them
(FR-033).

| Field | Type | Rule |
|---|---|---|
| `line_count` | unsigned integer | Counted from the **sanitized plain-text body at index time**. Never measured from drawn output (FR-039). |

**Where it lives**: with the message, written when the message is indexed.
Per the constitution's no-backwards-compatibility rule, the column takes a
plain default and existing rows are rebuilt by reindex rather than migrated
through a compatibility path.

**Why stored rather than derived**: FR-040 requires every rail row to exist
immediately, independent of whether that message's body has been prepared for
display. A count computed at draw time would make the rail wait for exactly
what the rail exists to let you skip.

**Displayed only above a threshold** — a count on every row is noise; a count
on the long ones is information.

### Rail state

**What it is**: what the rail knows, split deliberately into two parts that do
not share a source (FR-040).

| Field | Type | Rule |
|---|---|---|
| `rows` | ordered list of (position, sender, date, line_count) | Built from the **thread model**. Complete the moment the conversation is known. |
| `current` | position | Derived from **what is on screen** (FR-034), never from the last activation (FR-035). |
| `shown` | boolean | Persists **per window**, not per conversation (FR-047). |

**State transitions for `current`**:

| From | Event | To | Rule |
|---|---|---|---|
| any | user scrolls | message with greatest visible **area** | Settled, not continuous (FR-036); never animated |
| any | rail row activated | that message | Through the single entry point (FR-038); must not let the resulting scroll re-derive a different value (FR-037) |
| any | keyboard message navigation | next/previous | Same entry point as above |
| — | conversation opened | most recent message | FR-015, unconditional |

**Greatest visible area, not greatest visible fraction.** A two-line reply
fully in view has a visible fraction of 1.0; an eighty-line message filling
most of the screen has a fraction well below it. Ranking by fraction marks the
wrong one, which is the defect FR-035 was written to forbid.

### Sender styling policy

**What it is**: the enumerable decision, per CSS property, of whether a
sender's declaration survives to the screen (FR-019b).

| Field | Type | Rule |
|---|---|---|
| `admitted` | set of properties | Layout, colour, typography, spacing — FR-019a's floor |
| `refused` | set of properties, each with a reason | Every entry is either **containment** (FR-020, FR-021) or **privacy** (FR-022, FR-023). No other reason is permitted. |
| `scope` | per-message container identity | Every admitted rule applies only within its own message |

This is a table with a test per row, not a judgement made at render time.

## Changed

### Presentation state

**Was**: which messages are expanded, which is focused, and where the user had
scrolled, surviving reopening within a session.

**Now**: only which message is current, derived from what is on screen. Nothing
about expansion survives, because nothing is collapsed (FR-013), and no reading
position is restored, because a conversation always opens on its most recent
message (FR-015, FR-018).

## Removed

### Collapsed runs

`postio_ui::conversation::collapsed_runs()` and its tests, which fold runs of
consecutive collapsed messages into one divider. FR-013 forbids both the
collapsed message and the divider. ADR 0015 is amended in the same change (R6).
