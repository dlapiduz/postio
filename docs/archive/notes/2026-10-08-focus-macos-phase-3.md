# Focus on the Mac, phase 3: the message window against screen 04

2026-10-08, specs/009-focus-macos T071 (FR-061).

**Captures.**
- Made with `scripts/macos-shot.sh` over the in-memory `small` demo store.
  The main window is 1440×900 and 1024×768, in light and dark.
- The message window is a child of the main window, and
  `screencapture -l` takes a window with its children. So it is
  photographed over the list, as screen 04 draws it.
- Each state is reached by pressing keys once the list has landed
  (`POSTIO_DEMO_KEYS`, demo builds only). `⏎` and `⎋` now press Return
  and Escape:
  - `04-open-email`: `'j j j j ⏎'`, the to-do in a thread of six;
  - `mw-step-j`: then `j`, which shows the next row in the same window;
  - `mw-escape`: then `⎋ j`, so the window is closed and the list has
    the keyboard;
  - `mw-more`: `.` at 1024, where More holds Label, Move and Delete.
- The HTML bodies come from the corpus. A demo now names a screen after
  its seed, and the opened row is refiled as one of the handoff's bodies
  by `postio_demo::treatment_demo`, as GTK's screens 27–31 do:
  - `POSTIO_DEMO=small:27` is the newsletter that paints its own page
    (`mw-03-paper`, and at 1024 `mw-1024-paper`);
  - `small:28` is the office mail with its question (`mw-06-work-app`,
    and with `O` pressed `mw-08-work-paper`).
- Captures live in the main checkout's untracked
  `Design/review/focus-macos/`.
- References: `Design/focus-macos-design/screens/04-open-email.png` and
  `message-window/screens/01`–`04` and `06`–`08`. Screen 05 is the
  redlines, checked against the numbers below; 09 is the rule diagram.

Every key above went through the resolver and `invoke` to `postio-focus`'s
controller. Opening, stepping, closing, More, find, `O`, `v` and `[`/`]` are
its intents, applied by `FocusIntents.surface` and `MessageWindowModel`.
Every word around the body is `focus_message_view`'s, composed by the
`postio_ui` functions GTK's open message uses. The body is the engine's
treated document (M5). A difference below is a drawing difference, a
fixture difference or a recorded decision, not a second set of rules.

## What matches

**Geometry (M1, section 1).**
- At 1440 the window is 720 wide and 820 tall. It is centred on the
  main window, 40 below its top, and the list behind is not dimmed.
- At 1024 it is 655×688. Label, Move and Delete fold into More `.`
  (655 is below 700).
- `j` replaces the content in the same frame. Measured: the same
  720×820 before and after.
- The plain column is 560. The paper column is 640 at 1440 and 576 at
  1024, where the 640 newsletter is drawn at 0.9 and fits whole.

**Chrome (section 2).**
- The traffic lights are centred in a 52 pt title area. The subject is
  13.5 bold, with "Message 4 of 59 · thread of 6" under it in tertiary.
- On the right: one `k j` cap and the joined chevron pair.
- The 44 pt action row: Reply `e`, Reply all `E`, Forward `f`,
  Archive `a`, Snooze `s`, Remind `h`, Label `l`, Move `m`, Delete.
  The title area and the action row are one band, with the hairline
  under the row.

