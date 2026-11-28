# ADR 0037 — A misspelling is answered with a suggestion, not a wider match

- **Status:** Accepted (2026-10-08)
- **Decision:** The query language is never widened for approximate matching. A query means exactly what it says. A search that finds nothing may offer a different query, and interactive search may forgive; saved searches, virtual folders and rules never do.

## Rules
- A rule is a saved search plus actions (ADR 0008), so a looser match would make rules archive, label and move mail on near-misses unseen. The same string means the same thing typed, saved and in `config.toml` (constitution III).
- Search box: a single bare word that found nothing is answered with an offered word, only when that word finds mail in the same scope (`postio_session::search::execute`). The box keeps what was typed. The column says "Showing results for <word>" (`SearchResults::instead`). The typed word is one press away, quoted. A quoted word is never offered or rewritten. The offer runs only on an empty result, never on the typing path.
- Command bar: `ParsedQuery::forgiving()` is a flag on the parsed value, not syntax, so saved searches, folders and rules cannot be forgiving, and saving from the bar saves the words. A forgiving search runs exact first. If the page is not full it reruns with each unquoted positive word read as any of its near words (`postio_search::suggest::near`: itself, words it begins, misspellings of it, at most eight). Those hits come after every exact one. The second query is a disjunction of quoted literals, because the fork expands `word~N` only in a query of bare words.
- Conversation search (spec 010) forgives below `FORGIVE_BELOW` (50) exact conversations, whatever page was asked, so the dropdown and results count one list. Exact matches rank before near ones. The Files and People tabs walk the same widened match.
- An unfinished word is offered the word it begins, ahead of any correction.
- Candidates come from the index's term dictionary through `postio_index` (`words_near`), over both halves including bodies, reading at most 50 documents of each. The fork `dlapiduz/turso` provides this (root `Cargo.toml` `[patch]`, turso#8359). Ranking is pure arithmetic in `postio-search`, the leaf kept free of engine crates (`check-crate-boundaries.py`), tested without a database.
- A suggestion is a query the user could have typed. Nothing downstream distinguishes an accepted suggestion from one typed by hand.
- Saving from the box saves the word it showed, not the one typed (`CommandId::SaveSearch` has no handler yet).
- Nothing leaves the machine.

## Why
- A search that finds nothing is visible at once. A rule that fired on a near-miss is found later, after moving the mail.
- Rejected: a trigram tokenizer (reindexes the whole mailbox, changes bm25, does not answer the rules problem), edit distance in the match path, an explicit `~word` operator (helps only those who know).

## Revisit when
- The turso fork's term-dictionary expansion lands upstream.
