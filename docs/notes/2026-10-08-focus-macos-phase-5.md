# Focus on the Mac, phase 5: the bar and the folders popover against screens 07 to 10

2026-10-08, specs/009-focus-macos T087 (FR-061). The phase numbering
follows tasks.md: phase 7 of the spec is US5, and this is the fifth Mac
comparison.

**Captures.**
- Made with `scripts/macos-shot.sh` over the in-memory `small` demo
  store, at 1440×900, in light and dark.
- The bar's panel and the folders popover are child windows of the main
  window. `screencapture -l` takes a window with its children, so both
  are in the picture, as the message window was in phase 3. A capture
  with the popover open is wider than the window, because the popover
  hangs past its left edge.
- Each state is reached by keys replayed once the list has landed
  (`POSTIO_DEMO_KEYS`, demo builds only). The replay now spells `⌘` and
  `⌥` before a key, `↓`/`↑`, `⇥`, and `␣` for a space. A word of several
  characters is typed whole into whichever field is up, through the
  field's editor.
  - `p5-07-bar-search`: `'/ harbour'`, the hits under the search row;
  - `p5-07-chips`: `'/ from:ada ␣ ␣ invoice ⇥'`, Tab into the first chip;
  - `p5-07-hit-open`, `p5-07-hit-back`: Return twice opens the first hit
    in the message window, and `⎋` from a hit returns to the bar;
  - `p5-07-escape`: `'j j / harbour ⎋ j'`, so the keyboard is proved back
    on the list by the last `j`;
  - `p5-08-in-rec`: `'/ in:Rec'`, Receipts' conversations in the bar;
  - `p5-09-commands`: `'⌘k arch'`;
  - `p5-10-places`: `'g o ↓ ↓'`; `p5-10-places-rec`: `'g o rec'`;
    `p5-10-receipts`: `'g o rec ⏎'`, the folder opened as the list.
- Captures live in the main checkout's untracked
  `Design/review/focus-macos/`.
- References: `Design/focus-macos-design/screens/07-search-plain-english-chips.png`,
  `08-go-to-folder.png`, `09-command-list.png` and
  `10-folders-and-labels-popover.png`.

Every key above went through the resolver and `invoke` to
`postio-focus`'s controller: `/`, ⌘K, `g o` and the go-to keys are no
longer the frontend's own (`Intercepted` and
`postio_ffi::registry::INTERCEPTED` both lost them). What the bar offers,
what each line says and runs, the chips, the echo, the headings and the
places are the controller's and `postio_ui`'s, as on GTK. The Mac draws
them and decides only where the arrows' highlight stands.

## What matches

**The bar (07, 08, 09).**
- It drops from the toolbar's search field as a borderless child panel.
  Its right edge is on the field's, its top is just under the toolbar,
  and nothing behind it dims.
- The keyboard stays in the field. ↑/↓ walk the lines, Return runs one,
  Tab steps into the chips, and Escape closes the bar with the keyboard
  back on the row it left.
- The saved row shows each name with its `⌥n` keycap.
- The chips are in mono, with the chip being edited ringed in the accent.
  The echo under them says what was typed and which chip is being edited.
- The "Conversations" heading carries the match count on the right.
  - Each hit is one line: the sender, the subject and its first line,
    `in:` the folder in mono, and the date in mono.
  - Return on "Search mail for …" moves the highlight to the first hit.
  - Return on a hit opens it in the message window ("Message 1 of 7").
  - Escape there brings the bar back on the same words, with the
    highlight on the hit last read.
- `in:Rec` lists Receipts' conversations under "Receipts · folder · 4
  conversations · newest first", with the highlight on the first.
- ⌘K opens with `>` typed (C24). `>arch` lists only commands, each with
  its keycap, and the first is highlighted.
- The footer matches the pack's words: "↑↓ move · ↩ open · ⌥1–4 saved
  searches · ⌘S save query" for search, and "↑↓ move · ↩ run · >
  commands only" for commands, with "Local index" on the right.

**The popover (10).**
- It hangs from Inbox ▾ (`g o`, or a click on the button). The filter
  says "Go to folder or label" and holds the keyboard.
