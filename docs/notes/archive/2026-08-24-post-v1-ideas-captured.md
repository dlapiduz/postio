# Post-v1 ideas captured (mostly now tracked as GitHub issues)

*Archived 2026-09-14: a pre-tracker capture list; every bullet is either a GitHub issue now or already shipped, so the list itself no longer says anything the tracker does not.*

These were captured from conversations with the user before the migration to
GitHub Issues. Cross-referenced below where a GitHub issue already exists;
kept here anyway for the reasoning, which didn't all make it into the issue
bodies.

- **Filters/rules engine** — should reuse the search query parser as its
  condition language rather than inventing a second matching syntax. Now
  tracked as issue #5 (part of epic #19, Triage & Filters).
- **MCP support** — direction (server vs. client vs. both) is an *open
  decision*, and prompt injection via attacker-controlled email bodies is the
  dominant security constraint: no MCP tool may send/delete/move without
  explicit human confirmation. Now tracked as issue #14 (epic #22,
  Integration).
- **Richer signatures** — basic per-identity signatures were already in v1
  scope; this covers multiple named signatures, HTML/plaintext variants, and
  placement control. Now tracked as issue #12 (epic #17, Compose).
- **Unified palette** — the user wants VS Code style: one keypress, one box,
  fuzzy matching, with `>` prefix for commands and plain text for mail
  search. This refactored two previously-separate overlays into one. Already
  shipped; no open issue.
- **Smart labels** — deferred to the AI work. Design note: use cheap header
  signals (`List-Unsubscribe`, `Precedence`, `Auto-Submitted`) before
  reaching for a model, and categories must be visible and correctable. Now
  tracked as issue #8 (epic #19, Triage & Filters).
- **Multi-select / bulk actions** — the key design constraint is that
  selection cannot be a `Vec<MessageId>` for "select all" — the list is
  windowed over the paged store and must never materialise a mailbox, so model
  selection as an id set *or* a predicate (query + exclusions) and resolve it
  in one SQL statement. Bulk archive of 50k must be one update plus one
  queued operation. Also: selected and focused are distinct states (see
  above) — conflating them is the usual bug. Already shipped; no open issue.
