# Feature Specification: The Outbox, and reserved mailboxes every account has

**Feature Branch**: `feature/outbox-and-reserved-mailboxes`

**Created**: 2026-09-11

**Status**: Draft

**Input**: User description: "folder/inbox/sidebar. We have folder, we have mailboxes, we have views (Flagged/Snoozed). We need an outbox to hold outgoing emails, we need to make sure the 'reserved' folder like inbox/drafts/sent/trash/junk are mapped to local folders for an account"

---

## What is here today

The sidebar draws four different kinds of thing, and only two of them are the
same kind underneath:

| Row | What it actually is |
|---|---|
| Inbox, Archive, Sent, Drafts, Trash, Junk | A real `Mailbox`, `id > 0`, a server path, a `MailboxRole` |
| An ordinary folder | The same `Mailbox`, role `Regular`, drawn as a tree |
| Flagged, Snoozed | A `Mailbox` **invented in the GTK feed** with a negative id and an empty path, summed from the real folders' counts |
| A saved search | Not a `Mailbox` at all — a `[filters]` entry in its own section, handing back a query string |

Three consequences follow, and this feature exists because of them.

**A message you have just sent is in Drafts.** `Composer::send` sets the draft
to `Queued` and enqueues the send; the draft row deliberately stays where it is
so the send can still be cancelled. The drainer later files a copy into Sent and
deletes the draft. Between those two moments — which is however long the network
takes, or forever while offline — a message the user considers *sent* is sitting
in the folder that means *unfinished*, in a row that cannot say otherwise,
because `MessageListRow` carries `draft: bool` and nothing finer. Someone
looking for it checks Sent, does not find it, and has no reason to look in
Drafts.

**Drafts holds four unrelated things.** `Editing` (still writing), `Queued` /
`Sending` (on its way), `Failed` (it stopped and needs a person), and
`Unconfirmed` (nobody can say whether it went). The list renders all four
identically.

**A role can have no mailbox.** Role resolution has tiers — an account's own
map, then `[mailboxes]`, then RFC 6154 `SPECIAL-USE`, then a name guess — and
every one of them can come up empty. When it does, `by_role` returns `None`,
the sidebar draws no row, and the commands that need somewhere to put a message
have nowhere to put it. Nothing guarantees an account has a Junk mailbox at all.

The Flagged/Snoozed mechanism is the precedent that makes the Outbox cheap, and
also the thing this feature must not simply copy: those rows exist **only inside
`postio-gtk`**, so the macOS frontend never receives them, and `MailboxRole`'s
`Snoozed` variant cannot even be stored — the `mailboxes` table's `CHECK`
constraint does not list it.

---

## User Scenarios & Testing *(mandatory)*

### User Story 1 - A message on its way is visible while it is on its way (Priority: P1)

I write a message and press Send. The composer closes. I want to see, without
hunting, that the message exists and is going out — and I want it to stop being
somewhere that implies I have not finished it.

**Why this priority**: This is the reported defect and the whole reason for the
feature. It is also the smallest independently shippable slice: an Outbox that
holds in-flight sends delivers the value on its own, before anything changes
about Drafts, reserved roles, or the shared sidebar vocabulary.

**Independent Test**: Queue a send with the drainer paused. The message is
listed in the Outbox and is not listed in Drafts. Release the drainer. The
message leaves the Outbox and appears in Sent, and the Outbox row disappears
from the sidebar because it is empty.

**Acceptance Scenarios**:

1. **Given** an account with no queued sends, **When** the sidebar is drawn,
   **Then** there is no Outbox row.
2. **Given** a draft I am editing, **When** I press Send, **Then** an Outbox
   row appears in the sidebar with a count of 1, and the message is listed in
   the Outbox and not in Drafts.
3. **Given** a message in the Outbox, **When** the send is accepted by the
   server, **Then** the message leaves the Outbox, appears in Sent, and the
   Outbox row disappears because it is now empty.
