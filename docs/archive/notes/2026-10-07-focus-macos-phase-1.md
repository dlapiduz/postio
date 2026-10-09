# Focus on the Mac, phase 1: the inbox against screens 01 and 02

2026-10-07, specs/009-focus-macos T036 (FR-061).

**Captures.**
- Made with `scripts/macos-shot.sh 01 --seed small --both`.
- They are the Mac bundle over the in-memory `small` demo store
  (`postio-demo`), at 1440×900, in light and dark.
- Captures live in the main checkout's untracked
  `Design/review/focus-macos/`, beside the design they answer to.
- Reference: `Design/focus-macos-design/screens/01-inbox-light.png` and
  `02-inbox-dark.png`.

The rows, the marker lines and the strip are the engine's. The list is
filled by `postio-focus`'s feed (the same rules GTK's list runs on). The
rows' words come from `postio_ui::focus_row` through `FocusRowFfi`, and the
strip's from `focus_strip`. So a difference below is a Mac drawing
difference, or a fixture difference, not a second set of rules.

## What matches

**Toolbar.**
- Traffic lights, then the compose button.
- On the right: "Synced 16:09", and the search field reading "Search mail or
  run a command" with its ⌘K keycap.

**Header strip.**
- "Inbox ▾ `g o`", the counts, and the "Has action · N `!`" toggle on the
  quaternary fill.
- On the right: "N filtered today `g f`" and "N digest rule(s) `g d`".

**List.**
- The day heading: "Today · <weekday> <date>".
- Columns: the sender at x = 56 and the subject at x = 290 (of 1440), as
  the PNG has them.
- Row heights: one-line rows about 40 pt and marked rows about 72 pt.
- The unread dot and bold.
- At most two label pills, each with its dot.
- The preview runs to an ellipsis before the trailing paperclip, count
  badge and time.

**Marker lines.**
- An outlined kind chip ("Invite", "Question", "To-do") in the accent.
- The date in bold accent, and the sentence quoted verbatim in italic
  accent.
- The answering action right-aligned with its keycap: "Accept `y`",
  "Decline `Y`", "Reply `e`", "Snooze `s`".

**Digest row.**
- The stacked icon, "Weekly · digest", "Newsletters · N messages", the
  summary line, and the count badge.

**Cursor and colour.**
- The cursor: an accent ring with a faint accent fill, on the first row
  when the list opens (C30).
- Dark: semantic colours throughout, with the accent lightened by the
  system. No hex anywhere (`SemanticColourTests`).

## Differences, and why

| Difference | Why | Where it is settled |
|---|---|---|
| Counts and names differ ("59 · 21 unread", "Hollis Varga" where the PNG has "312 · 41 unread", "Grace Oyelaran") | The demo store is `postio-demo`'s seed, not the mockup's data | Fixture, not a defect |
| The cursor is on the first row with nothing selected; the PNG has three rows checked and the ring on the third | The PNG is mid-triage; a list opens on its first row (C30) | US2 adds selection and the bottom action bar (T047–T052) |
| No bottom action bar ("3 selected · Archive `a` · …") | Nothing is selected | US2 (T052) |
| To-do rows offer "Snooze `s`" without "Task `t`" | Task appears only with a vault configured (C9); the demo's config has none | Decision C9 |
| No "Task in Atlas · due Fri" chip on a row | Not built on either platform | M3 (T119) |
| Text runs slightly narrower than the PNG's | The PNGs were rendered without SF Pro, in a wider stand-in sans (pack README) | Not a defect (C16) |
| The invitation's date reads "Sat 10 Oct · 10:00–10:45" where the PNG reads "Tue 29 Sep" | The demo dates its mail relative to today | Fixture |
| The has-action toggle is drawn off (quaternary fill), as in the PNG; turning it on is not yet tested on the Mac | `!` reaches the controller in US2 | US2 (T039, T040) |

Found and fixed on the way:
- **The main window opened as a strip three rows tall.** The scene sized
  itself to its content, which reported almost none (`20d9c45f`).
- **The strip could not show the digest-rule count, and the search field
  had no ⌘K keycap** (`e04c5da6`).
- **A demo build started syncing** an account that has no server
  (`cd7e2edf`).

## What this phase does not cover

These come with later phases, and each has its own comparison:
- the email window (04);
- the pickers, the undo pill and the states (11–19);
- the 1024-wide check.
