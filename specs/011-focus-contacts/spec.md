# Feature Specification: Contacts for Postio Focus

**Feature Branch**: `feature/focus-contacts` (cut from `main` once spec 005's
backend port, `feature/contacts-mac`, has landed)

**Created**: 2026-10-19

**Status**: Draft, awaiting the maintainer's answers to the questions at the
end

**Input**: "Design the Contacts surface for Postio Focus, Mac first." Decided
by the maintainer before this spec: the surface is designed new for Focus;
it is built on the Mac first, through `postio-ffi`, `PostioKit` and
`PostioAppKit`, with its rules in the shared controller `postio-focus` where
they are cross-platform, so GTK can adopt it later the way spec 009's slices
did; and it is **Postio's own address book only** (the macOS system Contacts
stays only for recipient completion, spec 009 FR-051).

## Context

Spec 005 (`specs/005-contacts`, on `origin/feature/contacts`) specifies the
whole address book: a person who owns many addresses, groups, possible
duplicates, vCard import and export, `with:` search, editing, delete and
restore, and the person's name shown in the mail lists. It was drawn for the
old GTK shell (a sidebar entry, a reading-pane takeover, a `Context::Contacts`
list), which Postio Focus (spec 007, #1784) replaced. Another session is
porting 005's backend and command layers to `main` on `feature/contacts-mac`.

This spec is the **surface**: how a person gets to Contacts from Focus, what
the list and a person's page show, every verb by keyboard, import and export,
empty and failing states, and the Mac idioms. It does not restate 005's data
rules; it cites them.

| What | Where it is decided |
|---|---|
| A person owns addresses; join, detach, preferred; suppression; sightings; provenance; groups; suggestions; deletion and restore; vCard fidelity; name substitution in mail lists | Spec 005 (FR-010 to FR-061), unchanged. Cited as `005 FR-nnn` |
| Where Contacts is, what it looks like, which key does what, what it says when empty or failed, how it sits with Focus's one list and one dialog pattern | **This spec** |
| Behaviour (which row a key reaches, where the cursor goes after a join, the Esc ladder, which tab an empty state belongs to) | `postio-focus`, per ADR 0045; this spec states the rules, the plan says where they live |

### What this spec inherits

- **Spec 005** for the data model, the store, the commands' payloads and
  undo kinds, and `with:` search (`contracts/commands.md`,
  `contracts/query-with.md`, `contracts/vcard.md`). Where this spec changes a
  *binding*, a *context* or a *surface*, 005's command **ids** stay: ids are
  a file format (constitution II). Section "Command table" lists each id and
  what changed.
- **Specs 007, 009 and 010** in full. Focus has one list and no sidebar;
  everything else opens over it in one dialog pattern; `Esc` closes back to
  the same row; behaviour lives in `postio-focus` and the Mac draws intents
  (009 FR-002, ADR 0045); pickers at the row, the toast pill, the `?` key
  map, the one box; one secondary window at a time on the Mac (009 M4);
  results as a *mode* of the main window with History (010 D17); Mac-only
  commands carried by `registry::offered_on` until Linux adopts (010 D25).
- **The constitution**, in particular I (local-first), II (one command
  table), IV (test-first), V (performance, gated as counts), VI (privacy),
  VII (boundaries).
- **The design pack has no Contacts screens.** `Design/` holds the Focus
  macOS and Focus macOS search references; nothing in them draws a contact
  list or a person. This spec therefore describes the surface by analogy to
  screens that exist (the Filtered view's tabs, the message window, the
  label picker, the results mode). Question Q1 asks whether to commission
  screens; until then every layout here is a proposal, and the plan puts a
  design-review gate after the first slice that shows pixels.

### Decisions this spec takes

Each was open. The alternative rejected is recorded so it is not re-argued.

| # | Topic | Decision | Rejected |
|---|---|---|---|
| K1 | Getting there | `g p` ("people") from every surface that has the other `g` destinations; **Contacts** in the folders and places popover (`g l`'s sibling, Inbox ▾); in the command bar as a command and as `@` (finding one person); Go ▸ Contacts in the Mac menu bar; `@` on a message row or in an open message jumps to **that sender's** contact | `g c`, 005's and Gmail's key: Postio's `g c` is *Next scope* (PRODUCT.md §8 records Gmail's key displaced). A sidebar entry: Focus has none |
| K2 | Where it lives | Contacts is a **mode of the main window**, held by `postio-focus` with History entries, as 010's results view is: the list is replaced by the people list under the same toolbar and header strip. It is behind `Policy.caps.contacts_view` (true on the Mac, false on Linux until it adopts) | A window of its own: a second thing to manage beside the one list. A reading-pane takeover (005): Focus has no reading pane by default |
| K3 | A person's page | A person opens in the **secondary window**, as a message does: `Return` opens, `j`/`k` step to the next and previous person without closing, `Esc` or ⌘W closes and returns the keyboard to the same row with the selection kept. One secondary window at a time (009 M4) | An inline detail split beside the list: Focus's "one list, nothing beside it" rule, and `F8` reading beside the list is opt-in and for mail |
| K4 | The views | Five **tabs** in the header strip, switched with `1`–`5` as the Filtered view's reason tabs are: **1 Written to** (the default; 005 FR-005), **2 Everyone**, **3 Groups**, **4 Duplicates** (with a count), **5 Deleted** | Toggles on `v` sequences (005): `v` is Move now. A filter menu: five fixed views are better as numbered tabs, which the keyboard already knows |
| K5 | Finding | `/` in Contacts opens **the one box in its `@` (people) mode**, pre-set and filtering this list as you type. There is no contacts filter field and no second bar | 005's `contacts_filter` field: a new bar (ux-architect §6) |
| K6 | Editing | **Per field, in place, each commit a store write and an undo entry.** `e` puts the page's name, organisation and note and each address in editable fields; `Tab` moves between them, `Return` commits one, `Esc` reverts that field and a second `Esc` closes the page. There is no Save button and no "discard changes?" question | A form with Save/Cancel: it asks a confirmation where undo would do |
| K7 | Joining | `m` on a selection (or on a pair in Duplicates, or on one person: "Join with…" finds the other) opens a **picker at the row** as the label and snooze pickers are: the candidate names, `1`–`4` to pick one, `Tab` to type one, `Return` to confirm, the most recently seen name preselected. A conflict of organisations is a second step of the same picker. Undo is `z` / ⌘Z, and the toast says "Joined 2 contacts" | A join dialog: a modal for something undo reverses |
| K8 | Groups | A **Groups tab** whose rows are groups; `Return` on a group opens its members as the same list titled "Family" (Esc back to Groups). Adding people to groups is the **label picker's twin** (`l`: filter as you type, `Space` toggles, create from the typed text) | A groups sidebar: Focus has none |
| K9 | The word | The surface and its rows are **Contacts** and **contact** (005 and PRODUCT.md use it), the action is **Join** (005's word; the palette also finds it by "merge"), the bad copy is **Possible duplicates**. A person's preferred address is "Preferred"; the app never says "starred" or "primary" for it. New vocabulary, recorded here | "People" as the surface name; "Merge" as the verb |
| K10 | Neutral identity | A contact is drawn with **initials in a neutral circle**: no photo, no network, no hue (the accent is for markers and focus only, 009 FR-017) | A colour per person: spends the accent and invents a palette |
| K11 | Cursor and selection | Never conflated (constitution II). `x` marks, `m` joins the *marks* (or the cursor row when nothing is marked), `#` deletes the marks or the cursor row, `c` composes to them, `l` groups them | Verbs that act only on the cursor row |
| K12 | No confirmations | Join, detach, delete, restore, edit, group changes and import are `Recovery::Undo`. The only asked question is **Move** ("this address belongs to Grace Hopper") because it names an owner the person may not know about, and it is a popover that `Esc` cancels | A "Delete this contact?" dialog |

## The surface

### Entry and the ladder

```text
Inbox ── g p ──▶ Contacts (mode) ── Return ──▶ Person window
  ▲                │   ▲                          │
  └── Esc ─────────┘   └──────── Esc / ⌘W ────────┘
```

`Esc` climbs one rung per press, as 010 D18 does for results: popover or
picker, then the box (clearing its text), then the selection, then a group
back to the Groups tab, then Contacts back to the place it was opened from
(History: the inbox with its cursor and scroll, a message list, a search).

### The list (the main window in Contacts mode)

Under the unified toolbar (compose on the left; sync status and the search
field on the right, unchanged), the **header strip** reads
`Contacts · 1,204` on the left and the tabs on the right:

```text
Contacts · 1,204        1 Written to   2 Everyone   3 Groups   4 Duplicates 7   5 Deleted
```

One row per contact, **alphabetical by name**
under single-letter headers (A, B, …) set as the day headers are, windowed
over the paged store (never loaded whole). A row shows, left to right:

- a checkbox in the gutter when selected (as message rows);
- initials in a neutral circle (K10);
- the **name** (the user's, else the most recently seen, else the preferred
  address), with the organisation after it in the muted colour;
- the **preferred address**, muted, and `+2` when the person has more;
- a provenance mark for a contact the user made or imported (005 FR-005);
- **last in touch** (relative for today and this week, absolute beyond) and
  the number of messages, in the monospaced face.

Narrow windows drop columns from the right: message count, then last in
touch, then the address; the name never drops. The Duplicates tab's rows are
pairs (below) and the Groups tab's rows are groups (a name and "12 people").

**Footer** (the action bar, as for mail selections): with marks, the count and
the actions with their keys (`m` Join · `l` Group · `c` Compose · `#` Delete
· ⌘⇧E Export); without, the hint line "n new contact · / find · ⌘⇧I import".

### The person's page (the secondary window)

Sized and placed as the message window (009 M1): `clamp(640, W − 2·max(96,
0.18·W), 880)` wide, one centred column, `Esc` and ⌘W close. Top to bottom:

1. **Header**: initials, the name large, the organisation under it, and a
   provenance chip (`From mail`, `Made by you`, `Imported`). A *deleted*
   contact shows a "Deleted · R restores" strip instead of the verbs.
2. **Addresses**: one row per address, the preferred first and marked
   `Preferred`, each with its message count and when last seen. The rows are
   the page's cursor stops (`↓`/`↑`).
3. **Groups**: the groups it belongs to, as chips (`l` edits).
4. **Note**, and any vCard properties Postio does not model (phones,
   birthday, postal address), shown read-only (005 Assumptions).
5. **Mail**: "42 messages · last 3 days ago", and the five most recent
   conversations as rows (`Return` on one opens it in this window, as a
   digest's reference does; `Esc` returns).
6. **Action row** (the message window's pattern, each with its keycap):
   Compose `c` · Show mail `⌥↩` · Edit `e` · Add address `+` · Group `l` ·
   Join `m` · Delete `#`.

### Keys

All of Focus's bindings are Gmail's (PRODUCT.md §8); a command Gmail has no
key for keeps its own. Two new contexts carry the Contacts keys, both
`offered_on` Apple only until Linux adopts (010 D25): **`Contacts`** (the
list) and **`ContactCard`** (the person's page). Movement and selection reuse
the existing commands, which gain the contexts (cursor and selection stay
distinct): `j`/`k`, `g g`, `G`, `x`, `J`/`K`, `X`/`* a`, `Esc`, `z`.

The list and the page are context-scoped, so a letter may mean different
things in different contexts, as the Account list already does (`c` updates a
credential there and composes in mail; `m` sets the default account there).
No letter below collides *within* a context; the registry's
`bindings_do_not_collide_within_a_context` and `keybindings.md` generation
enforce it.

| Verb | Keys (Mac spelling) | Where | Recovery |
|---|---|---|---|
| Contacts | `g p` | every `g` destination surface | none |
| Open contact | `Return`, `o`, `→` | list | none |
| Next / previous contact | `j` `↓` / `k` `↑` (page: `j` / `k` only) | list, page | none |
| Next / previous row | `↓` / `↑` | page | none |
| Tabs | `1` `2` `3` `4` `5` | list | none |
| New contact (in Groups: new group is `g n`) | `n`, ⌘⌥N | list, page | Undo |
| Find | `/`, ⌘F | list | none |
| Compose to | `c`, ⌘N | list, page | none |
| Show mail | `⌥↩` | list, page | none |
| Edit (a group row: rename is `E`) | `e` | list, page | Undo |
| Join | `m` | list, page, Duplicates | Undo |
| Delete | `#`, `⌫` | list, page | Undo |
| Restore | `R` | Deleted tab, page | Undo |
| Add address | `+` | page | Undo |
| Detach address | `-` | page, on an address | Undo |
| Preferred | `P` | page, on an address | Undo |
| Not the same person | `-` | Duplicates | none |
| Group… | `l` | list, page | Undo |
| Leave this group | `L` | a group's members | Undo |
| New group | `g n` | list | Undo |
| Rename group | `E` | Groups | Undo |
| Import vCard… | ⌘⇧I | Contacts | Undo |
| Export vCard… | ⌘⇧E | Contacts | none |
| Contact of this sender | `@` | list, conversation, reader | none |
| Undo | `z`, ⌘Z | as for mail | none |

### Collisions found and how they are resolved

Checked against `crates/postio-core/src/registry.rs` and
`docs/keybindings.md` on `main` (2026-10-19). No Contacts command exists
there yet.

| Proposal | Collides with | Resolution |
|---|---|---|
| 005: `g c` opens Contacts | `next_scope` is `g c` (List, folder list) | Contacts is **`g p`** (K1) |
| 005: `d` deletes | `d` is *Digest rule…* in mail | Contacts delete is **`#` / ⌫**, Gmail's delete key; the id `contact_delete` stays and is bound to `#`, `Delete` |
| 005: `r` restores, `u` undoes | `r` is Reply; `u` is *Previous view* | Restore is **`R`** (the Filtered view's `R`, same meaning); undo is `z` / ⌘Z as everywhere |
| 005: `X` dismisses a suggestion | `X` selects all (spec 007 C19) | Dismiss is **`-`**, the dismiss key (`dismiss_marker`) |
| 005: `v e`, `v d`, `v s`, `v i`, `v x` | `v` is *Move to…* | No `v` sequences. Tabs are `1`–`5`; import and export are ⌘⇧I and ⌘⇧E |
| 005: `*` sets the preferred address | `* a` is *Select all*, so `*` is a chord prefix | **`P`** |
| 005: `Return`, `o` show mail | In Focus `Return` opens the thing | `Return` opens the contact's page; show mail is **`⌥↩`** |
| 005: `contacts_filter` on `/` | A second bar | `/` opens the one box in `@` mode (K5) |
| 005: `n`, `l`, `L`, `E`/`R` for groups | `n` Next in conversation; `l` Add label; `L` Digest mail like this; `E` Archive thread | All in contexts that are not those lists. Rename is **`E`** (not 005's `R`, which restore owns) |
| ⌘N | `compose` (009 M6) | In Contacts it composes **to the contact**, through `contact_compose`; new contact is ⌘⌥N |
| ⌘⇧I, ⌘⇧E | Registry has `ctrl+shift+…` bindings (`mod` becomes ⌘ on the Mac) that are composer-scoped | Different contexts; no collision |
| `@` as a key | The box's `@` mode when it is empty and typing | Single keys do not fire in a text field (009 FR-032), so `@` is a command only outside the box |

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Open Contacts and find a person (Priority: P1)

From the inbox the person presses `g p`. The inbox is replaced, with no
transition, by the Contacts list, alphabetical under letter headers, showing
people they have written to and people they have made. They press `/`, type
"ada", the list narrows as they type, and `Return` opens the person's page
in its own window. `Esc` closes the page onto the same row; `Esc` again
leaves Contacts for the inbox with the cursor where it was.

**Why this priority**: The surface does not exist without it, and it is
useful alone: it answers "who is this, and what have we said."

**Independent Test**: Over the demo store, `g p`, `/ada`, `Return`; the page
shows Ada's two addresses and their counts; `Esc` `Esc` `Esc` lands on the
original inbox row.

**Acceptance Scenarios**:

1. **Given** a store with mail exchanged with ten correspondents and one
   contact made by hand, **When** `g p` is pressed, **Then** the Written to
   tab shows the contacts the user made and those they have sent mail to, the
   cursor is on the first row, and no import was needed (005 FR-003).
2. **Given** newsletters the user never answered, **When** the list is shown,
   **Then** they are absent from tab 1, present in tab 2, and still offered
   by composer completion (005 FR-005).
3. **Given** `/` is pressed and "ada" typed, **When** a character is typed,
   **Then** only contacts whose name, organisation or any address matches
   remain, within the interaction budget, and the box shows the people mode
   marker; `Esc` clears the text, a second `Esc` closes the box.
4. **Given** the cursor on a message row in the inbox, **When** `@` is
   pressed, **Then** Contacts opens on that sender's page; `Esc` returns to
   the same message row.
5. **Given** the main window is narrower than 1024, **When** Contacts is
   shown, **Then** columns drop from the right and the name stays.
6. **Given** `?` is pressed in Contacts, **Then** every Contacts command
   appears with its binding, generated from the registry.

---

### User Story 2 - Read a person's page and act on it (Priority: P1)

On a contact the person presses `Return`. The page opens in its own window,
centred over the list, which stays visible. It shows the addresses (the
preferred first), groups, note, and recent mail. `c` composes to the
preferred address, `⌥↩` shows all mail from or to any of the person's
addresses as an ordinary editable search, `j`/`k` step to the next and
previous contact without closing.

**Why this priority**: Reading a person and writing to them is why the
surface exists.

**Independent Test**: Open a contact with two addresses; `c` opens a draft
addressed to the preferred one; `Esc` returns to the page; `⌥↩` lands on a
results view whose query names both addresses and finds the messages of both.

**Acceptance Scenarios**:

1. **Given** a contact with three addresses, **When** the page opens,
   **Then** each address shows with its message count and last seen, the
   preferred one first and marked.
2. **Given** the page is open, **When** `c` is pressed, **Then** the composer
   replaces the window (009 M4) addressed to the preferred address, and
   `Esc` or sending returns to the list on the same contact.
3. **Given** the page is open, **When** `⌥↩` is pressed, **Then** the main
   window shows a search whose visible, editable query is the one 005 FR-007
   defines for this person, and `Esc` returns to the Contacts list.
4. **Given** the page is open, **When** `j` is pressed, **Then** the next
   contact in the list shows in the same window at the same size.
5. **Given** a recent conversation row on the page, **When** `Return` is
   pressed, **Then** the message opens in the same window and `Esc` returns
   to the page.
6. **Given** a contact with a vCard phone number Postio does not model,
   **When** the page opens, **Then** it is shown read-only.

---

### User Story 3 - Create and edit a contact (Priority: P2)

`n` opens the page empty: name and one address. Typing the address and
`Return` on the last field creates the contact, which is offered in the
composer at once. `e` on an existing contact turns the page's fields
editable; each commit is saved at once and can be undone. Editing a contact
known only from mail makes it the user's own, keeping its history (005
FR-022).

**Why this priority**: Closes most of what an address book is for, and builds
on Story 2.

**Independent Test**: `n`, type "Grace Hopper", `Tab`, "grace@example.org",
`Return`; compose and type "gra": Grace is offered. Rename a mail-derived
contact and deliver another message from it: the name holds.

**Acceptance Scenarios**:

1. **Given** no mail from `grace@example.org`, **When** a contact is created
   with that address, **Then** composer completion offers it at once.
2. **Given** a mail-derived contact, **When** its name is committed,
   **Then** later mail with another display name does not overwrite it, and
   the message list shows the new name (005 FR-021, FR-032).
3. **Given** the name field holds uncommitted text, **When** `Esc` is
   pressed, **Then** the field reverts and the page stays; the next `Esc`
   closes it.
4. **Given** a new, empty page, **When** `Esc` is pressed, **Then** nothing is
   created and no question is asked.
5. **Given** a create with no address, **When** `Return` is pressed on the
   name, **Then** the focus moves to the address field with the hint "A
   contact needs at least one address"; nothing is saved.
6. **Given** any edit, **When** it is made, **Then** it is visible at once
   and waits on no network; `z` undoes it and the toast names it.

---

### User Story 4 - Delete, and bring back (Priority: P2)

`#` (or ⌫) on a contact, or on several marked, deletes them: they leave the
list, the composer and the `@` finder, and do not return when the next
newsletter arrives (005 FR-023). The pill says "Deleted Ada Lovelace · Undo
⌘Z". Later, tab 5 shows deleted contacts; `R` brings one back whole, history
included (005 FR-023a).

**Why this priority**: Mailing-list robots are most of a mailbox's
"contacts"; deletion is the cleaning tool.

**Independent Test**: Delete a mail-derived contact, deliver mail from its
address, confirm it is absent from tabs 1 and 2 and from completion; restore
it from tab 5 and confirm its counts include the mail that arrived meanwhile.

**Acceptance Scenarios**:

1. **Given** three marked contacts, **When** `#` is pressed, **Then** they
   leave the list, the cursor lands on the row after the last one, and the
   pill reads "Deleted 3 contacts".
2. **Given** the pill is showing, **When** `z` is pressed, **Then** all
   three return with their addresses and details and the cursor goes to the
   first.
3. **Given** a deleted contact, **When** tab 5 is opened and `R` pressed,
   **Then** it returns exactly as before, is offered in completion again,
   and tab 5 shows its empty state when it was the last.
4. **Given** the page of a deleted contact is open, **When** it is read,
   **Then** it carries the "Deleted · R restores" strip and no verb other
   than restore and compose works on it.

---

### User Story 5 - Join the same person's addresses (Priority: P1)

The person sees "Ada Lovelace" twice. They mark both with `x`, press `m`,
and a picker at the row offers the names the addresses were seen with, the
most recent preselected; `Return` confirms. One contact remains holding both
addresses, the history of both, and every group and note (005 FR-012,
FR-013). `z` undoes it exactly. Alternatively the Duplicates tab shows the
pair with its evidence; `m` then `Return` joins it, and `-` says they are
not the same person.

**Why this priority**: This is what the original request asked for by name.

**Independent Test**: With mail from `ada@example.com` and `ada@example.org`
both seen as "Ada Lovelace", the Duplicates tab shows the pair with "same
name · 14 and 3 messages"; `m` `Return` joins them in two keys; the list
shows one row; `z` restores two.

**Acceptance Scenarios**:

1. **Given** two marked contacts, **When** `m` is pressed, **Then** a picker
   opens at the cursor row listing the candidate names with `1`–`4`, the most
   recent preselected, `Tab` for a typed name.
2. **Given** the picker, **When** `Return` is pressed, **Then** the join
   happens at once, the list redraws with one row, the cursor is on the
   joined contact, and the pill says "Joined 2 contacts · Undo ⌘Z".
3. **Given** joined contacts with different organisations, **When** the name
   is confirmed, **Then** a second step of the same picker asks which
   organisation to keep; nothing the user entered is discarded without that
   choice (005 FR-013).
4. **Given** a single contact with no marks, **When** `m` is pressed,
   **Then** a finder at the row lists other contacts as the box's people mode
   does; choosing one continues as above.
5. **Given** the Duplicates tab, **When** `-` is pressed on a pair, **Then**
   it leaves the tab, is never offered again (005 FR-019), and the tab's
   count drops; there is no undo and no confirmation, since it hides a hint,
   not data.
6. **Given** the Duplicates tab, **When** the person does nothing, **Then**
   nothing is joined (005 FR-018).
7. **Given** a joined contact, **When** new mail arrives from either of its
   addresses, **Then** it counts toward that contact and no new one appears
   (005 FR-017).

---

### User Story 6 - Several addresses, one preferred (Priority: P2)

On a person's page `+` adds an address by typing it. If it belongs to someone
else, a popover names them and offers "Move it here" (`Return`) or leaves it
(`Esc`). `-` detaches the address under the cursor into a contact of its own;
`P` makes it the preferred one, which composition and completion offer first.

**Why this priority**: The rest of "one person, many addresses" (005 FR-014
to FR-016).

**Independent Test**: Add `ada@example.net` to Ada; press `P` on it; `c`
composes to it. Add `grace@example.org` (owned by Grace) and see the popover
name Grace.

**Acceptance Scenarios**:

1. **Given** a person with three addresses, **When** `-` is pressed on one,
   **Then** it becomes a contact of its own with its history, and the page
   keeps two.
2. **Given** a person with one address, **When** `-` is pressed on it,
   **Then** it is refused with "A contact needs at least one address · # to
   delete the contact".
3. **Given** an address owned elsewhere, **When** it is added, **Then** the
   popover reads "<address> belongs to Grace Hopper · Return moves it here ·
   Esc leaves it" and nothing else changes until the person chooses.
4. **Given** a suppressed address, **When** it is added or a contact is
   created with it, **Then** it is offered again and its earlier history is
   kept (005 FR-024).

---

### User Story 7 - Groups (Priority: P3)

`l` on a contact (or marks) opens a picker with the existing groups: type to
filter, `Space` toggles membership, typing a name that does not exist offers
"Create group Family" (`Return`). Tab 3 lists groups; `Return` on one shows
its members, `E` renames it, `#` deletes it (undoable), and `L` in the
members view takes the contact out. `c` on a group row composes to its
members' preferred addresses; `group:family` finds their mail (005 FR-040
to FR-042).

**Why this priority**: The store and the query language have groups; this
gives them a place.

**Independent Test**: Create "Family" from the picker on two marked contacts,
open tab 3, open the group, compose to it and see both preferred addresses.

**Acceptance Scenarios**:

1. **Given** two marked contacts, **When** `l`, "Family", `Return` are
   pressed, **Then** the group exists with both as members and the pill says
   "Added 2 contacts to Family".
2. **Given** tab 3, **When** `Return` is pressed on a group, **Then** the
   members show under the title "Family", `Esc` returns to tab 3 with the
   cursor on the group.
3. **Given** a group with a member of two addresses, **When** `c` is pressed
   on the group, **Then** the draft is addressed to that member's preferred
   address (005 FR-041).
4. **Given** a group is deleted, **When** `z` is pressed, **Then** it returns
   with its members.

---

### User Story 8 - Import and export vCard (Priority: P4)

⌘⇧I opens the system open panel for `.vcf` files (several may be chosen), or
the person drops a `.vcf` on the window. A summary sheet says what happened:
"Imported 41 contacts and 2 groups. Joined 3 into people you already had.
Skipped 2 cards that could not be read." ⌘⇧E opens the save panel: it
exports the marked contacts, or with none the contacts the tab and filter
currently show; deleted contacts are never exported (005 FR-050).

**Why this priority**: Real, independent of the rest, and the bridge from the
macOS Contacts app (which Postio otherwise never reads for its address book).

**Independent Test**: Import a fixture 3.0 file with a two-address card, a
phone number and a vendor property, and a group card; export; every
unmodelled property is reproduced as 005 FR-051 says.

**Acceptance Scenarios**:

1. **Given** the open panel, **When** two files are chosen, **Then** both are
   imported in one summary.
2. **Given** a card whose address already belongs to a contact, **When** it is
   imported, **Then** they become one contact and the summary lists the join
   (005 FR-053).
3. **Given** an unreadable file, **When** it is chosen, **Then** the sheet
   says "<name> is not a vCard file" and nothing is imported.
4. **Given** the Written to tab filtered by "ada", **When** export is run
   with no marks, **Then** the file holds exactly the contacts shown.
5. **Given** the save panel is cancelled, **Then** nothing is written and no
   message appears.
6. **Given** a `.vcf` dropped on the main window outside Contacts, **Then**
   Contacts opens and the import runs as ⌘⇧I would.

---

### User Story 9 - Contacts everywhere else (Priority: P3)

Recipient completion shows a contact **once**, under one name, with each of
its addresses beneath the name and the preferred first, ranked above people
known only from mail (005 FR-030), merged with macOS Contacts suggestions
(009 FR-051) by address with Postio's contact winning. A name the user set
reads in the message list, the conversation and the reader in place of the
header's display name (005 FR-032), and `@` in the box finds a contact by any
address.

**Why this priority**: What makes the address book a part of mail rather than
a screen beside it. Mostly the 005 port; this story is that the Mac's
surfaces show it.

**Independent Test**: With "Ada Lovelace" joined across two addresses, type
"ada" in the composer's To field and see one entry with two addresses; the
list rows from either address read "Ada Lovelace".

**Acceptance Scenarios**:

1. **Given** a joined contact and an identical address in macOS Contacts,
   **When** "ada" is typed, **Then** one entry shows, with Postio's name.
2. **Given** macOS Contacts permission was refused, **When** the composer
   completes, **Then** Postio's contacts still complete and no prompt
   reappears (009 edge case).
3. **Given** the name was set, **When** the inbox list is shown, **Then**
   rows from every address of that contact read the set name, and the reader's
   header details still show the name and address as the mail carried them.

---

### Edge Cases

- **Empty, never blank.** Each tab owns an empty state, with a key:
  - *Written to*, nothing yet: "No one yet. People you write to, and people
    you add, appear here. 1,204 people are known from mail: press 2. `n` adds
    someone · ⌘⇧I imports." (the count is real; when it is 0 the sentence
    drops).
  - *Everyone*, before the first sync: "Nothing synced yet. People appear as
    mail arrives." with the sync strip (009 FR-016) beneath the header.
  - *Duplicates*: "No possible duplicates."
  - *Groups*: "No groups yet. `l` on a contact makes one."
  - *Deleted*: "Nothing deleted. `#` on a contact moves it here."
  - *A filter with no match*: "No one matches 'xyz'. `Esc` clears the filter
    · `n` adds 'xyz'" (the typed text pre-fills a new contact's name or
    address).
- **Loading.** Local reads are instant; there is no spinner. A list that is
  still paging shows what has landed (the list never waits).
- **Offline, failing, first sync.** Contacts reads and writes the local store
  and works with the network absent; the sync strip and the toolbar label
  tell the sync state as on every surface. A person added offline needs no
  queue: there is no remote half (005, CardDAV out of scope).
- **A write fails** (the store is locked or full): the field or verb reverts,
  the pill reads "Couldn't save. Nothing changed. <reason>"; the controller
  keeps the intent, so pressing the same key again retries it.
- **An import that fails midway** (disk, an unreadable file): the sheet names
  how many cards were read and applied, and the import is one undo entry, so
  `z` removes what it applied (Q3).
- **Mixed ownership.** Deleting a contact made of mail-derived and
  user-made addresses hides it whole (005); the page of a deleted contact is
  read-only except restore.
- **Forged display names.** A name header never turns an unowned address into
  a contact (005 edge case); the list shows only owned addresses.
- **The user's own addresses** never appear as contacts (005).
- **A very large store.** 20,000 contacts: the list opens, scrolls and
  filters within the budgets and is never loaded whole.
- **Two windows.** If a message window is open, `g p` is not reachable from
  it (as `g` destinations are not); in the main window, opening a person's
  page closes the message window (009 M4) and reports it closed.
- **Reduced motion.** There are no transitions to reduce: mode and tab
  changes are immediate. The pill fades as it does elsewhere and honours the
  setting (009).
- **Input method composing.** No key is taken as a command (009 edge case).
- **Rebinding.** Every key here is overridable under `[keys]` by command id;
  a key rebound to one a Mac system shortcut owns is reported and the default
  kept (009).
- **Full keyboard access and VoiceOver.** A row is read as "Ada Lovelace,
  Lovelace Analytics, 3 addresses, last in touch 3 days ago, 42 messages,
  marked"; every verb is a menu item and a palette entry, and reachable
  without a pointer (SC-008).
- **Mouse.** A click moves the cursor, a double-click opens, ⌘-click toggles
  a mark, ⇧-click ranges, right-click shows the row menu generated from the
  registry; dragging a contact onto a group in tab 3 adds it. Dragging never
  joins (K7: a join always names its result).

## Requirements *(mandatory)*

### Functional Requirements

**Getting there and getting back**

- **FR-001**: `g p` MUST open Contacts from every surface that offers the
  other `g` destinations; Contacts MUST also be offered in the places
  popover, the command bar, Go ▸ Contacts in the menu bar, and `@` on a
  message row or open message (the sender's page).
- **FR-002**: Contacts MUST be a mode of the main window held by the shared
  controller, behind `Policy.caps.contacts_view` (true on the Mac). Entering
  and leaving MUST add a History entry; leaving MUST return to the place
  entered from with its cursor, selection and scroll.
- **FR-003**: `Esc` MUST climb one rung per press: picker or popover, box
  text, selection, a group's members, Contacts.
- **FR-004**: A person's page MUST open in the secondary window (one at a
  time, 009 M4), centred over the main window, with the list visible and not
  dimmed; `Esc` and ⌘W MUST close it and return the keyboard to the same row
  with the selection kept.

**The list**

- **FR-010**: The list MUST be windowed over the paged store and MUST never
  load every contact; alphabetical by displayed name under letter headers.
- **FR-011**: Tabs `1` to `5` MUST switch Written to, Everyone, Groups,
  Duplicates and Deleted, without animation, keeping each tab's cursor. The
  default MUST be Written to (005 FR-005). Duplicates MUST show a count.
- **FR-012**: A row MUST show initials in a neutral circle, the name, the
  organisation, the preferred address with `+n`, provenance, last in touch and
  message count, and MUST drop columns from the right before the name.
- **FR-013**: `/` MUST open the one box in its people mode, filtering the
  current tab by name, organisation and every address as the person types
  (005 FR-004). No other filter field may exist.
- **FR-014**: Cursor and selection MUST be separate; `x`, `J`, `K`, `X` and
  `* a` MUST work as in mail; every verb acts on the marks when there are
  any, otherwise on the cursor row.
- **FR-015**: With marks, the footer MUST show the count and the actions with
  their keycaps; without, a one-line hint.

**The page**

- **FR-020**: The page MUST show what 005 FR-006 lists, in the order of the
  surface section, with vCard properties Postio does not model shown
  read-only.
- **FR-021**: `j`/`k` MUST step to the next and previous contact in the list
  without closing or resizing the window; `↓`/`↑` MUST move among the
  page's rows.
- **FR-022**: A recent conversation row MUST open in the page's own window
  and `Esc` MUST return to the page.
- **FR-023**: Compose MUST open a draft addressed to the preferred address
  (the marked contacts' for several; a group's members' for a group), in the
  composer's place (009 M4). Show mail MUST open a results view for the
  query 005 FR-007 specifies, visible and editable in the box.

**Create, edit, delete, restore**

- **FR-030**: `n` MUST open an empty page that creates the contact when a
  name or address and at least one address are committed (005 FR-020);
  `Esc` MUST cancel an unsaved page without a question.
- **FR-031**: `e` MUST make the page's fields editable; each commit MUST be
  one store write and one undo entry; `Esc` MUST revert the field being
  edited and a second `Esc` close the page; single-letter keys MUST NOT fire
  while a field has the keyboard (009 FR-032).
- **FR-032**: Delete (`#`, `Delete`, ⌫) MUST delete the marks or the cursor
  row, and a group row's delete MUST delete the group; both are
  `Recovery::Undo`. The cursor MUST land on the row after the last deleted,
  and return on undo to the first restored.
- **FR-033**: `R` MUST restore a deleted contact whole (005 FR-023a); a
  deleted contact's page MUST show the restore strip.
- **FR-034**: Every verb MUST take effect locally and visibly without
  waiting on the network and MUST name itself in the pill and in Edit ▸ Undo.

**Joining, addresses, groups**

- **FR-040**: `m` MUST open a picker at the row that names the joined
  contact (the user-set and seen names, the most recent preselected, a typed
  one allowed), then, only on a conflict, asks which organisation to keep
  (005 FR-013). Nothing is joined until the picker is confirmed.
- **FR-041**: `m` on a single contact MUST offer a finder for the other; `m`
  in Duplicates MUST act on the pair under the cursor.
- **FR-042**: The Duplicates tab MUST show each pair with the evidence (the
  shared name and each address's message count); `-` MUST dismiss a pair for
  good (005 FR-019); the app MUST NOT join without the person's action
  (005 FR-018).
- **FR-043**: `+` MUST add an address; an address owned by another contact
  MUST be answered with a popover naming the owner and offering to move it
  (005 FR-015); `-` on an address MUST detach it (refused for the last, 005
  FR-014); `P` MUST set it preferred (005 FR-016).
- **FR-044**: `l` MUST open a group picker (filter as you type, `Space`
  toggles, a typed name offers "Create group"); the Groups tab MUST list
  groups with member counts, `Return` open one, `E` rename, `#` delete, `L`
  remove a member (005 FR-040).

**Import and export**

- **FR-050**: ⌘⇧I MUST present the system open panel for `.vcf` files (one or
  more) and a drop of `.vcf` files on the main window MUST do the same;
  nothing is read until the person chooses a file.
- **FR-051**: Import MUST end with a summary sheet stating contacts and
  groups made, joins to existing contacts (each listed, 005 FR-053), and
  cards skipped with the reason (005 FR-054). It MUST be dismissable with
  `Esc` or `Return`.
- **FR-052**: ⌘⇧E MUST present the save panel and export the marked
  contacts or, with none, exactly the contacts the tab and filter show,
  never deleted ones (005 FR-050, FR-051).
- **FR-053**: The vCard mapping and fidelity MUST be 005's
  (`contracts/vcard.md`); this spec adds no vCard rule.

**Command table and discovery**

- **FR-060**: Every command MUST be in the registry with a binding, a
  palette entry and an accessible action, derived from the one table; the
  menu bar's Contacts items, the `?` key map, the row menu and the keycaps
  MUST come from it (constitution II). `docs/keybindings.md` MUST regenerate.
- **FR-061**: The Contacts commands MUST be offered on Apple only until
  Linux adopts (`registry::offered_on`), so GTK's keymap and
  `keybindings.md` Linux rows do not change.
- **FR-062**: `contacts_import` and `contacts_export` MUST be on the
  frontend-presented list (`Intercepted` on both sides of the boundary,
  `macos/CLAUDE.md`) because a panel is toolkit-only.
- **FR-063**: Words MUST come from `postio-ui`, not Swift literals; the
  vocabulary decisions K9 apply.

**Where contacts reach the rest of Postio**

- **FR-070**: Recipient completion MUST show a contact once with each
  address and the preferred first, ranked above mail-only people (005
  FR-030), merged with macOS Contacts suggestions by address with Postio's
  contact winning, and MUST fall back to Postio's contacts alone when the
  system permission is refused or revoked.
- **FR-071**: The `@` mode of the box MUST find a contact by name or any
  address (005 FR-031).
- **FR-072**: A name the user set MUST read in the Focus list, conversation
  and reader for every address of the contact (005 FR-032), and the reader's
  header details MUST show the mail's own name and address.

**Privacy, speed, accessibility**

- **FR-080**: Contacts MUST make no network request: no avatar, favicon or
  directory lookup (005 FR-060). Nothing MUST read the macOS Contacts store
  for the address book; only completion does, as 009 FR-051 has it. A file is
  read or written only after the person chose it in a panel or dropped it.
- **FR-081**: Logs MUST carry ids, counts and outcomes, never a name,
  address or note (005 FR-061).
- **FR-082**: The list MUST open and scroll, and the filter MUST answer,
  within the budgets of the constitution (Success Criteria), gated as counts
  with `postio_storage::test_support::counting`.
- **FR-083**: Every verb MUST be completable without a pointer, and the list,
  page and sheet MUST be labelled for VoiceOver and honour full keyboard
  access and reduced motion.
- **FR-084**: Rows and states MUST use the system semantic colours; the
  accent MUST appear only on the focus ring and the Duplicates count marker
  (009 FR-017).

**Verification**

- **FR-090**: Every rule above MUST be tested in the shared layer first
  (`postio-focus`, `postio-ui`), the FFI exports in `ffi_suite`, and the Swift
  view models and keyboard paths without a window (009 FR-062).
- **FR-091**: Each interaction MUST be written as a storyboard in
  `storyboards/contacts/`, red on the base, before it is built (CLAUDE.md,
  009 FR-063). The Mac runner is out of scope; a Mac landing that changes an
  interaction is labelled `interactions-unreviewed` until Linux adopts and
  films them.
- **FR-092**: A screen MUST NOT be called done until it has been captured at
  1440×900 in light and dark and reviewed (Q1 says against what).

### Key Entities

- **Contacts mode**: the main window showing the people list. Holds the tab,
  the box text, the cursor, the selection and, in Groups, the open group.
- **Person's page**: the secondary window showing one contact; holds which
  field is being edited and the row cursor.
- **Contact, address, group, suggestion**: 005's entities, unchanged.
- **Pickers**: the join name, the organisation conflict, the group toggle
  and the move question, all the pickers pattern.
- **Import summary**: counts, joins and skipped cards, kept until dismissed.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: On a store with 20,000 contacts, `g p` shows the first rows
  within the interaction budget, and every key press, tab change, scroll
  step and keystroke of the filter is answered within 16 ms, gated as
  statement and row counts.
- **SC-002**: The filter answers within 100 ms on the same store.
- **SC-003**: A person can join the pair in Duplicates in two key presses
  (`m`, `Return`), join two named contacts in five (`x`, `j`, `x`, `m`,
  `Return`), and undo in one.
- **SC-004**: A person with several addresses appears once in the list, in
  completion and in the `@` finder, and "show mail" returns every message
  involving any of their addresses (005 SC-003).
- **SC-005**: Every Contacts verb is completable without a pointer and every
  one appears in the key map, the palette and the menu bar.
- **SC-006**: A deleted mail-derived contact stays absent after any number of
  further messages, and `R` brings it back with the history gathered while
  deleted (005 SC-005).
- **SC-007**: A vCard imported and exported reproduces what 005 SC-006
  requires.
- **SC-008**: Contacts produces no network traffic and reads the macOS
  Contacts store only for completion.
- **SC-009**: Every tab has an empty state with a key, and no state is a
  blank frame or a bare "something went wrong".
- **SC-010**: `bindings_do_not_collide_within_a_context` and the generated
  `keybindings.md` pass with the Contacts commands, and the Linux rows of
  `keybindings.md` are unchanged by this feature's Mac slices.

## Command table

005's ids stay. This table lists each id, its binding here (replacing 005's
proposal where they differ), and what is new. All are `offered_on` Apple
until Linux adopts (FR-061).

| id | Title | Binding | Contexts | Recovery | vs 005 |
|---|---|---|---|---|---|
| `open_contacts` | Contacts | `g p` | the `g` surfaces | None | was `g c` |
| `contact_open` | Open contact | `Return`, `o`, `Right` | Contacts | None | new |
| `next_contact`, `prev_contact` | Next / previous contact | `j` `Down` / `k` `Up` | Contacts, ContactCard | None | new |
| `card_next_row`, `card_prev_row` | Next / previous row | `Down` / `Up` | ContactCard | None | new |
| `contacts_written_to` | Written to | `1` | Contacts | None | new |
| `contacts_toggle_everyone` | Everyone | `2` | Contacts | None | was `v e` |
| `contacts_groups` | Groups | `3` | Contacts | None | new |
| `contacts_suggestions` | Possible duplicates | `4` | Contacts | None | was `v s` |
| `contacts_toggle_deleted` | Deleted | `5` | Contacts | None | was `v d` |
| `contacts_filter` | Find contact | `/`, `mod+f` | Contacts | None | opens the box in people mode |
| `contact_new` | New contact | `n`, `mod+alt+n` | Contacts, ContactCard | Undo | same |
| `contact_edit` | Edit | `e` | Contacts, ContactCard | Undo | was `e`, now also a group's rename-on-Return |
| `contact_delete` | Delete | `#`, `Delete` | Contacts, ContactCard | Undo | was `d` |
| `contact_restore` | Restore | `R` | Contacts (Deleted), ContactCard | Undo | was `r` |
| `contact_join` | Join | `m` | Contacts, ContactCard | Undo | same |
| `contact_add_address` | Add address | `+` | ContactCard | Undo | same |
| `contact_detach_address` | Detach address | `-` | ContactCard | Undo | same |
| `contact_set_preferred` | Preferred | `P` | ContactCard | Undo | was `*` |
| `contact_show_mail` | Show mail | `alt+Return` | Contacts, ContactCard | None | was `Return` |
| `contact_compose` | Compose to | `c`, `mod+n` | Contacts, ContactCard | None | same, plus ⌘N |
| `suggestion_dismiss` | Not the same person | `-` | Contacts (Duplicates) | None | was `X` |
| `contact_group_new` | New group | `g n` | Contacts | Undo | same |
| `contact_group_rename` | Rename group | `E` | Contacts (Groups) | Undo | was `R` |
| `contact_group_add` | Group… | `l` | Contacts, ContactCard | Undo | same |
| `contact_group_remove` | Leave this group | `L` | Contacts (a group) | Undo | same |
| `contacts_import` | Import vCard… | `mod+shift+i` | Contacts | Undo | was `v i`; undoable (Q3) |
| `contacts_export` | Export vCard… | `mod+shift+e` | Contacts | None | was `v x` |
| `contact_of_sender` | Contact of this sender | `@` | List, conversation, reader | None | new |

`toggle_selection`, `extend_selection_*`, `select_all`, `first_message`,
`last_message`, `back` and `undo` gain the two contexts rather than being
duplicated.

## Assumptions

- Spec 005's backend and command layer lands on `main` before this branch is
  cut; this spec consumes its reads (list pages by tab and filter, a
  contact's detail, suggestions, groups) and writes (`contracts/commands.md`)
  and adds none to the store.
- The Mac is the only app that offers these commands at first; Linux and the
  terminal adopt later (the plan's later slices).
- Alphabetical order is the only order in v1; recency is what Written to
  already means by membership. This is the UX architect's call, not asked.
- Fields edited are 005's: name, addresses, organisation, note. Other vCard
  properties are kept and shown read-only.
- No ADR: the rule that outlives the feature (behaviour in `postio-focus`) is
  ADR 0045; the decisions here are the feature's (constitution, "A spec
  records its decisions").
- Importing from the macOS Contacts app is done by exporting a vCard from it
  and importing the file; Postio never reads the system address book for its
  own (maintainer's decision).

## Out of Scope

CardDAV and any remote address book; reading the macOS address book for
Postio's own contacts; photos and avatars; birthdays and other fields as
editable; a Finder `.vcf` file association and dragging a contact out as a
`.vcf` (a later slice, Q4); Linux and terminal adoption (the plan's later
slices); contact-based rules, notifications or scoring.

## Questions for the maintainer

**Resolved 2026-10-19.** The maintainer approved the spec to build ("go
ahead and build it") after asking why editing has no Save button and
hearing the trade-off against an Edit/Done mode. Every question below takes
its recommended option (A), and K6's per-field editing stands. The record
of the options is kept here for whoever revisits them.

**Q1. Is there, or should there be, a design reference for Contacts?** The
`Design/` pack has none; every layout here is by analogy.
- A. Build from this description, review the first slice's screenshots
  against it, and adjust. *(Recommended: no new design work blocks the
  engine slices, and the plan's review gate after slice 1 catches drift.)*
- B. Commission screens first (list, page, the join picker, the import
  sheet, five empty states), as spec 009 and 010 had.
- C. Skip the review gate.

**Q2. The entry key.** `g p` (this spec) because `g c` is *Next scope*.
- A. `g p`. *(Recommended: nothing is displaced.)*
- B. Give Gmail's `g c` to Contacts and move *Next scope* elsewhere, which
  disturbs a binding people already use.
- C. `g p` and also ⌘⌥C (Mac chord in the Go menu).

**Q3. Should import be one undoable step?** 005 says import is not undoable
(its summary lists the joins). With a one-transaction import and one undo
entry the person can undo a bad file with `z`, which this spec asks for.
- A. One undo entry. *(Recommended: a wrong 4,000-card file otherwise needs
  4,000 deletes; it costs the port one entry kind.)*
- B. As 005 says: no undo, summary only.

**Q4. A Finder `.vcf` association and drag-out.** Double-click on a `.vcf`
opening Postio's import, and dragging a contact to Finder to make one.
- A. Later slice, after the surface is proven. *(Recommended.)*
- B. In slice 7 with import.
- C. Never.

**Q5. The word "Join".** 005 chose Join; `m` is the key because Gmail's `m`
(mute) has no Postio meaning here.
- A. Keep "Join", find it by "merge" in the palette. *(Recommended.)*
- B. Rename to "Merge" (the key then matches), changing 005's user words.

**Q6. Should the Duplicates tab's count ever appear outside Contacts**
(for instance a dot on Inbox ▾ or the `g p` entry)?
- A. No: a nudge is a notification; the count shows only inside Contacts.
  *(Recommended: matches "never a banner".)*
- B. A quiet count beside Contacts in the places popover.
