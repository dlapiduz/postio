# Contract: registry commands

**What it governs**: the verbs this feature adds. Principle II: a command that
is not in the registry does not exist — not merely unbound, but absent from
every way a user could discover it.

## Existing, reused

| Command id | Key | Scope in this pane |
|---|---|---|
| `reply` | `e` | The conversation's most recent message (FR-008) |
| `reply_all` | `E` | The conversation's most recent message |
| `forward` | — | The conversation's most recent message |
| `archive` | `a` | A single message |
| `archive_thread` | `A` | The whole conversation (FR-008) |
| `undo` | `u` | — |

## Added

| Command id | Acts on | Notes |
|---|---|---|
| reply to the focused message | The message under the rail's mark | FR-009. Key assigned in the registry — **not** the design brief's `⇧e`, which collides with `E` |
| forward the focused message | The message under the rail's mark | FR-009 |
| dismiss the rail | The pane | FR-041. Persists per window (FR-047) |

## Rules

1. Every action the pane draws invokes a registry command **by id**. No local
   reimplementation of a verb (FR-007).
2. Keys are chosen in the registry. `docs/keybindings.md` is regenerated, and
   the test that fails on drift is what proves it.
3. Bindings are overridable from `[keys]` in `config.toml`, keyed by command
   id — which makes these ids a file format, not an internal name.
4. Every command reachable from the keyboard is reachable with the mouse
   (FR-006, FR-056), and destructive ones are confirmed or undoable (FR-011).
5. Tooltips and accessible names state **scope in words** — "Reply to the
   latest message", "Archive all 6 messages" — never the bare verb (FR-008a).
