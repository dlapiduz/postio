# Focus search, step 3: the results view against screens 06 and 07

2026-10-09, specs/010-focus-search T067-T075. ⌘↩ in the dropdown turns
the Mac's main window into the results view: ‹ Inbox, the query as chips
and words and Save search in the toolbar; the filter bar, the timeline,
the grouped rows and the footer below. Every word, row and group is the
controller's (`postio-focus`, `FocusQuery` / `FocusResults` /
`FocusResultsPage` / `FocusResultsCursor`); the Mac draws them. This note
lists every difference from screens 06 and 07 and what was done about
each.

## The captures

`scripts/macos-shot.sh 06-results --seed search --size 1440x900 --light`
with `POSTIO_DEMO_KEYS='/ atlas ␣ budget ⌘⏎'` built and launched the demo
and then failed as step 2's did: `screencapture -l` answered "could not
create image from window" (no Screen Recording grant for this terminal).

So a demo can now draw its own window (`POSTIO_DEMO_SNAPSHOT=<path>`,
demo builds only): once the demo's keys are pressed it renders the main
window's frame view -- toolbar and content -- to a PNG. Screens 06 and 07
were compared from those:

```bash
POSTIO_FFI_FEATURES=demo scripts/macos-build.sh && scripts/macos-bundle.sh
POSTIO_DEMO=search POSTIO_WINDOW_SIZE=1440x900 POSTIO_APPEARANCE=light \
  POSTIO_DEMO_KEYS='/ atlas ␣ budget ⌘⏎' POSTIO_DEMO_SNAPSHOT=/tmp/06.png \
  macos/build/Postio.app/Contents/MacOS/Postio      # quit it after ~15 s
```

The picture is the views drawn again, not the screen's pixels: the
traffic lights draw inactive and the toolbar's material is flat, so a
capture with the grant should still be taken before the review, and this
list amended if it shows more. Leaving was checked the same way
(`'j x / atlas ␣ budget ⌘⏎ j ⎋'`): the inbox came back with the cursor on
the second row and that row still selected, its toolbar back.

## Toolbar (§3.1)

| # | Design | Built | Decision |
|---|---|---|---|
| 1 | Traffic lights, ‹ Inbox with an `Esc` keycap, the field filling the width, Save search ⌘S, 52 tall | The toolbar swaps its items while the results are up: ‹ Inbox (chevron, 13 semibold), the query box sized to the window less the other items, Save search with ⌘S | Same layout. The box's width is computed (window − 86 − the two buttons − 44), not laid out by the toolbar; recomputed on resize |
| 2 | The Esc keycap reads "Esc" | "⎋", as every Mac keycap spells Escape (`KeyCapSpelling`, 009) | **Explained**: one spelling for the key app-wide; the footer's "back to inbox" cap is the same glyph |
| 3 | The field: magnifier, chips (SF Mono 12.5, 26 tall, radius 6), words 14.5, "/ to edit" on the right; 34 tall, radius 8, hairline ring | `ResultsQueryBox`: those, chips as text attachments in `ChipQueryField` | Same. With no operator in the query there are no chips, as on screen 06 |
| 4 | `/` edits the query in the same dropdown, anchored to this field | `/`, a click or a typed key opens the bar on the query text; the bar's own field is laid over the chips while it is up and the panel hangs from it | **Explained** (tasks.md T070): the bar edits text the controller hands it, so Swift never spells query text |
| 5 | Save search opens the save popover | Drawn, inert | **Not fixed** until step 6 (T070 says so) |

## Filter bar (§3.2)

| # | Design | Built | Decision |
|---|---|---|---|
| 6 | Tabs as a segmented control with counts; ⌘1-3 | As designed, the selected tab bold on a raised fill; counts tertiary | Same. The counts are the seed's: 26 / 6 / 9 against the design's 48 / 12 / 6 |
| 7 | Eight 28-tall pill buttons, ▾ on the five with popovers; applied ones solid with their value | As designed (`FilterBarView`); the solid state follows `FilterButtonFfi.applied` | Same. The five popovers are step 4: until then a click on one sends nothing (T067) |
| 8 | Sort: "Best match ▾" on the right after a tertiary "Sort" | A borderless menu with Best match and Newest | Same |

