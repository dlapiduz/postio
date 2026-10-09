# Focus search, step 4: filter popovers and the timeline against screens 08 and 09

2026-10-09, specs/010-focus-search T076-T085. The filter bar's From, To,
Date, Anywhere and Label buttons open popovers that preview live: each
check edits the one query (D1) and the list, counts and timeline follow
while the popover is open; Esc, a click away or Back's first rung put
back exactly the query it opened on; Return keeps the preview. The
timeline narrows by a drag across its bars (reported once, on release)
and ⌥←/⌥→ step the range a month. The Date popover takes plain words
("since july" → `after:2026-07-01`) and offers presets with counts.
Everything decided is the controller's (`postio-focus/src/results.rs`);
the Mac reports a row's token, whether ⌥ was held, the fields' text, a
preset's token and the bars a drag covered.

## The captures

`screencapture` still has no grant here, so both screens come from the
demo's own snapshot, which now also draws the windows over the main one
(the popover is a window of its own) and accepts `@from`, `@date`, … in
`POSTIO_DEMO_KEYS` as a click on that filter button:

```bash
POSTIO_FFI_FEATURES=demo scripts/macos-build.sh && scripts/macos-bundle.sh
POSTIO_DEMO=search POSTIO_WINDOW_SIZE=1440x900 POSTIO_APPEARANCE=light \
  POSTIO_DEMO_KEYS='/ atlas ␣ budget ⌘⏎ @from ␣' POSTIO_DEMO_SNAPSHOT=/tmp/08.png \
  macos/build/Postio.app/Contents/MacOS/Postio
POSTIO_DEMO_KEYS='/ atlas ␣ budget ⌘⏎ ⌥o @from ␣ ⏎ @date since ␣ july' \
  POSTIO_DEMO_SNAPSHOT=/tmp/09.png …              # same otherwise
```

The snapshot is the views drawn again, not the screen: a popover's
material, arrow and shadow are not drawn (it shows as a flat grey
rectangle), so a capture with the grant should still be taken before the
review.

## Screen 08: From

| # | Design | Built | Decision |
|---|---|---|---|
| 1 | NSPopover with an arrow, under From, its left edge near the button and the arrow at the button's centre | An `NSPopover` (transient) hung from the button, centred on it | **Explained**: AppKit centres a popover on its anchor; the design's off-centre placement would need a positioning rect trick that breaks at the window's edges. The arrow is not in the snapshot (material) |
| 2 | 360 wide; field "Filter people in these results"; rows 44 tall: checkbox, 28 pt initials, name over address in SF Mono 11, a bar, the count | As designed (`ListPopoverView`) | Same |
| 3 | Coloured avatars | A system hue chosen by the name | Same intent; the hue is drawing only, so a person keeps theirs; the design's exact colours are its data |
| 4 | Six people, Ada 21 … Grace 3 | The seed's senders: Ada 13, You 13, Priya 6, Ravi 6, … nine rows | **Explained**: the seed's data. "You" is listed because you sent matching mail; the design shows no row for the user, and whether it should is open (below) |
| 5 | The highlighted row: accent ring, faint fill; Ada checked | As designed; Space checked Ada | Same |
| 6 | Footer "Space toggle · ⌥ -click excludes · ↩ apply" | The same, from `postio_ui::search_view::popover_hints` | Same |
| 7 | While Ada is checked, the field has no chip, From is outlined and ringed, and the list behind still shows Tomás, Priya … | The preview is live (FR-027): a `from:` chip in the field, From solid ("From: Ada Moreno") and ringed, the list and counts are Ada's, "previewing Ada Moreno · ↩ applies" under the count | **Explained**: the query is the one source of truth (D1), so the field and the button show what the list is showing. The PNG is a still of the moment before the preview landed |
| 8 | The count line's second line | First capture: "previewing From: Ada M…", cut at 210 | **Fixed**: the line names the value alone |
| 9 | Bars proportional to the count | A 3 pt capsule, at least 6 wide | Same |

## Screen 09: Date

| # | Design | Built | Decision |
|---|---|---|---|
| 10 | 470 wide, two columns; left 170 on the window background: Any time 21, Last 7 days 2, Last 30 days 6, This quarter 12 (ringed), This year 17, Custom… | As designed; counts are the seed's and are those of the query the popover opened on | Same layout. Nothing is ringed in the capture: the demo's today is 9 October, so This quarter starts 1 October, not on `after:2026-07-01` |
| 11 | "since july" with "→ after:2026-07-01" in the field | The same, from `natural` through the controller | Same |
| 12 | "Type a date in plain words, or drag across the months." | The same (`DATE_WORDS_HINT`) | Same |
| 13 | A 90-tall month chart, Oct to Sep, selected months dark with bold labels | 90 tall, Nov to Oct (today is October), the selected bars dark over a soft band, labels bold | First capture cut the labels to "D…", "M…": **fixed** (tightened, down to 0.75 scale) |
| 14 | "12 of 21  Jul – Sep 2026", "↩ apply" | "6 of 13  Jul – Oct 2026", "↩ apply" | Same; the open end runs to this month |
| 15 | Count line "12 conversations / previewing Jul – Sep · ↩ applies" | The same words with the seed's numbers | Same |
| 16 | Timeline hint "Jul – Sep selected · drag to change · ⌥← → steps a month" | "Jul – Oct selected · drag to change · ⌥←/⌥→ steps a month" | Same; the pair is spelled as every pair is |
| 17 | The Date button outlined and ringed while open, From solid | Date solid ("Since July") and ringed: the preview is the query | **Explained**, as 7 |
| 18 | The timeline's bars outside the range are faded but there | The same: the bars outside a range are the months of the query without its dates, remembered from its last answer | Same. A query that arrives already dated (typed, or from a saved search) draws only its own months until the undated one has been asked; asking it is a second read and is left for when it is needed |
| 19 | Sort "Newest" | ⌥o in the capture | Same |

