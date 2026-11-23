# ADR 0002 — An extensible command vocabulary

- **Status:** Amended 2026-11-22: the extension layer was removed; a real extension would re-add it (accepted 2026-08-24)
- **Decision:** the command registry is the one vocabulary. `CommandId` is a closed, fieldless, `Copy` enum, and commands are named by string wherever they are configured or bound.

## Rules
- `CommandId` stays fieldless. `registry::get` indexes `SPECS[id as usize]`, which compiles only for a fieldless enum (`E0605` otherwise, even for a `Copy` payload). A data-carrying variant breaks the O(1) lookup.
- Command ids are a file format (`ARCHITECTURE.md` §3). `[keys]` and the keymap resolver carry strings and parse to `CommandId` at the window boundary. An id the build does not know leaves the key alone (`Propagation::Proceed`).
- The registry is the single list the palette, the cheat sheet and `[keys]` read. A discoverable command that cannot run is the failure it prevents (`ARCHITECTURE.md` §2).
- A programmatic caller correlates through `CommandSender::send_tracked(cmd) -> InvocationId`. The pump tags the `EventSink` with the origin, so handlers do not change. The channel carries `EventEnvelope { event, origin }`; `EventStream::next` yields a bare `Event`, the `*_tracked` accessors yield the envelope.
- `Event::InvocationFinished { invocation, outcome }` is emitted once per tracked send and only for a tracked send: completed, rejected or failed, including a handler panic or no handler. A caller never waits on an answer that cannot come.
- Several consumers subscribe to one event hub (ADR 0013).

## Why
- Adding a `CommandId` variant breaks two match sites, not the 368 references a grep counts. The cost is the registry's shape, and an opened enum buys nothing, because the binding layer is already string-keyed.
- An extension layer (`ExtId`, `ActionId::Ext`, `ExtInvocation`, `on_ext`/`dispatch_ext`) was built and no production code used it. It was removed.

## Revisit when
- A real MCP or plugin consumer needs commands with payloads shaped like the built-ins'. Re-adding an extension layer then needs its own design, including how `[keys]` entries bind before the extension registers.
