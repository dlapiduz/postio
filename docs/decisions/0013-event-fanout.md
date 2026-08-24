# ADR 0013 — Event fan-out: a hub between producers and subscribers

- **Status:** Accepted — **GO** (2026-08-24)
- **Date:** 2026-08-24
- **Issue:** [#149 One EventStream cannot serve a window and an MCP server at
  once](https://github.com/dlapiduz/postio/issues/149)
- **Related:** [ADR 0002](0002-extensible-command-vocabulary.md) (whose
  implemented correlation ids end on exactly this open constraint),
  [ADR 0010](0010-mcp-surface.md) (which requires the configuration that
  constraint forbids), [#82](https://github.com/dlapiduz/postio/issues/82)
  (postio-session), [#14](https://github.com/dlapiduz/postio/issues/14) (MCP),
  [#137](https://github.com/dlapiduz/postio/issues/137) (extensions inherit
  whatever this decides)
- **Decision:** one **event hub**, owned by the composition root. Every
  producer's `EventSink` feeds it; every consumer **subscribes**, by name, and
  gets a private `EventStream` with exactly today's API. Subscriptions are
  **unbounded, like both of today's queues**, with a per-subscriber depth
  watermark that warns by label; disconnect-on-overflow is the recorded
  fallback, not the shipped behaviour. Every subscriber sees every envelope —
  scoping happens at each consumer's own trust boundary, never in the hub.
  **Events are notifications, not a journal**: a subscriber joins at *now* and
  reads the store for the past.

---

## The two ADRs pointing at each other

ADR 0002's implemented correlation-id work ends on an explicit constraint:

> there is still exactly one `EventStream`, so a tracked caller and the window
> cannot both read it. That holds while the only tracked callers are tests and
> a future headless consumer, and it is the first thing to settle when an MCP
> server wants to sit beside a running window.

ADR 0010 then decides MCP **is** "a second frontend over `postio-core`'s
bridge" beside the running window — the configuration the constraint forbids —
and never mentions fan-out. Neither ADR owns the decision. This one does.

## What is already built

Measured at `7010833`. The single-consumer property is not one fact but three,
and the third is the one the issue's framing missed:

| Piece | State |
|---|---|
| `EventSink` is `Clone`, many producers | Built — a spawned task keeps its handler's sink and origin |
| `EventStream` is deliberately **not** `Clone` | Built — `async_channel`'s receiver is work-stealing: a cloned receiver *steals* events, it does not duplicate them. Not-`Clone` is what makes delivery total |
| `EventEnvelope { event, origin }`, `send_tracked`, `InvocationFinished` | Built (ADR 0002, #33) |
| **There are already two event queues, not one** | Built — the bus's channel (made inside `BridgeBuilder::build`) and the engine's (`event_channel()` in `postio-app/src/lib.rs:153`), each with one reader |
| The window drains both | Built — `commands::drain` is called once per stream, over a `Rc<RefCell<Vec<Option<EventStream>>>>` handoff shared with onboarding |

So today's shape is *N producers, two channels, one reader each* — and the
reader is always the window. The `Vec<Option<EventStream>>` dance exists
because producers each brought their own channel and the consumer had to
collect them. That is fan-**in** done by hand at the consumer. A second
consumer would have to be threaded through the same handoff, twice, and the
engine's events and the bus's events would reach it through two more channels.
The number of channels is producers × consumers, which is the shape that does
not scale and the reason this ADR exists.

---

## Q1 — Where does fan-out live?

**Decision: a hub in `postio-core::bridge`, constructed by the composition
root, standing between every producer and every consumer.**

```text
   producers                     hub                     subscribers
   ----------                ----------                ---------------
   bus handlers  ──sink──►  ┌──────────┐  ──stream──►  window (GTK)
   sync engine   ──sink──►  │ EventHub │  ──stream──►  postio-mcp
   config watch  ──sink──►  └──────────┘  ──stream──►  a test, a CLI
```

- `EventHub::sink()` hands out `EventSink`s. They are today's type: `Clone`,
  origin-taggable, `emit` never blocks. The bridge takes its sink from the hub
  instead of making a private channel (`BridgeBuilder::build` grows a variant
  that accepts one; `Bridge::new`'s signature and behaviour are unchanged —
  it builds a hub, keeps the sink, and returns the first subscription).
- `EventHub::subscribe(label)` returns a private `EventStream` — the existing
  type, the existing methods (`next`, `try_next`, `next_blocking`, the
  `_tracked` variants, `len`, `is_closed`), the existing not-`Clone` rule.
  Each subscriber has its own queue; an event is delivered to every queue that
  exists at emit time. `Event` is already `Clone`, which is what makes this
  cheap.
- Internally: the sink writes into a subscriber table
  (`RwLock<Vec<...>>`-shaped; emit takes a read lock, subscribe a write lock).
  No pump task, no extra hop — an emit with one subscriber is today's
  `try_send` plus an uncontended read lock. That is the "no overhead when the
  window is the only subscriber" constraint held by construction rather than
  by benchmark.

**Fan-in comes free, and it simplifies the tree that exists.** With the engine
and the bus both holding sinks on one hub, the window needs **one**
subscription instead of two streams collected in a `RefCell`. The
`Vec<Option<EventStream>>` handoff and the two `commands::drain` calls
collapse to one each. This is the part of the decision that pays rent before
MCP exists.

One consequence to be deliberate about: today the two queues are independent,
so events from the engine and the bus interleave arbitrarily at the reader.
The hub preserves per-producer order (one sink's emits arrive in emit order)
and makes cross-producer interleaving the order of `emit` calls, which is
*more* ordered than today, not less. Nothing may start depending on
cross-producer order anyway — it was never there to depend on.

`event_channel()` stays, as the isolated pair it is: tests and any piece that
wants a private channel with no hub keep it. The hub is the composition
root's arrangement, not a global.

## Q2 — Backpressure: unbounded, with a watermark — and the fallback written down

Both of today's queues are unbounded, deliberately: the UI must never block a
handler, and a burst of IDLE updates must never stall the sync engine. The
question is whether a *second* subscriber changes the calculus. It does not,
yet:

- Event volume is mail-scale, not packet-scale — bounded by what a mailbox
  does, the same traffic the window already absorbs inside a 16 ms budget.
- Every subscriber is **Postio's own in-process code**. The MCP server's
  drain loop is Postio's loop; the external agent's slowness lives on the
  other side of the MCP server's own buffering and must not reach the drain.
  That is a rule for `postio-mcp`'s implementation, and it is the same rule
  the window already follows (drain fast, repaint on your own schedule).
- Loss is the property that must not be traded away. A subscriber that missed
  its own `InvocationFinished` hangs, and a subscriber that missed an unknown
  subset of repaint events holds state that is *silently* wrong — the failure
  ADR 0005 Q10 calls the dangerous one.

**Decision: every subscription is unbounded, exactly like today. The hub
tracks per-subscriber queue depth, and crossing a high watermark logs at
`warn` with the subscriber's label** — never content, per the logging rules —
**so a wedged drain loop is a line in the log rather than an OOM mystery.**

**The recorded fallback, for the day a subscriber legitimately paces on
something external:** bounded queue, and on overflow the hub **closes that
subscription**. The stream ends; the subscriber knows totally rather than
partially, resubscribes, and rebuilds from the store — the same shape as the
resync-integrity rule in ADR 0001 (a skipped untagged response forces a full
resync, never a quiet gap). Silent-gap semantics (tokio `broadcast`'s
`Lagged`) are rejected outright, in both the shipped design and the fallback:
a consumer that must handle "I missed an unknown subset" has to carry
resync-from-store logic that no consumer has today, to survive a condition
that none of them can currently reach.

## Q3 — What N subscribers see: everything, and the hub filters nothing

**Every subscriber receives every envelope, origins included.**

- An `InvocationId` is a process-local integer, not content. Seeing that
  *some* command finished leaks nothing a process-mate could not already see;
  the correlation filter (`is_from`) stays correct because ids are
  process-unique.
- Per-subscriber filtering in the hub would put ADR 0010 Q6's scoping —
  per-account, per-mailbox opt-in — in a layer that cannot know it. The hub
  knows labels and queue depths; which accounts an MCP session may reveal is
  the MCP server's contract with its own configuration, enforced where the
  data leaves the process (its tool results), not where events move inside
  it. One enforcement point, at the trust boundary, instead of two that must
  agree.
- The subscription **label** is the audit hook the issue asked for: the
  subscriber is identifiable at the bridge (`"gtk"`, `"mcp"`, a test's name),
  appears in the watermark warning and in `Debug` output, and gives ADR 0010's
  tool-call log a stable name for who was listening. Labels are for
  diagnostics; nothing may dispatch on them.

## Q4 — A late subscriber starts at *now*

The hub keeps no history. A subscriber that joins after startup — the MCP
server attaching to a running window, a settings pane opening — receives
events from the moment of subscription and reads the store for everything
before it.

This is already the contract the window itself lives by: it feeds from the
store at startup and applies events as diffs from there. Stating it as a rule
closes the tempting wrong door — an event replay buffer — which would make the
hub a second, unbounded, in-memory copy of recent mailbox activity with no
consumer that needs it. Events are notifications; SQLite is the record.

---

## Alternatives

**`tokio::sync::broadcast`.** The obvious primitive, rejected for its loss
model: capacity must be chosen, and overflow hands the slow subscriber
`Lagged(n)` — a silent gap every consumer must then know how to repair. It
would also change the consumer-facing types and put a bound on the window's
queue, which today is deliberately unbounded. The hub keeps total delivery,
today's types, and the window's guarantees.

**MCP as a separate process over its own session.** #82 makes it conceivable:
a second process, its own store handle, its own engines. Rejected — it is ADR
0010 Q2's trap with better manners. Events do not cross processes, so the
window goes blind to MCP's writes (a draft the agent created appears only
after a restart or a poll); two processes mean two operation-queue drainers
and two IDLE connections per account; and "one set of rules" becomes "two
copies of the rules, hopefully equal". In-process, behind the hub, there is
one engine, one queue, one event flow.

**A two-tier design: privileged window, lesser subscribers.** Bounded queues
and lag semantics for everyone but the primary. Rejected as complexity
without a beneficiary: it hard-codes "the window matters most" into a layer
that should not know which consumer is the window, and it buys protection
against a failure mode (a slow in-process drain) better handled by the
watermark plus the recorded disconnect fallback — uniformly, when evidence
arrives.

**Filtering per subscriber in the hub.** Rejected in Q3: scoping enforced in
the layer that cannot know the policy, duplicating the enforcement ADR 0010
already places at the tool surface.

---

## Consequences

- **`postio-core::bridge` gains the hub;** `Bridge::new` keeps its signature
  (hub built inside, first subscription returned), so no existing caller
  changes. `postio-core` gains no dependencies — the table is `std`.
- **`postio-app` simplifies:** the engine's sink comes from the hub, the
  `Rc<RefCell<Vec<Option<EventStream>>>>` handoff and the per-stream
  `commands::drain` calls become one subscription and one drain. When #82
  extracts `postio-session`, the *session* owns the hub and each frontend
  subscribes — which is the whole of what ADR 0010's "MCP is a second
  frontend" needs from this layer.
- **The implementation is its own `ready` issue**, filed with this ADR. The
  decision does not block on it; #14 and #137 can cite this document today.
- **ADR 0002 and ADR 0010 get pointer amendments** so the two passages that
  deferred to each other now name this ADR. The broader doc-drift pass stays
  #148's.
- The watermark threshold is an implementation constant with a comment, not
  configuration. Nobody tunes a queue depth in `config.toml`.

## What would falsify this

Two things, both measurable when real consumers exist:

- A subscriber that genuinely must pace on something external — at which
  point the recorded fallback (bounded + disconnect + resubscribe-and-resync)
  gets implemented for that subscriber, and this ADR gains an amendment
  saying so.
- An emit-path cost that shows up in the 16 ms interaction budget with
  several subscribers attached. The design bets that a read lock plus one
  `try_send` per subscriber is invisible at mail-scale volume; the perf
  budgets in CI are the referee, and losing that bet reopens the pump-task
  variant (one hop, no lock on the emit path).
