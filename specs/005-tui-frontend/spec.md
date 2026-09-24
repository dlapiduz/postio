# Feature Specification: Postio in the terminal

**Feature Branch**: `feature/tui-frontend`

**Created**: 2026-09-23

**Status**: Draft

**Input**: User description: "I want to create a TUI version of postio, can you
use a framework or something to create a TUI version that has feature parity
with the GTK version? it should have mouse support and markdown reading and
writing of emails. It should be blazing fast and smaller than the GTK version"
— and, added while this was being written: "I'd love for the same store to be
available to the GTK version and TUI".

## Why this exists

Postio promises speed, search and the keyboard (`docs/PRODUCT.md` §1). Those
are qualities of the engine, not of the GTK window around it. The terminal is
where many people who have too much email already work, and it is the one
place the desktop frontend cannot reach: an SSH session, a tiling-WM
scratchpad, a machine with no display server.

The architecture has kept this possible on purpose. `postio-session` is a
composition root that "a frontend that is not GTK can link"
(`ARCHITECTURE.md`, *The shape*), `postio-ui` holds the toolkit-free
presentation logic, and the command registry is the one table every command
surface is derived from (Constitution II). A terminal frontend is the second
Linux frontend those boundaries were paid for — a port, not a second mail
client.

It is also a test of that claim. Every place the terminal frontend has to
reach into GTK-side code to get behaviour is logic that belonged in a shared
crate, and moving it there is in scope.

## Clarifications

### Session 2026-09-23

- Q: Does the terminal frontend land on `main` once, with every user story
  done, or in slices? → A: Once. All seven user stories are complete before
  the branch lands; nothing reaches `main` earlier.
- Q: How does text editing behave in the terminal composer? → A: Standard
  text-box editing only (modeless: type to insert, arrows, Home/End, the
  usual Ctrl chords); anyone who wants vim hands the body to `$EDITOR`.
- Q: How is the terminal frontend installed and launched? → A: As its own
  `postio-tui` command, shipped two ways: a standalone release download that
  needs no GTK, and a flatpak of its own, separate from the desktop app's.
- Q: Where do the terminal frontend's colours come from? → A: The terminal's
  own palette for everything, plus Postio's accent for selection and focus on
  true-colour terminals; overridable in `config.toml`.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Triage the inbox from a terminal (Priority: P1)

Someone with an account already synced opens Postio in a terminal. Within half
a second they see the sidebar, the message list for their inbox and a reading
pane. They move through the list with `j`/`k`, read a conversation, archive it
with `a`, undo with `u`, flag, mark read or unread, delete, move to a folder,
and switch folders or accounts — every key the same as the desktop app,
because both read the same bindings.

**Why this priority**: This is the whole of the daily loop, and the smallest
thing that is worth opening. Without it nothing else in the frontend matters.

**Independent Test**: Start the terminal frontend against a store holding a
fixture mailbox with the network absent; drive it with keystrokes; assert on
the rendered screen (not on what a layer was handed) that the list, the
reader and the result of each action are what a person would see, and that
the store holds the same outcome the desktop app would have written.

**Acceptance Scenarios**:

1. **Given** a synced store and no network, **When** the terminal frontend
   starts, **Then** the sidebar, the inbox list and the reader are drawn and
   navigable without waiting on any server.
2. **Given** a conversation under the cursor, **When** the user presses the
   archive key, **Then** it leaves the list immediately, a notice offers undo,
   and the remote effect is queued rather than awaited.
3. **Given** three rows selected and the cursor on a fourth, **When** the user
   archives, **Then** the three selected rows are archived and the fourth is
   not — cursor and selection are distinct, as in the desktop app.
4. **Given** a burst of twelve archives, **When** the user presses undo once,
   **Then** all twelve return.
5. **Given** a key rebound under `[keys]` in `config.toml`, **When** the
   terminal frontend starts, **Then** the rebinding applies there exactly as it
   does in the desktop app.
