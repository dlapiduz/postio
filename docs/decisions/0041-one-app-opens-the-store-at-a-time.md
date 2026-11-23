# ADR 0041 — One app opens the store at a time; each runs the host inside it

- **Status:** Accepted (2026-10-14)
- **Decision:** The desktop app (Focus, ADR 0043), the terminal app and the macOS app each open the store themselves, one at a time. Each runs `postio-host` in its own process and reaches mail only through `postio-client`. An app that finds the store open elsewhere says so and does not open it. The app holding the store answers agents on a private socket.

## Rules
- Whichever app starts first opens the store, runs sync, drains the operation queue and does upkeep. There is no background process.
- A second app tells the person to close the other. It modifies nothing, starts no sync and queues nothing.
- Every app runs `postio-host` in-process and uses `postio-client`'s in-process transport (a spawn and a oneshot, no encoding). Each store operation is written once.
- A source install and both Flatpaks share `~/.local/share/postio`.
- The app listens on a Unix socket, `agents/mcp.sock` beside the store, and answers agents' `tools/call` with the same `postio_mcp::Server` that `postio-mcp` runs, over its own host (`postio_mcp::socket`). `postio-mcp` tries the socket first and opens the store itself only when no app listens. The wire is MCP JSON-RPC, one line each way, not the client protocol.
- The app reads `[mcp]` from its own `config.toml` on every call and writes the access log beside the store. `postio-mcp --config` does not apply to calls the app answers.
- `Connector::connect_agent` ties the agent's client to the verbs (selection and undo stack) of the window that connected last, so the window repaints and shows the undo toast, and `u` / Cmd+Z reverses an agent's change. Tracked sends report to the agent.
- The socket folder is `0700` and the socket `0600`. Both ends check the peer's user id (`getpeereid` or `SO_PEERCRED`) and refuse other users. `postio-mcp` refuses a socket or folder another user owns or others can reach. No TCP.
- Every app listens while it holds the store and removes the socket on shutdown. A crash-left socket is replaced. A socket something still answers on is never taken over. In-memory sessions and demos do not listen (the Mac demo does only with `POSTIO_DEMO_AGENTS=<socket>`).

## Why
- Turso holds an exclusive file lock. Its multi-process mode is experimental: no `VACUUM`, open panics. The queue claim and `LocalStore` caches assume one drainer and one writer.
- Rejected: a background `postio-daemon` (too much to start, find, keep alive and version-match); a serde encoding of `Req`/`Resp` so `postio-mcp` could be remote (ninety request kinds across a process boundary).

## Revisit when
- Turso's multi-process mode supports `VACUUM`, closes its panics and offers a cross-process commit signal, or both apps at once is wanted enough to pay for the daemon.
