# Focus search, step 8: typing intelligence against screens 02, 04 and 05

2026-10-09, specs/010-focus-search T108-T116. The dropdown now reads what
is being typed (design §2): one to three letters complete with a ghost and
suggestions, an operator's value lists people, labels or folders, and a
sentence the Mac lowered says what it understood, tile by tile. Every
decision is the controller's (`postio-focus/src/bar.rs`,
`dropdown.rs`); the engine answers `completions` (`postio-index`,
`executor/completions.rs`); the Mac draws `FocusDropdown`.

## Completions measured: over the 20 ms budget for `a` (stop rule)

`crates/postio-bench/benches/search_focus.rs`, release, Apple M1 Pro, the
step-1 corpus (20,000 messages) plus 2,000 contacts beside its 40 senders,
a quarter of them written to. 60 timed runs per shape after 5 warm-ups.
Run on this Mac with `postio-bench`'s GTK dev-dependencies taken out for
the run and put back (step 1's note says how); nothing about that is
committed.

**The machine was loaded**: other sessions were compiling throughout, load
average 46-57 on a 10-core machine at the end of the second run. Both
runs are recorded; the first is the less disturbed one, and every shape
step 1 also measured is slower in both than step 1 recorded, by about as
much as the load explains. Not a reference measurement: T138's final run
is.

| shape | query | p50 ms | p95 ms | p95 ms, run 2 | stmts | found |
|---|---|---:|---:|---:|---:|---:|
| **completions** | `a` | 27.70 | **28.49** | **35.34** | 4 | 2 |
| **completions** | `at` | 2.60 | 2.84 | 3.98 | 4 | 2 |
| **completions** | `atl` | 15.47 | 16.21 | **20.11** | 4 | 2 |
| **completions** | `from:a` | 1.62 | 1.72 | 2.14 | 1 | 4 |
| one word | `quarterly` | 6.65 | 7.27 | 8.31 | 3 | 203 |
| two words | `quarterly forecast` | 4.14 | 4.34 | 5.73 | 3 | 7 |
| operator only | `from:sender3` | 8.70 | 9.32 | 11.81 | 4 | 479 |
| operator + words | `from:sender3 regarding` | 18.91 | 19.60 | 24.75 | 4 | 460 |
| common word | `regarding` | 53.52 | 55.01 | 69.26 | 3 | 1,663 |
| typed `a` | `a` | 42.18 | 43.43 | 55.43 | 3 | 1,560 |
| typed `at` | `at` | 54.26 | 55.48 | 70.47 | 3 | 1,551 |
| typed `atl` | `atl` | 8.52 | 9.01 | 11.15 | 3 | 366 |
| zero hits, four filters | (step 1's) | 0.93 | 1.13 | 1.67 | 2 | 0 |
| relaxations of that | five variants | 2,664 | 2,732 | 5,990 | 5 | 1 variant |
| e2e one word | `quarterly` | 9.09 | 23.07 | 11.20 | 56 | 203 |
| e2e two words | `quarterly forecast` | 4.99 | 15.61 | 6.35 | 13 | 7 |
| e2e operator + words | `from:sender3 regarding` | 36.87 | 54.77 | 27.43 | 57 | 460 |
| e2e common word | `regarding` | 90.10 | 118.64 | 74.32 | 56 | 1,663 |
| e2e typed `atl` | `atl` | 9.48 | 10.01 | 18.53 | 56 | 366 |
| preview check a person | `quarterly from:sender3` | 2.22 | 2.26 | 5.70 | 4 | 9 |
| preview exclude a person | `quarterly -from:sender3` | 8.47 | 9.36 | 11.60 | 4 | 194 |
| preview check a label | `quarterly label:atlas` | 1.32 | 1.61 | 2.30 | 4 | 21 |
| preview check a folder | `quarterly in:inbox` | 6.80 | 7.37 | 11.14 | 3 | 203 |
| timeline range + word | `quarterly after:… before:…` | 102.43 | 104.21 | 132.32 | 3 | 24 |
| date words since + word | `quarterly after:…` | 90.33 | 92.71 | 230.50 | 3 | 24 |
| timeline range alone | `after:… before:…` | 10.93 | 11.65 | 35.11 | 3 | 352 |
| timeline range + operator | `from:sender3 after:… before:…` | 3.31 | 4.10 | 11.88 | 4 | 66 |
| e2e preview check a person | `quarterly from:sender3` | 2.47 | 2.64 | 13.27 | 16 | 9 |
| e2e timeline range + word | `quarterly after:… before:…` | 103.08 | 105.81 | 192.91 | 30 | 24 |

### Where `a`'s 28 ms goes, and what was not done about it

A run with timings printed around each statement (not committed) put the
documents read and the words ranked at about 1 ms; the rest is the one
count statement. For `a` the best completion in this corpus is `as`, which
every message's body says ("as of message N"), and its count is exact:
the conversations a search for `as` would find, which is a walk of the
whole mailbox to the cap -- the common-word shape's cost (55 ms there with
ranking and facets; ~27 ms for the count alone). `atl` (20 ms in run 2) is
the same story at 2% of messages.

So the budget is missed by the **exact count of a common completion**, not
by finding the words: no vocabulary table (D22's fallback) would change
that, because the table would give a document count, not the
conversation count the design and US7 ask the row to show. The ways out
are the maintainer's, and none was taken (stop rule):

1. Count the completed word as a floor -- the documents among those read,
   as the "did you mean" offer already does -- and show "50+", which
   breaks "every suggestion's count is the count the query would give".
2. Answer the suggestions without counts at once and count them in a
   second, cancellable request, as the relaxations are (the row draws its
   count when it lands).
3. Never complete to a word as common as `as` (a stop list, or a cap on
   document frequency among those read).

`from:a` is one statement over the contacts and far inside the budget.

### Also seen

- **The word offered is the best documents', not the commonest.** The
  fifty documents read for `at*` are the index's best-scored, not a
  sample of the mailbox, and in this corpus they say `atl` (2% of
  messages) more often than `atlas` (about half): `at` completes to
  `atl`. The demo's mail is too small to show it. A vocabulary table
  would fix this one; it belongs to the same decision.
- **`timeline range + word` and `date words since + word`** are 92-230 ms
  and the relaxations 2.7-6 s: main's regression #1809 (a word with
  `after:`), the maintainer's to decide, not touched here.
- **The common word and `typed at`** are 55 ms at p95 under this load
  against step 1's 44: over the 50 ms budget by the load's share. Rerun
  quiet before reading anything into it (T138).

## The captures

```bash
POSTIO_FFI_FEATURES=demo scripts/macos-build.sh && scripts/macos-bundle.sh
POSTIO_DEMO=search POSTIO_WINDOW_SIZE=1440x900 POSTIO_APPEARANCE=light \
  POSTIO_DEMO_KEYS='/ at' POSTIO_DEMO_SNAPSHOT=/tmp/02.png \
  macos/build/Postio.app/Contents/MacOS/Postio
# 04: POSTIO_DEMO_KEYS='/ from:ad'
# 05: POSTIO_DEMO_KEYS='/ invoices ␣ from ␣ ada ␣ last ␣ month'
#     and, to see results, '/ invoice ␣ from ␣ priya ␣ last ␣ month'
```

The first captures found two defects that were fixed (below); the lists
are against the captures after them.

### What none of the three screens can match yet: the field is 327 wide

The design's field grows leftward to 860 and the panel hangs from it at
the same width. Here both stay at the toolbar's resting width: the
search item reports 327 points before and a second after
`preferredWidthForSearchField = 860` (the app's own log, a debug build
of `MainToolbar.grow`). Holding the width with a constraint on the field,
and setting the item's `minSize`/`maxSize`, changed nothing either, so
the `NSSearchToolbarItem` keeps its own width whatever it is told. Every
row is therefore cut: titles end in "…", the folder and count columns
keep their 100 and 110 and squeeze the words, the footer's hints lose
their words. **Not fixed**: it is step 2's (T054 marked it "fixed" from
the code; no capture had been taken then) and it wants the field to be a
toolbar item of Postio's own -- the chip field the plan already names --
rather than the system's search item. Owed before T137's sweep; noted
here so the sweep does not rediscover it.

