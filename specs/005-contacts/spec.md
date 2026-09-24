# Feature Specification: Contacts

**Feature Branch**: `feature/contacts`

**Created**: 2026-09-23

**Status**: Draft

**Input**: User description: "contact screen and features for managing contacts. it should load from the existing info we have from emails but we should be able to map multiple emails to a single contact and things like that"

## Context

**This specifies the whole address book, not only the parts that are
missing.** Much of the foundation exists; the table says which. That is
*implementation status, not scope*: a requirement here holds whether it is met
today or not, and a test may be written against either kind.

| Asked for | State today |
|---|---|
| Loading from the mail we already have | Built. Every address seen on a message becomes a contact row with a sighting count and a last-seen time, counted once per message |
| Completion in the composer, and the `@` finder | Built, ranked: contacts the user created or imported above mail sightings, recency within a band |
| Creating a contact that never sent mail; deletion that stays deleted | Built in the store, no surface |
| Groups, and `group:` in search | Built in the store and in the query language, no surface |
| **A contacts screen** | **Not built** (#477) |
| **Several addresses belonging to one person** | **Not built, and not possible in the current model** — see below |
| vCard import and export | **Not built** (#475) |

### One person, many addresses — the decision this spec takes

The store's contact today *is* an address: one row per normalised address, and
the name, the sighting counts and the tombstone all live on it. That was
correct for completion, and it is why "Ada from work" and "Ada at home" are
two strangers to Postio.

**Decision: a contact is a person, and a person has one or more addresses.**
An address belongs to at most one contact. The evidence the mail provides —
how often and how recently an address was seen, and through which account —
stays with the *address*, because it is a fact about that address; the
contact's standing is derived from its addresses. The name the user gives, the
organisation and the note belong to the *person*.

Every address seen in mail starts as its own contact, exactly as today. The
user joins them. Postio never joins two addresses on its own, however alike
they look, because two people called "Alex Chen" are common and being
merged with a stranger is worse than having a duplicate; it may *suggest* a
join (Story 4).

**What survives from ADR 0007, and what does not.** ADR 0007 is about this
feature and nothing else, so per the Development Workflow it is folded into
this spec and deleted in this branch, with its citations re-pointed here.
What still holds, and is inherited here as requirements:

- Mail sightings and user-made contacts are one population with a
  **provenance** (from mail, made by the user, imported), not two address
  books; editing a mail-derived contact promotes it in place (Q1's reasoning,
  now at the person level).
- **Deleting stays deleted**: a deleted mail-derived address is remembered
  as suppressed so the next message from it does not bring it back (Q2).
- **Groups are named sets of people**, expanded to addresses at the moment
  they are picked in the composer, and `group:` in search means "from or to
  any member" (Q3).
- **vCard import keeps what Postio does not understand, verbatim**, reads
  3.0 and 4.0, writes 4.0 (Q4). Q4's "no third-party parser" does not
  survive: a Pimalaya parser now round-trips byte-for-byte, and the
  constitution's Pimalaya-first rule prefers it (`research.md` R9).
- **Contacts are shared across accounts; sightings are per account** (Q5).
- **Explicit beats frequent** in completion ranking (Q6).
- **Managing takes over the reading pane; finding stays in the `@` finder**
  (Q7). CardDAV remains out of scope and left possible (Q8).

What does *not* survive is Q1's "identity is the normalised address" at the
level of the contact: the address remains the identity of a *sighting*, and
the contact becomes the thing that owns sightings.

## Clarifications

### Session 2026-09-23

- Q: When a person is picked in the `@` finder or "show mail" runs, is the search a `contact:` term resolved at search time, or the person's addresses written out? → A: The addresses written out, at the moment of picking (no `contact:` field).
- Q: Does a join of people with no user-set name give the joined person a name that then shows everywhere? → A: Yes — every join ends with a name, picked from the names the addresses were seen with (most recent preselected) or typed; it counts as user-set.
- Q: Which people does an export with no selection contain? → A: Exactly the people the list currently shows (its filter and the "everyone from mail" toggle included); never suppressed people.
- Q: After the undo window, can deleted people be seen and brought back? → A: Yes — a "Deleted" filter on the Contacts list shows them, and a "restore" command returns one whole, history included.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - See the people I correspond with (Priority: P1)

The user opens Contacts from the sidebar or the palette. It takes over the
reading pane and shows the people Postio knows about — built from the mail
already in the store, with no import and no setup — each with a name, their
addresses, and when they were last in touch. The user can type to narrow the
list, walk it with the keyboard, and open one person to see their details and
jump to their mail. `Esc` returns to exactly where they were.

**Why this priority**: Without the screen nothing else in this spec has a
place to happen, and it is useful alone: it answers "who is this, and what
have we said" for anyone the user has exchanged mail with.

**Independent Test**: With a store holding mail from a handful of fixture
correspondents, open Contacts, narrow by typing part of a name, open one, and
confirm the addresses and last-contact time a person would read match the
mail; press the "show mail" command and land on a search for that person's
messages; press `Esc` and land back on the same message, same selection.

**Acceptance Scenarios**:

1. **Given** a store with mail exchanged with ten correspondents and no
   contacts ever made by hand, **When** the user opens Contacts, **Then** all
   ten appear, each with the name the mail carried, without any import step.
2. **Given** a store that also holds newsletters the user never replied to,
   **When** the user opens Contacts, **Then** those senders are not in the
   default view, appear when the user toggles "everyone from mail", and are
   still offered by composer completion.
3. **Given** the Contacts screen is open, **When** the user types "ada",
   **Then** only people whose name, organisation or any address matches
   remain, and the first match is selected.
4. **Given** a person is selected, **When** the user invokes "show mail",
   **Then** the message list shows mail from or to *any* of that person's
   addresses.
5. **Given** the user opened Contacts from a message in the inbox, **When**
   they press `Esc`, **Then** the inbox, the selected message and the reading
   position are as they left them.
6. **Given** the screen is open, **When** the user opens the cheat sheet,
   **Then** every contacts command appears with its binding.

---

### User Story 2 - Join several addresses into one person (Priority: P1)

The user notices that the same person appears three times — a work address, a
personal one, an old one — and joins them into one contact. From then on that
person appears once in the list, once in completion (offering each address
beneath the one name), and "show mail" finds all of their mail. The user can
also add an address to a person by typing it, and detach an address that was
joined by mistake.

**Why this priority**: This is the thing the request asked for by name, and
the thing the current model cannot express at all.

**Independent Test**: With mail from `ada@work.example` and
`ada@home.example`, join them; confirm the list shows one person, completion
on "ada" shows one name with both addresses, and "show mail" returns messages
from both. Undo the join and confirm two people are back, each with their own
history.

**Acceptance Scenarios**:

1. **Given** two people in the list, **When** the user marks both and invokes
   "join", **Then** one person remains holding both addresses, the sighting
   history of both addresses is preserved, and the list redraws at once.
2. **Given** a join of two or more people, **When** the join happens,
   **Then** the user picks the joined person's name — from the names the
   user set and the display names the addresses were seen with, the most
   recent preselected — or types one; no other field is silently discarded —
   notes are kept together, both organisations are offered.
3. **Given** a person was just joined, **When** the user presses `u`,
   **Then** the join is undone and both original people return exactly as
   they were.
4. **Given** a person with three addresses, **When** the user detaches one,
   **Then** that address becomes a person of its own carrying its own
   sighting history, and the original keeps the other two.
5. **Given** a person, **When** the user adds an address that already belongs
   to someone else, **Then** Postio says who it belongs to and offers to move
   it, rather than creating a second owner.
6. **Given** a person with several addresses, **When** the user marks one as
   preferred, **Then** completion and "compose to" offer that address first.
7. **Given** a joined person the user has named, **When** the user reads
   mail from either address in the message list or the reader, **Then** it
   shows under the person's name, and the reader's header details still show
   the address the mail actually came from.
8. **Given** a joined person, **When** new mail arrives from any of their
   addresses, **Then** it counts toward that person, and no new, separate
   person appears.

---

### User Story 3 - Create, edit and delete a person (Priority: P2)

The user adds someone who has never written — a name and an address typed in —
and they are immediately offered in completion. The user corrects a name the
mail got wrong ("ADA LOVELACE (via List)" becomes "Ada Lovelace"), adds an
organisation and a note. The user deletes a mailing-list robot and it does not
come back when the next newsletter arrives.

**Why this priority**: Closes the first acceptance criterion of #4 and is
most of what an address book is for, but it builds on the screen (Story 1)
and the person model (Story 2).

**Independent Test**: Create a person with a name and an address that appears
in no mail and confirm completion offers them; rename a mail-derived person
and confirm the new name survives the next message from them; delete a
mail-derived person, deliver another message from that address, and confirm
they are absent from the list, completion and the finder.

**Acceptance Scenarios**:

1. **Given** no mail from `grace@example.org`, **When** the user creates a
   person with that address, **Then** composer completion offers it at once.
2. **Given** a mail-derived person, **When** the user sets their name,
   **Then** later messages carrying a different display name do not
   overwrite it.
3. **Given** a mail-derived person, **When** the user deletes them and
   another message from that address arrives, **Then** they do not reappear
   anywhere contacts are offered.
4. **Given** a deleted mail-derived address, **When** the user creates a
   person with that address again, **Then** it is offered again and its
   earlier sighting history is kept.
5. **Given** a person was just deleted, **When** the user presses `u`,
   **Then** the person returns with all their addresses and details.
6. **Given** a person was deleted in an earlier session, **When** the user
   shows the Deleted filter and invokes "restore" on them, **Then** they
   return exactly as before, with the sighting history gathered while
   deleted, and are offered in completion again.
7. **Given** any edit, **When** it is made, **Then** it is visible
   immediately and needs no network.

---

### User Story 4 - Postio suggests who might be the same person (Priority: P3)

Joining by hand is fine for three duplicates and tedious for thirty. Postio
offers suggestions — people whose names match, or who reply from one address
to mail sent to another — and the user accepts or dismisses each. A dismissed
suggestion is not offered again.

**Why this priority**: Makes Story 2 practical on a real mailbox, but Story 2
is complete without it.

**Independent Test**: With fixture mail where `ada@work.example` and
`ada@home.example` both carry the display name "Ada Lovelace", confirm a
suggestion to join them is offered; dismiss it and confirm it does not
return; accept another and confirm it behaves exactly as a manual join,
including undo.

**Acceptance Scenarios**:

1. **Given** two unjoined people carrying the same display name, **When** the
   user views suggestions, **Then** the pair is offered with the evidence
   (the shared name, message counts for each address).
2. **Given** a suggestion, **When** the user dismisses it, **Then** it is
   never offered again, even after more mail arrives.
3. **Given** suggestions exist, **When** the user does nothing, **Then**
   nothing is joined.

---

### User Story 5 - Groups (Priority: P3)

The user makes a group ("Family"), adds people to it from the list, renames
it, and deletes it. Picking the group in the composer fills in its members'
preferred addresses; `group:family` in search finds their mail.

**Why this priority**: The store and the query language already support
groups; this story gives them a place to be managed (#477).

**Independent Test**: Create a group of two people, pick it in a new draft's
To field and confirm both preferred addresses are filled in; search
`group:<name>` and confirm mail from or to both — through any of their
addresses — is returned.

**Acceptance Scenarios**:

1. **Given** the Contacts screen, **When** the user creates, renames, adds
   members to, removes members from and deletes a group, **Then** each is
   reachable by keyboard and takes effect at once.
2. **Given** a group whose member has two addresses, **When** the group is
   picked in the composer, **Then** the member's preferred address is used.
3. **Given** a group, **When** the user searches `group:<name>`, **Then**
   mail from or to any address of any member is returned.

---

### User Story 6 - vCard import and export (Priority: P4)

The user imports a `.vcf` file exported from another address book, and later
exports their contacts back out. Nothing in the file that Postio does not
understand — photos, phone numbers, birthdays, vendor properties — is lost in
the round trip.

**Why this priority**: Real, but independent of everything above, and the
least asked-for here (#475).

**Independent Test**: Import a fixture vCard 3.0 file holding a person with
two email addresses, a phone number and a vendor property, plus a group card;
confirm one person with two addresses and one group appear; export and
confirm every property Postio does not model is present byte-for-byte.

**Acceptance Scenarios**:

1. **Given** a vCard with two `EMAIL` lines on one card, **When** it is
   imported, **Then** one person with two addresses appears.
2. **Given** an imported address that already belongs to a mail-derived
   person, **When** it is imported, **Then** the card and that person are
   one person afterwards, with the card's details and the address's history.
3. **Given** a group card, **When** it is imported, **Then** a group appears
   rather than a person.
4. **Given** the default list view and no selection, **When** the user
   exports, **Then** the file holds the people the list shows — made,
   imported and written-to — and no one known only from received mail.
5. **Given** an imported contact, **When** it is exported, **Then** every
   property Postio does not model is reproduced verbatim, and the file is
   vCard 4.0.

---

### Edge Cases

- **The user's own addresses.** Addresses belonging to the user's own
  identities are not listed as contacts; they are managed as identities.
- **Mailing lists and automated senders.** They arrive as contacts like any
  address but stay out of the default list unless the user has written to
  them; deletion (suppression) removes one from completion too, and it is
  one keystroke.
- **A forged display name.** The name replacement is keyed by address, never
  by display name: mail from an address that belongs to no one, whose header
  claims to be "Ada Lovelace", shows its header as-is and is not presented
  as the contact Ada.
- **Joining a person into a group member.** Group membership follows the
  person: joining two people keeps every group either belonged to.
- **Joining a suppressed address.** Adding a suppressed address to a person
  lifts the suppression — the user has said they want it.
- **Deleting a person made of mail-derived and user-made addresses.** The
  person is hidden, not destroyed: every mail-derived address is suppressed,
  and the person's details and addresses are kept so undo, or "restore" from
  the Deleted filter, returns all of it.
- **Same address seen through two accounts.** It is one address owned by at
  most one person; the per-account sightings are kept apart as evidence.
- **Address case and form.** Addresses that differ only in letter case are
  the same address.
- **An empty name.** A person with no name the user set shows the most
  recently seen display name, then the preferred address.
- **A very large store.** Tens of thousands of correspondents: the list opens
  and scrolls within the performance budgets, and is never loaded whole.
- **A saved search made from a person.** Because picking a person writes
  their addresses into the query, a search pinned from it names the
  addresses as they were; joining or detaching an address later does not
  change it. The user re-picks the person to refresh it.
- **Detaching the last address.** A person must keep at least one address;
  detaching the last one is refused with the reason (use delete instead).
- **A malformed vCard.** Import skips cards it cannot read, says how many it
  skipped and why, and imports the rest.

## Requirements *(mandatory)*

### Functional Requirements

**The screen**

- **FR-001**: The Contacts screen MUST be reachable from the sidebar and the
  command palette, MUST take over the reading pane, and MUST return to the
  exact prior place (view, selection, reading position) on `Esc`.
- **FR-002**: Every contacts action MUST be a command in the command registry
  with a default binding, appear in the cheat sheet, and be rebindable; none
  may be mouse-only.
- **FR-003**: The list MUST be populated from the mail already in the store
  with no import or setup step, and MUST include people the user created or
  imported.
- **FR-004**: The list MUST be filterable by typing, matching name,
  organisation and every address of a person.
- **FR-005**: By default the list MUST show people the user made or imported
  plus people the user has sent mail to; people known only from mail they
  *received* MUST be one toggle away, and MUST still be offered in composer
  completion and the `@` finder. The list MUST mark which people the user
  made or imported. *(Maintainer, 2026-09-23: "People I've written to".)*
- **FR-006**: A person's detail view MUST show their name, every address
  (marking the preferred one), organisation, note, groups, when they were
  last in touch and how many messages involve them.
- **FR-007**: "Show mail" MUST open the message list on mail from or to any of
  the person's addresses, and "compose to" MUST open a draft addressed to
  their preferred address. The search it opens MUST be an ordinary query in
  the one query language naming each of the person's addresses at that
  moment, visible and editable in the search bar; no person-level search
  field is added. (The language gains one address field meaning "from or to
  any of these", since it has no "or" — `research.md` R6.)

**People and addresses**

- **FR-010**: A contact MUST be able to hold one or more addresses; an address
  MUST belong to at most one contact.
- **FR-011**: Addresses differing only in letter case MUST be treated as one
  address.
- **FR-012**: The user MUST be able to join two or more people into one; the
  result MUST keep every address, every address's sighting history, every
  group membership, and every note.
- **FR-013**: Every join MUST end with a name for the joined person: the user
  picks one of the user-set names and display names the addresses were seen
  with (the most recently seen preselected, so confirming is one keystroke)
  or types one, and it is thereafter a user-set name (FR-021, FR-032). When
  joined people have conflicting organisations the user MUST choose which to
  keep; nothing the user entered may be discarded without their choice.
- **FR-014**: The user MUST be able to detach an address from a person, making
  it a person of its own with its own history; detaching a person's last
  address MUST be refused with the reason.
- **FR-015**: The user MUST be able to add an address to a person by typing
  it; if it already belongs to another person, Postio MUST name that person
  and offer to move it.
- **FR-016**: The user MUST be able to mark one address per person as
  preferred; a person with one address has it preferred implicitly.
- **FR-017**: Mail arriving from or to an address that belongs to a person
  MUST count toward that person and MUST NOT create a new person.
- **FR-018**: Postio MUST NOT join addresses into one person without the
  user's action.
- **FR-019**: Postio SHOULD suggest people who are likely the same (a shared
  display name; a reply sent from one address to mail addressed to another),
  showing the evidence; a dismissed suggestion MUST NOT be offered again.

**Create, edit, delete**

- **FR-020**: The user MUST be able to create a person with a name and at
  least one address, whether or not any mail involves that address.
- **FR-021**: The user MUST be able to edit a person's name, organisation and
  note; a user-set name MUST never be overwritten by a display name from
  later mail.
- **FR-022**: Editing a person known only from mail MUST promote it to a
  user-made person in place, keeping its history.
- **FR-023**: Deleting a person MUST remove them from the list, composer
  completion and the `@` finder; every mail-derived address among theirs MUST
  stay suppressed so later mail does not bring them back.
- **FR-023a**: The Contacts list MUST offer a "Deleted" filter showing deleted
  people, and a "restore" command that returns one exactly as it was before
  deletion — name, organisation, note, every address with its history, and
  group memberships — and lifts the suppression of its addresses. Deleted
  people appear nowhere else (FR-023).
- **FR-024**: Creating a person with, or adding to a person, a suppressed
  address MUST lift its suppression and keep its earlier history.
- **FR-025**: Join, detach, delete and edit MUST each be undoable with `u`,
  restoring the prior state exactly.
- **FR-026**: Every contacts change MUST take effect locally and visibly
  without waiting on the network.

**Where contacts reach the rest of Postio**

- **FR-030**: Composer completion MUST show a person once, under one name,
  offering each of their addresses with the preferred one first; ranking
  MUST keep people the user made or imported above people known only from
  mail.
- **FR-031**: The `@` finder MUST find a person by any of their addresses or
  their name, and picking one MUST search mail from or to all of their
  addresses, written out as in FR-007.
- **FR-032**: When the user has set a person's name, the message list, the
  conversation view and the reader MUST show that name for any of the
  person's addresses in place of the header's display name; the reader's
  header details MUST still show the name and address exactly as the mail
  carried them. A person with no user-set name shows what the mail said.
  *(Maintainer, 2026-09-23: "Yes, everywhere".)*

**Groups**

- **FR-040**: The user MUST be able to create, rename and delete a group and
  add or remove people from it, from the Contacts screen.
- **FR-041**: Picking a group in the composer MUST fill in each member's
  preferred address at that moment; later changes to the group MUST NOT
  change an existing draft.
- **FR-042**: `group:<name>` in search MUST match mail from or to any address
  of any member.

**vCard**

- **FR-050**: The user MUST be able to import a `.vcf` file (vCard 3.0 or
  4.0) and export as vCard 4.0 either a selection or, with nothing selected,
  exactly the people the list currently shows — its typed filter and the
  "everyone from mail" toggle included. Suppressed (deleted) people MUST
  never be exported.
- **FR-051**: Every property Postio does not model MUST be kept verbatim on
  import and reproduced verbatim on export — byte-identical from a 4.0 card;
  from a 3.0 card, changed only where 4.0 forbids the 3.0 spelling (version
  line, binary encoding, preference and charset parameters).
- **FR-052**: A card with several `EMAIL` properties MUST import as one
  person with several addresses; a group card MUST import as a group.
- **FR-053**: An imported address that already belongs to a person MUST be
  joined with that person, not duplicated.
- **FR-054**: Import MUST skip unreadable cards, report how many and why, and
  import the rest.

**Privacy**

- **FR-060**: Nothing in this feature may contact the network: no avatar,
  favicon or profile lookups, no directory queries.
- **FR-061**: Logs MUST carry counts, ids and outcomes only — never a name,
  address or note.

### Key Entities

- **Contact (person)**: Someone the user corresponds with. Has a name the user
  may set, an organisation, a note, a provenance (from mail, made by the user,
  imported), group memberships, and any vCard properties Postio does not
  model. Owns one or more addresses, one of which is preferred. Shared across
  accounts.
- **Address**: One email address, compared without regard to letter case.
  Belongs to at most one contact. Carries the evidence the mail provides —
  how many messages, when last seen, the display name last seen, per account
  — and whether the user has suppressed it.
- **Join suggestion**: A proposed pair of contacts that may be one person,
  with the evidence for it; accepted, dismissed, or pending. A dismissal is
  remembered.
- **Group**: A named set of contacts, chosen by the user. Shared across
  accounts.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: On a store with 20,000 distinct correspondents, the Contacts
  screen shows its first rows within the interaction budget, and scrolling
  and filtering keep every keystroke within 16 ms.
- **SC-002**: Filtering the list by name or address returns results within
  100 ms on the same store.
- **SC-003**: A person with several addresses appears exactly once in the
  list, once in completion and once in the `@` finder, and "show mail"
  returns 100% of the messages involving any of their addresses.
- **SC-004**: A user can join two duplicate people, including confirming the
  preselected name, in four keystrokes or fewer from the list, and undo it
  in one.
- **SC-005**: A deleted mail-derived person stays absent after any number of
  further messages from their addresses.
- **SC-006**: A vCard 4.0 file imported and exported again reproduces every
  property Postio does not model byte-for-byte; a 3.0 file does so except for
  the spellings FR-051 names.
- **SC-007**: No contacts action produces any network traffic.
- **SC-008**: Every contacts action is completable without a pointer.
- **SC-009**: Renaming a person changes how every message from any of their
  addresses reads in the list and the reader, with no message re-downloaded
  and the list staying within its scroll budget.

## Assumptions

- **Fields edited in v1** are name, addresses (with one preferred),
  organisation and note. Other vCard properties — phones, birthdays, postal
  addresses, photos — are kept and exported, shown read-only if present, but
  not editable here.
- **Contacts are shared across accounts** and there is no per-account address
  book; sightings stay per account as evidence (inherited from ADR 0007 Q5).
- **No photos or avatars are fetched**; a person is represented by initials.
- **The user's own identities** are not contacts and do not appear in the
  list.
- **CardDAV and any remote address book are out of scope**; nothing here
  prevents them later.
- **No backwards compatibility**: the store's contact tables may be reshaped;
  existing mail-derived contacts are rebuilt from the mail on resync.
- **Issues #477 and #475 are delivered by this branch** and are closed when
  it lands; the open epic #18 and #4 are left for the maintainer to close.
- **ADR 0007 is folded into this spec** and deleted in this branch, with its
  citations in code and docs re-pointed to `specs/005-contacts`.
