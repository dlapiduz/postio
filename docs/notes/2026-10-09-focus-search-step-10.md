# Focus search, step 10: the People tab against design §3.11

2026-10-09, specs/010-focus-search T132-T134. The results' third tab
lists who the matched mail is from and to. The design has no picture of
it (§3.11: "Not drawn yet. Build the tab with a simple list (avatar,
name, address, message count, last date). ↩ runs `from:<person>`."). So
this step is compared against those words and against the rows around
it: a Conversations row (§3.4, screen 06) and the From popover's people
(§3.6, screen 08).

## What is built

- **Engine**: `executor::people` (`postio-index/src/executor/people.rs`)
  folds the conversation search's own capped walk by address
  (`conversations::correspondents`). Each person gets the matched messages
  from them (`received`) and to them (`sent`, not also from them) and the
  newest of those. Then one statement reads the addresses and a name their
  mail gave, leaving out the person's own: every account's and identity's
  address. That makes two statements whatever the match. The answer is
  `suggest::Person`, the shape the data model names. The order is most
  messages first, then the most recent, then the address.
- **Request**: `Req::People` goes through client, host and session. It
  is cancellable, on its own `Lane::People` in the controller, and asked
  when the tab is first shown for a query, as the Files tab's cards are.
- **Controller** (`postio-focus/src/results.rs`): j/k and the arrows move
  the ring, and ↩ opens `from:<address>` on the Conversations tab. What
  was shown goes behind it, so ⌘[ comes back to the People tab. The tab's
  count and the sub-line's "N people" are the rows' count once they are
  read, and the search's own count until then.
- **Words** (`postio-ui`): `person_messages` ("3 messages"),
  `person_accessible`, and `people_hints` ("move", "search their mail",
  "switch tab").
- **Mac**: `PeopleListView` (PostioKit) takes the table's place. It reads
  rows through `focus_search_person` / `PersonRowFfi`. A click tells the
  controller, and a double click is ↩.
- **The search demo** writes "You" as an identity of its account. Without
  it the first row was "You, you@example.com, 128 messages": the seed's
  account address is `test@example.com`, and nothing said that
  `you@example.com` was also yours.

## The capture

```bash
POSTIO_FFI_FEATURES=demo scripts/macos-build.sh && scripts/macos-bundle.sh
POSTIO_DEMO=search POSTIO_WINDOW_SIZE=1440x900 POSTIO_APPEARANCE=light \
  POSTIO_DEMO_KEYS='/ atlas ⌘⏎ ⌘3' \
  POSTIO_DEMO_SNAPSHOT=/tmp/people.png macos/build/Postio.app/Contents/MacOS/Postio
# ↩ on the second person: POSTIO_DEMO_KEYS='/ atlas ⌘⏎ ⌘3 j ⏎'
```

### "atlas", People

| # | Design (§3.11 and its neighbours) | Built | Decision |
|---|---|---|---|
| 1 | A simple list | Nine rows in the table's place, under the same filter bar and timeline | Same |
| 2 | Avatar | 32 round, initials in white on the From popover's hue for the name (`PopoverRowView.avatar`) | Same as the popover's; 32 rather than its 28 because the row is 58 tall |
| 3 | Name, address | Name semibold 13.5 over the address in SF Mono 12, secondary: the popover's name-over-address | Same |
| 4 | Message count | "58 messages", secondary, right-aligned in a 110 column | **Explained**: the matched messages from or to them, not all mail with them. The tab is "the people in the results" (FR-032), and the popover's counts are of the results too |
| 5 | Last date | "9 Oct" in the result rows' 62 date column, with the same `hit_date` rule | Same |
| 6 | Rhythm | 58-tall rows, a hairline under each, the accent ring and 7% tint on the focused one, the same leading and trailing as a Conversations row | Same |
| 7 | ↩ runs `from:<person>` | ↩ on Tomás: the field holds the chip "from: Tomás Reyes", Conversations is selected (23), and From reads "From: Tomás Reyes" | Same. The query is the person alone, as US9's test says, not added to "atlas" |
| 8 | — | Footer "j/k move · ↩ search their mail · ⌘1/⌘3 switch tab" | The tab's own keys, as the Files tab has its own |
| 9 | You are not a person to search for | No "You" row | Same, through the identity fix above |
| 10 | — | Sub-line "8 files · 9 people", tab "People 9" | First capture: the sub-line said "10 people". It used the search's count, which counts you. **Fixed**: once the rows are read, the sub-line uses the same count as the tab |

## Left as it is

- **Before the tab is opened, "N people" counts you.** The conversation
  search counts every address on the matched mail (`people_in`), and
  leaving the person out there would cost the search a read of their
  addresses on every keystroke. The count corrects itself when the tab
  reads its rows, as the Files tab's does.
- **No header over the list.** The Files tab has "Files whose name or
  contents match". §3.11 asks for a simple list, and the count line and
  the tab already say what it is.
- **Rows the person wrote and someone else's thread** show "You" as the
  sender on the Conversations tab after ↩ (screen 06's rule: a
  conversation is drawn by its best message). `from:tomas` finds the
  threads with Tomás's mail in them, and some end with your reply.