**Column (sections 3–5).**
- One centred column holds the thread chip ("Latest of 6 in this thread
  `[` earlier message"), the 26 pt subject, the label pill and
  "+ Label `l`".
- The sender block: hairlines above and below; From, To and Cc in the
  44 pt column; the name semibold and the address in mono; the date in
  mono on the right.
- The action card is the accent at 8% (12% in dark, C26): the outlined
  kind tag, the date in mono, the sentence quoted in italics, Snooze `s`
  (or Reply `e`) and Dismiss `-`.
- The same sentence is marked in the body: the accent behind it and a
  2 pt accent underline.
- The quote fold ("On Monday, … wrote:", "▸ 3 quoted lines") is the
  document's.

**Treatments (section 7).**
- **Paper.** The newsletter is drawn as sent, on a sheet with a hairline
  edge. In dark it is dimmed by black at 8%, which is `brightness(0.92)`
  exactly, and never inverted. The line above it reads "Original layout,
  on paper · this message sets its own background · Use app colours
  `O`".
- **App colours.** The office mail's ink, links, table and hairlines are
  the platform's (`-apple-system-*`) and follow dark. The line reads
  "App colours · sender colours and fonts removed · Show original `O`".
- **`O`.** It switches the treatment in place and keeps the place being
  read. On the work mail it offers "Always for this sender".

**Closing.** `⎋` closed the window, and the next `j` moved the list's
cursor, because the keyboard was back on the table.

## Differences, and why

| Difference | Why | Where it is settled |
|---|---|---|
| Names, subjects and counts differ; the newsletter and the work mail show "Latest of 6 in this thread" and a Harbor label, which the PNGs do not | `postio-demo`'s seed, and `treatment_demo` refiles the row that opens screen 04, which is a thread of six | Fixture |
| The work mail's card has no date where the PNG has "Fri 2 Oct" | The demo's question marker carries no due date | Fixture |
| The work mail's "Please confirm by Friday…" is the sender's red in light | The contrast guard keeps a colour the sender set on purpose where it reaches 4.5:1 (R9, `guard_kept_colours`). In dark the same colour fails and the ink returns | Recorded decision (R9) |
| Body text is Barlow; the PNGs draw SF Pro | A body in app colours is set in Barlow (C25), served over `postio-font:` | Decision C25 |
| The switch's cap reads `O` where the pack draws `⇧O` | Caps are the keymap's spelling (C22) | Decision C22 |
| Delete's cap is the keymap's primary binding's glyph, not `⌫` | `KeyCapSpelling` spells the first binding; `⌫` is the registry's Mac alternate | C22; a cap of the Mac default is M6's to settle in the registry |
| The window is 655 wide at 1024 where the pack draws 656 | M1's formula gives 655.36, and the shared geometry rounds it | Recorded decision, not a defect |
| The app-colours table's lines are fainter than the PNG's strong hairlines | `--r-hairline-strong` is `-apple-system-grid`, the closest semantic colour | Tune with the palette (section 6) |
| The action sentence's 8% fill is barely visible; its underline carries the mark | The accent at 8% over the content surface, as specified | Not a defect |
| `v` in the demo shows "This account is not syncing, so that message cannot be fetched" in mono | The demo's account has no server, and its messages keep no raw source | Fixture |
| Attachment chips are below the fold in `04-open-email` | The column scrolls as one, and the to-do's body is longer than the window | Not a defect |
| No "‹ Summary" in the title area, and no digest action row | A message opened from a digest is US9's (screen 23) | T100s |
| No "Load images" notice for held-back remote images | Remote images are asked for blocked. The per-sender grant is not wired into this window | tasks.md T069, As built |

Found and fixed on the way:
- **The marker card was missing on a to-do** (`9cb05683`). The card was
  looked up by the position line's index, which counts messages, while
  the list also draws a digest row.
- **A body in app colours was black on the dark window** (`d3d93e9c`,
  `color-scheme: light dark`). WebKit resolves the system's colours as
  dark only for a page that declares it can be drawn so.
- **The body was the classic reader's framed grey box**
  (`99248973`). The column now flows it, as GTK's open message does.
- **Body text was the system's sans** (`d482f8b4`). The
  `postio-font:` handler was set on the web view's copy of its
  configuration, which is never asked.
- **The window was 872 tall, not 820.** A toolbar keeps a window's
  content size and grows the frame (`529e20a5`).
- **A 640 newsletter at 1024 was cut, not zoomed** (`bf363636`). The fit read the
  page's width, and the reader's body box scrolls sideways on its own.
- **One-line messages opened blank.** Two causes:
  - The body was sized by the page's `scrollHeight`, which the classic
    reader's scroll anchors (at multiples of `vh`) inflate up to the
    clamp. It is now sized by its content (`889d8075`).
  - A demo opened behind the terminal, and WebKit stops painting a web
    view in a window it judges covered. A demo now opens in front
    (`d65c1825`).
- **A focus ring on Reply, and a seam between the title and the row.**
  Focus effects are off in the window, and the toolbar's separator is
  off (`23d04eaf`).

## What this phase does not cover

These come with later phases, and each has its own comparison:
- compose and reply (05, 06);
- the digest's window and a message opened from it (22, 23);
- remote images in this window.
