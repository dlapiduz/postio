# Focus on the Mac, phase 9: Filtered, the digest, rules and capture against screens 21 to 25

2026-10-08, specs/009-focus-macos T118 (FR-061). The ninth Mac comparison,
for US9 (phase 11 of tasks.md).

**Captures.** `scripts/macos-shot.sh` over the `small` demo at 1440×900,
light and dark, the state reached with `POSTIO_DEMO_KEYS` through the
resolver and `invoke`, as a press would go:

| Capture | Seed | Keys | Reference |
|---|---|---|---|
| `21-filtered` | `small` | `g f` | screen 21 |
| `22-digest` | `small:22` | `j ⏎` | screen 22 |
| `22-digest-list` | `small:22` | `j ⏎ ⇥` | (Tab, the list page) |
| `23-digest-email` | `small:23` | `j ⏎ ⏎` | screen 23 |
| `24-digest-rule` | `small` | `j j d` | screen 24 |
| `25-capture` | `small:25` | `j j j j t ⌘p` | screen 25 |

Captures live in the main checkout's untracked `Design/review/focus-macos/`.

**Demo configuration.** The Mac enforces C6 at the read (with no
`[focus.model]` digest summary the summary is not read), and capture opens
only with a vault (C9). So `small:22`/`small:23` add a `[focus.model]` with
only `digest_summary` on, at a socket path that does not exist -- nothing
can be reached and nothing asks, since the demo's summary is already
written -- and `small:25` adds a throwaway vault with projects
(`postio_demo::demo_vault`), as GTK's `shot` does
(`crates/postio-ffi/src/demo.rs`, demo builds only).

## What matches

- **21.** Filtered in the list's place: "‹ Inbox", the title and its line,
  the seven reason tabs with counts (All on), the note on the right, the
  day heading, rows of sender, subject and first line, the reason pill and
  the time; the focused row ringed in the accent with "Restore, never
  filter this sender" and `R`; the footer's keys.
- **22.** Its own window over the undimmed list, 720 wide at 1440 with a
  560 column (M1): the stacked icon, title and line, Archive all as a
  filled default button with `⇧A`'s cap; Summary and the list tab with
  Tab's cap, the rule line with "Edit rule and cadence" and `d`; the
  topics, each statement ending in its numbered chip, the focused
  reference marked in the accent with its card under its topic.
- **23.** In the same window and the same size: "‹ Summary" with Esc's
  cap, the subject and its position, the `k j` cap and the stepper on the
  right, the citation banner in the accent at 8%, the message's subject,
  sender block and body as the message window draws them, the cited
  passage underlined in the accent.
- **24.** Cancel with Esc, "Digest this sender", Create filled with
  Return's cap; From pre-filled in mono on a quaternary field; Weekly on
  Sunday at 09:00; "Would have caught N messages in the last 90 days"
  with the newest in a bordered list and "and N more"; the note; "Match a
  list or a search instead…". The list behind dims, as a sheet does.
- **25.** Cancel, Task/Note with `t`/`n`, Add task filled with ⌘↩; the
  From line; the Task field ringed while it has the keyboard, the
  sentence verbatim and "⌥S use the subject instead"; Due with its quick
  picks, the chosen one outlined; the project with its reason and Change
  ⌘P, the vault's projects listed with their notes and open counts, the
  chosen one ringed; the exact line with its `postio://` link before the
  date (C21); the footnote.

## Differences, each fixed or explained

1. **Fixed: "‹ Summary" had no chevron.** The controller's word is
   "Summary"; the chevron is the drawing's, as Filtered's "‹ Inbox".
2. **Filtered's header is under the main toolbar, not in it.** The pack
   replaces the toolbar's compose and search with "‹ Inbox" and the
   title; the Mac keeps its toolbar (search and the sync label stay
   reachable) and draws Filtered's header in the list's place. The pack's
   "Sweep the inbox…" is absent from screen 21; the controller offers it,
   so it is drawn on the right of the header.
3. **The note says "Nothing here is deleted automatically", not "Kept for
   30 days, then deleted".** C4, and the words are the controller's.
4. **Counts and senders are the demo's** (9 filtered, a weekly digest of
   6 from 5 senders, two topics), not the pack's 186 and 14.
5. **No hint footer under the summary.** The pack draws "] [ next /
   previous reference · ↩ open email · Tab summary / messages · D stop
   digesting sender"; `DigestViewFfi` carries the card's key and the tab
   key but no footer hints, and the Mac does not compose words of its
   own. A shared `postio_ui::digest` footer would close it.
6. **The focused reference's card is its title and key only.** The pack's
   card shows the email's opening lines; that is M3's "the reference's
   email under its paragraph", T123.
7. **No action row on the email from a digest.** M3, T124 (Reply, Forward,
   Archive, Note, Label, Unsubscribe, Stop digesting). `U` and `D` work as
   keys meanwhile.
8. **The email's position line is the controller's "Source 1 of 6"**, not
   the pack's "Weekly · Newsletters › source 6 of 14".
9. **The rule sheet hangs from the title bar** (AppKit's `beginSheet`), so
   it sits at the top of the window rather than over the middle of the
   list, and its link is drawn in the accent (links are one of the
   accent's uses, contracts/mac-surfaces.md) rather than underlined in
   the label colour.
10. **Capture is a window with a title bar** ("Task") and traffic lights,
    as the pack's SPEC and M4 say (email, digest, compose and capture are
    windows); the PNG draws a sheet-like panel. Its height is screen 25's
    (620), not the message window's.
11. **Capture's Due line is "Due" and the day**; the pack adds "· from
    “by Wednesday”", where the controller does not say which words the
    day came from. The project list is in the vault's order, with the
    chosen one ringed, rather than the chosen one first.
12. **C22 spelling:** caps are the keymap's (`⇧A` is `A`, Escape `⎋`,
    Return `↩`), as everywhere on the Mac.

## Not compared here

- The digest rules list (`g d`) and "Digest mail like this" (`L`): no Mac
  task in this phase draws them; they stay in `KNOWN_ORPHANS`.
- `postio://` from another app: routed and tested
  (`LinkRoutingTests`), and the scheme is registered in `Info.plist`;
  opening a captured line from Obsidian on this machine needs the bundle
  registered with Launch Services, which is T128's walk.
