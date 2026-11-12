# Focus threads, step 4: the message window's thread bar against screen 03

Spec 013, User Story 3 (T049). The `threads` demo seed was opened on
Harbor API draft v3 and stepped back to message 4 with `[`. On the Mac
the screen was taken with the in-app snapshot (`POSTIO_DEMO_SNAPSHOT`),
because `screencapture` failed in that session. On Linux the window was
rendered on the verify box. Each was compared with the maintainer's
local `Design/focus-macos-threads/screens/03-window-older-message.png`,
which is not in the repository.

Harbor opens on message 5, its first unread (FR-013), so one `[` reaches
message 4, where the design's state needs two.

## What matches, on both platforms

- **The thread bar** under the action row, with a hairline below:
  - the "Thread" label over "6 · 2 new";
  - a chip per message (avatar over "Name · time");
  - the current chip ringed and filled in the accent, since it is the
    focus;
  - unread chips dotted in the accent;
  - "… N more" when the chips do not fit the width the window reports.
- **The kicker** over the subject: "Message 4 of 6 · ] newer · [ older".
  Its keys come from the keymap.
- **The older-message banner**: it says reply and forward act on this
  message while the filing keys act on the thread, and ends with
  "Latest }".
- **The Reply to row** in the sender block: "Ana Ruiz, Mon 21 Sep
  (message 1)", only when the message answers one other than the one
  before it.
- **Keys**: `[ ] { }` and `g u` step in place without moving the list,
  and `j`/`k` still move to the next and previous thread.

## Differences, and why

| Difference | Platform | Why |
|---|---|---|
| Chip clicks do nothing | both | The controller has no `ThreadBarChipClicked` input yet. T085(d) |
| Thread ▾ is inert | both | The thread menu is US5 (T062–T068) |
| The window's title line reads "Message 1 of 10"; the design reads "Thread of 6 · 2 unread" | Mac | The title line predates this spec's walk. To align with the kicker when US5 touches the window |
| Avatars are neutral; the design gives each person a colour | both | As in the list (step 2–3 note); settle once for both |
| The banner sits slightly lower and lighter than the design | Mac | Spacing within the semantic fill; left as is |
| The Reply to label column is 64 pt; the other sender rows use 44 | Mac | To align with the sender block's column |
| Chips are left-packed with a 4 pt gap; Thread ▾ is 88 wide (76 truncated its label) | Mac | The label's width at the system font |
| Names and times differ from the design's | both | Invented seed data |

## A regression found on the way, and not one

`a_hundredth_open_builds_no_second_message_view` hung once each open
asked for its thread. The cause was the GTK test harness, not the app:
tokio's cooperative budget inside `settle()` (`docs/gotchas.md`, commit
`3d02916e1`). Guards now pin one thread read per open, and forty opens
drawing forty bars.
