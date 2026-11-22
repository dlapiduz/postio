# ADR 0022 — Extensions contribute table rows, not pixels

- **Status:** Accepted (2026-08-28) as policy. No extension layer exists: ADR 0002's extension commands were removed on 2026-11-22, and extensions are unscheduled (#137, v3).
- **Decision:** MCP is the extension mechanism, and there is no second one. An extension adds rows to tables Postio already renders (commands, and queries that become saved searches), never widgets. No dylib host, no WASM runtime, no UI description language.

## Rules
- Contributions are commands and named queries. Every discoverable surface (keymap, palette, cheat sheet, context menu, key hints, `keybindings.md`) derives from one table, so a row reaches all of them on every frontend. A named query becomes a pinned saved search.
- Invoking a contributed command goes back out to the extension as an MCP tool call: a declaration in-process, execution out-of-process.
- Permission, isolation, audit and distribution are ADR 0010's: opt-in per account and mailbox, revocable on the next call, a subprocess over stdio, ids and outcomes logged. Injection follows ADR 0009's threat model, because an extension reads attacker-controlled mail.
- An extension cannot render a message body, draw a pane or intercept a display synchronously. If it must show something, it shows mail, a query result or a command that acts.
- Nothing extension-owned runs on the frame path (16 ms budget), and `postio-core` gains no optional dependencies, so no in-process host.
- Command ids are a file format (`ARCHITECTURE.md` §3), so an extension's ids are a compatibility surface from its first release.

## Why
- A drawing extension is toolkit-specific and dead on the other frontend (ADR 0019). A neutral UI language is a second product.
- A dylib has no stable ABI or sandbox. WASM buys a sandbox for code that has nothing left to do in-process. A subprocess plus protocol is MCP, which exists.

## Revisit when
- Extensions are scheduled: decide lifecycle (late registration against resolved `[keys]`), versioning, installation UI and the scope of query contributions. Re-adding an extension layer needs ADR 0002's "Revisit" first.
- A web-view third frontend, a concrete extension that is none of command, query or mail, or rules needing third-party actions.