6. **Given** a mailbox of 100,000 messages, **When** the user scrolls from top
   to bottom, **Then** only the visible window of rows is ever read, and each
   step redraws within the interaction budget.

---

### User Story 2 - Read mail as Markdown (Priority: P1)

The reader shows each message as Markdown-styled terminal text: headings,
emphasis, lists, links, block quotes and code are drawn with the terminal's
own styling, quoted history folds away and expands on request, and HTML mail
arrives through the same sanitiser the desktop reader uses. A conversation is
one scrolling document, walked with `J`/`K`.

**Why this priority**: Reading is most of what an email client is used for,
and a terminal cannot show HTML. Markdown is the form that keeps the
structure of a message while being native to a character grid.

**Independent Test**: Render every message in the `.eml` corpus to the
terminal reader and assert on the resulting text grid: no raw HTML tags, no
script content, no remote-resource URL fetched or rendered as an inline
image, structure (lists, quotes, links) preserved, and quoted history folded.

**Acceptance Scenarios**:

1. **Given** an HTML message with headings, lists, links and a quoted reply,
   **When** it is opened, **Then** it is shown as styled Markdown with the
   quote folded and a visible affordance to expand it.
2. **Given** a message whose HTML contains script, tracking pixels and remote
   images, **When** it is opened, **Then** none of them causes a network
   request, and remote images appear as labelled placeholders that obey the
   per-sender allow rule the desktop app uses.
3. **Given** a plain-text message, **When** it is opened, **Then** it is shown
   as written, with no Markdown interpretation that would mangle it (a line
   starting `#` is not turned into a heading).
4. **Given** a link in a message, **When** the user activates it, **Then** it
   opens in the user's browser only on that deliberate act, and the full
   target is visible before activation.
5. **Given** a message with attachments, **When** it is opened, **Then** each
   attachment is listed with name and size and can be saved or opened with
   the system handler, fetching the payload only then.

---

### User Story 3 - Write mail in Markdown (Priority: P1)

The user presses `e` to reply (or the compose, reply-all and forward keys),
and the composer takes over the reading pane, as it does on the desktop. They
write Markdown. Recipients autocomplete from contacts; Cc and Bcc appear on
demand; the identity is pickable; the draft autosaves. Files arrive the way
they do in any modern terminal app: dragged from the file manager onto the
terminal, pasted as paths, or picked by typing a path; an image copied to the
clipboard (a screenshot, say) is pasted straight in. They can hand the body to their own `$EDITOR` and come back. `Ctrl+Enter`
sends, and the message goes to the Outbox at once, online or not.

**Why this priority**: A mail client that cannot reply is a reader. It was
the one thing the macOS slice deferred, and it is what the request asks for
by name.

**Independent Test**: Compose a Markdown message through keystrokes against
the mock backend, send it, and assert on the bytes handed to the outgoing
queue: a multipart message whose plain part is the Markdown the user wrote
and whose HTML part is generated from the same document the desktop composer
uses, with no remote reference in it.

**Acceptance Scenarios**:

1. **Given** a message open, **When** the user presses reply, **Then** the
   composer opens in the reading pane with recipients and subject filled and
   the original quoted, and `Esc` returns to exactly where they were.
2. **Given** a body using bold, italic, lists, links, quotes and code, **When**
   it is sent, **Then** recipients see that formatting in an HTML client and
   readable Markdown in a plain-text one.
3. **Given** a body that uses no Markdown formatting, **When** it is sent,
   **Then** it goes as plain text only, as the desktop composer does for a
   message that used none of its formatting.
4. **Given** the same content produced in the desktop composer and in the
   terminal composer, **When** both are sent, **Then** the HTML parts are
   identical — both are generated from Postio's own document model.
5. **Given** a draft in progress, **When** the terminal is closed without
   sending, **Then** the draft is in Drafts and reopens in either frontend.