### Screen 02, "at"

| # | Design | Built | Decision |
|---|---|---|---|
| 1 | Ghost "las" in tertiary after the caret | Drawn after the typed text in tertiary; a few points too far right, so it reads "at las" | **Not fixed**: the offset is the field editor's text origin, which the system search field does not publish; worth a pixel pass once the field is Postio's own (above) |
| 2 | "Tab completes" on the field's right, with the Esc cap | The system field's clear button | **Not fixed**, with the field (above) |
| 3 | Suggestions: "atlas as a word 62 Tab" focused | "atlas … 49 ⇥", focused and ringed, "at" in the find yellow | Same, cut by the width; 49 is the seed's count of conversations a search for `atlas` finds |
| 4 | "label:Atlas label 30" | "label:… 44", mono, the label dot in orange | Same, cut; the dot is the system orange, not the label's own colour |
| 5 | "Atlas planning · mailing list · atlas-planning@example.org 14" | Absent | **Explained**: the search seed has no mailing-list mail; the row's words are `list_detail`'s test and its count `completions`' |
| 6 | "Files named “at…” Atlas-Q3-budget.xlsx, Atlas-Sep-actuals.pdf and 16 more 18" | "Files n… 5" | Same row, the seed's five names, cut |
| 7 | Top hits so far: Ada Moreno · Re: Atlas Q3…, Tomás Reyes · Atlas staffing…, Northfield Elementary · Field-trip… | Three of "You · Re: …" | **Explained**: the top hits are the conversation search for the word `at` as typed (step 1's "typed at"), and in the seed that word is most often in your own replies. The design's hits match *atlas* by prefix; prefix matching in conversation search is not built (step 1 note: `atl*` finds what `atl` finds) |
| 8 | "Show all 214 results for “at”" ⌘↩ | "Show all 106 results f…" ⌘↩ | Same words, cut; the seed's count |
| 9 | Footer "Tab complete · ↑↓ move · ↩ open · ⌘↩ all results", "214 matches · 12 ms" | The same four keys and the count, words cut | Same, cut |

### Screen 04, "from:ad"

| # | Design | Built | Decision |
|---|---|---|---|
| 10 | The field's text in SF Mono 14 | SF Mono 14 | Same |
| 11 | "People matching “ad”", note "by how often you write to each other" | The same, the note cut | Same |
| 12 | Four people, 48 tall: initials avatar, bold name, mono "address · N messages · last X"; "Admin team (list)" without a last | Two: Ada Moreno and Ben Adeyemi (his surname begins "ad"), avatars in the popovers' colours, the lines cut | **Explained**: the seed's address book; `person_detail`'s test holds the line's words. Ranked two-way, but the seed records no sent mail, so it is one way here |
| 13 | "↩ adds" on the focused person's right | Absent | **Not fixed**: the focused row moves in Swift without a redraw from the controller; the words belong on whichever row the ring is on, which `DropdownView` would draw from the highlight. Small, and the footer says the same |
| 14 | "Latest from Ada Moreno", "preview of the focused person", two rows with times | The same title and note, two of Ada's, `in:INBOX` / `in:Receipts`, dates | Same; dated rows ("9 Oct") where the design has times for today's: `hit_date` is the dropdown's one date rule |
| 15 | Footer "↩ add as chip · ⌥↩ exclude (-from:) · ↑↓ choose · ⌫ back to words", right "Contacts and everyone you have mail with" | The same keys and words, cut | Same |

### Screen 05, "invoices from ada last month"

| # | Design | Built | Decision |
|---|---|---|---|
| 16 | "Understood as" band in the window colour, tiles: `invoice*` from "invoices", `from:Ada Moreno` from "from ada", `after:2026-08-01` and `before:2026-09-01` from "last month"; "each part is a chip you can edit" | The band and tiles: `invoices` from ‘invoices’, `from:ada` from ‘from ada’, then the two dates from ‘last month’ off the right edge (the tiles scroll); the note gives way | **Explained**: the lowering keeps the plural and writes no `*` (step 2 note, #16); `from:ada` because the seed's address book does not resolve "ada" to an address, so there is no name to show; the dates are the month before the demo's today (September), and the band is narrower than four tiles |
| 17 | The origin in curly double quotes: from “invoices” | Curly single quotes: from ‘invoices’ | **Explained**: SPEC §1-2 writes from ‘last month’; the picture's double quotes disagree with its own spec, and the spec is kept |
| 18 | Results: three of Ada's invoices, "August 2026 · newest first", the first ringed; "Show all 3 results" | No results: the seed's Ada sent no invoice in September, so the section and Show all are absent | **Explained**: the data. `invoice from priya last month` (`05c.png`) shows the state with a hit: "Results", "September 2026 · newest first", Priya's invoice ringed with `in:INBOX` and "19 Sep", "Show 1 result" ⌘↩ |
| 19 | Matched words in the find yellow ("Invoice") | First capture lit the sender's name too | **Fixed**: plain English marks the free words only (`fix(focus)`), as screen 05 does |
| 20 | Footer "↑↓ move · ↩ open · Tab edit as chips · ⌘⌫ keep as words", "parsed on this Mac · 3 matches · 21 ms" | The same keys, cut; "parsed on this Mac · 0 matches · N ms", cut | Same, the seed's count |
| 21 | — | First capture: four tiles at their own width pushed the panel's layout out, the results and footer under the band | **Fixed**: the tiles scroll inside the band (`fix(macos)`) |

## Left as it is

- **⌘⌫ then ⌘↩** opens the results on the sentence lowered again: the
  words kept as words are the dropdown's, and `show_all_results` lowers
  what is typed. Keeping them would need the results to carry "literal",
  which the one-query rule (D1) has no place for.
- **The operator states never ask a conversation search** while the value
  is typed: `from:ad` lists people, not mail from "ad". Return or ⌥↩
  makes the chip and the panel searches it.
- **Text the controller writes is settled** (`Bar::settled`): a saved
  search `from:juno` or Tab's "Narrow to" ending in an operator is a chip
  made, not an operator being typed.
