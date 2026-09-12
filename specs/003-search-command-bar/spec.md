# Feature Specification: Search and Command Bar

**Feature Branch**: `feature/search-command-bar`

**Created**: 2026-09-11

**Status**: Draft

**Input**: User description: "search/command bar: we need to be able to search any emails indexed quickly and we need to be able to use it for commands as well. Most of this is built but I want to have a spec and I want to add the ability to search for / use a command for folders, so I can go to inbox or drafts from the search bar"

## Why this spec exists

All of this is built, including the part that prompted the request.

The bar is one box in the header. Typing goes to mail search; a prefix in an
empty box is absorbed and becomes a mode, shown as a marker in the field:

| Typed | Mode | What it does |
|---|---|---|
| anything | Search | searches mail, operators become chips |
| `>` | Command | fuzzy-matches the command registry |
| `#` | Mailbox | **jumps to a folder** |
| `+` | Label | puts a label on the selection |
| `@` | Contact | finds a correspondent, then searches their mail |

`#` already does what was asked: it lists the folders the sidebar shows, from
the same list the sidebar is given, and activating one goes there. It is
wired, and a test drives it.

**So the gap is not the capability. It is that too little says the capability
exists.** The resting field says only *Search all mail*, and the prefixes are
absent from `keybindings.md` — they are not commands in the registry, and
that file is generated from the registry.

**Corrected during implementation.** An earlier draft of this paragraph said
the `?` cheat sheet did not list them either, reasoning that it too is
generated from the registry. That was wrong, and reasoned rather than
checked: `cheatsheet.rs::prefix_section` renders every mode under an "In the
search box" heading, and a test drives it from the mode table so a new mode
is covered without anybody editing it. That shipped under `postio-2ee`,
before this feature.

Which makes the evidence sharper, not weaker. The maintainer — who owns the
project — asked for a folder jump that had already shipped *and* was already
in the cheat sheet. One surface behind a key a stuck person presses was not
enough. That is the case for the bar saying so itself, and for the reference
a person reads before installing anything carrying it too.

That is what this feature adds, and the built behaviour is written down here
because it is the thing discoverability has to describe accurately. Sections
marked **Built** are being ratified, not rebuilt. Sections marked **New** are
the work.

The second new part is a **direct route** to the destinations people reach
most. The bar is a good general answer and a long way round for "go to my
inbox", which is why every mail client people arrive from binds that to two
keys. Postio already has `g`-prefixed sequences; this adds the family users
bring with them.

These two new parts are the same work at different ends. A destination
expressed as a command in the registry gets its binding, its palette entry,
its cheat-sheet line and its row in `keybindings.md` from the one table that
already generates all four — so making the destinations reachable by chord is
also what makes them discoverable, and neither has to be solved twice.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Find a message anywhere, quickly (Priority: P1) — Built

Someone remembers a message exists and wants it on screen. They open the bar,
type what they remember — words from the body, a sender, a phrase in the
subject — and the matching messages appear as they type, narrowing with each
keystroke. Operators let them be precise (`from:`, `subject:`, `is:unread`,
`in:`, `before:`) and the operators they typed are shown back to them as chips,
so the query language is learned by using it rather than by reading a manual.

**Why this priority**: It is the bar's reason to exist. A mail client whose
archive cannot be interrogated is a folder tree with extra steps, and this is
the only feature that makes a full local backfill worth having.

**Independent Test**: Seed a store, open the bar, type a word that appears in
some messages and not others, and confirm only the matching ones are listed and
that the count and elapsed time are shown.

**Acceptance Scenarios**:

1. **Given** a store containing messages that contain "quarterly" and messages that do not, **When** the user types `quarterly` in the bar, **Then** only the messages containing it are listed, with a count of how many matched.
2. **Given** a query `from:ada quarterly`, **When** it is typed, **Then** `from:ada` is shown as a chip and the results are restricted to that sender.
3. **Given** a result list, **When** the user presses the key that opens a message, **Then** that message opens in the reading pane without the bar having to be dismissed first.
4. **Given** a mailbox still syncing, **When** a search runs, **Then** the readout says the results are over what has arrived so far rather than presenting a partial answer as a complete one.

---

### User Story 2 - Run a command without knowing its key (Priority: P2) — Built