4. **Given** a message in the Outbox and no network, **When** I navigate away
   and back, **Then** the message is still in the Outbox with its state shown,
   and nothing waited on the network to tell me so.
5. **Given** a message in the Outbox whose send has not begun, **When** I
   cancel it, **Then** it returns to Drafts as an ordinary editable draft and
   the Outbox row disappears if it was the only one.
6. **Given** a message in the Outbox whose send is already in flight, **When**
   I try to cancel it, **Then** I am told it is too late and the message stays
   in the Outbox.
7. **Given** a message scheduled to send later, **When** I open the Outbox,
   **Then** it is listed there with its due time shown, not as an ordinary
   queued send.

---

### User Story 2 - Drafts means unfinished, and says when one needs me (Priority: P2)

I open Drafts expecting the things I have not finished writing. A send that
failed is not one of those, but it is also not on its way — it has stopped, and
it is waiting for me. I want Drafts to be honest about both.

**Why this priority**: Story 1 moves the in-flight rows out, which is most of
the improvement. What remains is that `Failed` and `Unconfirmed` are still
indistinguishable from `Editing`, and a failed send that nobody notices is the
worse of the two failures — but it is a smaller, separable change.

**Independent Test**: Put one draft in each of `Editing`, `Failed` and
`Unconfirmed`. Drafts lists all three, each row says which it is, and the
sidebar's Drafts row carries a count of 3 and an attention count of 2.

**Acceptance Scenarios**:

1. **Given** drafts in `Editing`, `Failed` and `Unconfirmed`, **When** I open
   Drafts, **Then** all three are listed and each row states its state.
2. **Given** one `Editing` and one `Failed` draft, **When** the sidebar is
   drawn, **Then** the Drafts row shows both a total and a separate attention
   count for the one that needs me.
3. **Given** a draft whose send failed, **When** I open it, **Then** I am told
   why it failed and can retry it.
4. **Given** a retried draft, **When** the retry is queued, **Then** the draft
   moves to the Outbox and the Drafts attention count falls by one.
5. **Given** no draft needs attention, **When** the sidebar is drawn, **Then**
   the Drafts row shows a plain count and no attention marker.

---

### User Story 3 - Every account has a mailbox for every reserved role (Priority: P3)

I add an account whose server has no Junk folder, or has one under a name
nothing recognises. I want Postio to have a Junk mailbox for that account
anyway, so that `!` works and so that the sidebar is the same shape for every
account I own.

**Why this priority**: It is the correctness floor under the other stories —
filing to Sent, archiving, and deleting all need a mailbox to exist — but the
common case already resolves through `SPECIAL-USE` or the name guess, so it
bites a minority of accounts. It is also the story that writes to the user's
server, which is the one that deserves the most care.

**Independent Test**: Point an account at a mock server that lists only `INBOX`.
After discovery, the account has exactly one selectable mailbox for each of
Inbox, Archive, Sent, Drafts, Trash and Junk, and each was created on the
server exactly once.

**Acceptance Scenarios**:

1. **Given** an account whose server lists only `INBOX`, **When** discovery
   completes, **Then** the account has exactly one mailbox for each reserved
   role and each unmapped one was created on the server.
2. **Given** an account whose server already has a folder for every reserved
   role, **When** discovery runs, **Then** nothing is created and no folder is
   renamed.
3. **Given** an account where the server refuses to create the missing folder,
   **When** discovery completes, **Then** the role is shown as unmapped rather
   than silently absent, the refusal is not retried on every pass, and I am
   told how to map it by hand.
4. **Given** a role Postio created a folder for, **When** I map that role to a
   different folder in settings, **Then** the map changes at once and the
   folder Postio created is left alone, not deleted.
5. **Given** two server folders that both look like Sent, **When** the sidebar
   is drawn, **Then** exactly one carries the role and the other is listed as
   an ordinary folder.
6. **Given** discovery has run once and created what was missing, **When** it
   runs again, **Then** it creates nothing.

