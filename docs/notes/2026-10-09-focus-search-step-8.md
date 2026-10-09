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
