# Feature Specification: Threads in Postio Focus, Mac and Linux

**Feature Branch**: `feature/focus-threads`

**Created**: 2026-11-11

**Status**: Draft

**Input**: Issue #2099 ("Lost the ability to see threads": conversations are
threaded, but only their quotes can be read, not their messages), and a
design handoff commissioned to answer it (`Design/focus-macos-threads/`:
`SPEC.md`, 11 screens at 1440×900, `source/*.dc.html`). The maintainer asked
for the same experience on the Mac and on Linux.

## Context

Focus already threads. What it lost, in the move away from the stacked
conversation reader (spec 007 C2, a deliberate departure from ADR 0032), is
any way to *see* a thread: the list shows one row per conversation with no
way into it, and the message window shows one message with a small
"earlier / later" chip. The design keeps one message at a time and makes the
thread visible around it. What it asks for, against what the engine has:

| The design asks for | The engine has today |
|---|---|
| Threads built from `Message-ID`, `In-Reply-To`, `References`; subject only as a fallback, within 7 days, never for digests and newsletters | Incremental JWZ (`postio-model/src/threading.rs`): references first, then subject when the subject carries reply decoration. **No time window, and no exclusion for list or bulk mail** |
| A thread spans folders and includes the user's sent replies | Yes: `Thread::mailbox_ids` spans folders; threads never span accounts |
| Archive, snooze, labels, folder, remind and mute belong to the thread; read, markers, HTML override and attachments to the message | Mixed. `archive` (`e`) and `archive_thread` (`E`) are separate commands; labels are a union of the messages' labels; **no mute** |
| Split one message (and its replies) into its own thread, remembered | **Nothing** |
| Lists collapsed by default, one thread expandable in place | Rows are conversations (`postio-ui/src/focus_list.rs`, `FocusRow::Conversation`), drawn from the newest message. **No expansion** |
| Participants "Lena, Ben, Ana, you", count, "2 new", "Ben: Looks good…" | A thread summary with count and unread; the row draws the newest message's sender and snippet |
| Message window: thread bar of chips, `[` `]` `{` `}`, first unread, older-message banner, Reply to row, thread menu, outline | One message, a position line, and a thread chip stepping with `n`/`p` (`postio-ffi/src/focus_message.rs`, GTK `open.rs`). **`[`/`]` are unbound in the reader**, though spec 007 C2 names them |
| Search grouped by thread, matching messages inside, opened at the match | Conversation search (`postio-index/src/executor/conversations.rs`, spec 010): one row per conversation with a match count; Quick Look lists the matches (#2085, #2098). **Rows do not expand, and ↩ opens the latest message** |
| A draft reply shown as the thread's last item | `Draft` carries `in_reply_to` and `thread_id`, so it can be placed |
| Reply to an older message, and forward a whole thread three ways | Reply quotes the message replied to; forward is one message |

### What this spec inherits

- **Spec 007** (Focus on Linux) and **spec 009** (Focus on the Mac) in full;
  where this spec is silent, they govern. In particular 007 C2 (one message
  at a time), 009 FR-002/FR-004 (behaviour lives in `postio-focus`; Swift and
  GTK decide nothing) and FR-061 (screens compared at 1440×900).
- **Spec 010** (search): the results view, Quick Look and its match count.
- **The design**: `Design/focus-macos-threads/SPEC.md` and `screens/01`–`11`
  (untracked; read in place, never copied here), with the general Mac rules
  in `Design/focus-macos-design/` (`SPEC.md`, `KEYS.md`).
- **ADR 0044** (interactions are storyboarded), **ADR 0045** (behaviour in
  `postio-focus`), **ADR 0049** (each layer asserts its own question).
- **The constitution**, in particular I (local-first), II (the keyboard is a
  system), IV (test-first), V (performance, gated as counts) and VI (privacy).

### Decisions the maintainer took (2026-11-11)

| # | Topic | Decision |
|---|---|---|
| T1 | Platforms | **Mac and Linux together.** Every slice lands on both Focus apps, from the same rules; only the chrome differs |
| T2 | First unread | **`g u`** in the message window, joining the go-to family. `u` keeps Gmail's meaning (back). Inside the outline, where there is no "back", bare `u` is first unread |
| T3 | Outline and whole-thread forward | **Outline is `alt+t`.** There is no whole-thread forward key: `f` in a thread of two or more opens compose with the *What to forward* chooser on "This message", so the whole thread is `f` then `2`. Nothing existing is rebound |
| T4 | The design pack | Stays untracked in `Design/`, cited by path and screen number |

### Decisions this spec takes

**D1. The design's keys are translated, not adopted.** The design was drawn
on the keymap before #1834 adopted Gmail's. Every action keeps the key the
registry already gives it; only actions with no key get one. The footer and
menus read their keycaps from the registry, so they show Postio's keys, not
the screens'.

| Design | Action | Postio |
|---|---|---|
| `e` / `E` / `f` | Reply / reply all / forward (message-scoped, D2) | `r` / `a` / `f` |
| `a` `s` `l` `m` `h` | Archive / snooze / label / move / remind (thread-scoped) | `e` / `b` / `l` / `v` / `h` |
| `⇧F` | Forward the whole thread | `f` then `2` (T3) |
| `⌥E` | Reply to the latest message | `alt+e`, as designed. `alt+r` is Retry send; `alt+e` is taken only by the terminal composer's Edit externally, which this does not reach |
| `⇧M` | Mute or unmute the thread | `m` (free on mail surfaces; Gmail's mute) |
| `⇧T` | Thread outline | `alt+t` (T3; `T` is Capture a task) |
| `⌥M` | Move only this message | `alt+v` (move is `v`; `alt+m` is Mark as sent) |
| `⌥⌫` | Delete only this message | `alt+BackSpace` (taken only in search's field) |
| `⇧O` | Always show this message's HTML as sent | `O`, the existing `switch_treatment`, made to persist per message |
| `[` `]` | Older / newer message | `[` / `]`; `p`/`n` and `alt+Up`/`alt+Down` stay as alternates |
| `{` `}` | First / latest message | `{` / `}` |
| `u` | First unread | `g u` (T2); `u` in the outline |
| `→` / `←` | Expand / collapse in lists | `Right` / `Left`. In Focus lists they stop being alternates of Open and Back; Open stays `Return`/`o`, Back stays `Escape` |
| `⌘G` / `⌘⇧G` | Next / previous match in the thread | `mod+g` / `mod+shift+g`, the existing `find_next`/`find_previous`, which step across messages when the window was opened from search |
| `⌘⇧↓` / `⌘⇧Q` | Compose: show newer messages / expand quoted messages | `mod+shift+Down` / `mod+shift+q` (to be confirmed free in the composer by the plan) |
| `1` `2` `3` | Compose: what to forward | `1` `2` `3` while the chooser is focused |
| `⌘P` | Export thread as PDF | **Out of scope**: Postio has no print path |

**D2. Scope is one function.** Every action resolves its target from the
focus and the action alone, by the design's §3 table:

| Focus | Reply, reply all, forward, flag | Archive, snooze, label, move, remind, mute, delete | Read / unread | Move or delete only this |
|---|---|---|---|---|
| Collapsed thread row | The **latest** message | The thread | The thread | Not offered |
| Child row, or the message window | **This** message | The thread | This message | This message |

Flag follows reply (a flag is a message's, like a marker); delete follows
archive (only `alt+BackSpace` deletes one message). With archive
thread-scoped everywhere, `archive_thread` (`E`) duplicates `archive` and is
removed; `E` becomes free.

**D3. Thread state is stored once, and written through to the messages the
server knows.** Mute, snooze, remind and split are local thread state. Archive
moves every one of the thread's inbox messages; a label is applied to every
message; move moves every message outside Sent and Drafts. A message that
joins a thread later is shown with the thread's state; it is not rewritten on
the server unless an action asks for it, except that a muted thread's new
reply is archived on arrival (FR-012).

**D4. The classic GTK reader and the terminal app keep building and keep
their own views.** They take the model changes (grouping, D2 and D3, mute,
split) because the rules are shared; their layouts are not redesigned here.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - See every message in a thread from the list (Priority: P1)

The inbox shows each thread as one row: participants ("Lena, Ben, Ana, you"),
a count badge, the subject (bold when unread), labels, the latest message's
first line with its sender ("Ben: Looks good…"), "2 new", a paperclip, and
the time. `Right` expands the focused thread in place; its messages appear as
child rows joined by a line, each with an initials avatar, the sender, that
message's own new text (never its quote), "↳ to Ana" when it answers an older
message, and its time. An unsent reply is the last child, in italics, "Draft ·
you … not sent". `j`/`k` walk into and through the children; `Left` collapses.
Only one thread is expanded at a time. `Return` on a child opens that message.

**Why this priority**: This is #2099. Being able to see each message is the
smallest change that gives threads back.

**Independent Test**: In the demo store, open the inbox (screen 01), press
`Right` on "Harbor API draft v3", then `j` four times (screen 02, and 11 in
dark). Each child shows its own text; `Return` opens message 4, not the
latest. Screens compared on both platforms.

**Acceptance Scenarios**:

1. **Given** a thread of six with two unread, **When** the inbox is drawn,
   **Then** its row reads participants newest first with "you" last and a
   badge of 6, the latest message's sender and first line, and "2 new" in a
   label-coloured pill, not the accent.
2. **Given** a one-message thread, **When** it is drawn, **Then** it shows the
   sender's full name and no chevron, and `Right` does nothing.
3. **Given** thread A expanded, **When** `Right` is pressed on thread B,
   **Then** A collapses and B expands, and the focus stays on B's row.
4. **Given** a child row whose message quotes the previous one, **When** it
   is drawn, **Then** its text is the message's new text only.
5. **Given** a focused child row, **When** `Left` is pressed, **Then** the
   thread collapses and the focus returns to the thread row.
6. **Given** a thread row, **When** `Return` is pressed, **Then** the window
   opens at the first unread message; with none unread, at the message with an
   open action marker; otherwise at the latest.

---

### User Story 2 - Every action says what it acts on (Priority: P1)

The footer names the scope ("Harbor API draft v3 · 6 messages", or "Message 4
of 6 · Ben Adeyemi") and lists the keys with their effect ("reply to this",
"archive thread"). The focused child shows a hint: `r` `f` "this one". Reply
and forward act on the latest message from a collapsed row and on *this*
message from a child row or the window; archive, snooze, label, move, remind
and mute always act on the thread.

**Why this priority**: Expansion without clear scope makes a person guess
whether `e` archives one message or the conversation. Every thread action
depends on this rule.

**Independent Test**: Unit-test the scope rule against every cell of D2's
table. Then on both platforms: on a child row press `r` (the reply quotes
that message), `e` (the whole thread leaves the inbox), and `z` (it returns).

**Acceptance Scenarios**:

1. **Given** a collapsed thread row, **When** `r` is pressed, **Then** the
   reply answers the latest message.
2. **Given** child 4 of 6, **When** `f` is pressed, **Then** the forward
   chooser opens on "This message", quoting message 4.
3. **Given** child 4 of 6, **When** `l` adds a label, **Then** every message
   of the thread carries it, and one undo removes it from all of them.
4. **Given** a collapsed row, **When** `alt+v` is pressed, **Then** nothing
   moves and the footer does not offer it.

---

### User Story 3 - Walk a thread in the message window (Priority: P1)

The message window keeps one message at a time and adds a thread bar under
the actions: a "Thread / 6 · 2 new" label, one chip per message (avatar,
"Name · time"), the current chip ringed, unread chips dotted, and a Thread ▾
button. `[`/`]` step older/newer, `{`/`}` first/latest, `g u` first unread.
A kicker reads "Message 4 of 6 · ] newer · [ older". When the message answers
one other than the one before it, the sender block shows a Reply to row
("Ana Ruiz, Mon 21 Sep (message 1)"). When the message is not the latest, a
quiet banner says reply and forward act on it while archive, snooze, label and
move act on the whole thread, and offers "Latest }". `j`/`k` still move to the
next and previous *thread* in the list.

**Why this priority**: The window is where reading happens; without a way
through the thread here, the list is only half the fix.

**Independent Test**: Open "Harbor API draft v3", press `[` twice (screen 03).
Press `}`: the latest is shown and the banner goes. Press `g u`: the first
unread is shown. Screen 03 compared on both platforms.

**Acceptance Scenarios**:

1. **Given** the window on message 4 of 6, **When** `]` is pressed, **Then**
   message 5 is shown, its chip is ringed, and it is marked read.
2. **Given** the latest message, **When** `]` is pressed, **Then** nothing
   moves and nothing errors.
3. **Given** a thread of 40 messages, **When** the bar is drawn, **Then** it
   shows the first chip, a "… N more" chip and the latest that fit, and the
   current and every unread chip are always visible.
4. **Given** message 4, **When** `j` is pressed, **Then** the next *thread*
   in the list opens.

---

### User Story 4 - A reply arrives while reading (Priority: P2)

When a new message joins the open thread, the bar gains a chip with an unread
dot and an accent-tinted banner names the sender and quotes the first line,
ending "Read it }". The window never moves the reader. The list row behind it
updates its participants, count, "1 new" and time.

**Why this priority**: Live arrival is common in active threads, and moving
someone mid-sentence is the worst outcome; but threads work without it.

**Independent Test**: With message 4 open, deliver a reply into the demo
store (screen 08). The shown message, its scroll position and focus are
unchanged; `}` shows the new one.

**Acceptance Scenarios**:

1. **Given** the window open on any message, **When** a reply arrives,
   **Then** the shown message and its scroll offset do not change.
2. **Given** two replies arrive, **When** the banner is drawn, **Then** it
   names the latest and the bar gains both chips.

---

### User Story 5 - Thread menu, outline, mute and split (Priority: P2)

Thread ▾ opens a menu in two groups. **Whole thread:** Outline and filter
(`alt+t`), Forward the whole thread (`f`, `2`), Reply to the latest message
(`alt+e`), Mute thread (`m`). **This message only:** Split into its own
thread, Move just this message (`alt+v`), Delete just this message
(`alt+BackSpace`), Always show its HTML as sent (`O`). Entries may carry a
one-line explanation. The outline lists every message (number, avatar, name,
first line, date, unread dot), filtered by a field (`/`) and by person chips
with counts; ↑↓ move, `Return` shows a message, `u` goes to the first unread.

Muting moves new replies out of the inbox; they stay searchable, and the row
stays where it was, marked "muted" with a count of replies since. Splitting
moves a message and the replies to it into a new thread, and later mail
respects the split.

**Why this priority**: These are the tools for long and misgrouped threads;
the common case is served by Stories 1 to 3.

**Independent Test**: Screens 04 and 05 compared. Mute a thread, deliver a
reply: the inbox does not gain it, search finds it, the row reads "muted · 1".
Split message 4: two threads, and a new reply to message 4 joins the split
one.

**Acceptance Scenarios**:

1. **Given** a muted thread, **When** a reply arrives, **Then** it is archived
   on arrival, the row says "muted" with a count of 1, and search finds it.
2. **Given** a muted thread, **When** `m` is pressed, **Then** it is unmuted
   and later replies arrive in the inbox.
3. **Given** message 4 split off, **When** a reply whose `In-Reply-To` is
   message 4 arrives, **Then** it joins the split thread, not the original.
4. **Given** the outline filtered to "Ben", **When** `Return` is pressed on a
   row, **Then** the window shows that message and the outline closes.

---

### User Story 6 - Search answers in threads (Priority: P2)

Results group by thread. A thread with several matches shows "3 of 6 messages
match" in place of its snippet and, for the top result, is expanded by
default: only the matching messages are children, each tagged with which one
it is and where it matched ("message 5 · file"), with the passage highlighted;
a run of non-matching messages is one short line ("Message 4 doesn't match ·
Ben, Thu"). A single-match thread stays collapsed with "Message 5 of 8 ·
Tomás: …". `Return` on a matching child opens the window *at that message*:
every match is marked in the bar, a banner reads "‘rate limit headers’ · match
1 of 3 in this thread (messages 3, 5 and 6)", `mod+g`/`mod+shift+g` step
through matches across messages, and `Escape` returns to the results with the
same row focused.

**Why this priority**: Search is how people reach old threads, and it opening
at the latest message hides why it matched; but search already works.

**Independent Test**: Search `rate limit headers` (screen 06); `j` to message
3, `Return` (screen 07); `mod+g` twice reaches message 6; `Escape` returns to
message 3's row.

**Acceptance Scenarios**:

1. **Given** a thread where messages 3, 5 and 6 match, **When** results are
   drawn, **Then** its children are those three, with one line for message 4.
2. **Given** the window on the last match, **When** `mod+g` is pressed,
   **Then** it stays and says it is the last match.
3. **Given** a match inside an attachment, **When** its child is drawn,
   **Then** its tag says "file" and the passage names the file.

---

### User Story 7 - Reply to an older message, forward a thread (Priority: P3)

Replying to a message that is not the latest shows a bar under the composer's
title: "You're replying to message 4 of 6, not the latest", a line naming the
newer messages, and two actions: Show them (`mod+shift+Down`, the newer
messages read-only in the composer) and Reply to latest (`alt+e`, which
switches the quote and the threading headers and keeps what was typed).
Forwarding in a thread of two or more opens a *What to forward* chooser:
**1 This message** (quoted as usual, the default), **2 Whole thread** (every
message oldest first in one readable message, each headed by sender and date,
attachments included), **3 As attachments** (each message as an `.eml`). For
the whole thread the subject is "Fwd: <subject> (6 messages)", and the quoted
messages show as collapsed rows that `mod+shift+q` expands.

**Why this priority**: Both are correct today in the simple case; this makes
the uncommon case safe.

**Independent Test**: From message 4 of 6 press `r` (screen 09); `alt+e`
keeps the typed text and now quotes message 6. Press `f`, `2` (screen 10):
the body holds six headed messages and every attachment.

**Acceptance Scenarios**:

1. **Given** a reply to message 4 with text typed, **When** `alt+e` is
   pressed, **Then** the typed text is kept, the quote is message 6's, and
   the reply threads under message 6.
2. **Given** the chooser on 2, **When** the draft is sent, **Then** the sent
   message holds all six messages oldest first and every attachment once.
3. **Given** a one-message thread, **When** `f` is pressed, **Then** no
   chooser is shown.

---

### Edge Cases

- A message with references to two threads (a late parent): the threads merge,
  and an expanded thread stays expanded around the merged one.
- A thread whose messages are in Archive, Sent and the inbox: the inbox row
  counts every message; children include the sent replies.
- A message the user split off that is later referenced by a *new* message
  that also names the original thread: the split wins (it was the person's
  decision).
- Two newsletters with the same subject a week apart, or a weekly digest: never
  grouped by subject. Two replies titled "Re: Hello" more than 7 days apart
  with no references: not grouped.
- An expanded thread loses its last inbox message (archived elsewhere): it
  collapses and the focus moves to the next row, as for any row that leaves.
- An expanded thread gains a message: the new child appears in place, and the
  focused child stays focused.
- A child message whose body is not yet fetched: its row shows the sender and
  time and a quiet placeholder; it never awaits the network to draw.
- A message that is entirely quote, or empty: its child row says so ("Quoted
  reply only") rather than showing the quote.
- A thread of hundreds: the list's child rows and the outline are windowed;
  the bar collapses as in Story 3.
- A draft in a thread that is collapsed: counted in nothing but shown when
  expanded; a draft whose thread is archived stays in Drafts.
- Reduced motion: the chevron rotates and children appear without animation.
- The window open on a message deleted elsewhere: it moves to the nearest
  remaining message of the thread and says so; with none left, it closes.

## Requirements *(mandatory)*

### Functional Requirements

**Model**

- **FR-001**: Threads MUST be built from `Message-ID`, `In-Reply-To` and
  `References`, and MUST include messages in every folder of the account,
  sent replies included.
- **FR-002**: Subject grouping MUST apply only when a message has no reference
  linking it, its subject carries reply or forward decoration, it is not
  mailing-list or bulk mail, and the candidate thread's latest message is
  within 7 days of it.
- **FR-003**: A message that replies to a message other than the one before
  it MUST record which message it answers.
- **FR-004**: Archive, snooze, labels, folder, remind-if-no-reply and mute
  MUST be thread state (D3); read, action markers, the HTML rendering override
  and attachments MUST stay message state.
- **FR-005**: One rule MUST resolve every action's target from the focus and
  the action, exactly as D2's table; frontends MUST NOT decide scope.
- **FR-006**: `archive_thread` MUST be removed, `archive` acting on the thread
  everywhere (D2).
- **FR-007**: Splitting MUST move a message and every message that replies to
  it, directly or not, into a new thread, and MUST be remembered so later mail
  that references them joins the split thread.
- **FR-008**: A draft reply MUST belong to its thread and be ordered last.

**Lists**

- **FR-009**: Every Focus list (inbox, folders, Filtered, search results)
  MUST draw threads collapsed, with the row of Story 1: participants (up to 3
  first names newest first, then "you", then "+N"), a count badge on threads
  of two or more, subject bold when unread, labels, "Sender: first line" of
  the latest message, "N new", a paperclip when any message has an
  attachment, "muted" when muted, and the latest time.
- **FR-010**: At most one thread MUST be expanded per list; `Right` expands,
  `Left` collapses, a click on the chevron toggles.
- **FR-011**: Child rows MUST show the message's own new text with quoted
  text removed, "↳ to <name>" per FR-003, a paperclip, the time, and an
  unread mark; the draft child MUST be drawn as Story 1 says.
- **FR-012**: A muted thread's new messages MUST be archived on arrival,
  remain searchable, and increment a "replies since muting" count shown on
  the row.
- **FR-013**: `Return` on a thread row MUST open the first unread message,
  else the message with an open action marker, else the latest; on a child
  row, that message.
- **FR-014**: The footer MUST name the scope and list the focused row's
  actions with their scope-specific labels; keycaps MUST come from the
  registry.

**Message window**

- **FR-015**: The window MUST show the thread bar, kicker, Reply to row and
  older-message banner of Story 3, and MUST keep current and unread chips
  visible however long the thread.
- **FR-016**: `[` `]` `{` `}` and `g u` MUST move within the thread; `j`/`k`
  MUST keep moving between threads in the list.
- **FR-017**: Showing a message MUST mark it read, as opening does today.
- **FR-018**: A message joining the open thread MUST add a chip and the
  new-reply banner, MUST update the list row behind the window, and MUST NOT
  change the shown message, its scroll position or the focus.
- **FR-019**: Thread ▾ MUST open the menu and outline of Story 5; the outline
  MUST filter by text and by person.

**Search**

- **FR-020**: Search results MUST group by thread as Story 6 says, expanding
  the top multi-match thread by default and collapsing runs of non-matching
  messages into one line.
- **FR-021**: Opening a matching child MUST open the window at that message,
  mark every match in the bar, show the match banner, and let `mod+g` and
  `mod+shift+g` step through matches across the thread's messages; `Escape`
  MUST return to the same result row.

**Compose**

- **FR-022**: Replying to a message other than the latest MUST show the
  older-reply bar; Reply to latest MUST keep the typed text and switch the
  quote and the `In-Reply-To` and `References` headers.
- **FR-023**: `f` in a thread of two or more MUST open the chooser on "This
  message"; the three forms MUST produce what Story 7 says, and the
  whole-thread subject MUST read "Fwd: <subject> (N messages)".

**Keys and platforms**

- **FR-024**: Every key in D1 MUST be in the registry, in `docs/keybindings.md`
  and the in-app key map (`?`), rebindable, and checked for clashes by the
  registry's own tests.
- **FR-025**: Behaviour MUST live in the shared Rust layers (rules in
  `postio-model`/`postio-ui`, the controller in `postio-focus`); the AppKit
  and GTK Focus apps MUST only draw it and route keys.
- **FR-026**: Each slice MUST land on both platforms (T1), with a storyboard
  per changed keyboard path written red first, and a frontend case per path a
  person takes on each platform (ADR 0049).
- **FR-027**: Screens 01–11 MUST be compared at 1440×900 on the Mac, screen 02
  also in dark, and the same screens on GTK under spec 007's role mapping,
  with differences listed.

**Privacy and performance**

- **FR-028**: Nothing MUST be fetched to draw a child row, a chip or the
  outline; a body not yet local shows a placeholder. Whole-thread forward
  MUST NOT send anything the user did not choose.
- **FR-029**: Logs MUST carry thread and message ids and counts, never
  subjects, names or text.

### Key Entities

- **Thread**: a conversation within one account. Holds its messages in date
  order, its participants, counts (messages, unread, replies since muting),
  and its own state: archived, snoozed, remind, labels, folder, muted.
- **Message (in a thread)**: its sender, date, own new text, read state,
  markers, attachments, rendering override, and the message it answers.
- **Split decision**: a remembered rule that a message, and replies to it,
  form their own thread.
- **Thread draft**: an unsent reply, tied to the message it answers and shown
  as the thread's last item.
- **Scope**: the resolved target of an action, either the thread or one
  message.
- **Thread match set**: for a search, which of a thread's messages matched,
  where (text or file), and their order.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: From the inbox, any message of any thread is shown in at most
  two keypresses after reaching its row (`Right` and `j`s, or `Return` and
  `[`/`]`), on both platforms.
- **SC-002**: Expanding or collapsing a thread, and stepping in the window,
  each take under 16 ms, and expanding reads a bounded number of statements
  whatever the thread's length (gated as counts).
- **SC-003**: Opening the inbox stays under the 500 ms startup budget with
  threads collapsed; no list reads a whole mailbox or a whole thread to draw a
  row.
- **SC-004**: Every cell of D2's table is covered by a test of the scope rule,
  and every action in the footer says thread or message.
- **SC-005**: Over the threading corpus (replies, forks, cross-folder,
  subject reuse, mailing lists, digests), every fixture groups as its
  expectation says, with no subject-only grouping past 7 days or of list mail.
- **SC-006**: A reply arriving while reading never changes what is shown, in
  every storyboard that delivers one.
- **SC-007**: Screens 01–11 compared on both platforms with every difference
  listed and either fixed or justified.
- **SC-008**: #2099's reproduction ("find a message with a thread and try to
  see a message in it") passes on the Mac and on Linux.

## Out of scope

- Export as PDF (`⌘P` in the design's menu): Postio has no print path.
- Redesigning the classic GTK conversation reader and the terminal app (D4).
- Server-side threading (Gmail `X-GM-THRID`, JMAP `threadId`) beyond the hint
  the model already treats it as.
- Threads spanning accounts.
- The Mac storyboard runner (as in spec 009 FR-063).

## Assumptions

- The demo store gains the design's threads (Harbor API draft v3 and the rest,
  with made-up names on reserved domains) so every screen is reproducible.
- "Own new text" uses the quote detection `postio-body` already applies to
  fold quotes; a message it cannot split is shown whole.
- "First names" come from the display name, falling back to the address's
  local part; "you" is any of the account's identities.
- Mute and split are local and do not sync between machines; their effects
  (archived replies) reach the server as ordinary actions.
- Checked against `docs/keybindings.md` on 2026-11-11: `alt+v`, `{`, `}`,
  `g u`, `mod+shift+q` and `mod+shift+Down` are free; `alt+t` and `m` are
  bound only on surfaces this does not use (search results' filter, the
  account list, contacts). The registry's clash tests are the final word.