Someone knows what they want to do but not which key does it. They open the
bar, type a few letters of the command's name, and the commands that match are
listed with the key that would have done it — so using the bar teaches the
binding for next time. Only commands that make sense where they are standing
are offered.

**Why this priority**: It is what makes a keyboard-first client learnable
rather than a memory test, and it is the reason the command registry is a
single enumerable table.

**Independent Test**: Open the bar in a known context, type letters of a
command's title, confirm the command is listed with its binding, and run it.

**Acceptance Scenarios**:

1. **Given** the bar is open, **When** the user types letters that appear in a command's title in order, **Then** that command is listed with the matched letters marked and its current binding shown.
2. **Given** a command that cannot act where the user is standing, **When** the bar is open there, **Then** that command is not offered.
3. **Given** a command whose binding has been changed in configuration, **When** it is listed, **Then** the binding shown is the one in force, not a default.
4. **Given** an empty query, **When** the bar is opened, **Then** every command reachable from that context is listed.

---

### User Story 3 - Go to a folder by name (Priority: P3) — Built

Someone wants to be somewhere else: Inbox, Drafts, a project folder six levels
into a tree, a saved search. They open the bar, type `#` and enough of the
folder's name to identify it, and go. The alternatives — stepping through
folders one at a time, or leaving the keyboard for the sidebar — remain.

**Why this priority**: It is the last common navigation with no name-based
route from the keyboard, and the one the request named. It is already built
behind `#`; what it lacks is any way to find out that it is.

**Independent Test**: With an account whose folders include Drafts, open the
bar, type `drafts`, activate the folder row, and confirm the message list is
showing Drafts.

**Acceptance Scenarios**:

1. **Given** an account with a Drafts folder, **When** the user types `drafts` in the bar and activates the folder row, **Then** the message list shows Drafts and the sidebar reflects it as the current folder.
2. **Given** a folder nested several levels deep, **When** the user types part of its own name, **Then** it is offered and identified well enough to tell it from a same-named folder elsewhere.
3. **Given** more than one account with a folder of the same name, **When** the user types that name, **Then** each account's folder is offered as a separate destination, distinguishable by account.
4. **Given** a query that matches both a folder and one or more commands or messages, **When** the results are listed, **Then** all three kinds are offered together and each row says plainly what activating it will do.
5. **Given** a saved search in the sidebar, **When** the user types its name, **Then** it is offered as a destination on the same terms as a folder.
6. **Given** a folder whose name came from the server and contains characters with special meaning to the display layer, **When** it is listed, **Then** the name is shown literally as text.

---

### User Story 4 - Find out what the bar can do (Priority: P1) — New

Someone who did not build Postio opens the bar. Today it says *Search all
mail*, and that is all it says — so they search mail, and never learn that the
same box goes to folders, runs commands, labels a selection or finds a
correspondent. They reach for the sidebar with the pointer, or step through
folders one at a time, or ask for a feature that has already shipped.

With this, the box tells them: the modes are visible from the box itself
without typing a prefix first, they are listed where a user goes to learn what
Postio can do, and each one says what it is for in words rather than by symbol
alone.

**Why this priority**: P1, above the modes themselves, because a capability
nobody can find is worth what an absent one is worth — and four of the five
modes are in that position now. This is the only story here that changes what
anyone can actually do.

**Independent Test**: Give the bar to somebody who has not read the source,
ask them to go to Drafts without using the pointer, and see whether the box
tells them enough to get there.

**Acceptance Scenarios**:

1. **Given** the bar at rest, **When** a user looks at it without typing, **Then** it indicates that it does more than search mail.
2. **Given** the bar is open and empty, **When** the user has typed no prefix, **Then** the available modes and the character that reaches each are shown, with what each is for.
3. **Given** a user in a mode, **When** they look at the box, **Then** it says which mode they are in and how to get back out.
4. **Given** the documentation a user is pointed at to learn Postio's keyboard, **When** they read it, **Then** the bar's modes are described there alongside the bindings, including the prefix for each.
5. **Given** a new mode is added to the bar later, **When** the documentation is generated, **Then** the new mode appears without anybody editing a second list by hand.
6. **Given** a screen-reader user, **When** the bar is at rest and when it is in a mode, **Then** the same facts are available to them as text.

---