## Timeline (§3.3)

| # | Design | Built | Decision |
|---|---|---|---|
| 9 | "48 conversations" bold 13 over "12 files · 6 people · last 12 months" | The controller's lines | Same; the numbers are the seed's |
| 10 | Twelve bars, Oct to Sep, 36 tall at most, grey | Twelve bars, Nov to Oct: the demo's today is the real date (9 October), so the window ends in October | **Explained**: the design is dated September |
| 11 | Bar colour | Tertiary fill; quaternary was paler than the design, fixed after the first capture | **Fixed** |
| 12 | Dragging across months narrows | Display only | **Not fixed** until step 4 (T072 says display only) |

## Rows (§3.4)

| # | Design | Built | Decision |
|---|---|---|---|
| 13 | Group header: "Top hits  why each one ranked is under the sender" (no count), "September 2026  9 · newest first" | "Top hits  3 · why each one ranked…": the controller gives Top hits a count, and the view draws count then note for every group | **Not fixed**, the controller's: `ResultGroupFfi.count` for Top hits could be empty (a one-line change in `postio-focus` with its test). Left for the Rust side rather than hidden in Swift |
| 14 | Every row has a passage: a window of about 120 characters around the first match, never the email's first line | Rows matched in "subject + body" have no passage at all; some body matches show the quoted header ("On Fri 9 Oct 2026 at 10:59, You wrote:") rather than a window around the match | **Not fixed: an engine defect, not the view's.** The view draws `ResultRowFfi.passage` as it arrives and redraws on `FocusResultsPage`; the passages `postio_session::search::passages` returns over the search seed are empty or not the match's. Needs its own task before the review (owner: the Rust side) |
| 15 | Top hits' reason: at most two reasons plus matches ("you replied · 3 matches") | "you replied · frequent sender · …", truncated in the 176 column | **Not fixed**, the controller's words: the seed's top hits have more than two reasons, and `reason_line` keeps at most two plus matches, so the third is the match count; it does not fit 176 at 11.5 pt. Check against `reason_line`'s tests when 14 is fixed |
| 16 | Folder "in:Inbox" | "in:INBOX" for the demo's inbox | **Explained**: the folder's own name in the seed, which `in_folder` does not recase |
| 17 | Date column 62, "26 Sep" | Dates past a year carry the year ("22 Oct 2025") and were cut to "22 Oct 2…" | **Fixed**: the date takes the room it needs |
| 18 | Label pills with the label's own colour (orange dot for Atlas) | A plain tertiary dot | **Not fixed**, the boundary's: `ResultRowFfi.pills` crosses with `color: None` (`focus_search.rs`); the list's pills carry the colour. Small Rust follow-up |
| 19 | Focus ring: 2 pt accent ring around the row, faint accent fill | As designed, drawn by `ResultRowBackground` | Same |
| 20 | Unread: the dot in the gutter, bold sender and date | As designed | Same |
| 21 | Source tag: a filled 17-tall pill, italic file names | As designed; "subject + body", "body + subject" are the controller's joined tags (T065's `sources_tag`) | Same |

## Footer (§3.5)

| # | Design | Built | Decision |
|---|---|---|---|
| 22 | "j k move · Space Quick Look · ↩ open · x select · / edit query · Esc back to inbox" | "j/k move · ↩ open · x select · / edit query · ⎋ back to inbox" | Quick Look joins with step 5. "j/k" was first drawn "J/K": a pair's cap went through the chord path and was capitalised -- **fixed** (`KeyCapSpelling`) |
| 23 | "48 conversations · local index · 41 ms" | The controller's, with the real time ("8 ms") | Same |

## Dark (screen 07)

Every colour is semantic (label, secondary, tertiary, separator, the
text and window backgrounds, the accent) and the find yellow is the
system yellow at 28% in dark, the design's `rgba(255,214,10,0.28)`. The
dark capture matches screen 07 in the same ways and differs in the same
ways as the light one; nothing is dark-only.

## What step 3 left GTK

`crates/postio-gtk` names none of what changed: it builds its policy with
`Policy::for_platform` (so `results_view` is off), constructs
`KeyContext` values but matches none exhaustively, and uses none of the
new `postio-ui` words or FFI exports.
