# ADR 0022 — Extensions contribute table rows, not pixels

- **Status:** Accepted (2026-08-28) — for the part that is forced. Four
  questions are deliberately left open; see *What this does not decide*.
- **Date:** 2026-08-28
- **Decision by:** a `/ux-architect` session working the `needs-architecture`
  queue.
- **Issue:** [#137](https://github.com/dlapiduz/postio/issues/137)
- **Related:** [ADR 0002](0002-extensible-command-vocabulary.md) (the
  vocabulary seam), [ADR 0010](0010-mcp-surface.md) (MCP is a frontend over
  the bridge), [ADR 0019](0019-macos-frontend.md) (a second frontend, and the
  reason this ADR ends where it does), [ADR 0009](0009-ai-subsystem.md)
  (attacker-controlled mail), [#82](https://github.com/dlapiduz/postio/issues/82)
  (extract `postio-session`)
- **Decision:** **MCP is the extension mechanism, and there is no second one.
  An extension adds rows to tables Postio already renders — commands, and the
  queries that become saved searches — never widgets.** No dylib host, no WASM
  runtime, no toolkit-neutral UI description language. The cost, stated
  plainly: **no extension can render a message body, draw a pane, or intercept
  a display synchronously.**

---

## What #137 already settled, and what it left

ADR 0002 built the seam and answered *how* an extension's commands reach the
keymap, the palette, the cheat sheet and `[keys]`. ADR 0010 decided MCP is a
frontend over `postio-core`'s bridge — commands down, events up — and rejected
direct store access because two writers with different invariants is how a
mailbox gets corrupted. #137's own title records that the extension API is
MCP.

What was left is the fork it names as the biggest one: **UI extensions — the
one case MCP supposedly cannot express.** It turns out MCP can express it,
once the question stops being "how does an extension draw" and becomes "what
does an extension contribute".

## The argument that decides it is ADR 0019, and #137 predates it

**Postio has a second frontend scheduled.** A native macOS frontend over the
same engine, Swift over Rust, thirteen of fifteen crates already building
unchanged. Every architectural boundary in this workspace — `postio-core` has
no GTK, `postio-gtk` has no SQL or protocol — exists to keep that possible,
and ADR 0019 turned the possibility into a plan.

An extension that draws is necessarily toolkit-specific. Which leaves two
shapes, and both are bad:

- **Toolkit-specific extensions.** An extension written against GTK4 is dead
  on macOS. For a project whose entire architecture is *one engine, two
  frontends*, an extension ecosystem that splits per frontend is exactly
  backwards — it puts the fragmentation in the layer the architecture was
  built to keep whole.
- **A toolkit-neutral UI description language**, rendered by each frontend.
  That is a second product. Postio would own a layout language, its
  versioning, its accessibility semantics, its theming story, and two
  implementations of it — in order to let somebody put a panel in a mail
  client.

Neither is worth what it costs, and this is an architecture judgement rather
than a taste one: it follows from a decision already made.

Two further constraints point the same way and would matter even without ADR
0019. **The 16 ms interaction budget** (`PRODUCT.md` §18) rules out calling
into an extension synchronously on the path that produces a frame, whatever
the mechanism — a WASM call, an IPC round trip and a dylib call across an
unstable ABI are three different ways to blow a frame budget you do not
control. And **`postio-core` must not gain optional dependencies**
(`ARCHITECTURE.md` §9), so an in-process host cannot live where the registry
does.

## What an extension contributes instead

Every discoverable surface in Postio is generated from an enumerable table.
That is not incidental; it is `PRODUCT.md` §8's structural requirement, and it
is what makes this design work:

> the keymap, the `Ctrl+K` palette, the `?` cheat sheet, the context menu, the
> key hints on the focused row and `keybindings.md` are **all derived** from
> one table.

So an extension that adds one row to that table reaches six surfaces at once,
on **every** frontend, having drawn nothing. It cannot break a layout, cannot
stall a frame, cannot draw something that looks like Postio and is not, and
cannot fall behind when a surface is redesigned.

The seam is already built and already assumes this. `ExtCommand` is precisely
a table row — id, title, binding, alternates, contexts, `destructive`,
`recovery` — and the id in its own documentation is `"mcp:summarise-thread"`.
The mechanism was chosen when the seam was designed; this ADR is writing down
what the tree already says.

**Two contribution kinds, and they are the same idea twice:**

1. **Commands** — `registry::register`, which exists, enforces its invariant,
   and has been waiting for a caller. Invoking one goes back out to the
   extension as an MCP tool call; it is a declaration in-process and an
   execution out-of-process.
2. **Queries** — a named query string is a saved search (`PRODUCT.md` §7: *a
   virtual folder is a saved search that is pinned*), so an extension that
   wants a place in the sidebar contributes a query with a name and gets a
   real sidebar row, keyboard navigation, rename and reorder, on both
   frontends, for free.

An extension that wants to *show* something shows it the way Postio shows
things: as mail, as a query result, or as a command that acts. If the thing it
wants to show is none of those, the honest answer is that it is not a mail
client feature.

## Why there is no second mechanism, in one line each

- **Native `dylib`.** No stable Rust ABI, no sandbox, a crash takes the window
  with it, every `rustc` bump breaks every extension, and it cannot exist on
  the Swift frontend at all.
- **WASM component.** The only in-process option that is not reckless, and
  still the wrong one here: it buys a sandbox for code that, under this
  decision, has nothing left to do in-process. Its whole advantage is running
  untrusted code near the data, and untrusted code near the data is what ADR
  0010 Q2 spent its length refusing.
- **Subprocess + protocol.** This *is* the decision. It is MCP, and it already
  has a specification, clients, and an ADR.
- **Declarative only.** #137 observes that the row nobody proposes may be
  enough. It is nearly right: declarative for what an extension *contributes*,
  MCP for what it *does*. Purely declarative would mean an extension cannot
  act, which throws away the seam ADR 0002 built.

## Permission, isolation, distribution: already answered, do not re-answer

#137 lists five questions a design must answer. Four of them are answered by
an extension being an MCP server, and re-answering them would be inventing a
second policy for one channel:

| Question | Where it is already answered |
|---|---|
| Trust and permission | ADR 0010 Q6 — off by default, opt-in per account and per mailbox, revocable on the next call, never the next restart |
| Failure isolation | The subprocess. A hung extension is a hung stdio pipe, not a hung window — and since nothing extension-owned is on the frame path, a slow one cannot cost 16 ms |
| Audit | ADR 0010 Q6 — every call logged with ids, counts and outcomes, never content, readable from the settings panel |
| Distribution and first-run trust | ADR 0010 Q6 — stdio only, launched by the client. No listening socket is the strongest form of "nothing leaves this machine that the user did not ask for" |
| Injection | ADR 0009's threat model applies wholesale. An extension reading mail is reading attacker-controlled text |

The fifth — **lifecycle** — is genuinely new and is left open below, because
ADR 0002 was careful that registration happens before `[keys]` is resolved and
an MCP server that registers late has to be reconciled with that.

## What this costs

Stated as a loss rather than buried, because it is the reason someone will
want to reopen this:

- **No custom rendering.** Nobody can write a renderer for a body format
  Postio does not handle, or draw a chart in the reading pane.
- **No synchronous interception.** Nothing can run *before* a message is
  displayed and change what is displayed. A rule that acts on arrival is
  [ADR 0008](0008-filters-and-rules.md)'s territory and stays there.
- **No new panes.** No sidebar of an extension's own, no third column.

## What this does not decide

Deliberately, and this is a decision about scope rather than an omission.
Extensions are unscheduled (#137 is p4, filed for v3), and four questions have
answers that depend on facts that do not exist yet. Answering them now would
be guessing in the shape of a decision, which is worse than an open question
with its name written down:

1. **Lifecycle.** When an extension loads, what happens when one registers
   after `[keys]` is resolved, and what a failed registration does to a
   binding a user has already written down.
2. **Versioning.** Command ids are a file format (`ARCHITECTURE.md` §3), so an
   extension's ids are a compatibility surface from the day it ships. What
   Postio promises across releases needs a released extension to promise it
   about.
3. **How an extension is installed and named** in the settings panel, which
   needs the settings panel that exists at that point, not this one.
4. **Whether the query contribution needs a scope narrower than a command's.**
   A saved search reads mail; the per-account opt-in may or may not be the
   right granularity for it.

**#82 (extract `postio-session`) is on the critical path** and is a
prerequisite for MCP anyway. Nothing else here is startable until extensions
are scheduled.

## What would falsify this

- **A third frontend that is a web view**, where a toolkit-neutral UI
  description already exists and the second bullet of the ADR 0019 argument
  stops costing what it costs.
- **A concrete extension somebody wants that is genuinely none of a command, a
  query, or mail.** The argument above asserts that set is empty for a mail
  client; one real counterexample is worth more than the assertion.
- **Rules (ADR 0008) turning out to want an extension point.** A rule is a
  saved search plus actions, and if third-party actions are ever wanted, the
  action side is exactly the command contribution above — but the *timing* is
  synchronous-on-arrival, which this ADR says nothing about and would have to.
