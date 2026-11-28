# ADR 0028 — A rule runs the same verb a keystroke does, from one storage-level implementation

- **Status:** Accepted (2026-09-14). Amends ADR 0008 Q5.
- **Decision:** The mutating half of the verbs lives in `postio-storage`; the command bus and the rules pass both call it. `forward` keeps ADR 0008 Q5's guard (arbitrary targets, never an own-account address), with the target a literal.

## Rules
- There is no rules-only mutation path. A verb writes the row, enqueues the operation (`Operation::Delete { from, trash }` for trash), and knows nothing about its caller.
- Split: `postio-storage/src/actions.rs` (`relocate`, `set_flag`) writes rows and enqueues. `postio-session` resolves the target from the view, pushes undo and emits.
- A storage verb takes a `&Connection` already inside a transaction and never opens its own. Compose with `postio_storage::transaction`. A rule's action must commit in the same transaction as the message insert (ADR 0008 Q3).
- No new crate. `postio-storage` learns nothing about `Command`.
- A rule pushes no undo entry. Nothing a rule does is unrecoverable (a rule may not `delete`).
- A rule's `move:` stays within its account, so the cross-account saga (ADR 0005 Q9) is never reached. Cross-account rule targets need a new decision.
- `forward` target is a constant literal: no templating from the message, ever.
- Forward guards: (1) never forward a message a rule already forwarded (Postio-set header); (2) refuse any address of a configured account; (3) rate-cap per rule per hour, and raise `Attention` at the cap rather than drop mail. Guards 1 and 3 are loop and volume guards, not security guards.
- A forward is an ordinary queued send and appears in Sent. The rules view shows each target in full.

## Why
- Re-implementing verbs in `postio-sync` is a second implementation that drifts. Moving the pass up into `postio-session` breaks the same-transaction rule.
- A literal target removes the exfiltration class. Anyone who can write `config.toml` already runs as the user, so visibility, not prohibition, answers the persistence risk.
- Rejected: confirmation dialogs (the rule is the consent), own-accounts-only (deletes the feature).

## Revisit when
- Rules gain cross-account targets.
- A message can say which rule touched it (#1118).
- The rules pass lands on `main`; it is on `feature/rules` and is the second caller.
