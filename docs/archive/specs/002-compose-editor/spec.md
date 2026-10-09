# Feature Specification: The Compose Editor

**Feature Branch**: `002-compose-editor`

**Created**: 2026-09-10

**Status**: Draft

**Input**: User description: "compose editor. A mail editor that is: keyboard friendly, can write markdown, works for replies, can do rich text editing, works for attachments"

Added in follow-up: "we should count for replies to emails with html on them and how it looks"; "we need to make sure the editor is visually aligned from a style perspective to the rest of the app"; "the editor needs to support things like links and images as well as attachments"; "we need to make sure the editor supports to, cc, bcc and that it captures the elements of a thread in an email. also needs to support signatures"

## Context

**This specifies the whole compose editor, not only the parts that are
missing.** Most of it already exists, and the table below says which — but
that is *implementation status, not scope*. A requirement here is a
requirement whether it is satisfied today or not: it is what the editor must
do, what a test may be written against, and what a regression would violate.
Writing only the gaps would leave the built behaviour undefined, which is how
it gets changed by accident.

Three things are genuinely absent — markdown input, an editor that looks like
the rest of the application, and a reply to rich HTML that is legible rather
than accidental — and they are marked as such below rather than given a
section of their own.

| Asked for | State today |
|---|---|
| Opening, and where the composer lives | Built in part. `c` composes, `e`/`E`/`f` reply and forward; the composer takes over the reading pane and can be detached. **One-in-the-pane, many-detached is a decision of 2026-09-10 and is new** |
| Sending, scheduling, saving, discarding | Built. Send is undoable for a grace period; schedule-send, save-draft, discard and mark-as-sent are all bound |
| Identity selection | Built. A picker on the draft, defaulting to the account's identity |
| Rich text editing | Built. ADR 0003 chose true WYSIWYG over a restricted HTML subset; bold, italic, bulleted and numbered lists, links and quote blocks are in the command registry |
| Works for attachments | Built. `attach_file` is bound, the toolbar offers it (#1197), and attachments round-trip including inline `cid:` images |
| Works for replies | Built. `postio-body` owns quoting and reply construction (`replying.rs`, `quote.rs`) |
| Keyboard friendly | Built in structure. Every composer command is in the registry with a binding, per Constitution II |
| To, Cc and Bcc | Built in the model — a draft carries all three |
| Threading a reply | Built. The model computes `In-Reply-To` and `References` from the parent's chain |
| Signatures | Built in part. A signature is applied on send-as and placement relative to the quote is already a config setting. **Leaving the body alone on an identity change is a decision of 2026-09-10 and reverses the current replace-on-change behaviour.** HTML signatures are deferred |
| Links, and images inside the body | Built. A link command exists, and inline images are minted as `cid:` parts |
| **Quoting a rich HTML original** | Built, but **reduced** — carrying the sender's own HTML into the quote is a change of direction, see below |
| **The editor looking like Postio** | **Not built** — see below |
| **Can write markdown** | **Not built, and explicitly rejected** — see below |

So the value here is not a new editor. It is: a definition the existing one can
be measured against, the gaps named, and a decision made about markdown.

### Two of these are worth knowing before planning

**A reply must quote a rich HTML message as it looked — and that is a change
of direction.** Today the quote is rebuilt from the parsed document rather than
from the sender's markup, so anything outside the supported subset "has no
representation rather than being stripped on the way out". That phrasing is
`postio-body`'s own and it is a deliberate security property: a reply re-emits
quoted content into the world, and a closed type cannot carry a script or a
tracking pixel forward because neither has a representation in it.

The maintainer decided on 2026-09-10 that fidelity wins: reply to a newsletter
or a templated message and the quote should look as it did. **This supersedes
the reply-construction property of ADR 0003 and ADR 0004 and needs an ADR of
its own** — a spec cannot amend an accepted decision, and without that record
the next person to read ADR 0004 will revert this as a defect.

What makes it tractable is that the machinery exists and the safety rule does
not have to move: FR-045 already says a reply may re-emit only what Postio
would render when *reading*, which is the same sanitiser, the same refused
declarations, and the same style scoping that stops a sender's CSS reaching
Postio's own chrome. The change is which input the quote is built from, not
what is permitted in it.

**The editing surface carries no stylesheet at all.** The reader wraps a
message body in a generated sheet and a themed ground colour; the editor's
document is a bare `contenteditable` body with a security policy and nothing
else. So it renders in the engine's defaults — the wrong typeface, the wrong
size, a white page in dark mode — while every pixel around it uses the app's
own tokens. That is why "visually aligned" is a user story here (Story 4) and
not a styling footnote.

### Markdown was a decision, not a gap — and it has been taken

ADR 0003 (Accepted, 2026-08-24) records a **product decision taken by the
maintainer** to reject a Markdown-authored composer in favour of true WYSIWYG,
and lists the costs that decision accepts. "Markdown-authored, HTML generated"
is named in its Alternatives section as the rejected option.

"Can write markdown" has two readings with very different consequences, and
**the maintainer chose the first (2026-09-10)**:

- **Markdown as input shortcuts inside the WYSIWYG editor** — typing
  `**bold**`, `# `, or `- ` produces formatting, and the document remains the
  HTML subset. This *complements* ADR 0003 and contradicts nothing. **Chosen.**
- **Markdown as the draft's canonical form** — what the user types is the
  plaintext part and HTML is rendered from it. This is precisely ADR 0003's
  rejected alternative, and adopting it would have superseded an accepted ADR.
  **Not chosen.**

So ADR 0003 and ADR 0004 stand unamended, `postio-body` keeps the document and
the sanitiser, and markdown is a way of *typing* what the editor could already
express. That bound is what makes it small: a markdown sequence is only
supported when it maps to formatting the subset already has, so this adds no
new document structure, no second plaintext path, and no mode.

## Clarifications

### Session 2026-09-10

- Q: When someone replies to a message built with rich HTML, what should survive into the quoted text? → A: C — keep the sender's HTML, sanitised, so the quote looks as it did
- Q: How many composers should be able to exist at once? → A: D — the reading pane holds at most one draft; any other open draft lives in a detached window
- Q: If the identity changes after the signature was hand-edited, is the edit kept or replaced? → A: C — changing identity never touches the body; the signature is the user's to change
- Q: Do inline images count against the same size limit as attachments? → A: A — one total for both, and the refusal names the largest items
- Q: With recipients only in Bcc, what does To show? → A: A — the conventional `undisclosed-recipients:;` empty group

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Write and send a message without touching the mouse (Priority: P1)

A person presses `c`, types a recipient, moves to the subject, writes a body
with some emphasis and a list in it, attaches a file, and sends — hands never
leaving the keyboard, and never guessing which key does what.

**Why this priority**: it is the whole feature in one gesture, and it is the
promise Constitution II makes. Every other story is a refinement of it.

**Independent Test**: drive the composer from `c` to sent using only key
events, asserting on what is on screen at each step. Delivers a usable
composer on its own.

**Acceptance Scenarios**:

1. **Given** the message list has focus, **When** the user presses `c`, **Then** a composer opens with the keyboard in the first recipient field.
2. **Given** the composer is open, **When** the user moves between recipient, subject and body using the keyboard alone, **Then** focus lands in each in a defined order and the focused field is visibly marked.
3. **Given** the composer is open with Cc and Bcc not shown, **When** the user reveals them by keyboard, **Then** they appear and take focus without the draft or the caret being disturbed.
3. **Given** the body has focus, **When** the user applies emphasis and a list with their bindings, **Then** the formatting appears as it will be sent.
4. **Given** a complete message, **When** the user presses the send binding, **Then** the composer closes, the message appears in Sent, and the send is undoable for the grace period.
5. **Given** any composer command, **When** the user opens the command palette or the `?` sheet, **Then** that command and its current binding are listed there.

---

### User Story 2 - Reply with the original quoted correctly (Priority: P1)

A person replies to a message in a thread. The quoted original is present,
attributed, and folded out of the way; the cursor starts where they will type;
recipients are filled from the message being answered.

**Why this priority**: replying is the most frequent compose gesture in a mail
client, and a reply that quotes badly is visible to everyone who receives it.

**Independent Test**: reply to a corpus message and assert on the resulting
draft's recipients, quoted block and cursor position, without sending.

**Acceptance Scenarios**:

1. **Given** a message is being read, **When** the user replies, **Then** the recipients are those of a reply to that message and the subject carries the conventional prefix exactly once.
2. **Given** a reply has been opened, **When** it appears, **Then** the original is quoted with an attribution line and is folded, and the cursor is above it.
3. **Given** a reply to a message inside a thread, **When** it is opened, **Then** it answers the message that was focused, not the newest in the thread.
4. **Given** a reply-to-all, **When** it is opened, **Then** the sender's own addresses and aliases are absent from the recipients.
5. **Given** a reply to a message that was sent as rich HTML — a newsletter, a message with tables, a corporate template — **When** the reply opens, **Then** the quoted original appears as it did when read, and the user's own text is visually separate from it.
6. **Given** such a reply, **When** it is sent, **Then** the recipient sees the original as the sender built it, carrying nothing Postio would refuse to render.
7. **Given** a reply is sent, **When** the recipient's client shows it, **Then** it appears inside the original conversation rather than as a new one — and the same is true of Postio's own list.
8. **Given** an identity with a signature, **When** a reply opens, **Then** the signature is present exactly once, in the configured position relative to the quote.

---

### User Story 3 - Links, images and attached files (Priority: P2)

A person puts a link in a sentence, drops a screenshot into the body where it
belongs, and attaches a document alongside the message. Three different things
with three different outcomes for the recipient, and the composer makes which
is which obvious.

They also see what they attached and how large it is, remove one they did not
mean to add, and are warned before sending a message that mentions an
attachment without having one.

**Why this priority**: attachments are how a compose surface loses a person's
work or embarrasses them. It is high value but the message is still sendable
without it, which is what makes it P2 rather than P1.

**Independent Test**: add and remove files on a draft and assert the draft's
attachment list and the visible rows, without sending.

**Acceptance Scenarios**:

1. **Given** the composer is open, **When** the user attaches a file by keyboard, by the toolbar, or by dropping it on the composer, **Then** a row appears naming the file and its size.
2. **Given** text is selected, **When** the user inserts a link, **Then** the selected text carries it, and the link's target is visible before it is committed.
3. **Given** the body has focus, **When** the user drops or pastes an image into it, **Then** the image appears at that point in the message and travels inside the body for the recipient, not as a separate file to open.
4. **Given** a message with both an inline image and an attached file, **When** it is sent, **Then** the recipient sees the image in the body and the file listed as an attachment.
2. **Given** an attached file, **When** the user removes it, **Then** the row goes and the draft no longer carries it.
3. **Given** a draft whose text mentions attaching something, **When** the user sends with nothing attached, **Then** they are asked before it goes.
4. **Given** attachments and inline images totalling more than the account's send limit, **When** the user tries to send, **Then** they are told which limit, by how much, and which items are largest, before anything is queued.

---

### User Story 4 - The editor looks like the rest of the application (Priority: P2)

A person moves from reading a message to writing one and the text does not
change typeface, size, colour or background under them. In dark mode the
composer is dark. The editor reads as part of Postio rather than as a web page
embedded in it.

**Why this priority**: the editing surface is not a widget the theme reaches —
it is a document, and it currently carries no stylesheet at all, so it renders
in the engine's defaults while everything around it uses the app's own. The
reader already solves this for message bodies; the editor is the one surface
that does not.

**Independent Test**: open the composer in each colour scheme and compare the
editing surface's typeface, size, foreground and background against the
reader's and the app's, with no message involved.

**Acceptance Scenarios**:

1. **Given** the application in light mode, **When** the composer opens, **Then** the text being typed uses the same typeface, size and colour the reader uses for a message body.
2. **Given** the application in dark mode, **When** the composer opens, **Then** the editing surface is dark, with no white frame before or after it draws.
3. **Given** the colour scheme changes while a draft is open, **When** it changes, **Then** the editing surface follows without the draft being reloaded or the caret moving.
4. **Given** a quoted original inside a reply, **When** it is shown, **Then** it is visually distinguishable from what the user is typing, in the same way the reader distinguishes a quote.

---

### User Story 5 - Never lose a draft (Priority: P2)

A person writes half a message, navigates away, closes the window, or the
application stops unexpectedly, and finds the text again.

**Why this priority**: losing typed text is the single worst thing a compose
surface can do, and recovery is cheap.

**Independent Test**: type into a composer, trigger each way of leaving it, and
assert the text is recoverable.

**Acceptance Scenarios**:

1. **Given** unsent text, **When** the user navigates away from the composer, **Then** the draft is saved without being asked.
2. **Given** a saved draft, **When** the user opens it from Drafts, **Then** the text, formatting, recipients and attachments are as they were left.
3. **Given** unsent text, **When** the user discards deliberately, **Then** they are asked first, because discarding is not undoable.

---

### User Story 6 - Markdown while typing (Priority: P3)

A person who thinks in markdown types `**bold**`, `# `, or `- ` and gets
formatting, without leaving the editor or changing a mode.

**Why this priority**: it is the one new capability, and it is a convenience on
top of an editor that already works. Everything it produces is reachable
without it, so it must not block the four stories above.

**Independent Test**: type each supported sequence and assert the resulting
document structure.

**Acceptance Scenarios**:

1. **Given** the body has focus, **When** the user types a supported markdown sequence, **Then** the corresponding formatting is applied and the literal markers are not in the sent message.
2. **Given** an unintended conversion, **When** the user undoes once, **Then** the literal text they typed is restored rather than the formatting reapplied.
3. **Given** text that resembles markdown but is not meant as it, **When** the user sends, **Then** what they see in the editor is what is sent.

### Edge Cases

- What happens when a reply's original contains formatting the editor's subset does not support — is it preserved in the quote, flattened, or dropped, and is the user told?
- What happens when a detached composer is closed while its draft is still queued to send, or the application quits with several detached windows open?
- What happens when a send fails after the composer has closed — where does the person find the message, and is the text still editable?
- What happens when an attachment is removed from disk between attaching and sending?
- What happens when a recipient is typed but not committed and the user sends — is the half-typed address included, dropped, or refused?
- What happens to formatting when a draft is edited in a plain-text-only path, given ADR 0003 records this round trip as lossy?
- What happens when the account has no identity to send as?
- What happens to the literal markers in the plain-text alternative when a message was written with markdown shortcuts — are they absent, or doubled by a plaintext rendering that re-adds them?
- What happens when a user pastes a block of markdown rather than typing it?
- What happens when the original being replied to is HTML with no usable plain-text alternative, and its structure is mostly layout — is there a quote at all?
- What happens when a reply quotes a message that itself quoted several earlier messages, each with its own formatting?
- What happens when the account's send limit is unknown — no server has told Postio one yet, or the account was just added?
- What happens when a link's visible text and its target disagree, which is the shape of a phishing link being quoted forward?
- What happens to an inline image when the draft is reopened later, or edited on the other frontend?
- What happens when a Bcc-only message is later found in Sent — does it still show who it went to, given the sent copy is the user's own record?
- What happens when a reply-to-all would include a mailing list as well as every individual on it?
- What happens when the message being replied to has no threading information, or has a chain long enough to matter?
- What does the composer show when the selected identity and the signature in the body disagree, given FR-030 leaves that to the user to resolve?
- What happens when an identity has no signature at all?
- How does the composer behave at the narrow breakpoint, where the reading pane it occupies is smaller?

## Requirements *(mandatory)*

### Functional Requirements

**Keyboard and discoverability**

- **FR-001**: Every composer action MUST be reachable by keyboard, and MUST appear in the command palette and the `?` sheet with its current binding.
- **FR-002**: Every composer action MUST also be reachable by mouse, and none MUST require it.
- **FR-003**: Focus MUST move between recipient, subject, body and attachment controls in a defined, reversible order, and the focused control MUST be visibly marked.
- **FR-004**: Single-character bindings MUST NOT fire while the user is typing into a text field.
- **FR-005**: Every composer binding MUST be overridable by command id from configuration.

**Opening a composer, and where it lives**

- **FR-006**: Users MUST be able to start a new message, a reply, a reply-to-all and a forward, each by keyboard, palette entry and visible control.
- **FR-007**: The composer MUST open with the keyboard already in the field the user will type in first — recipients for a new message, the body for a reply.
- **FR-008**: The composer MUST take over the reading pane by default; the message list MUST keep its scroll position and its cursor while it does.
- **FR-009**: Users MUST be able to detach the composer into a window of its own, as a deliberate action and never as the default.
- **FR-010**: The reading pane MUST hold at most one draft at a time. Any other draft that is open MUST be in a detached window of its own.
- **FR-011**: Starting another message while the pane holds one MUST move the pane's draft into a detached window rather than refusing, discarding it, or asking — the new draft then takes the pane.
- **FR-012**: Leaving the composer MUST restore whatever the reading pane was showing before it, without reloading it.
- **FR-013**: A draft MUST be editable in exactly one place at a time, so the same draft can never be open in the pane and a window at once; asking for one that is already open MUST bring that surface forward instead of opening a second.
- **FR-014**: Every open composer MUST make clear which draft it is showing, and a detached window MUST be identifiable without being focused.

**Identity**

- **FR-015**: A draft MUST send as one of the account's identities, defaulting to the account's own, and the user MUST be able to change it from the composer.
- **FR-016**: Changing the identity MUST update what the recipient will see as the sender. It MUST NOT alter the body, per FR-030.

**Subject**

- **FR-017**: A draft MUST carry a subject the user can edit, and a reply or forward MUST prefill it with the conventional prefix applied exactly once however many times the thread has been replied to.
- **FR-018**: Sending with an empty subject MUST ask first rather than being refused or sent silently.

**Recipients**

- **FR-019**: The composer MUST support To, Cc and Bcc, each holding several addresses, each reachable and editable by keyboard alone.
- **FR-020**: Cc and Bcc MUST be revealable and hideable without disturbing the draft, the caret, or addresses already entered; hiding a field that holds addresses MUST NOT silently drop them.
- **FR-021**: A Bcc recipient MUST NOT be disclosed to any other recipient of the message.
- **FR-022**: A message addressed only in Bcc MUST be sendable, and MUST go out with the conventional empty group `undisclosed-recipients:;` in To — never with a real address, never with To absent.
- **FR-023**: The composer MUST show, before sending, how many recipients a message has and on which field, so a reply-to-all to a large list is not a surprise.
- **FR-024**: An address that is malformed or incomplete MUST be reported before the message is queued, naming the address and the field it is in.
- **FR-025**: Recipients MUST be completable from the address book, and completion MUST be operable by keyboard.

**Threading**

- **FR-026**: A reply MUST carry the threading of the message it answers, so that it appears inside the same conversation in the recipient's client.
- **FR-027**: A reply MUST appear in the correct conversation in Postio's own list once sent, without waiting for it to come back from the server.
- **FR-028**: Threading MUST be preserved across saving and reopening a draft, and across a send that failed and was retried.
- **FR-029**: A message composed fresh MUST start its own conversation, and a forward MUST NOT be threaded into the conversation it came from.

**Signatures**

- **FR-030**: A draft MUST open carrying the signature of the identity it starts as, inserted without the user typing it.
- **FR-031**: Changing the identity on an open draft MUST NOT alter the body in any way, including the signature already in it. The signature is the user's to change, and a draft whose identity changed MAY therefore carry the previous identity's sign-off until they do.
- **FR-032**: No action MUST ever produce two signatures in one draft. Under FR-030 nothing is inserted after the draft opens, so reopening a saved draft MUST NOT add another either.
- **FR-033**: A signature on a reply or a forward MUST be placed according to the configured position relative to the quote.
- **FR-034**: The user MUST be able to edit or delete the signature in a draft, and that edit MUST survive saving and reopening.

**Editing**

- **FR-035**: Users MUST be able to apply and remove emphasis, bulleted and numbered lists, links and quote blocks, and see the result as it will be sent.
- **FR-036**: The editor MUST show what will be sent — no formatting may appear only on receipt, and none may be silently dropped between editing and sending.
- **FR-037**: Every editing action MUST be undoable, and undo MUST restore what the user typed rather than an intermediate state.
- **FR-038**: Pasting formatted content MUST reduce it to the supported set rather than carrying arbitrary markup into the message.
- **FR-039**: A plain-text alternative MUST accompany every formatted message, and MUST be readable on its own.

**Replies**

- **FR-040**: A reply MUST be addressed from the message being answered, and a reply-to-all MUST exclude the sender's own addresses and aliases.
- **FR-041**: A reply MUST quote the original with an attribution line, folded by default, with the cursor placed for typing.
- **FR-042**: Inside a thread, a reply MUST answer the focused message.
- **FR-043**: A forward MUST carry the original's attachments and inline images.
- **FR-044**: A reply to a message sent as rich HTML MUST quote it as it appeared when read — the original's structure and styling MUST be carried into the quote, not rebuilt from a reduced form of it.
- **FR-045**: The quote MUST be built from the same sanitised rendering the reader shows. Where the original has no renderable HTML, the quote MUST fall back to its text alternative rather than being empty.
- **FR-046**: The user MUST be able to tell, before sending, what the recipient will see as the quote; the quoted block in the editor MUST be what goes out.
- **FR-047**: A reply's quote MUST be sanitised with remote images **blocked**, whatever the user was allowed to see while reading the original. Allowing a sender's images is a decision about the reader's own privacy; carrying it into a reply would hand the recipient a tracker on the strength of somebody else's decision (ADR 0033 Q2).
- **FR-048**: A reply MUST NOT re-emit anything from the original that Postio would not render when reading it — no remote-loading content, no tracking pixels, no script — regardless of how the original was built. This is what "sanitised" means in FR-042 and FR-043, and it is the rule that does not move.

**Links, images and attachments**

- **FR-049**: The composer MUST offer three distinct outcomes and MUST make clear which is which: a **link** on text, an **image inside the body**, and a **file attached alongside** the message.
- **FR-050**: Users MUST be able to insert, edit and remove a link on selected text, and MUST be able to see its target before committing it.
- **FR-051**: Users MUST be able to place an image in the body by keyboard, by a visible control, and by dropping or pasting it, and it MUST appear to the recipient at that point in the message rather than as a separate file.
- **FR-052**: An image placed in the body MUST still be a real part of the sent message, so a recipient whose client shows no remote content still sees it.

- **FR-053**: Users MUST be able to attach files by keyboard, by a visible control, and by dropping them on the composer.
- **FR-054**: Each attachment MUST be listed with its name and size, and MUST be removable.
- **FR-055**: Inline images and attached files MUST count against one size total, because both travel as parts of the same message and the receiving server counts both.
- **FR-056**: The composer MUST refuse, before queueing anything, to send a message whose total exceeds the account's limit, and MUST say which limit, by how much, and which items are largest — including an inline image where that is the offender.
- **FR-057**: The composer MUST ask before sending a message whose text mentions an attachment when none is present.

**Drafts, sending and scheduling**

- **FR-058**: Users MUST be able to send, schedule a send for later, save as a draft, discard, and mark a message as sent, each by keyboard, palette entry and visible control.
- **FR-059**: A send MUST be reversible for a grace period, during which the message has not left; after it, the message MUST be findable in Sent.
- **FR-060**: A scheduled send MUST state when it will go, MUST be cancellable before then, and MUST be visible as pending rather than as an ordinary draft.
- **FR-061**: Discarding MUST ask first, because it is the one composer action that cannot be undone.
- **FR-062**: The composer MUST refuse to send a message with no recipients, naming what is missing.

- **FR-063**: Unsent work MUST be saved without the user asking, and MUST survive navigating away, closing the window, and the application stopping unexpectedly.
- **FR-064**: Reopening a draft MUST restore its text, formatting, recipients and attachments.
- **FR-065**: Sending MUST be local-first — the composer MUST NOT await the network, and a send with no connection MUST queue and say so.
- **FR-066**: A message that fails to send MUST remain editable and MUST be findable, and the failure MUST name what went wrong.

**Markdown**

- **FR-067**: Markdown MUST be an input method inside the editor, not the draft's stored form: typing a supported sequence MUST apply formatting, and the document MUST remain the same one the toolbar and the formatting bindings produce.
- **FR-068**: A markdown sequence MUST be supported only where it maps to formatting the editor already offers as a command. Markdown MUST NOT introduce structure the editor cannot otherwise produce.
- **FR-069**: A conversion MUST happen as the user types, and the literal markers MUST NOT appear in the sent message.
- **FR-070**: A markdown conversion MUST be reversible by a single undo, restoring the literal characters typed and leaving them unconverted.
- **FR-071**: Text that resembles markdown but was not converted MUST be sent as the user sees it.
- **FR-072**: The plain-text alternative MUST stay readable for a message written this way, per FR-037 — a conversion MUST NOT leave doubled or stray markers in it.

**Appearance**

- **FR-073**: The editing surface MUST use the application's own typeface, text size, foreground and background, matching what the reader uses for a message body.
- **FR-074**: The editing surface MUST follow the active colour scheme, including dark mode, and MUST NOT show a light frame before or after it draws.
- **FR-075**: A colour-scheme change while a draft is open MUST be reflected without reloading the draft, losing the caret, or losing undo history.
- **FR-076**: A quoted original MUST be visually distinguishable from the text the user is writing, consistent with how a quote is drawn when reading.
- **FR-077**: The editor MUST honour the application's text-size and density settings, and MUST remain usable at the narrow breakpoint.
- **FR-078**: Styling carried by a quoted original MUST be confined to that quote: it MUST NOT alter the appearance of the text the user is writing, of an earlier quote nested inside it, or of Postio's own chrome. Under FR-042 the quote carries the sender's own styling, which makes this the requirement that keeps it contained.

**Privacy**

- **FR-079**: The composer MUST NOT make a network request that the user did not ask for — no remote content fetched while editing, and no address or body text leaving the machine before send.

### Key Entities

- **Draft**: an unsent message — recipients, subject, body in both formatted and plain forms, attachments, the identity it sends as, and its state (being edited, queued, failed). Survives restarts.
- **Attachment**: a file bound to a draft — name, size, type, and whether it is inline content referenced from the body or a separate part.
- **Reply context**: what a draft is answering — the message, its thread, the quoted original and its attribution.
- **Identity**: the address and signature a draft sends as, one of possibly several on an account.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A person can go from the message list to a sent reply using only the keyboard, with no step requiring a pointer.
- **SC-002**: Every composer action is listed with its binding in both the palette and the `?` sheet — measured as zero actions missing from either.
- **SC-003**: Typing in the editor stays within the interaction budget, so no keystroke is perceptibly late.
- **SC-004**: No draft is lost in normal operation: after navigating away, closing the window, or an unexpected stop, the text is recoverable in 100% of cases.
- **SC-005**: A reply's recipients and quoted original are correct for every message in the test corpus, including replies inside threads.
- **SC-006**: What the sender sees in the editor matches what a recipient receives, for every construct the editor can produce.
- **SC-007**: A message is never silently sent without an attachment it claims to have, and never silently rejected for size — both are surfaced before anything is queued, and an oversize message names what is making it large.
- **SC-008**: Sending never waits on the network: the composer closes immediately whether or not a connection exists.
- **SC-009**: Every supported markdown sequence produces formatting the user could also have reached from a command, and no sequence produces anything else — measured as zero constructs reachable only by typing markdown.
- **SC-010**: A message written with markdown shortcuts is indistinguishable, as received, from the same message formatted with the toolbar.
- **SC-011**: A Bcc recipient is never visible to another recipient — zero disclosures across the test corpus, including messages addressed only in Bcc.
- **SC-012**: Replies land in the right conversation for every message in the test corpus, in the recipient's client and in Postio's own list.
- **SC-013**: A signature appears exactly once in a sent message, for every combination of identity change, save-and-reopen, reply and forward.
- **SC-014**: The editing surface matches the reader's body text on typeface, size, foreground and background, in both colour schemes.
- **SC-015**: A reply to any HTML message in the test corpus quotes it as the reader shows it, and carries nothing the reader would refuse — zero scripts, zero remote-loading content, zero tracking pixels re-emitted, across the whole corpus.

## Assumptions

- The existing composer is the starting point, not a rewrite. ADR 0003 (WYSIWYG over a restricted HTML subset) and ADR 0004 (`postio-body` owns the document and the sanitiser) hold for *authoring*: the markdown decision of 2026-09-10 is an input method and changes neither.
- **They do not hold for the quote.** FR-042's decision — that a reply carries the sender's sanitised HTML rather than a rebuilt reduction of it — supersedes the reply-construction property both ADRs rest on, and requires a new ADR before it is built. This specification records the decision; it does not amend the ADRs, and must not be read as having done so.
- The set of supported markdown sequences follows the editor's existing formatting commands rather than any particular markdown dialect. Sequences with no counterpart in the subset — tables, footnotes, embedded HTML — are out of scope by construction, not by omission.
- "Keyboard friendly" means Constitution II's standard — one registry entry per command, with binding, palette entry and accessible control derived from it — rather than a new set of shortcuts.
- "Already built" is never a reason a requirement is absent here. Scheduled send, mark-as-sent, the detached window, identity selection and the subject line are specified above because they are part of the editor, not because they are missing.
- Identity selection is existing behaviour. Signatures are in scope for how the editor presents and preserves them, not for a new signature format: HTML signatures are recorded as deferred by the model, and this feature does not change that.
- Threading, recipient computation and the wire headers a reply needs already exist in the model. This feature specifies what the editor must honour and show, not a new threading mechanism.
- The address book is an existing surface; completion here consumes it rather than defining it.
- Genuinely out of scope, and excluded on purpose rather than by omission: spelling and grammar checking, message templates, and any AI assistance (deferred to epic E12). Contact management is out of scope; the composer consumes the address book rather than editing it.
- Encryption and signing of outgoing mail are out of scope for this feature.
- Attachment size limits come from the account's own configuration rather than a constant in the code, per Constitution VII (providers are data, not code).