### User Story 5 - Go straight to the places you go most (Priority: P2) — New

Someone who has used mail on the web arrives with `g i` in their fingers. They
press it expecting the inbox. In Postio today, nothing happens. The same is
true of every other destination they reach dozens of times a day: the route
exists, but it costs a prefix and a name, or a trip to the sidebar.

With this, the common destinations answer to a two-key sequence — `g i` for the
inbox, and a consistent family for the rest of the roles the sidebar shows.

**Why this priority**: P2 — below being able to find the modes at all, above
ratifying what already ships. It is a small amount of work with an outsized
effect on how the client feels to somebody arriving from elsewhere, and it
carries the discoverability of the destinations along with it for free.

**Independent Test**: With mail in more than one folder, press `g i` from the
message list and confirm the inbox is showing; repeat for each bound
destination.

**Acceptance Scenarios**:

1. **Given** the user is anywhere they could use the message list, **When** they press `g i`, **Then** the inbox is shown and becomes the current folder everywhere the current folder is shown.
2. **Given** an account whose inbox is not named "Inbox" — a different provider, a different language — **When** the user presses `g i`, **Then** they still arrive at that account's inbox.
3. **Given** the destinations that have a sequence, **When** the user opens the bar in command mode or the cheat sheet, **Then** each appears there by name with the sequence beside it, without anybody maintaining a second list.
4. **Given** a user who rebinds one of these in configuration, **When** they use the new binding, **Then** it works and the old one no longer does, on the same terms as every other command.
5. **Given** a destination that does not exist for the current account, **When** its sequence is pressed, **Then** the user is told plainly rather than being left on an empty list wondering whether it worked.
6. **Given** the user is typing in the bar or composing, **When** they type `g` followed by a letter, **Then** the letters are entered as text and no navigation happens.

### Edge Cases

- **A folder named like a command.** A folder called "Archive" and the command "Archive" both match `archive`. Both are offered; the rows say which is which, and activating one never silently does the other.
- **No matches at all.** The bar says so rather than showing an empty area that could be mistaken for a slow search.
- **A query that matches everything.** The work done, and the time taken, must not grow with the size of the store.
- **Folders that arrive or vanish while the bar is open.** A sync that discovers or removes a folder mid-query must not leave a destination listed that no longer exists, nor crash on activating one that has just gone.
- **Very many folders.** An account with hundreds of folders must not make the bar slower to open or to type in.
- **A folder the user cannot currently reach**, such as one belonging to a disabled account: it is not offered as a destination.
- **Search over a store that is still filling.** Results are over what is indexed; the bar says so.
- **A message whose body has not been fetched yet.** It can still be found by the parts that are indexed, and its absence from body matching is not reported as "no results".
- **Activating a destination from inside a search.** Going to a folder from a result list leaves the search behind cleanly, with a way back.

- **A mode a user cannot use.** If labelling is unavailable where they stand, the bar must not advertise a mode that will do nothing.
- **The hint competing with the query.** Whatever tells the user about the modes must not obstruct typing, and must get out of the way once a query is under way.
- **A new mode added later.** Both the bar's own hint and the documentation must gain it without a second hand-maintained list, on pain of the drift a single registry exists to prevent.
## Requirements *(mandatory)*
- **A sequence whose first key already means something.** `g g`, `g f` and `g a` are taken. `g a` is the one that collides with the convention being copied — it is "next scope" here and "all mail" there — and an arriving user's muscle memory will find it.
- **A destination with no folder behind it.** An account with no Junk folder, or no Archive: the sequence must say so rather than appear to do nothing.
- **More than one account.** "The inbox" is ambiguous once a second account exists, and the sequence must land somewhere defensible rather than on whichever account sorted first.

### Functional Requirements

#### The bar itself

- **FR-001**: Users MUST be able to open the bar from the keyboard without first moving focus to it with a pointer.
- **FR-002**: The bar MUST accept a single line of text and offer results that narrow as the text is typed, without the user submitting first.
- **FR-003**: The bar MUST be operable entirely from the keyboard: opening, moving through results, activating one, and dismissing.
- **FR-004**: Dismissing the bar MUST leave the user where they were, with no change made.
- **FR-005**: Every row MUST state what activating it will do, such that a user can tell a message, a command and a destination apart without activating one to find out.
- **FR-006**: The bar MUST be usable with assistive technology: each row's kind, its label and its effect available as text.

