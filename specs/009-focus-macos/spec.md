# Feature Specification: Postio Focus on macOS

**Feature Branch**: `feature/focus-macos`

**Created**: 2026-10-07

**Status**: Draft

**Input**: User description: a design handoff for "the macOS version of
Postio Focus" (`Design/focus-macos-design/`: `SPEC.md`, `KEYS.md`,
`message-window/SPEC.md`, 25 screens at 1440×900 numbered as the Linux
handoff), with the brief "same product, native chrome; one engine; native,
fast, keyboard-first". Refined by the maintainer on 2026-10-07 with the four
decisions recorded below as M1 to M4.

## Context

Postio is one product with several interfaces (ADR 0043, spec 007 C27). Its
GTK interface is Focus, and so is its terminal (C29). The Mac app in `macos/`
is not: it was built as the classic three-pane app (ADR 0019, #1306), with a
sidebar, a list over a mailbox and a conversation reader. This spec rebuilds
the Mac app as Focus, so that all three interfaces are the same product over
the same engine.

What the Mac has today, and what it lacks for Focus
(`specs/007-postio-focus/macos.md`, which this spec folds in):

| Today | For Focus |
|---|---|
| The boundary to the engine exists (`postio-ffi`, one `Session`, an event stream) and is tested on both platforms | It registers as the three-pane app, so the registry withholds every Focus command from its keys, palette and menus |
| The list is a native table over the shared list window | It has no Focus list (inbox-first, has-action), no markers, no day headers in Focus's form |
| Reading is one document in one web view (ADR 0032) | Focus opens one message at a time in its own window, with two body treatments |
| Composer, first run, accounts, settings, notifications, find | Usable as they are, in Focus's frames |
| — | No filtered view, digests, digest rules, pickers at the row, capture, `postio://`, or engine-backed undo in the Edit menu |

The Linux app also holds Focus's *behaviour* in its window code
(`crates/postio-gtk/src/window.rs`): what each command does in Focus, which
surface a key goes to, keeping the cursor across the has-action filter and
across undo, feeding the list, the header strip's counts, and each surface's
own state. A second app cannot share behaviour that lives in the first app's
window. Moving it to a toolkit-free layer is part of this feature, not a
precondition for it.

### What this spec inherits

- **Spec 007 in full**: its user stories, its requirements, and its table of
  decisions C1 to C30. Where this spec is silent, spec 007 governs the Mac
  exactly as it governs Linux.
- **The design for the Mac**: `Design/focus-macos-design/` (untracked; it is
  the maintainer's reference set and names the maintainer, so it is read in
  place and never copied into this public repository). Its `SPEC.md` "What
  changes from Linux" table is the authority for chrome; spec 007 is the
  authority for behaviour (M2).
- **ADR 0019** for the boundary (one engine, an FFI, SwiftPM, Linux stays
  green), less its three-pane surface, which this spec retires.
- **ADR 0032** (one document per reading surface), **ADR 0041** (every
  frontend reaches mail through the client), **ADR 0042** (the reading
  renderer is disconnected), **ADR 0043** (Focus is the one desktop design),
  **ADR 0044** (interactions are storyboarded).
- **The constitution**, in particular II (one command table), IV
  (test-first), V (performance), VI (privacy) and VII (boundaries).

### Where the inputs disagree

Decisions the maintainer took on 2026-10-07, against the Mac design pack or
filling its silences:

| # | Topic | Decision |
|---|---|---|
| M1 | Window geometry | The Mac pack's numbers apply **on the Mac only**: message window `clamp(640, W − 2·max(96, 0.18·W), 720)`; text column `min(560, w − 80)`; paper column `min(640, w − 80)`; More fold below 700; the digest window is the message window's size with a 560 column. Linux keeps spec 007's numbers. Both live in the one shared geometry, keyed by platform |
| M2 | Pack vs spec 007 | **Spec 007's recorded decisions win** wherever the Mac pack contradicts them. In particular: C3, bindings under `[keys]` in `config.toml`, not `keys.toml`; C4, Filtered never deletes; C6, C8, C9, gating on a model, the detector and a vault; C10, header counts only while in use; C12, Delete is undoable; C19, `X` selects every conversation in the view; C24, ⌘K opens the bar with `>` typed; C26, the action card is the accent at 8% light and 12% dark; C7, no Markdown toggle; C25, a body drawn in app colours is set in Barlow, the chrome in the system font; C30, every list opens with the cursor on its first row |
| M3 | Features Linux lacks | Task after sending (⌘T), a separate Bcc key (⌘⇧B), show quoted text (⌘⇧Q), the row's "Task in <project> · due <day>" line (spec 007 FR-181), a digest reference's email shown under its paragraph, and the action row on an email opened from a digest are built **after the Mac reaches parity, in the shared layers, so both apps get them** |
| M4 | Secondary windows | **One secondary window at a time** on the Mac (email, digest, compose, capture). Raw source and a message opened from a digest's list replace the window's content in place rather than stacking. The reading pane beside the list (`F8`, spec 007 FR-038) comes after parity |
| M5 | Message bodies | The Mac keeps its web view for message bodies and loads **the same treated document Linux composes**: the treatment decision, app colours, paper and the per-sender choice all come from the shared layers. The contrast guard for colours kept in app colours runs in the shared layers, against the app's own surface, so both apps keep the same colours. Drawing Mac bodies with Linux's renderer (ADR 0042) stays a later option; spec 006 left the Mac on its web view, and this keeps it there |

Mac platform conventions this spec takes without a decision, because the Mac
pack's "standard macOS keys" require them and they change nothing on Linux:

| # | Convention |
|---|---|
| M6 | ⌘W closes the focused window (on Linux `ctrl+w` stays Quit); ⌘Q quits; ⌘N composes, as `c` does; ⌘, opens Settings; ⌘F searches from the main window and finds in the message window; Delete is ⌫ on the Mac, where most keyboards have no forward-delete key. Each is a per-platform default of the one registry, not a second keymap |

## User Scenarios & Testing *(mandatory)*

### User Story 1 - The Mac opens on the Focus inbox (Priority: P1)

A person launches Postio on the Mac and sees the Focus inbox: a unified
toolbar (compose on the left; sync status and a search field on the right),
the header strip (Inbox ▾ with its count, the Has action toggle, and the
filtered and digest counts when in use), and one row per conversation,
newest first under day headers. Rows with an invite, question or to-do grow a
second line with the kind, the date, the sentence quoted verbatim and the
one action that answers it, with its key. Digest rows show a stacked icon.
There is no sidebar.

**Why this priority**: It is the product's home, and every other story
starts from it. It also proves the engine's Focus list reaches the Mac.

**Independent Test**: Launch the bundle over the seeded demo store. The
window matches screen 01 in light appearance and 02 in dark, with every
difference listed and explained. The list scrolls without dropped frames
over a store of 10,000 conversations.

**Acceptance Scenarios**:

1. **Given** a store with mail, **When** the app opens, **Then** the inbox is
   shown in arrival order with the cursor on the first row (C30), and
   sender, subject and first line appear exactly as sent.
2. **Given** a message the engine has marked with an action, **When** its row
   is drawn, **Then** it has a second line with the kind, the date, the
   quoted sentence and the action's button with its keycap, in the system
   accent.
3. **Given** the system appearance changes between light and dark, or the
   system accent colour changes, **When** the window redraws, **Then** every
   colour follows without a restart and the accent appears only on markers
   and keyboard focus.
4. **Given** the classic three-pane Mac app's surfaces, **When** this story is
   done, **Then** none of them remain: no sidebar, no conversation rail, no
   three-pane commands offered on the Mac.

---

### User Story 2 - Triage from the keyboard (Priority: P1)

The person moves with `j`/`k`, selects with `x`, extends with ⇧J/⇧K,
selects all with `X`, clears with Esc, toggles Has action with `!`, and acts
with single keys: archive `a`, reply `e`, snooze `s`, label `l`, move `m`,
mark read `r`, delete ⌫. While rows are selected, the action bar at the bottom
shows the count, the actions with their keys, and hints. Every action is
undoable with ⌘Z, which also appears in Edit › Undo with the action's name.

**Why this priority**: Keyboard triage is the product. Without it the Mac is
a viewer.

**Independent Test**: Over the demo store, select three rows, archive them,
see the undo pill ("Archived 3 messages · Undo ⌘Z"), wait for it to fade,
choose Edit › Undo "Archive 3 messages", and see the three rows return with
the cursor where it was. Screen 03 and 15 match, differences listed.

**Acceptance Scenarios**:

1. **Given** the cursor on a row, **When** `x` is pressed, **Then** the row is
   selected, shows a checked box, and the cursor does not move; the cursor
   and the selection are never the same thing.
2. **Given** a selection, **When** `!` is pressed, **Then** only rows with an
   action are shown, the selection clears, and the cursor stays on the same
   message when it is still shown.
3. **Given** a text field has the keyboard, **When** a single letter is typed,
   **Then** it goes into the field and no command fires.
4. **Given** an undoable action has completed, **When** the Edit menu opens,
   **Then** Undo names that action, and choosing it undoes exactly what ⌘Z
   would, from the engine's one undo history.
5. **Given** `[keys]` in the Mac's `config.toml` rebinds a command, **When**
   the file is saved, **Then** the running app uses the new key, its menu
   item and its keycaps show it, and the key map (`?`) shows it.

---

### User Story 3 - Read one message in its own window (Priority: P1)

↩ on a row opens the message in its own window, with traffic lights,
centred over the main window, which stays visible and undimmed. It shows only
that message; `[`/`]` step through the thread in the same window, and
`k`/`j` step to the previous and next message in the list without closing.
The action row holds every action with its key. HTML mail is drawn in app
colours or, when it paints its own page, as the original on a paper sheet,
and ⇧O switches between them. Esc or ⌘W closes it and returns the keyboard
to the same row with the selection kept.

**Why this priority**: Reading is the second half of triage, and the body
treatments are the dark-mode fix the design exists for.

**Independent Test**: Open a plain message, an HTML newsletter, and a work
message, each in light and dark, in a 1440-wide and a 1024-wide main window.
Each matches its screen in `message-window/screens/` and screen 04, with the
widths from M1, differences listed.

**Acceptance Scenarios**:

1. **Given** a main window `W` wide, **When** a message opens, **Then** its
   window is `clamp(640, W − 2·max(96, 0.18·W), 720)` wide and `H − 80` tall,
   and stepping with `j`/`k` never resizes it.
2. **Given** a body the engine classifies as painting its own page, **When**
   it is shown, **Then** it is drawn as sent on a white sheet, dimmed in dark
   appearance, never inverted or recoloured, and zoomed to fit its column no
   lower than 0.85.
3. **Given** any body, **When** it is shown, **Then** no remote content is
   fetched, no script runs, and nothing reaches the network unless the person
   allowed images for that sender.
4. **Given** a message window is open, **When** another message, a digest, the
   composer or capture is opened, **Then** it replaces the open window (M4).
5. **Given** a message with an action, **When** it is shown, **Then** the
   action card sits under the sender block and the quoted sentence is
   highlighted in the body.

---

### User Story 4 - Write and reply (Priority: P2)

`c` or ⌘N opens the composer in its own window; `e`, `E`, `f` open a reply,
reply-all or forward pre-filled from the thread. Recipients complete from
Contacts (after asking once) and from past mail. ⌘↩ sends; Send later picks
a time; Remind if no reply (⌘H) shows its date when on. Esc or ⌘W closes the
window and keeps the draft locally.

**Why this priority**: Replying is the most common action an inbox asks for,
but reading and triage work without it.

**Independent Test**: Reply to a question row, close the window with Esc,
reopen the draft, send it; the outbox shows it and the row's marker clears.
Screens 05 and 06 match, differences listed.

**Acceptance Scenarios**:

1. **Given** the composer is open, **When** Esc is pressed, **Then** the draft
   is saved locally and the window closes without asking.
2. **Given** Contacts permission was never asked, **When** the person first
   types in a recipient field, **Then** the system asks once; whatever the
   answer, past correspondents still complete.
3. **Given** a reply, **When** it opens, **Then** no contact list appears
   until the person types in a recipient field.

---

### User Story 5 - Find and go anywhere from one bar (Priority: P2)

`/` opens search and ⌘K opens commands (with `>` typed, C24), dropping down
from the toolbar's search field with no dimming. Plain English becomes
editable operator chips locally; Tab steps into the chips and ⌘⌫ returns to
plain words; saved searches sit on top (⌥1–4) and ⌘S saves one. `in:Folder`
lists a folder; `g o` or Inbox ▾ opens the folders and labels popover. Every
command row shows its key.

**Why this priority**: Without a sidebar, the bar is how the person gets
anywhere that is not the inbox.

**Independent Test**: Type "from ada last week with attachments", see the
chips, open a result, go to `in:Receipts`, open the popover and pick a label.
Screens 07 to 10 match, differences listed.

**Acceptance Scenarios**:

1. **Given** the bar is open over a focused row, **When** a command is chosen,
   **Then** it acts on that row or on the selection that existed when the bar
   opened.
2. **Given** any place in the popover, **When** ↩ is pressed, **Then** it opens
   as a list, the same view as `in:` gives, with the cursor on its first row.

---

### User Story 6 - Pickers at the row (Priority: P2)

`s` snooze, `h` remind if no reply, `l` label and `m` move open popovers
anchored to the focused row (or acting on the selection). Number keys pick a
preset, Tab jumps to a typed date parsed locally ("tue 9am"), ↩ confirms and
Esc closes. Label and Move filter as you type; Label toggles with Space and
can create a label.

**Why this priority**: These are the actions that keep mail out of the inbox
without archiving it.

**Independent Test**: Snooze a row to "tue 9am" typed, label a selection of
two, move one to a folder, and undo each. Screens 11 to 14 match, differences
listed.

**Acceptance Scenarios**:

1. **Given** the same typed text on Linux and on the Mac, **When** it is
   parsed, **Then** both resolve it to the same time.

---

### User Story 7 - The app says what state it is in (Priority: P2)

An empty inbox shows a quiet centred message with the next digest time and
shortcuts that exist. First sync, offline and a sign-in error each show one
full-width strip under the header strip, and the toolbar's sync label
matches. "Update password…" opens a sheet that stores the password in the
Keychain. None of these block reading, searching or acting on local mail.

**Why this priority**: Every surface must handle its states (the UX
invariants); without them the app looks broken when it is only offline.

**Independent Test**: Start with an empty store, with the network off, and
with a revoked password. Screens 16 to 19 match, differences listed.

**Acceptance Scenarios**:

1. **Given** the network is off, **When** the person archives a message,
   **Then** it leaves the list at once and the change is sent when the
   network returns.

---

### User Story 8 - The key map teaches the keys (Priority: P3)

`?` toggles a key map over a dimmed list, grouped as on Linux, generated from
the one command table, naming the file and section that rebinds keys
(`config.toml`, `[keys]`, C3). The menu bar lists every command offered on
the Mac with its key.

**Why this priority**: Discovery matters once the keys exist.

**Independent Test**: Rebind one command, open `?` and the menu bar, and see
the new key in both. Screen 20 matches, differences listed.

**Acceptance Scenarios**:

1. **Given** any command offered on the Mac, **When** the menu bar is read,
   **Then** the command is in it with the key the key map shows.

---

### User Story 9 - Filtered, digests and capture (Priority: P3)

`g f` opens the filtered view with a reason on each row and tabs by reason
(`1`–`7`); `R` restores a message and stops filtering its sender. ↩ on a
digest row opens the digest in its own window, on its summary when a model is
connected (C6) and on its list otherwise; `]`/`[` move between references, ↩
opens a reference's email in the same window, Esc returns. `d` on a message
opens a small sheet to digest that sender, with a preview of what it would
have caught. `t` and `n` open the capture window when a vault is configured
(C9), whose preview is the exact line written, with a `postio://` link that
reopens the message.

**Why this priority**: These are the features that make Focus more than a
fast list, but each depends on configuration the person may not have.

**Independent Test**: Over the demo store with a model stub and a test vault,
walk each surface. Screens 21 to 25 match, differences listed; clicking a
captured line's link in another app brings Postio forward on that message.

**Acceptance Scenarios**:

1. **Given** a captured task line, **When** its `postio://` link is opened
   anywhere on the Mac, **Then** Postio opens that message in the message
   window.
2. **Given** a digest's plain list, **When** a message in it is opened,
   **Then** it opens in the digest's window, not in a second window (M4).

---

### User Story 10 - Both apps gain what the Mac design adds (Priority: P4)

After parity, the features of M3 are built once in the shared layers and
appear on both Linux and the Mac.

**Why this priority**: They are additions to the product, not part of
bringing the Mac to it.

**Independent Test**: Each feature passes its test in the shared layer and
is visible in both apps.

**Acceptance Scenarios**:

1. **Given** a message with an open task in the vault, **When** its row is
   drawn on either platform, **Then** it shows "Task in <project> · due
   <day>".

---

### Edge Cases

- The main window is narrower than 1024: the message window keeps its 640
  minimum and the content column shrinks to `w − 80`; the action row folds
  into More below 700.
- The person resizes a message window: its column is recomputed from the
  window's own width, and `j`/`k` keep the person's size.
- A secondary window is open when the main window is closed: the secondary
  window closes with it, and a draft in it is saved.
- The store was made by a newer or incompatible build: the app says why it
  cannot open and offers to start the store over, as Linux does.
- A key is rebound to one a Mac system shortcut owns: the conflict is
  reported and the default kept, as on Linux.
- An input method is composing text: no key is taken as a command.
- Contacts permission is denied or revoked later: completion falls back to
  past mail without asking again.
- A `postio://` link names a message that is no longer in the store: the app
  says so and opens nothing.
- The engine reports an undoable action while a text field has the keyboard:
  ⌘Z undoes typing in the field; Edit › Undo still offers the mail action
  once the field loses the keyboard.

## Requirements *(mandatory)*

### Functional Requirements

**One product, one engine**

- **FR-001**: The Mac app MUST be a Focus interface: it MUST be offered
  exactly the commands Focus is offered, filtered only by what the Mac
  platform has no surface for, and MUST NOT offer any three-pane-only command.
- **FR-002**: Every rule about what a command does in Focus, which surface a
  key reaches, where the cursor goes after a list change or an undo, which
  counts the header strip shows, and each Focus surface's own state, MUST
  live in a shared, toolkit-free layer that both the Linux and the Mac apps
  drive. Neither app's window code may hold such a rule.
- **FR-003**: Moving a rule out of the Linux app MUST NOT change Linux
  behaviour, and the Linux app MUST build and pass its suites after every
  move.
- **FR-004**: The Mac app MUST reach mail only through the engine boundary;
  it MUST hold no mail logic, no search parsing, no date parsing, no keymap
  parsing, no undo history and no body classification of its own.
- **FR-005**: The classic three-pane Mac surfaces, and any engine export only
  they used, MUST be removed when Focus replaces them (no compatibility
  path).

**Chrome (from the Mac pack's "What changes from Linux")**

- **FR-010**: The main window MUST use a unified toolbar: traffic lights,
  compose on the left, sync status and a search field on the right.
- **FR-011**: The email, digest, compose and capture surfaces MUST be their
  own windows, centred over the main window, with the list visible and not
  dimmed; at most one is open at a time (M4); Esc and ⌘W close it and return
  the keyboard to the same row with the selection kept.
- **FR-012**: The key map and the digest rule MUST be sheets or small panels
  over a dimmed list.
- **FR-013**: The command bar MUST drop down from the toolbar's search field,
  without dimming.
- **FR-014**: Pickers and the folders menu MUST be popovers anchored to the
  focused row or the Inbox ▾ button.
- **FR-015**: Undo MUST show as a pill at the bottom centre of the main
  window.
- **FR-016**: First sync, offline and sign-in error MUST each show as one
  full-width strip under the header strip.
- **FR-017**: Colours MUST be the system's semantic colours, following light,
  dark and the person's accent colour; the accent MUST be used only for
  action markers, the focus ring and the Has action toggle. Default buttons
  MUST be filled with the label colour, not the accent.
- **FR-018**: Chrome MUST be set in the system font and its monospaced face;
  a body drawn in app colours follows C25.
- **FR-019**: A keycap MUST be one reusable element (monospaced 10 pt, 15 pt
  tall, 1 px outline, radius 3), and every keycap MUST be spelled by the
  shared hint code from the registry (C22).

**Screens**

- **FR-020**: Screens 01 to 25 MUST behave as spec 007 specifies for the same
  screen number, with the chrome of FR-010 to FR-019 and the geometry of M1.
- **FR-021**: The message window MUST follow `message-window/SPEC.md` for its
  chrome, its single centred content column, its vertical rhythm, its
  components and its two body treatments, with C25 and C26 applied (M2). The
  previous/next pair sits on the right, after one `k j` keycap.
- **FR-022**: The body treatment MUST be decided by the engine, and the same
  message MUST get the same treatment on Linux and the Mac. ⇧O switches it,
  and "Always for this sender" is stored where Linux stores it.
- **FR-023**: The body view MUST run with script off, load from memory with
  no base address, and block remote content unless allowed for that sender.

**Keys and menus**

- **FR-030**: Every key MUST be resolved by the shared key resolver from the
  one command table, including two-key sequences such as `g o`.
- **FR-031**: Bindings MUST be overridable from `[keys]` in `config.toml` in
  the Mac's configuration folder (`~/Library/Application Support/Postio/`),
  in the same format as Linux, and a change MUST take effect without a
  restart.
- **FR-032**: Single-letter keys MUST NOT fire while a text field has the
  keyboard or an input method is composing.
- **FR-033**: The menu bar MUST list every command offered on the Mac with
  its key, generated from the one command table, so that it doubles as
  discovery.
- **FR-034**: The conventions of M6 MUST be per-platform defaults in the one
  command table, and a test MUST assert the whole keymap resolves on both
  platforms.

**Undo**

- **FR-040**: The engine's undo history MUST be the only undo history for mail
  actions. ⌘Z, the pill's Undo and Edit › Undo MUST all undo the same entry,
  and Edit › Undo MUST name it.
- **FR-041**: ⌘Z MUST undo the last mail action even after the pill has gone,
  and MUST undo typing when a text field has the keyboard.

**Updates**

- **FR-045**: Sync progress, new mail, list changes, marker changes and
  completed or undone actions MUST reach the Mac from the engine as they
  happen, without the Mac polling for them.

**System integration**

- **FR-050**: Passwords and tokens MUST be stored in the Keychain through the
  engine's existing credential store.
- **FR-051**: Recipient completion MUST combine Contacts, asked for once, with
  past mail.
- **FR-052**: The `postio://` scheme MUST be registered, and a
  `postio://message/<id>` link MUST open that message.
- **FR-053**: Notifications, `mailto:` handling and the first-run account flow
  MUST keep working in the Focus app.
- **FR-054**: Nothing MUST leave the machine that the person did not ask for
  (constitution VI): no prefetch, no remote images by default, no telemetry.

**Order and verification**

- **FR-060**: The work MUST be built in the order of the design brief: the
  inbox (01, 02); keys, selection, action bar and Has action (03); the email
  window (04); compose and reply (05, 06); the command bar, search, go-to and
  folders (07 to 10); pickers and the undo pill (11 to 15); app states (16 to
  19); the key map (20); filtered, digest, digest this sender and capture (21
  to 25); then M3. Each step MUST leave the app runnable.
- **FR-061**: A screen MUST NOT be called done until it has been captured at
  1440×900 in light and dark appearance and compared with its PNG, with every
  difference listed and either fixed or explained. The message window MUST
  also be compared with every screen in `message-window/screens/` and checked
  with a 1024-wide main window.
- **FR-062**: Shared rules MUST be tested in the shared layer. On the Mac, the
  keymap layer and the view models MUST be tested without a window.
- **FR-063**: Every interaction this spec changes MUST be written as a
  storyboard in the toolkit-neutral format of spec 008, before it is built;
  a storyboard Linux already has applies to the Mac unchanged. Linux films
  them, which also guards the shared controller. **Maintainer (2026-10-07):
  the Mac storyboard runner (spec 008's later phase) is not in scope**; until
  it exists, a Mac landing that changes an interaction is labelled
  `interactions-unreviewed`, and FR-061's comparison is the Mac's check.

### Key Entities

- **Focus controller**: the shared, toolkit-free owner of Focus's behaviour.
  It takes resolved keys and engine events, holds the cursor, selection,
  current place and filter, and each surface's state, and tells the app what
  to show: open this message, show this picker at this row, show this
  notice.
- **Window geometry**: the per-platform sizes of the message and digest
  windows and their content columns, computed from the main window's size.
- **Body treatment**: app colours or original on paper, decided per message by
  the engine, overridable per message and per sender.
- **Undo entry**: one completed, undoable mail action with its name, held by
  the engine.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: All 25 screens and all 9 message-window screens are captured in
  light and dark and compared with their designs, with every difference
  listed; none is unexplained.
- **SC-002**: The Mac app offers exactly the commands Focus offers, less those
  the Mac has no surface for, and a test proves it on both platforms.
- **SC-003**: No rule listed in FR-002 exists in more than one app's code:
  each is defined once and used by both apps.
- **SC-004**: The list scrolls a 10,000-conversation store without visible
  stutter, opens in under 500 ms, and every key press is answered within one
  frame (16 ms), as the constitution requires.
- **SC-005**: A person can triage a day's inbox (open, reply, archive, snooze,
  label, undo) on the Mac without touching the pointer.
- **SC-006**: The same message gets the same body treatment, and the same
  typed date resolves to the same time, on Linux and the Mac.
- **SC-007**: Linux's suites pass after every change this feature makes to a
  shared layer.

## Assumptions

- The binding technology, the Swift package layout without an Xcode project,
  and the bundle and CI scripts stay as ADR 0019 set them; how the work is
  built is the plan's to decide.
- The app is not sandboxed, as today; security-scoped bookmarks for the vault
  and attachments are needed only if it becomes so, which is out of scope.
- The engine's existing requests already answer every Focus surface on Linux,
  so the Mac needs new exports, not new engine behaviour; any genuinely new
  engine request is the smallest one that serves and is reported.
- The Linux defects found while comparing the two (`t`/`n`/`d` dropped in the
  open-message dialog, tracked on #1754) are fixed in the shared controller
  once the dialog's key routing moves there.
- Shared code that the Mac app needs must not require the Mac's windowing
  toolkit, so that the same layer can later serve iOS (#1264).