6. **Given** the user invokes "edit in external editor", **When** the editor
   exits, **Then** the composer holds the edited body and nothing else about
   the draft has changed.
7. **Given** no network, **When** the user sends, **Then** the message is in
   the Outbox immediately and leaves at most once when the link returns.
8. **Given** the composer open, **When** the user drags one or more files from
   the file manager onto the terminal, **Then** each is attached, listed with
   name and size, and the body text is unchanged.
9. **Given** an image on the clipboard, **When** the user pastes into the
   composer body, **Then** the image is inserted inline at the cursor, shown
   as a labelled placeholder, and sent as an inline image.
10. **Given** one or more file paths on the clipboard, **When** the user
    pastes, **Then** each existing file is attached; text that is not a path to
    an existing file is pasted as text.
11. **Given** a path that cannot be read, **When** it is dropped or pasted,
    **Then** the user is told which file and why, and nothing else about the
    draft changes.

---

### User Story 4 - Search, palette and jump (Priority: P2)

`/` opens search and results appear while typing, in the same query language
as everywhere else; saved searches sit in the sidebar. `Ctrl+K` opens the
command palette listing every command in the registry; `?` shows the cheat
sheet. Both are generated from the registry, so nothing the desktop app can do
is missing from them.

**Why this priority**: Search is one of the three things Postio must be best
at, and the palette is how every command stays discoverable. It follows P1
because triage and reading are usable without it for a first session.

**Independent Test**: Type queries from the search corpus and assert on the
list drawn; enumerate the palette and assert it lists every command the
registry holds, with the same key the desktop app shows.

**Acceptance Scenarios**:

1. **Given** a synced store, **When** the user types `from:ada is:unread`
   character by character, **Then** results update on every keystroke, and
   half-typed states like `is:` never show an error.
2. **Given** a command added to the registry, **When** the terminal frontend
   is built, **Then** it appears in its palette and cheat sheet with no change
   to terminal-frontend code.
3. **Given** a saved search in `config.toml`, **When** the frontend starts,
   **Then** it appears in the sidebar and returns the same results as in the
   desktop app.

---

### User Story 5 - Everything reachable by mouse too (Priority: P2)

Every surface responds to the mouse: click a sidebar row, a list row or a
button to activate it; scroll the list and the reader with the wheel;
shift-click and ctrl-click to extend or toggle the selection; click a link to
see and open it; drag a pane divider to resize; click to place the cursor in
the composer.

**Why this priority**: Requested by name, and the constitution says the mouse
must remain excellent. It is P2 because the keyboard alone already makes every
command reachable.

**Independent Test**: Feed synthetic mouse events at known cells and assert
on the resulting screen and store state.

**Acceptance Scenarios**:

1. **Given** the list, **When** the user clicks a row, **Then** the cursor
   moves there and the reader shows it; ctrl-click toggles it in the
   selection without moving what the reader shows beyond the clicked row.
2. **Given** the reader, **When** the user scrolls the wheel, **Then** the
   reader scrolls and the list does not.
3. **Given** a terminal that reports no mouse events, **When** the frontend
   runs, **Then** nothing about it is degraded except that clicks do nothing.

---

### User Story 6 - One store, both frontends (Priority: P2)

The user runs the desktop app on their workstation and the terminal frontend
in a terminal on the same machine, against the same store. An archive in one
disappears from the other's list; mail synced by one appears in the other; a
draft started in one opens in the other. They do not configure anything to
make this true.

**Why this priority**: Asked for directly. Without it the terminal frontend
would be a second mail client with a second copy of the mailbox, which is the
thing `ARCHITECTURE.md` (and ADR 0010) calls a second application sharing a
file, not a second frontend.

**Independent Test**: Open two sessions on one store in two processes, act in
one, and assert on what the other presents, within the stated delay, with no
corruption, no duplicate outgoing message, and no duplicate remote effect.

**Acceptance Scenarios**:

1. **Given** both frontends open on one store, **When** a message is archived
   in one, **Then** the other's list shows it gone within one second.
