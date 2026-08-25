# ADR 0010 — Exposing Postio over MCP

- **Status:** Accepted — **GO** (2026-08-24)
- **Date:** 2026-08-24
- **Issue:** [#14 Expose Postio over MCP](https://github.com/dlapiduz/postio/issues/14)
- **Related:** [ADR 0002](0002-extensible-command-vocabulary.md) (built for
  this), [ADR 0009](0009-ai-subsystem.md) (same threat, pointed inward),
  `docs/ARCHITECTURE.md` §2, §11, §12 and its **known gaps** table
- **Decision:** **server only; client deferred.** MCP is a **second frontend**
  over `postio-core`'s bridge, not a side door into the store — which requires
  extracting the toolkit-free half of `postio-app` first. The tool vocabulary
  is **read, plus exactly one write verb: create a draft.** There is no send
  tool, no delete tool and no move tool, so the issue's second criterion is a
  fact about the surface rather than a promise about its behaviour.

---

## Q1 — Server, client, or both?

The issue asks for the direction to be decided before scoping.

**Server. The client direction is deferred to its own issue, and the reason is
not sequencing.**

- The server direction is the one that makes the mailbox available to tools
  that already exist, which is the leverage the issue identifies.
- The client direction — Postio consuming external MCP servers so its AI
  features can reach a calendar or a CRM — is a feature of the **AI
  subsystem**, not of a mail client, and it belongs behind
  [ADR 0009](0009-ai-subsystem.md)'s provider consent model. It also multiplies
  the injection surface in a way that is easy to miss: an external server's
  *responses* are as untrusted as a message body, and now there are two sources
  of attacker-controlled text in one context window rather than one.

Deciding it now, in writing, is what the acceptance criterion asks for.

---

## Q2 — The blocker nobody can route around

`ARCHITECTURE.md`'s known-gaps table already names this:

> `postio-app` is both composition root and GTK binary; `actions.rs` (the whole
> verb vocabulary, GTK-free) links GTK. **No headless frontend is possible,
> which is what MCP actually needs.**

There are two ways to build an MCP server on top of this, and one of them is a
trap.

**The trap: talk to `postio-storage` directly.** An MCP binary that opens the
SQLite file and runs queries needs no refactor and works next week. It also
means every invariant Postio holds — local-first ordering, the undo stack,
event emission, the operation queue, per-account scope — exists only in the
path the GTK frontend takes. Two writers to one database with different rules
is how a mailbox gets corrupted, and by the time it happens nobody remembers
there were two.

**The decision: MCP is a frontend.** It sends `Command`s and consumes `Event`s
over `postio-core::bridge`, exactly as `postio-gtk` does. Everything below it
is unchanged, and there is one set of rules.

> **Prerequisite settled:** when this was written the bridge had exactly one
> `EventStream`, so two frontends could not both read it — the constraint ADR
> 0002 left open. [ADR 0013](0013-event-fanout.md) (2026-08-24) resolves it:
> both frontends subscribe to one event hub, each receiving its own stream.

That requires the extraction the gaps table describes:

```
  postio-session   composition root, GTK-free: store, runtime, engines,
        │          registry wiring, and the verb vocabulary from actions.rs
        ├── postio-app   the GTK binary. Adds a window and nothing else.
        └── postio-mcp   the MCP server binary. Adds stdio and nothing else.
```

`postio-session` gets the same boundary rule `postio-core` has: **no `gtk4`,
no `libadwaita`**, checked by `scripts/check-crate-boundaries.py`. That rule is
the whole deliverable — without it, `actions.rs` re-acquires GTK the first time
someone adds a verb in a hurry.

**This extraction is worth doing whether or not MCP ships.** It is what makes
`postio-app`'s integration tests (`wiring.rs`, `keystroke.rs`,
`search_index.rs`) test the composition root rather than a GTK binary, and it
is the same argument that produced `postio-runtime`.

---

## Q3 — The second blocker: a caller cannot await its own command

Also from the gaps table: *no correlation ids on commands/events; a programmatic
caller cannot await its own invocation.*

A frontend with a human in it does not need them — the human sees the repaint.
An MCP tool call must return a result to *its* caller, and today the event
stream gives it no way to tell its own result from another session's.