---

### User Story 4 - The sidebar's rows are defined once, for every frontend (Priority: P2)

I use Postio on Linux; the same rows should exist on the macOS frontend. More
practically: the Outbox should be defined once, not invented separately by each
frontend that draws a sidebar.

**Why this priority**: The Outbox is a *fourth* kind of row, and today the only
way to add one is a second negative-id sentinel inside `postio-gtk` — which
would leave the macOS frontend without an Outbox and put the definition of
"what is in the Outbox" in a widget. Doing Story 1 without this is doing it
twice.

**Independent Test**: Ask the shared sidebar model for one account's rows. It
returns the reserved mailboxes, the Outbox when non-empty, the views, and the
ordinary folders, each labelled with which kind it is — and both frontends
render that same answer.

**Acceptance Scenarios**:

1. **Given** an account with a queued send, **When** either frontend asks for
   the sidebar's rows, **Then** both receive an Outbox row with the same count.
2. **Given** any sidebar row, **When** it is inspected, **Then** it states
   which kind it is — a mailbox, a view, or a saved search — rather than the
   kind being inferred from its id or its empty path.
3. **Given** a row that is a view, **When** something asks it for a destination
   to move a message into, **Then** it answers that it is not one.
4. **Given** the reserved mailboxes and the views, **When** the sidebar is
   drawn, **Then** their order is the shared order, identical on both
   frontends.

---

### Edge Cases

**The Outbox**

- What happens when the app closes with messages in the Outbox? They are still
  there at the next start, in the same state, and the drainer picks them up —
  the Outbox is a reading of durable rows, not an in-memory list.
- What happens when a send is in flight and the process dies? The draft is
  `Unconfirmed`. It leaves the Outbox and goes to Drafts, marked as needing a
  person, because nobody can say whether it went.
- What happens when a message is in the Outbox and its account is removed? The
  queued send goes with the account; the Outbox row goes with it.
- What does a thread show when one of its messages is in the Outbox? The
  conversation shows it as on its way, not as a sent message.
- Can a message be moved *into* the Outbox? No. The Outbox is a consequence of
  sending, not a destination.
- What happens to the Outbox under the unified scope? It follows Flagged and
  Snoozed: one Outbox per account.

**Reserved roles**

- What happens when the server refuses to create a folder — no permission, the
  name is taken by a non-selectable node, or the hierarchy forbids it? The role
  stays unmapped and says so; the refusal is remembered so the next pass does
  not try again.
- What happens when the created folder later disappears from the server? The
  role is unmapped again and the next discovery pass may create it again — this
  is the existing unroling behaviour, and the creation is not special-cased to
  happen only once for all time.
- What happens when Postio is offline while adding an account? Nothing is
  created; the roles that resolve locally work, and the rest are created on the
  first pass that reaches the server.
- What does Postio call a folder it creates? The role's own name, unless a
  provider preset says otherwise — never a constant chosen for one provider.
- What happens when two accounts on the same server share folders? Creation is
  per account, and a folder that already exists is never created twice.
- Is the Inbox ever created? No. RFC 3501 names it and every server has it; a
  server that does not is broken in a way this feature does not paper over.

**The vocabulary**

- What happens to a view when it is handed to something that needs a folder —
  a move, an append, a per-folder setting? It refuses, and that refusal is
  checkable rather than depending on every caller remembering.
- What happens to a saved search whose name collides with a reserved role?
  Nothing: they are different kinds of row and are drawn in different sections.

---

## Requirements *(mandatory)*

### Functional Requirements

**The Outbox — what it holds**

- **FR-001**: The system MUST provide an Outbox for each account, holding
  exactly those of that account's drafts whose send is under way — the states
  reached by queueing a send and by the drainer beginning one.
- **FR-002**: A draft that is being written MUST NOT appear in the Outbox.
- **FR-003**: A draft whose send has stopped and needs a person — it failed, or
  it cannot be confirmed — MUST NOT appear in the Outbox. It belongs in Drafts,
  per FR-020.
