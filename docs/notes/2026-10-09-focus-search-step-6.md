# Focus search, step 6: selection, the bulk bar and Save search against screen 12

2026-10-09, specs/010-focus-search T093-T101. In the results `x` checks
the focused result and ⇧X checks every conversation the query matches;
while anything is checked the footer is the bulk bar ("5 selected",
Archive a, Label l, Move m, Mark read r, Snooze s, and "⇧X select all N"
on the right until everything is). ⌘S, or a click on Save search, hangs
the Save popover. Everything decided is the controller's
(`postio-focus/src/results.rs`); the Mac draws `FocusResults` and
`FocusSavePopover`.

## What a verb from the results reaches

- **Checked rows**: their conversations, as one `MessageTarget::Threads`
  (one unit, one undo), the same aim the inbox's selection makes.
- **⇧X**: never the rows read. The controller sends the query and the rows
  taken back out (`Everything { query: Some(..), except }`, the inbox
  predicate's query form) as `Req::SendMatching`; the host resolves it on
  the command queue with the results' own match
  (`postio_session::search::matching`, which walks `search_conversations`)
  into one `Messages` target, so the verb runs in order with every other
  command and one ⌘Z takes it all back. Capped as the results' count is.
- **Nothing checked**: the focused result, as before.
- **l, m, s** open the list's pickers at the focused result over the same
  aim (`Anchor::Result`); the checks go once a picker acts.
- **After a verb lands, or its undo**, the results are asked again. Search
  spans every folder, so an archived result *still matches*: its row stays
  and says `in:Archive`; a deleted one leaves. Dropping archived rows would
  show a list the query does not describe. The focus ring stays where it
  was.

## Saving

The popover writes four things to `[saved_searches.<key>]`: `name`,
`pinned` (and, pinned, the next free `order` -- the others are given the
places they were shown in), `notify` (new, off unless said, not written
when off) and the query, whose dates are rewritten relative when Keep the
date rolling is on (`after:90d`) and to their day when it is off
(`after:2026-07-01`) -- D14, both the one language
(`postio_search::edit::Edit::Dates`). The name offered is
`postio_ui::search_view::save_name`: the words, capitalised, then "from"
and "to" with each person's first name.

## The badge

A search with `notify = true` shows "N new" beside its count in the empty
dropdown: the conversations whose newest match arrived after the search
was last viewed (`saved_search_seen`, D15). Running it, or saving it with
Notify on, sends `MarkSeen` (`seen_up_to` = the app clock's now). A search
never viewed has no badge. New mail recounts only the searches that
notify, while the dropdown is up, merged by key. Counts are keyed by the
`[saved_searches]` key now, not the name.

## The capture

```bash
POSTIO_FFI_FEATURES=demo scripts/macos-build.sh && scripts/macos-bundle.sh
POSTIO_DEMO=search POSTIO_WINDOW_SIZE=1440x900 POSTIO_APPEARANCE=light \
  POSTIO_DEMO_KEYS='/ atlas ␣ budget ⌘⏎ @from ␣ ⏎ @date since ␣ july ⏎ ⌥o x j x j j x j x j j x ⌘s' \
  POSTIO_DEMO_SNAPSHOT=/tmp/12.png macos/build/Postio.app/Contents/MacOS/Postio
```

## Screen 12

| # | Design | Built | Decision |
|---|---|---|---|
| 1 | Popover 364 wide under Save search, its right edge 16 in from the window's | First capture: centred on the button, it hung past the window and was cut off | **Fixed**: hung from a point level with the button, 16 in |
| 2 | Popover white, a hairline, a soft shadow | White content inside a grey frame | **Explained**: the snapshot draws the popover's frame without its material or shadow; first capture had the whole popover grey, so the content now has the text background |
| 3 | "Save as a saved search", Name "Atlas budget from Ada", chips `from:Ada Moreno`, `after:2026-07-01`, `atlas budget` | The same words, the same chips, wrapped the same | Same |
| 4 | Name field with the accent focus ring | A rounded field, no ring | **Explained**: the snapshot is drawn with the popover not key; a live popover rings the field |
| 5 | Pin on (a filled black switch), "Appears at the top of search as ⌥5" | Pin on, drawn grey; "Appears at the top of search" | **Explained**: the switch is grey because the snapshot's window is not key. The demo has four saved searches and only ⌥1-⌥4 exist, so a fifth has no key; first capture said "as" with nothing after it -- **fixed**, the note drops "as" when there is no key |
| 6 | "Off: always since 1 July. On: always the last 90 days" | "... the last 100 days" | **Explained**: counted from the demo's today (9 Oct), not the screen's (29 Sep) |
| 7 | Cancel (grey), Save ↩ (filled black) | The same | Same |
| 8 | 12 conversations, Sep/Aug/Jul groups, five checked, the ring on the seventh | 6 conversations (the seed's Ada matches since July), Oct/Sep/Aug/Jul groups, five checked, the ring on the sixth | **Explained**: the seed's mail and dates; the same keys check the same pattern |
| 9 | Checked rows: black filled box, neutral tint | Accent-filled box, neutral tint | **Explained**: the box wears the system accent, as the list's does (spec 009); the design's accent is graphite |
| 10 | Bulk bar: "5 selected", Archive a, Label l, Move m, Mark read r, Snooze s, each word bold then its cap | The same | Same; first build drew them as the footer's hints (cap first) -- **fixed** |
| 11 | "⇧X select all 12" on the right | "X select all 6" | **Explained**: the keymap spells Select all as `X`, and the caps draw a capital as itself everywhere; spelling Shift into every capital's cap is a change to `KeyCapSpelling` for the whole app, left for its own review |
| 12 | Save search button with ⌘S, outlined | The same | Same |
| 13 | Sort control under the popover | Hidden under it | Same |

## Left as it is

- **Two Return paths**: Return in the popover's field reaches both the
  field's submit and the key monitor's interception; the model sends one
  save per opening, so it is written once.
- **A query with no date** draws no rolling switch rather than a disabled
  one.
- **⌘S in the dropdown** (not the results) still saves at once with the
  defaults, as it did: the popover hangs from the results' button.
