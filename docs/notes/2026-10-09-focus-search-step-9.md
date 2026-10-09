# Focus search, step 9: attachment contents and the Files tab against screen 11

2026-10-09, specs/010-focus-search T126-T131. A word only inside a
downloaded attachment now finds its conversation, tagged with the file
and the place in it ("Sheet ‘Summary’, row 14: …"); the results' Files
tab is a grid of cards; Space hands the system's Quick Look a copy of the
file and ⌘↓ a save panel.

## What was built

- **The search arm** (`postio-index`, `executor/conversations.rs`):
  `HITS_JOIN_WITH_FILES` is `HITS_JOIN` with a third arm over
  `attachment_passages`, grouped by content so a sheet whose every row
  says the word is one hit. Only the conversation search has it: GTK's
  `search` keeps `HITS_JOIN`'s corpus (D11). **Keep the two joins in
  step**: main's #1812 (`INDEXED BY idx_messages_content`) is not on this
  branch yet, and when it lands it is owed in both strings. Relaxation
  counts still use `HITS_JOIN`, so a way out of no results does not count
  file matches.
- **Where in the file** is read only for the shown rows that matched in
  a file (`executor::file_matches`, one statement); the budget stays at
  five statements (`index_suite::search_statement_budget`, now with
  attachment shapes). `contents_complete` is "no downloaded attachment is
  waiting for the extractor", asked inside the folders read.
- **Passages** of a `FileContent` match are cut from the located unit,
  a page's in one read (`index::attachment_units`); Quick Look's
  conversation matches list each attachment's first matching unit.
- **The Files tab** (`executor::files`, `Req::Files`): one card per
  attachment whose name or contents match, or for a query with no words
  every attachment of the matched mail, newest first. A file never
  downloaded matches by name and has no contents line; nothing is fetched.
  The tab's count is the cards' once they are read, the search's
  attachment count before (the two agree for a query with no words).
