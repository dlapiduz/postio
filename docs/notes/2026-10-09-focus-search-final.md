# Focus search, final: the bench, and what the common word costs

2026-10-09, specs/010-focus-search. The last measurement of the engine
before the final sweep (T137, T138 extend this note). Three shapes are still
over the 50 ms budget, all of them a word in nearly every message, and the
time is in one statement: the walk that feeds the facets.

## Bench

`crates/postio-bench/benches/search_focus.rs`, release build, Apple M1 Pro
(32 GB), in-memory store, 20,000 messages (step 1's corpus, step 9's
attachment units), 60 timed runs per shape after 5 warm-ups. Method as in
step 1's note: the bench's GTK dev-dependencies taken out of
`crates/postio-bench/Cargo.toml` for the build and put back, nothing about
that committed.

Built on `192d8a12`, which carries two changes since step 9's table:

- **A search no longer walks `attachments`** to say whether every
  downloaded attachment has been read (`bb2650fd`). The question is
  answered from `attachment_text_owed`, a set the index keeps by trigger,
  in one seek. `index_suite::driven_join_plan` now plans *every* statement
  `search_conversations` issues, not only those joined to the hits -- the
  gap this walk got through. The bench's attachments have no blobs, so
  the walk it removed was 6,000 rows of a `blob_id IS NOT NULL` filter:
  under a millisecond here, and growing with every attachment on disk in a
  real store. The statement now costs 0.08 ms.
- **A PDF skipped for want of the extraction helper is read once the
  helper is there** (`192d8a12`), rather than at the next extractor
  version. No effect on this bench.

`uptime` load average 4.1-4.8 while the shapes ran (other sessions
building). A second run at load 7-14 put every heavy shape 20-70% higher
(`common word` p95 95.7 ms); the table is the quieter run.

