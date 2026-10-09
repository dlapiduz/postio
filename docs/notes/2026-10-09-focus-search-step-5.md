# Focus search, step 5: Quick Look against screen 10

2026-10-09, specs/010-focus-search T086-T092. Space on a result opens
Quick Look over the results: a floating panel, 780 by 470, radius 14,
nothing dimmed. The results keep the keyboard -- the panel is a child
window that never becomes key -- so j/k still move the focus ring and the
panel follows in place, ]/[ ring the next and previous match card, Return
opens the message and closes it, `a` archives and moves it to the next
result (closing after the last), and Space or Esc's first rung closes it.
A new query, sort, tab, filter popover or leaving the results takes it
down. Everything decided is the controller's (`postio-focus/src/results.rs`,
`QuickLook`); the cards are `postio_session::search::conversation_matches`.

## What a card is

Every match in the conversation, oldest first: each message's own words
("Body", with who wrote it and when), its quoted history ("Earlier
reply") only when the conversation does not hold the mail it quotes, a
matching file name, and the subject once, last. A quote of a message the
conversation holds is that message's card already and is not said twice
(compared with case and spacing folded, since a quote rewraps). The ring
starts on the result's own message. Passages are cut as the results'
rows cut theirs (`FirstLine::Avoided`), never from an attribution line.
The panel draws at once from what the row knows and redraws when the
conversation's matches land (`Request::QuickLookMatches`, on its own lane,
so j supersedes a read nobody will draw).

## The capture

`screencapture` still has no grant; the demo's snapshot composites the
panel (a window of its own) over the main window:

```bash
POSTIO_FFI_FEATURES=demo scripts/macos-build.sh && scripts/macos-bundle.sh
POSTIO_DEMO=search POSTIO_WINDOW_SIZE=1440x900 POSTIO_APPEARANCE=light \
  POSTIO_DEMO_KEYS='/ atlas ␣ budget ⌘⏎ @from ␣ ⏎ @date since ␣ july ⏎ ⌥o ␣' \
  POSTIO_DEMO_SNAPSHOT=/tmp/10.png macos/build/Postio.app/Contents/MacOS/Postio
```

The window server's shadow is not in a snapshot; a capture with the grant
should be taken before the review.

## Screen 10

| # | Design | Built | Decision |
|---|---|---|---|
| 1 | Panel at 330, 160 in the 1440 by 900 window, 780 by 470, radius 14 | The same: centred across the window, 160 under its top, kept inside a smaller one | Same |
| 2 | Deep shadow (0 30 90, 35 %), hairline ring, no dimming | The window server's panel shadow and a separator hairline; nothing dimmed | **Explained**: the system shadow is the platform's; not in the snapshot |
| 3 | Header 46 tall on the toolbar tint: "Quick Look" bold, "1 of 12 · j k moves through results while it stays open", Open ↩ (filled), Archive a, Close Space | The same words and keys, from `postio_ui::search_view`; first capture's tint was the window grey | **Fixed**: the tint is a faint lift off the panel's white |
| 4 | Subject 22/28 bold with Atlas and budget marked | The same | Same |
| 5 | Sender line "Ada Moreno ada@example.org · to you, Finance team · Sat 26 Sep, 15:51 · thread of 3" | "Ada Moreno ada@example.com · Fri 9 Oct, 13:59 · thread of 3" | **Explained**: no recipients. Neither the row nor the matches carry the best message's To; adding them is a read of their own, left for when the reader's header words are shared. The date is the seed's |
| 6 | "4 matches in this conversation · ] [ jump between them" | "3 matches …" with the same hint | Same; the count is the seed's |
| 7 | Cards: a 120 column with where (600, secondary) and when (tertiary), the passage 14/22, 8 apart, on #f5f5f7; the current one on the accent's tint with a 2-point ring | The same; first capture's cards were the window grey and a file's name italic | **Fixed**: the faint tint, and the name upright as the design sets it |
| 8 | Order: Body (the result's message), the file, Earlier reply, Subject | Oldest message first, each message's matches together, the subject last; the ring on the result's own message | **Explained**: the plan's order (T087); a conversation reads oldest first, as the reader stacks it. The design's order is one message's sources, which is what a conversation of one gives |
| 9 | "Earlier reply · You · 24 Sep" | The seed's thread quotes only mail it holds, so no Earlier reply card; one, when shown, has no who or when | **Explained**: a quote does not say who wrote it reliably (attributions are free text in every language); the message it quotes, when held, is its own "Body" card with its writer |
| 10 | Body passage whole: "Hi Diego, the final Q3 numbers … close the quarter?" | "Done. The final Q3 numbers … so finance…" | **Explained**: a passage is the ~120-character window every row cuts (D7); the words are the seed's |
| 11 | The file card: "Atlas-Q3-budget.xlsx · Sheet “Q3”, row 3", the cell's words | The file's name as both place and passage, with who and when | **Explained**: attachment contents are step 9; `Source::FileContent` already draws its location ("Sheet ‘Q3’, row 3") in that column |
| 12 | The chips "from: Ada Moreno", "after: 2026-07-01" | First capture "from:Ada Moreno", no gap | **Fixed**: 4 points between the operator and its value. The name itself is this step's T066 fix (the chip read the address) |
| 13 | Footer "j k move · Space Quick Look · ↩ open · x select · / edit query · Esc back to inbox" | Step 3's footer had no Quick Look | **Fixed**: `results_hints` names Space |
| 14 | "1 of 12", Newest, the July–September rows | "1 of 6", the seed's | Same |

## Left as it is

- **G or End with Quick Look open, onto a row whose page is not read**:
  Quick Look closes rather than waiting for the page; Space opens it again
  once the row is drawn. Rare, and holding a pending look is more state
  than it earns now.
- **An archived result stays in the list** under `in:Inbox` until the query
  is asked again: search spans every folder, so it still matches, and the
  row's folder is not rewritten locally.
- **Undo and Refresh were swallowed in the results** (every verb dispatch
  was taken for a verb on the focused result); fixed in this step, so ⌘Z
  after `a` takes the archive back.
