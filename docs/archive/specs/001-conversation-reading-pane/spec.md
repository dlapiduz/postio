# Feature Specification: The Conversation Reading Pane

**Feature Branch**: `001-conversation-reading-pane`

**Created**: 2026-09-08

**Status**: Draft

**Input**: User description: "Lets work on the spec for the 'preview'/read message pane. We had some issues with it. It should show a header with the information about the message or conversation. The action buttons (reply, forward reply all) should apply to the message or the last message of the conversation and should have an option to see actions for a specific message in a thread. The view should include all messages in a thread with a collapsible view. The mail should render the layout the sender intended."

## Context

The reading pane is where a user spends most of their time in Postio, and it is
the surface with the most open complaints against it:

| Complaint | Where |
|---|---|
| A thread costs one web process per expanded message; nothing releases them | ADR 0032, #1316 |
| Black flicker moving between messages | ADR 0032 (observed live) |
| A reading pane with no header cannot be safely acted on | #1259 |
| The same six header rules exist twice, in two frontends | #1285 |
| Reported repaint when a message is marked read | #946 |

This specification describes what the pane must *do*, so that those defects
have a definition to be fixed against. It does not choose a rendering
mechanism — ADR 0032 is Proposed and its experiment (#1316) is running.

**One thing here is a change of direction, not a defect.** "The mail should
render the layout the sender intended" is not currently true and is not
currently *intended* to be true: `postio-body`'s sanitizer removes `<style>`
tag-and-contents and strips every inline `style` attribute — the code says
*"so Postio CSS always wins"* — and reader view drops `style`, `bgcolor`,
`width` and `class` on top. Every message today renders under Postio's own
stylesheet and nothing else. That choice is also the stated precondition for
ADR 0032's one-document proposal: messages cannot contaminate one another
precisely because their CSS is gone.

**That posture is now reversed, under one condition.** The pane renders the
sender's CSS, and the privacy guarantee is unchanged: nothing loads from the
network unless the user allowed that sender. This makes two things that were
previously free into explicit work — confining a sender's CSS to its own
message, and blocking the remote resources CSS can name that image blocking
never sees. Both are requirements below.

## Design inputs

A designer's brief and three screens were added on 2026-09-08 and are inputs to
this spec:

| File | What it settles |
|---|---|
| `Design/conversation-rail-brief.md` | The implementation brief — header anatomy, message stack, rail, keyboard, reader view |
| `Design/screens/28-conversation-rail-full-window.png` | The target: full window with folder sidebar, thread list, conversation, rail |
| `Design/screens/29-rail-collapsed-narrow.png` | Narrow window with the rail unmounted, plus rail anatomy and the collapse ladder |
| `Design/screens/30-header-hybrid-actions.png` | The header's action row, close up |

The brief introduces one surface this spec did not have — **a message rail**,
a persistent index of the conversation down the trailing edge — and it settles
several things this spec had left open. Both are absorbed below.

**The brief is written against a generic web frontend and three of its
assumptions do not hold here.** They are recorded in Conflicts rather than
silently applied: two contradict decisions already taken, and one is an
internal error in the brief's own keyboard table.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Know who wrote this, and act on it (Priority: P1)

A user opens a message and can immediately see who it is from, who it was
addressed to, what it is about and when it arrived — and can reply, reply all,
forward or archive it without touching the keyboard or knowing a shortcut
exists.

**Why this priority**: A mail client that does not say who a message is from is
one you cannot safely act on — every phishing judgement starts with the sender,
and the remote-image and unsubscribe decisions are made per sender. This is
also the one complaint reported by a user against a shipped build (#1259).
Everything else in this spec assumes the header exists.

**Independent Test**: Open any single message and confirm the four facts are
drawn and the four verbs are clickable. Delivers a usable reading pane on its
own, with no thread behaviour at all.

**Acceptance Scenarios**:

1. **Given** a message is open, **When** the pane draws, **Then** the sender's
   display name and address, the recipients, the subject and the arrival time
   are all visible without scrolling.
2. **Given** a message is open, **When** the user clicks Reply, **Then** the
   composer opens addressed to that message's sender, and the action taken is
   the registry's Reply command rather than a local reimplementation.
3. **Given** a message with more recipients than fit the available width,
   **When** the header draws, **Then** the list is shortened in a way that
   states how many are hidden, and the full list is reachable.
4. **Given** a message whose sender is not yet trusted for remote images,
   **When** the pane draws, **Then** a banner states images were blocked and
   offers to allow them for that sender.

---

### User Story 2 - Read a conversation without losing the thread (Priority: P1)

A user opens a conversation of many messages and sees the whole exchange in one
place — every message readable, nothing hidden behind a disclosure — landing on
the most recent message, with a conversation-level header saying what the
exchange is about and who is in it.

**Why this priority**: This is the feature the user asked for first, and it is
what distinguishes a reading pane from a message viewer. P1 alongside Story 1
because a conversation of one message is Story 1 — the two share a surface.

**Independent Test**: Open a thread of ten messages and confirm every message's
body is present and readable in one scroll, and that the pane opens on the most
recent one.

**Acceptance Scenarios**:

1. **Given** a conversation of N messages, **When** it opens, **Then** all N are
   present in order, and the pane header states the conversation's subject and
   its participants rather than only the newest message's.
2. **Given** a conversation of any length, **When** it is drawn, **Then** every
   message's body is present and readable — no message is reduced to a summary
   row, and there is no "N earlier messages" divider to open.
3. **Given** a message containing quoted text or a signature, **When** it is
   drawn, **Then** those parts are folded away behind a marker and the rest of
   the message is not.
4. **Given** a conversation is open, **When** the user presses the thread
   navigation keys, **Then** focus moves between messages and the focused
   message is visibly distinct from the others.
5. **Given** any conversation, **When** it opens, **Then** the pane is
   positioned on the most recent message, whether or not earlier messages are
   unread.
6. **Given** a conversation the user has read before, **When** they reopen it,
   **Then** it opens on the most recent message again — the pane does not
   restore where they previously stopped.

---

### User Story 3 - Act on the right message (Priority: P2)

A user replying to a conversation gets the reply they meant: the conversation's
action buttons act on the latest message, and any individual message in the
thread offers its own actions for when the user means *that* one.

**Why this priority**: Acting on the wrong message of a thread is a mistake a
user cannot see until it is sent. It is P2 only because Story 1 already
delivers correct actions for the single-message case, which is most mail.

**Independent Test**: In a thread, use the conversation action bar and confirm
the composer quotes the latest message; then use one older message's own
actions and confirm the composer quotes that one.

**Acceptance Scenarios**:

1. **Given** a conversation of several messages, **When** the user activates
   Reply from the conversation action bar, **Then** the reply is composed
   against the most recent message in the conversation.
2. **Given** a conversation of several messages, **When** the user opens an
   individual message's actions and chooses Reply, **Then** the reply is
   composed against *that* message.
3. **Given** any message's actions are shown, **When** they draw, **Then** they
   offer at minimum reply, reply all, forward and archive, and each is the
   registry's command.
4. **Given** a destructive action is offered on an individual message,
   **When** it is activated, **Then** it is confirmed or undoable.

---

### User Story 4 - See the message as it was sent (Priority: P2)

A user opening a newsletter, a receipt or a formatted announcement sees the
layout the sender built, not a flattened approximation of it.

**Why this priority**: It is a stated goal and it is the largest visible gap
between Postio and every other mail client. P2 rather than P1 because it
changes a privacy and security posture that currently holds, and because the
pane is usable without it.

**Independent Test**: Render a multi-column newsletter from the fixture corpus
and compare against the same message rendered by a reference client.

**Acceptance Scenarios**:

1. **Given** a message whose sender specified a multi-column layout, **When**
   it renders, **Then** the columns appear as columns.
2. **Given** a message that specifies its own colors and typography, **When**
   it renders, **Then** those are honoured, and the surrounding Postio chrome
   remains visually distinct from the message content.
3. **Given** a message wider than the pane, **When** it renders, **Then** the
   message content is readable without the pane itself scrolling horizontally.
4. **Given** a rendered message, **When** it draws, **Then** no remote resource
   has been requested unless the sender is allowed — including resources named
   by the sender's CSS rather than by an image tag.
5. **Given** two messages from different senders shown at the same time,
   **When** the first specifies aggressive styling, **Then** the second and the
   application's own chrome are unaffected by it.
6. **Given** a bulk message that reader view would simplify, **When** the user
   asks for the original, **Then** the sender's layout is what they get.

---

### User Story 5 - Never lose your place in a long thread (Priority: P2)

A user reading a long conversation always knows where they are in it: a rail
down the trailing edge lists every message, marks the one they are actually
reading as they scroll, and lets them jump to any other.

**Why this priority**: This is what the design buys in exchange for 150px of
width — a 400-line message no longer costs you your place. P2 because the pane
is usable without it and it degrades cleanly to nothing on a narrow window.

**Independent Test**: Open a six-message thread, scroll through it, and confirm
the marked row tracks what is on screen; then click a row and confirm the pane
moves to it.

**Acceptance Scenarios**:

1. **Given** a conversation of several messages, **When** the rail draws,
   **Then** every message has a row showing its position, its sender, and — for
   messages long enough to matter — how long it is.
2. **Given** the user scrolls the conversation, **When** the visible messages
   change, **Then** the marked row becomes the message occupying the most of
   the viewport, not the one most recently clicked and not merely the one with
   the greatest visible fraction of itself.
3. **Given** the user clicks a rail row, **When** the pane moves, **Then** that
   message is brought to the top of the pane and the marked row does not
   oscillate or fight the scroll.
4. **Given** the user walks the conversation from the keyboard, **When** focus
   moves, **Then** the rail and the pane agree, because both went through the
   same entry point.
5. **Given** the marked row changes during a fast scroll, **When** it updates,
   **Then** the change is not animated and does not flicker between rows.
6. **Given** a window too narrow for the rail, **When** the pane draws, **Then**
   the rail is absent, the reading measure is unchanged, and the conversation's
   position is still stated and still reachable.
7. **Given** a single-message conversation, **When** the pane draws, **Then**
   there is no rail and no position indicator.
8. **Given** a screen reader, **When** it reaches a rail row, **Then** the row
   announces its position, sender, date and length even though the visible row
   is terse.

---

### User Story 6 - The pane stays quiet (Priority: P3)

A user reading a message is not interrupted by the pane redrawing, flickering,
or reacting to state changes that do not concern what is on screen.

**Why this priority**: These are the defects that made the pane feel unfinished
(#946, ADR 0032's flicker). P3 because they are quality-of-experience against a
pane that otherwise works, but they are the reason this spec exists.

**Independent Test**: Move through a long thread and observe; separately, let a
message be marked read while staying on it.

**Acceptance Scenarios**:

1. **Given** a message is open, **When** it is marked read by dwell, **Then**
   nothing in the reading pane redraws.
2. **Given** a conversation is open, **When** focus moves from one message to
   another, **Then** no blank or black frame is shown at any point.
3. **Given** a conversation of any length, **When** it is scrolled from top to
   bottom, **Then** the resources it holds do not grow with the number of
   messages visited.
4. **Given** a message's flags change elsewhere in the app, **When** the change
   arrives, **Then** the pane updates only what the change affects.

### Edge Cases

- **Navigating during a backfill.** The ordinary case, not an exception: the
  user arrives at a message whose body has not landed, and it lands while they
  are looking at it. This must cost one render and must not move them.
- **Holding a navigation key down.** The user flicks through a folder faster
  than any renderer can keep up. Nothing may queue behind them.
- **A message with no body yet.** The header and actions are drawn from the
  envelope, which is local; the body area states that it is being fetched. The
  pane must be useful before the body arrives.
- **A message that failed to decode**, or whose body is malformed. The pane
  says so and still offers the header, the actions and the raw source; it must
  not present a truncated body as if it were complete.
- **A thread of 200 messages.** Every message is represented and the pane opens
  within the interaction budget. ADR 0032 records that whether this is a real
  case is an open question; the pane must not fail at it either way.
- **A message that is the only one in its thread.** The pane shows one message
  and no conversation chrome that would imply others exist.
- **A conversation spanning a message that is in the trash or archived.** The
  spec assumes it is shown with its state indicated rather than hidden.
- **A message whose subject differs from the thread's.** The conversation
  header states the thread's subject; the per-message header states the
  message's own when it differs.
- **A message the user is composing a reply to.** The composer takes over the
  reading pane, so the pane must be able to yield and to restore where the user
  was reading.
- **An extremely long single message.** The pane scrolls; the header remains
  reachable.
- **A message with a very large recipient list.** The header shortens rather
  than growing without bound.

## Requirements *(mandatory)*

### Functional Requirements

**The header**

- **FR-001**: The pane MUST show, for the message it is presenting, the
  sender's display name and address, the recipients, the subject, and the time
  it arrived.
- **FR-002**: When the pane presents a conversation of more than one message,
  it MUST show a conversation-level header stating the conversation's subject,
  its participants, how many messages it contains, and the date span they
  cover.
- **FR-002a**: The conversation header MUST be pinned — it does not scroll with
  the messages — and MUST NOT grow beyond two rows at any content length. The
  subject is the most prominent text in the pane, occupies one line, and
  truncates rather than wrapping.
- **FR-003**: Every message in a conversation MUST carry its own header
  identifying its sender and date.
- **FR-004**: Header presentation rules that do not depend on a toolkit — how a
  date reads, how a long recipient list shortens, what an address collapses to
  — MUST have exactly one definition shared by every frontend.
- **FR-005**: The pane MUST show the banner states that apply to the presented
  message: remote images blocked pending a per-sender decision, unsubscribe
  available, reader view active, decode failure.

**Actions**

- **FR-006**: The pane MUST offer reply, reply all, forward and archive as
  visible, mouse-reachable controls.
- **FR-007**: Every action the pane offers MUST be the command registry's
  command, invoked by id, and MUST NOT be a local reimplementation of that
  verb.
- **FR-008**: In the conversation-level action bar, reply, reply all and
  forward MUST act on the **most recent message**, and archive MUST act on the
  **whole conversation**.
- **FR-008a**: Because FR-008's scoping is not self-evident, the header MUST
  state it in the interface itself, and each action's tooltip and accessible
  name MUST say it in words — "Reply to the latest message", "Archive all 6
  messages" — rather than naming the verb alone.
- **FR-009**: Each individual message in a conversation MUST offer a way to
  reach actions that apply to that message specifically: at minimum reply to
  this message, forward this message, and a menu of the rest.
- **FR-009a**: A message's own actions MUST appear on hover or keyboard focus
  and MUST reserve no space when idle — nothing on screen may shift position
  when they appear or disappear.
- **FR-010**: The conversation action bar MUST always act on the most recent
  message, never retargeting as the user scrolls or focuses another message, so
  that its meaning is fixed.
- **FR-010a**: Where a message other than the most recent is focused, the pane
  MUST make an individual message's own actions the visibly available way to
  act on it.
- **FR-011**: Destructive actions offered by the pane MUST be confirmed or
  undoable.

**The conversation**

- **FR-012**: The pane MUST present every message in a conversation, in order.
- **FR-013**: Every message's body MUST be visible in the stack. No message is
  reduced to a summary row, there is no divider standing for a run of hidden
  messages, and there is nothing for the user to expand in order to read the
  conversation.
- **FR-014**: The most recent message MUST be distinguishable — by emphasis and
  a marker — without being given a different layout from the others (FR-052).
- **FR-015**: On opening a conversation, the pane MUST be positioned on the
  most recent message, regardless of which messages are unread.
- **FR-016**: The pane MUST support moving focus between messages in a
  conversation from the keyboard, and MUST make the focused message visually
  distinct.
- **FR-017**: Quoted content within a message MUST be collapsible, and MUST be
  collapsed by default.
- **FR-018**: Reopening a conversation MUST return to the most recent message.
  The pane does not restore a previous reading position — FR-015 is
  unconditional, and the rail is how a user returns to where they were.

**Rendering**

- **FR-019**: The pane MUST render a message's layout as the sender specified
  it, honouring sender-provided styling — both rules carried in the message and
  styling attached to individual elements.
- **FR-019a**: At minimum, the following MUST survive to the screen: structural
  layout (columns, tables used for layout, alignment, relative widths), colour,
  typographic emphasis and font choice, and spacing. A message that arranges
  itself in three columns MUST appear in three columns.
- **FR-019b**: Where a styling property must be refused, it MUST be refused for
  a stated reason — containment (FR-020, FR-021) or privacy (FR-022, FR-023) —
  and the set of refused properties MUST be enumerable and testable. "Dropped
  because it was easier" is not a permitted reason.
- **FR-020**: A sender's styling MUST be confined to the message it arrived
  with. It MUST NOT affect the presentation of the application, of the pane's
  own chrome, or of any other message shown at the same time.
- **FR-021**: A message MUST NOT be able to escape its own bounds — no
  overlaying the application's controls, no positioning that removes content
  from the message's box, no sizing that displaces the interface.
- **FR-022**: The pane MUST NOT request any remote resource on behalf of a
  message unless the user has allowed that sender. This applies to every way a
  message can name a resource, including from within its styling — background
  images, fonts, imported stylesheets — and not only to image elements.
- **FR-023**: A message MUST NOT be able to signal a remote party as a side
  effect of being rendered: no fetch triggered by layout, by a conditional
  style, by pointer interaction, or by font selection.
- **FR-024**: The pane MUST NOT execute any script contained in a message.
- **FR-025**: Postio's own chrome MUST remain visually distinguishable from
  message content, so that a message cannot present itself as the application.
- **FR-026**: A message wider than the pane MUST be readable without the pane
  scrolling horizontally.
- **FR-027**: The user MUST be able to reach the message's original source.

**Behaviour**

- **FR-028**: Marking a message read MUST NOT cause the reading pane to redraw.
- **FR-029**: Moving between messages MUST NOT show a blank or unpainted frame.
- **FR-030**: The resources the pane holds MUST NOT grow without bound with the
  number of messages a user visits within a conversation.
- **FR-031**: The pane MUST be useful before a body arrives: header, actions
  and banners are drawn from what is local.
- **FR-032**: The pane MUST NOT wait on the network to present anything it
  already has locally.

**The message rail**

- **FR-033**: When a conversation has more than one message and the window is
  wide enough, the pane MUST show a rail listing every message in the
  conversation — position, sender, and a length indication on messages long
  enough for their length to matter.
- **FR-034**: The rail MUST mark the message the user is currently reading,
  derived from what is actually on screen rather than from what was last
  clicked.
- **FR-035**: The marked message MUST be the one occupying the greatest visible
  **area** of the pane, not the greatest visible fraction of itself — otherwise
  a short reply fully in view outranks a long message filling most of the
  screen.
- **FR-036**: The marked row MUST NOT oscillate during scrolling: its updates
  are settled rather than continuous, and are never animated.
- **FR-037**: Activating a rail row MUST bring that message to the top of the
  pane, and doing so MUST NOT cause the resulting scroll to re-mark a different
  row — moving the pane and marking the rail MUST NOT be able to drive each
  other.
- **FR-038**: Rail activation and keyboard message navigation MUST resolve
  through one path, so that the two can never disagree about which message is
  current.
- **FR-039**: A message's length indication MUST be derived from stored message
  data, computed when the message is indexed — never by measuring what has been
  drawn.
- **FR-040**: The rail MUST be built from the conversation's own model, so every
  message has a row immediately, independent of whether that message's body has
  been prepared for display.
- **FR-041**: The rail MUST state the user's position in the conversation and
  the current message's length, and MUST offer a way to dismiss the rail.
- **FR-042**: As the window narrows, the rail MUST degrade in defined steps:
  full, reduced to position and sender initials, then absent with the position
  still stated in the header and the index still reachable on demand.
- **FR-043**: The reading measure MUST NOT give up width to the rail. When the
  rail is absent nothing may float over the message content.
- **FR-044**: The rail and the on-demand index MUST be one component with one
  set of behaviours presented two ways, not two implementations.
- **FR-045**: A single-message conversation MUST show no rail and no position
  indicator.
- **FR-046**: The rail MUST be presented to assistive technology as a list of
  messages with the current one marked, and each row's accessible name MUST
  carry position, sender, date and length even though the visible row is terse.
- **FR-047**: Whether the rail is shown MUST persist per window, not per
  conversation.

**Presentation limits**

- **FR-048**: Message content MUST be held to a reading measure and MUST NOT
  become wider than it, whatever the message contains.
- **FR-049**: Content that cannot fit the measure — preformatted text, wide
  tables — MUST scroll within its own block, never widening the message or the
  pane.
- **FR-050**: Messages MUST NOT be given fixed heights. Single-line elements
  truncate rather than wrapping.
- **FR-051**: A conversation of 100 or more messages MUST NOT prepare every
  body for display at once; bodies are prepared as the user approaches them.
- **FR-052**: Every message MUST use the same frame regardless of its state.
  The most recent message may be distinguished by emphasis and a marker, but
  MUST NOT use a different layout.
- **FR-053**: A blocked-images notice MUST belong to the message it concerns,
  never to the pane as a whole.

**The cost of moving**

These are regression locks as much as requirements. Every mechanism named here
was diagnosed on #749 and fixed — fonts are served rather than inlined
(ADR 0023), the view carries its own ground colour, the unconditional repaint
was narrowed. This spec replaces the pane those fixes live in, which is exactly
the circumstance in which they are lost.

- **FR-057**: Moving from one message or conversation to another MUST reuse the
  existing rendering surface. A new surface MUST NOT be created per message,
  per conversation, or per navigation step.
- **FR-058**: One user gesture MUST cause at most one render. Selecting a
  message that is already displayed MUST cause none.
- **FR-059**: What is handed to the renderer for a message MUST NOT carry bulk
  that is identical between messages. Shared presentation resources — fonts,
  stylesheets — are provided once and referenced, never embedded in each
  message's document.
- **FR-060**: While a new message is being prepared, the pane MUST continue to
  show the previous one, and MUST NEVER expose an unpainted surface. The pane's
  own ground colour is the floor at every instant.
- **FR-061**: Navigating faster than the pane can render MUST coalesce: only
  the selection the user settles on is rendered, and superseded work is
  abandoned rather than queued.
- **FR-062**: When content arrives for the message already displayed — a body
  completing during backfill, an attachment payload — the pane MUST update only
  what changed, MUST NOT re-render the whole document, and MUST NOT move the
  user's scroll position.
- **FR-063**: Leaving a conversation MUST release what it held. The pane's
  resource use MUST be bounded by what is currently displayed, not by how many
  conversations have been visited in the session.
- **FR-064**: Arriving at a message whose body is not yet local, then having it
  arrive, MUST cost **one render per state the user is actually shown** — the
  waiting state, then the body — and no more. Under this project's
  backfill-first sync this is the ordinary case, not an edge case.

  *Corrected 2026-09-08.* This said "one render, not two", which was wrong:
  the waiting plate and the body are two different things to show, and a rule
  forbidding the second would forbid telling the user anything while they wait.
  What #749 actually found was **repeats** — the same state drawn again — not
  the transition between two states. `app_suite`'s `body_arrives.rs` already
  holds the real line: twenty `BodyLoaded` events for a message already on
  screen produce **zero** further paints.
- **FR-065**: The cost of moving MUST be expressed as counts that are identical
  on any machine — renders per gesture, rendering surfaces created per
  conversation, store queries per open, bytes handed to the renderer per
  message — and MUST be asserted at those counts. A wall-clock assertion is not
  an acceptable substitute.

**Keyboard**

- **FR-054**: Every action the pane offers — including reply to the focused
  message, forward the focused message, and dismissing the rail — MUST exist as
  a registry command with an id, and MUST be reachable from the keyboard and
  from the mouse.
- **FR-055**: Moving between messages in a conversation MUST be distinct from
  moving between conversations in the list.
- **FR-056**: Every control in the pane MUST show a visible focus indicator of
  the application's own, and MUST have a hover state and a context menu
  equivalent.

### Key Entities

- **Conversation**: An ordered set of messages belonging to one account, with a
  subject, a participant set, a message count, and an unread count. What the
  pane presents.
- **Message**: One item in a conversation — sender, recipients, date, subject,
  read and flag state, a body that may not be local yet, and attachments.
- **Presentation state**: Which message is current, derived from what is on
  screen. Not persisted: a conversation always opens on its most recent
  message.
- **Sender trust**: The per-sender decision on remote images, which the pane
  reads to choose a banner and which its banner sets.
- **Reader action**: A verb the pane offers, identified by a registry command
  id, together with the message it would apply to.
- **Message length**: A count derived from a message's text when it is indexed,
  stored with the message, used by the rail to show which messages are long.
  Never a measurement of drawn output.
- **Rail state**: Whether the rail is shown, and which message is current.
  Shown-or-hidden persists per window; the current message is derived from what
  is on screen.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A user can identify the sender, the subject and the arrival time
  of an open message, and reach reply, without using the keyboard and without
  prior knowledge of the interface.
- **SC-002**: Opening a conversation presents its messages within the
  application's interaction budget, and the time to do so does not grow with
  the number of messages in the conversation.
- **SC-003**: Reading through a 50-message conversation from top to bottom
  consumes a bounded amount of system resources — the same order at the end as
  after the first ten messages.
- **SC-004**: No blank, black or unpainted frame is observable when moving
  between messages, at any thread length.
- **SC-005**: In a side-by-side comparison of a formatted newsletter against a
  reference mail client, the structural layout — columns, ordering, image
  placement — matches.
- **SC-006**: Zero remote requests are made while reading messages from senders
  the user has not allowed, measured over a full pass of the fixture corpus —
  counting every way a message can name a resource, not only image tags.
- **SC-006a**: A message built to restyle its surroundings changes nothing
  outside its own bounds, verified against a fixture written to attempt it.
- **SC-007**: A reply composed from the conversation action bar quotes the most
  recent message 100% of the time, and a reply composed from an individual
  message quotes that message 100% of the time.
- **SC-008**: A screen-reader pass over an open conversation reaches every
  message's sender, date and body, every action, and every rail row with its
  position announced.
- **SC-008a**: Scrolling a conversation at any speed, the rail's marked row
  matches the message occupying most of the pane, and never changes more than
  once per settling interval.
- **SC-008b**: Opening a 100-message conversation costs the same time as
  opening a 10-message one, within the interaction budget.
- **SC-009a**: Moving between messages and between conversations creates zero
  additional rendering surfaces, asserted as a count over a walk of the whole
  fixture corpus.
- **SC-009b**: Holding the navigation key through a folder of 200 conversations
  produces no more renders than the number of conversations the user actually
  settles on, and never a black or unpainted frame.
- **SC-009c**: Reading 50 conversations in succession leaves the pane holding
  the same resources as after reading one.
- **SC-009d**: A message arriving mid-backfill for the conversation on screen
  costs one render and does not move the scroll position.
- **SC-009**: The header rules that do not depend on a toolkit have exactly one
  definition in the codebase, verified by a check rather than by review.

## Assumptions

- **The pane presents one conversation at a time**, opened from the message
  list, in the third pane of the three-pane layout. Opening a message in a
  window of its own is out of scope here.
- **Thread membership is already solved.** Threading is local and reconstructed
  by existing rules; this spec consumes threads, it does not define them.
- **A thread belongs to one account**, so the pane never presents a
  cross-account conversation.
- **The composer takes over the reading pane** rather than opening beside it;
  the pane must yield to it and restore afterwards.
- **The registry already holds every verb the pane offers.** This spec adds no
  new commands; it makes existing ones visible and correctly targeted.
- **Attachments have their own presentation** and are referenced here only as
  something a message header must indicate.
- **Reader view stays** as the simplified presentation for bulk mail, with the
  original one activation away. "The original" renders with the sender's own
  styling, on an inset sheet that is visibly not the application.
- **The original ask said "a collapsible view".** That was superseded by the
  design brief and the decision on it: the collapsing that remains is of quoted
  text and signatures within a message, not of messages.
- **No reading position is persisted.** A conversation opens on its most recent
  message every time. Whether the rail is shown persists per window.
- **The design brief's measurements** — the rail's width, the reading measure,
  the header's height, the window widths at which the rail degrades — are
  visual truth and belong to the design canvas, not to this spec. This spec
  states only that the steps exist and what happens at each.
- **"Preview pane" and "reading pane" are the same surface.** This spec uses
  "reading pane" throughout.

## Decisions

Taken by the maintainer, 2026-09-08:

- **Full layout fidelity, with the privacy posture unchanged.** The pane
  renders the sender's CSS (FR-019). The condition attached to this decision was
  explicit — *"as long as we block tracking pixels and loading external
  resources"* — which is FR-022 and FR-023, and which is why those two say
  "including from within its styling" rather than naming image tags. The
  consequence for ADR 0032 is recorded under Dependencies.
- **The conversation action bar is fixed to the most recent message**
  (FR-008, FR-010). It does not follow focus. Acting on an older message is
  done through that message's own actions (FR-009).
- **This spec is mechanism-neutral.** It defines what the pane must do;
  ADR 0032's experiment (#1316) measures and selects how a conversation is
  rendered. Neither pre-empts the other.

Taken against the design brief, 2026-09-08:

- **The brief's layout contract is not adopted.** It requires that *"nothing
  inside a message may set its own width, alignment, font or color"* and that
  reader view *"drop[s] all sender CSS, fonts, widths, colors and layout
  tables"*. Sender styling is rendered instead (FR-019, FR-019a), and where it
  must be constrained, the reason must be containment or privacy and the
  constraint must be enumerable (FR-019b). Reader view survives as the
  simplified presentation for bulk mail, and the brief's **inset sheet** is
  adopted as the containment device for showing a message on its own terms.
- **The brief's "nothing is hidden" is adopted, with the landing position
  changed.** Every message's body is visible; there are no collapsed messages
  and no "N earlier messages" divider (FR-013). Quoted text and signatures
  still fold (FR-017) — that is within a message, not a message. **The pane
  opens on the most recent message** (FR-015), which is *not* what the brief
  implies and *not* what ADR 0015 says. This supersedes ADR 0015 twice over,
  and that ADR must be amended: it requires read messages collapsed, and it
  puts the first unread in focus.
- **The brief's proposed keyboard shortcuts are not adopted.** Keys come from
  the command registry, which already binds reply, reply all, archive, archive
  thread and thread navigation. Verbs the pane newly needs — reply to the
  focused message, forward the focused message, dismiss the rail — are added to
  the registry and take their keys from it (FR-054). The brief's table is
  disregarded, which also disposes of its `E` / `⇧e` collision.

## For the designer

Two items in the brief need no decision here but should go back:

- **The keyboard table collides with itself.** `E` for "reply all to latest"
  and `⇧e` for "reply to the focused message" are the same keystroke. The
  proposals are not being adopted, but the table should be corrected before it
  is reused.
- **Section 1 describes a bug this project does not have.** The brief opens
  with thread grouping as *"a correctness bug — do this first"*, on the premise
  that the list renders one row per message. Postio decided one row per thread
  in ADR 0015 and has the `threads` table and repository, `RowKind::Thread` in
  `postio-ui/src/list.rs`, and shared participant logic in
  `postio-ui/src/conversation.rs`. The first item in the brief's order of work
  is largely already built.

## Dependencies

- **ADR 0032 (Proposed)** and its experiment **#1316** decide the rendering
  mechanism this spec sits on top of. **FR-019 removes a precondition that ADR
  0032 relies on**: it argues one document is safe here because sanitizing has
  already deleted every sender's CSS, so messages cannot contaminate each other.
  Once CSS is rendered, FR-020 is work that ADR 0032 currently assumes it gets
  for free, and the ADR should be amended to say so.
- **ADR 0015 (Accepted)** must be amended by this work. FR-013 supersedes its
  *"read ones collapsed"*, and FR-015 supersedes its *"first unread in focus"*.
  `postio-ui/src/conversation.rs`'s `collapsed_runs()` and its tests are the
  shipped implementation of the superseded rule and are removed with it.
- **#1285** removes the duplicate header rules FR-004 forbids.
- **#1259** is the same header requirement for the macOS frontend.
- **#946** is the reported repaint FR-028 forbids.
- **#749 (closed)** diagnosed the four mechanisms behind "the pane flashes black
  between messages": a full document teardown per switch, no ground colour on
  the view, ~1.2 MB of inlined fonts in every document, and several gestures
  issuing two loads. All four were fixed. FR-057 to FR-065 exist so that
  replacing this pane does not reintroduce them, and its issue body is the best
  available description of what to measure.
- **ADR 0023 (Accepted)** is why FR-059 is satisfiable: the reader's fonts are
  served over a scheme rather than inlined into every document.
- **`postio_storage::test_support::counting`** is how FR-065's counts are read
  for anything that touches the store. Counts that concern rendering rather
  than storage need an equivalent, and this spec assumes one is built rather
  than assuming timings will do instead.
- **The fixture corpus** must gain a multi-column newsletter, a decode-failure
  message, and a message that attempts to restyle its surroundings and to fetch
  remote resources from its styling — for SC-005, SC-006, SC-006a and the decode
  edge case.
