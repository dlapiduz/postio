# 0037 — A misspelling is answered with a suggestion, not a wider match

- **Status:** Accepted
- **Date:** 2026-09-12
- **Issue:** #1524

## Context

Searching `hanah` finds nothing, though the mailbox holds mail from `Hannah`.
The index matches whole words — the engine's tokenizer folds case only, and
`postio_model::fold` folds diacritics on both the indexed text and the query
(ADR 0038) — so a dropped letter is simply a different word.

The obvious fix is to make matching tolerant — a trigram tokenizer, or edit
distance in the match path. The tree says otherwise.

## Decision

**The query language is never widened for approximate matching.** A query
means exactly what it says. A search that finds nothing may *offer a
different query*, which the user accepts deliberately.

## Why

A query string here is not only a way to draw a list. `docs/PRODUCT.md`: a
saved search is a query with a name; a virtual folder is a saved search that
is pinned; **a rule is a saved search plus actions**. ADR 0008 makes the query
language the only way to say which messages a rule acts on.

So a looser match does not merely make search friendlier. It makes **rules
archive, label and move mail on terms that are only approximately present**,
without anybody watching. A search that finds nothing is visible in the same
second it happens; a rule that fired on a near-miss is discovered later, if at
all, and it has already moved the mail.

Constitution III requires the same string to mean the same thing typed in the
box, saved to the sidebar, and written into `config.toml`. Widening keeps that
sentence literally true while destroying what it is for.

## Rejected

**A trigram tokenizer.** Reindexes the whole corpus — which since ADR 0016 is
the entire mailbox — changes `bm25` ranking for every existing query, and
grows the index. All of that would be arguable on its own; none of it answers
the rules problem, which remains exactly as bad.

**Edit distance in the match path.** The rules problem without the reindex.

**An explicit operator, `~hanah`.** Only helps somebody who already knows they
misspelled it, which is the one case that does not need help.

## Consequences

- Matching is unchanged, so saved searches, virtual folders and rules are unaffected by this feature existing. That is the point.
- A suggestion is a query the user could have typed. Accepting it rewrites the box, and nothing downstream can tell the difference between an accepted suggestion and the same thing typed by hand.
- The vocabulary is rebuilt, on a search that found nothing, by scanning the newest `VOCABULARY_DOCUMENTS` (5,000) rows of `search_documents` and keeping the commonest `VOCABULARY_CAP` (4,096) terms (`crates/postio-index/src/executor.rs`) — metadata columns only, never body terms. It needs no schema change. *(As first written this read "FTS5's `fts5vocab`"; the engine keeps no term dictionary to read — ADR 0038.)*
- The ranking is arithmetic over two strings and a count, so it lives in `postio-search` — the leaf `check-crate-boundaries.py` keeps free of the engine (`rusqlite`, `turso`, `turso_core`) — and is tested without a database. Reading the vocabulary is `postio-index`'s half.
- It runs only when a query returned nothing, never on the typing path.
- Nothing leaves the machine: the vocabulary is the local index.

## What this does not decide

Whether a suggestion is ever applied *automatically*. It is not, today, and
the reasoning above is why: an automatic correction is a query the user did
not type, which is the same objection in a smaller hat.
