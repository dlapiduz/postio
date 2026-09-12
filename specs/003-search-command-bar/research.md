# Phase 0: Research — Search and Command Bar

**Feature**: [spec.md](./spec.md) | **Plan**: [plan.md](./plan.md) | **Date**: 2026-09-11

Everything here was settled by reading the code and the constitution. No
`NEEDS CLARIFICATION` remains.

---

## R1. What "already built" actually covers

**Decision**: Treat search, commands and folder-jumping as shipped, and verify
the wiring rather than the widget before building anything on top.

**Rationale**: `postio-gtk/src/finder.rs` is one box with five modes — search
(no prefix), `>` command, `#` mailbox, `+` label, `@` contact — and a prefix
typed into an empty box is absorbed and becomes the mode. `#` is fed by
`window.rs:1255`, where `folders.connect_loaded` calls
`finder.set_mailboxes`; that signal fires in `feed.rs:973`, on the same path
and from the same list that calls `sidebar.set_mailboxes`. Activation reaches
`window.rs:1278`, which resolves the pending-move question and otherwise calls
`sidebar().select(id)` and `show(id)`. Every link exists.

**Alternatives considered**: Taking the request at face value and building
folder navigation. Rejected on evidence — it would have duplicated a shipped
path and left the actual defect (nobody can find it) in place.

---

## R2. Why the capability is invisible

**Decision**: Name the cause as structural, not cosmetic: the prefixes are not
in `postio-core::registry`, and every discovery surface is generated from it.

**Rationale**: Constitution II requires the keymap, the palette, the `?` cheat
sheet, the context menu, the key hints and `docs/keybindings.md` to derive from
one table, and says *"a command that is not in the registry does not exist —
not merely unbound, but absent from every way a user could discover it."* A
prefix is in the registry's reach nowhere, so every surface generated from it
misses the modes unless something adds them by hand.

**Corrected during implementation.** This first said the modes reached *no*
surface. One had already been fixed by hand: `cheatsheet.rs::prefix_section`
lists them under "In the search box", shipped under `postio-2ee`. What was
missing was `docs/keybindings.md` and the bar itself — and the fact that a
cheat-sheet section alone did not stop the project's own maintainer asking
for a mode that was in it. A hand-added section is also exactly the drift
this research argues against: it is a second place the truth lives, and it
happened to be right. Both now read the one table.

**Alternatives considered**: Documenting the prefixes in `keybindings.md` by
hand. Rejected — that file is generated and a test fails when it drifts, so a
hand-written section is both against the grain and unenforceable.

---

## R3. One command per destination, or one command with a parameter

**Decision**: One `CommandId` per destination role.

**Rationale**: `[keys]` in `config.toml` is keyed by command id, and the keymap
resolves a binding to an id. One parameterised id — `GoTo { role }` — could
therefore carry only one binding, which cannot give `g i` and `g d` different
meanings. `Move { to: Option<MailboxId> }` is not a counter-example: its
payload is *answered* by a picker, not chosen by which key was pressed.

**Alternatives considered**: `GoTo { role }` with `alternate_bindings`.
Rejected: alternates are more keys for the *same* command, so every sequence
would go to the same folder.

---

## R4. Which roles get a sequence

**Decision**: Four destinations get one — `g i` inbox, `g d` drafts, `g t`
sent, `g s` flagged. Archive is recommended but left for the design authority
to letter. Junk, Trash and Snoozed get none and stay reachable through `#`.

**Rationale**: `core_suite/command_registry.rs` asserts every built-in command
has a **non-empty** default binding, so "in the registry but palette-only" is
not available to a built-in — every destination command added here must be
given a real sequence. That makes each one a cost, and the `g` space is
already partly spent: `g g` is the first message, `g f` focuses the folder
list, `g a` is the next scope. Four sequences carry straight over from the
convention users arrive with (`i`, `d`, `t`, `s` mean the same there), so each
is justified by something other than taste. Junk, Trash and Snoozed are rare
enough that a strained letter would cost more than it returns, and R5 makes
the route they do have findable.

