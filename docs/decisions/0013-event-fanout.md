# ADR 0013 — Event fan-out: a hub between producers and subscribers

- **Status:** Accepted (2026-08-24); built as `EventHub` in `crates/postio-core/src/bridge.rs` (#176)
- **Decision:** one event hub, owned by the composition root. Every producer's `EventSink` feeds it; every consumer subscribes by name and gets a private `EventStream`.

## Rules
- `EventHub::sink()` hands out `Clone`, origin-taggable sinks whose `emit` never blocks. `EventHub::subscribe(label)` returns today's `EventStream` API (`next`, `try_next`, `next_blocking`, `_tracked`, `len`, `is_closed`); `EventStream` stays not-`Clone`, because a cloned receiver steals events instead of duplicating them. `event_channel()` stays for tests that want a private channel.
- Emit takes a read lock on the subscriber table: no pump task, no extra hop. One subscriber costs a `try_send`.
- Subscriptions are unbounded, like the queues before. A per-subscriber depth watermark logs `warn` with the label, never content. The fallback is a bounded queue that closes the subscription on overflow, so the subscriber resubscribes and rebuilds from the store. Silent-gap semantics (tokio `broadcast` `Lagged`) are rejected.
- Every subscriber sees every envelope, origins included (an `InvocationId` is a process-local integer). The hub filters nothing: scoping is enforced where data leaves the process, such as MCP's tool results ([ADR 0010](0010-mcp-surface.md)).
- Labels (`"gtk"`, `"mcp"`, a test name) are for diagnostics. Nothing dispatches on them.
- Events are notifications, not a journal. A subscriber joins at now and reads the store for the past. No replay buffer.
- Per-producer order holds; nothing may depend on cross-producer order.
- Several frontends in one process subscribe to the hub; the session owns it.

## Why
- Channels scale as producers times consumers when each consumer collects streams by hand. A hub makes the window one subscription.
- Loss is the property not to trade: a subscriber that missed its own `InvocationFinished` hangs, and a silent gap leaves state wrong (ADR 0005 Q10).
- A separate MCP process cannot see events, doubles queue drainers and IDLE connections, and leaves two copies of the rules.
- A privileged-window tier hard-codes which consumer matters.

## Revisit when
- A subscriber must pace on something external, or the watermark fires in practice.
