Work `postio-plp4` — open Postio's command vocabulary so an extension,
an MCP tool or an AI action can be a first-class command.

## Read before anything else

- `bd show postio-plp4` — the bead carries the design sketch and the
  acceptance criteria. It is the spec; this prompt is only the framing.
- `docs/ARCHITECTURE.md` §2 (the registry is the source of truth) and §3
  (command ids are a file format). Both are constraints on your design.
- `docs/architecture-review-2026-08.md` §4 and §5 — the full argument.
- `crates/postio-core/src/{command,registry,dispatch,bridge}.rs`.

## Before you write a line of code

**1. Run `/lanes`.** This bead lives in `postio-core`, but its blast radius
does not: there are ~368 `CommandId::` references across ~34 files in
`postio-core`, `postio-gtk`, `postio-app` and their tests. `postio-gtk` and
`postio-app` are crates other sessions routinely own. If anyone is active in
either, **stop and report** rather than editing their crate — CLAUDE.md is
explicit that a bead needing someone else's crate goes in the notes, not in
their files.

**2. Measure before you design.** Of those ~368 references, how many are
exhaustive `match` arms and how many are just constructions
(`CommandId::Archive` used as a value)? Only the matches break when the enum
gains a variant. That number decides the whole shape of the change — whether
the extension id belongs *inside* `CommandId` or *beside* it — and it is
cheap to get. Do not skip it and guess; the bead's design sketch proposes
`CommandId::Ext(ExtId)` but that is a proposal, not a decision.

**3. Land the design as an ADR** in `docs/decisions/` before implementing.
`0001-imap-library.md` is the model: context, options weighed, decision,
consequences. This is a change to the contract every frontend and every future
extension sees — it deserves the same treatment the IMAP library got. If the
ADR concludes the bead should be several beads, `bd create` them and say so.

## Constraints that are not up for negotiation

- **Keep what the closed enum buys.** Exhaustive matching so a command cannot
  be silently unhandled; `CommandId` serialising as a stable string so `[keys]`
  in `config.toml` stays a real file format; `destructive` + `recovery`
  machine-checked. A design that trades any of these away for extensibility is
  the wrong design — the ask is to *widen* the vocabulary, not open it.
- **Extension commands register, they do not bypass.** They must reach the
  `Ctrl+K` palette and the `?` cheat sheet on the same footing as built-ins,
  and be bindable from `[keys]` with no new syntax. A command outside the
  registry does not exist — that is the registry's own doc comment, and it is
  the property that makes the app good.
- **`destructive` and `recovery` stay mandatory on dynamic specs.** Today that
  invariant is a test over a static table. For runtime registration it has to
  move *into the registration call* so it cannot be skipped. An AI- or
  plugin-invoked destructive action with no undo is far worse than a built-in
  one, because the user did not type it.
- **Correlation must be purely additive.** `send_tracked` / `Invocation::id()`
  / `origin: Option<InvocationId>`. No existing call site should need an edit,
  and the GTK frontend should be free to ignore it entirely.
- **`postio-core` gains no optional dependencies.** See `ARCHITECTURE.md` §9 —
  cargo resolves features as a union across the workspace, which is the whole
  reason `postio-runtime` and `postio-app` are separate crates. A feature flag
  here would put SQLite in the view layer's graph.
- **TDD, as always.** Failing test first. `python3 scripts/check-crate-boundaries.py`
  stays green.

## Sequencing suggestion

The two halves are separable and the first is the risky one. Consider:
vocabulary (ADR → `ExtId` → dynamic registry → palette/cheatsheet/keys) landing
and being green before correlation ids go anywhere near it. Commit each
increment; do not batch.

## Done when

The bead's acceptance criteria pass — including the two easy ones to forget:
registering a destructive extension command with no `Recovery` is *rejected*,
and existing built-in commands still compile with no edits at their call sites.