## Open questions

- **Two people checked.** The popover's checks are `from:` clauses, and
  the query language ANDs two of them: checking Ada and Tomás finds what
  both sent, which is nothing. The design draws checkboxes, which read as
  "either". OR within one field would be a change to the one query
  language (constitution III) and belongs to the maintainer.
- **"You" among the senders.** It is in the facets because the user sent
  matching mail. Leaving it out is a rule for the facets, not the popover.

## The bench with the preview pattern (T084)

`crates/postio-bench/benches/search_focus.rs`, release, Apple M1 Pro,
20,000 messages, 60 runs after 5 warm-ups, today 30 September 2026; the
GTK dev-dependencies taken out of `postio-bench` for the run and put back,
as in step 1. The bench was stale since step 3 (`passages` takes a
`FirstLine`; it now passes `Avoided`, as the results do).

The preview pattern is the same request asked again per check, with one
term added or excluded, then once more on Esc: its cost per toggle is one
executor shape below.

| shape | query | p50 ms | p95 ms | stmts | found | step 1 p95 |
|---|---|---:|---:|---:|---:|---:|
| one word | `quarterly` | 5.67 | 5.77 | 3 | 203 | 2.63 |
| two words | `quarterly forecast` | 3.82 | 3.86 | 3 | 7 | 0.68 |
| operator only | `from:sender3` | 7.40 | 7.54 | 4 | 479 | 7.88 |
| operator + words | `from:sender3 regarding` | 16.85 | 17.10 | 4 | 460 | 12.47 |
| common word | `regarding` | 49.65 | **50.86** | 3 | 1,679 | 44.71 |
| typed `a` | `a` | 39.77 | 40.22 | 3 | 1,560 | 40.17 |
| typed `at` | `at` | 49.66 | **50.21** | 3 | 1,555 | 42.38 |
| typed `atl` | `atl` | 6.77 | 6.86 | 3 | 366 | 3.39 |
| zero hits, four filters | `quarterly from:sender30 label:atlas has:attachment after:2025-01-01` | 0.83 | 0.84 | 2 | 0 | 0.73 |
| relaxations of that | five variants | 2,443 | **2,509** | 5 | 1 variant | 2.74 |
| e2e one word | `quarterly` | 6.92 | 6.97 | 56 | 203 | 3.27 |
| e2e two words | `quarterly forecast` | 4.09 | 4.20 | 13 | 7 | 0.85 |
| e2e operator + words | `from:sender3 regarding` | 18.11 | 18.72 | 57 | 460 | 11.93 |
| e2e common word | `regarding` | 51.02 | **52.23** | 56 | 1,679 | 43.47 |
| e2e typed `atl` | `atl` | 8.10 | 8.25 | 56 | 366 | 4.72 |
| preview: check a person | `quarterly from:sender3` | 1.92 | 1.94 | 4 | 9 | new |
| preview: exclude a person | `quarterly -from:sender3` | 6.90 | 6.95 | 4 | 194 | new |
| preview: check a label | `quarterly label:atlas` | 1.16 | 1.17 | 4 | 21 | new |
| preview: check a folder | `quarterly in:inbox` | 5.72 | 5.84 | 3 | 203 | new |
| timeline range + word | `quarterly after:2026-07-01 before:2026-10-01` | 97.08 | **97.69** | 3 | 24 | new |
| Date words "since" + word | `quarterly after:2026-07-01` | 85.22 | **88.50** | 3 | 24 | new |
| timeline range alone | `after:2026-07-01 before:2026-10-01` | 10.07 | 10.39 | 3 | 352 | new |
| timeline range + operator | `from:sender3 after:2026-07-01 before:2026-10-01` | 2.87 | 2.91 | 4 | 66 | new |
| e2e preview: check a person | `quarterly from:sender3` | 2.23 | 2.25 | 16 | 9 | new |
| e2e timeline range + word | `quarterly after:2026-07-01 before:2026-10-01` | 97.69 | **98.40** | 30 | 24 | new |

**Over the 50 ms budget**, and why:

- **Every word with `after:`** -- the timeline range, the Date words and
  the relaxations (four of five variants carry `after:2025-01-01`) -- is
  #1809: since #1805 (content identity) a free-text query with a date
  filter is checked per row across the content join. The cost scales
  with the range: a quarter costs ~90 ms here, `after:2025-01-01` ~600 ms
  a variant. Without words (`timeline range alone`, 10 ms) or with an
  operator driving it (`+ from:`, 3 ms) the same dates are fast. The
  maintainer is deciding #1809; per the stop rule nothing here works
  round it.
- **The common word and `at`** (50.2-52.2 ms, 44.7 and 42.4 in step 1),
  and the 2-4 ms every other free-text shape gained, are the "few ms
  slower" #1809 reports for every free-text shape after the same rebase.
  Nothing step 4 added runs on these paths: a popover's check is one
  more request of an existing shape, and the timeline's bars are read
  from the answer already in hand.

The preview pattern itself is within budget whenever its shape is: a
check on a person, label or folder narrows the request and costs 1-7 ms;
only a date with words is over, and that is #1809.

## What step 4 left GTK

`crates/postio-gtk` names none of what changed. The two new commands
(`step_range_back`, `step_range_forward`) are the results' context alone
and not offered on Freedesktop (D25): the Linux golden table gains two
unbound rows and `docs/keybindings.md` is unchanged. `Intent::Popover`
and the new `Input`s are matched nowhere in GTK (it never enters the
results, `results_view` off).