- **FR-004**: A draft MUST appear in exactly one of Drafts and the Outbox at
  any moment, never in both and never in neither.
- **FR-005**: Pressing Send MUST move the message from Drafts to the Outbox
  within the interaction budget, without awaiting the network.
- **FR-006**: When a send is accepted, the message MUST leave the Outbox and
  appear in Sent.
- **FR-007**: A message scheduled to send at a future time MUST be listed in
  the Outbox with its due time shown, distinguishable from one waiting only on
  the drainer.
- **FR-008**: Cancelling a send from the Outbox MUST return the message to
  Drafts as an ordinary editable draft.
- **FR-009**: When a send is already in flight, cancelling MUST be refused with
  an explanation, and the message MUST stay in the Outbox.
- **FR-010**: The Outbox MUST NOT be a destination: no command that moves,
  copies or files a message may target it, and asking it for a folder to write
  into MUST answer that it is not one.
- **FR-011**: The Outbox MUST survive a restart — it is a reading of durable
  rows, and a message queued before a restart is in it afterwards.

**The Outbox — how it appears**

- **FR-012**: The Outbox row MUST be hidden from the sidebar when the Outbox is
  empty, and MUST appear when it is not.
- **FR-013**: The Outbox row MUST show how many messages it holds.
- **FR-014**: The Outbox MUST have a fixed position in the sidebar's shared
  order, adjacent to Drafts and Sent, so that its appearing and disappearing
  does not reorder the rows around it.
- **FR-015**: Every row in the Outbox MUST state which state it is in, rather
  than being rendered identically to every other row.
- **FR-016**: The Outbox MUST be reachable by keyboard and MUST have a command
  registry entry, like every other navigable surface.
- **FR-017**: The Outbox MUST be announced to assistive technology with what
  its count means, as the other counted rows are.
- **FR-018**: Both frontends MUST receive the Outbox from the same shared
  definition; neither may invent it.
- **FR-019**: Logs about the Outbox MUST carry ids, counts and outcomes only —
  never recipients, subjects, or message content.

**Drafts**

- **FR-020**: Drafts MUST hold what is being written, what failed to send, and
  what cannot be confirmed — and nothing that is on its way.
- **FR-021**: A draft row MUST state which of those it is; the list MUST be
  able to distinguish them rather than carrying a single "is a draft" flag.
- **FR-022**: The sidebar's Drafts row MUST show a total and, separately, how
  many drafts need a person's attention.
- **FR-023**: When no draft needs attention, the attention marker MUST NOT be
  drawn.
- **FR-024**: Retrying a failed draft MUST move it to the Outbox and MUST lower
  the Drafts attention count.
- **FR-025**: Opening a failed draft MUST say why it failed.

**Reserved roles**

- **FR-026**: The reserved roles are Inbox, Archive, Sent, Drafts, Trash and
  Junk. Each enabled account MUST have exactly one selectable mailbox for each
  of them once discovery has reached the server.
- **FR-027**: When a reserved role resolves to no folder, the system MUST
  create one on the server and map the role to it.
- **FR-028**: The system MUST NOT create a folder for a role that already
  resolves, MUST NOT rename or move an existing folder, and MUST NOT create the
  same folder twice across discovery passes.
- **FR-029**: The Inbox MUST NEVER be created; a server that does not have one
  is reported, not repaired.
- **FR-030**: The name of a created folder MUST come from the role or from the
  provider preset table — never from a constant or a branch naming one
  provider.
- **FR-031**: When the server refuses to create the folder, the system MUST
  leave the role unmapped, MUST show it as unmapped rather than omitting the
  row, MUST tell the user how to map it by hand, and MUST NOT retry the refused
  creation on every subsequent pass.
- **FR-032**: Role creation MUST NOT block the UI: the sidebar, the list and
  every command remain usable while a creation is outstanding.
