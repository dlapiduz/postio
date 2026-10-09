# Contract: what `postio-ffi` exports for search

UniFFI 0.32, proc-macro, as spec 009's
[ffi-focus.md](../../009-focus-macos/contracts/ffi-focus.md). New exports
live in a new `crates/postio-ffi/src/focus_search.rs`; the bar's existing
exports in `focus_bar.rs` keep their names and meaning. Every record is an
`*Ffi` mirror of a `postio-focus` view, with ids as `i64`, words already
composed in Rust by `postio-ui`, and highlight ranges as UTF-16 offsets
(what `NSAttributedString` takes), converted once in `focus_search.rs`.

Swift calls these and draws what comes back. It never parses a query,
builds one, or decides which state the dropdown is in (009 FR-004).

## Calls (on `Session`)

All are synchronous and cheap: each hands an `Input` to the controller and
returns. Answers arrive as `UiEvent`s.

| Export | What it tells the controller |
|---|---|
| `focus_bar_typed(text)` | *(existing)* the field's text now. In search mode it now also reshapes the dropdown (FR-011) |
| `focus_bar_run(token)` | *(existing)* ↩ on a dropdown row: open a hit, apply a suggestion, re-run a recent search |
| `focus_bar_tab() -> bool` | *(existing)* Tab: accept the ghost, add the first "Narrow to", or turn plain English into chips |
| `focus_search_highlighted(token)` | the arrow highlight moved (Swift's, 009 FR-004): the controller asks for "Latest from <person>" when the row is a person |
| `focus_search_exclude(token)` | ⌥↩ on a person, label or folder row |
| `focus_search_forget(token)` | ⌥⌫ on a recent row |
| `focus_search_show_all()` | ⌘↩, or "Show all": enter the results view |
| `focus_search_edit(edit: TermEditFfi)` | a filter button, a chip's ✕, a popover check, a relaxation picked |
| `focus_search_tab(tab: ResultsTabFfi)` | ⌘1–3 or a click on a tab |
| `focus_search_order(order: ConversationOrderFfi)` | the Sort menu |
| `focus_search_months(first: i32, last: i32)` | a timeline drag ended over months `first..=last` (0 = oldest bar) |
| `focus_search_popover(kind: PopoverKindFfi)` | a filter button pressed: open its popover and remember the query to restore |
| `focus_search_popover_filter(text)` | the popover's own search field ("Filter people in these results") |
| `focus_search_popover_done(apply: bool)` | ↩ (true) or Esc / click-away (false) |
| `focus_search_date_words(text)` | the Date popover's plain-words field |
| `focus_search_quick_look_file(attachment: i64) -> Option<String>` | Space on a file card: the path of a temporary copy for `QLPreviewPanel` (FR-053), or none |
| `focus_search_save(name, pin, notify, rolling)` | Save ↩ in the Save popover |
| `focus_search_row(position: u64) -> Option<ResultRowFfi>`, `focus_search_row_count() -> u64` | the results table's rows, over the controller's window, as `row_at` is for the list |
| `focus_search_file(position: u64) -> Option<FileCardFfi>`, `focus_search_person(position: u64) -> Option<PersonRowFfi>` | the Files and People tabs |

History, Quick Look, selection and the Esc ladder need no new exports: they
are commands (`HistoryBack`, `QuickLook`, `ToggleSelect`, `Back`, …) through
the existing `key(..)` and `command(id, origin)`.

`TermEditFfi` mirrors `postio_search::edit::Edit`:

```text
enum TermEditFfi {
  Add { field: String, value: String, negated: bool },   // "from", "ada@example.com"
  Remove { token: u32 },
  Toggle { field: String, value: String },                // "has", "attachment"; "is", "unread"
  SetMonths { first: i32, last: i32 },
  ClearFilters,
}
```

`field`/`value` are the operator's keyword and its value; Rust spells the
token (D13). Swift never concatenates query text.

## Events (`UiEvent`, appended at the end)

| Variant | Payload | When |
|---|---|---|
| `FocusDropdown { view: DropdownViewFfi }` | the whole panel | every keystroke's answer that is still current |
| `FocusQuery { view: QueryViewFfi }` | the field's chips and words, and the filter bar's button states | after any edit, in the dropdown and in results |
| `FocusResults { view: ResultsViewFfi }` | the results view's frame: counts, tabs, timeline, groups, footer | entering results, and after every query, tab or order change |
| `FocusResultsPage { first: u64, count: u64 }` | rows `first..first+count` changed; re-read them | pages and passages landing |
| `FocusLeaveResults` | back to the inbox; the list's own cursor events follow | Esc's last rung, ⌘[ |
| `FocusPopover { view: PopoverViewFfi }` | a filter popover's rows, preview counts and date chart | open, and each live change |
| `FocusQuickLook { view: Option<QuickLookViewFfi> }` | the panel, or `None` to close it | Space, j/k, ]/[ |
| `FocusSavePopover { view: Option<SaveViewFfi> }` | name prefilled, chips, switches | ⌘S |
| `FocusRelaxations { view: NoResultsViewFfi }` | the no-results page | a zero-hit answer |

## Records

```text
record DropdownViewFfi {
  state: DropdownStateFfi,            // Empty | Prefix | Words | Operator | PlainEnglish | Commands
  ghost: Option<String>,              // drawn tertiary after the caret
  understood: Vec<UnderstoodTileFfi>, // PlainEnglish only
  sections: Vec<DropdownSectionFfi>,
  highlight: Option<u64>,             // the row focused by default (e.g. "Show all")
  footer_hints: Vec<KeyHintFfi>,
  footer_count: Option<String>,       // "48 matches · 38 ms"
}
record DropdownSectionFfi { title: String, note: Option<String>, rows: Vec<DropdownRowFfi>, pills: Vec<PillFfi> }
record DropdownRowFfi {
  token: u64, kind: DropdownRowKindFfi,   // Recent | Hit | Word | Label | List | File | Person | Folder | ShowAll | CheatSheet | Example
  title: Vec<RunFfi>,                     // with highlight runs
  detail: Vec<RunFfi>,                    // passage, address line, "6 results"
  folder: Option<String>,                 // "in:Inbox"
  right: Option<String>,                  // date, count or "yesterday"
  keycap: Option<String>,                 // "⌘↩", "⌥1"
  initials: Option<String>,               // people rows
}
record PillFfi { token: u64, label: String, count: Option<String>, keycap: Option<String> }
record UnderstoodTileFfi { term: String, origin: String }   // "from:ada", "from ‘ada’"
record RunFfi { text: String, highlighted: bool }

record QueryViewFfi {
  chips: Vec<ChipFfi>, words: String, hint: String,     // "/ to edit", "⌘⌫ clears filters"
  buttons: Vec<FilterButtonFfi>,
}
record ChipFfi { token: u32, operator: String, value: String, excluded: bool, focused: bool }
record FilterButtonFfi { kind: FilterKindFfi, label: String, applied: bool, open: bool }
  // FilterKindFfi: From | To | Date | Anywhere | Label | Attachment | HasAction | Unread

record ResultsViewFfi {
  tabs: Vec<TabFfi>,                  // label, count, selected, keycap
  order: ConversationOrderFfi,
  count_line: String, sub_line: String,              // "48 conversations", "12 files · 6 people · last 12 months"
  months: Vec<MonthBarFfi>,                          // 12
  groups: Vec<GroupFfi>,                             // title, count, first row position
  footer_hints: Vec<KeyHintFfi>, footer_right: String,   // "48 conversations · local index · 41 ms"
  selected: u64, bulk: Vec<KeyHintFfi>,              // the bulk bar when selected > 0
}
record MonthBarFfi { label: String, conversations: u64, height: f64 /* 0..1 */, selected: bool }
record ResultRowFfi {
  id: i64, thread: Option<i64>, group: u32, top_hit: bool,
  sender: String, unread: bool, reason: Option<String>,
  subject: Vec<RunFfi>, pills: Vec<LabelPillFfi>, attachments: bool, count_badge: Option<String>,
  source_tag: String, source_is_file: bool, passage: Vec<RunFfi>,
  folder: String, date: String, checked: bool, accessible: String,
}
record FileCardFfi { attachment: i64, message: i64, kind: String /* "XLSX" */, name: Vec<RunFfi>, line: String, matched: Vec<RunFfi>, subject_line: String, preview_unit: Option<String> }
record PersonRowFfi { address: String, name: Option<String>, initials: String, messages: u64, last: String }
record PopoverViewFfi { kind: PopoverKindFfi, rows: Vec<PopoverRowFfi>, presets: Vec<PillFfi>, parsed: Option<String> /* "→ after:2026-07-01" */, months: Vec<MonthBarFfi>, result: Option<String> }
record PopoverRowFfi { token: u64, title: String, detail: Option<String>, initials: Option<String>, count: u64, share: f64, checked: bool, excluded: bool }
record QuickLookViewFfi { position: String, subject: Vec<RunFfi>, sender_line: String, matches_line: String, cards: Vec<MatchCardFfi>, current: u32 }
record MatchCardFfi { place: String, when: String, passage: Vec<RunFfi> }
record SaveViewFfi { name: String, chips: Vec<ChipFfi>, pin: bool, notify: bool, rolling: bool, rolling_note: String }
record NoResultsViewFfi { title: String, body: String, relaxations: Vec<RelaxationFfi>, searched: String }
record RelaxationFfi { number: u32, label: String, query: String, count: String }
```

`ResultRowFfi.accessible` is the VoiceOver sentence ("Ada Moreno, Re:
Atlas Q3 budget, matched in body: …, Inbox, 26 Sep"), composed in Rust
(design §5).

## Rules

- Events are append-only at the end of `UiEvent` (009 convention); the
  Swift `Engine` switch gains one case each.
- An event for a stamp the controller has moved past is never sent: Swift
  needs no staleness logic.
- Every count and time string is composed by `postio-ui`; Swift formats
  nothing but layout.
- `focus_search_quick_look_file` writes into the app's temporary directory
  and the controller deletes the copy when Quick Look closes (FR-053).
