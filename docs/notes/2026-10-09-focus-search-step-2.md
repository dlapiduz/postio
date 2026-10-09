# Focus search, step 2: the dropdown against screens 01 and 03

2026-10-09, specs/010-focus-search T047-T055. The Mac's bar draws the
search dropdown's empty and words states from the controller
(`postio-focus`, `Intent::Dropdown`), under a toolbar field that grows to
860. This note lists every difference from the design's screens 01 and 03
that the code shows, and what was done about each.

## The captures: pending, display unavailable

`POSTIO_DEMO_KEYS='/' scripts/macos-shot.sh 01-typing-empty --seed search
--size 1440x900 --light` built and launched the demo, found the window, and
`screencapture -l` answered "could not create image from window": the
terminal this ran from has no Screen Recording grant, or the display was
asleep. Screen 03 is the same command with `POSTIO_DEMO_KEYS='/ atlas ␣
budget'` and `03-typing-words`. Both are owed: run them from a session with
the grant, compare with `Design/focus-macos-search/screens/01-*.png` and
`03-*.png`, and amend the list below with what only a picture shows
(spacing, weights, the shadow). T055 stays unticked until then.

The differences below are read from the code against SPEC.md §2, the
`McSearch*.dc.html` sources and the PNGs.

## Both screens

| # | Design | Built | Decision |
|---|---|---|---|
| 1 | The field is 34 tall, radius 8, 15 pt text (14 pt SF Mono for an operator), a 2 pt accent ring and a 4 pt soft accent halo | The toolbar's `NSSearchField` (`NSSearchToolbarItem`): the system's height, radius, font and focus ring | **Not fixed.** The toolbar item draws its own field; the design's is the chip field (`ChipQueryField`, plan "Swift"), which steps 3 and 8 build because chips need it anyway. Built here: the width |
| 2 | The field grows leftward to 860, right edge 12 from the window's | `preferredWidthForSearchField` = `CommandBarGeometry.fieldWidth` (860 at 1440; the window less 172 below that) while the bar is up, 320 after | **Fixed (T054).** The right edge is where the toolbar puts its trailing item; the 12 is unverified until a capture |
| 3 | An `Esc` keycap inside the field's right end | The ⌘K keycap while the field is empty, nothing once typed | **Not fixed**, with 1: it belongs to the chip field |
| 4 | Panel 6 below the field, same left edge and width, radius 10, `separatorColor` outline | `CommandBarGeometry.frame`: 6 below, the field's left edge and width, inside the window; radius 10 and a 1 pt separator stroke in `CommandBarView` | **Fixed (T054)** |
| 5 | Shadow `0 16 48 rgba(0,0,6,0.22)` | The window server's shadow for a borderless panel (`hasShadow`) | **Not fixed.** A custom shadow needs a transparent inset drawn inside the panel; the system's reads as the same lift. Revisit with the captures |
| 6 | The inbox behind is not dimmed | Not dimmed | Same |
| 7 | Rows 38 tall, inset 6, radius 6; 24 icon column, 100 folder column (SF Mono 11.5), 110 right column; focused row: 2 pt accent ring inset, accent fill 8-10% | `DropdownView.Metrics` as written; ring 2 pt, fill 9% | Same in code |
| 8 | Section titles 12 pt bold secondary, notes 12 pt tertiary | As designed | Same |
| 9 | Footer 34 tall, key hints left, count right | 34 tall; hints from `postio_ui::search_view::{empty,words}_hints` through the keymap | Same, except 13 below |
| 10 | Row icons are the design's glyphs (↺, an envelope, ≡) | SF Symbols `arrow.counterclockwise`, `envelope`, `line.3.horizontal` | **Explained**: the Mac's symbol set, at 11 pt tertiary |

## Screen 01, the empty state

