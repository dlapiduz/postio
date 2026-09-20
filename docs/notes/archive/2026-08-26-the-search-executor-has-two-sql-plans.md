# The search executor has two SQL plans, and which one a statement gets is not a preference

*Archived 2026-09-14: the plans, the `MATCH` mechanics and every timing here are FTS5's; the engine is Turso since ADR 0038 and the index is a `USING fts` index queried through `fts_match`/`fts_score`. The two-plan split itself survives as two thresholds in `crates/postio-index/src/executor.rs` (`RANK_BY_RELEVANCE_LIMIT`, `PROBED_FORM_LIMIT`), measured again on the new engine in the constants' own comments.*

#408, and every number here was measured against the 120,000-
message `search_budget` bench.

- A query narrow enough to rank is **driven by the match**: walk the postings
  of both FTS indexes, look each hit up in `messages` by primary key.
- One too broad to rank is **driven by `messages`**, ordered by its own
  `(account_id, received_at)` index, asking each row "did you match?" through a
  correlated `EXISTS` with `rowid = m.id AND … MATCH ?`. That shape is what
  made FTS5 answer with a docid seek — the plan said
  `VIRTUAL TABLE INDEX 0:=M5`, and the `=` is the rowid.

Getting it wrong is expensive in both directions, and every wrong turn was
tried: letting SQLite choose cost **49 ms** on a word matching 1% of the
corpus (it drove from `messages` and probed a co-routine it could not size); a
`GROUP BY` over the union cost **297 ms** on a common word (an aggregate must
materialise every match before anything runs); probing the union per row cost
**570 ms**; and `count` driven by `messages` cost **2.8 s** on a rare word.

Two consequences worth knowing before editing that file:

- **Adding a column to the candidate-pool statement can lose its plan.** The
  file already recorded this for the hydrate columns; it is equally true of
  correlated subqueries in the select list, which is why the broad path
  carries no `bm25` at all. That is deliberate rather than missing — the path
  is only taken when the match is too wide to rank, where bm25 is near-uniform
  and recency is the intended fallback.
- **`hydrate` touches no FTS table.** The scores ride out with the candidate
  pool. Re-asking the indexes for the scores of ids you already have is the
  297 ms mistake wearing a different hat.
