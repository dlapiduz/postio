# What closing the command sweep cost (2026-09-21, #1571–#1576, #1584, #1585)

The sweep in `crates/postio-ffi/tests/ffi_suite/command_coverage.rs` started
the day at 49 orphans and ended at 4. This is what the 45 actually were,
because "not wired up" turned out to describe almost none of them.

## The same root cause, nine times

Every one of the following was a decision living in `Engine.swift`,
`Shell.swift` or a `View`'s `@State` — the executable target, which
`macos/Package.swift` gives the test target no way to reach:

| What was dead | Where the decision lived |
|---|---|
| `H` (*render part once*) | `@State showingImages` in `ExpandedMessage` |
| `⌘O` (*view original*) | the same, one flag over |
| the seven account verbs | `@State selected` in `SettingsPaneView` |
| the four saved-search verbs | there was no cursor at all |

The fix each time is the same shape and takes about twenty minutes: a small
`@Observable` type in `PostioKit` with the rule and a handful of tests
(`RenderedOnce`, `OriginalView`, `SettingsAccounts`, `SavedSearches`,
`PartsModel`), and one line left in the executable target that calls it.

**The tell is always the same**: a button works and the key beside it does
nothing. A command cannot reach an `@State`, so any state a command needs is
state a view may not own. That is worth checking for directly the next time a
key is reported dead — it is faster than reading the dispatch path.

## A command that needs a dialog asks, it does not call

Five surfaces here needed the same mechanism and each had invented it
separately or not at all. The settled shape is a *wish* on the model — a
value plus a **token** — and an `.onChange(of: model.wishToken)` in the view
that grants it. Watching the value alone is the bug: two saves in a row are
two saves, and `onChange` on an unchanged value fires once.

`PartsModel.Wish`, `SettingsAccounts.Wish` and `SavedSearches.Wish` are the
three written this way. `ComposeModel`'s older `wantsLink` / `wantsAttachment`
booleans are the same idea before the token, and they have the
two-in-a-row bug in principle; they are reset to `false` on grant, which
papers over it.

## `Context::Search` is not "the field has focus"

Two commands — `o` (*toggle result order*) and `⌘S` (*save search as folder*)
— are scoped to `Context::Search`, and this frontend was reporting that
context only while the query field had the keyboard. Both are pressed *after*
leaving the field, and the field is a text field where `KeyMonitor` refuses
bare characters. So both resolved in the one place they cannot be pressed and
nowhere they can.

The list is in `Context::Search` for as long as what it is showing is
results (`SearchContext`). Worth remembering as a general shape: a context
named after a *surface* usually means the surface's **output**, not its input.

## A ranking fixture that cannot fail

Three attempts were needed to write one test — "`o` puts the results in date
order" — and the first two passed while proving nothing.

1. **Days apart, no non-matching corpus.** `rank_score` folds recency in with
   a calibrated weight, and days of it outweigh any term density, so
   relevance *was* date order.
2. **Shortest document newest.** BM25 favours the shorter document, so the
   best match was also the newest — same list twice again.
3. What works is the shape `index_suite`'s
   `newest_order_answers_in_date_order_however_the_ranking_disagrees` already
   proved: twenty non-matching messages so IDF is not zero, and **hours**
   rather than days between the two matches.

The fix that matters is not the fixture, it is the assertion beside it:
`assert_ne!(by_relevance, newest_first)`. A test that two orders differ needs
to fail when the fixture stops telling them apart, or it is asserting that
`[3,2,1] == [3,2,1]`.

## Addressing a part by row id breaks after one fetch

`postio-app`'s `save_all_parts` handed each row's `AttachmentId` to
`export_part_as`. A whole-message fetch **replaces** a message's attachment
rows — the parser re-reads the structure and `MessageRepository::update`
writes the new set — so the first part of a batch that had to be fetched
invalidated every id held beside it, and every part after it failed. Any
message whose attachments are not already local, which is the ordinary state
of one that has been described and not downloaded.

`postio_session::reading::part_bytes`' doc has said so all along, and the FFI
boundary has never named a part by a row id for exactly this reason. The rule
for anything holding a part across an operation: **`2` is `2` in every parse
of the same bytes; a row id is not.**

## Four commands that are decisions, not wiring

What is left in `KNOWN_ORPHANS` is worth stating so nobody picks it up
expecting an afternoon:

- `insert_image` needs an inline attachment with a `Content-ID` and a
  `postio-cid:` handler in the **composer's** web view. The reader has one;
  the composer does not.
- `detach_composer` has nothing to detach — compose on macOS is already a
  window of its own and never takes over the reading pane.
- `next_scope` cycles an account strip. This sidebar lists every account's
  folders at once instead of re-rooting to one, and the account scope is
  *derived* from the open list rather than chosen.
- `toggle_rail` needs the conversation rail, which is not drawn here.

Two of the four are the same kind of thing: **a frontend that chose a
different shape does not have a home for a command written against the other
shape.** That is a product question, and `KNOWN_ORPHANS` is the wrong place
to answer it — but it is the right place to say which question it is.

## Sharing a rule you cannot compile

`schedule_presets` — what "tomorrow morning" means — lived in
`postio-gtk/src/composer.rs`. Two frontends each deciding that is two
products, and the one that is wrong sends somebody's mail at the wrong hour
without ever saying so, so it moved to `postio_ui::compose`.

`postio-gtk` cannot be compiled on this Mac (no gobject), which makes any
edit to it unverifiable until CI. The shape that keeps the risk small: put
the rule in the shared crate with its tests, and leave a **thin shim** at the
old call site returning the old type — here, four call sites and four tests
kept working through a three-line function. Deleting the helpers it used
(`at_local_time`, `MIN_SCHEDULE_LEAD`) then required trimming the `chrono`
import, because CI's clippy is `-D warnings` and an unused import is an error
there and a warning here.
