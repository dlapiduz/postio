# Implementation Plan: Contacts for Postio Focus

**Branch**: `feature/focus-contacts` | **Date**: 2026-10-19 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/011-focus-contacts/spec.md`

## Summary

Contacts becomes a mode of Focus's main window, with a person's page in the
secondary window, built for the Mac and ready for Linux.

1. **Wait for the data, then add no data.** Spec 005's store, session and
   command layers land on `main` from `feature/contacts-mac` (another
   session's work). This feature adds **no schema** and no store rule. Its
   Rust work is the controller (`postio-focus`), the words (`postio-ui`),
   the registry rows (`postio-core`), and the FFI exports (`postio-ffi`).
2. **Behaviour in `postio-focus`** (ADR 0045): a `contacts` module holds the
   mode, tabs, box filter, cursor and selection, the page's state, the
   pickers' contents, the Esc ladder, History entries and the cursor rules
   after join, delete and undo. Behind `Policy.caps.contacts_view`, true on
   the Mac, as 010 did for `results_view`.
3. **The Mac draws it.** AppKit holds the people table and the page window;
   SwiftUI holds their content; panels are `NSOpenPanel`/`NSSavePanel`.
   `PostioKit` holds the view models and has no AppKit.
4. **Linux adopts later**, in slices G1 to G3, with no controller change.

Built in the slices below, each runnable and committed on one branch, one PR
reviewed against the spec (constitution, Development Workflow), Linux CI
proving that every shared change was additive (010 S2).

## Technical Context

**Language/Version**: Rust 1.99.0 (`rust-toolchain.toml`; on this Mac
`unset RUSTUP_TOOLCHAIN MAKEFLAGS` first); Swift tools 6.0

**Primary Dependencies**:
- **Rust:** `postio-focus`, `postio-ui`, `postio-core`, `postio-client`,
  `postio-session`, `postio-ffi` (UniFFI 0.32). Spec 005's `postio-vcard`
  (Pimalaya `vcard-rs`) is *consumed* through the client, never linked by an
  interface. No new dependency.
- **Swift:** AppKit, SwiftUI, UniformTypeIdentifiers (the `.vcf` type for the
  panels), Contacts (unchanged, completion only).

**Storage**: none new. Reads and writes are 005's. If the port's reads do not
serve a surface below, the smallest additional read is added **in the port's
crates** and reported (see "What this needs from 005's port").

**Testing**:
- **Rust:** `cargo test -p postio-focus` (rule tests, both hosts);
  `cargo test -p postio-ui` (words, `keybindings_doc`);
  `cargo nextest run -p postio-core` (registry rules incl. collisions);
  `cargo nextest run -p postio-ffi --test ffi_suite` (new `focus_contacts.rs`);
  counts via `postio_storage::test_support::counting` on every read path.
- **Swift:** Swift Testing via `scripts/macos-test.sh`: view models and key
  paths without a window.
- **Storyboards:** `storyboards/contacts/*.toml`, written red before each
  slice builds (CLAUDE.md); the Mac runner is out of scope (009 FR-063), so
  Mac landings are labelled `interactions-unreviewed` until slice G3.
- **Screens:** `scripts/macos-shot.sh` at 1440×900, light and dark, over the
  demo store (a `small:30` seed with invented contacts, slice 1).

**Target Platform**: macOS 14+ (arm64). Linux stays green on every commit;
GTK cannot be built on this Mac, so every shared change is additive.

**Project Type**: desktop app (Rust engine + native Swift frontend)

**Performance Goals**: first rows < 16 ms after `g p` with 20,000 contacts,
key to frame < 16 ms, filter < 100 ms, no transition on mode, tab or
selection changes (constitution V)

**Constraints**: local-first (no verb awaits the network); no network, no
avatars; logs carry ids and counts only; `PostioKit` imports no AppKit; the
controller does no I/O; no worktree path in anything rustc sees

**Scale/Scope**: 20,000 contacts; ~30 registry commands, 2 contexts, 1 mode,
1 window kind, 4 pickers, 1 sheet; about 20 Swift files

## Constitution Check

| Principle | How this plan meets it | Status |
|---|---|---|
| I. Local-first | Every verb is a 005 command: store write, event, repaint, undo entry; none awaits a reply from the network (there is no remote half). Panels are modal only for the system's file choice | Pass |
| II. One command table | Every command is a `CommandId` with `SPECS` row, palette entry and menu section; bindings differ from 005's proposals, ids do not. Mac-only through `offered_on`. `keybindings.md` regenerates. Collisions checked in the spec and asserted by `bindings_do_not_collide_within_a_context` | Pass |
| III. One query language | Show mail opens an ordinary query (005's `with:`); the people filter reuses the box's `@` mode; no filter field or second language | Pass |
| IV. Test-first | Each slice lists its red tests first (below); storyboards before building; Swift tested without a window | Pass |
| V. Performance | Windowed pages over 005's keyset reads; count assertions on list page, filter, detail and suggestions reads; wall-clock bench reported, not gated | Pass |
| VI. Privacy | No network, no avatars (initials); a file is read only after the person chose it in a panel or dropped it; system Contacts read only for completion; logs carry counts; fixtures use `example.com` and invented names | Pass |
| VII. Boundaries | The controller is toolkit-free and does no I/O; the FFI stays the only Swift boundary; `PostioKit` has no AppKit; the interface links no vCard parser or store engine. `check-no-silent-tracking.py` scans the new Swift: panels carry a `POSTIO-CONSENT:` comment naming the person's choice | Pass |
| Workflow | One spec-driven feature branch, one PR, a commit per task ending `Refs: specs/011-focus-contacts` | Pass |

Re-check after Phase 1: unchanged.

## No ADR

The rule that outlives the feature, behaviour in `postio-focus`, is ADR
0045. The decisions K1 to K12 belong to this feature and live in the spec.

## What this needs from 005's port

The port (`feature/contacts-mac`) is not touched by this branch. This
feature depends on the following, and each is a place to confirm before
slice 1, in a comment on the port's PR, not an edit to it:

| Need | 005 gives | Check |
|---|---|---|
| Pages of contacts by tab (written-to, everyone, deleted), with a filter, in name order with letter headers | R5 keyset pages, term index | Page key is `sort_key`; a "letter header" is derived from `sort_key`'s first char, which the controller computes |
| A contact's detail: addresses with counts and last seen, groups, note, extra vCard properties, recent conversations | `contacts` detail read | **Recent conversations** (five, newest) may not exist: a small read over `with:`'s executor |
| Suggestions with evidence | join candidates | Evidence fields: shared name, each address's message count |
| Groups with member counts; members pages | groups repo | Member count in the list read |
| Commands | `contracts/commands.md` payloads and undo kinds | `ImportVcard` as one undo entry (Q3), a `Join` that takes the chosen name and organisation |
| Command ids | 22 ids | This spec adds 8 and rebinds many: see the spec's command table. If the port has already registered 005's bindings, rebinding is a one-line change per row there or here; coordinate on the port's PR to avoid two edits to `registry.rs` |
| Name substitution in mail reads | R7 | Nothing to do; the FFI rows already carry the substituted name |

## Design by crate

### `postio-core`

- `CommandId` rows for the spec's command table; `Context::Contacts` and
  `Context::ContactCard`, added to every exhaustive match (005's list:
  `postio-ui/src/keymap.rs`, `cheatsheet.rs::heading`,
  `postio-ffi/src/registry.rs`, `keybindings_doc.rs::where_available`,
  `postio-ui/src/focus.rs`).
- `offered_on`: every Contacts command is false on Freedesktop until slice
  G1; `next_message`-family commands gain the two contexts for Apple only
  (010 D25), so Linux's keymap rows are unchanged.
- A test that every Contacts command has a title, a binding and a palette
  entry, and that none collide within `Contacts` or `ContactCard`
  (the registry's existing generic tests do this once the rows exist).

### `postio-ui` (words)

Titles, hints, footers, empty-state sentences (the six in the spec), the
pill texts ("Joined 2 contacts", "Deleted 3 contacts", "Added 2 contacts to
Family"), the refusals ("A contact needs at least one address · # to
delete the contact"), the Move popover sentence, the import summary lines,
and the key spellings. Pure functions, tested as the other words are. The
page's geometry is `focus_dialog`'s, keyed by platform.

### `postio-focus` (the controller)

New `contacts.rs` (and `contact_card.rs`), toolkit-free, no I/O:

- **State**: `ContactsMode { tab, filter, group: Option<GroupId>, cursor,
  selection, window }` and `CardState { contact, row_cursor, editing:
  Option<Field>, mode: View | New | Deleted }`. The list is a `ListWindow`
  of pages, one resident copy, as the feed is.
- **Rules** (each a red test first): tab switches keep each tab's cursor;
  `Esc` ladder; the verb acts on marks else cursor; cursor after delete,
  join and their undo; `m` on one contact asks for the other; the join
  picker's candidates and preselection; conflict organisation second step;
  `n` in Groups vs elsewhere; `-` on a pair vs an address; refusals;
  empty-state kind per tab and filter; `@` from a message row picks the
  sender's contact; `/` opens the box in people mode and routes typing to
  the filter; History entries on entry and exit.
- **Intents** to the app: show the Contacts mode, show a page, open a
  picker at a row, show the Move popover, show the import summary, present
  the open or save panel, show a pill. The panels follow the "intercepted"
  pattern: the controller decides *that* a panel is wanted; Swift presents it.
- **Requests** to the host (`perform.rs`): the 005 commands and reads,
  through `postio-client`.
- **Policy**: `Policy.caps.contacts_view`.

### `postio-ffi`

New `focus_contacts.rs` (mirrors in the `*Ffi` records of 009's pattern):

- reads: a page of rows (kind person, group or pair), the page's detail,
  counts for the header strip and the tabs, import summary;
- events: `UiEvent` cases for the new surfaces and for `ContactsChanged`
  (list refetch, as for mail events);
- `focus_contact_*` commands go through `invoke`; panels return their
  chosen URLs through one export (`focus_contacts_files_chosen`), never a
  path decision in Swift.
- `INTERCEPTED` gains `contacts_import` and `contacts_export`, and
  `theInterceptedListsAgreeAcrossTheBoundary` is updated in the same commit.
- `recipient_suggestions` (009 R11) becomes person-shaped (slice 8).

### Swift

| Target | New or changed |
|---|---|
| `PostioKit` | `ContactsModel.swift` (rows, tabs, header strip, empty states), `ContactCardModel.swift`, `ContactPickerModel.swift` (join name, group toggle, move), `ImportSummaryModel.swift`, `ContactsIntents.swift` (cases of `FocusIntents`) |
| `PostioAppKit` | `ContactsTable.swift` (the people table over `FocusListTable`'s machinery, letter headers, columns that drop), `ContactCardWindowChrome.swift` (the secondary window; the message window's sizing), `ContactPickerPopover.swift`, `MoveAddressPopover.swift`, `ImportSummarySheet.swift`, `ContactsPanels.swift` (`NSOpenPanel`/`NSSavePanel`, drop target; `POSTIO-CONSENT:` comments), `MenuBar.swift` (Go ▸ Contacts and the Contacts menu from the registry) |
| `Postio` | `MainWindow.swift` (Contacts mode in the content area, drop target), `Engine.swift` (events) |
| Tests | `PostioKitTests/ContactsModelTests.swift`, `ContactPickerTests.swift`, `ContactsKeyboardTests.swift`; `PostioAppKitTests/ContactsTableTests.swift`, `ContactCardGeometryTests.swift` |

Neutral initials use the system secondary label colour on a quaternary
fill; the accent is reserved (spec FR-084).

## Slice plan

Each slice is runnable, committed on `feature/focus-contacts`, with its
storyboards and red tests written first. The Mac slices are 1 to 8; GTK and
the terminal are G1 to G3 and are **later**.

| Slice | Delivers (stories) | Controller / registry | FFI | Mac |
|---|---|---|---|---|
| 0 | Gate, not code: confirm "What this needs from 005's port"; storyboards `contacts/` red (open-find-page, join-two, delete-undo, groups, import); demo seed `small:30` | — | — | — |
| 1 | **Open and find** (US1): `g p`, the Contacts mode, tabs 1 and 2, letter headers, `/` in people mode, empty states for those tabs, `Esc` ladder, History, `@` from a message row (entry only) | registry rows for open/tabs/filter/movement; `Context::Contacts`; `contacts.rs` list half; `Policy.caps.contacts_view` | page read, counts, `UiEvent` cases | `ContactsTable`, header strip, menu bar entry, places popover entry. **Design-review gate (Q1)** after this slice's screenshots |
| 2 | **Read a page, write, show mail** (US2): `Return`, the secondary window, `j`/`k`, address rows, recent mail, `c`, `⌥↩`, extra vCard properties read-only | `ContactCard` context, `contact_card.rs` | detail read, recent conversations read | `ContactCardWindowChrome`, geometry shared with the message window |
| 3 | **Create, edit, delete, restore, undo** (US3, US4): `n`, `e`, per-field commit, `#`/⌫, tab 5, `R`, pills, Edit ▸ Undo naming | commands wired to 005's; cursor rules after delete/undo | command exports, `ContactsChanged` | in-place fields, strips, pill texts |
| 4 | **Addresses and joining** (US5, US6): `m` with marks, the join name picker, the organisation step, `+`, the Move popover, `-`, `P` | pickers contents; refusals | join with name and organisation | `ContactPickerPopover`, `MoveAddressPopover` |
| 5 | **Duplicates** (US5): tab 4, pair rows with evidence, `m` and `-` on a pair, the count | suggestions rows | suggestions read | pair row layout |
| 6 | **Groups** (US7): tab 3, `l` picker, group members view, `E`, `L`, `g n`, drag onto a group, compose to a group | group rows and rungs | groups reads | group rows, drag target |
| 7 | **Import and export** (US8): ⌘⇧I, ⌘⇧E, drop, the summary sheet, unreadable-file and cancel paths | import/export intents; `INTERCEPTED` | files-chosen export, summary | `ContactsPanels`, `ImportSummarySheet` |
| 8 | **Everywhere else** (US9): person-shaped completion merged with system Contacts, `@` mode finding by any address, the menu bar and `?` complete, VoiceOver pass, narrow-window pass, light/dark screens, full-store budget counts | `recipient_suggestions` shape | person-shaped suggestions | `ComposeModel`/`ContactsSource` merge by address |

### Later slices (GTK and the terminal)

| Slice | Delivers | Notes |
|---|---|---|
| G1 | GTK turns `contacts_view` on; the list and page as GTK widgets over the same intents; `offered_on` opens the commands on Freedesktop; Linux rows of `keybindings.md` regenerate | No controller change by design. Needs a machine that builds GTK |
| G2 | GTK file chooser (portal) for import and export; drop target | `POSTIO-CONSENT:` equivalents |
| G3 | Storyboards `contacts/*` play on `postio-gtk`'s runner and are filmed; `/ux-review` for the whole surface; the `interactions-unreviewed` label retires | Closes FR-091 |
| G4 | The terminal (`postio-tui`) draws the list and page from the same intents | After `postio-tui`'s Focus adoption |

## Test plan

| Layer | What | Where |
|---|---|---|
| Registry | rows, titles, no collisions in `Contacts`/`ContactCard`, `offered_on` hides them on Linux, keybindings doc | `postio-core`, `postio-ui` `keybindings_doc` |
| Words | empty states, pills, refusals, the Move sentence, import summary | `postio-ui` tests |
| Controller | tabs and per-tab cursors; Esc ladder; marks-else-cursor; cursor after delete, join, undo; join picker candidates and preselection; organisation step; `n`/`-`/`E` dispatch by tab and row; refusals; `/` people mode; History; `@` sender | `postio-focus/tests/contacts.rs`, `contact_card.rs` |
| Budgets | statements and rows for the list page, filter, detail, suggestions; first page unchanged by group/tab count | `postio-storage` `counting` assertions in the port's suites, referenced here |
| FFI | a typed sequence over the demo store emits intents in order; panels return URLs; a failed write keeps the intent for retry | `ffi_suite/focus_contacts.rs` |
| Swift | model sync with intents; keyboard paths; column dropping; card geometry equals message window's | `macos/Tests/…` |
| Privacy | `check-no-silent-tracking.py` passes with the panels' consent comments; no `URLSession` | `scripts/check.sh` |
| Storyboards | the five flows, red on base | `storyboards/contacts/` |
| Linux | CI's `postio-gtk` suites unchanged (the commands are not offered) | CI |

## Project Structure

### Documentation (this feature)

```text
specs/011-focus-contacts/
├── spec.md
├── plan.md                  # this file
├── contracts/
│   └── focus-contacts.md    # intents, the surface's FFI shapes, panel rules
├── checklists/requirements.md
└── tasks.md                 # /speckit-tasks
```

### Source Code (repository root)

```text
crates/
├── postio-core/src/{registry,command,context}.rs   # + commands, + two contexts
├── postio-ui/src/{keymap,cheatsheet,focus_dialog,contacts_words}.rs
├── postio-focus/src/{contacts,contact_card}.rs     # NEW
├── postio-focus/tests/{contacts,contact_card}.rs   # NEW
└── postio-ffi/src/{focus_contacts,registry,event}.rs
macos/Sources/{PostioKit,PostioAppKit,Postio}/…     # per the Swift table
macos/Tests/…
storyboards/contacts/*.toml
docs/keybindings.md                                  # regenerated
```

**Structure Decision**: as above; Rust layering follows ADR 0043 rule 1
(logic in the lowest layer that can hold it), Swift layering follows #1264.

## Complexity Tracking

| Violation | Why needed | Simpler alternative rejected because |
|---|---|---|
| Two new contexts rather than one with row-kind dispatch (005's R10) | The page is a window of its own with its own `j`/`k` meaning; one context would give `j` two meanings in one place | One context: the registry has no notion of the focused window, so `next_contact` and a row cursor key could not both be `j`/`Down` |
| Commands dispatching by tab or row kind (`n`, `e`, `#`, `m`, `-`) | One binding per verb, as 005 chose (R10) | A command per tab: five more ids and a keymap that teaches tabs rather than verbs |
| Mac-only for now (`offered_on`) | GTK cannot be built or verified on this Mac | Offering on Linux with no surface: keys that do nothing (010 D25) |