2. **Given** both frontends open, **When** new mail arrives, **Then** it is
   fetched once, not once per frontend, and appears in both.
3. **Given** both frontends open, **When** a message is sent from either,
   **Then** it leaves exactly once.
4. **Given** the desktop app is not running, **When** the terminal frontend
   starts, **Then** it syncs on its own; and the reverse holds.

---

### User Story 7 - Set up and configure without the desktop app (Priority: P3)

Someone on a machine with only a terminal adds an account — preset
discovery, password or app password, or OAuth — and changes the settings the
desktop settings window offers, all from the terminal frontend. For OAuth the
consent URL is shown to be opened in a browser (on this machine or another),
and the loopback redirect completes the sign-in.

**Why this priority**: Parity requires it, and a terminal-only machine cannot
otherwise start. It is P3 because a user with the desktop app, or an existing
store, does not need it.

**Independent Test**: Run the add-account flow against the mock backend and a
fake keyring; assert the account row, the keyring entry and the resulting
first sync.

**Acceptance Scenarios**:

1. **Given** no accounts, **When** the terminal frontend starts, **Then** it
   offers to add one rather than showing an empty shell.
2. **Given** an OAuth provider, **When** the user starts sign-in, **Then** the
   consent URL is displayed in full and copyable, and nothing is opened or
   fetched until the user acts.
3. **Given** a setting changed in the terminal frontend, **When** the desktop
   app is running, **Then** it picks the change up the way it picks up any
   `config.toml` change.

---

### Edge Cases

- **Terminal too small**: below a minimum size the frontend collapses panes
  the way the desktop layout does by width (ADR 0024) — width decides what is
  shown, never what the user asked for — and below an absolute minimum it
  says so rather than drawing garbage.
- **Resize mid-action**: resizing while composing or with a selection loses
  neither.
- **Colour**: works on 16-colour, 256-colour and true-colour terminals, and
  with `NO_COLOR` set (no colour at all, accent included); colour is never
  the only carrier of meaning (unread, flagged and selected each have a
  non-colour mark). Below true colour the accent falls back to the terminal
  palette's nearest role rather than an approximated shade.
- **Unicode**: wide (CJK) characters, emoji and right-to-left text in subjects
  and bodies do not misalign columns or corrupt the grid.
- **Hostile content**: terminal escape sequences inside a subject, sender name,
  filename or body are neutralised and never reach the terminal — mail is
  attacker-controlled, and an escape sequence is this medium's `<script>`.
- **Huge messages**: a multi-megabyte body opens without stalling input; the
  reader renders what is on screen first.
- **Images**: inline images appear as labelled placeholders that can be opened
  with the system viewer; remote images stay blocked per sender.
- **Paste with no clipboard tool**: over SSH, or where no clipboard is
  reachable, pasting an image says the clipboard is unavailable; pasted text
  and dropped paths still work, because the terminal delivers those itself.
- **A paste that looks like a path but is prose**: only text that names an
  existing, readable file becomes an attachment; anything else is text.
- **Huge or many files dropped at once**: each is attached without stalling
  input, and the attachment size warning the desktop composer gives applies.
- **Store locked or keyring unavailable**: the frontend says which, in words,
  and exits cleanly rather than opening empty.
- **Other frontend quits mid-sync**: the surviving frontend takes over
  background sync without losing queued operations.
- **SSH with no browser**: every "open" action (link, attachment, OAuth
  consent) degrades to showing the target so the user can act elsewhere.
- **Suspend and resume** (`Ctrl+Z`, then `fg`), and handing the terminal to
  `$EDITOR`, restore the screen exactly.

## Requirements *(mandatory)*

### Functional Requirements

**Parity**

- **FR-001**: The terminal frontend MUST offer every command in the command
  registry, with the same command ids, the same default keys, and the same
  `[keys]` overrides as the desktop app. A command reachable in the desktop app
  and not in the terminal frontend is a defect, and a test MUST enumerate the
  registry to prove there is none.
