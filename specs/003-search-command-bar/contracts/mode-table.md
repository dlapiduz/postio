# Contract: the bar's mode table

**Feature**: [../spec.md](../spec.md) | **Consumers**: the GTK bar's hint, the
`?` cheat sheet, the generated documentation, the macOS frontend

The bar answers several questions, and a character chooses which. That set is a
product decision, so it lives in `postio-ui` and every surface reads it from
there — the same reason the palette matcher moved in #658 and the search chips
in #1157.

## The table

| Prefix | Name | Purpose |
|---|---|---|
| *(none)* | Search | Search all mail |
| `>` | Command | Run a command |
| `#` | Mailbox | Go to a folder |
| `+` | Label | Label the selection |
| `@` | Contact | Find a correspondent |

The prefixes are what ships and are **not** being changed by this feature.

## Guarantees

1. **One enumeration.** The bar's hint, the cheat sheet and the documentation all read this table. A mode added later appears in all three from one edit — FR-032, SC-010.
2. **Prefixes are unique**, and exactly one mode has none.
3. **Name and purpose are user-facing text**, in a user's words, not identifiers.
4. **No GTK.** The table is plain data in `postio-ui`, which `check-crate-boundaries.py` keeps free of toolkit dependencies, so the macOS frontend consumes it rather than re-deriving it.
5. **A mode that cannot act where the user stands is not advertised there** — FR-033.

## Why this is not the registry

A prefix is not an invocation: it selects which question the box is asking,
then the user keeps typing. It has no context predicate of its own, nothing to
undo, and no meaning outside the box. Putting it in `postio-core::registry`
would give every mode a fake binding and a fake command — and the registry's
own test asserts every command has a real one. So the modes get the same
single-table treatment in the one place they can have it, which is what
[../research.md](../research.md) R2 argues.
