# Threads storyboards (spec 013)

The keyboard paths spec 013 films, one per acceptance scenario that involves
a key press. This file names them; the `.toml` files come later, one story at
a time, each written red on `main` before its build (ADR 0044, tasks.md
T027, T038, T044, T051, T064, T071, T081) and reviewed by `/ux-review` (T089).
Format: [../README.md](../README.md) and
`specs/008-storyboards/contracts/storyboard-format.md`.

Every storyboard names `apps = ["focus"]`, `seed = "threads"` (the demo seed,
T001 and T004) and `design = "<screen>"`, a screen of
`Design/focus-macos-threads/screens/`, compared at 1440x900. Keys are the
registry's (spec D1 and the Clarifications), never the design's own: the
design's `⇧M`, `⇧T`, `⇧F`, `→`, `⇧O` and `u` are `m`, `alt+t`, `f` then `2`,
`Right`, `O` and `g u` here. Steps press commands where the command is the
subject and raw keys where the key itself is.

Screens: 01 inbox collapsed, 02 thread expanded, 03 window on an older
message, 04 thread menu, 05 outline, 06 search grouped by thread, 07 opened
from search, 08 new reply while reading, 09 reply to an older message, 10
forward the thread, 11 expanded in dark.

| Slug | Story, scenarios | Keys pressed | Shows | Screen |
|---|---|---|---|---|
| `expand-collapse` | US1 #2, #3, #5 | `Right` on a thread row, `j` into the children, `Right` on another thread, `Left` on a child | "Harbor API draft v3" opens in place with its children joined by a line; moving to another thread collapses the first and expands the second, focus on its row; `Left` on a child collapses and returns focus to the thread row; `Right` on a one-message thread does nothing. Light, and dark (`vary`). | 02 (11 dark; 01 before the first key) |
| `open-child` | US1 #4, #6 | `Right`, `j` x4, `Return`; separately `Return` on a collapsed row | A child shows its own new text, never its quote; `Return` on child 4 opens message 4, not the latest; `Return` on the thread row opens at the first unread. | 02, then 03 |
| `scope-footer` | US2 #1 to #4 | `r`, `f` on a collapsed row and on child 4; `e` then `z` on a child; `l` on a child; `alt+v` on a collapsed row | The footer names its scope ("Harbor API draft v3 · 6 messages", "Message 4 of 6 · Ben ..."), the child's "this one" hint on `r` `f`, reply on a collapsed row answers the latest, forward from a child opens on "This message", `e` takes the whole thread out and `z` brings it back, and `alt+v` on a collapsed row moves nothing and is not offered. | 02 |
| `window-walk` | US3 #1 to #4 | `[` `[`, `]`, `}`, `{`, `g u`, `j` | The thread bar's current chip rings as the window steps older and newer; the older-message banner appears away from the latest and goes on `}`; `]` on the latest does nothing; `g u` lands on the first unread; `]` marks the message read; `j` opens the next thread, not the next message. | 03 |
| `live-reply` | US4 #1, #2 | none until the arrival (`event = "new_mail"`), then `}` | A reply arrives while message 4 is open: a chip with an unread dot and the accent banner ("... Read it }") appear, the shown message and scroll do not move, the row behind updates its count and "1 new"; two replies name the latest and add both chips; `}` shows it. | 08 |
| `menu` | US5 menu, split (#4) | `alt+period`, `alt+shift+s`, `Escape` | Thread ▾ opens its two groups, whole thread then this message only, each entry with its key; split moves message 4 and its replies into a new thread, two threads in the list afterwards. | 04 |
| `outline` | US5 #5 | `alt+t`, `/` and a name, `Down`, `u`, `Return` | The outline lists every message with number, name, first line, date and unread dot; the field and person chips filter it; `u` goes to the first unread; `Return` shows that message and closes the outline with the window on it. | 05 |
| `mute` | US5 #1 to #3 | `m` on a row, `event = "new_mail"`, `m` again | A muted thread's reply does not reach the inbox, its row reads "muted · 1" and stays put; a reply addressed to the person alone does reach the inbox; `m` again unmutes and later replies arrive in the inbox. | none; the row's own state (nearest 01) |
| `search-open-at-match` | US6 #1 to #3 | `/`, type `rate limit headers`, `Return`, `Down` to message 3, `Return`, `mod+g` x2, `mod+shift+g`, `Escape` | Results group by thread, the top result expanded to its matching messages and one line for the one that does not match; `Return` opens the window at that message with every match marked in the bar and the "match 1 of 3" banner; `mod+g` steps across messages, says "last match" at the end; `Escape` returns to message 3's row. | 06, then 07 |
| `reply-older` | US7 #1 | `Down` or `[` to an older message, `r`, type text, `mod+shift+Down`, `alt+e` | Replying to message 4 of 6 shows "You're replying to message 4 of 6, not the latest"; `mod+shift+Down` shows the newer messages; `alt+e` keeps the typed text and switches the quote and threading to the latest. | 09 |
| `forward-thread` | US7 #2, #3 | `f`, `2`, `Return`; separately `f` on a one-message thread | The *What to forward* chooser opens on "This message" and `2` picks Whole thread (subject "Fwd: ... (6 messages)", six headed messages, attachments, `mod+shift+q` expanding the quoted rows); a one-message thread opens no chooser. | 10 |

The list and window layouts themselves (US1 #1, US3 #3, US5 #1 to #3 as data)
are asserted by unit and `focus_suite` cases, not filmed; a storyboard is
here only where a key moves the keyboard or changes what is on screen.