- **FR-002**: The terminal frontend MUST cover the v1 surface of
  `docs/PRODUCT.md` §23: multiple accounts and the unified inbox; folders,
  views (Flagged, Snoozed, Outbox) and saved searches in the sidebar;
  threads; read/unread, archive, delete, flag, move, label, snooze; reading
  with attachments, quote folding, per-sender remote-image permission and
  one-click unsubscribe on request; compose, reply, reply-all, forward,
  attachments, drafts, signatures, identities, scheduled send; recipient
  autocomplete from contacts; search with operators; the command palette and
  cheat sheet; background sync; offline use; undo; notifications.
- **FR-003**: Where a desktop capability has no terminal equivalent (a
  pop-out composer window, rendered images), the terminal frontend MUST
  provide the nearest equivalent listed in this spec (a composer tab/split,
  placeholders with open-in-viewer) rather than omit the command.
- **FR-004**: Behaviour that both frontends need MUST be expressed once, in
  the shared toolkit-free layers, and consumed by both. Logic currently held
  only in the desktop view layer that the terminal frontend needs MUST move
  to a shared layer, not be copied.
- **FR-005**: No functionality may be removed from, or degraded in, the GTK
  desktop app or the macOS frontend. Moving logic into a shared layer MUST
  leave both behaving exactly as before, and their existing test suites MUST
  pass unchanged except for import paths — a test that has to be weakened to
  pass is evidence of a regression, not of a refactor.
- **FR-006**: The terminal frontend MUST be built on established, maintained
  terminal-application building blocks — for the screen and input, Markdown
  rendering and editing, text input, clipboard, and terminal capability
  detection — and on patterns proven in widely used terminal applications,
  rather than on components written from scratch. Writing one of these
  ourselves requires a stated reason why no existing one fits, recorded in
  the plan. This is the same rule "Pimalaya first" sets for protocols.

**Reading**

- **FR-010**: The reader MUST present each message as Markdown-styled
  terminal text, preserving headings, emphasis, lists, links, block quotes,
  code and tables at least as faithfully as a plain-text rendering would.
- **FR-011**: HTML mail MUST pass through the same sanitiser the desktop
  reader uses before conversion; nothing the desktop reader refuses may appear
  in the terminal reader.
- **FR-012**: Quoted history MUST fold by default and expand on request, as
  in the desktop reader.
- **FR-013**: All text originating in a message — headers, bodies, filenames,
  display names — MUST have terminal control sequences neutralised before it
  is drawn.
- **FR-014**: A link MUST show its full destination before it is followed,
  and MUST be followed only on deliberate activation.

**Writing**

- **FR-020**: The composer MUST accept Markdown and MUST translate it into
  Postio's own composer document, so that the outgoing bytes are produced by
  the same generator the desktop composer uses.
- **FR-021**: A message that uses formatting MUST be sent as multipart with a
  plain-text part (the Markdown as written) and an HTML part generated from
  the document; a message that uses none MUST be sent as plain text only.
- **FR-022**: The composer MUST offer a live preview of the rendered message,
  and MUST let the user edit the body in `$EDITOR` and return.
- **FR-022a**: Editing in the composer MUST be modeless, standard text-box
  editing: typing inserts; arrows, Home/End, PageUp/PageDown and the common
  readline-style Ctrl chords move and delete; selection, cut, copy, paste and
  undo work as in any text field. There is no modal (vim-style) editing inside
  the composer; `$EDITOR` handoff (FR-022) is how a user gets their own
  editor. While the composer has focus, printable keys insert text and never
  trigger list commands.
- **FR-023**: Drafts MUST autosave and MUST be readable and editable by
  either frontend. A draft written in the desktop composer MUST open in the
  terminal composer as Markdown, and vice versa, without losing formatting the
  document model can represent.