- Mailboxes, Folders and Labels are listed in sections.
  - Each place has its mark: an SF Symbol by role, or a label's dot.
  - Each has its count (Filtered says "9 today") and its direct key
    (`g i`, `g t`, `g s`, `g z`, `g r`, `g f`, …).
  - The place the list shows is bold.
- The footer follows the highlight: "↵ open Sent · Esc close · same as
  in:Sent in the command bar".
- Typing filters. Return opens the place as the list, through the
  controller, with the cursor on row 0, and Inbox ▾ then reads
  "Receipts".
- Light and dark both follow the semantic colours. Nothing in Swift names
  a colour.

## Differences, and why

| Difference | Why | Where it is settled |
|---|---|---|
| The panel is 640 wide; the pack draws it about 860 | "As wide as the field or 640, whichever is wider" (contracts/mac-surfaces.md), and the field is 320 | Contract. Widening it is a one-line change to `CommandBarGeometry.minWidth` if the maintainer prefers the pack |
| The pack draws a query field inside the panel (the words, the caret, `Esc`). The Mac's words are in the toolbar's field, and the panel draws a chips row only when there are chips | The contract keeps the keyboard in the toolbar's field. A second field showing the same words would be two carets for one input | Contract; recorded in tasks.md T085 |
| 08's pack shows `in:Receipts` as a chip | The controller lowers words to chips only for a search (`command_bar::chips`). `in:` is routed to the folder's list and makes none, as on GTK | Controller (slice 8) |
| The saved pills have no counts ("Waiting on reply 5") | `BarViewFfi.saved` carries the names only | FFI gap, for whoever counts saved searches on both apps |
| The echo spells the key `cmd+BackSpace`, not `⌘⌫` | The controller composes the echo with `postio_ui::hints`, which spells the keymap's words. The Mac re-spells only keycaps it draws itself (C22, the gap noted at `KeyCapSpelling`) | C22: the shared short spelling is not exported |
| 09's commands have no emphasised match ("**Arch**ive") and no detail ("the focused message · Re: …") | `BarLineFfi` has no match positions, and the controller gives command lines no detail | Controller and FFI (slice 8) |
| 09 in the pack blends Commands, Go to and Search under typed `arch`; ⌘K here shows commands only | C24: ⌘K opens with `>`, which limits the bar to commands. `/` then `arch` gives the blend | Decision C24 |
| The saved row's "⌘S saves the current query" disappears at 640 when four saved searches fill the row | Shown whole or not at all, and never cutting a name; the footer still names ⌘S | Not a defect |
| The popover has an arrow and the system's popover material, where the pack draws a plain sheet | `NSPopover`, as the contract asks; its arrow cannot be hidden with public API | Contract |
| Label dots are grey | The demo's labels carry no colour; the list's pills show the same | Fixture |
| After going to Receipts, the strip still counts the inbox ("59 · 21 unread") | `focusStrip` composes the inbox's counts; a strip line for another place is not exported | Later work on the strip |
| `g f`, and Filtered in the popover, show "Filtered is not built on the Mac yet" | The Filtered view is T113's (US9) | T113 |

Found and fixed on the way:
- **Escape closed the panel but left the keyboard in the field**
  (`fix(focus): Back takes the bar off the stack itself`).
  - The controller's Back on the bar said `CloseSurface(Bar)` and kept the
    bar on its stack until the frontend said `SurfaceClosed`. A line run,
    by contrast, takes the bar off itself.
  - The Mac deliberately does not echo `SurfaceClosed(Bar)`. A label's Go
    closes the bar and reopens it on the label's search, and an echo
    landing late would close the new bar.
  - So Back now dismisses the bar as a run does, and sends the keyboard
    home.
- **The popover opened on Snoozed, not Inbox.** Before the places read
  lands, only the uncounted views are listed, and the highlight was kept
  by name when the read arrived. Now it is kept only when the arrows put
  it there.
- **The saved row cut every name** to fit its hint, and **`in:Inbox` was
  cut before a hit's first line was.** The first line was also left as a
  single stray letter after a long subject.

## What this phase does not cover

- The storyboards (T083) are filmed on Linux; this Mac cannot build GTK.
- GTK has not adopted the controller's bar (slice 8's As built), so the
  two apps' bars have the same rules but not yet the same code on GTK's
  side.
- The pickers (11 to 14) and the undo pill (15) are phase 6.
