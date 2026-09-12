# Contract: the bar's mode table

**Feature**: [../spec.md](../spec.md) | **Consumers**: the GTK bar's hint, the
`?` cheat sheet, the generated documentation, the macOS frontend

The bar answers several questions, and a character chooses which. That set is a
product decision, so it lives in `postio-ui` and every surface reads it from
there — the same reason the palette matcher moved in #658 and the search chips
in #1157.

## The table

| Prefix | Marker | Purpose |
|---|---|---|
| *(none)* | `/` | Search all mail |
| `>` | `>` | Run a command |
| `#` | `#` | Go to a folder |
| `+` | `+` | Add a label |
| `@` | `@` | Find a correspondent |

The purposes are the strings `Mode::placeholder()` already returns, verbatim —
this table is a lift of what ships, not a rewording of it.

The prefixes are what ships and are **not** being changed by this feature.

## Guarantees

1. **One enumeration.** The bar's hint, the cheat sheet and the documentation all read this table. A mode added later appears in all three from one edit — FR-032, SC-010.
2. **Prefixes are unique**, and exactly one mode has none.
3. **Marker and purpose are user-facing text**, in a user's words, not identifiers. There is deliberately no separate `name`, because nothing would render it — a hint lists the prefix and what it does, and so does the documentation.
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
