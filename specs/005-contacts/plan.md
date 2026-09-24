# Implementation Plan: Contacts

**Branch**: `feature/contacts` | **Date**: 2026-09-23 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/005-contacts/spec.md`

## Summary

Make a contact a **person who owns addresses**, and give the address book the
screen it has never had. The store gains an owner column on the existing
`addresses` table, a per-account `contact_sightings` table for the mail's
evidence, and a person-shaped `contacts` table with a `live | deleted |
merged` state that makes deletion, restore and exact undo one mechanism
(research R1–R4, R8). Sync keeps recording sightings on first insert and now
also counts who the user wrote to (R3). A Contacts pane occupant with its own
keyboard context takes over the reading pane, windowed over keyset pages and
filtered through a term index so 20,000 people stay inside the budgets (R5,
R10). The user's name for a person replaces the header name in storage's three
sender reads, keyed by address (R7). One search field, `with:`, carries a
person's written-out addresses (R6). vCard import/export adopts Pimalaya's
`vcard-rs` in a new leaf crate (R9). ADR 0007 is folded into the spec and
deleted (R12).

## Technical Context

**Language/Version**: Rust, pinned by `rust-toolchain.toml` (edition 2024)

**Primary Dependencies**: GTK4 + libadwaita (`postio-gtk`), Turso (the store,
ADR 0038), **new: `vcard-rs =0.4.0`** (Pimalaya; `default-features = false`,
parser + encodings) in a new `postio-vcard` crate

**Storage**: the encrypted Turso store; `crates/postio-storage/src/schema.rs`
`HEAD` changes (data-model.md); existing stores are rebuilt by resync (R13)

**Testing**: cargo nextest (integration suites), `cargo test --lib` (unit),
`app_suite` custom harness, `gtk_suite` headless, `test_support::counting` for
budgets

**Target Platform**: Linux desktop (GTK4/libadwaita, Wayland first);
`postio-ffi` mirrors the new context for the macOS frontend

**Project Type**: desktop application (Cargo workspace)

**Performance Goals**: interaction < 16 ms, local search/filter < 100 ms,
startup unaffected (< 500 ms) — gated as counts

**Constraints**: no network, ever (FR-060); logs carry ids and counts only
(FR-061); the UI never awaits the network; a store write, event, repaint for
every edit; never load the whole list

**Scale/Scope**: 20,000 distinct correspondents (SC-001); 22 new commands, one
context, one pane occupant, one search field, one crate, six new/changed
tables

## Constitution Check

*GATE: checked before Phase 0 and again after Phase 1 design.*

| Principle | How this plan satisfies it | Status |
|---|---|---|
| I. Local-first | Every contacts mutation is a `Command` handled in `postio-session` as store transaction → `ContactsChanged` → repaint; no remote half exists (CardDAV out of scope). The one destructive command (`contact_delete`, which deletes a person or a group by the focused row) is `Recovery::Undo`, and so is every structural edit (R8) | ✅ |
| II. Keyboard is a system | 22 commands, each with a default binding, in `postio-core::registry` with bindings, palette entries and cheat-sheet rows (contracts/commands.md); `Context::Contacts` joins every generated surface; list movement/selection reuse existing commands so cursor ≠ selection holds; `docs/keybindings.md` regenerates | ✅ |
| III. One query language | `with:` is one `Field` + one executor arm in the existing language (contracts/query-with.md); no person-level or second language; half-typed values never error | ✅ |
| IV. Test-first | Every task names its failing test; tests assert what a person sees — the rendered list row's name, the reader header, the completion popover's rows — not what a layer was handed (quickstart.md) | ✅ |
| V. Performance | Keyset paging, covering indexes per view, term index for filtering; counting assertions on each contacts read path, on sync's statements per address, and "unchanged" on the message-list page (R2, R5, R7) | ✅ |
| VI. Privacy | No avatars/favicons/directory lookups (initials only); logs carry counts; fixtures use reserved domains; vCard files are read only when the user picks one | ✅ |
| VII. Boundaries | `postio-gtk` gets rows through app/runtime, never SQL; `postio-core`/`postio-session` stay GTK-free; new `postio-vcard` leaf gets a boundary check (no DB, gtk4, tokio); **Pimalaya first** was surveyed 2026-09-23 and adopted (R9) | ✅ |
| No backwards compatibility | Schema reshaped in `HEAD`; stores rebuilt by resync; no user-made contacts exist to lose (R13) | ✅ |
| One fact, one home | ADR 0007 folded into the spec and deleted, citations re-pointed (R12); no parallel ADR | ✅ |

**Post-design re-check (after Phase 1):** unchanged — all pass. Two things
changed during design and are recorded rather than hidden:
- **A vCard dependency is now taken**, reversing ADR 0007 Q4, because a
  Pimalaya crate now meets the round-trip need (R9). The spec's inherited
  bullet and FR-051/SC-006 are updated to match.
- **`with:` is added to the query language**, because the clarified "written
  out addresses" cannot otherwise mean "from or to any of them" (R6).

No violations; Complexity Tracking is empty.

## Project Structure

### Documentation (this feature)

```text
specs/005-contacts/
├── spec.md
├── plan.md              # this file
├── research.md          # R1–R13
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── commands.md      # context, command ids, payloads, events, undo kinds
│   ├── query-with.md    # the with: field
│   └── vcard.md         # import/export mapping
├── checklists/requirements.md
└── tasks.md             # /speckit-tasks
```

### Source Code (repository root)

```text
crates/
├── postio-model/src/
│   ├── contact.rs              # Contact → person; ContactAddress, ContactState
│   ├── contact_group.rs        # drops account_id
│   └── ids.rs                  # + AddressId
├── postio-storage/src/
│   ├── schema.rs               # contacts, addresses.contact_id, contact_sightings,
│   │                           # contact_terms, join candidates/dismissals; indexes
│   ├── repository/contacts.rs  # rewritten: record, list pages, filter, detail,
│   │                           # join/unjoin, move/detach, delete/restore, suggestions
│   ├── repository/contact_groups.rs
│   ├── repository/messages.rs  # LIST_COLUMNS + read_recipients name substitution
│   └── repository/threads.rs   # participants_for name substitution
│   tests/storage_suite/        # contacts*.rs, contact_rank_index.rs, budgets
├── postio-sync/src/contacts.rs # written-to, reply candidates; own-address detection first
├── postio-search/src/          # Field::With, Filter::With, parser
├── postio-index/src/executor.rs# with: arm; group: via addresses.contact_id; affinity via sightings
├── postio-vcard/               # NEW leaf crate: parse/export over vcard-rs
│   └── tests/corpus/
├── postio-core/src/            # Context::Contacts, command ids, SPECS rows, Command payloads,
│                               # UndoKind, menu sections, Event::ContactsChanged
├── postio-session/src/actions.rs # handlers + inverses
├── postio-runtime/src/store/   # contacts page/detail reads for the app
├── postio-ui/src/
│   ├── list.rs                 # ListWindow generalised over a key type
│   ├── contacts.rs             # NEW toolkit-free: view/filter state, display rules, join naming
│   ├── keymap.rs, cheatsheet.rs, focus.rs
├── postio-gtk/src/
│   ├── contacts/               # NEW: pane occupant, list view, detail, editor, join/name dialog
│   ├── shell.rs                # ReaderOccupant::Contacts, fallback rank
│   ├── sidebar.rs              # "Contacts" row
│   ├── finder.rs               # persons; writes with:
│   ├── composer.rs             # completion rows: one name, addresses beneath
│   └── window.rs               # handled_here arms, focus controller
│   tests/gtk_suite/            # contacts_* tests, registered in main.rs
├── postio-ffi/src/registry.rs  # UiContext::Contacts
└── postio-app/src/
    ├── contacts.rs             # NEW install(): pane, feeds, import/export file dialogs
    ├── compose.rs, search.rs   # completion + finder over persons
    └── lib.rs                  # install order
    tests/app_suite/            # contacts_* cases, registered in CASES