- **FR-033**: A role mapped by the user MUST take precedence over anything the
  system created, and changing the map MUST NOT delete the folder the system
  created.
- **FR-034**: Exactly one mailbox may carry a role at a time; where two server
  folders would both qualify, one carries it and the others are listed as
  ordinary folders.
- **FR-035**: The user MUST be able to see, for each reserved role, which
  server folder it is mapped to, and to change it.
- **FR-036**: The Outbox MUST NOT be mappable to a server folder: it is a
  client-side view over drafts, and offering it as a mapping target would imply
  a folder that does not exist.

**One vocabulary for sidebar rows**

- **FR-037**: The system MUST distinguish, as a first-class property rather
  than by inference from an identifier or an empty path, a row that is a real
  mailbox from a row that is a view over messages filed elsewhere.
- **FR-038**: The set of sidebar rows for an account — reserved mailboxes,
  Outbox when non-empty, views, ordinary folders, saved searches — MUST be
  produced by one shared definition that both frontends consume.
- **FR-039**: The shared definition MUST carry each row's kind, display name,
  count, attention count where it has one, and position, so that a frontend
  renders rather than decides.
- **FR-040**: A view MUST answer that it is not a destination when asked for a
  folder to write into, and that answer MUST be checkable rather than resting
  on each caller's memory.
- **FR-041**: Where a role exists only as a view and can never name a server
  folder, storing it as though it named one MUST be impossible.

### Key Entities *(include if data involved)*

- **Mailbox**: a place messages are filed. Belongs to one account; has a role,
  a path on the server, counts, and — new here — a **kind** saying whether it
  is a real folder or a view over messages filed elsewhere.
- **Mailbox role**: what a mailbox is *for*, independent of what the server
  calls it. Split by this feature into **reserved roles**, which every account
  must have a real folder for (Inbox, Archive, Sent, Drafts, Trash, Junk), and
  **view roles**, which never name a folder (Flagged, Snoozed, and now Outbox).
- **Outbox**: the view over one account's drafts whose send is under way. Has a
  count, no path, no folder, and no ability to receive a moved message.
- **Draft**: a message being written or being sent, carrying the state that
  decides whether it is in Drafts or in the Outbox and what its row says.
- **Account role map**: the per-account record of which server folder each role
  is mapped to, above `[mailboxes]` and above what the server advertises.
- **Sidebar row**: the shared, frontend-agnostic description of one line in the
  sidebar — its kind, name, counts and position.

---

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: From pressing Send, the message is listed somewhere the user can
  find it within one interaction budget (< 16 ms of local work), with no
  network round trip in between.
- **SC-002**: A message that has been sent is never listed among the messages
  the user has not finished writing — in 100% of sends, at every moment between
  Send and Sent.
- **SC-003**: After first sync, 100% of accounts show exactly one mailbox for
  each of the six reserved roles, or show that role as unmapped with a stated
  reason; no account silently lacks one.
- **SC-004**: A send that failed can be counted from the sidebar without
  opening any folder, and can be retried in at most two actions from there.
- **SC-005**: The sidebar shows no Outbox row whenever the Outbox is empty, and
  shows one within one interaction budget of a send being queued.
- **SC-006**: Both frontends, given the same account, list the same sidebar
  rows in the same order.
- **SC-007**: Repeated discovery passes over an account with all roles resolved
  issue zero folder-creation commands.
- **SC-008**: Reading the Outbox costs a bounded number of statements and rows
  that does not grow with the size of the mailbox.

---

## Assumptions

- **The Outbox is per account**, matching Flagged and Snoozed, which the
  sidebar already knows how to place and count. No unified Outbox is
  introduced; the unified scope shows each account's own.
- **`Queued` and `Sending` are the Outbox; `Editing`, `Failed` and
  `Unconfirmed` are Drafts.** This is the maintainer's call of 2026-09-11,
  chosen over an Outbox that also held the stopped ones: the Outbox is
  transient and usually empty, and anything needing a person is in Drafts,
  which is where a person goes to finish writing. `Sent` is neither — it is the
  moment the draft row goes away.