**Archive is the one with no natural letter.** `g a` would be the convention's
choice and is taken; Postio's own archive verb is `a`, and `e` — the other
obvious hint — already means reply here. `g r` is the recommendation, on the
sole strength of the letter appearing in the word. It is called out in
[spec.md](./spec.md) as a design call, and this is the note that says why it
has no good answer rather than pretending one was found.

**Alternatives considered**: A sequence for all eight roles (rejected: three
strained bindings for destinations people rarely visit, and `g` space spent
that a later feature will want). Rebinding `g a` to the archive for
convention-parity (rejected: it is "next scope" here and "all mail" there —
not even the same destination, so the trade loses a real command for an
imperfect match; spec.md records this).

---

## R5. Where the mode table lives

**Decision**: Move the mode enumeration — prefix character, name, and what the
mode is for — into `postio-ui`, and re-export it from `postio-gtk::finder`.

**Rationale**: The modes are a product decision: which questions the box can
answer, and which character asks each. ADR 0019 forbids the macOS frontend
re-deriving those, and this is the third instance of exactly that move — the
palette matcher went to `postio-ui` in #658 and the search chips in #1157, both
because a second frontend would otherwise grow a second answer. `postio-ui`
carries no GTK dependency, which `check-crate-boundaries.py` enforces, and the
table is plain data. The strings already exist inside `finder.rs`
(`Mode::Mailbox => '#'` and `Mode::Mailbox => "Go to a folder"`); they are
being lifted and given a public shape, not invented.

**Alternatives considered**: Leaving `Mode` in `postio-gtk` and having the
docs generator read it. Rejected — it would make a docs test depend on a GTK
crate, and would leave Swift to re-derive the table, which is the drift ADR
0019 exists to prevent.

---

## R6. How the modes reach documentation without a second list

**Decision**: Generate a modes section into `docs/keybindings.md` from the
`postio-ui` table, tested the way the bindings already are.

**Rationale**: `core_suite/keybindings_doc.rs` already regenerates that file
from the registry and fails when it drifts, so the pattern, the file and the
enforcement all exist. A second generated section from a second table is the
same mechanism pointed at the one thing the registry cannot describe. This is
what satisfies FR-032 and SC-010: a sixth mode added later appears in the bar's
hint and the documentation from the one edit.

**Alternatives considered**: A separate `docs/finder.md`. Rejected — a user
looking for "how do I get to my inbox" goes to the keyboard documentation, and
a second file is a second place to not look.

---

## R7. Which inbox `g i` reaches when there are several accounts

**Decision**: Follow whatever the sidebar's current scope already resolves; do
not invent a second answer here.

**Rationale**: Constitution III and the account model already settled this —
the tri-tab scope decides whose mail is in view, and `g a` (next scope) is how
a user changes it. A destination command that picked its own account would be
a second, competing answer to a question the app has already answered, and
would surprise a user who had just set the scope deliberately. `feed.rs:1198`
already resolves the inbox by role this way for the window's first folder.

**Alternatives considered**: Always the default account's inbox (rejected:
ignores a scope the user set); a unified destination spanning accounts
(rejected as new product surface, not asked for, and `in:` already composes
with account scope for the querying case).

---

## R8. How the feature gets tested, given how it was missed

**Decision**: The acceptance test presses a key at the composition root and
asserts on the folder a person is then looking at. `app_suite` is the home.

**Rationale**: Constitution IV: *"tests MUST assert on what a person would see,
not on what a layer was handed."* The existing `gtk_finder.rs` case hands the
finder a fixture via `set_mailboxes` and asserts a handler fired — which would
pass unchanged if nothing in the application ever called it. That is the shape
of defect this codebase keeps producing, and it is why a shipped feature could
be invisible for long enough to be re-requested. `app_suite` exists precisely
to prove a change reaches the running app, and `keystroke.rs` already drives
keys into it.

**Alternatives considered**: Extending `gtk_finder.rs` only. Rejected as
insufficient on its own — it is the layer test, and it cannot fail for the
reason this feature exists.
