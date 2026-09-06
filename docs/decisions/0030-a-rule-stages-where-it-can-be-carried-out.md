# ADR 0030 — A rule stages where it can be answered *and* carried out

- **Status:** Accepted (2026-09-06)
- **Date:** 2026-09-06
- **Decision by:** `/ux-architect`, on [#1142](https://github.com/dlapiduz/postio/issues/1142), which was filed `needs-architecture` because #481 could land every action except `forward:` without it.
- **Issue:** [#1142](https://github.com/dlapiduz/postio/issues/1142)
- **Amends:** [ADR 0008](0008-filters-and-rules.md) Q3, whose `Stage` is derived from the query alone, and Q5, which lists `forward:<address>` without saying when it runs.
- **Related:** [ADR 0016](0016-full-mailbox-backfill-by-default.md) (there is no backfill horizon), [ADR 0028](0028-a-rule-runs-the-same-verb-a-keystroke-does.md) (a rule runs the same verb a keystroke does), [#481](https://github.com/dlapiduz/postio/issues/481) (the action vocabulary), [#1118](https://github.com/dlapiduz/postio/issues/1118) (a message says which rule touched it)
- **Decision:** **a rule's stage is the earliest point at which it can be both answered and carried out** — the later of what its query needs and what its actions need. `forward:` needs a body to carry out, so a rule carrying `forward:` is `OnBody` whatever its query says. **The forward never fetches its own body**: it waits for the backfill lane that was going to fetch it anyway.

---

## The question

`RuleSet::compile` derives `Stage` from the query and nothing else
(`postio-search/src/rules.rs`):

```rust
let stage = if needs_body(&query) { Stage::OnBody } else { Stage::OnArrival };
```

So `query = "from:lists@example.com"`, `actions = ["forward:ada@example.com"]`
stages `OnArrival`, and the arrival pass deliberately holds no body. Forwarding
there would send headers with nothing under them — the one failure mode ADR
0008 Q3 was written to prevent, arriving through the half of the rule that Q3
did not look at.

#1142 posed it as two costs to weigh: `OnBody` means "a message the user never
opens may never be forwarded", and fetching means "a fetch per matching
message, which is the thing ADR 0016 exists to avoid". **The first of those
costs does not exist**, and that is what decides it.

## Q1 — What `Stage` means

ADR 0008 Q3 introduced `Stage` to answer *when can this rule be evaluated*.
That was the whole question while every action was a local mutation on a row
that was already there: `move:`, `label:`, `flag`, `archive` need the message
and nothing else, so a rule that could be answered could be carried out in the
same breath.

`forward:` is the first action with a requirement of its own, and it makes the
distinction visible rather than creating it.

**Decision: `Stage` is the earliest point at which the rule can be answered
*and* carried out** — the later of the two requirements:

```
stage(rule) = max( needs_body(query), needs_body_to_run(actions) )
```

with `OnArrival < OnBody`. Today `needs_body_to_run` is true for exactly one
action, `forward:`. It is a function over the action vocabulary rather than a
special case in `compile`, so the next action with a requirement is a match
arm and not a rediscovery of this ADR.

The module's own framing survives unchanged, and is worth restating because it
is what makes this cheap: each rule declares nothing, and the engine derives
what it needs.

## Q2 — A rule stages as a whole; its actions never split across the two points

The alternative shape is tempting and wrong: run `move:` on arrival, hold
`forward:` back until the body lands.

Two things break.

- **`stop` would mean two things for one rule.** ADR 0008 Q4 and #481 make
  `stop` halt the rules below it *on that pass*, and the two passes are
  deliberately independent — "a `stop` among the header rules says nothing
  about the body rules". A rule that exists in both passes is in the middle of
  two different orderings at once, and its position in each says something
  different about which rules run.
- **The forward would fire from a folder the rule had already emptied.** The
  same rule's `move:` runs on arrival; the message is in the target mailbox
  by the time the body lands. Every action of a rule reading the message in a
  different state than the one it matched is a category of bug with no bottom.

ADR 0008 Q6 already makes this the rule for failure — *"there is no partial
application: the actions for one rule on one message run in one transaction"*.
This is the same sentence applied to timing rather than to errors.

So a `forward:` rule's `move:` and `label:` move to the body point with it.
The visible cost is the honest one Q3 already accepted: the message is in the
Inbox, unmoved and unforwarded, until its body lands.

## Q3 — Why waiting is not "may never be forwarded"

The objection assumed a backfill that fetches on demand. That is not what
Postio has.

- **[ADR 0016](0016-full-mailbox-backfill-by-default.md): there is no
  horizon.** "Every selectable folder backfills to completion by default —
  every message's body, eventually, in the background." Nobody has to open
  anything. A body arrives because Postio decided to fetch it, not because a
  user asked.
- **The wait is short, by construction.** The backlog is a max-heap on
  delivery time — newest first (`postio-sync/src/backfill.rs`, "Newest
  first"). A message that has just arrived is at the *front* of the background
  lane. `OnBody` for a new arrival means "after the body already on the wire",
  not "after the forty-thousand-message backlog".

So the shape a forwarding rule actually has is: mail arrives, its body is the
next thing the backfill was going to fetch anyway, and the forward goes out
behind it. The rule is late by one body, not by a user's habits.

## Q4 — What is rejected: the forward pulling its own body

Postio has an interactive lane that would do this, and it must not be used
here. Its definition is the reason (`backfill.rs`, module docs):

> **the interactive lane always wins.** It ignores the size cap, ignores a
> metered connection, ignores the pause switches and jumps whatever the
> backlog holds. […] A message being opened is not speculative work, and it is
> the only fetch anybody is watching a spinner for.

A rule is not a person. Letting an unattended background policy jump ahead of
the user's own click inverts the single rule that lane has, and it does so
invisibly — the user would experience it as their mail client having become
slower to open messages, with nothing on screen connecting that to a rule they
wrote weeks ago. It would also ignore the metered-connection and disk-ceiling
switches on the user's behalf, for mail they are not reading.

A third path — a rules-only fetch beside both lanes — is refused by ADR 0028
and by ADR 0008 Q5's own "there is no rules-only mutation path", extended to
reads for the same reason: two implementations of "get this body" is two
policies about cost, and the second one is always the one nobody remembered
to teach about metered connections.

The correct reading of "a fetch per matching message" is that there is **no
extra fetch at all**. The body was already going to be fetched. Staging is
about ordering with respect to work already scheduled.

## Q5 — Where the wait is said out loud

ADR 0008 Q3 already built the surface for this, for `body:` rules:

> **The config validator says so.** A rule containing `body:` gets a validation
> note — "runs after the body is fetched, not on arrival" — surfaced in the
> settings panel's validity line.

`forward:` gets the same note, from the same mechanism, for the same reason.
The user writing the rule is told when it will run, at the moment they write
it, rather than discovering it from a timestamp later.

**The one case where the wait is unbounded gets its own sentence.** ADR 0016
lets a user exclude a folder from backfill, and `seed` "queues nothing at all"
for one. A body-staged rule over an excluded folder never runs — silently, and
"nothing is a dead end" forbids that. So: **when any folder is excluded from
backfill, a rule staged `OnBody` says so in its settings row**, naming the
folders, beside the target it already shows in full (ADR 0028 Q2). Static,
computed at config load, no per-message machinery. It is not resolved
silently in either direction: Postio does not quietly re-enable backfill for
the folder, and it does not quietly forward a bodyless message.

## Consequences

- `Stage` derivation reads the actions as well as the query. One function over
  the action vocabulary, `needs_body_to_run`, true for `forward:` alone today.
- A `forward:` rule's every action runs at the body point, in one transaction,
  as ADR 0008 Q6 already requires.
- No new fetch path, no change to either backfill lane, no change to
  `Stage`'s two values.
- The settings panel's rule row gains the Q3 note for `forward:`, and the
  excluded-folder warning from Q5.
- ADR 0008 Q5's three `forward:` guards are untouched by this and remain as
  ADR 0028 restated them. Guard one — never forward a message a rule already
  forwarded — is checked against a Postio-set header, and moving the forward
  to the body point does not move that check: the header is on the message
  either way.
- A rule that carries `forward:` **and** `body:` was already `OnBody` and is
  unaffected, which is the sanity check that this rule composes.