#### Searching mail — Built

- **FR-007**: Users MUST be able to find a message by words that appear in its indexed content, including its body where the body has been indexed.
- **FR-008**: Users MUST be able to constrain a search by sender, recipient, subject, flag state, date range, attachment presence, attachment filename, size, mailing list, mailbox and account.
- **FR-009**: Recognized operators MUST be shown back to the user as labelled chips, and removing a chip MUST put the query back into the state it would have been in without that operator.
- **FR-010**: The bar MUST report how many messages matched and how long the search took.
- **FR-011**: A search MUST run against local, already-indexed content and MUST NOT require the network to produce results.
- **FR-012**: When the store is still being filled, the bar MUST say that results cover what has been indexed so far.
- **FR-013**: The work a search performs MUST NOT grow in proportion to the number of messages stored.

#### Running commands — Built

- **FR-014**: Every command in the command registry MUST be findable by name in the bar.
- **FR-015**: The bar MUST offer only commands that can act in the context the user is in.
- **FR-016**: A command row MUST show the key currently bound to it, reflecting user configuration rather than defaults.
- **FR-017**: Matching MUST tolerate gaps, so letters typed in order but not adjacently still find a command, and the matched letters MUST be marked in the row.
- **FR-018**: An empty query MUST list every command reachable from the current context.

#### Going to folders — New

- **FR-019**: Users MUST be able to reach any mailbox they can currently open by typing part of its name into the bar and activating the row.
- **FR-020**: Activating a folder row MUST show that folder's messages and MUST make it the current folder everywhere the current folder is shown.
- **FR-021**: Saved searches MUST be offered as destinations on the same terms as mailboxes.
- **FR-022**: A folder row MUST carry enough context to distinguish it from a same-named folder elsewhere, including which account it belongs to and where it sits if it is nested.
- **FR-023**: Where several accounts have a folder of the same name, each MUST be offered separately, and any unified destination that spans accounts MUST be distinguishable from a single account's folder.
- **FR-024**: Folder names MUST be displayed as literal text, never interpreted as markup or formatting by the display layer.
- **FR-025**: Mailboxes the user cannot currently open MUST NOT be offered as destinations.
- **FR-026**: Folder destinations MUST be derived from the same folder list the sidebar shows, so that a folder reachable in one is reachable in the other.
- **FR-027**: Adding folders as destinations MUST NOT change what an existing query means: a query that found messages or commands before MUST still find them.

#### Finding out the bar exists — New

- **FR-028**: The bar at rest MUST indicate that it does more than search mail.
- **FR-029**: When the bar is open with no prefix typed, it MUST show the available modes, the character that reaches each, and what each is for.
- **FR-030**: When a mode is active, the bar MUST show which mode it is in and how to leave it.
- **FR-031**: The modes and their prefixes MUST appear in the documentation a user is pointed at to learn Postio's keyboard, alongside the key bindings.
- **FR-032**: The mode list shown to the user and the mode list in the documentation MUST both derive from one enumeration, so a mode added later cannot appear in one and not the other.
- **FR-033**: A mode that cannot act where the user is standing MUST NOT be advertised as available there.
- **FR-034**: Everything the bar says about its modes MUST be available to assistive technology as text.
- **FR-035**: Nothing added for discoverability may obstruct typing a query or slow the bar's response to a keystroke.

#### Going straight there — New

- **FR-036**: Users MUST be able to reach the inbox with the two-key sequence `g i`.
- **FR-037**: The common destinations the sidebar shows MUST each be reachable by a two-key sequence beginning `g`, consistent with each other and, where it does not collide, with the convention users arrive with.
- **FR-038**: These sequences MUST target a mailbox's **role**, not its name, so that an inbox called something else, or named in another language, is still what `g i` reaches.
- **FR-039**: Each such destination MUST be a command in the registry, so that its binding, its palette entry, its cheat-sheet line and its documented row all derive from the one table rather than from four hand-kept lists.
- **FR-040**: These sequences MUST be rebindable on the same terms as every other command.
- **FR-041**: A sequence whose destination does not exist for the current account MUST say so rather than silently do nothing.
- **FR-042**: These sequences MUST NOT fire while the user is entering text, including in the bar, the composer, and any other text field.
- **FR-043**: Adding these MUST NOT change the meaning of any sequence already bound.

