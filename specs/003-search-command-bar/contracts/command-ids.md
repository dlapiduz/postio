# Contract: destination command ids

**Feature**: [../spec.md](../spec.md) | **Consumers**: users' `config.toml`, the
palette, the `?` cheat sheet, `docs/keybindings.md`, the macOS frontend

A command id is a **file format**. Constitution II: bindings are overridable
from `[keys]` in `config.toml`, keyed by command id, "which makes command ids a
file format and forbids casual renaming". Once these ship, a user's
configuration may name them, and renaming one silently breaks that file.

## The ids

```toml
# config.toml — what a user may write once this ships
[keys]
go_to_inbox   = "g i"
go_to_drafts  = "g d"
go_to_sent    = "g t"
go_to_flagged = "g s"
```

| Id | Title | Default | Context | Destructive |
|---|---|---|---|---|
| `go_to_inbox` | Go to inbox | `g i` | list surfaces | no |
| `go_to_drafts` | Go to drafts | `g d` | list surfaces | no |
| `go_to_sent` | Go to sent | `g t` | list surfaces | no |
| `go_to_flagged` | Go to flagged | `g s` | list surfaces | no |

## Guarantees

1. **The ids are stable.** They may gain bindings; they may not be renamed.
2. **Every one has a non-empty default binding**, which `command_registry.rs` asserts for every built-in.
3. **Every one is rebindable** from `[keys]`, replacing the primary binding.
4. **Every one appears** in the palette, the cheat sheet and `docs/keybindings.md`, because all three are generated from the registry.
5. **None collides** with `g g`, `g f` or `g a`.
6. **Going somewhere destroys nothing**, so none is `destructive` and none needs `Recovery`.

## Not in this contract

Archive, Junk, Trash and Snoozed have no id here. Archive awaits a letter from
the design authority (see [../research.md](../research.md) R4); the other three
are reached through `#` in the bar.
