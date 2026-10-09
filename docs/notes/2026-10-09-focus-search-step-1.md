# Focus search, step 1: the engine measured against 50 ms

2026-10-09, specs/010-focus-search T035-T037. Conversation search answers a
full query, facets and months included, inside the budget on a 20,000
message mailbox. Every shape's p95 is under 50 ms; the common word is the
one with little room.

## What was measured

`crates/postio-bench/benches/search_focus.rs`, release build, Apple M1 Pro
(32 GB), in-memory store, 60 timed runs per shape after 5 warm-ups,
`cargo bench -p postio-bench --bench search_focus`.

The corpus is deterministic: 20,000 messages in threads of 1-12, 40 senders
with a Zipf spread, 12 labels (0-2 per message), 30% with an attachment,
spread over 24 months, one account. The plan's 2,000 extracted attachment
units are not there: nothing reads attachment contents until step 9, so
there is no table for them. Add them in T120's neighbourhood and re-run.

Executor shapes call `search_conversations` with a first page of 50, best
match order. The `e2e` shapes are what the results view does per query: the
session's `conversations` (first page of 50, facets, months, names) and then
`passages` for those 50 hits. That sum is the number the footer will print.
`relaxations` is `relaxation_counts` over `relax(query)` for the zero-hit
query. Statement counts come from `test_support::counting`, so they are
statements through `postio_storage::sql`, on the benching thread.

| shape | query | p50 ms | p95 ms | stmts | conversations |
|---|---|---:|---:|---:|---:|
| one word (~1%) | `quarterly` | 2.36 | 2.63 | 3 | 203 |
| two words | `quarterly forecast` | 0.63 | 0.68 | 3 | 7 |
| operator only | `from:sender3` | 7.11 | 7.88 | 4 | 479 |
| operator + words | `from:sender3 regarding` | 11.15 | 12.47 | 4 | 460 |
| common word (~95%) | `regarding` | 43.25 | 44.71 | 3 | 1,646 |
| typed `a` | `a` | 39.85 | 40.17 | 3 | 1,560 |
| typed `at` | `at` | 42.13 | 42.38 | 3 | 1,523 |
| typed `atl` | `atl` | 3.33 | 3.39 | 3 | 366 |
| zero hits, four filters | `quarterly from:sender30 label:atlas has:attachment after:2025-01-01` | 0.72 | 0.73 | 2 | 0 |
| relaxations of that | five variants, one has hits | 2.68 | 2.74 | 5 | 1 variant |
| e2e one word | `quarterly` | 3.20 | 3.27 | 56 | 203 |
| e2e two words | `quarterly forecast` | 0.84 | 0.85 | 13 | 7 |
| e2e operator + words | `from:sender3 regarding` | 11.68 | 11.93 | 57 | 460 |
| e2e common word | `regarding` | 42.98 | 43.47 | 56 | 1,646 |
| e2e typed `atl` | `atl` | 4.22 | 4.72 | 56 | 366 |

(Numbers from the last of four runs; the earlier ones agreed within about 2
ms on every shape.)

## What to know

- **The common word is the shape to watch.** 43-45 ms p95 against a 50 ms
  budget is about 10% of headroom, on a fast Mac. `a` and `at`, which a
  person types on the way to a word, cost the same because they match
  nearly every message. A real mailbox with more recipients per message or
  longer bodies will be slower here than this corpus. The budget holds; it
  does not hold with room to spare, and nothing in step 2 should add work to
  this path without measuring it again.
- **The end-to-end time is the executor time plus about 1 ms** for the shapes
  with few hits, and about the same for the heavy ones. The passages cost is
  small because it reads only the page's bodies (one `load_body` each).
- **The statement count of the end-to-end shape is not constant.** The
  executor is 3-4 statements and the session adds three name reads, which
  is flat; `passages` then reads one body per hit that matched in its body,
  so up to 50 more (56 = 3 + 3 + 50). It is bounded by the page, not by the
  mailbox, which is the property the CLAUDE.md budget needs, but the
  statement-budget test in step 1's plan should assert "at most page + 6",
  not "the same count at 100 and 2,000 messages" for the passages leg.
- **`typed a` and `typed at` are the words as typed.** The bench sends the
  literal word the search bar would send while it is typed; `completions`
  (the prefix `a*` path) arrives in step 8 and T115 extends this bench with
  it. A trailing `*` in a free-text term is not a prefix match in
  `search_conversations`: `atl*` found the same conversations as `atl`.
- **Relaxations never drop free text.** `relax` offers one-token variants of
  filters, and a query whose only failing token is a word offers nothing
  that removes the word. The first version of the zero-hit shape used a
  word nothing contained and got no relaxation with a hit. Not a defect in
  this step; a note for US6 if "remove the word" is expected.

## Running it on a Mac

`postio-bench` lists GTK and libadwaita among its dev-dependencies, so on a
machine without pkg-config and gtk4 `cargo bench -p postio-bench` stops in
`glib-sys`. This run took the GTK-only dev-dependencies (`adw`, `gtk`,
`postio-gtk`, `postio-widgets`, `postio-render`, `postio-host`) out of
`crates/postio-bench/Cargo.toml` for the run only and put them back; nothing
about that is committed. `search_focus.rs` uses none of them. A Mac that will
run this often wants the bench's GTK benches behind a target-specific
dependency table, or `brew install pkgconf gtk4 libadwaita`.

Not measured: a cold cache (the store is in memory and warm), a store on
disk, and anything past 20,000 messages.

## Shared names the engine changed that `postio-gtk` uses (T037)

`postio-gtk` cannot be compiled on this Mac, so this is a grep, not a build;
Linux CI is the proof. The changed shared surface of `postio-search` and
`postio-index` on this branch, checked against `crates/postio-gtk`:

- `Filter`, `Field`: gained variants (`Label`, `HasAction`, `Account`,
  `Group`, `Header`); none removed or renamed. `postio-gtk` names neither:
  the `Field` hits in it are `postio_widgets::composer::Field` and the
  pickers' `Field`.
- `filter_condition`: internal to the executor; `postio-gtk` does not call it.
- `natural::lower`: signature unchanged (`text`, `today`, `names` ->
  `ParsedQuery`); now a thin wrapper over the new `lower_with_origins`. Its
  one caller in the tree is `crates/postio-gtk/src/bar.rs:826`.
- `ParsedQuery`, `ResultOrder`, `SearchHit`, `Instead`, `facets::Scope`,
  `SearchRequest`, `search`, `parse`, `index::{ensure_schema, index_body,
  messages_missing_body_text_for_account}`: all additive changes only (the
  diff removes no public item of `postio-search`, `postio-index`'s public
  surface). Used from `bar.rs` and the `focus_suite` cases `idle_passes`,
  `support`, `settings_wiring`.
- `ParsedQuery::text_terms` is untouched; `searchable_terms` was added beside it.

Nothing in `postio-gtk` needs to change for the engine work through step 1.