- **Creating the missing folder on the server is the maintainer's call of
  2026-09-11**, chosen over a local-only mailbox and over showing the role as
  unavailable. It is the only option that makes the role work end to end
  without the user doing anything, and it is what other clients do.
- **Constitutional note, recorded rather than argued.** Principle VI says
  nothing leaves this machine that the user did not ask for, and a folder
  creation is a write to the user's account that they did not individually
  request. This specification proceeds on the maintainer's decision and holds
  it within these bounds: creation happens only as a consequence of the user
  adding or enabling an account, only for a role that resolves to nothing, only
  once, never for the Inbox, and is visible afterwards in the account's
  mailbox settings where it can be changed. If the maintainer later wants it
  behind a confirmation, that is a change to FR-027 alone.
- **The Outbox is a view, not a stored folder.** It has no server path, is
  never synced, and cannot be a move target — the same shape as Flagged and
  Snoozed, which is what makes the sidebar work already.
- The set of reserved roles is the existing six. Nothing here adds a role a
  server could carry, and Flagged/Snoozed/Outbox stay client-side.
- The existing send machinery is the starting point, not a rewrite. ADR 0021's
  state machine, the operation queue, the drainer's ordering, and the stable
  `Message-ID` reserved at queue time all hold unchanged. What changes is where
  those states are *shown*.
- No backwards compatibility is owed. A new column takes a plain default and
  the store may be rebuilt or resynced; the migration is still written and the
  meaning of each column is still explained.

---

## Dependencies

- **`feature/mailbox-roles` is built and unmerged, and this feature needs it.**
  Epic #962 and its five children (#963–#967, all closed) landed the
  per-account role map — the `mailbox_roles` table, discovery reading it every
  pass, the `MapMailboxRole` command, the settings rows, the docs — onto
  `origin/feature/mailbox-roles`, which is 15 commits ahead of `main` with no
  pull request and no commit since 2026-09-04. FR-033 and FR-035 are that
  branch's work, not this one's. **That branch must merge before this feature
  is planned**, or this feature will re-derive it and conflict with it.
- **That branch carries an ADR number that `main` has since taken.** It adds
  `docs/decisions/0027-mailbox-roles-are-mapped-per-account.md`; `main` now has
  `0027-the-header-index-is-budgeted-per-message.md`. Renumbering is part of
  merging it, not part of this feature.
- **ADR 0021 (exactly-once send)** is inherited unchanged, with one correction
  owed: its "What the user sees" table says every send state is reachable from
  the Drafts list, which FR-001 and FR-020 make false. The table is updated in
  this branch; the decision it records is not reopened.
- **ADR 0005 (multiple accounts)** settles that an account's mappings are
  state, not preference. Inherited.
- **#1491** is the reported defect this feature answers. It stays open until
  the branch lands, and it is not an issue to be claimed alongside this work —
  the spec is the queue.
- **Per `CLAUDE.md`, this feature gets no ADR of its own**: the decisions above
  and the alternatives rejected are recorded here. A new ADR is warranted only
  if the kind-on-a-mailbox rule (FR-037, FR-041) turns out to be a contract
  other work must obey, in which case it is that rule alone and not a restating
  of this spec.

---

## Out of Scope

- Changing how sending works: retry policy, backoff, the operation queue's
  shape, exactly-once delivery. This feature moves where states are shown.
- A unified Outbox across accounts.
- Deleting a folder Postio created, or cleaning up after a role is remapped.
- Making saved searches and views the same kind of thing; they stay two
  mechanisms, in two sections.
- Provider presets carrying folder names for silent servers (#959's durable
  answer). FR-030 leaves the door open by taking the name from the preset table
  when it has one; filling that table is not this feature.
- Creating folders for non-reserved roles, or letting the user create folders
  from the sidebar.