- **Copies for the system** (`Req::AttachmentCopy { attachment, dir }`):
  bytes already on this machine only, written under
  `postio_focus::file_copies()` (the user's temporary folder, which is the
  app's on the Mac), removed when the panel goes or anything that closes
  Quick Look does (FR-053). The controller owns it: `Request::AttachmentCopy`
  → `Intent::FileCopy` → `UiEvent::FocusFileCopy`, closed with
  `focus_search_file_done`. That replaces the contract's synchronous
  `focus_search_quick_look_file`.
- **Keys**: `SaveFile` (`mod+Down`, Results, Apple only) is new. The
  grid's bare arrows are the collection view's (`KeyDisposition.belongsToGrid`):
  a card across, a row of four down, reported with `focus_search_point`.
  j/k step the ring as the list's do.
- **The demo** (`POSTIO_DEMO=search`) now stores the seed's files'
  bytes under the test keys, so the attachment indexer reads them.

## Measured

`search_focus`, release, Apple M1 Pro, the step-1 corpus plus 2,000
extracted units (ten pages each of 200 attachments; one attachment in ten
says `kestrel`, which no mail says). GTK dev-dependencies taken out for the
run and put back, as step 1's note says. Load average 6-7 during the run
(another session compiling). 60 timed runs after 5 warm-ups.

| shape | query | p50 ms | p95 ms | stmts | found |
|---|---|---:|---:|---:|---:|
| one word | `quarterly` | 6.36 | 6.49 | 3 | 203 |
| two words | `quarterly forecast` | 4.33 | 4.74 | 3 | 7 |
| operator only | `from:sender3` | 8.21 | 9.23 | 4 | 479 |
| operator + words | `from:sender3 regarding` | 18.30 | 19.71 | 4 | 460 |
| **common word** | `regarding` | 52.79 | **53.97** | 3 | 1,665 |
| typed `a` | `a` | 42.15 | 43.16 | 3 | 1,560 |
| **typed `at`** | `at` | 53.15 | **54.49** | 3 | 1,530 |
| typed `atl` | `atl` | 7.54 | 7.92 | 3 | 366 |
| **completions** | `a` | 25.81 | **26.36** | 4 | 2 |
| completions | `at` | 2.28 | 2.34 | 4 | 2 |
| completions | `atl` | 14.16 | 15.42 | 4 | 2 |
| completions | `from:a` | 1.55 | 1.58 | 1 | 4 |
| zero hits, four filters | (step 1's) | 1.26 | 1.28 | 2 | 0 |
| **relaxations** of that | five variants | 2,508 | **3,003** | 5 | 1 variant |
| e2e one word | `quarterly` | 7.70 | 8.11 | 56 | 203 |
| e2e two words | `quarterly forecast` | 4.59 | 4.68 | 13 | 7 |
| e2e operator + words | `from:sender3 regarding` | 20.55 | 21.86 | 57 | 460 |
| **e2e common word** | `regarding` | 57.94 | **59.20** | 56 | 1,665 |
| e2e typed `atl` | `atl` | 10.41 | 10.95 | 56 | 366 |
| preview check a person | `quarterly from:sender3` | 2.82 | 3.08 | 4 | 9 |
| preview exclude a person | `quarterly -from:sender3` | 8.95 | 9.52 | 4 | 194 |
| preview check a label | `quarterly label:atlas` | 1.87 | 2.15 | 4 | 21 |
| preview check a folder | `quarterly in:inbox` | 7.31 | 7.88 | 3 | 203 |
| **timeline range + word** | `quarterly after:… before:…` | 104.24 | **104.95** | 3 | 24 |
| **date words since + word** | `quarterly after:…` | 91.44 | **92.42** | 3 | 24 |
| timeline range alone | `after:… before:…` | 11.22 | 11.76 | 3 | 352 |
| timeline range + operator | `from:sender3 after:… before:…` | 3.67 | 3.79 | 4 | 66 |
| file word (new) | `kestrel` | 4.84 | 5.25 | 4 | 20 |
| e2e file word (new) | `kestrel` | 5.23 | 5.37 | 8 | 20 |
| Files tab, file word (new) | `kestrel` | 3.74 | 3.96 | 3 | 20 |
| Files tab, one word (new) | `quarterly` | 4.91 | 5.17 | 2 | 0 |
| Files tab, operator (new) | `from:sender3` | 7.31 | 8.59 | 3 | 350 |
| e2e preview check a person | `quarterly from:sender3` | 2.76 | 2.88 | 16 | 9 |
| **e2e timeline range + word** | `quarterly after:… before:…` | 100.75 | **102.32** | 30 | 24 |

- **The third arm costs little.** Every shape step 8 measured is within
  its run's spread of step 8's first run (`one word` 6.5 against 7.3,
  `common word` 54.0 against 55.0, `e2e common word` 59.2 against 118.6
  and 74.3). The file shapes are 4-9 ms; `file word` is one statement more
  than `one word` because its page's rows matched in a file
  (`file_matches`), and `e2e file word` one more again for the passages'
  units.
- **Over budget, none of it new**: the common word, `typed at` and `e2e
  common word` (54-59 ms against 50: step 1's 10% headroom, gone under
  this load as step 8 found), `completions a` (step 8's stop rule, the
  maintainer's), and the date shapes and relaxations: #1809, whose fix
  (#1812) is on main and not on this branch. Not worked around.
- `Files tab, one word` finds nothing because `quarterly` is only in
  bodies, and the tab is files whose name or contents match.

## The capture

```bash
POSTIO_FFI_FEATURES=demo scripts/macos-build.sh && scripts/macos-bundle.sh
POSTIO_DEMO=search POSTIO_WINDOW_SIZE=1440x900 POSTIO_APPEARANCE=light \
  POSTIO_DEMO_KEYS='/ atlas ␣ budget ⌘⏎ @from ␣ ⏎ @date since ␣ july ⏎ ⌘2' \
  POSTIO_DEMO_SNAPSHOT=/tmp/11.png macos/build/Postio.app/Contents/MacOS/Postio
```

Taken before T153/T154 moved extraction into a helper process: until
T155 bundles the helper, a demo bundle records its PDFs `skipped`, and the
PDF card shows no contents line.

### Screen 11, "atlas budget" from Ada since July

| # | Design | Built | Decision |
|---|---|---|---|
| 1 | Header "Files whose name or contents match", note "contents are indexed on this Mac for PDF, Office, iWork, text and images with text" | Same title; note "contents are indexed on this Mac for PDF, Office documents and text" | **Explained**: iWork documents and text in images are not read (below), and the note says only what is true; while the indexer has files left it says "still reading the contents of files on this Mac" |
| 2 | Four cards across, 12 between, 20 at the sides | Same | Same |
| 3 | Cards on the window's grey, radius 10; the focused one on the accent tint with a 2-point ring | Same | Same |
| 4 | 120-tall preview: a sheet's grid or a page's lines, the matching line in the find yellow; the JPG a photo's grey | Same, drawn; a deck gets a slide's frame; the mark sits on the row or line the location names, a page's in its middle | Same. Real thumbnails (QuickLookThumbnailing) were not taken: they would read every file to draw a card, for a picture of a file the next keystroke opens in Quick Look |
| 5 | Type tiles XLSX, PDF, NUM, JPG on their tints | XLSX, PDF, PPTX on the system green, red and orange | **Explained**: the seed has no Numbers file or photo; the tints are system colours at the design's strengths, since a literal colour is refused (`SemanticColourTests`) |
| 6 | Eight cards: two workbooks, PDFs, a Numbers table, a photo | Four: the Q3 workbook, the September actuals, the review deck, the template | **Explained**: the seed's files that match from Ada since July |
| 7 | "Ada Moreno · 26 Sep · 48 KB" | "Ada Moreno · 9 Oct · 3.2 KB" | Same words; the seed's dates and sizes |
| 8 | "Sheet “Q3”, row 3: Total Atlas budget 1,240,000" | "Sheet ‘Q3’, row 1: Atlas budget, Q3 final" | **Explained**: single curly quotes, as the spec's own words and step 8's #17; the row and words are the seed workbook's |
| 9 | "in “Re: Atlas Q3 budget, final numbers”" | "in ‘Re: Atlas Q3 budget, final numbers’" | **Explained**, as 8 |
| 10 | Tabs "Conversations 12 · Files 9 · People 1" | "Conversations 7 · Files 4 · People 2" | Same; the seed's. Files counts the cards once read; before ⌘2 a word query's count is the matched mail's attachments, which can be more |
| 11 | Sub-line "from Ada since July · 9 files" | "4 files · 2 people · last 12 months" | **Not fixed**: the results' sub-line is step 3's, the same on every tab; the design's words for a narrowed query are its own and want a rule in `postio-ui` |
| 12 | Sort "Newest" | "Best match" | **Not fixed**: the query's sort (it has words); the cards are newest first whatever it says. Hiding or changing it on the Files tab is a design question for the review |
| 13 | Footer "↑↓←→ move · Space Quick Look · ↩ open the message · ⌘↓ save file · ⌘1–3 switch tab" | "↑/↓/←/→ move · Space Quick Look · ↩ open the message · ⌘↓ save file · ⌘1/⌘3 switch tab" | Same keys and words; a pair is spelled with its slash (`hints::pair`), as everywhere else |
| 14 | Footer right "12 conversations · local index · 41 ms" | "7 conversations · local index · 4 ms" | Same |
| 15 | Timeline Jul-Sep selected | Jul-Oct | **Explained**: the demo's today is in October, so "since July" runs to it |
| 16 | — | First capture: the system focus ring on the Conversations tab button | **Fixed**: the table left the window with the keyboard, which went to the next key view; the grid now takes it after SwiftUI has swapped them in |
| 17 | — | A thin scroller at the grid's right in the second capture | Left: the system's, shown while the grid settles |
| 18 | Space: the system's Quick Look on the file; ⌘↓: a save panel | Built; not in the capture | **Explained**: both are the system's own windows, outside the snapshot, as the storyboard's `look` and `save` steps are skipped for the runner. `ffi_suite::the_files_tab_crosses_as_cards_and_space_hands_over_a_copy` proves the copy and its removal |
| 19 | Arrows move the ring | Built (`FilesGridTests`) | `POSTIO_DEMO_KEYS` goes through the resolver, not the grid, so a demo capture cannot press them |

## Out of scope: iWork and text in images

The design's note promises iWork documents and "images with text" (Vision
OCR). Neither is read: `postio-extract` is a pure leaf (FR-052) and reads
PDF, OOXML and text. An iWork file is a zip of protobuf (`.iwa`, Snappy
framed) with no public schema; reading it is a parser of its own to
maintain against Apple's format changes. Text in images needs Vision,
which is the Mac's alone, an inference engine the leaf may not hold, and
Linux would have no equal. Both are `Location::ImageText` /
later-step questions; the header's note says what is read instead.
