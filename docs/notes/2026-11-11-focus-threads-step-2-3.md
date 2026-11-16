# Focus threads, steps 2–3: thread rows and expansion against screens 01, 02 and 11

Spec 013, User Story 1 (T036). The `threads` demo seed was shot at 1440×900:
on the Mac with `scripts/macos-shot.sh --seed threads`, and on Linux by
rendering the GTK window on the verify box. Each was compared with the
maintainer's local `Design/focus-macos-threads/screens/01`, `02` and `11`.
Those PNGs are not in the repository.

## What matches, on both platforms

- **Thread row.** Unread dot, then the chevron (none on a one-message
  thread). Participants: up to three first names, newest first, then
  "you", then "+N" ("Ben, Lena, Ana, you"). The count badge sits right
  after the participants. The subject is bold when unread, followed by
  the latest message's sender and first line ("Ben: Looks good…"), then
  "2 new" as a filled pill (not the accent), the paperclip, and the
  time.
- **Child rows** under the expanded thread:
  - tinted, joined by a vertical line with a dot per message (the
    accent when unread);
  - an initials avatar, the sender, and the message's own text, never
    its quote;
  - "↳ to Ana" on the reply to an older message;
  - the paperclip and the time;
  - the draft last, in italics ("Draft · you … not sent").
- **The focused child** carries the reply and forward keycaps and "this
  one".
- **Keys.** `Right` expands, `Left` folds, `j`/`k` walk the children,
  `Return` on a child opens that message, and `Return` on a row opens the
  first unread. A click on the chevron expands and folds.

## Differences, and why

| Difference | Platform | Why |
|---|---|---|
| Hint keycaps read `r f`, not `e f` | both | Keys come from the live keymap; the design predates the Gmail keymap (spec D1) |
| The row subject reads "Re: Harbor API draft v3"; the design shows "Harbor API draft v3" | both | The list draws the newest message's subject. Showing the root's needs the thread's display subject on the list page; to fix in a later step |
| Some latest lines end in "(41)", "(2)" | both | The demo seed's message text, not the UI |
| Seed names, times and order differ from the design's | both | Invented seed data; the day heading follows each runner's demo clock |
| Avatars are neutral grey (Mac), or hashed colours (GTK); the design gives each person a colour | both, differently | The Mac follows the semantic-colours rule. The two platforms should agree; to settle when the thread bar (US3) draws the same avatars |
| Plain one-message rows are 40 tall; the design's are 46 | Mac | Thread rows are 46. The one-message row height predates this spec and is unchanged here |
| The chevron swaps between two symbols rather than rotating 90° | both | No transition, so reduced motion needs no special case. Acceptable under the ≤ 100 ms-or-none rule |
| The chevron's and dot's x positions are a few points off the design's | GTK | Within the GTK metric roles; left as is |
| No footer ("Message 4 of 6 · Ben Adeyemi …") | both | US2 (T037–T041) |
| No "Threads: newest message first" note in the header | both | Not in any task; raise with the maintainer if wanted |

## Not compared

Screen 11 (dark) was shot on both platforms and differs from 02 only in
colour roles. No difference in layout was found.
