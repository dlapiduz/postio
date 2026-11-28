# ADR 0010 — Exposing Postio over MCP

- **Status:** Accepted (2026-08-24); built as `crates/postio-mcp`; amended 2026-10-14 (archive, move and label join the draft as writes) and 2026-11-28 (counts, paging, query writes, `create_folder`). User guide: `docs/mcp.md`.
- **Decision:** a server only; the MCP client direction is deferred. MCP is a frontend over the same commands and events as the apps, not a side door into the store. The write tools are `create_draft` plus opt-in archive, move, label and `create_folder`.

## Rules
- MCP reaches the store only through `postio-client` and `postio-host` ([ADR 0041](0041-one-app-opens-the-store-at-a-time.md)), never `postio-storage` directly: two writers with different rules corrupt a mailbox. `postio-mcp` may not depend on the store engine, the protocol, a toolkit, WebKit, an inference engine or an HTTP or WebSocket server (`check-crate-boundaries.py`).
- It opens the store per call and closes it, so a long agent session never locks the person out. A call while another Postio holds the store gets that app's sentence. It starts no engine and makes no connection.
- Read tools: `list_accounts`, `list_mailboxes`, `search_mail` (the query language, verbatim; a total and a cursor), `mail_stats` (counts by sender, domain or month), `read_thread`, `read_message`. Counts are computed over shared folders inside the engine query, never filtered afterwards.
- `create_draft` goes through the composer's own save (`Req::SaveDraft`); it never queues `Operation::Send`.
- There is no send, forward, delete, mark-read or empty-trash tool: absent, not gated. Confirmation dialogs raised by an agent train click-through and fail when Postio is closed.
- Organizing tools (`archive_messages`, `move_messages`, `add_label`/`remove_label`) send the command a keystroke sends, via `send_tracked`, and wait for their own `InvocationFinished`. Each needs its verb in `actions` for the account. Every message and the destination must be shared; one refused message refuses the call. `move_messages` goes only into inbox, archive or regular folders, decided by role. Each answers with the source folders and says whether undo is available.
- An organizing tool takes ids or a `query`: the shared matches not yet done, at most 5,000 a call in tracked chunks of 500. `dry_run` gives the count; a real call must pass it as `expect`.
- `create_folder` needs `"folders"`; its path and parent must already be shared, and it never takes a role's name or goes under Trash or Junk ([ADR 0050](0050-a-folder-is-created-locally-first.md)).
- Opt-in is `[mcp.accounts."<address>"] mailboxes = [...]` and `actions = [...]`, read on every call. An unshared account or folder is refused in the words used for one that does not exist, and is absent from `list_accounts`. Revocation takes effect on the next call.
- Returns are attacker-controlled. Bodies are `to_text()` or restricted HTML from `postio-body`, never raw `text/html`. Untrusted content is fenced in its own field. Tool names, descriptions and schemas are static literals; nothing derived from mail reaches them. Organizing tool descriptions say they act only on what the person asked. Quoting into a draft goes through the composer's `parse`.
- Transport is stdio only: no listening socket.
- Every call is logged to `mcp-access.log` beside the store: ids, counts and outcomes, never content, and label ids not names; a query call logs its count, never the query. This is separate from `egress_log`.
- The injection test runs with every organizing verb on, `"folders"` included: a hostile fixture leaves the message untouched, no folder made and nothing queued. JSON-RPC is written by hand, not via `rmcp`, because schemas must be literals.

## Why
- The server direction opens the mailbox to existing tools. Filing is the useful agent work, and it is visible and reversible; sending is not.
- Counting in the engine and acting on a query turn thousands of calls into a few; `expect` keeps a call to what the agent was shown.
- Needing a headless frontend forced `postio-session` (GTK-free composition root) and event fan-out ([ADR 0013](0013-event-fanout.md)); both are built.

## Revisit when
- Someone wants an agent write beyond filing: build a `Proposal` queue where a human accepts (#2275).
