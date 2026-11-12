# Focus threads, steps 5–6: live replies, the thread menu and outline, search in threads

Spec 013, User Stories 4, 5 and 6 (T055, T068, T075). The `threads` demo
seed was shot on both platforms and compared with the maintainer's local
`Design/focus-macos-threads/screens/04`, `05`, `06`, `07` and `08`. Those
PNGs are not in the repository.

- **Mac.** `screencapture` returned black in this session, so the in-app
  snapshot (`POSTIO_DEMO_SNAPSHOT`) was used.
  - Screen 08: `⏎ [ @reply`.
  - Screen 04: `⏎ ⌥.`
  - Screen 05: `j j ⏎ ⌥t`, on the 23-message thread.
- **Linux.** The GTK window was rendered on the verify box.

## What matches, on both platforms

- **08, a reply while reading.** The shown message, its scroll and the
  focus stay put. The bar gains a chip with an unread dot, and an
  accent-tinted banner names who replied, quotes the first line, and
  offers "Read it }". The list row behind the window updates.
- **04, Thread ▾ (`alt+period`).** The menu has two groups:
  - Whole thread: outline (`alt+t`), forward it (`f`, `2`), reply to
    latest (`alt+e`), mute (`m`).
  - This message only: split (`alt+shift+s`), move (`alt+v`), delete
    (`alt+BackSpace` on the Mac, `alt+Delete` on Linux), always show its
    HTML (`O`).

  Each entry has its keycap from the keymap. Split is disabled on a
  thread's first message, with the reason.
- **05, the outline (`alt+t`).** A filter field (`/`), person chips with
  counts, numbered rows (bold when unread, with an unread dot), the
  current one ringed, and the footer ↑↓ · ↩ · `u`.
- **The muted row** reads "muted · 1" and keeps its place in the inbox.
- **06, search.** Results group by thread. "3 of 6 messages match" opens
  expanded, showing only its matches, each tagged ("message 5 · file")
  with its passage, and "Message 4 doesn't match · Ben, Thu" between them.
- **07, opened from search.** The window opens at the match: "Message 3
  of 6", a match banner ("match 1 of 3 in this thread (messages 3, 5 and
  6)"), and matched chips with a yellow square. `mod+g` reaches message 6;
  `Escape` returns to the child.

## Differences, and why

| Difference | Platform | Why |
|---|---|---|
| The popovers may sit about 60 pt right of the Thread button | Mac | The in-app snapshot draws a popover behind the window; to check on a real display |
| No "Export as PDF" in the menu | both | Out of scope: Postio has no print path |
| The outline's footer has no "N more above · scroll" | both | Not built; the list scrolls |
| Search children have no avatar, sender or time | Mac | `ResultChild::Match` does not carry them; GTK reads them itself. Moving them into the shared child is T085(i) |
| A single-match result reads its old snippet, not "Message 5 of 8 · Tomás: …" | both | Needs the match's position in its thread from the search executor. T085(g) |
| Clicking a search child does nothing; the keys reach it | both | No click input yet |
| The match banner sits under the older-message banner | Mac | Both apply when the match is not the latest; the design shows one |
| The match square uses the dropdown's find yellow | GTK | `/gtk-design` has no find-colour role; the existing `FIND_LIGHT`/`FIND_DARK` is reused |
| Wording and positions differ slightly | both | Invented seed data |

## Found on the way

- Mute and split were not on the session's bus, so a posted mute was
  refused and never reached undo. Fixed (`4f4caee44`).
- Splitting the shown message made the window say it was gone. The
  window now follows the message into its new thread (`c39f295de`).
- Deleting just this message left the window on a deleted message. It
  now steps to the nearest remaining message (`5344eb294`).