- **FR-025**: Files dragged onto the terminal while the composer is open MUST
  be attached, as they are when dropped on the desktop composer.
- **FR-026**: Pasting into the composer MUST attach each pasted path that
  names an existing file, MUST insert a clipboard image inline at the cursor,
  and MUST otherwise insert the text. The clipboard MUST be read only on the
  user's paste, never speculatively.
- **FR-027**: Attaching by typing or picking a path MUST remain available for
  terminals and sessions where neither dropping nor pasting works.
- **FR-024**: Every mutating action, send included, MUST be local-first:
  store write, enqueue, emit, repaint. The terminal frontend MUST NOT await
  the network for any visible outcome.

**Input**

- **FR-030**: Every surface MUST respond to mouse clicks, wheel scrolling and
  modifier-clicks for selection, where the terminal reports them; the
  frontend MUST remain fully operable without a mouse.
- **FR-031**: The list MUST keep a cursor and a selection that are distinct,
  with selection-as-predicate ("select all" is not a set of ids).

**Shared store**

- **FR-040**: The terminal frontend MUST open the same store, the same
  `config.toml` and the same keyring entries as the desktop app, with no
  import, export or second copy.
- **FR-041**: Both frontends MUST be able to run at the same time on one
  store. A change made through either MUST become visible in the other within
  one second, without the user refreshing.
- **FR-042**: While both run, each remote effect (fetch, flag change, move,
  send) MUST be performed once, not once per frontend; exactly-once send
  (ADR 0021) MUST hold across processes.
- **FR-043**: When either frontend exits, the other MUST continue to sync and
  drain the operation queue on its own.

**Performance and size**

- **FR-050**: The terminal frontend MUST meet the constitution's budgets:
  startup to usable screen < 500 ms with a populated store, interaction
  < 16 ms, local search < 100 ms, and MUST never load a mailbox into memory.
  Read paths MUST carry statement and row count assertions, as the desktop
  paths do.
- **FR-051**: The terminal frontend MUST NOT depend on GTK, libadwaita,
  WebKit or any display server, and this MUST be enforced by the crate
  boundary checks, not by convention.
- **FR-052**: The installed terminal frontend MUST be smaller on disk than
  the installed desktop app, and MUST use less memory than the desktop app
  showing the same mailbox.
- **FR-053**: The terminal frontend MUST be launched as its own command,
  `postio-tui`, and MUST be released in two forms: a standalone download that
  runs with no GTK, WebKit or display server installed, and a flatpak of its
  own, separate from the desktop app's and built on a runtime that carries no
  GTK. Both MUST be produced by the release workflow, from the same commit
  and gated by the same suite as the desktop app's release.

**Appearance**

- **FR-054**: The terminal frontend MUST draw with the terminal's own palette
  (foreground, background and the ANSI colours), so that it follows the
  user's terminal theme, light or dark. On a true-colour terminal it MUST
  additionally use Postio's accent, taken from the generated design tokens
  and never retyped, for the selected row and the focus indicator only.
  Every colour role MUST be overridable from `config.toml`.

**Privacy**

- **FR-060**: The terminal frontend MUST make no network request the user did
  not ask for: no remote images without per-sender permission, no read
  receipts, no link prefetch, and no unsubscribe without deliberate
  activation. Its logs MUST carry no message content.

### Key Entities

- **Terminal session**: one running terminal frontend — its screen size,
  colour capability, mouse support, and the panes it is showing. Holds no mail
  of its own; everything it shows is read from the shared store.
- **Rendered message**: a message as terminal text — the Markdown-styled,
  sanitised, escape-neutralised form of what the desktop reader shows, with
  fold state for quoted history and placeholders for images and attachments.
- **Markdown draft**: the composer's text as the user writes it, and its
  translation to and from Postio's composer document — the single thing both
  frontends' drafts are stored as.
