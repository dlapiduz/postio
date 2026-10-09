# A score that is zero and says nothing

2026-09-12, `feature/turso-store`.

## The finding

Turso's `fts_score` returns a real score under two conditions, and `0.0` — not
an error, not `NULL` — under every other:

1. **The call must be the whole select-list expression.** `fts_score(c, ?1)`
   scores; `fts_score(c, ?1) AS s` scores; `-fts_score(c, ?1)` and
   `0 - fts_score(c, ?1)` and `1.0 * fts_score(c, ?1)` all answer `0.0`.
2. **Its query term must be the same *expression* as the `fts_match` that
   selected the row.** Not the same value: `fts_match(c, ?2)` with
   `fts_score(c, ?1)`, both parameters bound to `"report"`, answers `0.0`.

Both are pinned in `crates/postio-storage/tests/turso_capabilities.rs`:
`the_score_is_lost_to_any_arithmetic_around_it` and
`the_score_needs_the_same_parameter_as_the_match`.

## Why it is worth a note

Because nothing fails. The same rows come back, in the same set, with the same
count. Only the *ranking* silently flattens to a constant.

Postio's search executor hit both at once. It wrote
`-fts_score(sender, recipients, subject, filenames, list_id, ?)` because
`bm25()` is negative-is-better and `rank_score` and every comment around it
assume that convention — and it bound four separate parameters, two for the
scores and two for the matches. So every candidate scored `0.0`,
`rank_score(0.0, received_at, now, affinity)` reduced to `-3.0 × recency -
affinity`, and **search answered every query in date order**.

Which is a plausible-looking result. Search returned the right messages; the
newest ones were first; nothing logged anything. One test in the workspace
could tell the difference — `newest_order_answers_in_date_order_however_the_
ranking_disagrees`, which builds a corpus where a saturated older match must
beat a glancing newer one — and it is the only reason this was found before
somebody noticed their search had stopped being useful.

## The rule this leaves

**Project `fts_score` bare and do the arithmetic outside**, where the alias is
an ordinary column: `SELECT -h.s FROM (SELECT fts_score(c, ?1) AS s …) h`. And
**write the term as an explicit `?N` and use it in both the score and the
match**. A bare `?` after an explicit `?N` continues from `N + 1` here exactly
as it does in SQLite, so the rest of a statement still numbers itself — but a
bare `?` written *before* the explicit ones collides, which is how
`flag_refinements` came to bind five parameters into three slots.

## The wider lesson

A ranking function is the one part of a search that has no failing state. It
cannot 404 and it cannot throw; it can only be wrong in an order nobody
measures. So the test that catches it has to construct a case where the
correct order is not the obvious order — an older, better match — and that
test is worth its cost on any engine.
