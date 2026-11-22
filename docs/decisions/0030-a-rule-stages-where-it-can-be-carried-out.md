# ADR 0030 — A rule stages where it can be answered *and* carried out

- **Status:** Accepted (2026-09-06). The rules engine is on the unmerged `feature/rules` branch, not `main`. Amends ADR 0008 Q3 and Q5.
- **Decision:** A rule's stage is the later of what its query needs and what its actions need. `forward:` needs a body to carry out, so a rule carrying it is `OnBody`. The forward never fetches its own body; it waits for the backfill lane that fetches it anyway.

## Rules
- `stage(rule) = max(needs_body(query), needs_body_to_run(actions))`, with `OnArrival < OnBody`. `needs_body_to_run` is a function over the action vocabulary, true only for `forward:` today. A new action with a requirement is a new match arm.
- A rule stages as a whole. Its `move:` and `label:` run at the body point with its `forward:`, in one transaction (ADR 0008 Q6). Actions never split across the two points, because `stop` would mean two things for one rule and the forward would read a message the same rule already moved.
- A rule never pulls its own body. The interactive lane is for a person opening a message. It ignores caps and metered settings, so a rule using it would slow the user's clicks invisibly.
- No rules-only fetch path (ADR 0028, ADR 0008 Q5). One policy about fetch cost.
- The config validator notes "runs after the body is fetched, not on arrival" for `forward:` as it does for `body:`.
- When any folder is excluded from backfill, an `OnBody` rule says so in its settings row and names the folders (static, computed at config load). Postio neither re-enables backfill nor forwards a bodyless message.
- The three forward guards are unchanged (ADR 0028). The guard 1 header is on the message at either point.

## Why
- ADR 0016 has no backfill horizon: every body arrives eventually, newest first. A new arrival is at the front of the background lane, so the rule is late by one body. "May never be forwarded" does not occur.
- There is no extra fetch. Staging only orders against work already scheduled.
- Forwarding on arrival would send headers with nothing under them.

## Revisit when
- Another action needs the body or another fact before it can run.
- The rules engine reaches `main`.
