# ADR 0008 — Filters and rules: one language, two evaluators

- **Status:** Accepted (2026-08-24). Built on main: saved searches, `header:`, and the in-memory matcher the Focus digest rules use. Not built on main: `[[rules]]`, `postio-model::rule` and the sync rules pass (branch `feature/rules`; see ADRs 0028 and 0030).
- **Decision:** the query language stays the only way to say which messages. A second evaluator, an in-memory matcher in `postio-search`, sits beside the SQL executor in `postio-index`, held equal by a differential test. Rules are an ordered `[[rules]]` array, and a rule fires when every fact it needs exists.

## Rules
- The matcher takes a `ParsedQuery` and a `&Message` and returns `bool`. `postio-search` stays pure (no SQL, no toolkit).
- The differential test (`postio-index` `digest_matcher`) runs every fixture query over the `.eml` corpus through both evaluators and asserts identical result sets. An executor bug is repeated in the matcher on purpose, and a fix changes both in one commit.
- New conditions extend the one language, never a rules-only dialect. `OR` is an infix keyword, `AND` binds tighter, parentheses group; `ParsedQuery` gains a derived boolean tree, and the flat token vector stays the lexical form. `body:` is one `Field` row.
- A query's fact requirement is `HEADERS_ONLY` or `NEEDS_BODY`, derived from its fields. Header-only rules run in the sync pass that inserts the message, in the same transaction, before any event. Body rules run on backfill completion. The validator notes "runs after the body is fetched, not on arrival". A rule stages as a whole, at the later of what its query and actions need (ADR 0030).
- Rules are `[[rules]]` (`name`, `query` or `filter`, `actions`, `stop`, `enabled`); file order is evaluation order. `[saved_searches]` stays a map of views. `postio-config` keeps query and actions as text; `postio-model::rule` parses them. `stop` defaults to `false`.
- Actions: `move:<mailbox>`, `label:<name>`, `flag`, `unflag`, `mark-read`, `mark-unread`, `archive`, `trash`, `forward:<address>`, `stop`. A rule never deletes permanently.
- `forward` target is a literal, never interpolated from the message. It never re-forwards a message already forwarded, refuses an address of any configured account, and is rate-capped per hour; the cap raises `Attention` instead of dropping mail. It goes through the ordinary operation queue and shows in Sent.
- Every action is local-first and uses the same implementation as the keystroke (ADR 0028).
- A failing rule leaves the message where it is, is suspended for the pass, raises `Attention` naming the rule, and the next rule still runs. One rule's actions on one message are one transaction with its insert.
- Dry-run is the executor over existing mail. Manual application is a registry command that applies actions to a `Selection::Everything` predicate and is one undo unit.

## Why
- A rule on arrival has no row to query, so the matcher is a second evaluator; two evaluators that disagree would make a dry-run lie.
- Eager body fetch discards the backfill design; evaluating against an absent body files mail wrongly.
- A map has no order. Sieve means two parsers. "Insert then query" shows mail landing before it is filed.

## Revisit when
- `feature/rules` merges: update the Status line.