### Key Entities

- **Query**: the line of text the user typed, together with the operators recognized within it and their positions in that text.
- **Result row**: one offered outcome. Three kinds — a **message** to open, a **command** to run, a **destination** to go to — each carrying a label, the marks showing why it matched, and what activating it does.
- **Destination**: somewhere the message list can be pointed: a mailbox belonging to an account, a mailbox role spanning accounts, or a saved search.
- **Command**: an entry in the single command registry, with its title, the contexts it can act in, and the key bound to it.
- **Mailbox**: a folder in an account, with a name, a position in a tree, and possibly a role such as Inbox or Drafts.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A user can reach any folder they can name in under three seconds, without touching a pointer.
- **SC-002**: Searching a store of 50,000 messages returns its first results in under 100 milliseconds.
- **SC-003**: The work a search performs is unchanged when the store it searches grows by an order of magnitude.
- **SC-004**: 100% of the destinations shown in the sidebar — every mailbox and every saved search — are reachable by name from the bar.
- **SC-005**: 100% of the commands in the registry are findable by name from the bar in a context where they can act.
- **SC-006**: A user who has never read the source can reach all five of the bar's modes using only what the bar and the documentation tell them.
- **SC-009**: A user who has never read the documentation can go to Drafts from the keyboard on the first attempt, without being told the prefix beforehand.
- **SC-010**: Adding a mode to the bar requires editing exactly one list for it to appear in both the bar's hint and the generated documentation.
- **SC-011**: A user arriving from another mail client reaches their inbox with the sequence they already know, on the first try, without reading anything.
- **SC-012**: Every destination that has a sequence is reachable in exactly two keystrokes from anywhere the message list is usable.
- **SC-007**: Opening the bar and typing stays responsive with an account of 500 folders — no perceptible delay between keystroke and narrowed results.
- **SC-008**: No search or navigation performed from the bar causes any network request.

## Assumptions

- **The bar is one box, and already is.** Two surfaces — a `ctrl+k` palette and a `/` query bar — were converged into one, and this spec describes that box. Results are ranked within the active mode and never blended across modes, because a list mixing commands and messages is harder to scan than either.
- **Folder names are untrusted text.** They come from a mail server, so they are displayed literally and never interpreted.
- **Destinations follow the sidebar.** Whatever the sidebar decides a user can go to — including roles that span accounts and saved searches — is what the bar offers, rather than a second list that could disagree.
- **`g a` stays as it is.** It means "next scope" today, and the convention being copied would have it mean "all mail" — so the archive gets a different letter rather than an existing binding being taken away for a destination that is not even the same one. Named here because an arriving user's fingers will find `g a` anyway, and that is a thing to have decided rather than discovered.
- **Which letters, beyond `g i`, is a design call**, not a product one: the requirement is a consistent family covering the sidebar's roles, and the specific letters belong with whoever owns the keymap.
- **Multi-account destinations follow whatever the sidebar already does** for a role that spans accounts, rather than inventing a second answer here.
- **Ranking across kinds is a design decision, not a product one**, and is left to the design authority provided FR-005 holds: a user can always tell what a row will do.
- **Body search covers what has been indexed.** A message whose body has not yet been fetched is still findable by its indexed parts.
- **No new persisted state.** Going to a folder from the bar leaves the same trace that going to it from the sidebar does; this feature adds no history or recents store of its own.
- **Scope is Linux and the existing frontend for this increment**, with the product decisions stated here belonging to the shared layer so a second frontend inherits them rather than re-deriving them.

## Out of scope

- Searching the server rather than the local index.
- Any new query operator.
- Creating, renaming, moving or deleting folders from the bar.
- Changing what any mode does, or adding a sixth. This feature makes the five
  that exist findable; it does not redesign them.
- Changing the prefix characters. They are what ships and what any
  documentation written here will describe.
- Rebinding or removing any sequence that exists today, `g a` included.
- A sequence for every folder. The roles the sidebar shows get one; a folder
  six levels into a tree is what the bar is for.
- Acting on messages in bulk from the result list beyond what the existing commands already do.
- Remembering recent or frequent destinations to reorder results.
