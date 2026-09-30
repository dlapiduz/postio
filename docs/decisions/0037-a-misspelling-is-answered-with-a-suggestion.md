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
- The candidates come from the index's own term dictionary (2026-09-24). Turso is built from a fork, `dlapiduz/turso` (the root `Cargo.toml`'s `[patch]`): v0.8.0-pre.11 plus the unmerged turso#8359, which expands an unquoted `word*` or `word~N` against the dictionary, and a per-word `~N` so one query can ask for it on an index that otherwise stays exact. The offer runs `postio_search::suggest::widened` through both halves of the index, bodies included, reads at most 50 documents of each, and recovers the word from their text. *(Before this it was a vocabulary rebuilt from the newest 5,000 senders and subjects: a list older than that, or a word only a body held, was never offered.)*
- An unfinished word is offered the word it begins, ahead of any correction: whole-word matching finds nothing for half a word, and three letters are a beginning though too few to correct.
- The ranking is arithmetic over two strings and a count, so it lives in `postio-search` — the leaf `check-crate-boundaries.py` keeps free of the engine (`rusqlite`, `turso`, `turso_core`) — and is tested without a database. Reading the vocabulary is `postio-index`'s half.
- It runs only when a query returned nothing, never on the typing path.
- Nothing leaves the machine: the vocabulary is the local index.

## Amended 2026-09-24: the search box applies it

The maintainer asked for partial and misspelled words to find mail. Offered
three ways -- widen the box only, widen everything including rules, or show a
rewritten search -- they chose the last, and it keeps the decision above
intact: **the query language is still never widened.**

- In the search box, a single bare word that found nothing is answered with
  the offered word, when that word finds mail in the same scope
  (`postio_session::search::execute`, the search both frontends run). The
  executor, which also answers saved searches, virtual folders and rules, is
  untouched: those still match exactly what they say.
- The box keeps what was typed. It searches as it is typed, so replacing the
  text would hijack every pause in the middle of a name; instead the column
  says **"Showing results for <word>"** (`SearchResults::instead`), so the
  list is never silently about a word the box does not show.
- The typed word is one press away, **quoted**. Quotes are how the query
  language says "this word, exactly" (`TextTerm::quoted`), and a quoted word
  is never offered or rewritten. So the objection above -- a query the user
  did not type -- is answered by saying so on screen and by a way out that
  is itself an ordinary query.
- Saving a search from the box is not wired yet (`CommandId::SaveSearch` has
  no handler). When it is, a rewritten search saves the word it showed, not
  the one typed: what is saved is what was seen.
