---

description: "Task list for feature implementation"
---

# Tasks: Contacts

**Input**: Design documents from `/specs/005-contacts/`

**Prerequisites**: spec.md, plan.md, research.md, data-model.md, contracts/, quickstart.md — all written and committed

**Tests**: Test-first is Constitution IV and NON-NEGOTIABLE. Every task that
changes behaviour names its test *first* and its code second, and the test is
**observed red** before the code is written. A task whose test was never seen
red has not been done — tighten the assertion, never re-break the code.
Tests assert what a person would see (the row a list draws, the name a header
shows, the rows a popover offers), not what a layer was handed.

## Format: `[ID] [P?] [Story] Description`

- **[P]** — touches files nothing else in its phase touches, so it can run beside its siblings
- **[US#]** — the user story it serves; setup, foundational and polish tasks carry none

## Path Conventions

Paths are workspace-relative from `~/src/postio-worktrees/contacts`. Commits
end `Refs: specs/005-contacts` and the task id — never `Refs: #<issue>`.
A new `gtk_suite` test is registered by name in
`crates/postio-gtk/tests/gtk_suite/main.rs`; a new `app_suite` case is a module
plus a row in `crates/postio-app/tests/app_suite/main.rs`'s `CASES`.

## Rules that hold for every task

1. **`postio-gtk` learns nothing about the store.** Contacts rows reach it
   through `postio-app`/`postio-runtime`, the way message pages do
   (`postio-gtk/src/feed.rs` → `postio-app/src/feed.rs`). A diff that puts SQL
   or a storage type in `postio-gtk` is the boundary failing.
2. **Every commit is green for the crates it touches.** Phase 2 changes a
   shared type under several crates; its tasks may land as one commit if
   splitting them would leave a red tree — say so in the body.
3. **Logs carry ids and counts.** No task logs a name, address or note (FR-061).
4. **Nothing touches the network** (FR-060). No avatar, favicon or lookup.
5. **Fixtures use reserved domains** (`example.com`, `.test`, `.invalid`,
   `.example`). `scripts/checks/check-no-personal-data.py` applies.

---

## Phase 1: Setup

- [X] T001 Create the leaf crate `crates/postio-vcard/` (`Cargo.toml`, `src/lib.rs` with a module doc citing research R9), add it to the workspace members in the root `Cargo.toml`, and depend on `vcard-rs = { version = "=0.4.0", default-features = false, features = ["parser", "base64", "quoted-printable", "encoding-rs"] }` (confirm feature names against the crate's `Cargo.toml` and drop any the parser does not need). Proof: `cargo tree -p postio-vcard -e normal` lists no serde, chrono, tokio or database crate
- [X] T002 Add a `postio-vcard` rule to `scripts/checks/check-crate-boundaries.py` banning the database engine (`turso`, and `rusqlite` so the rule survives a rename), `gtk4` and `tokio`, with the check's own test case — mirror how `postio-search` and `postio-body` are declared pure leaves
- [X] T003 [P] Add `AddressId` to `crates/postio-model/src/ids.rs` with the `local_id!` macro beside `ContactId` (`:112`), re-export it from `crates/postio-model/src/lib.rs`, and extend `crates/postio-model/tests/model_suite/serde_roundtrip.rs` with it — red first on the missing type

---

## Phase 2: Foundational — the person model (blocks every user story)

**Purpose:** move the store from "a contact is an address" to "a contact is a
person owning addresses" while every existing contacts behaviour — sync
counting, completion ranking, the `@` finder, `group:`, search affinity —
stays green. After this phase no user sees a difference except that shared
contacts now reach the UI. Shape and reasons: `data-model.md`, research R1–R4.

### Model

- [X] T004 Reshape `Contact` in `crates/postio-model/src/contact.rs` into the person of data-model.md: `id`, `name`, `organization`, `note`, `source`, `state: ContactState`, `preferred: AddressId`, `addresses: Vec<ContactAddress>`, `seen_name`, `times_seen`, `last_seen_at`, `written`; add `ContactState { Live, Deleted, Merged }` with `as_str`/`from_name` like `ContactSource`; add `ContactAddress { id, address, times_seen, last_seen_at, written }`. Keep `display_name()` with precedence name → seen name → preferred address. Tests first in the module's `#[cfg(test)]` and `crates/postio-model/tests/model_suite/invariants.rs` (`:385`): display-name precedence including a whitespace-only name, state round-trip, `from_name` refusing unknowns
- [X] T005 [P] Drop `account_id` from `ContactGroup` in `crates/postio-model/src/contact_group.rs`; update `serde_roundtrip.rs` (`:183-191`, `:233`)

### Schema

- [X] T006 Rewrite the contacts part of `HEAD` in `crates/postio-storage/src/schema.rs` per data-model.md: `contacts` as the person (drop `account_id`, `address*`, `suppressed`, `vcard_extra`; add `organization`, `note`, `state` CHECK, `merged_into`, `preferred_address`, `seen_name`, `sort_key`, `name_key`, `written`, `vcard`, `created_at`, `updated_at`); `addresses.contact_id` REFERENCES contacts ON DELETE SET NULL; new `contact_sightings`, `contact_terms`, `contact_join_candidates`, `contact_join_dismissals`; `contact_groups` without `account_id`, name unique case-insensitively. Replace `idx_contacts_account_address`, `idx_contacts_shared_address`, their `_read` twins and `idx_contacts_rank` (`:645-656`, `:795-798`) with the indexes data-model.md lists, each with non-partial read twins where the planner needs them (see the comment at `:780-793`). Every column gets a comment saying what it *means*. Update the expected-object list (`:929-993`) so `schema::declared()` and `storage_suite/schema_fidelity.rs` pass; name each dropped index as deliberately absent. Test first: a `schema_fidelity.rs` case asserting the new tables and indexes exist and the old ones are named absent
- [X] T007 Update `crates/postio-storage/tests/storage_suite/schema_fidelity.rs::a_contact_accumulates_sightings` (`:731`) and the raw contact inserts at `:738`, `:755`, plus `contact_rank_index.rs:71,87,158,168`, `threading_lookup_cost.rs:79`, `crates/postio-index/tests/index_suite/executor.rs:1254`, `crates/postio-bench/benches/search_budget.rs:189`, `crates/postio-runtime/examples/store_diag.rs:145` to the new tables — each must still assert what it asserted before

### Recording (sync write path)

- [X] T008 Write the new record path in `crates/postio-storage/src/repository/contacts.rs`: `record_message(&Message, own: &dyn Fn(&EmailAddress) -> bool)` — for each distinct non-own address on the message (from, sender, reply_to, to, cc, bcc, deduped by normalised form, as today): look up `(addresses.id, contact_id)`; if unowned, insert a `live`/`mail` person (sort/name keys from the header name or address), set `contact_id`, and write its `contact_terms`; upsert `contact_sightings (address_id, account_id)` (`times_seen + 1`, monotonic `last_seen_at`, `last_name`); and when the message's `From` is own and the address is in to/cc/bcc, `times_written + 1` (research R3); then update the person's aggregates and `seen_name`/`sort_key` if the name advanced. A non-`live` owner still counts (research R4). Keep statements literal for statement caching (#728). Tests first in `crates/postio-storage/tests/storage_suite/contacts.rs`, replacing the address-row tests at `:26-671` with person-shaped equivalents that assert the same behaviours: a sighting creates one person; a second message from the same address counts on the same person; two accounts keep separate sightings but one person; a message from an own address to X makes X `written`; a received message does not; a deleted person's address still counts and the person stays deleted; `Ada@Example.com` and `ada@example.com` count on one address and one person (FR-011)
- [X] T009 Add a counting budget in `crates/postio-storage/tests/storage_suite/contacts_budget.rs` using `postio_storage::test_support::counting`. The counter sees **reads only** (an `execute` is uncounted), so the budget is exact on the read half: recording a message costs one read per known correspondent (the address and its owner in one lookup), two per address never stored; the writes are held to seeking by `threading_lookup_cost.rs`. Registered in `storage_suite/main.rs`
- [X] T010 Move own-address detection ahead of stripping in `crates/postio-sync/src/contacts.rs:36-64`: build the `is_own` closure from `Account::owns_address` (`crates/postio-model/src/account.rs:412`) — replacing the local copy at `:41-48` — and pass it to `record_message` instead of cloning and stripping the message. Update the unit tests at `:93`, `:123`, `:146` and add one: a message in which the user is the sender makes every recipient `written`. The call sites `crates/postio-sync/src/initial.rs:634` and `crates/postio-sync/src/resync.rs:589` keep their first-insert guard unchanged; `sync_suite/loopback.rs:306`, `resync.rs:176`, `:255`, `write_batch.rs:52`, `:122`, `initial.rs:252` must stay green unmodified in what they assert
- [X] T011 [P] Update `crates/postio-storage/src/seed.rs` (`record_correspondents`, `:536`) and `crates/postio-bench/benches/sync_writes.rs:347` to the new signature; `seed.rs:888` stays green

### Existing readers moved onto persons

- [X] T012 Rewrite completion search in `crates/postio-storage/src/repository/contacts.rs` (`search`, `:157`): `complete(prefix, limit) -> Vec<Contact>` — live people whose `contact_terms` match the prefix, ordered by the Q6 bands (`source = 'mail'` last, then `last_seen_at DESC`, `times_seen DESC`, `id`) off the rank index, each with its addresses preferred-first. No account argument (people are shared; research R2). Port every ranking test in `storage_suite/contacts.rs` (`:221`, `:282`, `:329`, `:373`, `:717`, `:743`, `:786`, `:820`, `:861`) to people; add: a person with two addresses appears once. Add a counting case to `contacts_budget.rs` on 20,000 people: one completion is at most two statements (people, then their addresses), materialises at most `limit` people and their addresses, and `counting::scans` reports no full scan — it runs on the UI thread (`blocking::now`), so this budget is the 16 ms one. Update `crates/postio-storage/tests/storage_suite/contact_rank_index.rs` so its copied `ORDER BY` (`:87-91`) matches, its plan test (`:94`) still proves no sort, and its 20,000-row fixture is people
- [X] T013 Point composer completion at people in `crates/postio-app/src/compose.rs::install_recipient_suggestions` (`:899-955`): call `complete`, emit one `RecipientCandidate` per address with the person's display name, preferred first; groups via members' preferred addresses (`resolved_address`, `:958`, becomes "the person's name + preferred address"). `crates/postio-gtk/tests/gtk_suite/gtk_composer_recipient_select.rs` (`:137`, `:168`, `:232`, `:257`) and `gtk_composer_recipients.rs:33` stay green
- [X] T014 Point the `@` finder at people: `crates/postio-app/src/search.rs::load_contacts` (`:857`) loads live people with addresses (drop `Some(account)`; keep a cap), and `crates/postio-gtk/src/finder.rs` (`contacts` `:340`, `contact_name` `:379`, `set_contacts` `:686`) matches a person by name or any address and shows them once. The storage read behind `load_contacts` gets a counting case in `contacts_budget.rs`: a fixed number of statements however many people there are, rows bounded by the cap, no full scan beyond the cap's index walk. The query written on pick is unchanged here (`from:`) — `with:` arrives in US1. `gtk_suite/gtk_finder.rs:499` and `app_suite/search_wiring.rs:176`, `search_live.rs:196` stay green
- [X] T015 [P] Resolve `group:` through owners in `crates/postio-index/src/executor.rs:1525-1537`: members → `addresses.contact_id` → `recipients.address_id`, skipping non-`live` members. `crates/postio-index/tests/index_suite/group_filter.rs` (`:115`, `:129`, `:136`, `:144`) updated to create people; add a case: a member with two addresses matches mail through either
- [X] T016 [P] Move search affinity onto sightings in `crates/postio-index/src/executor.rs:1343` (`hydrate_sql`): `max(contact_sightings.times_seen)` by `address_id`, scoped by account when scoped. `executor.rs::hydrate_probes_contacts_by_address_key` (`:1813`) re-pins the probe to the sightings key; `index_suite/executor.rs:1226` stays green
- [X] T017 [P] Update `crates/postio-storage/src/repository/contact_groups.rs` to groups without `account_id` (`list()` takes no account; `members` returns live people only for expansion, and a separate `all_members` for the management surface); port `storage_suite/contact_groups.rs`

**Checkpoint**: `scripts/test-sanity.sh` green; `cargo nextest run -p postio-storage --test storage_suite`, `-p postio-sync --test sync_suite`, `-p postio-index --test index_suite` green; completion and finder behave as before in `app_suite`. Commit (possibly as one commit for T004–T017 — rule 2).

---

## Phase 3: User Story 1 — See the people I correspond with (Priority: P1) 🎯 MVP

**Goal**: `g c` or the sidebar opens a Contacts pane over the reading pane,
listing people built from the mail (default: made, imported or written to),
filterable by typing, with a detail view, "show mail", "compose to", and `Esc`
back to exactly where the user was.

**Independent Test**: quickstart.md Story 1 rows — `app_suite::contacts_screen`.

### Tests first (observe each red)

- [X] T018 [P] [US1] `crates/postio-storage/tests/storage_suite/contacts_list_views.rs`: on a seeded store, the default view lists people written to plus `user`/`import` people and excludes a received-only newsletter sender; the everyone view includes it; pages are ordered by displayed name; keyset paging returns disjoint consecutive pages; a filtered page matches name, organisation, seen name and any address's local part or domain; each row carries its `source` so the list can mark people the user made or imported (FR-005); an address that is one of the user's identities is never listed, even when it was recorded before the identity was added (spec edge case)
- [X] T019 [P] [US1] `crates/postio-storage/tests/storage_suite/contacts_budget.rs` (extend): on 20,000 people, one page of each view and one filtered page is one statement, materialises ≤ page-size rows, and `counting::scans` reports no full scan; the filtered view stops at its cap (research R5)
- [X] T020 [P] [US1] `crates/postio-search/tests/parse.rs`: `with:a@example.com,b@example.com` parses to `Filter::With` with both addresses; `with:`, `with:a@`, `with:a@example.com,` are intermediate states, not errors; `-with:` negates
- [X] T021 [P] [US1] `crates/postio-index/tests/index_suite/with_filter.rs` (register in `main.rs`): `with:` matches a message where any listed address is from, to, cc or bcc; not a display-name substring; an unknown address matches nothing; two `with:` tokens AND
- [X] T022 [P] [US1] `crates/postio-core/tests/core_suite/command_registry.rs`: a per-context case like `the_account_row_actions_are_commands_in_the_account_list` (`:435`) asserting the US1 commands of contracts/commands.md exist in `Context::Contacts` with their bindings, and `open_contacts` is `g c` in any context
- [X] T023 [P] [US1] `crates/postio-gtk/tests/gtk_suite/gtk_contacts_pane.rs`: mounting the Contacts occupant hides the reader, focus on its list enters `Context::Contacts`, and closing restores the prior context, pane, selected row index and scroll value — the same assertions `gtk_composer_detach.rs` makes (`~:340-363`); plus `gtk_reader_pane_owner.rs` gains "contacts takes the pane and gives it back" beside `:318`
- [X] T024 [P] [US1] `crates/postio-app/tests/app_suite/contacts_screen.rs`: in a wired app over seeded mail, `g c` shows the correspondents' names as rows; typing `ada` leaves only matching rows with the first selected; a person the user made carries the made-or-imported mark and a mail-only person does not (FR-005); "show mail" leaves a `with:` query in the search box and lists mail from and to every address; `Esc` returns to the same folder, message, selection and scroll; the cheat sheet lists every Contacts command; and the `egress_log` table is unchanged after all of it (SC-007, FR-060)

### Implementation

- [X] T025 [US1] ~~Generalise `ListWindow`~~ — revised (research R5): a toolkit-free `ContactsWindow` in `crates/postio-ui/src/contacts.rs` (pages, generation, LRU bound, abandon, evictions; unit tests) and `ContactsModel`/`ContactItem` in `crates/postio-gtk/src/contacts/model.rs` (one item per position filled in place, `hold` while answering `item()`; unit tests), because `ListWindow` carries message-only meaning (thread aim, `RowFacts`)
- [X] T026 [US1] Add list reads to `crates/postio-storage/src/repository/contacts.rs`: `page(view: ContactView { Written, Everyone, Deleted }, after: Option<(sort_key, id)>, limit)`, `count(view)`, `filtered(view, text, cap)` via `contact_terms` ranges, `detail(id)` returning the person with per-address sightings summed across accounts, group names, and the number of **distinct** messages involving any of their addresses (`COUNT(DISTINCT r.message_id)` over `recipients` by the owner's address ids — not a sum of per-address counts, which double-counts a message carrying two of them; FR-006). Every view excludes people whose only addresses are the user's own identity addresses. Add to T019's budget: `detail` is a fixed number of statements returning one person, and the distinct count is one aggregate statement. Makes T018/T019 green
- [X] T027 [US1] ~~Expose through `postio-runtime`~~ — revised: the app reads through `crate::search::ask` on the runtime, as the `@` finder's loader already does, in `crates/postio-app/src/contacts.rs`; `MailStore` is the frontends' message seam and nothing outside the GTK app lists people yet. Storage gained `page_from(view, after, skip, limit)` so the app keeps offset→cursor marks the way `read_page` does
- [X] T028 [P] [US1] Add `Field::With` / `Filter::With(Vec<String>)` in `crates/postio-search/src/query.rs` (`:53`, `:205`, keyword table `:90`) and its arm in `crates/postio-search/src/parser.rs` (`:167`, beside `group`), per contracts/query-with.md. Makes T020 green
- [X] T029 [P] [US1] Add the `Filter::With` arm to `filter_condition` in `crates/postio-index/src/executor.rs:1490`: `m.id IN (SELECT r.message_id FROM recipients r JOIN addresses a ON a.id = r.address_id WHERE r.kind IN ('from','to','cc','bcc') AND a.address_normalized IN (…))`, one bound parameter per address, normalised. Makes T021 green
- [X] T030 [P] [US1] Create `crates/postio-ui/src/contacts.rs` (toolkit-free): `ContactsState` (view, filter text, cursor, selection — distinct), `show_mail_query(&[EmailAddress]) -> String` producing `with:a,b`, row text rules (displayed name, secondary line "n addresses · last in touch <relative date>", and a made-or-imported mark for `user`/`import` people, FR-005), and `RowKind { Person, Address, Group, Suggestion }` with `applies(command, kind) -> Result<(), Hint>` for focused-row dispatch (contracts/commands.md). Unit tests in the module first; register in `crates/postio-ui/src/lib.rs`
- [X] T031 [US1] Add `Context::Contacts` at the end of `Context::ALL` in `crates/postio-core/src/context.rs` (`:23-96`, `as_str`), and to every exhaustive match: `crates/postio-ui/src/keymap.rs` (`KeyContext` `:518`, layering `:562-584` as `[Contacts, Global]`, `From<Context>` `:596`), `crates/postio-ui/src/cheatsheet.rs::heading` (`:74`), `crates/postio-ffi/src/registry.rs` (`UiContext` `:36`, `From`s `:59-99`, test `:296`), `crates/postio-ui/tests/ui_suite/keybindings_doc.rs::where_available` (`:35`), `crates/postio-ui/src/focus.rs` test list (`:119`). `context.rs` tests `:258`, `:270` stay green
- [X] T032 [US1] Register the US1 commands of contracts/commands.md — `open_contacts`, `contact_show_mail`, `contact_compose`, `contacts_filter`, `contacts_toggle_everyone` — in `crates/postio-core/src/command.rs` (`command_ids!` `:33`, `Command` `:358`, `id()` `:831`, `default_for()` `:932`), `crates/postio-core/src/registry.rs` `SPECS` (`:336`, in `CommandId::ALL` order), and `crates/postio-core/src/menu.rs::section_for` (`:117`); add `Context::Contacts` to the context sets of the list-movement and selection commands, `back`, and **`undo`** (scoped today to `MESSAGE_SURFACES` plus `Context::Accounts`, so without this `u` does nothing in Contacts — FR-025). Extend T022 first: `u` resolves to `undo` in `Context::Contacts`, and no binding collides there. Makes T022 green; `command_registry.rs` (`:35`, `:62`, `:198`, `:216`, `:266`, `:310`, `:554`) stays green
- [X] T033 [US1] Add `Event::ContactsChanged { names_changed: bool }` to `crates/postio-core/src/event.rs` beside `MessageListChanged` (`:97`), with its fan-out entry (ADR 0013)
- [X] T034 [US1] Add `ReaderOccupant::Contacts` to `crates/postio-gtk/src/shell.rs` (`:79`) ranked above `Composer` in `fallback()` (`:136`) and a `set_contacts_open` flag beside `set_composing` (`:463`); extend `the_fallback_ranks_composer_over_search_over_reading` (`:699`) first
- [X] T035 [US1] Build `crates/postio-gtk/src/contacts/` (`mod.rs`, `pane.rs`, `list.rs`, `detail.rs`): the pane mounts into `shell.reader()` and registers as an occupant like `composer.rs::mount` (`:2059`); `take_pane`/`release_pane` copy the composer's contract (`:2607-2651`) including the focus reset; the list is a `GtkListView` over a `GListModel` fed through a `PageSource`-shaped trait (`crates/postio-gtk/src/list.rs:129`) on the generalised `ListWindow`; list and detail side by side above a breakpoint, stacked below; a filter entry bound to `contacts_filter`; a view switch bound to `contacts_toggle_everyone`; an empty state for "no one yet". Follow `.claude/skills/gtk-design/SKILL.md` §3 and render to PNG before calling it done. Makes T023 green
- [X] T036 [US1] Wire the pane in `crates/postio-gtk/src/window.rs`: `handled_here` arms (`:2360`) for `open_contacts`, `back` while contacts is open (beside `:2491`), a focus controller entering/leaving `Context::Contacts` like `ensure_keys_focus_controller` (`:3345`); contacts commands dispatch on the focused row's `RowKind` and show the hint from `applies` when it does not fit, never a silent failure
- [X] T037 [US1] Add a "Contacts" row to `crates/postio-gtk/src/sidebar.rs` below "Saved searches" (`:458-472`) in a single-row list of its own, included in `selectable_lists()` and special-cased in `step()` (`:1698`) to fire `open_contacts`; `crates/postio-gtk/tests/gtk_suite/gtk_sidebar_contacts_row.rs`: walking to it with the keyboard and activating it opens the pane
- [X] T038 [US1] Create `crates/postio-app/src/contacts.rs::install` (called from `crates/postio-app/src/lib.rs` after `compose::install`, `:671`): feed the pane from the runtime reads on the runtime thread with a generation check (the `feed.rs` pattern), refetch the visible page on `ContactsChanged`, and handle `contact_show_mail` (close the pane, set the finder's query to `ContactsState::show_mail_query`, run it) and `contact_compose` (close the pane first, then open a draft to the preferred address). Add the ids to the right owner list in `crates/postio-app/tests/app_suite/command_wiring.rs` (`:96-151`). Makes T024 green
- [X] T039 [US1] Make the `@` finder write `with:` on pick: `crates/postio-gtk/src/finder.rs::contact_query` (`:392`) emits `with:` over all the person's addresses; update `gtk_finder.rs` expectations (the old `from:` assertion was the defect research R6 names)

**Checkpoint**: US1 is a usable, shippable address-book viewer. Run `cargo nextest run -p postio-storage --test storage_suite contacts`, `-p postio-gtk --test gtk_suite contacts`, `cargo test -p postio-app --test app_suite contacts_screen`.

---

## Phase 4: User Story 2 — Join several addresses into one person (Priority: P1)

**Goal**: join (with a chosen name), detach, add and move an address, choose a
preferred address — all undoable — and the user's name for a person shows for
every address in the list and reader (FR-032).

**Independent Test**: quickstart.md Story 2 rows — `app_suite::contacts_join`, `app_suite::contact_name_in_list_and_reader`.

### Tests first (observe each red)

- [X] T040 [P] [US2] `crates/postio-storage/tests/storage_suite/contacts_join.rs`: joining keeps every address, every sighting, the union of group memberships, and concatenated notes; absorbed people become `merged`; the survivor's aggregates equal the sum; `written` survives; unjoin restores both people byte-for-byte (fields, addresses, memberships, aggregates); detaching makes a new person with that address's own history and refuses the last address; `add_address` on an address owned elsewhere returns the owner; `move_address` moves it, including from a deleted person (lifting suppression, FR-024); preferred must be one of the person's own
- [X] T041 [P] [US2] `crates/postio-session/tests/session_suite/contacts.rs` (revised: `mod contacts` in `crates/postio-session/src/actions.rs`'s tests, where the other verbs' undo tests live): each of `JoinContacts`, `DetachAddress`, `AddAddress`, `MoveAddress`, `SetPreferred` writes the store, emits `ContactsChanged`, records one undo entry, and `undo` restores the prior state exactly
- [X] T042 [P] [US2] `crates/postio-storage/tests/storage_suite/sender_names.rs`: with a live person named "Ada Lovelace" owning two addresses, `list_page` rows, and `participants_for` give that name, and `ContactRepository::user_names` hands the reader the same (revised: `read_recipients` stays the header's record -- `update()` and send.rs write a read `Message` back, so substituting there would persist the user's name into recipients) for mail from either address; `read_recipients` also returns the raw header name; a deleted person's name is not used; mail from an unowned address whose header says "Ada Lovelace" keeps its header (the forged-name edge case)
- [X] T043 [P] [US2] `crates/postio-storage/tests/storage_suite/list_statement_count.rs` (extend): a page's statement and row counts are identical before and after naming a person whose mail is on the page (research R7)
- [X] T044 [P] [US2] `crates/postio-ui/src/contacts.rs` unit tests: `join_name_choices(people)` lists user-set names then seen names, most recent preselected, de-duplicated; organisation conflict detection
- [X] T045 [P] [US2] `crates/postio-app/tests/app_suite/contacts_join.rs`: select two people in the pane (`x`, `Down`, `x` -- the list's own arrows walk it, contracts/commands.md), `m`, confirm the preselected name with `Return` (≤ 4 keystrokes, SC-004); one row remains; composer completion on "ada" shows one name with both addresses, preferred first; `u` restores two rows
- [X] T046 [P] [US2] `crates/postio-app/tests/app_suite/contact_name_in_list_and_reader.rs`: after the join, the message list rows and the reader header show "Ada Lovelace" for mail from both addresses — read from the drawn widgets — and the reader's details show the raw header name and address

### Implementation

- [X] T047 [US2] Implement `join`, `unjoin`, `detach_address`, `add_address`, `move_address`, `set_preferred` in `crates/postio-storage/src/repository/contacts.rs`, each one transaction; aggregates, `sort_key`, `name_key` and `contact_terms` recomputed for every person touched; a person left with no addresses is folded (`merged`) rather than removed, and the move hands back their prior state so undo revives them exactly (revised: removal left the inverse nobody to give the address back to). Makes T040 green
- [X] T048 [US2] Add the `Command` payloads of contracts/commands.md (`JoinContacts`, `UnjoinContacts`, `DetachAddress`, `AddAddress`, `MoveAddress`, `SetPreferred`) to `crates/postio-core/src/command.rs`, their `UndoKind`s and `describe` text to `crates/postio-core/src/undo.rs` (`~:37-101`), and handlers in `crates/postio-session/src/actions.rs` returning `Applied` with the inverse (pattern: `map_mailbox_role`, `:1685-1770`); add them to `WIRED` (`:61`). Makes T041 green
- [X] T049 [US2] Register `contact_join`, `contact_add_address`, `contact_detach_address`, `contact_set_preferred` in the registry (`command.rs`, `registry.rs` `SPECS`, `menu.rs`) with `Recovery::Undo`; `recovery_undo_means_the_undo_stack_and_nothing_else_claims_it` (`command_registry.rs:234`) stays green
- [X] T050 [US2] Build the join panel and address actions in `crates/postio-gtk/src/contacts/join.rs` and `detail.rs` (revised: inline panels in the detail column rather than a dialog; the window routes every `Ask` payload to `ContactsPane::ask`, never the bus): the name chooser from `join_name_choices` (keyboard-first, `Return` confirms the preselection), organisation choice only on conflict, an address row focus so detach/preferred act on the focused address, the "belongs to <name> — move it?" prompt for `add_address`. Wire in `crates/postio-app/src/contacts.rs`. Makes T044/T045 green
- [X] T051 [US2] Substitute the sender name in storage (research R7): `LIST_COLUMNS`' sender subquery in `crates/postio-storage/src/repository/messages.rs:672-682` resolves `coalesce(<live owner's name>, recipients.name)` through `addresses.contact_id`; the same in `crates/postio-storage/src/repository/threads.rs::participants_for` (`:1327`) and `messages.rs::read_recipients` (`:2611`), the last also returning the raw name. Carry the raw name through `crates/postio-app/src/reading.rs`'s `Envelope` (`:1437`) to `crates/postio-ui/src/reader/header.rs` so the details line shows it. Makes T042/T043/T046 green
- [X] T052 [US2] On `ContactsChanged { names_changed: true }`, refetch the message list's and open conversation's visible pages in `crates/postio-app/src/feed.rs` (the ordinary refetch path); emit `names_changed` from every handler that changes a live person's name or an address's owner (done as `Plan::Repaint` over resident pages, the reader re-read through its `BodyLoaded` path, and `ContactsPane::refresh` keeping the cursor)
- [ ] T052b [US2] An open conversation re-reads its entries' sender names on `ContactsChanged { names_changed: true }` without resetting its document's scroll; today it picks the names up on the next open

**Checkpoint**: US1 + US2 are the whole of what the request named. `cargo test -p postio-app --test app_suite contacts_join contact_name_in_list_and_reader`.

---

## Phase 5: User Story 3 — Create, edit and delete a person (Priority: P2)

**Goal**: create someone who never wrote, edit name/organisation/note (promoting
a mail person), delete so they stay gone, restore from the Deleted view.

**Independent Test**: quickstart.md Story 3 rows — `storage_suite::contacts_lifecycle`, `app_suite::contacts_delete_restore`.

### Tests first (observe each red)

- [X] T053 [P] [US3] `crates/postio-storage/tests/storage_suite/contacts_lifecycle.rs`: `create` with a never-seen address makes a `user` person offered by `complete` at once; `create` with an address owned by a deleted person moves it and keeps its sightings (FR-024); `edit` promotes `mail` → `user` in place and a later sighting with another display name leaves `name` alone (FR-021/22); `delete` hides the person from every view but Deleted, from `complete` and from name substitution, and a later message from its address still counts on it and leaves it deleted (SC-005); `restore` returns it whole, sightings gathered meanwhile included
- [ ] T054 [P] [US3] `crates/postio-session/tests/session_suite/contacts.rs` (extend): `CreateContact`, `EditContact`, `DeleteContact`, `RestoreContact` each emit and undo exactly
- [ ] T055 [P] [US3] `crates/postio-app/tests/app_suite/contacts_delete_restore.rs`: delete a person with `#`, send a fixture message from their address through the sync mock, confirm the row does not return and completion does not offer them; toggle the Deleted view (`v d`), `r` restores; `u` after a delete restores too

### Implementation

- [X] T056 [US3] Implement `create`, `edit` (+ `put_fields` for its inverse), `delete`, `restore` in `crates/postio-storage/src/repository/contacts.rs`, recomputing terms and keys; `create` and `add_address` refuse an address that is one of the user's own identity addresses, with the reason (spec edge case). `delete` on a group-kind row deletes the group (T067). Makes T053 green
- [ ] T057 [US3] Add `CreateContact`, `EditContact`, `DeleteContact`, `RestoreContact` payloads, undo kinds and handlers (`command.rs`, `undo.rs`, `actions.rs`, `WIRED`). Makes T054 green
- [ ] T058 [US3] Register `contact_new`, `contact_edit`, `contact_delete` (destructive, `Recovery::Undo`; acts on a person or, in US5, a group), `contact_restore`, `contacts_toggle_deleted` in the registry and menu; `destructive_commands_offer_a_way_back` (`command_registry.rs:216`) stays green
- [ ] T059 [US3] Build the editor in `crates/postio-gtk/src/contacts/editor.rs` (name, addresses, organisation, note; keyboard-reachable; validation of addresses via `EmailAddress` parsing with the reason shown inline) and the Deleted view in `list.rs`; wire in `crates/postio-app/src/contacts.rs`. Makes T055 green

**Checkpoint**: #4's first acceptance criterion is met by a surface.

---

## Phase 6: User Story 4 — Postio suggests who might be the same person (Priority: P3)

**Goal**: offer likely duplicates with evidence; accept behaves as a join; a
dismissal is permanent.

**Independent Test**: quickstart.md Story 4 row — `storage_suite::contact_suggestions`.

### Tests first (observe each red)

- [ ] T060 [P] [US4] `crates/postio-storage/tests/storage_suite/contact_suggestions.rs`: two live people with the same `name_key` are suggested with their per-address counts; a reply-candidate pair is suggested; a dismissed pair is never suggested again, after more mail, and after either side joins a third person; nothing is ever joined without a command; the suggestions read has a counting budget (no full scan)
- [ ] T061 [P] [US4] `crates/postio-sync/tests/sync_suite/reply_candidates.rs` (register in `main.rs`): a reply from Y to the user's message sent only to X records candidate (X, Y); a reply from X records nothing; a reply to someone else's message records nothing

### Implementation

- [ ] T062 [US4] Record reply candidates at sync in `crates/postio-sync/src/contacts.rs` using the parent threading already resolved in `commit_batch`/`incremental`; write `contact_join_candidates` in `crates/postio-storage/src/repository/contacts.rs`. Makes T061 green
- [ ] T063 [US4] Implement `suggestions(limit)` (name-key self-join ∪ candidates, minus dismissed address pairs, live people only) and `dismiss(a, b)` — which writes the pair of the two people's preferred addresses; the read hides a suggestion when *any* dismissed pair spans the two people (data-model.md), which is what keeps it dismissed across later joins — in `crates/postio-storage/src/repository/contacts.rs`. Makes T060 green
- [ ] T064 [US4] Add `DismissSuggestion`; register `contacts_suggestions` and `suggestion_dismiss` (`X`); make `contact_join` (`m`) on a suggestion-kind row open the join dialog for that pair; build the suggestions view in `crates/postio-gtk/src/contacts/suggestions.rs` showing the evidence; an `app_suite` case `crates/postio-app/tests/app_suite/contacts_suggestions.rs` that joins one with `m` (then `u`) and dismisses one with `X`

---

## Phase 7: User Story 5 — Groups (Priority: P3)

**Goal**: manage groups from the Contacts screen; composer expansion uses each
member's preferred address; `group:` works through any member address.

**Independent Test**: quickstart.md Story 5 row — `app_suite::contact_groups`.

### Tests first (observe each red)

- [ ] T065 [P] [US5] `crates/postio-storage/tests/storage_suite/contact_groups.rs` (extend): create/rename/delete, add/remove members; a case-insensitive duplicate name is refused; expansion returns preferred addresses of live members only; deleting a person keeps its membership and restoring it returns
- [ ] T066 [P] [US5] `crates/postio-app/tests/app_suite/contact_groups.rs`: create "Family" from the pane, add two people, pick the group in a new draft's To field — both preferred addresses fill in; edit the group afterwards and the draft does not change (FR-041); `group:family` in search finds mail through a member's non-preferred address

### Implementation

- [ ] T067 [US5] Group commands (`CreateGroup`, `RenameGroup`, `DeleteGroup` + restore inverse, `AddMembers`, `RemoveMembers`) in `command.rs`/`undo.rs`/`actions.rs`; register `contact_group_new`, `contact_group_rename`, `contact_group_add`, `contact_group_remove`; group deletion is `contact_delete` on a group-kind row (contracts/commands.md), handled by `DeleteGroup` with a `RestoreGroup` inverse
- [ ] T068 [US5] Groups in the pane: a groups section above the people list in `crates/postio-gtk/src/contacts/list.rs` (a group row filters the list to its members), member add/remove from the selection; wire in `crates/postio-app/src/contacts.rs`; composer expansion in `crates/postio-app/src/compose.rs` uses preferred addresses. Makes T065/T066 green

---

## Phase 8: User Story 6 — vCard import and export (Priority: P4)

**Goal**: import `.vcf` (3.0/4.0) into people and groups without losing what
Postio does not model; export 4.0.

**Independent Test**: quickstart.md Story 6 row — the `postio-vcard` corpus round trip and `app_suite::contacts_import_export`.

### Tests first (observe each red)

- [ ] T069 [P] [US6] Add the fixture corpus under `crates/postio-vcard/tests/corpus/` per contracts/vcard.md (3.0 with `item1.` groups, `PHOTO;ENCODING=b`, `TEL`, `X-` properties, two `EMAIL`s; the 4.0 equivalent; a group card; a file with one malformed card), reserved domains only, and `crates/postio-vcard/tests/round_trip.rs`: parse → export reproduces every unmodelled property byte-for-byte from 4.0, and from 3.0 differs only in the four rewrites; two `EMAIL`s → one person, `PREF` → preferred; `KIND:group` → group with members; the malformed card is skipped with a reason and the rest parse; `CATEGORIES` survives verbatim
- [ ] T070 [P] [US6] `crates/postio-storage/tests/storage_suite/contacts_import.rs`: applying a parsed card whose address a mail person owns makes one person with the card's fields and the address's history (FR-053); a card spanning two people joins them and lists the join in the summary (FR-053); a user-set name beats the card's `FN` and is counted; the card text lands in `contacts.vcard`
- [ ] T071 [P] [US6] `crates/postio-app/tests/app_suite/contacts_import_export.rs`: import through the ask-seam (not a real `GtkFileDialog` — see `crates/postio-gtk/src/parts.rs:600-624`, #988), see the people and group appear and the summary; export the default view with nothing selected and confirm only listed people are in the file (FR-050); the `egress_log` table is unchanged (SC-007); and the log captured during import (`POSTIO_LOG=debug` into a test subscriber) contains counts and none of the fixture's names or addresses (FR-061)

### Implementation

- [ ] T072 [US6] Implement `parse`, `export`, `export_group` in `crates/postio-vcard/src/lib.rs` (plus `upgrade.rs` for the 3.0 → 4.0 rewrites, `map.rs` for property ↔ model) over `vcard-rs`. Makes T069 green
- [ ] T073 [US6] Implement `apply_import(cards) -> ImportSummary` and the export read (people in a view or selection, with stored cards) in `crates/postio-storage/src/repository/contacts.rs`, one transaction per import. Makes T070 green
- [ ] T074 [US6] Register `contacts_import` (`v i`) and `contacts_export` (`v x`) — every command has a default binding (Principle II); in `crates/postio-app/src/contacts.rs` open/save with `gtk::FileDialog` through an ask-seam like `parts.rs::connect_ask`, read and parse off the UI thread (`compose.rs::install_attach` pattern, `:1036`), write the file like `reading.rs::write_part` (`:1612`); show the summary; log counts only. Makes T071 green

---

## Phase 9: Polish & cross-cutting

- [ ] T075 Fold ADR 0007 (research R12): delete `docs/decisions/0007-address-book.md`, remove its row from `docs/decisions/README.md`, re-point every `git grep -n 0007` citation in code and docs to `specs/005-contacts` (the 36 today: `postio-gtk/src/composer.rs`, `gtk_composer_recipient_select.rs`, `postio-index/src/executor.rs`, `group_filter.rs`, `postio-model/src/contact.rs`, `contact_group.rs`, `mime.rs`, `postio-search/src/query.rs`, `tests/parse.rs`, `postio-storage/src/repository/contact_groups.rs`, `contacts.rs`, `storage_suite/contact_groups.rs`, `contact_rank_index.rs`, `contacts.rs`, `docs/PRODUCT.md`, `docs/field-report.md`), and check each still says something true
- [ ] T076 [P] Update `docs/PRODUCT.md`: move the contacts management surface and vCard from "Out, deliberately" (`:645`) to "In"; update the §6 note (`:173`) and the compose note (`:357`); describe `with:` beside the other operators in the search section
- [ ] T077 [P] Regenerate `docs/keybindings.md` (`POSTIO_UPDATE_DOCS=1 cargo test -p postio-ui`) and `docs/config.md` if it lists commands; `keybindings_doc.rs:169` and `:196` green
- [ ] T078 [P] Render the Contacts pane (wide and narrow, light and dark, populated, empty, filtered-to-nothing, Deleted view) through the `/gtk-design` render loop and fix what does not match the canvas tokens; attach nothing personal
- [ ] T079 Walk quickstart.md end to end; run `scripts/check.sh`, `cargo clippy` on every touched crate with `-D warnings`, and the integration suites the diff touches; fix what fails
- [ ] T080 Land with `scripts/issue-land.sh --detach`; the PR body closes #477 and #475 and names specs/005-contacts as the acceptance

---

## Dependencies & Execution Order

### Phase dependencies

- **Setup (1)**: none. T001–T002 only gate Phase 8; T003 gates Phase 2.
- **Foundational (2)**: blocks every story — the model and schema change under all of them.
- **US1 (3)**: after Phase 2. Blocks US2–US5, which all act inside the pane.
- **US2 (4)**: after US1. Blocks US4 (accept = join).
- **US3 (5)**: after US1; independent of US2 (may run beside it).
- **US4 (6)**: after US2.
- **US5 (7)**: after US1; independent of US2–US4.
- **US6 (8)**: storage/crate work (T069, T070, T072, T073) after Phase 2 and T001–T002 — can run from the start of Phase 3; the surface (T071, T074) after US1. T070's join case needs US2's `join`.
- **Polish (9)**: after every story wanted in this landing.

### Within a story

Tests (observed red) → storage → core/session commands → toolkit-free `postio-ui` → widgets → app wiring → the `app_suite` case green.

### Story completion graph

```text
Setup ─► Foundational ─► US1 ─┬─► US2 ─► US4
                              ├─► US3
                              ├─► US5
                              └─► US6 (surface)
        Setup(T001-2) + Foundational ─► US6 (crate + storage)
```

## Parallel opportunities

- **Phase 1**: T003 beside T001–T002.
- **Phase 2**: T005 beside T004; T011, T015, T016, T017 in parallel once T006/T008 exist.
- **US1**: all tests T018–T024 in parallel; then T028 + T029 (search) ‖ T030 (ui) ‖ T026 (storage), converging on T035–T038.
- **US2**: tests T040–T046 in parallel; T047 (storage) ‖ T051 (name substitution) ‖ T044's ui functions.
- **US3 ‖ US5 ‖ US6-crate** once US1 is done, if more than one pair of hands works the branch.

### Parallel example: User Story 1

```text
T018 storage list views test   T020 with: parse test     T023 gtk pane test
T019 list budget test          T021 with: executor test  T024 app_suite contacts_screen
T022 registry test
→ then: T026 storage reads ‖ T028/T029 with: ‖ T030 postio-ui::contacts
→ then: T031–T034 core + shell → T035–T038 widgets and wiring → T039 finder
```

## Implementation strategy

### MVP first

1. Phase 1 + Phase 2 (the person model, nothing visible changes).
2. Phase 3 — **US1**: the screen. Stop and validate: `app_suite::contacts_screen`, the budgets, a `/gtk-design` render.
3. Phase 4 — **US2**: join and names everywhere. With US1 this is the request as asked.

### Incremental delivery

The branch lands **once**, as one PR reviewed against spec.md (Development
Workflow). The story order still matters: each checkpoint is a green,
demonstrable state to commit and rebase from, and if the branch must land
before US4–US6, it can land at the US2 or US3 checkpoint with the remaining
stories' spec requirements filed as follow-up work through
`scripts/issue-file.sh` — the maintainer's call, not a default.

## Notes

- One commit per task (Phase 2 may be one commit; rule 2).
- Rebase onto `main` as you go; `main` moves under a branch this long, and the rebase is what finds a shared type's new callers.
- Land with `--detach` on the default tier; run the suites your diff touches first.