docs/
├── decisions/0007-address-book.md   # DELETED (folded), README row removed
├── PRODUCT.md                        # scope lines updated
└── keybindings.md, config.md         # regenerated
scripts/checks/                       # postio-vcard boundary rule
```

**Structure Decision**: the existing workspace layout, with one new leaf crate
(`postio-vcard`) kept off `postio-model`'s compile path (R9). Toolkit-free
rules go in `postio-ui::contacts` so they are provable in milliseconds; the
widget only draws them.

## Phasing (for `/speckit-tasks`)

Ordered so every commit is green and `main` is never half-migrated — this
lands as one PR, but the order is what keeps the branch bisectable.

1. **Foundation — the person model** (blocks everything): schema, model
   types, repository record path with the sighting table and written-to,
   sync call sites, the existing completion/finder/`group:`/affinity readers
   moved onto persons with their current tests kept green, fixtures updated.
   Budget assertions on the write path.
2. **Story 1 — the screen (P1)**: `ListWindow` generalisation, contacts pages
   and filter reads with budgets, `Context::Contacts` and the navigation
   commands, pane occupant with the Esc contract, sidebar row, `g c`,
   detail view, `with:` and "show mail"/"compose to", the default/everyone
   views.
3. **Story 2 — join (P1)**: join with naming, detach, add/move address,
   preferred address, undo for each; completion shows one name; **FR-032**
   name substitution in storage and the refetch on rename.
4. **Story 3 — create/edit/delete (P2)**: create, edit, promotion, delete,
   restore and the Deleted view.
5. **Story 4 — suggestions (P3)**: `name_key`, reply candidates at sync,
   dismissals, the suggestions view.
6. **Story 5 — groups (P3)**: group management surface, expansion to
   preferred addresses, `group:` through owners.
7. **Story 6 — vCard (P4)**: `postio-vcard` crate + boundary check + corpus,
   import/export commands and file dialogs, summary.
8. **Polish**: fold ADR 0007 (delete, README, 36 citations, PRODUCT.md),
   regenerate docs, `/gtk-design` render check, quickstart walk.

## Complexity Tracking

No constitution violations to justify.
