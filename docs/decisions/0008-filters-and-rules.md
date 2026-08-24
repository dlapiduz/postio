# ADR 0008 — Filters and rules: one language, two evaluators

- **Status:** Accepted — **GO** (2026-08-24)
- **Date:** 2026-08-24
- **Issue:** [#5 Filters and rules engine](https://github.com/dlapiduz/postio/issues/5)
- **Related:** `docs/ARCHITECTURE.md` §6 (one matching language), §4 (selection
  is a predicate), [ADR 0005](0005-multiple-accounts.md)
- **Decision:** the query language stays the only way to say *which messages*,
  and gains a **second evaluator** — an in-memory matcher in `postio-search`
  beside the FTS5 executor in `postio-index`, held to agreement by a
  differential test. Rules live in a new **ordered `[[rules]]` array**, not in
  `[filters]`, because a map has no order and the issue requires one. A rule
  fires **when every fact it needs exists**, which is not always on arrival.

---

## What is already built

| Piece | State |
|---|---|
| Query parser, `Field`, `Filter`, `Clause`, `ParsedQuery` | Built (`postio-search`) |
| Negation with a leading `-`, on operators and free text | Built |
| FTS5 execution of a parsed query | Built (`postio-index/src/executor.rs`) |
| `[filters.<name>] { query, pinned }` | Built (`config/src/filters.rs`), **no runtime reads it** |
| `postio-config` deliberately keeping the query as *text* | Built, and the idiom this ADR extends |
| Sidebar rendering of pinned filters | Absent |
| Any rules engine | Absent |
| `OR` in the query language | **Absent** — tokens are implicitly ANDed |
| `body:` and `header:` operators | Absent |

---

## Q1 — Where does a rule get evaluated?

This is the question the issue does not ask and everything else depends on.

`postio-index` executes a `ParsedQuery` by compiling it to SQL and an FTS5
`MATCH`. A rule on arrival has no row to run SQL against — the sync pass is
holding a `Message` it has just parsed and is deciding what to do with it
before it is committed anywhere a query could see.

So there are two evaluators, and this is a decision rather than an accident:

| | Runs | Input | Lives in |
|---|---|---|---|
| Executor | search bar, sidebar filters, dry-run | the database | `postio-index` |
| **Matcher** | rules, on arrival | one `Message` in memory | **`postio-search`** |

`postio-search` is pure — `postio-model` and `chrono`, no SQL, no toolkit — and
a matcher is exactly that shape: `ParsedQuery` in, `&Message` in, `bool` out.
Putting it there keeps `postio-sync` free of the query language's internals
and makes the matcher unit-testable against the `.eml` corpus with no database
at all.

**The load-bearing test is a differential one.** Two evaluators of one language
that disagree is the worst outcome available here — worse than not having
rules — because a dry-run would show one answer and the rule would do another.
So: index the whole corpus, run every query in a fixture list through both
paths, and assert the result sets are identical. That test is the reason this
design is safe, and it is the first thing to write.

---

## Q2 — The language has to grow, and the growth is small

Rules need conditions the search bar does not have yet. Each one is added to
*the* language, never to a rules-only dialect.

| Needed | Today | Add |
|---|---|---|
| `from`, `to`, `subject`, `list`, `has:attach`, `is:`, size, date | present | — |
| **`header:`** arbitrary header match | absent | `header:x-mailer=…`, one `Field` row |
| **`body:`** | absent | one `Field` row; see Q3 for what it costs |
| **`or`** | **absent** — tokens are implicitly ANDed | see below |
| `not` | present as `-` | — |

**`OR` is the one that is not one row.** Today `ParsedQuery` is a flat
`Vec<Token>` conjoined implicitly, and `Clause` carries a `negated` flag. That
is the right shape for a search bar rendering chips, and it cannot express
`from:ada OR from:grace`.

Decision: **add `OR` as an explicit infix keyword with `AND` binding tighter,
and parentheses for grouping** — `from:ada OR (from:grace has:attach)`. The
flat token vector stays the *lexical* form (the chips do not change), and
`ParsedQuery` gains a derived boolean tree that both evaluators consume. A
query with no `OR` produces a tree identical in meaning to today's conjunction,
so nothing that works now changes.

This is worth doing carefully and worth doing once. `ARCHITECTURE.md` §6's
whole claim is that the same string means the same thing in the search bar, the
sidebar and `config.toml`; an `OR` that existed only for rules would end that.

---

## Q3 — "Applied on arrival" is not always possible, and pretending it is would lose mail

Postio syncs headers newest-first and backfills bodies lazily —
`BodyState` exists precisely because a message is listed, threaded and
header-searchable long before its body is local. A rule containing `body:`
therefore *cannot* be evaluated when the message arrives.

The tempting answers are both wrong. Fetching the body eagerly for rule
evaluation throws away the backfill design and makes first sync slow on a large
mailbox. Evaluating the rule anyway, against an absent body, silently makes
`body:invoice` false and files mail in the wrong place.

**Decision: each rule declares nothing; the engine derives what it needs.**
A parsed query's fact requirement is computable from its fields —
`HEADERS_ONLY` or `NEEDS_BODY`. Then:

- Header-only rules run in the sync pass that inserts the message, in the same
  transaction as the insert, before any event is emitted. The user never sees
  it land in the Inbox first.
- Body-requiring rules run on the backfill completion for that message, through
  the same executor path. The message is in the Inbox in between, which is
  honest — it *is* in the Inbox until Postio knows enough to move it.
- **The config validator says so.** A rule containing `body:` gets a validation
  note — "runs after the body is fetched, not on arrival" — surfaced in the
  settings panel's validity line, which `postio-config` already has for
  `rejected_secrets`.

---

## Q4 — Rules are an ordered array; `[filters]` stays what it is

`[filters]` is a map, `HashMap<String, FilterConfig>`. A map has no order.
Issue #5 requires that "rule evaluation order is deterministic and documented",
and the only way to get order out of a map is an `order = 3` field on every
entry — which users duplicate, skip, and have to renumber to insert a rule in
the middle.

**Decision: rules are `[[rules]]`, a TOML array of tables. The file *is* the
order.**

```toml
[[rules]]
name    = "receipts"
query   = "from:billing has:attach"
actions = ["move:Receipts", "mark-read"]
stop    = true

[[rules]]
name   = "needs-reply"
filter = "needs-reply"       # reuse a named [filters] query
actions = ["flag"]
enabled = false              # dry-run it first
```

And **`[filters]` keeps its existing job**: named saved queries, `pinned = true`
putting one in the sidebar. That is a *view*, not a rule, and conflating them
would mean every sidebar shortcut had to think about actions and ordering.
A rule may name a filter (`filter = "needs-reply"`) so a query the user already
tuned is not written twice.

This supersedes the fourth row of `ARCHITECTURE.md` §6's table, which reads
"a filter / rule — a saved search plus actions". The relationship is right; the
spelling is `[[rules]]` referencing `[filters]`, not `[filters]` growing an
`actions` key.

**Config keeps everything as text.** `postio-config` does not parse the query
today, on purpose, and it does not parse the action either — `"move:Receipts"`
is a string to it. `postio-model::rule` parses both into typed forms. That is
what keeps `postio-config`'s dependency list at four crates and none of them
domain.

**Short-circuit is explicit per rule, and the default is `false`.** Every
matching rule runs unless one sets `stop = true`. A `stop`-by-default engine
makes "add a label to everything from this list" silently disable everything
below it.

---

## Q5 — Actions, and the two that need constraining

```
move:<mailbox>   label:<name>   flag   unflag   mark-read   mark-unread
archive          trash          forward:<address>          stop
```

- **`trash`, never `delete`.** A rule moves to the Trash mailbox; permanent
  removal is not available to a rule. Issue #5's fourth criterion — a rule that
  errors never silently drops mail — is much easier to hold when no rule can
  destroy anything in the first place.
- **`forward` is the one that leaves the machine**, and
  `ARCHITECTURE.md` §11's test is "did the user ask for it". Writing the rule is
  asking, so it ships — with three guards, because an auto-forward rule is also
  the classic exfiltration primitive and a mail loop generator:
  - it never forwards a message that a rule already forwarded (a Postio-set
    header, checked on arrival);
  - it refuses a target that is an address of any configured account;
  - it is rate-capped per hour, and hitting the cap raises `Attention` rather
    than dropping the mail.
  A forwarded message appears in Sent like any other, because the send goes
  through the ordinary operation queue. It is not invisible.

**Every action is local-first, exactly like a keystroke** (`ARCHITECTURE.md`
§1): SQLite write, enqueue the remote operation, emit the event. There is no
rules-only mutation path, which means rules inherit offline behaviour,
reconciliation and event flow for free.

---

## Q6 — Errors never drop mail

A rule can fail: a `move:` naming a mailbox that no longer exists, a malformed
query that survived validation, a `forward` with no network.

**Decision, per message and per rule:**

1. The failure is recorded against the rule, and the *message is left exactly
   where it was*. There is no partial application: the actions for one rule on
   one message run in one transaction with the message's insert.
2. The rule is suspended for the remainder of the pass, so one broken rule does
   not log ten thousand times.
3. The account raises `Attention` with the rule's name — the state
   `postio-sync` already has, and which the UI already renders.
4. Processing continues with the next rule. A broken rule three deep does not
   stop the two below it.

---

## Q7 — Dry-run, and re-running over an existing mailbox

Both fall out of Q1, which is the payoff for having one language.

- **Dry-run is the executor.** Running the rule's query through `postio-index`
  over existing mail is exactly "what would this match", ranked and paged like
  any search. The settings panel shows the count and the first page. No new
  machinery.
- **Manual application** is a command in the registry — so it has a key, a
  palette entry and a cheat-sheet line, per §2 — that runs the executor to get
  the set and then applies the actions to it.
- **It is one undo unit.** `Selection::Everything { except }` means applying a
  rule to a 100k mailbox is a predicate the store resolves, not 100k ids the
  frontend built (§4), and the `UndoStack` coalescing that already makes twelve
  archives one `u` makes this one `u` too. A bulk rule application that could
  not be undone would be the most destructive command in the application.

---

## Alternatives

**A separate rule condition syntax.** Sieve is the obvious candidate and is a
real standard. Rejected for the reason §6 already gives: two parsers, two sets
of operator semantics, and a rule that does not agree with the search bar about
what `from:team` means. Postio's users would have to learn a second language to
do the thing the first one already describes.

**One evaluator: insert first, then run the query.** Attractive — no matcher,
no differential test — and it means every message lands in the Inbox and is
then moved, so the user watches their mail get filed after they have already
seen it. It also makes rule application a second write of every row on every
sync pass.

**`[filters]` grows `actions` and `order`.** The literal reading of §6.
Rejected in Q4: hand-numbered ordering in a map is a bad configuration surface,
and views and rules want different fields.

**Skip `OR`.** Cheapest, and it makes the first real rule anyone writes
impossible: "from any of these three people".

---

## Consequences

- `postio-search` gains a boolean tree, `OR`, `header:`, `body:`, and a
  matcher. It stays pure.
- `postio-model` gains `rule` — the typed action vocabulary.
- `postio-config` gains `[[rules]]` and one validation note; its dependency
  list does not change.
- `postio-sync` gains a rules pass in `initial` and `resync`, beside the
  contacts pass that already runs there and for the same reason.
- `ARCHITECTURE.md` §6's table row for filters/rules needs updating to
  `[[rules]]`.
- The differential test between the two evaluators is the gate on all of it,
  and belongs in CI from the first commit rather than at the end.
