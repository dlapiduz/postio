# Implementation Plan: Search and Command Bar

**Branch**: `feature/search-command-bar` | **Date**: 2026-09-11 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/003-search-command-bar/spec.md`

## Summary

Three of the spec's five stories already ship. The work is the other two, and
they turn out to be one mechanism.

The bar can go to a folder — `#` in the box, wired through to the same folder
list the sidebar draws. Nobody can find it, because a prefix inside a text
field is not a command in `postio-core::registry`, and the registry is what
generates every surface a user could discover something on: the keymap, the
palette, the `?` cheat sheet, the context menu, the key hints, and
`docs/keybindings.md`. Constitution Principle II names this exact failure —
*"a feature is not complete when its widget works: it is complete when its
command is in the registry"* — and the bar's modes are the case it warned
about.

So the approach is to make the destinations **commands**, not to add a hint
and a chord separately:

1. **One `GoTo…` command per mailbox role**, in the registry, bound to the
   `g` sequences the maintainer asked for (`g i` and family). This is what
   delivers User Story 5, and it delivers most of User Story 4 for free — a
   registry command appears in the palette, the cheat sheet and the generated
   docs without anyone writing a second list.
2. **Lift the bar's mode table into `postio-ui`**, where the palette matcher
   and the search chips already live, and render the hint and a generated
   documentation section from it. This is the rest of User Story 4: the four
   prefixes are affordances rather than commands, so they need their own one
   enumeration rather than a place in the registry.

Nothing about the bar's behaviour changes. No mode is added, removed or
re-keyed, and no query means anything new.

## Technical Context

**Language/Version**: Rust, pinned to 1.98.0 by `rust-toolchain.toml`

**Primary Dependencies**: GTK4 + libadwaita in `postio-gtk`; no new dependency is
introduced by this feature

**Storage**: SQLite via `postio-storage`. This feature performs no new query:
role resolution reads the mailbox list the feed already holds in memory

> Engine changed after this landed: see `specs/004-turso-store`.

**Testing**: `cargo test --lib` for units; `cargo nextest run` for integration
suites — `core_suite` (registry and generated docs), `gtk_suite` (the bar),
`app_suite` (the composition root, which is what proves a key reaches a folder)

**Target Platform**: Linux desktop (Fedora 40+). `postio-core` and `postio-ui`
are shared with the macOS frontend per ADR 0019 and must not gain GTK

**Project Type**: Desktop application; a Cargo workspace of 20 crates

**Performance Goals**: Interaction < 16 ms — pressing `g i` and drawing the hint
both sit inside it. Local search < 100 ms, unchanged by this work

**Constraints**: The registry is the single enumerable table (Principle II).
`postio-ui` must not depend on GTK and `postio-gtk` must not contain SQL or
protocol code (Principle VII). Every binding stays overridable from `[keys]`,
which makes command ids a file format

**Scale/Scope**: ~78 command ids today, 5 bar modes, 8 mailbox roles of which
the sidebar shows all 8

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design.*

| Principle | Verdict | Note |
|---|---|---|
| I. Local-first, UI never awaits network | **Pass** | Going to a folder is a local read of a list already in memory; nothing here reaches the network |
| II. The keyboard is a system | **Pass — and this feature is the remedy** | See below |
| III. Search is navigation, one query language | **Pass** | No operator is added and no second language appears. `in:drafts` filters, `g d` navigates; these are different acts and the spec keeps them apart |
| IV. Test-first | **Pass, with a named hazard** | See below |
| V. Performance is a functional requirement | **Pass** | No read path changes, so there is no new counting assertion to carry. The hint must draw inside the 16 ms interaction budget |
| VI. Privacy is a feature | **Pass** | No network, no new logging. Folder names stay untrusted text and keep the escaping `palette::highlight` already applies |
| VII. Boundaries are enforced | **Pass, and tightened** | The mode table moves to `postio-ui`, which is where a product decision the macOS frontend must share belongs |

**On Principle II.** The current state is the situation the principle exists to
prevent. Whether a *prefix* is literally "a command" is arguable — it is an
affordance inside a text field, not an invocation — but the consequence the
principle names has happened regardless: a working capability is *"absent from
every way a user could discover it"*, which is how it came to be requested as
new work by the person who owns the project. This plan does not merely comply;
it closes the hole. Destinations become registry commands and therefore inherit
every discovery surface, and the affordances that genuinely cannot be commands
get the same single-table treatment in the one place they can.

**On Principle IV.** The hazard is specific and this codebase has been bitten by
it. `gtk_finder.rs` proves `#` jumps to a folder by calling
`finder.set_mailboxes(&folders())` with a fixture and asserting a handler fired.
That is an assertion about what a layer was handed, and it would pass unchanged
if nothing in the real application ever called `set_mailboxes` — which is
exactly the class of defect Principle IV's second paragraph describes. The tests
for this feature MUST press a key at the composition root and assert on the
folder a person would then be looking at. `app_suite` is where that belongs.

**No violations to justify.** The Complexity Tracking table is therefore omitted.

## Project Structure

### Documentation (this feature)

```text
specs/003-search-command-bar/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
│   ├── command-ids.md   # The new ids and bindings, as `[keys]` spells them
│   └── mode-table.md    # The bar's modes, as the shared enumeration
└── tasks.md             # Phase 2 output (/speckit-tasks — not created here)
```

### Source Code (repository root)

```text
crates/
├── postio-core/
│   ├── src/command.rs          # CommandId: one id per destination
│   ├── src/registry.rs         # CommandSpec: title, `g` binding, contexts
│   └── tests/core_suite/
│       ├── command_registry.rs # the registry's own invariants
│       └── keybindings_doc.rs  # regenerates docs/keybindings.md, fails on drift
├── postio-ui/
│   └── src/finder_modes.rs     # NEW: the mode table, shared with macOS
├── postio-gtk/
│   ├── src/finder.rs           # Mode re-exported from postio-ui; renders the hint
│   ├── src/window.rs           # act(): resolve the role, select, show
│   ├── src/cheatsheet.rs       # gains the modes section
│   └── tests/gtk_suite/
│       ├── gtk_finder.rs       # existing; the hint's own behaviour
│       └── gtk_cheatsheet.rs   # existing; the modes appear
├── postio-app/
│   └── tests/app_suite/        # NEW case: a key press reaches a folder
└── docs/keybindings.md         # generated; gains the destinations and a modes section
```

**Structure Decision**: No new crate. The work lands in four existing ones,
split by the boundary rule that already governs this bar: what decides
*meaning* (which destinations exist, what the modes are and what they are for)
goes in `postio-core` and `postio-ui` so the macOS frontend inherits it; what
decides *drawing* stays in `postio-gtk`. That is the same split #658 made for
the palette matcher and #1157 made for the search chips, and this feature is
the third instance of it rather than a new idea.