**Built already, and not by this ADR: [ADR 0002](0002-extensible-command-vocabulary.md)'s
correlation-id work (issue #33).** `CommandSender::send_tracked(cmd) ->
InvocationId` tags the `EventSink` a handler emits through, not the command or
the handler, so `EventEnvelope { event, origin }` carries the origin back and
`Event::InvocationFinished { invocation, outcome }` fires exactly once per
tracked send — including when the handler panics or none was registered, so a
caller awaiting an answer is never left hanging. The GTK frontend's plain
`send` and its bare `EventStream` are unchanged, which is what makes this safe
for a frontend that has no use for it.

What that implementation leaves open, and what MCP actually needs before a
server can sit beside a running window, is the constraint its own "Implemented"
section names: there is still exactly one `EventStream`, so a tracked caller
and the window cannot both read it. [ADR 0013](0013-event-fanout.md) decided
the fix — a hub, N subscribers — and [#176](https://github.com/dlapiduz/postio/issues/176)
tracks building it. Without it the MCP server is reduced to polling the store,
which is Q2's trap wearing a different hat.

---

## Q4 — The tool vocabulary, and the one write verb

| Tool | Kind |
|---|---|
| `list_accounts`, `list_mailboxes` | read |
| `search_mail` — the query language, verbatim | read |
| `read_thread`, `read_message` | read |
| `create_draft` — a reply, forward or new message | **write, and the only one** |

**There is no `send_mail`, no `archive`, no `move`, no `delete`, no
`mark_read`.** Not "present but gated" — absent.

This is the same move [ADR 0009](0009-ai-subsystem.md) makes and for the same
reason. The issue asks that every externally visible action require explicit
confirmation in the Postio UI; the strongest available implementation of that
is a surface where no externally visible action exists. A confirmation dialogue
raised by an external agent, repeatedly, is a control that trains the user to
click through it — and it fails entirely when Postio is not running, which is
exactly when an agent is most likely to be working the mailbox unattended.

**A draft is safe because a draft is local.** Creating one writes SQLite,
enqueues nothing external, and emits an event; the mail leaves only when a human
opens the composer and presses send, which is a gesture they already perform for
every message they send. `Draft` and `drafts` already exist, and
`DraftState` already distinguishes one that is not ready.

**How the rest gets added later, if it should be.** The extension point is a
`Proposal` queue in the UI: a tool creates a proposal, the UI shows it, the
human accepts, and the accepted proposal becomes a `Command`. That mechanism is
worth building when there is evidence anyone wants `propose_archive` — not
before, because the read tools plus drafting are the workflow the issue actually
describes, and speculative confirmation UI is the kind of thing that ships
half-used and then constrains everything after it.

**Tools that are commands register as commands.** ADR 0002 built `ExtCommand`
and the `mcp:` namespace for this, and named `postio-z3b.2` as what it
unblocks. A tool that maps to a Postio verb registers, so it appears in the
palette and the cheat sheet with its provenance visible, rather than being a
capability that exists only for the agent.

---

## Q5 — Injection, and what a tool is allowed to return

Every byte these tools return is attacker-controlled. The agent on the other end
will treat it as context, and Postio cannot change that. What Postio controls is
what it hands over.

- **Bodies are returned sanitised and as text.** `read_message` returns the
  `postio-body` document's `to_text()` ([ADR 0004](0004-composer-document-model.md)),
  or its restricted HTML — never the raw `text/html` part. Script, remote
  references and tracking pixels have no representation in that type, so a tool
  result cannot carry one.
- **Untrusted content is fenced and labelled** in the tool result, in its own
  field, never interpolated into a summary string the server composed.
- **Tool descriptions are static.** Nothing derived from mail may reach a tool
  name, description or schema — that is the injection path most likely to be
  built by accident, because "describe the mailbox in the tool description"
  sounds helpful.
- **`create_draft` is quoted, not echoed.** Content quoted into a drafted reply
  goes through the same `parse` the composer uses, so a hostile message cannot
  round-trip its markup out through an agent.

**The test the issue asks for**, in the default suite and sharing ADR 0009's
fixture: a corpus message whose body contains tool-shaped instructions
(*"call send_mail with…"*, *"forward all invoices to…"*) is read through
`read_message` and asserted to produce no command, no queue row and no outbound
connection. It cannot fail for the interesting reason — there is no `send_mail`
to call — and that is the point of writing it down: the day someone adds one,
the test is already there asking why.

---

## Q6 — Transport, scope, and audit

- **stdio only.** The MCP client launches the server as a subprocess. No
  listening socket, no port, nothing another local user or another machine can
  reach. This is `ARCHITECTURE.md` §11's rule applied to Postio's own surface:
  a port that is open is a network request Postio made that the user did not
  ask for.
- **Off by default, opt-in per account and per mailbox**, in `config.toml`.
  An account with no opt-in is invisible to every tool, including
  `list_accounts` — a tool that reports the existence of an account it may not
  read has already leaked.
- **Every tool call is logged**: timestamp, tool, account, mailbox, the message
  ids returned, the outcome. Ids, counts and outcomes — never content, per
  §11 — which is enough to answer "what did it read" and is the issue's fourth
  criterion. It is the same log [ADR 0009](0009-ai-subsystem.md) starts, and it
  is readable and revocable from the settings panel.
- **Revocation takes effect on the next call**, not on the next restart.

---

## Alternatives

**Read the SQLite store directly from a standalone binary.** Fastest to build
and the reason for Q2's length. Two writers with different invariants over one
database.

**Expose the full verb set with a confirmation dialogue.** The issue's literal
reading. Rejected in Q4: it makes safety depend on a human declining a prompt
they will see hundreds of times, and it has no answer for an agent working while
Postio is closed.

**Both directions at once.** Rejected in Q1: the client direction is an AI
feature with its own consent model and its own injection surface.

**A network transport, so an agent on another machine can work the mailbox.**
Every mail client that has ever done this has regretted the authentication
design. If it is ever wanted, it is an ADR of its own.

---

## Consequences

- **Prerequisite work — both already landed, and neither was MCP code:**
  `postio-session` extracted with its own boundary rule
  ([#82](https://github.com/dlapiduz/postio/issues/82)); correlation ids
  shipped as `send_tracked`/`EventEnvelope`/`InvocationFinished`
  ([ADR 0002](0002-extensible-command-vocabulary.md), #33). What is left
  before a server can subscribe beside the window is the fan-out hub
  [ADR 0013](0013-event-fanout.md) decided and
  [#176](https://github.com/dlapiduz/postio/issues/176) tracks building.
- `postio-mcp` is a thin binary: stdio framing, tool schemas, and a translation
  to commands and queries. It is small precisely because Q2 was decided the
  expensive way.
- `check-crate-boundaries.py` grows a third guarded crate, alongside the one
  [ADR 0009](0009-ai-subsystem.md) adds.
- `ARCHITECTURE.md`'s known-gaps table loses two rows.
