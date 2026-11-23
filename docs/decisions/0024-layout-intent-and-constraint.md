# ADR 0024 — Layout intent is stored; the viewport's constraint is applied, never written back

- **Status:** Accepted (2026-09-02). The three-pane shell it was built in left with the classic app (ADR 0043); the principle stands. Focus applies it: `[focus] reading` is the intent, and a window narrower than 980 px opens the dialog without touching the setting. `postio-tui` derives its panes by width the same way.
- **Decision:** a surface records what the user asked for, and the viewport's constraint is applied on top of it at render time. A constraint is never written back into the intent.

## Rules
- Intent is written only by the user's own actions (a toggle, opening a message, going back) and persisted where that makes sense.
- Constraint (the layout mode) is written only by width breakpoints and is never persisted: last week's answer is worse than none.
- Effective state is computed from both, fresh, every time. Persistence saves the intent, never the effective value.
- A toggle against the constraint is an override while the constraint lasts, not a new standing preference.
- Navigation declares what the user is looking at unconditionally (opening a message claims the reader, back claims the list). No handler branches on the viewport mode.
- Narrowing and widening is lossless; quitting narrow and reopening wide restores the preference.
- A mode switch has no transition (motion budget).
- Test the derivation as pure functions over the intent and constraint, and test with a real window that the right authority writes the property and that saved state is the intent.

## Why
- One boolean held two facts. A breakpoint's answer was saved as the user's preference, so narrowing then reopening wide lost the sidebar with no explanation.
- A surface that branches on the viewport is wrong in the mode its author did not consider.

## Revisit when
- A new adaptive surface appears: apply this rule rather than a second owner restoring its own snapshot.