| shape | query | p50 ms | p95 ms | stmts | found |
|---|---|---:|---:|---:|---:|
| one word | `quarterly` | 3.36 | 3.92 | 3 | 203 |
| two words | `quarterly forecast` | 0.81 | 1.01 | 3 | 7 |
| operator only | `from:sender3` | 8.70 | 9.65 | 4 | 479 |
| operator + words | `from:sender3 regarding` | 19.83 | 20.68 | 4 | 460 |
| **common word** | `regarding` | 53.95 | **55.61** | 3 | 1,678 |
| typed `a` | `a` | 43.53 | 44.74 | 3 | 1,560 |
| **typed `at`** | `at` | 53.73 | **55.53** | 3 | 1,557 |
| typed `atl` | `atl` | 4.43 | 5.55 | 3 | 366 |
| completions | `a` | 4.59 | 5.37 | 4 | 2 |
| completions | `at` | 2.37 | 3.16 | 4 | 2 |
| completions | `atl` | 4.33 | 4.94 | 4 | 2 |
| completions | `from:a` | 1.57 | 1.77 | 1 | 4 |
| zero hits, four filters | (step 1's) | 1.02 | 1.38 | 2 | 0 |
| relaxations of that | five variants | 3.78 | 4.52 | 5 | 1 variant |
| e2e one word | `quarterly` | 4.74 | 5.57 | 56 | 203 |
| e2e two words | `quarterly forecast` | 1.23 | 1.56 | 13 | 7 |
| e2e operator + words | `from:sender3 regarding` | 20.88 | 21.88 | 57 | 460 |
| **e2e common word** | `regarding` | 55.24 | **57.01** | 56 | 1,678 |
| e2e typed `atl` | `atl` | 5.99 | 6.95 | 56 | 366 |
| preview check a person | `quarterly from:sender3` | 2.17 | 2.79 | 4 | 9 |
| preview exclude a person | `quarterly -from:sender3` | 4.66 | 6.02 | 4 | 194 |
| preview check a label | `quarterly label:atlas` | 1.30 | 1.49 | 4 | 21 |
| preview check a folder | `quarterly in:inbox` | 2.95 | 3.72 | 3 | 203 |
| timeline range + word | `quarterly after:… before:…` | 1.09 | 1.33 | 3 | 24 |
| date words since + word | `quarterly after:…` | 1.06 | 1.49 | 3 | 24 |
| timeline range alone | `after:… before:…` | 10.97 | 11.70 | 3 | 352 |
| timeline range + operator | `from:sender3 after:… before:…` | 3.14 | 3.93 | 4 | 66 |
| file word | `kestrel` | 1.04 | 1.30 | 4 | 20 |
| e2e file word | `kestrel` | 1.46 | 1.67 | 8 | 20 |
| Files tab, file word | `kestrel` | 0.49 | 0.63 | 3 | 20 |
| Files tab, one word | `quarterly` | 1.54 | 1.84 | 2 | 0 |
| Files tab, operator | `from:sender3` | 8.03 | 8.96 | 3 | 350 |
| e2e preview check a person | `quarterly from:sender3` | 2.51 | 3.08 | 16 | 9 |
| e2e timeline range + word | `quarterly after:… before:…` | 1.78 | 2.19 | 30 | 24 |

Completions are all under their 20 ms budget. Over the 50 ms budget:
`common word` 55.6, `typed at` 55.5, `e2e common word` 57.0 -- the same
three as step 9's table (52.3, 53.4, 55.2 there, at a lower load), within
11-14% of the budget.

## Where the common word's time goes

Measured with timers around each phase of `search_conversations` and the
walk statement re-run with one projection column at a time removed, on
the bench's corpus, the instrumentation not committed. The counting hooks
count statements and rows; they do not time them. Figures are medians,
load 4-9, which is why the total is a little above the table's p50.

| phase | ms |
|---|---:|
| **the walk** (`projection_sql` over `HITS_JOIN_WITH_FILES`, 10,001 rows until the fold holds `TOTAL_HITS_CAP`) | **52.8** |
| of which: the fold's own Rust per row (`Fold::take`) | 3.4 |
| grouping into conversations | 1.2 |
| ordering and ranking the pool | 0.2 |
| hydrating the pool and the page (250 rows) | 1.7 |
| facets over 1,674 conversations, files, people | 0.2 |
| searched / corpus complete / contents complete | 0.1 |
| **total** | **56.2** |

The facets *fold* is not the cost. What the facets need *from the walk* is.
The walk statement alone, 10,001 rows, with each per-row subquery removed
in turn:

| walk statement | ms | the column's share |
|---|---:|---:|
| as it is | 57.5 | |
| the hits and the join to `messages`, `m.id` only | 13.6 | |
| plus the message's own columns | 19.2 | 5.6 |
| plus people (`group_concat` over `recipients`) | | **19.3** |
| plus labels (`group_concat` over `message_labels`) | | 9.4 |
| plus the attachment count (`count(*)` over `attachments`) | | 7.2 |
| plus "has a live marker" (`markers`) | | 3.6 |

So two thirds of the walk is four correlated subqueries per matched
message, and people and labels are the two that exist only for the
facets. The plan is already a seek for each (`idx_recipients_message`,
the label key, `idx_attachments_message`, the marker's key); this is the
cost of 10,000 seeks of each, not of a bad plan.

## Options, not taken

None of these is implemented: the first two are schema work on the write
path, the last two change what a person sees, and all four want the
maintainer's choice.

1. **Maintained people and label lists per message.** Two columns on
   `messages` (the packed address ids the people facet reads, the label
   ids), kept by triggers on `recipients` and `message_labels` -- deferred
   during a sync batch the way `search_documents` is (`defer_documents`).
   The walk reads two columns instead of two subqueries: about -27 ms,
   common word ~30 ms. Nothing visible changes. Costs a storage migration
   and a trigger write per recipient and label row on the sync path, which
   wants `insert_cost_curve` measured before it lands (ADR 0040).
2. **The attachment count as a column.** `messages.has_attachments` is
   already maintained (the content projection's trigger sets it from
   `EXISTS (... attachments)`); a count beside it removes the third
   subquery: about -7 ms. Cheapest of the four; with 1, common word ~23 ms.
3. **Lower the walk's cap and show broad counts as floors.** The walk is
   linear in rows: a 5,000-message cap is roughly -26 ms. The total, the
   facets and the months of a word in most of the mailbox become "5,000+"
   sooner than they do now. Visible, so the maintainer's.
4. **Facets after the page.** Answer the page from the cheap walk (the
   19 ms row above, plus hydration) and send people and labels in a second
   reply. First paint ~22 ms; the facet column fills in ~35 ms later.
   Visible, and a second reply in the protocol.

Measured as a cheaper fifth: a covering index on
`recipients (message_id, kind, address_id)` makes the people subquery
index-only and took the walk from 56.9 to 52.5 ms -- not enough on its
own, and it adds an index entry to every recipient row.

## Screens

T137, the last look: all thirteen screens at 1440 by 900 in light, and
the results in dark (screen 07), against `Design/focus-macos-search/screens/`.
This list is the single place for what still differs: the open rows of
steps 2 to 10's notes are carried into it (below the screens), so nothing
needs reading there to know what is left. Every row is **fixed**,
**explained** (a deliberate difference, or the demo's data), or **not
fixed** with what it would take.

### How they were taken

**Real captures this time.** The terminal now has the Screen Recording
grant, so `scripts/macos-shot.sh` works, and the sweep photographed the
screen rather than the demo's own snapshot: a region capture of the
window's bounds (`screencapture -R`), which takes the popovers, the
dropdown panel, the Quick Look panel, materials, the field's halo and
the window server's shadows with it (`screencapture -l` takes one
window, and a popover is a window of its own). Each was taken over the
search seed after the step notes' key sequences:

```bash
POSTIO_FFI_FEATURES=demo scripts/macos-build.sh && scripts/macos-bundle.sh
POSTIO_DEMO=search POSTIO_WINDOW_SIZE=1440x900 POSTIO_APPEARANCE=light \
  POSTIO_DEMO_KEYS='<keys>' macos/build/Postio.app/Contents/MacOS/Postio &
# ~12 s later: screencapture -o -x -R<x,y,w,h of the window> <file>
```

| screen | keys |
|---|---|
| 01 | `/` |
| 02 | `/ at` |
| 03 | `/ atlas ␣ budget` |
| 04 | `/ from:ad` |
| 05 | `/ invoices ␣ from ␣ ada ␣ last ␣ month` |
| 06, 07 (dark) | `/ atlas ␣ budget ⌘⏎` |
| 08 | `/ atlas ␣ budget ⌘⏎ @from ␣` |
| 09 | `/ atlas ␣ budget ⌘⏎ ⌥o @from ␣ ⏎ @date since ␣ july` |
| 10 | `/ atlas ␣ budget ⌘⏎ @from ␣ ⏎ @date since ␣ july ⏎ ⌥o ␣` |
| 11 | `/ atlas ␣ budget ⌘⏎ @from ␣ ⏎ @date since ␣ july ⏎ ⌘2` |
| 12 | `… ⏎ ⌥o x j x j j x j x j j x ⌘s` (as 10, then the checks) |
| 13 | `/ from:ada@example.com ␣ has:attachment ␣ before:2026-08-01 ␣ subject:budget ⌘⏎` |

The real captures settle every "unseen in the snapshot" row of the
earlier notes: the popovers draw their material and arrow, the Save
popover's name field is ringed and its Pin switch is on in the accent,
the field's halo is there, and the panels have the system's shadow.

### What is the same everywhere, and why

Rows that hold on every screen, said once:

| # | Design | Built | Decision |
|---|---|---|---|
| A1 | Today is Saturday 26 September; counts like 48 results, 18,204 messages | The demo's today is the real date (Friday 9 October), and its counts are the search seed's (27 for `atlas budget`, 459 messages) | **Explained**: the seed's data. Month windows end in October, "since July" runs to October, This quarter starts 1 October |
| A2 | The Escape cap reads "Esc", Tab "Tab" | "⎋", "⇥" | **Explained**: one spelling per key app-wide (`KeyCapSpelling`, spec 009), as every Mac keycap spells them |
| A3 | Avatars in the design's colours | A system hue chosen by the name | **Explained**: drawing only; a person keeps theirs (step 4 #3) |
| A4 | Checkboxes, the Pin switch and rings in graphite | The system accent | **Explained**: the system accent, as spec 009's list does (step 6 #9) |
| A5 | Window grey `#f5f5f7` under cards and the date popover's presets | `windowBackgroundColor`, a shade darker (239 against 245) | **Explained**: the system's semantic colour; a literal is refused (`SemanticColourTests`) |
| A6 | The toolbar in dark is `#3a3a3c` | The system's titlebar material, a shade lighter | **Explained**: the platform's material |
| A7 | No scroller | A thin overlay scroller beside a list just drawn | **Explained**: the system's, shown while a list settles (step 9 #17) |

### Screens 01 to 05, the dropdown

| # | Screen | Design | Built | Decision |
|---|---|---|---|---|
| 1 | 01-05 | Rows 10 in from their ground, 12 between columns: the icon at 596, the words at 620 | 6 in and no gap: the words at 604, touching the icon; a hit's passage ran into its folder ("navigation a…in:INBOX", 02) | **Fixed** (`fix(macos): lay the dropdown's rows out on its grid`) |
| 2 | 01 | Recent: "yesterday", "Mon", "21 Sep" | "yesterday", "Sun", "30 Sep" | **Explained**: A1, and step 2 #12 (the design's two dates cannot both hold under T045's rule) |
| 3 | 01 | Saved searches 5 / 38 / 19 / 4 | 0 / 46 / 26 / 0 | **Explained**: the seed's real counts (step 2 #14) |
| 4 | 01 | "invoices from ada last month becomes from:ada invoice after:2026-08-01 before:2026-09-01" | "… becomes invoices from:ada after:2026-09-01 before:2026-10-01" | **Explained**: the one language's lowering keeps the sentence's order and plural (FR-004: it must equal `lower`), and the month is the one before today (step 2 #16) |
| 5 | 01 | Footer right "local index · 18,204 messages" | Nothing on the right | **Not fixed**: needs the store's message count in the empty state and a `postio-ui` word (step 2 #13). Small follow-up |
| 6 | 02 | Ghost "at\|las" right after the caret | "at las": a gap of about five points | **Fixed** (`fix(macos): set the ghost right after the typed word`): placed from the field editor's last glyph, less the label's inset |
| 7 | 02 | "Tab completes" before the Esc cap in the field | The cap alone | **Not fixed**: a `postio-ui` word and an FFI export (step 2 #3, step 8 #2). Small follow-up |
| 8 | 02 | "Atlas planning · mailing list" row | Absent | **Explained**: the seed has no list mail (step 8 #5) |
| 9 | 02 | Top hits match *atlas* by prefix | Three of your replies that say "at" | **Explained**: the conversation search for the word as typed; prefix matching there is not built (step 8 #7) |
| 10 | 03 | Every top hit has a passage after its subject | Only the hit that matched in a file had one: a one-line body's match was kept out as "the row's preview" | **Fixed** (`fix(focus): mark the first line's match in the dropdown`): the dropdown's row shows no preview, so it asks `FirstLine::Avoided` as the results do. D7's "the row already shows the first line" holds for neither Mac surface |
| 11 | 03 | Narrow to: "from: Ada Moreno 21", "from: Tomás Reyes 9", attachment, label | "from: You", then Ada | **Fixed** (`fix(focus): offer no pill or From row for yourself`): the facets' names say which people are yours, by the People tab's notion of you, and Narrow to skips them |
| 12 | 03, 06 | "in:Inbox" | "in:INBOX" | **Fixed** (`fix(ui): call the inbox Inbox where search names it`), carrying step 2 #25 and step 3 #16: IMAP's INBOX is the inbox in any case |
| 13 | 03 | Dates "26 Sep" | One top hit "22 Oct 25" | **Explained**: another year's short form (step 3 #17) |
| 14 | 04 | "from:" tertiary and "ad" primary in the field | "from:ad" all primary, in SF Mono | **Not fixed**: the field colours no part of what is typed; an operator's prefix in tertiary needs the field to style its own text as it is edited. Small follow-up |
| 15 | 04 | "↩ adds" on the focused person's right | Absent | **Not fixed** (step 8 #13): a `postio-ui` word drawn on the highlighted row. Small follow-up; the footer says the same |
| 16 | 04 | Four people; "15:51" | Two; "9 Oct" | **Explained**: the seed's address book, and `hit_date`'s one rule (step 8 #12, #14) |
| 17 | 05 | "each part is a chip you can edit" after the tiles | Absent: the tiles' scroll view took every point even with room to spare | **Fixed** (with 1): the note shows when the tiles fit and gives way when they do not |
| 18 | 05 | `invoice*`, `from:Ada Moreno`, the August dates; three results | `invoices`, `from:ada`, September; no results | **Explained** (step 8 #16, #18): the lowering, the seed's address book and mail, today's month. Double quotes against single: step 8 #17, the spec's own quotes |

### Screens 06 to 13, the results

| # | Screen | Design | Built | Decision |
|---|---|---|---|---|
| 19 | 07 | The focused row on the accent at 14% (`accsoft`, dark) | 7%: hardly off the list's ground (43,49,60 against 59,65,78) | **Fixed** (`fix(macos): tint search's focus as the design does`): `SearchRuns.focusFill`, 8% light and 14% dark, on every search surface with a ring |
| 20 | 06 | Top hits' reasons: two at most ("you replied · 3 matches") | "you replied · frequent sender · …", cut at 176 | **Not fixed** (step 3 #15): `reason_line` keeps two reasons *and* the matches, while D20 says two at most. Changing it is a words decision (which two), and its test says the opposite today |
| 21 | 06 | The timeline 74 tall in the PNG; Top hits at 190 | 66 (`McSearch.dc.html`'s height); Top hits at 183 | **Explained**: built to the source's 66, which the PNG renders taller |
| 22 | 06 | Group headers "September 2026  9 · newest first" | The same rule, the seed's months | Same |
| 23 | 08 | Six senders, none of them you | Eight, no "You" row | **Fixed** with 11: the From popover leaves you out unless the query already asks for you, so the check can be taken off. **To keeps you**: mail to you is a real narrowing |
| 24 | 08 | The popover hangs under From | Centred on the button as it was when opened; checking Ada widens it to "From: Ada Moreno" | **Explained** (step 4 #1, #7): AppKit centres a popover on its anchor; the preview is live, so the button's words change under it |
| 25 | 09 | "This quarter" ringed, July to September | "Any time" ringed, July to October | **Explained**: A1 (This quarter starts 1 October here, step 4 #10) |
| 26 | 09 | Timeline hint "Jul – Sep selected · drag to change ·" / "⌥← → steps a month" | "… drag to change · ⌥←" / "⌥→ steps a month": the pair breaks across the lines | **Explained**: the pair is spelled as every pair is (step 4 #16); the 230 column wraps it where it falls |
| 27 | 10 | One card per file: "Atlas-Q3-budget.xlsx · Sheet “Q3”, row 3" | Two: the file's name as its own passage, then its row | **Fixed** (`fix(session): one Quick Look card for a file`): a file whose contents matched is its content's card |
| 28 | 10 | "to you, Finance team" on the sender line | No recipients | **Not fixed** (step 5 #5): neither the row nor the matches carry the best message's To; a read of its own. Small follow-up |
| 29 | 10 | Deep shadow (0 30 90, 35%) | The window server's panel shadow, lighter | **Explained**: the platform's shadow for a panel (step 5 #2) |
| 30 | 10 | Earlier reply "You · 24 Sep"; cards in the result's order | No earlier reply in the seed; oldest first | **Explained** (step 5 #8, #9) |
| 31 | 11 | Eight cards incl. a Numbers table and a photo; note names iWork and images | Four; the note names PDF, Office documents and text | **Explained** (step 9 #1, #5, #6): iWork and text in images are not read |
| 32 | 12 | "Appears at the top of search as ⌥5" | "Appears at the top of search" | **Explained** (step 6 #5): the demo already pins four, and only ⌥1–⌥4 exist |
| 33 | 12 | "⇧X select all 12" | "⇧X select all 7" | Same: the cap spells Shift since step 6, and the seed's count |
| 34 | 13 | Four ways out | Two | **Explained** (step 7 #3, #4): a way out that finds nothing is not offered; most first |
| 35 | 13 | "Mail older than January 2019 isn't downloaded: search the server too ⌥↩" | Absent | **Explained**: searching the server is out of scope (S4, step 7 #10) |
| 36 | 13 | The whole field ringed while nothing matches | The same | Same: step 7 #11 was fixed after its note (`feat(focus): ring the query field while nothing matches`) |

### Carried from steps 2 to 10

What the earlier notes left open, and where it stands now. Rows they
called **Same** or **Fixed** are not repeated.

| Note | # | What | Now |
|---|---|---|---|
| step 2 | 5 | The panel's own shadow (`0 16 48`) | **Explained**: the window server's, visible in the real captures, lighter |
| step 2 | 24 | At rest the placeholder is the bar's long one, cut at 320 | **Not fixed**: one placeholder is exported; a resting one is a `postio-ui` word and an export. Small follow-up |
| step 2 | 25, step 3 16 | "in:INBOX" | **Fixed** (12) |
| step 3 | 14a | Rows matched by a file name alone show no passage | **Explained**: a name is its own words |
| step 3 | 15 | The reason line's third part | **Not fixed** (20) |
| step 4 | 4, open question | "You" among the senders | **Fixed** (11, 23) |
| step 5 | 5 | No recipients in Quick Look | **Not fixed** (28) |
| step 6 | 2, 4, 5 | The Save popover drawn grey, its field unringed, its switch grey | **Settled** by the real capture: the snapshot's artefacts |
| step 6 | 11 | "X select all" | **Fixed** since: the cap reads "⇧X" |
| step 7 | 11 | The field not ringed while nothing matches | **Fixed** since (36) |
| step 8 | field | The field held at 327 wide, every row cut | **Fixed** since (`ca9f2bff`, `e9e652a2`): 860, right edge 12 |
| step 8 | 1 | The ghost's offset | **Fixed** (6) |
| step 8 | 2, 13 | "Tab completes", "↩ adds" | **Not fixed** (7, 15) |
| step 9 | 18, 19 | The system's Quick Look and save panel; arrows on the grid | **Explained**: the system's own windows, outside a demo's keys (`ffi_suite` proves the copy) |
| step 10 | left as is | Before the People tab is opened, "N people" counts you | **Explained**: the search would read every address per keystroke to leave you out; the count corrects itself when the tab reads its rows |

So nothing is unexplained (SC-005). What is **not fixed** is all small,
and each wants a word from `postio-ui` or a read the controller does not
make yet: the empty state's message count (5), "Tab completes" (7), the
operator's prefix in tertiary (14), "↩ adds" (15), the reason line's
two-at-most (20, a words decision), Quick Look's recipients (28) and the
resting placeholder.