| # | Design | Built | Decision |
|---|---|---|---|
| 11 | Placeholder "Search mail, people and files, or type > for commands" | `postio_ui::search_view::PLACEHOLDER`, set on the field | Same |
| 12 | Recent: "atlas budget 48 results yesterday", "from:ada invoice 6 results Mon", "has:attachment in:Receipts after:2026-09-01 19 results 21 Sep"; queries with operators in mono; "⌥⌫ forgets one" | The search seed remembers the same three queries and counts, run 1, 5 and 9 days before the demo's today (the real date); `recent_when` says "yesterday", the weekday, then "30 Sep"-style | **Explained**: the design's "Mon" and "21 Sep" cannot both hold under T045's rule for a Saturday 26 September (21 Sep is within the week, so "Mon"); the third is a week back instead |
| 13 | Footer right: "local index · 18,204 messages" | Nothing on the right in the empty state | **Not fixed.** No count is read for the empty state; it wants the store's message count, which `FocusCounts` does not carry. Small follow-up: a `postio-ui` word and the count from the saved-counts read |
| 14 | Saved searches: name bold, count, ⌥n keycap, outlined, small radius | As designed; radius 6, 1 pt separator stroke | Same. The counts are the seed's real ones for `[saved_searches]` in `postio_demo::config()` (`from:juno`, `subject:atlas`, `in:Receipts`, `from:northfield`), not the design's 5/38/19/4 |
| 15 | Search by: eight operators in four columns, ops mono, hints secondary | `cheat_sheet()` in a four-column grid | Same |
| 16 | "Or just type it: **invoices from ada last month** becomes `from:ada invoice after:2026-08-01 before:2026-09-01`" in a light box | The sentence lowered live by `natural::lower_with_origins` against today and the address book, the box `quinary` | **Explained**: the lowering is the one language's (FR-004 says it must equal `lower`). It keeps "invoices" plural, and writes `from:ada` only when the address book knows Ada (it does in the demo); the dates are the month before the real today's |

## Screen 03, the words state

| # | Design | Built | Decision |
|---|---|---|---|
| 17 | Top hits: up to four, "Sender · Subject" with matched words in the find yellow, a passage, `in:Inbox`, the date | `Request::Conversations { limit: 4 }`, the highlight from `highlight::find` over the query's terms, passages from `Request::Passages` once the hits land, `in_folder`, `hit_date` | Same. Matched words use the system yellow at 38% (light) and 28% (dark), the design's two values |
| 18 | The fourth hit's detail is a file name ("Atlas-budget-template.xlsx") | A file-name match has no passage until step 9's attachment text, so its detail is empty | **Not fixed** until step 9 (`Source::FileName` has a name, which `postio-ui` can word then) |
| 19 | "ranked by sender, recency and where the words matched" | `TOP_HITS_NOTE` | Same |
| 20 | Narrow to: four pills "from: Ada Moreno 21", "from: Tomás Reyes 9", "has: attachment 12", "label: Atlas 30", the title inline | `narrow_pills`: two top senders, attachments, the top label, each only when it narrows (fewer than all), four at most; title inline, hairlines above and below | Same |
| 21 | "Show all 48 results  in the main window, with filters, a timeline and Quick Look ⌘↩", focused by default | As designed, with the binding's keycap; ⌘↩ (and Show all) keep the query among the recents and move the highlight to the first hit until the results view (step 3) | Same |
| 22 | Footer "↑↓ move · ↩ open message · ⌘↩ all results · Tab add first filter" and "48 matches · 38 ms" | As designed; the time is the search's own (`ConversationResults::elapsed`) | Same |
| 23 | The typed words in 15 pt | The system field's font | With 1 |

## What else this step decided

- `>`, `in:` and `@` still draw spec 009's lines on the Mac: their own
  dropdown states (operator, plain English, short prefix) are step 8's.
- Typing is not debounced (D8). The driver aborts a lane's superseded task;
  twelve keystrokes of "atlas budget" over the search seed completed ten
  searches before and at most two after (`ffi_suite/focus_search.rs`).