- **Shared store**: the one encrypted store and configuration both frontends
  use, and the coordination that lets two processes use it at once without
  doubling remote work.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Every command in the registry is reachable in the terminal
  frontend by key, palette and (where it has a place on screen) mouse — 100%,
  proven by enumeration.
- **SC-002**: A user can open Postio in a terminal, read the newest
  conversation, reply to it and archive it in under 15 seconds without leaving
  the keyboard, with the network absent.
- **SC-003**: With a populated store, the terminal frontend shows a usable
  inbox in under 500 ms and redraws after any keystroke in under 16 ms; local
  search answers in under 100 ms.
- **SC-004**: The terminal frontend's installed size is under half that of
  the desktop app's, and its resident memory showing the same mailbox is under
  half as well.
- **SC-005**: Across every HTML message in the corpus, the terminal reader
  shows zero raw tags, zero script content, zero unrequested network requests
  and zero terminal control sequences from message content.
- **SC-006**: A message composed in Markdown and sent from the terminal is
  indistinguishable to its recipient from the same content composed in the
  desktop app.
- **SC-007**: With both frontends open on one store, an action in either is
  visible in the other within one second, and across a scripted session of
  mixed actions no message is sent twice and no remote operation is performed
  twice.

## Assumptions

- **Linux terminals first.** Target is a modern terminal emulator on Linux
  (the v1 platform), local or over SSH. Other operating systems are not in
  scope, though nothing here should rule them out.
- **Same bindings, not new ones.** The terminal frontend uses the registry's
  default keys unchanged; where a terminal cannot deliver a key the desktop
  uses (some `Ctrl+Shift` combinations), the command gets an additional
  terminal-reachable default rather than a different one.
- **Markdown dialect** is CommonMark with the common extensions for tables,
  strikethrough and autolinks. Formatting the composer document model cannot
  represent is sent as its plain Markdown text.
- **Images are not drawn in the grid in this feature; they are the next
  iteration** (maintainer, 2026-09-23). Here they are labelled placeholders
  that open in the system viewer. Nothing in this feature may make drawing
  them later harder: the placeholder occupies the place in the document the
  image will.
- **Drag and drop and paste follow the established terminal-app pattern**
  (maintainer, 2026-09-23, citing Claude Code): a terminal delivers a dropped
  file as its path in a bracketed paste, and an image on the clipboard is read
  from the system clipboard when the user pastes. Both are what the composer
  recognises.
- **Notifications** use the desktop notification service where one is
  reachable and an in-screen notice otherwise.
- **"Blazing fast"** means the constitution's existing budgets, which are
  already the desktop app's; the terminal frontend must meet them, not a
  separate faster set.
- **"Smaller"** means installed size and resident memory (SC-004), measured
  against the desktop app built from the same commit, form for form: the
  standalone download against the desktop app's binary with its libraries,
  and the terminal flatpak (with its runtime) against the desktop flatpak
  (with its runtime).
- **Shared-store concurrency** is a requirement set by the maintainer
  (2026-09-23). How two processes coordinate on one encrypted store, and which
  of them runs sync, is the plan's to decide; the current desktop app is
  single-instance within itself and has not been built for a second process.
- **One landing, whole** (clarified 2026-09-23). The feature branch lands on
  `main` once, when all seven user stories meet their acceptance scenarios.
  Priorities order the work on the branch; they are not release slices. The
  branch is rebased onto `main` as it goes, and every commit on it keeps the
  GTK and macOS frontends green (FR-005).
- **Reuse over invention** (maintainer, 2026-09-23: *"I dont want to
  reinvent the wheel here"*). The plan surveys what existing terminal
  applications and libraries already do — layout, mouse handling, Markdown
  display, in-terminal text editing, drop and paste, `$EDITOR` handoff — and
  adopts it. Which libraries is the plan's decision, not this spec's.
- **Nothing is taken away from the other frontends** (maintainer, same day).
  Refactoring the GTK app and the macOS frontend to share logic with the
  terminal frontend is in scope; changing what either does is not (FR-005).
