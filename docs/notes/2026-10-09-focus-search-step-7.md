# Focus search, step 7: no results and its ways out against screen 13

2026-10-09, specs/010-focus-search T102-T107. A search that finds nothing
shows, in the rows' place, "Nothing matches all four filters", one
sentence, and the looser searches that would find something -- each drops
or loosens exactly one term, says so in words, shows the query it runs and
its count, and is numbered. 1-4 run one; j/k move the focus among them and
Return runs the focused one; ⌘⌫ clears the filters and keeps the words
(D24). Everything decided is the controller's
(`postio-focus/src/results.rs`); the Mac draws `FocusRelaxations`.

## What happens, in order

1. The results frame lands with no conversations. The controller draws
   the page at once -- title, sentence, "Counting looser searches…", how
   much was searched -- and asks `Request::Relaxations` on a lane of its
   own (`Lane::Relaxations`): the counts are one search each, and waste
   the moment the query changes. Typing is never behind them.
2. The counts land (`Client::relaxations`, which drops zeros and sorts).
   The controller keeps those above zero, most first, and four at most,
   and redraws the page and the footer ("1–N loosen a filter"). An answer
   for a query since changed is dropped by its stamp. A failed answer, or
   one with nothing above zero, says "Loosening any one of them still finds
   nothing…" instead of counting forever.
3. A way out runs as a term edit does: the query is replaced in place, so
   ⌘[ goes back past it (as past a chip's ✕), the page goes
   (`FocusRelaxations(None)`) and the results are asked again.

The relaxation counts are slow on `main` today (~600 ms a variant with a
date, regression #1809, the maintainer's to decide). Nothing here works
around it; the page simply does not wait for them.

## Honest about attachment contents

`ConversationResults::contents_complete` was `true` because nothing reads
an attachment's text yet, so "nothing is outstanding". Read by the page as
"the search looked inside the files", that would have printed "including
attachment contents" -- a claim no index backs. The executor now says
`false` until step 9 extracts them (T126 makes it "every downloaded
attachment is read"), and `postio_ui::search_view::searched` drops the
clause whenever it is false (US6 scenario 2). "all" likewise needs
`corpus_complete`; an index still backfilling says "Searched 18,204
messages on this Mac. Some are still being indexed."

## The capture

```bash
POSTIO_FFI_FEATURES=demo scripts/macos-build.sh && scripts/macos-bundle.sh
POSTIO_DEMO=search POSTIO_WINDOW_SIZE=1440x900 POSTIO_APPEARANCE=light \
  POSTIO_DEMO_KEYS='/ from:ada@example.com ␣ has:attachment ␣ before:2026-08-01 ␣ subject:budget ⌘⏎' \
  POSTIO_DEMO_SNAPSHOT=/tmp/13.png macos/build/Postio.app/Contents/MacOS/Postio
```

The query is the seed's analogue of screen 13's (`from:ada has:attachment
before:2026-03-01 subject:"budget v4"`): the demo's attachments are all
from the last two months, so a date in March finds no way out but one.

## Screen 13

| # | Design | Built | Decision |
|---|---|---|---|
| 1 | "Nothing matches all four filters", 20 bold, 70 below the filter bar, the column 560 wide and centred | The same | Same |
| 2 | "Each line below loosens one filter and shows how many conversations you would get. Pick one, or press its number." | The same words, wrapped one word later | Same |
| 3 | Four ways out: "Remove “before March”" 4, "Look for “budget v4” anywhere, not just the subject" 2, "Remove “has attachment”" 1, "Anyone, not just Ada Moreno" 3 | Two: "Remove “has attachment”" 9, "Remove “before August”" 3 | **Explained**: the seed's mail. Every other single change still finds nothing, and a way out that finds nothing is not offered (FR-030). The words for the other kinds are `relaxation_line`'s tests, screen 13's among them |
| 4 | The design's rows are not in count order (4, 2, 1, 3) | Most first | **Explained**: the spec orders them by count (FR-030, SPEC §3.10 "order them by count"); the picture's data is not |
| 5 | Row: number cap, words 13.5 over the query in mono 11.5 tertiary, the count bold on the right, 50 tall | The same | Same |
| 6 | Query line one line: "from:ada has:attachment subject:"budget v4"" | First capture cut the first row's query in the middle ("bef…2026-08-01"): the seed's query names a whole address | **Fixed**: the query wraps to a second line rather than hide a term, and the row gained 8 of vertical padding so a wrapped row breathes |
| 7 | First row on the accent's tint with a 2-point accent ring | The same; the ring is rounded at all four corners | Same; the design's ring follows the box's corners only at its top -- not worth a shape of its own |
| 8 | The ringed chip is `before:`, the term the first way out drops | `has:` is ringed: the first way out here drops `has:attachment` | Same rule, different data |
| 9 | "Searched all 18,204 messages on this Mac, including attachment contents." | "Searched all 459 messages on this Mac." | **Explained**: the seed's size, and no attachment contents are read before step 9, so the clause is not claimed (see above) |
| 10 | "Mail older than January 2019 isn't downloaded: **search the server too** ⌥↩." | Absent | **Explained**: searching the server is out of scope (S4); nothing offers it, here or in the footer |
| 11 | Field hint "⌘⌫ clears filters", the chips staying, the whole field ringed 2 pt in the accent | The same hint, as glyphs from the keymap; the field keeps its hairline | **Not fixed**: the design rings the field in this state (`fieldRing`), not only when it has focus. The toolbar's `ChipQueryField` draws its border in AppKit and knows nothing of the results' state; ringing it means handing it the page's presence, which is a toolbar change for its own review. The focused chip's ring, which says which term the first way out drops, is drawn |
| 12 | Timeline hidden | Hidden | Same |
| 13 | Footer: "1–4 loosen a filter", "⌘⌫ clear filters", "⌥↩ search the server"; right "Searched 18,204 messages · 41 ms" | "1–2 loosen a filter", "⌘⌫ clear filters"; right "Searched 459 messages · 3 ms" | Same, but for the server (S4) and the seed's count |
| 14 | Tabs Conversations 0, Files 0, People 0; buttons From: Ada Moreno, Before March, Attachment applied | The same, "Before August" | Same |
| 15 | Chip `from: Ada Moreno` | The same: the chip names the person, the query keeps the address | Same |

## Left as it is

- **⌘⌫ with hits** is not the results' to answer; the controller answers
  `BackToWords` only while nothing matched (D24: one meaning per key and
  place).
- **Picking a way out keeps no history entry** for the search that found
  nothing, as a chip's ✕ keeps none; ⌘[ goes back to the inbox.
- **A query of only filters**, cleared with ⌘⌫, becomes the empty query,
  whose results are every conversation.
