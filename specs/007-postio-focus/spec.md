# Feature Specification: Postio Focus

**Feature Branch**: `feature/postio-focus`

**Created**: 2026-09-26

**Status**: Clarified and planned ([plan.md](./plan.md)); built on
`feature/postio-focus`. Nothing lands on `main` until the maintainer says so.

**Input**: The maintainer's handoff (`Design/postio-focus-design/PROMPT.md`):
a dense, keyboard-first inbox on the Postio engine that shows mail as it
arrived, calls out real actions, holds some mail back into digests on a
cadence the user chooses, and hides spam and updates, built for
**performance and consistency first** and from **reusable components**.

## Why this exists

Postio promises speed, search and the keyboard (`docs/PRODUCT.md` §1). For
someone with a lot of mail, most of it is noise (spam, promotions,
notifications) or matters only now and then (newsletters, statements, school
updates). A layout that shows every message, every day, with a reading pane
that pulls you into mail you did not mean to read, does not serve them.

Focus is Postio's desktop app ([ADR 0043](../../docs/decisions/0043-focus-is-the-one-desktop-app.md),
C27). It has no folder sidebar. Home is a dense inbox, driven from the
keyboard, that shows mail **as it arrived**: sender, subject and first line,
verbatim, with no rewritten subjects or scores. A message opens over the list
and closes back to the same place, or, for those who choose it, beside the
list (FR-038). Focus does four things to mail and nothing else:

1. It **calls out real actions**: an invitation, a direct question, a to-do.
2. It **holds some mail back into digests** on a cadence the user chooses.
   When the user brings a local model, each digest opens on a summary written
   on this machine, and every statement in it cites the mail it came from.
3. It **hides spam and automated updates**. Each one keeps its reason and is
   one key from being restored.
4. It **links mail to the user's Obsidian vault**.

The argument for each of these, and what peers taught, is in the product
brief (`Design/focus-product-brief.md`). This spec does not repeat it.

Everything Postio already promises still holds: instant, offline, undoable,
keyboard-first and private. Focus uses the same store, sync, command registry
and budgets as every Postio app (constitution I–VII).

Focus is a frontend on the engine in the terminal's shape
(`specs/005-tui-frontend`): it runs `postio-host` in its own process and
reaches mail only through `postio-client` (ADR 0041). Two things set it
apart:

- **The GTK it draws outside its own window lives in `postio-widgets`**: the
  message view, the composer, the settings window, and the small widgets
  that make up Postio's visual language (keycaps, key hints, chips, action
  bars, pickers). That crate depends on no app (ADR 0043).
- **Focus has engine work of its own**: invitations, filtering, digests,
  reminders, a simple built-in detector for mail that needs action, an
  Obsidian writer and, when the user brings one, a local model for
  classification and digest summaries. All of it is local.

The maintainer ranked the priorities in this order: **performance**, then
**consistency**, then **reusable components**.

## Clarifications

The maintainer's decisions on the handoff's open questions and the ones this
spec raised (2026-09-26 unless dated):

- **Landing.** "Don't land anything until I say so. Let's keep it in a
  branch." The milestones order the work on `feature/postio-focus`; none of
  them is a landing.
- **Focus's rules act only while Focus runs.** Filtering, digest holding and
  reminders act on mail while Focus has the store. Mail another app files
  lands in the inbox as usual, and Focus sorts it when it next opens, even if
  the user saw it in the other app meanwhile.
- **A digest opens on its summary**, as drawn in 01, 22 and 23: the row
  carries the opening of the summary, the digest opens on a summary the local
  model writes on this machine with numbered references to its messages, and
  `Tab` switches to the plain list. FR-172 to FR-175 keep the summary honest.
- **The open message shows one message at a time**, as drawn in 04: the
  latest by default, `[`/`]` through the thread, and `j`/`k` through the list
  (FR-037).
- **Accept and Decline go through the outbox**, held for about ten seconds.
  The toast's Undo or `mod+z` cancels the reply and nothing is sent. After
  that it cannot be taken back.
- **Filtered mail is archived and never deleted automatically.** Filtered
  lists all of it.
- **One keymap for every app**, the one `KEYS.md` sets out, overridable
  under `[keys]` in `config.toml` by command id.
- **The local model is the user's own and optional**: "The local model
  should be bring your own. I don't want to embed it as part of the app and
  it needs to be optional." The user runs a model runtime of their choosing
  on this machine and names it in `config.toml`. Postio embeds, bundles,
  downloads and starts nothing. Focus is complete without a model, and every
  feature that uses one stays off until the user turns it on (FR-165 to
  FR-169).
- **A built-in detector marks questions and to-dos when no model is
  connected**: "I think we can plan for a simple embedded classifier for the
  needs action question if no ai is connected." It is plain code, not a
  language model. The user's model takes over the question when it is
  connected (FR-104 to FR-108).
- **Focus is the one desktop app** (2026-10-02, C27).

## The inputs, and which one wins

| Input | What it decides |
|---|---|
| `Design/postio-focus-design/screens/NN-*.png` | How every screen looks: layout, density, hierarchy, copy, which controls exist. 01–20 are the inbox's core; 21–25 are Filtered, the digest summary and its email, the digest rule dialog, and Obsidian capture |
| `Design/postio-focus-design/SPEC.md` | What each screen does, and the GTK colour and widget mapping |
| `Design/focus-message-dialog/SPEC.md` and `screens/` | The open message's frame, column, rhythm, components and body treatments (FR-039); C25 and C26 replace its fonts and accent |
| `Design/postio-focus-design/KEYS.md` | The default key bindings, for every app |
| `Design/postio-focus-design/source/*.dc.html` | The exact spacing, sizes and copy behind the PNGs. They are not code to port |
| `Design/focus-product-brief.md` | The why, the principles, the four things Focus does to mail, and how classification works |
| `Design/postio-focus-design/PROMPT.md` | The maintainer's handoff: priorities, reuse, the open decisions |

- Every screen is specified here against its PNG, by number, and built
  against it. Every difference between the running app and its PNG is
  written down in [screens.md](./screens.md) with its reason (FR-095).
- Where `SPEC.md` or the handoff disagrees with a PNG, **the PNG wins on
  appearance**. Where the constitution, the handoff or the maintainer settles
  a *behaviour*, that wins over a PNG's copy. The table below records each
  case.
- The PNGs were rendered without the web fonts. The app uses Adwaita Sans and
  Adwaita Mono, so a comparison is about layout and hierarchy, not glyph
  metrics.

**The design folders are not in the repository.** They live, untracked, in
the maintainer's checkout: the PNGs carry a real person's first name in the
sample mail, and one source mockup (`DCompose.dc.html`) a real surname. This
branch cites them by path and commits none of them until the maintainer has
re-rendered and scrubbed them (constitution VI).

### Where the inputs disagree

Each row is the decision as it stands. Tasks, code and `screens.md` cite the
ids.

| # | Screens | Decision |
|---|---|---|
| C1 | 04, 23 | The body is drawn from the message's HTML by the reading renderer, sanitised with remote images blocked (FR-033), not from the plain-text part the PNGs show; `v` shows the raw source. Much mail is HTML only |
| C2 | 04 | One message at a time, the latest by default, `[`/`]` through the thread, as 04 draws it, rather than `SPEC.md`'s stack of earlier messages. A deliberate departure from ADR 0032 (FR-037) |
| C3 | 20 | Keys are rebound under `[keys]` in `config.toml` (constitution II); there is no `keys.toml`. The key map's footer says so |
| C4 | 21 | Filtered mail is archived and never deleted automatically; 21's header says "Nothing here is deleted automatically" where the PNG says "Kept for 30 days" |
| C5 | 01–03, 15–19 | A digest row's first line is the opening of its summary when there is one: the one place the list shows text a model wrote (FR-011, FR-124). Otherwise it is the digest's senders |
| C6 | 22, 23 | A digest opens on its summary when the user has brought a model, and on its plain list otherwise (FR-125, FR-172 to FR-175) |
| C7 | 05, 06 | The composer is Postio's one composer, in Focus's frame. There is no Markdown toggle; the subtitle says what will be sent ("Plain text · N words") |
| C8 | 01, 03, 17–19 | Question and To-do markers come from a built-in detector when no model is connected (FR-104 to FR-108), so they show as drawn without a model |
| C9 | 01, 03, 04, 25 | Task `t`, Note `n` and "Task in <project> · due <day>" appear only when a vault is configured (`[focus.vault]`) |
| C10 | 01, 10, 16 | A header count shows only while its feature is in use: "N filtered today" while filtering is on, "N digest rules" while there are rules |
| C11 | 09 | Every command the bar lists shows its key (constitution II). The mockup's "Archive everything read, older than a week" is not a Postio command |
| C12 | `KEYS.md`, 15 | Delete is on the `Delete` key in the one keymap (FR-081). It moves mail to Trash and is undoable |
| C13 | `KEYS.md` | **Maintainer (2026-10-02):** Focus offers Flag and Unflag on `*`, in the row menu and the command bar; rows carry no flag mark; `g *` lists flagged mail. Flags other clients set can be cleared here |
| C14 | 11 | Snooze and Send later share one preset table (ADR 0029). Both say "Later today", screen 11's wording, and after 6pm "Tomorrow evening" |
| C15 | — | The digest rules list (`g d`) uses 21's full-view frame; the digest window's message list uses 22's dialog frame, with rows as in 08 |
| C16 | `README.md` | Compare layout, not glyph metrics: the PNGs were rendered without the web fonts |
| C17 | the brief | Focus's engine work is local, with no new protocol, backend or sync mode. The one new outgoing message is an RSVP, through the existing outbox |
| C18 | 01 | The strip says "186 filtered today", the PNG's wording, not `SPEC.md`'s "186 filtered (g f)" |
| C19 | `KEYS.md` | `X` selects every conversation in the current view as a predicate (constitution V), not just the rows on screen |
| C20 | `KEYS.md` | One keymap for every app, `KEYS.md`'s, with the keys of commands only another app has moved out of its way (FR-081, [contracts/keymap.md](./contracts/keymap.md)) |
| C21 | 25, the brief | A task line's `postio://` link goes before the date: `- [ ] … [✉](postio://message/…) 📅 2026-09-30`. The Obsidian Tasks plugin reads its fields from the end of the line |
| C22 | 01–20 | Key caps are generated from the registry and spelled by the shared hint code: the keymap's spelling (`J`, `X`, `ctrl+k`), shortened in a tight cap to `Del`, `↵` and `⇧` (`hints::short`). A literal glyph in a string is refused by `check-key-hints-are-derived.py`. Each screen's comparison records the notation |
| C23 | 05 | Recipient suggestions open at four characters (`postio_ui::recipients::MIN_COMPLETION_PREFIX`), where 05 draws three. Whether it becomes three is a `/ux-architect` call |
| C24 | 09 | **Maintainer (2026-09-29):** `mod+k` opens the command bar in command mode with `>` typed, and `/` opens it for mail search. The bar opens in place, in the top bar's field, with its results below |
| C25 | message dialog `SPEC.md` §2, §4, §5 | **Maintainer (2026-10-01): the system font for the chrome.** The chrome is Adwaita Sans and Adwaita Mono at the handoff's sizes, weights and gaps; no Barlow, Barlow Condensed or IBM Plex Mono in it (FR-093). A body drawn in app colours is set in Barlow (FR-039) |
| C26 | message dialog `SPEC.md` §5, §6 | **Maintainer (2026-10-01): the system accent everywhere.** The open message follows the GNOME accent like the rest of Focus: the action card's fill is libadwaita's `--accent-color` at 8% (light) and 12% (dark), and links and the card's tag are that accent. The handoff's surface, ink, hairline and scrim values stand |
| C27 | — | **Maintainer (2026-10-02): Focus is the one desktop app**, named "Postio" (binary `postio`, app id `dev.postio.Postio`) once the package switches (T253); until then it builds as `postio-focus` with app id `dev.postio.Postio.Focus`. The classic three-pane app (`postio-gtk`, the `postio` binary in `postio-app`) is retired rather than kept as a mode: it keeps building until it is removed and gets no new work ([ADR 0043](../../docs/decisions/0043-focus-is-the-one-desktop-app.md)). What it did and where Focus does it is [`classic-parity.md`](classic-parity.md). Removal waits for the maintainer |

## Milestones, and landing

**Nothing lands on `main` until the maintainer says so** (Clarifications).
The work stays on `feature/postio-focus` and is rebased onto `main` as it
goes, and every commit keeps the terminal and macOS green. The milestones
group the work:

- **Milestone 1**: screens 01–21 and 24, and the differentiators that need
  no model: invite markers from the message's calendar part, Question and
  To-do markers from the built-in detector, filtering by headers and
  structure with the Filtered view, and digest rules by sender, each digest
  opening on its plain list.
- **Milestone 2, the optional local model the user brings**: the model
  answering the needs-action question in place of the built-in detector,
  digest summaries (22, 23), and digests by mailing list, by search, and
  "more like this".
- **Milestone 3**: Obsidian (25) and `postio://` links.

Without a model or a vault, the surfaces they feed keep their place empty
rather than drawing something else there: a row keeps its marker line and
the chip's place, a digest row and window keep the summary's, Filtered's
reasons have room for a model's, digest rules have room for list, search and
"more like this" matches, and the needs-action question has one detector
seam the model answers through.

**Before the branch can land** it needs two things only the maintainer can
give:

- **The constitution's one-desktop-app wording.** Version 1.3.0, on this
  branch, names Focus and allows its optional, user-supplied local model; its
  Scope still says two desktop apps. The change lands with the branch, with
  the maintainer's approval (FR-009).
- **The design folders**, re-rendered and scrubbed, so the screens can be
  committed.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Work the inbox as it arrived (Priority: P1)

Screens 01, 02, 03 and 15.

The user opens Focus. Within half a second they see their inbox, newest
first, one row per conversation, grouped under day headings ("Today ·
Saturday 26 September"). Each row shows:

- the sender, subject and first line of the newest message, exactly as sent;
- unread conversations in bold;
- up to two label pills after the subject;
- an attachment mark, the conversation's message count, and the time.

A row carrying an action marker grows a second line and a dot in the accent
colour. Digest rows (User Story 10) sit among the conversations. The header
strip reads "Inbox ▾" with the conversation and unread counts, and a "Has
action · 7" toggle.

The user moves with `j`/`k`, selects with `x`, extends with `J`/`K`,
selects everything in the view with `X`, and clears with `Esc`. While
anything is selected, a bulk bar at the bottom shows:

- the count;
- Archive `a`, Snooze `s`, Mark read `r`, Digest these… `d`, Label `l`, Move
  `m`, Delete;
- the selection keys.

A right-click on a row offers its verbs with their keys. Selecting a row
never opens it. An archive leaves the list at once, and a toast says what
happened ("Archived 3 messages · Undo"). `mod+z` undoes it, even after the
toast has gone. `!` narrows the inbox to rows with a marker. Light and dark
follow the system.

**Why this priority**: This is the product. Every other story is reached
from this list.

**Independent Test**: Start Focus against a store holding a fixture mailbox,
with the network absent. Drive it with keystrokes, and assert on the rendered
widget tree (what a person sees, not what a layer was handed):

- rows show the sender, subject and first line verbatim;
- unread rows are bold, and at most two labels show;
- the cursor and the selection are distinct, and the bulk bar acts on the
  selection;
- after each action, the store holds what the same command writes from any
  app.

**Acceptance Scenarios**:

1. **Given** a synced store and no network, **When** Focus starts, **Then**
   the inbox is drawn and navigable within the startup budget, without
   waiting on any server.
2. **Given** a message whose subject is "RE: Q3 numbers!!" and whose text
   begins "Hi all —", **When** it is listed, **Then** its conversation row
   shows exactly those strings: nothing rewritten, summarised or scored.
3. **Given** three rows selected and the cursor on a fourth, **When** the
   user presses `a`, **Then** the three are archived and the fourth is not;
   the toast says "Archived 3 messages"; and one `mod+z` returns all three.
4. **Given** the cursor on a row, **When** the user presses `j` or `k`,
   **Then** only the cursor moves: nothing opens and nothing is marked read.
5. **Given** rows with and without markers, **When** the user presses `!`,
   **Then**:
   - only rows with a marker remain;
   - the toggle carries the count;
   - the strip says "Showing 7 of 312 · ! again to show all";
   - the selection is cleared;
   - the cursor stays on the same message if it is still shown.

   Pressing `!` again restores the full inbox.
6. **Given** the system switches between light and dark, **When** Focus is
   open, **Then** it follows at once. The accent appears only on action
   markers, the focus ring and the has-action toggle.
7. **Given** an inbox of 100,000 conversations, **When** the user scrolls
   from top to bottom, or presses `X` and archives, **Then** only the
   visible window of rows is ever read. The selection is a predicate, and
   each step redraws within the interaction budget.
8. **Given** a conversation with three labels, **When** it is listed,
   **Then** two label pills show, and neither is drawn in the accent hue.

---

### User Story 2 - Open a message, and come back to the same place (Priority: P1)

Screen 04, and the message dialog's handoff.

`Enter` opens the conversation under the cursor in a dialog over the list,
the way compose opens. The list stays in place behind it, dimmed.

- **The header** has the step controls (`k`/`j`, each with its key inside),
  the subject over its position ("Message 5 of 312 · thread of 6"), and the
  close button. The steps go to the previous or next message in the list
  without closing.
- **The action row** holds every action with its key: Reply `e`, Reply all
  `E`, Forward `f`, Archive `a`, Snooze `s`, Remind `h`, Label `l`, Move `m`
  and Delete. In a narrow dialog, Label, Move and Delete fold into More `.`.
- **The dialog shows one message of the conversation**, the latest by
  default. It says "Latest of 6 in this thread", and `[` and `]` step to the
  older and newer messages (FR-037).
- **One centred column**, in order:
  - the subject, its labels and "+ Label";
  - the sender block: From with name and address, To, Cc, and the date;
  - an action card with its action, when the message has a marker, with the
    triggering sentence highlighted in the body;
  - a quiet line naming the body's treatment, for an HTML body;
  - the body, as the reading renderer draws it;
  - attachments as chips;
  - quoted text folded, with its line count ("31 quoted lines").

`v` shows the raw source and `O` switches the body between app colours and
the original. `Esc` closes the dialog and returns to the same row, with the
selection kept. A message left open for a second is marked read.

**Why this priority**: Reading is most of what mail is for. Focus's promise
is that opening a message is deliberate and coming back costs nothing.

**Independent Test**: Open messages from the fixture mailbox by keystroke,
and assert what the dialog shows:

- the header fields and the action card;
- the folded-quote count and the attachment chips;
- stepping with `j`/`k` and with `[`/`]`.

Close the dialog with `Esc`, and assert that the cursor row and the
selection are unchanged. Count that no new message view is built after the
first open.

**Acceptance Scenarios**:

1. **Given** the cursor on a row and two other rows selected, **When** the
   user presses `Enter` and then `Esc`, **Then** the dialog opens over the
   list and closes. The cursor is on the same row, and the same two rows are
   still selected.
2. **Given** the dialog open on message 5 of 312, **When** the user presses
   `j`, **Then** message 6 is shown without closing, and the list's cursor
   behind the dialog moves with it.
3. **Given** a thread of six opened on its latest message, **When** the user
   presses `[`, **Then** the previous message in the thread is shown and the
   position line says so. `]` steps back.
4. **Given** an HTML-only message with remote images, **When** it is opened,
   **Then** its body is drawn from its HTML, sanitised, with remote images
   blocked until allowed once or always for that sender. No script runs and
   no network request is made.
5. **Given** any message, **When** the user presses `v`, **Then** its raw
   source is shown.
6. **Given** a message with a marker, **When** it is opened, **Then** the
   action card sits under the sender block with its action and key. For a
   quoted marker, the triggering sentence is highlighted in the body where it
   appears.
7. **Given** the hundredth open in a session, **When** it happens, **Then**
   the dialog opens in one frame, reusing the one message view. Memory does
   not grow with the number of opens.
8. **Given** a quoted reply history, **When** the message is shown, **Then**
   the history is folded with its line count and expands on request.
9. **Given** attachments or links, **When** the user presses `o`, **Then**
   they are offered to open or save, and nothing opens without a deliberate
   choice. A link's full target is visible first.

---

### User Story 3 - Write and reply with Postio's composer (Priority: P1)

Screens 05 and 06.

`c` composes. `e`, `E` and `f` reply, reply to all and forward, from the list
or from the open message. The composer opens in the open message's frame, in
a dialog over the list or in the pane beside it, and may be detached to a
window of its own. It is Postio's one composer, with its rich text,
attachments, identities, signatures, drafts, send later and outbox:

- **Header**: Detach; the title ("New message", "Reply", "Reply to all",
  "Forward") over what will be sent and what has happened to it ("Plain text
  · 58 words · Draft saved locally 16:12"); close.
- **Action row**: Send (`mod+Return`), Send later ▾, Attach, and Remind
  (`mod+h`, "Remind · Tue 29 Sep" once a day is chosen).
- **One column** below it: From (the identity), To, Cc and Bcc on demand,
  Subject, Labels; the formatting toolbar; the body; its attachments.

Recipients complete from the address book and from past mail, ranked by how
often the user has written to each address. No dropdown appears until the
user types in a field.

A reply to all is pre-filled:

- recipients from the thread;
- a "Re:" subject;
- the thread's labels, marked "from the thread";
- the quoted text, folded under the draft.

Labels set before sending are applied to the sent message's conversation.
`Esc` closes the composer and keeps the draft, which reopens from Drafts.

**Why this priority**: A mail client that cannot reply is a reader.

**Independent Test**: Compose and reply through keystrokes against the mock
backend, and assert:

- the outgoing bytes are what the shared composer produces for the same
  content;
- chosen labels land on the sent conversation;
- a remind-if-no-reply set in the composer exists after the send.

**Acceptance Scenarios**:

1. **Given** a message open, **When** the user presses `E`, **Then** the
   composer opens with every recipient from the thread, a "Re:" subject, the
   thread's labels and the quote folded. `Esc` returns to where the user was.
2. **Given** the same content written in Focus and in any other Postio app,
   **When** both are sent, **Then** the messages that leave are identical.
3. **Given** a draft in progress, **When** the user presses `Esc`, **Then**
   the draft is saved locally, and it opens again from Drafts in any app.
4. **Given** labels chosen before sending, **When** the message is sent,
   **Then** its conversation carries them.
5. **Given** "Remind if no reply · Tue 29 Sep" set in the composer, **When**
   the message is sent and nobody replies by then, **Then** the conversation
   returns to the top of the inbox marked "No reply since <date>" (see
   User Story 5).
6. **Given** the user types "grac" in To, **When** suggestions appear,
   **Then** they come from the address book and from past mail, ranked by how
   often the user wrote to each. Choosing one adds a chip with the name and
   the address.
7. **Given** no network, **When** the user sends, **Then** the message is in
   the Outbox at once, and it leaves at most once when the link returns.

---

### User Story 4 - Search, go to and run commands from one bar (Priority: P1)

Screens 07, 08, 09 and 10.

`/` or `mod+k` opens one bar for search, commands and going places.

- **Saved searches** are pinned across its top, with their counts and
  `alt+1`–`alt+4`. `mod+s` saves the current query.
- **Plain English** ("the invoice Ada sent last month") is lowered, on this
  machine, into editable operator chips of Postio's one query language. The
  bar shows the words the user typed and names the chip being edited. `Tab`
  steps into the chips, and `mod+BackSpace` returns to the plain words.
- **Results** are one line each: sender, subject, first line, where the
  conversation lives (`in:Inbox`, `in:Receipts`) and the date. `Enter` opens
  one. A misspelt query offers "Search instead for …" (ADR 0037), and `O`
  switches the results between relevance and date.
- **Folders**: `in:` completes folder names and lists a folder's
  conversations, newest first (08). There is no folder sidebar.
- **Commands and places**: typing a word also lists:
  - the commands it matches, each with its key;
  - places to go, with their counts and keys;
  - a plain search row ("Search mail for 'arch'").

  They are filtered together as the user types, and `>` limits the list to
  commands. A command acts on the row or the selection that was focused when
  the bar opened.

`g o`, or a click on "Inbox ▾", opens a popover listing:

- mailboxes (Inbox, Drafts, Sent, Archive, Snoozed, Flagged, Filtered, and
  the Outbox while it holds anything), each with its direct key;
- folders and labels, with counts.

Typing filters the popover, and `Enter` goes to the chosen place.

**Why this priority**: Focus has no sidebar. Search, go-to and the command
bar are how the user moves (brief, principle 5).

**Independent Test**:

- Type queries from the search corpus character by character, and assert
  the results and the chips.
- Lower a table of plain-English phrases, and assert the expected chip
  sets.
- Enumerate the command rows, and assert every one shows the key the keymap
  resolves.
- Assert that a command acts on the selection held before the bar opened.

**Acceptance Scenarios**:

1. **Given** a synced store, **When** the user types "the invoice Ada sent
   last month", **Then** the chips shown are operators of the one query
   language that together mean "from Ada, about an invoice, received last
   month". Running the chips returns what typing those operators by hand
   returns.
2. **Given** half-typed input such as `is:` or `after:2026-`, **When** it is
   shown, **Then** no error appears and results keep updating.
3. **Given** three rows selected, **When** the user opens the bar and runs
   "Archive selection", **Then** exactly those three are archived.
4. **Given** the user types `in:Rec`, **When** Receipts is offered and
   chosen, **Then** Receipts' conversations are listed newest first, with
   the folder's count.
5. **Given** a saved search pinned at `alt+2`, **When** the user presses
   `alt+2` in the list, **Then** its results are shown.
6. **Given** the folders popover open, **When** the user types "trav" and
   presses `Enter`, **Then** Travel's conversations are shown.
7. **Given** no network, **When** the user searches, **Then** results come
   from the local index within the search budget.
8. **Given** any plain-English input, **When** it is lowered, **Then**:
   - words that map to no operator stay free text;
   - nothing leaves the machine;
   - the same input always gives the same chips.

---

### User Story 5 - Snooze, remind, label and move from a picker at the row (Priority: P1)

Screens 11, 12, 13 and 14.

`s`, `h`, `l` and `m` open a small picker anchored to the focused row. A
picker acts on the selection when there is one, and names what it acts on
("Ada Moreno · Atlas Q3 budget"). In every picker:

- number keys choose a preset;
- `Tab` jumps to a typed-date field ("tue 9am"), parsed on this machine;
- `Enter` confirms, and `Esc` closes.

- **Snooze** offers Later today, Tomorrow morning, Monday morning and Next
  week, each with its time. It says the message leaves the inbox and comes
  back at the top at that time, and that snoozed mail is under `g z`, where
  `B` wakes it.
- **Remind if no reply** offers Tomorrow, In 2 working days, End of the week
  and In a week. It says that a reply from anyone cancels the reminder, and
  otherwise the thread comes back to the top of the inbox marked "No reply
  since <date>".
- **Label** filters as the user types. It shows which labels are applied and
  each label's count, toggles a label with `Space`, and offers "Create label"
  for a name that does not exist yet.
- **Move** filters folders and lists recent destinations first (`1`, `2`).
  `Enter` moves the mail, it leaves the inbox, and `mod+z` undoes.

**Why this priority**: These are the verbs of triage. Remind if no reply is
Focus's own.

**Independent Test**: Drive each picker by keystroke, against fixture data at
a fixed clock. Assert the computed times, the store's resulting state and the
undo entry. For reminders, assert both paths: a reply from someone else
cancels it, and no reply brings the thread back with its marker text.

**Acceptance Scenarios**:

1. **Given** a Saturday at 16:09, **When** the snooze picker opens, **Then**
   it offers the four presets with their times, as screen 11 shows:
   - Later today, 18:00
   - Tomorrow morning, Sun 27 Sep 08:00
   - Monday morning, Mon 28 Sep 08:00
   - Next week, Sat 3 Oct 08:00.
2. **Given** "tue 9am" typed, **When** it is confirmed, **Then** the mail is
   snoozed to the coming Tuesday at 09:00, local time.
3. **Given** a reminder set for Tue 29 Sep 09:00, and a reply from another
   participant on Monday, **When** Tuesday 09:00 passes, **Then** nothing
   returns: the reply cancelled the reminder.
4. **Given** a reminder set and no reply by its time, **When** the time
   passes, **Then** the conversation returns to the top of the inbox marked
   "No reply since Sat 26 Sep". If Focus was not running at that moment, it
   appears when Focus next opens, and it works offline.
5. **Given** the label picker, **When** the user types a new name and
   confirms, **Then** the label is created and applied. `Space` on an applied
   label removes it, and every change is undoable.
6. **Given** three rows selected, **When** the user moves them to Receipts,
   **Then** all three leave the inbox, and one `mod+z` returns them.
7. **Given** a picker open, **When** the user presses `Esc`, **Then**
   nothing changes.

---

### User Story 6 - Always know what state Postio is in, and never be blocked by it (Priority: P1)

Screens 16, 17, 18 and 19.

An empty inbox is a quiet, centred "Inbox is empty". Under it are when the
next digest comes ("Next digest: Weekly · Newsletters, Saturday 16:00") and
shortcuts to Filtered, Archive and Compose. While the first sync has not
finished its first pass, the inbox says it is syncing and how far, never
that it is empty.

First sync, offline and a sign-in error each show as one banner under the
header strip. The header's sync label, which otherwise reads "Synced 16:09",
says the same thing in a word or two:

| State | Banner | Header label |
|---|---|---|
| First sync | "12,408 of 18,204 messages, newest first. You can read and search what's here.", with a progress bar | "Syncing 12,408 of 18,204" |
| Offline | "Everything you do is saved here and syncs when you're back.", with Retry now | "Offline" |
| Sign-in error | "Can't sign in to \<server\>. The server rejected the password for \<address\>. Mail on this computer is still available.", with Update password… | "Sync failed" |
| An account failing for another reason | The account, and the reason in the sync's own words when it gave them, with Retry now | "Sync failed" |

None of these blocks anything local, and every account that works stays
listed.

**Why this priority**: Constitution I requires that sync state is visible,
and that nothing the user does waits on it.

**Independent Test**: Drive sync-state changes through the host's test seam,
and assert the banner and the header label for each. With the network
absent, archive, label and search, and assert each takes effect locally and
is queued.

**Acceptance Scenarios**:

1. **Given** no network, **When** the user archives, labels and searches,
   **Then** each takes effect at once, and the changes queue for sync.
2. **Given** a first sync in progress, **When** the user reads and searches,
   **Then** everything that has arrived is readable and searchable, and the
   banner shows progress.
3. **Given** a rejected password, **When** the user chooses Update password…,
   **Then** the credential dialog opens, and mail stays available meanwhile.
4. **Given** an empty inbox, **When** it is shown, **Then** it names only
   what exists: the next digest if there are digests, and the filtered count
   if filtering is on.

---

### User Story 7 - One key map, taught everywhere (Priority: P1)

Screen 20.

Every action has one key, and the app teaches it:

- every button carries its keycap;
- every command-bar row shows its key;
- `?` opens the key map.

The key map is grouped: Move and select, Open, Act, Invites, Go and find, In
search, Digests and filtering, and Obsidian. `?` or `Esc` closes it. Its
footer says where to rebind a key, and that every key has a visible button.

Postio has one default keymap, the one `KEYS.md` sets out, shared by every
app (Clarifications). A key means one thing everywhere:

- undo is `mod+z` only;
- `D` stops digesting a sender;
- `U` unsubscribes;
- `R` restores from Filtered.

All of it comes from the one command registry. Any binding can be changed in
`config.toml` under `[keys]`, by command id, and Settings' Keyboard section
rebinds it there.

**Why this priority**: Constitution II. A command that is not in the registry
does not exist, and a key the app does not teach is a key nobody finds.

**Independent Test**: Enumerate the registry for every app. Assert that every
command reachable in Focus has a key, a command-bar row and a visible
button, and that no default key means two different commands in any app.
Then override a key, and assert that the key map, the command bar and the
button's keycap all show the key the keymap resolves.

**Acceptance Scenarios**:

1. **Given** `[keys]` overriding archive, **When** Focus starts, or the
   config reloads while it runs, **Then** the new key works. The key map,
   the command bar and the Archive button's keycap all show it.
2. **Given** a new command registered for Focus, **When** Focus is built,
   **Then** it appears in the key map and the command bar with no
   Focus-specific code.
3. **Given** the one default keymap, **When** it is enumerated across every
   app, **Then** no key is bound to two commands in one context, and each
   command has the same key in every app that has it.
4. **Given** the key map open, **When** the user presses `?` or `Esc`,
   **Then** it closes.
5. **Given** the terminal, **When** it starts, **Then** its defaults are the
   one keymap's (`s` snoozes, `ctrl+z` undoes), and every command it offers
   has a key.

---

### User Story 8 - Answer an invitation from the row (Priority: P2)

Markers on 01, 03 and 04.

A message carrying a calendar invitation gets a marker, with no model
involved. The marker shows:

- an "Invite" chip;
- the date and time from the invitation ("Tue 29 Sep · 10:00–10:45"), in the
  user's time zone;
- Accept (`y`) and Decline (`Y`), on the row and in the open message.

Pressing `y` queues an acceptance to the organiser through the outbox,
local-first. For about ten seconds the reply can be cancelled, from the undo
toast ("Accepted · Undo") or with `mod+z`. After that it cannot be taken
back. The row then shows what was answered.

An updated invitation replaces the marker's time. A cancelled one says so
and offers no Accept. Invitations count toward "Has action".

**Why this priority**: The first differentiator, and the only one decided
entirely by the message's own structure. It is also the one that sends mail,
so its safety has to be right before anything else sends on the user's
behalf.

**Independent Test**: File corpus invitations through the filing path and
assert the markers: requests, updates, cancellations, recurring events and
time zones. Then RSVP against the mock backend, and assert:

- nothing reaches the transport before the window ends;
- cancelling within the window sends nothing;
- after the window, exactly one reply leaves.

**Acceptance Scenarios**:

1. **Given** a message with a calendar request part, **When** it is filed,
   **Then** its row shows Invite, the event's date and time in the user's
   time zone, and Accept `y` / Decline `Y`. The marker is computed when the
   message is filed, not when the row is drawn.
2. **Given** `y` pressed, **When** the user cancels within the ten seconds,
   **Then** nothing is sent, and the invitation is as it was.
3. **Given** `y` pressed and the window passed, **When** the outbox drains,
   **Then** exactly one reply goes to the organiser. Offline, it leaves when
   the link returns.
4. **Given** a later cancellation of the same event, **When** it is filed,
   **Then** the marker says the event was cancelled, and Accept and Decline
   are gone.
5. **Given** an invitation whose event has already ended, **When** it is
   listed, **Then** the marker shows the event without Accept or Decline.

---

### User Story 9 - Spam and updates filtered, each with its reason, none lost (Priority: P2)

Screen 21, and the counts on 01, 10 and 16.

While Focus runs, spam, promotions and automated updates (notifications,
receipts, shipping, social) are archived automatically as they are filed,
and never reach its inbox. Mail another app filed while Focus was not
running is sorted when Focus next opens (Clarifications). This is the one
place Focus acts on its own, and it is bounded.

- **Guard rules win over any rule or classifier.** Mail is never filtered if
  any of these is true:
  - the user has written to the sender;
  - it comes from the user's own domain;
  - it comes from a sender the user pinned;
  - it belongs to a conversation the user took part in.
- **When in doubt, mail goes to the inbox.**
- **Decisions come from the message's own structure and headers**, and, when
  the user has brought one, the model for what those leave undecided. That
  means list and bulk headers, automated senders listed as data (never as
  code), and the server's own spam verdicts. While Focus runs, they are made
  when the message is filed, so filtered mail is never seen arriving in its
  inbox.

`g f` opens Filtered: everything hidden, newest first, each row with its
reason ("promotion", "notification · Forge"), and tabs by reason with counts
(`1`–`7`). `R` restores a message to the inbox and never filters that sender
again. That correction is remembered, and it is undoable like any other
action. The header says how many were filtered today ("186 filtered today",
`g f`). Filtered mail is archived, and never deleted automatically.

**Why this priority**: This is where the product wins or loses trust (brief:
"Risks"). Its guard rules, reasons and restore are part of the feature, not
polish on it.

**Independent Test**: Run a fixture corpus with known answers through filing.
It holds automated mail of each kind, spam, and guarded mail: from
correspondents, from the user's own domain, from a pinned sender, and in
conversations the user took part in. Assert which messages were filtered and
why, that zero guarded messages were filtered, and that restore is
remembered and undoable. Then file mail as another app would, open Focus,
and assert the catch-up sorts it the same way.

**Acceptance Scenarios**:

1. **Given** a notification from an automated sender the user has never
   written to, **When** it arrives, **Then** it goes to Filtered as
   "notification · <source>" and never appears in the inbox.
2. **Given** a promotion from an address the user has written to, **When**
   it arrives, **Then** it goes to the inbox.
3. **Given** a message in a conversation the user took part in, **When** it
   arrives, **Then** it is not filtered, whatever its headers say.
4. **Given** a filtered message, **When** the user presses `R`, **Then** it
   returns to the inbox, and its sender is never filtered again. One
   `mod+z` reverses both.
5. **Given** Filtered open, **When** the user presses the Notifications tab's
   number, **Then** only notifications are listed.
6. **Given** a message whose classification is uncertain, **When** it
   arrives, **Then** it goes to the inbox.
7. **Given** filtered mail older than 30 days, **When** Filtered is opened,
   **Then** that mail is still listed, archived. Filtered mail is never
   deleted automatically.
8. **Given** a promotion that arrived while another Postio app had the
   store, **When** Focus next opens, **Then** it moves from the inbox to
   Filtered, with its reason, as if Focus had filed it.

---

### User Story 10 - Digests on a cadence the user chooses (Priority: P2)

Screen 24, and the digest rows on 01 and 16. The digest window uses 22's
frame.

`d` on a message opens "Digest this sender", pre-filled with the sender's
address. The user chooses:

- how often: Daily, Weekly or Monthly;
- on which day, and at what time.

The dialog previews the recent mail the rule would have caught ("Would have
caught 9 messages in the last 90 days"), and Create saves the rule.

From then on, that sender's mail is **held**: it skips Focus's inbox, but it
is never hidden from search, and `g d` shows it. Mail with an invitation, a
question or a to-do still comes straight to the inbox.

When the cadence comes due, **one digest row** appears in the inbox. It
shows:

- its cadence and name ("Weekly · Newsletters");
- its count;
- the opening of its summary ("Summary of 14 messages from 6 senders: …")
  when there is one, or its senders.

`Enter` opens it as a window over the inbox: on its summary when there is
one (User Story 13), and `Tab` switches to the plain list; on the plain list
otherwise. From either, the user can:

- archive the whole digest (`A`);
- open one message;
- change the rule and its cadence (`d`);
- stop digesting a sender (`D`);
- unsubscribe (`U`), only on that deliberate key.

"Digest these…" in the bulk bar makes one rule for the senders of the
selection. `g d` lists every rule, with its cadence, its next delivery and
what it holds now. "Match a list or a search instead…" is User Story 14.

**Why this priority**: Mail the user wants weekly should not arrive daily.
This is the differentiator that most reduces inbox volume without a model.

**Independent Test**: At a fixed clock, create a sender rule against a
fixture mailbox. Assert:

- the preview count equals running the rule over the last 90 days;
- new mail from the sender skips the inbox;
- after the clock passes the due time, exactly one digest row appears with
  the held messages;
- archiving the whole digest is one undoable action.

**Acceptance Scenarios**:

1. **Given** a weekly rule for a sender, due Sunday 09:00, **When** their
   mail arrives on Wednesday, **Then** it does not appear in the inbox. On
   Sunday at 09:00, one digest row appears holding it.
2. **Given** the rule's sender sends an invitation, **When** it arrives,
   **Then** it comes to the inbox with its marker.
3. **Given** Focus not running through a due time, **When** Focus next
   opens, **Then** the due digest row is there: nothing held is lost or
   duplicated.
4. **Given** a digest open, **When** the user presses `A`, **Then** all its
   messages are archived, and one `mod+z` restores them.
5. **Given** `D` on a message in a digest, **When** it is confirmed,
   **Then** that sender stops being digested, and their future mail goes to
   the inbox.
6. **Given** a due time with nothing held, **When** it passes, **Then** no
   row appears.
7. **Given** held mail, **When** the user searches for it, **Then** it is
   found, and results say where it is waiting.

---

### User Story 11 - One store, any Postio app (Priority: P1)

The user reads mail in the terminal, closes it, opens Focus, and finds the
same mailbox:

- the same folders;
- the archive they just made;
- the draft they left;
- the same keys, because every app shares one default keymap.

If another Postio app has the store open, Focus says so in the sentence the
other apps use, and does not open the store; "Try again" opens it once the
other app has closed. The reverse holds. Digest rules, filter decisions,
corrections and reminders made in Focus live in the store and configuration
that every app shares. They act only while Focus runs.

**Why this priority**: ADR 0041 requires it. Without it, Focus would be a
mail client with its own copy of the mailbox.

**Independent Test**: Open the store in one app's process and start Focus:
it refuses with the sentence and leaves the store unchanged. Close the first
app and open Focus, and assert Focus presents what the first app wrote. Do
the reverse.

**Acceptance Scenarios**:

1. **Given** the terminal open, **When** Focus starts, **Then** it says
   "Postio is already open in another window. Close it to open Postio here."
   and does not touch the store. The reverse holds.
2. **Given** a message archived in Focus, **When** Focus is closed and the
   terminal opened, **Then** the message is in the archive, not the inbox.
   The reverse holds.
3. **Given** a draft left in either app, **When** the other is opened,
   **Then** the draft is in Drafts and opens for editing.
4. **Given** filtering on and the terminal open, **When** a promotion
   arrives, **Then** the terminal shows it in its inbox, because Focus's
   rules act only while Focus runs. When Focus next opens, the promotion
   moves to Filtered.

---

### User Story 12 - Questions and to-dos called out, quoted verbatim (Priority: P2)

When a message sent directly to the user asks them a question, or asks them
to do something, its row gets a marker: "Question", or "To-do" with a due
date if the sentence names one. The marker quotes the triggering sentence
**verbatim**, never paraphrased, and the open message highlights it where it
appears. The marker's action is Reply `e` for a question, and Snooze `s` for
a to-do, with Task `t` beside it when a vault is configured. The user can
dismiss a wrong marker (`-`), and the dismissal is remembered as a
correction.

Two detectors can answer this "needs action" question, and one answers at a
time (Clarifications):

- **Built in.** When no model is connected, a simple detector built into
  Postio decides. It is plain code over the message's own text and headers,
  runs inside Postio, and needs no model runtime. It is conservative: when
  unsure, it marks nothing.
- **The user's model.** When the user has connected a model (FR-165), the
  model decides instead. It returns only a fixed schema: a category, spans
  as character offsets into the body, and a due date. It never returns text
  of its own.

Either way, the quote is a span of the message's own text, so it is verbatim
by construction.

**Why this priority**: The screens draw these markers in the core inbox, and
the built-in detector needs no model. A marker that is often wrong is worse
than none (brief: "Risks"), so the built-in detector is held to a precision
bar (SC-013) and errs toward silence.

**Independent Test**: Run a labelled fixture corpus through classification,
first with no model connected, then with a fake model that returns canned
spans. The corpus holds:

- questions and to-dos sent directly to the user;
- the same sentences in quoted history, signatures, bulk mail, and mail the
  user is only copied on;
- instruction-shaped text meant to hijack a model.

Assert:

- every marker quotes a byte-exact span of the newest message's own text;
- the built-in detector meets SC-013's precision on the corpus;
- markers are computed off the UI path;
- nothing was sent, and no request was made beyond the model's own when one
  is connected.

**Acceptance Scenarios**:

1. **Given** no model connected, and a message sent directly to the user
   containing "Can you approve these by Friday so finance can close the
   quarter?", **When** it is classified, **Then** its row shows Question,
   with that sentence exactly as written.
2. **Given** no model connected, and "Please leave comments by Wednesday;
   I'd like to freeze it Thursday." sent directly to the user on Saturday 26
   September, **When** it is classified, **Then** its row shows To-do, due
   Wed 30 Sep, quoting "Please leave comments by Wednesday".
3. **Given** the same question in quoted history, in a signature, in a
   newsletter, or in mail the user is only copied on, **When** it is
   classified, **Then** no marker appears.
4. **Given** a body containing instructions addressed to an assistant,
   **When** it is classified, **Then** nothing is sent, no command runs, and
   no network request is made, beyond the local model's own when one is
   connected.
5. **Given** a marker dismissed as wrong, **When** the message is classified
   again, **Then** the marker does not return. **When** the same sender
   sends a similar message, **Then** the dismissal is taken into account
   (brief, layer 2).
6. **Given** a model connected, **When** mail arrives, **Then** its markers
   come from the model. **When** the model is not running, **Then** the
   built-in detector answers instead, and nothing waits for the model.
7. **Given** no model configured, **When** mail arrives, **Then** markers
   still appear from the built-in detector, and Postio opens no connection
   to any model runtime.

---

### User Story 13 - Read a digest as a summary that cites its mail (Priority: P3)

Screens 22 and 23, and the digest row on 01.

When the user has brought a local model, `Enter` on a digest row opens the
digest window on its summary. The summary is one reading column, grouped by
topic ("Your town · The Evening Ledger · 5"). Each statement ends in a
numbered reference to the message it came from.

- `]` and `[` move between references. The focused reference shows its
  message ("Local-First Weekly · Issue 112: Sync without servers"), with
  "open the full email".
- `Enter` opens that email in the same window, never a second one:
  - the header reads "Summary" with `Esc`;
  - a banner says where the email was cited ("Cited as 6 in the summary; the
    passage is highlighted");
  - the cited passage is highlighted;
  - `j`/`k` step through the digest's sources;
  - `Esc` returns to the summary, at the same reference.
- `Tab` switches to the plain list of the messages.
- `A` archives the whole digest, `d` edits the rule and cadence, `D` stops
  digesting the referenced sender, and `U` unsubscribes, only on that key.

The footer says what wrote the summary: "Written on this computer by the
local model from these 14 messages only. Every statement links to the email
it came from."

**Why this priority**: It needs the local model. It is the one place Focus
shows text a model wrote, so it carries its own guarantees (FR-172 to
FR-175).

**Independent Test**: Open a digest with a fake local model that returns
canned statements and references, and assert:

- every statement shows a reference;
- every reference opens its message with the cited passage highlighted,
  byte-exact;
- a statement returned without a valid reference is not shown;
- markup or links in the model's output render as plain characters;
- instruction-shaped text in the digest's mail produced no action and no
  request.

**Acceptance Scenarios**:

1. **Given** a due digest and a reachable local model, **When** the user
   opens it, **Then** it opens on the summary, and every statement ends in a
   numbered reference.
2. **Given** reference 6 focused, **When** the user presses `Enter`,
   **Then** its email opens in the same window, with the cited passage
   highlighted and a banner naming the reference. `Esc` returns to the
   summary at reference 6.
3. **Given** model output containing `<a href=…>` or a bare URL, **When** it
   is shown, **Then** it appears as plain characters, never as a link or as
   markup.
4. **Given** no local model configured, or the configured one not running,
   **When** the digest opens, **Then** it opens on its plain list, and the
   row shows its senders instead of a summary line.
5. **Given** a digest whose mail contains instructions addressed to an
   assistant, **When** the summary is written, **Then** nothing is sent, no
   command runs, and no request leaves the machine.
6. **Given** the summary open, **When** the user presses `Tab`, **Then** the
   plain list of the digest's messages is shown. `Tab` again returns to the
   summary.

---

### User Story 14 - Digests by list, by search, or like this one (Priority: P3)

"Match a list or a search instead…" (24) makes a digest rule from a mailing
list or from a query in the one language, previewed on recent mail before it
is saved. "Digest mail like this" makes a rule from a selected message, and
the local model, when the user has brought one, checks which mail is alike.
Without a model, "Digest mail like this" is absent, and list and search rules
work either way.

**Why this priority**: "Like this" needs the model; list and search rules
widen what sender rules already do.

**Independent Test**: Create each kind of rule against a fixture mailbox,
and assert that the preview equals what the rule later holds.

**Acceptance Scenarios**:

1. **Given** a rule `list:weekly.example.org`, **When** mail from that list
   arrives, **Then** it is held for the digest.
2. **Given** a query rule, **When** its preview is shown, **Then** it lists
   what the query matches over the recent window. The query language is the
   same one search uses (ADR 0008).

---

### User Story 15 - Capture tasks and notes into Obsidian (Priority: P3)

Screen 25.

With a vault configured, `t` on a message opens a capture sheet for a task,
and `n` for a note:

- The task text is the action sentence, verbatim. `alt+s` swaps in the
  subject.
- The due date comes from the mail when the mail gives one, with quick picks
  beside it.
- A project is suggested from the vault, with its reason, and `mod+p`
  changes it.
- The preview is the exact markdown line, in the Obsidian Tasks format, with
  a `postio://` link back to the message placed before the date, where the
  Tasks plugin still reads the date (C21).

`mod+Return` appends the line to the vault on this machine. The row then
shows "Task in <project> · due <day>". When the task is ticked in Obsidian,
Postio offers to archive the conversation. Opening the `postio://` link opens
the conversation in Postio.

**Why this priority**: The last of the four things Focus does to mail, and
the only one outside the mailbox.

**Independent Test**: Against a temporary vault directory:

- capture a task, and assert the exact bytes appended and that the file is
  otherwise unchanged;
- tick it on disk, and assert the archive offer;
- resolve the `postio://` link, and assert which conversation opens.

**Acceptance Scenarios**:

1. **Given** a to-do marker, **When** the user captures it, **Then** exactly
   one line, `- [ ] <sentence> [✉](postio://message/<id>) 📅 <date>`, is
   appended to the chosen note. Nothing else in the vault changes, and the
   Tasks plugin reads the line's due date.
2. **Given** a `postio://` link, **When** it is opened, **Then** the
   conversation opens in Postio. Nothing is fetched, and a link Postio cannot
   resolve is refused with a message.

---

### Edge Cases

- **Another app has the store.** Focus says so in the shared sentence and
  leaves the store untouched (User Story 11).
- **A store from another build.** A store whose schema a recorded migration
  reaches is migrated in place. Otherwise Focus refuses it with a way
  forward: "Start a fresh store" sets the old one aside and keeps the
  accounts and `config.toml` (`docs/notes/2026-10-01-store-migrations-and-starting-over.md`).
- **A huge inbox.** 100,000 or more conversations scroll, select all and
  archive without loading the mailbox into memory. A bulk action on a
  predicate is one undo unit.
- **A conversation split across decisions.** The list shows one row per
  conversation. A conversation the user took part in is never filtered or
  held. A new message in a held or filtered conversation that a guard covers
  (a reply to the user, say) brings the conversation to the inbox.
- **Mail another app filed.** Focus's rules act only while Focus runs
  (Clarifications). Mail another app filed lands in the inbox there. When
  Focus next opens, it sorts that mail in the background, newest first, so a
  message the user saw in another app's inbox can then move to Filtered or
  into a digest. Rows the catch-up moves leave the way an archive does. It
  never moves the row under the cursor or a message that is open.
- **Mail already in the inbox while Focus runs.** Nothing moves a message out
  of Focus's inbox after the user has seen it there, except the user's own
  actions.
  - Filtering and holding are decided when mail is filed.
  - Applying filtering to mail that was in the inbox before filtering was
    turned on is a deliberate, previewed, undoable command.
- **A late decision.** A model's decision made after a message was listed
  may add a marker. It never moves the message under the cursor, the message
  that is open, or one the user has acted on.
- **The user's model is configured but not running.** Focus works as it
  does without a model, and says so once where a model-backed feature would
  appear. It does not keep retrying. When the model is back, recent mail it
  missed can gain markers, but nothing already listed moves.
- **A body not yet on this machine.** Question and To-do markers need the
  message's text, so they appear once the body is local. Postio never
  fetches a body just to classify it.
- **Mail in a language the built-in detector does not read.** It marks
  nothing, rather than guess (Assumptions).
- **Reminders.**
  - A reply from someone other than the user cancels the reminder. The
    user's own later message does not.
  - A reminder that falls due while Focus is not running takes effect when
    Focus next opens.
  - Offline, reminders still fall due, because they are local.
- **Invitations.**
  - Updates replace the marker's time, and cancellations remove its actions.
  - An RSVP made offline waits in the outbox, and its ten-second window runs
    from the keypress.
  - A malformed calendar part yields no marker, never an error the user has
    to dismiss.
- **Digests.**
  - A digest whose due time passed several times while Focus was not running
    arrives as one row, holding everything since the last delivery.
  - A due digest holding nothing makes no row.
  - Deleting a rule releases what it held into the inbox.
  - A summary statement without a valid reference is not shown. A digest
    whose summary cannot be written opens on its plain list.
- **Time.**
  - Typed dates ("tue 9am") and presets are computed in the user's local
    zone.
  - A time that daylight saving makes ambiguous or skips resolves to a real
    instant, never a crash.
  - A digest due at 16:00 is due at 16:00 local time after a change of
    zone.
- **`mod+z` while typing.** In a text field or the composer's body, `mod+z`
  undoes typing. Everywhere else it undoes the last action.
- **Labels.** A label with no colour gets a stable one, never the accent's
  hue. More than two labels show as two pills, and the open message shows
  them all.
- **Hostile content.** The following are data, never markup or
  instructions:
  - a subject, sender name or first line containing control characters,
    bidirectional overrides, or anything shaped like markup;
  - a quoted marker sentence;
  - a digest row's senders;
  - a model's summary.
- **Long text.** At narrower widths, the first line and then the labels
  give way before the sender, subject and time. Nothing wraps into a third
  line, and row heights never change.
- **No first line.** An image-only or empty message shows no first line,
  rather than a placeholder that looks like content.
- **Multiple accounts.** There is one inbox across all accounts. Folders and
  labels are grouped by account in the popover and the pickers when there is
  more than one account (Assumptions).
- **Sign-in and offline states overlap.** One banner shows at a time,
  choosing the one that asks something of the user first: sign-in error,
  then offline, then first sync.

## Requirements *(mandatory)*

### Functional Requirements

**Shape and boundaries**

- **FR-001**: Focus MUST be Postio's desktop application, with its own
  binary, app id and launcher (C27).
- **FR-002**: Focus MUST run the engine host in its own process and MUST
  reach mail only through the client interface every frontend uses
  (`postio-host`, `postio-client`, ADR 0041).
- **FR-003**: Only one app may have the store open at a time. When Focus
  finds the store open elsewhere, it MUST say so in the sentence the other
  apps use, and MUST NOT modify the store or start sync. The reverse holds
  for the other apps.
- **FR-004**: Focus MUST open the same store, the same `config.toml` and the
  same keyring entries as the other apps, with no import, export or second
  copy. What any app wrote MUST be what Focus presents, and the reverse.
- **FR-005**: No functionality may be removed from, or degraded in, the
  terminal app or the macOS frontend. Their test suites MUST pass unchanged
  apart from default key bindings, which follow the one keymap (FR-081). A
  test that has to be weakened to pass is evidence of a regression, not of a
  refactor. The classic app is retired (ADR 0043): until it is removed it
  keeps building and its suites stay green.
- **FR-006**: Behaviour that needs no toolkit MUST be expressed once, in the
  shared toolkit-free layers, and consumed from there. This covers list
  state, selection, paging, the keymap, key hints, the command bar and
  finder, date parsing, recipient completion, the open message's geometry
  and design tokens (`postio-ui`), and commands and undo (`postio-core`).
  Focus MUST extend these layers. It MUST NOT duplicate them.
- **FR-007**: The GTK that Focus draws outside its own window MUST live in
  `postio-widgets`, which depends on no app:
  - the message view, on the reading renderer;
  - the composer;
  - the settings window and its presenters;
  - the keycap, key-hint, chip, action-bar, notice and toast widgets, and the
    pickers.

  Focus MUST NOT depend on the classic app's crates. The rule for
  `postio-widgets` (what may live there, what may not, and who depends on
  it) is ADR 0043, enforced by `scripts/checks/check-crate-boundaries.py`.
- **FR-008**: Code moving out of the classic app's crates MUST go to the
  lowest layer that can hold it, and the move MUST keep every app that still
  builds green at every step.
- **FR-009**: The documents that name Postio's frontends, boundaries and
  keys MUST describe Focus, in this branch, together with the checks that
  enforce them:
  - the constitution's Scope and Principle VII, including the optional,
    user-supplied local model and one desktop app, amendments the
    maintainer approves (see *Milestones, and landing*);
  - `docs/PRODUCT.md` §2, §8 and §23, and `docs/keybindings.md`;
  - `docs/ARCHITECTURE.md`.
- **FR-019**: Focus MUST reopen at the size and maximised state it was
  closed at, and a second launch MUST raise the window it already has.

**The list**

- **FR-010**: The inbox MUST show one row per conversation, newest first,
  grouped under day headings. Digest rows sit among the conversations.
- **FR-011**: A conversation row MUST show the sender, the subject and the
  first line of the newest message exactly as they arrived. Nothing in a
  conversation row may be a rewritten subject, a summary, a priority score or
  text Postio wrote. The one model-written text the list shows is a digest
  row's summary line (FR-124), and that line says it is a summary.
- **FR-012**: A row MUST also show:
  - unread as bold;
  - up to two label pills after the subject, never in the accent's hue;
  - an attachment mark, the conversation's message count and the time;
  - its action marker, when it has one: a dot, the marker's kind and date,
    the quoted sentence or the event time, and the marker's action with its
    key;
  - in Drafts and the Outbox, a draft's sending state (FR-056).
- **FR-013**: There MUST be exactly two row heights, one line and two lines,
  each fixed. No row's content may be measured to lay the list out. Focus,
  hover and selection change what a row draws, never its height.
- **FR-014**: The list MUST be windowed over the paged store and MUST never
  load a mailbox into memory. Focus MUST use the shared paging
  (`postio-ui`'s `ListWindow` and `Paging`), not a second implementation.
- **FR-015**: The list MUST keep a cursor and a selection that are distinct.
  Select all (`X`) MUST be a predicate over the current view, not a set of
  ids. While anything is selected, a bulk bar MUST show the count, the bulk
  actions with their keys, and the selection keys. The mouse MUST reach the
  same commands: a click moves the cursor, a double-click opens, `ctrl` and
  `shift` clicks select, and a right-click offers the row's verbs.
- **FR-016**: Selecting or moving to a row MUST NOT open it or mark it read.
- **FR-017**: `!` MUST toggle a has-action filter. The toggle carries the
  count, and the strip says how many of how many are showing. The filter
  hides plain mail and digest rows without moving them, clears the
  selection, and keeps the cursor on the same message when that message is
  still shown.
- **FR-018**: The header strip MUST show:
  - where the user is ("Inbox ▾"), opening the folders popover;
  - its conversation and unread counts;
  - the has-action toggle;
  - the filtered-today count (`g f`) while filtering is on, and the
    digest-rule count (`g d`) while there are rules.

  The top bar MUST hold compose, the command-bar field with its keys, the
  sync label, the main menu and close.

**What drawing a row may read**

- **FR-020**: Drawing a row MUST NOT read a message body. Action markers,
  digest membership, filter reasons and digest summary lines MUST be computed
  off the UI path, when mail is filed or classified or a digest is written.
  They MUST be stored where the list's own query reads them.
- **FR-021**: A quoted marker MUST be stored as character offsets into the
  body, plus a short excerpt of the sentence, so the row can show it and the
  open message can highlight it without the model or the list reading the
  body again.
- **FR-022**: Every read path this feature adds or changes MUST carry
  assertions on statements issued and rows read, through
  `postio_storage::test_support::counting`.

**Opening a message**

- **FR-030**: `Enter` MUST open the conversation over the list, in one
  frame, in the dialog or, when the user has chosen it, the pane (FR-038).
  Every open MUST reuse one message view rather than building one per open.
- **FR-031**: `Esc` MUST close the message and return to the same row, with
  the selection unchanged.
- **FR-032**: With a message open, `j`/`k` MUST step to the next and
  previous message in the list without closing it, and move the list's
  cursor to match. `[`/`]` MUST step to the older and newer message in the
  conversation. The header MUST say where the user is in both.
- **FR-033**: The body MUST be drawn by the reading renderer (ADR 0042):
  sanitised, with remote images blocked until allowed once or always for
  the sender, no script, and no network request. It MUST NOT be a
  plain-text-only view. `v` MUST show the raw source.
- **FR-034**: Quoted history MUST be folded with its line count and MUST
  expand on request. Attachments MUST be listed with name and size.
  Attachments and links MUST open only on a deliberate act, with a link's
  full target shown first. `o` MUST offer the message's links and parts to
  open, and its attachments to save one at a time or all together; an
  attachment chip opens the same chooser at its part.
- **FR-035**: A message with a marker MUST show an action card under its
  sender block, with the marker's action and key. A quoted marker MUST
  highlight its sentence in the body where it appears.
- **FR-036**: The open message's action row MUST offer every action on the
  message, each with its key: reply, reply all, forward, archive, snooze,
  remind, label, move and delete, with label, move and delete folding into
  More (`.`) when the row is too narrow. A to-do's action card MUST offer
  Task when a vault is configured, and `t` and `n` MUST open the capture
  sheet from the open message.
- **FR-037**: The open message MUST show one message at a time, the latest
  by default, as screen 04 draws it. Focus opens a message, not a thread,
  and walks the thread with `[`/`]`. This departs from ADR 0032's stacked
  conversation on purpose, and ADR 0032 records it.
- **FR-038**: The open message MUST be placeable beside the list instead of
  over it. `toggle_reading_pane` (`F8`, and a check item in the main menu)
  switches between the dialog and the pane and writes `[focus] reading`
  (`"dialog"`, the default, or `"pane"`), so the choice outlives the session.
  The pane is the same message view as the dialog, `min(820, W - 404)` wide
  beside a list of at least 404 px. A window narrower than 980 px opens the
  dialog whatever the setting says. With the pane, the list keeps the
  keyboard, and the pane follows the cursor only while a message is open;
  the composer takes the pane's place while it is open.
- **FR-039**: The open message's frame MUST follow from the window, never
  from the message: the dialog is `clamp(640, W − 2·max(96, 0.18·W), 820)`
  wide and the window's height less 80, the list behind dimmed. Everything
  inside the message shares one centred column, `min(480, dialog − 96)` for
  a body drawn in **app colours** and `min(640, dialog − 48)` for one drawn
  on **paper**. Every HTML body is sanitised and classified:
  - **paper** when, after sanitising, it paints a page background, has a
    fixed-width layout table of 480 px or more, or an image wider than
    300 px. It is drawn as sent on a white sheet, never inverted or
    recoloured, dimmed in dark mode, and zoomed to fit its column down to
    0.85 before it scrolls sideways;
  - **app colours** otherwise: the sender's colours, faces and sizes are
    removed, the body is set in Barlow in the app's ink with links in the
    accent, and a colour the sender kept survives only at 4.5:1 against the
    surface.

  A quiet line above an HTML body names its treatment, and `O` switches
  between the two, remembered for the sender when the user asks.
  `mod+plus`, `mod+minus` and `mod+0` zoom the body, kept in `[reader]
  zoom`, and `mod+f` finds in the message without moving the reading
  position.

**Acting**

- **FR-040**: Archive, delete, snooze, unsnooze, label, move, flag, mark
  read or unread, and undo MUST go through the same commands and the same
  undo stack every Postio app uses.
- **FR-041**: After archive, delete, move, snooze or label, an undo toast
  MUST say what happened ("Archived 3 messages · Undo"). `mod+z` MUST undo
  the last action, including after the toast has gone. There is no
  single-key undo.
- **FR-042**: Snooze, remind, label and move MUST be pickers anchored to the
  focused row, acting on the selection when there is one and naming what
  they act on. In every picker:
  - number keys choose a preset;
  - `Tab` reaches a typed-date field, parsed on this machine;
  - `Enter` confirms, and `Esc` closes without changing anything.

  Label MUST filter as the user types, toggle with `Space`, show which
  labels are applied, and create a label that does not exist. Move MUST
  filter, list recent destinations first, and undo with `mod+z`.
- **FR-043**: Snooze and remind presets MUST come from one shared preset
  table, so every picker offers the same times, computed the same way.
- **FR-044**: Remind if no reply MUST be set from its picker (`h`) or from
  the composer (`mod+h`). A reply from anyone but the user MUST cancel it.
  Otherwise, when it is due, the conversation MUST return to the top of the
  inbox marked "No reply since <date>".
- **FR-045**: A reminder MUST be local-first, work offline, and survive a
  restart. It MUST fall due even if Focus was not running at the time,
  taking effect when Focus next opens. It MUST be undoable when set or
  cleared.
- **FR-046**: A message that stays open for the dwell
  (`postio_ui::dwell::DWELL_TO_READ`, one second), in the dialog or the pane,
  MUST be marked read; stepping on, closing or `r` before then leaves it
  unread. `r` in the open message marks it unread again. Marking on dwell
  is kept off the undo stack.
- **FR-047**: `*` MUST flag a conversation and unflag a flagged one, from
  the key, the row menu and the command bar, with Undo (C13). Rows draw no
  flag mark, and `g *` lists flagged mail. `B` and the row menu MUST wake a
  snoozed conversation in the Snoozed list and the open message.

**Compose**

- **FR-050**: Compose, reply, reply all and forward MUST use Postio's one
  composer (rich text, attachments, identities, signatures, drafts, send
  later, the outbox) in the open message's frame: a dialog over the app, or
  the pane when a message is read beside the list, that may be detached to
  a window of its own. Focus MUST NOT have a composer of its own.
- **FR-051**: `Esc` MUST close the composer and keep the draft locally. A
  draft MUST open in any app.
- **FR-052**: Recipient completion MUST draw on the address book and past
  mail, rank by how often the user has written to each address, show that
  count, mark mailing lists, and offer nothing until the user types in a
  recipient field.
- **FR-053**: Labels set in the composer MUST be applied to the sent
  message's conversation. A reply MUST start with the thread's labels.
- **FR-054**: A reply MUST start with its recipients, "Re:" subject and
  labels filled from the thread, and the quoted text folded under the draft.
- **FR-055**: Sending MUST be local-first: the message is in the Outbox at
  once, online or not, and leaves at most once (ADR 0021).
- **FR-056**: A draft that left the composer MUST say its state where it is
  listed and when it is opened: waiting to send and sending in the Outbox,
  not sent and not confirmed in Drafts. Opened, it MUST offer what settles
  it: Cancel send while waiting; Retry send when not sent; Retry send and
  Mark as sent when not confirmed; and Edit, which takes a waiting send off
  the queue before anything is edited.
- **FR-057**: A `mailto:` URI MUST open the composer filled from it.
- **FR-058**: Selected messages MUST be draggable out of the list to a file
  manager as `.eml` files, produced lazily.

**Command bar, search and going places**

- **FR-060**: `/` and `mod+k` MUST open one bar that filters search,
  commands and places together as the user types. `>` MUST limit it to
  commands. Every command row MUST show its key.
- **FR-061**: A command run from the bar MUST act on the row or the
  selection that was focused when the bar opened.
- **FR-062**: Plain English MUST be lowered, on this machine, into editable
  chips of the one query language (constitution III). It MUST NOT use a
  second language or the network, and MUST NOT need a model. It MUST be
  deterministic. Words it cannot lower MUST stay free text. `Tab` MUST step
  into the chips, and `mod+BackSpace` MUST return to the plain words.
- **FR-063**: Saved searches MUST be pinned across the top of the bar, with
  counts and `alt+1`–`alt+4`. `mod+s` MUST save the current query as a
  saved search, the same kind every app reads.
- **FR-064**: `in:` MUST complete folder names and list a folder's
  conversations newest first. Results MUST be one line each, say where each
  conversation lives, and open over the list on `Enter`.
- **FR-065**: `g o`, or a click on the place name in the header strip, MUST
  open a popover listing mailboxes with their direct keys (`g i`, `g t`,
  `g s`, `g r`, `g z`, `g *`, `g f`), then folders and labels, with counts. Typing
  MUST filter it, and `Enter` MUST go to the chosen place. A mailbox, folder
  or label shown this way MUST have the same rows and actions as the inbox.
- **FR-066**: A query with a likely misspelling MUST offer "Search instead
  for “…”" as a row the bar can run (ADR 0037), and `O` MUST switch a result
  list between relevance and date once a result is chosen.

**States**

- **FR-070**: An empty inbox MUST show a quiet, centred message. It names
  the next digest when there is one, and offers shortcuts to Filtered,
  Archive and Compose, listing only what exists. While the first sync has
  not finished its first pass, it MUST say it is syncing and how far, never
  that the inbox is empty.
- **FR-071**: First sync, offline and a sign-in error MUST each show as one
  banner under the header strip, with the sync label matching. Each banner
  offers what the state needs: progress, Retry now, or Update password….
- **FR-072**: No sync state may block anything local. Everything already on
  this machine MUST stay readable, searchable and actionable, and changes
  MUST queue until sync is back.
- **FR-073**: An account failing for a reason other than its password MUST
  be named in the banner, with the reason (the sync's own words when it gave
  them) and Retry now, while every working account's mail stays listed
  (ADR 0005).

**Keyboard**

- **FR-080**: Every Focus action MUST be a command in the one command
  registry. Its key, its command-bar row, its line in the `?` key map, and
  the keycap on every button that performs it MUST all be generated from
  that registry. A Focus action that is not in the registry does not exist,
  and every command Focus is offered MUST reach a handler.
- **FR-081**: There MUST be one default keymap for every Postio app, and it
  MUST be the one `KEYS.md` sets out (Clarifications).
  - A command that only another app has MUST keep a key, chosen so that it
    does not collide with the keymap.
  - Every binding MUST be overridable from `[keys]` in `config.toml`, by
    command id. There is no `keys.toml` and no per-app key table.
- **FR-082**: A key MUST mean one thing across Focus's surfaces, and a
  command MUST have the same key in every app that has it. No key may be
  bound to two commands in one context. Surfaces only another app has keep
  their own context keys. Undo is `mod+z` only. Inside a text field,
  `mod+z` undoes typing.
- **FR-083**: Every command reachable in Focus MUST have a key, a command-bar
  row and a visible, clickable control. The mouse MUST work everywhere and
  MUST never be required.
- **FR-084**: `?` MUST toggle the key map, and `Esc` MUST close it. The key
  map MUST be grouped as screen 20 groups it, and its footer MUST name
  `[keys]` in `config.toml`.
- **FR-085**: The terminal app MUST follow the one keymap. Raw mode delivers
  `ctrl+z` as a key, and the terminal binds it to undo; it does not suspend.

**Visual language, appearance and accessibility**

- **FR-090**: Focus MUST use libadwaita's named colours, with light and
  dark both following the system through the platform's style manager. The
  open message's surface, ink, hairlines and scrim are the handoff's values
  for those roles (C26).
- **FR-091**: The accent colour MUST be the system's (C26), and MUST be
  reserved for action markers and the open message's action card, links,
  the keyboard focus ring and the has-action toggle. Default buttons (Send,
  Create, Add task, Archive all) MUST be plain raised buttons with bold
  labels, not the suggested-action style, so the accent stays reserved.
  Label colours that Postio assigns MUST avoid the accent's hue; a colour
  the user or their server set is drawn as set (Assumptions).
- **FR-092**: There MUST be one keycap style, one close button, one icon
  button, one dialog pattern for every surface over the app, and one picker
  pattern (a popover anchored to the row).
- **FR-093**: The chrome MUST be set in Adwaita Sans, with keys, addresses,
  counts and operators in Adwaita Mono (C25). A body drawn in app colours is
  set in Barlow (FR-039).
- **FR-094**: Transitions MUST take no more than 100 ms, or be absent, and
  MUST honour reduced motion.
- **FR-095**: Every screen MUST be built against its PNG, and every
  difference between the running app and the PNG MUST be recorded in
  `screens.md` with its reason.
- **FR-096**: Every surface MUST be usable with a screen reader and without
  a mouse. A row MUST announce its sender, subject, first line, unread
  state and marker. A keycap MUST be announced once, as its control's
  shortcut, not read as text.

**Invitations**

- **FR-100**: A message carrying a calendar invitation MUST show an Invite
  marker with the event's date and time in the user's time zone, and Accept
  (`y`) and Decline (`Y`). The marker MUST come from the message's own
  calendar part, with no model involved, and MUST be computed when the
  message is filed.
- **FR-101**: Calendar parsing MUST follow a survey of the Pimalaya family,
  recorded in research R9 (constitution VII).
- **FR-102**: Accepting or declining MUST send a reply to the organiser only
  on the user's keypress. The reply goes through the existing outbox,
  local-first. It waits about ten seconds, during which the undo toast and
  `mod+z` cancel it and nothing is sent. After that it cannot be undone.
- **FR-103**: An updated invitation MUST replace the marker's time. A
  cancelled one MUST say so and offer no Accept. A past one MUST show no
  Accept or Decline. After an answer, the marker MUST show what was
  answered.

**Questions and to-dos**

- **FR-104**: A message sent directly to the user that asks them a question,
  or asks them to do something, MUST get a Question or To-do marker. The
  marker MUST quote the triggering sentence byte-exact, as a span of the
  newest message's own text, never of quoted history or a signature. A To-do
  MUST carry a due date when the sentence names one, read on this machine.
- **FR-105**: When no model is connected, a built-in detector MUST answer
  this question. It is plain code over the message's own text and headers.
  It runs inside Postio, off the UI path, needs no model runtime, and makes
  no network request. Given the same message and the same corrections, it
  MUST always give the same answer.
- **FR-106**: The built-in detector MUST be conservative:
  - it considers only mail sent directly to the user, never mail they are
    only copied on, list or bulk mail, or mail from automated senders;
  - when unsure, it marks nothing.

  Its precision is held to SC-013.
- **FR-107**: When the user has connected a model, the model MUST answer the
  question in place of the built-in detector (FR-170). When that model is
  not running, the built-in detector MUST answer, so markers never wait on
  the model.
- **FR-108**: A dismissed marker MUST be remembered as a correction, and MUST
  never return on that message. Both detectors MUST take dismissals into
  account: three dismissals of one kind from one sender stop that kind for
  that sender (research R10).

**Filtering**

- **FR-110**: While Focus runs, spam, promotions and automated updates
  (notifications, receipts, shipping and social) MUST be archived
  automatically as they are filed, and MUST never appear in its inbox.
- **FR-111**: Guard rules MUST win over every rule, heuristic and classifier.
  A message is never filtered if any of these is true:
  - the user has written to its sender;
  - it comes from the user's own domain;
  - it comes from a pinned sender;
  - it belongs to a conversation the user took part in.
- **FR-112**: When a decision is uncertain, the message MUST go to the inbox.
- **FR-113**: Every filtered message MUST keep its reason, from a fixed
  vocabulary with room for a source ("notification · Forge"), and which
  layer decided it.
- **FR-114**: Automated senders and header patterns MUST be data, never
  named constants or special-cased branches (constitution VII).
- **FR-115**: `g f` MUST open Filtered: newest first, each row with its
  reason, and tabs by reason with counts, reachable by number.
- **FR-116**: `R` MUST restore a message to the inbox and never filter that
  sender again. The correction MUST be remembered and MUST be undoable.
  Opening a message from Filtered MUST use the same dialog as the inbox.
- **FR-117**: Filtered mail MUST be archived, and MUST NOT be deleted
  automatically. Filtered MUST list all of it.
- **FR-118**: Filtering MUST apply to mail filed after it is turned on.
  Applying it to mail already in the inbox MUST be a deliberate command
  (`F`) that shows what would move, and MUST be one undoable action.
- **FR-119**: Filtering MUST be on when Focus first opens, and
  `[focus] filtering` MUST turn it off.

**Digests**

- **FR-120**: `d` MUST open "Digest this sender", pre-filled with the
  sender's address. It MUST offer Daily, Weekly or Monthly, a day and a
  time, and a preview of the mail the rule would have caught in the last 90
  days. "Digest these…" MUST make one rule for the senders of the
  selection.
- **FR-121**: Mail matching a rule MUST be held until its digest is due.
  Held mail stays filed in the inbox, where the other apps and devices see
  it, and Focus keeps it out of its own inbox. It MUST remain searchable,
  and it MUST be listed under its rule at `g d`.
- **FR-122**: Mail with an invitation, a question or a to-do MUST NOT be
  held. Mail in a conversation the user took part in MUST NOT be held.
- **FR-123**: When a digest comes due, exactly one digest row MUST appear in
  the inbox, holding everything the rule held since its last delivery. A
  digest holding nothing MUST make no row. A digest that came due while
  Focus was not running MUST appear when Focus next opens, without loss or
  duplication.
- **FR-124**: A digest row MUST show its cadence, its name and its count.
  When the digest has a summary, its first line MUST be the summary's
  opening, as 01 draws it ("Summary of 14 messages from 6 senders: …");
  otherwise it MUST show the digest's senders.
- **FR-125**: `Enter` on a digest row MUST open the digest window over the
  inbox: on its summary when there is one (FR-172), and on the plain list of
  its messages otherwise. `Tab` switches between the two. From the window:
  - `A` archives the whole digest as one undoable action;
  - `Enter` opens the focused message, or the referenced email, in the same
    window;
  - `d` edits the rule and its cadence;
  - `D` stops digesting a sender;
  - `U` unsubscribes, only on that deliberate key (constitution VI);
  - `Esc` closes, or returns from an opened email to where the user was.
- **FR-126**: `g d` MUST list every digest rule, with its cadence, its next
  delivery and what it holds now. Every rule MUST be editable and removable
  there. Removing a rule MUST release what it held into the inbox.
- **FR-127**: Digest rules MUST be expressed in the one query language, so
  that a sender, list or search rule means what the same query means in
  search (ADR 0008). A rule's preview MUST be the query run over existing
  mail.

**Classification**

- **FR-130**: Deciding filter reasons, digest holding and markers MUST be
  one classification step, run where mail is filed. The step runs in
  layers, and an earlier layer's decision stands:
  1. the guard rules;
  2. the user's corrections. They rank above the rules because a restore
     must beat the rule that filtered the message;
  3. structure and rules: calendar parts, list and bulk headers, automated
     senders as data, the user's digest rules, and the built-in
     needs-action detector when no model is connected;
  4. only when the user has brought one, the local model, for what layers
     1–3 left undecided and for the needs-action question in place of the
     built-in detector.
- **FR-131**: Classification MUST never run on the UI path, and MUST never
  block the UI. A message MUST be listable, openable and actionable before
  it has been classified.
- **FR-132**: The component that classifies MUST be unable to send mail,
  because there is no send path in what it depends on. Its output MUST be a
  fixed schema: a category, spans as character offsets into the body, and a
  due date. It MUST never produce text of its own. Message text MUST be
  treated as data, never as instructions (ADR 0009). This MUST be enforced
  by a boundary check. The digest summariser (FR-172 to FR-175) is the one
  component that writes text, and it is bounded separately.
- **FR-133**: Classification MUST run on this machine only: its rules inside
  Postio, and its model, when the user has brought one, at a local endpoint
  (FR-168). It MUST NOT use a cloud model or reach any other host.
- **FR-134**: Filtering, digest holding, digest delivery and reminders MUST
  act only while Focus is the app running (Clarifications).
  - Mail another app files lands in the inbox as usual. The other apps never
    apply Focus's rules.
  - When Focus opens, it MUST classify everything filed since it last ran,
    in the background, newest first.
  - That mail can then leave the inbox, even if the user saw it in the other
    app.

**Performance**

- **FR-140**: Focus MUST meet the constitution's budgets:
  - startup to a usable inbox under 500 ms, with a populated store;
  - ordinary interaction under 16 ms;
  - local search under 100 ms;
  - transitions of 100 ms or less, or none.

  These MUST be gated as counts: statements, rows and per-row work.
  Timings are measured nightly and report without gating.
- **FR-141**: The first classification of an existing store MUST run in the
  background, at low priority, on at most one CPU core. The inbox is
  classified first, newest first.
  - Filtering, digests and invitations MUST read no message bodies apart
    from calendar parts.
  - The needs-action detector MUST read only the bodies of inbox mail from
    the last 30 days that was sent directly to the user, and only bodies
    already on this machine. It never fetches a body to classify it.

  The time budgets are in SC-011. The same limits apply to the catch-up when
  Focus opens (FR-134).
- **FR-142**: When the user turns the model on, its first pass MUST be
  limited to the inbox and the last 30 days of mail, sent one request at a
  time, in the background. The runtime is the user's own, so Postio bounds
  what it asks of it rather than how it runs. Its progress MUST be visible,
  not hidden. A digest's summary is written after the digest is delivered,
  and MUST never delay the digest row, which shows its senders until the
  summary lands.

**Privacy**

- **FR-150**: Focus MUST make no network request the user did not ask for.
  In particular:
  - no remote image without the user's permission, once or for the sender;
  - no read receipt;
  - no link prefetch;
  - no unsubscribe or RSVP without a deliberate keypress;
  - no model call off this machine, and no connection to a model runtime
    the user has not configured.
- **FR-151**: Logs MUST carry no message content: ids, counts and outcomes
  only. Stored excerpts, reasons and summaries are message content, and live
  only in the encrypted store.
- **FR-152**: Every fixture, test, screenshot committed to the repository,
  issue and commit MUST use reserved domains and fictional people.
- **FR-153**: Focus's notifications for new mail MUST follow `[sync]`'s
  settings, and MUST never fire for mail Focus filtered or held.

**Configuration**

- **FR-160**: Focus's own settings MUST live in a `[focus]` section of
  `config.toml`, reloaded live. Keys stay under `[keys]`.
- **FR-161**: Digest rules, pinned senders and filtering corrections MUST be
  kept where every app reads them. They MUST be readable and correctable by
  the user, as the other rules and saved searches are.
- **FR-162**: Settings MUST open over the list (`mod+comma`, and the main
  menu) in the open message's dialog frame, as the shared settings window
  (ADR 0031) drawn in Focus's type and accent. It MUST show Accounts,
  Filters, Composing, Keyboard, Sync & storage (with a per-folder backfill
  exclusion, ADR 0016), Privacy and Config file, and no key Focus does not
  honour. Every account verb MUST be reachable from the keyboard. `mod+e`
  MUST open `config.toml` in the person's editor from anywhere in Focus.
  Changes to `[keys]`, `[filters]`, `[sync]`, `[focus]`, `[compose]`,
  `[reader]` and `[storage]` MUST take effect without a restart.
- **FR-163**: Focus's first run MUST add an account (the form, or OAuth
  sign-in), then ask how much history to sync, through the shared
  onboarding presenter.

**The local model the user brings**

- **FR-165**: The local model MUST be one the user brings: a model and a
  runtime they install, choose and run on this machine (for example Ollama,
  or a llama.cpp server), named in `config.toml` (Clarifications). Postio
  MUST NOT embed, bundle, download, install or start a model or an inference
  runtime. No language model, no inference engine and no model file may be
  compiled into or shipped with any Postio app, and a dependency check MUST
  enforce it. The built-in needs-action detector (FR-105) is none of these:
  it is plain code, with at most a small table of rules or weights compiled
  in, and it needs no inference engine.
- **FR-166**: The model MUST be optional. It is off until the user names a
  provider. Postio MUST NOT look for a model runtime the user has not
  configured, not even on this machine (constitution VI: no speculative
  connection). Each feature that uses the model MUST have its own switch.
- **FR-167**: Focus MUST be complete without a model. Without one:
  - Question and To-do markers come from the built-in detector (FR-105);
  - digests open on their plain list, and digest rows show their senders
    (FR-175);
  - "Digest mail like this" is absent, while list and search rules still
    work;
  - filtering decides by guards, structure, rules and corrections alone, and
    anything they leave undecided goes to the inbox;
  - every command that needs the model is absent from every surface
    (ADR 0009).

  A configured model that is not running, or too slow, MUST be treated the
  same way. Nothing waits on it.
- **FR-168**: The provider MUST be on this machine: a loopback address or a
  local socket. Focus MUST refuse an endpoint on another host, and say why
  (brief: "Local, always"; ADR 0009's `Locality`). Every call to the model
  MUST be recorded in the egress log, with ids, counts and outcomes only
  (ADR 0009, Q6).
- **FR-169**: Which runtime and model the user brings MUST be configuration,
  not code. Postio speaks a documented local interface, and no runtime or
  model is named or special-cased in code (constitution VII: providers are
  data).
- **FR-170**: When connected, the user's model MUST answer the needs-action
  question in place of the built-in detector, under the same rules (FR-104,
  FR-106, FR-108). It MUST return only the fixed schema (FR-132), so its
  quotes are byte-exact spans of the body, never paraphrased.
- **FR-171**: Digest rules MUST accept a mailing list, a query and "more like
  this", each previewed before it is saved.
- **FR-172**: When the user has brought a model (FR-165), each digest MUST
  open on a summary of its messages, as screen 22 draws it:
  - written on this machine by the local model, from those messages only;
  - grouped by topic;
  - with a footer that says what wrote it.
- **FR-173**: Every statement in a summary MUST end in a numbered reference
  to the message it came from, pinned to a passage in that message. A
  statement without a valid reference MUST NOT be shown. Opening a reference
  MUST show the email in the same window with the passage highlighted (23).
  From there, `j`/`k` step through the digest's sources, and `Esc` returns
  to the summary at the same reference.
- **FR-174**: A summary MUST be plain text. Anything in the model's output
  that looks like markup, a link or an image MUST be shown as characters, so
  a summary can never carry a live link, a tracking pixel or a command (ADR
  0009, Q4). The summariser MUST get no tools and MUST have no send path.
  Message text MUST be fenced as data, never treated as instructions.
- **FR-175**: When no local model is configured or reachable, or a summary
  cannot be written, the digest MUST open on its plain list, and its row
  MUST show its senders. Nothing waits on the model: a digest is listable
  and openable before its summary exists.

**Obsidian and links**

- **FR-180**: Obsidian capture MUST write plain markdown into a vault folder
  the user configures, on this machine, with no plugin and no network:
  - tasks in the Obsidian Tasks format, with a `postio://` link placed
    before the date fields, which the Tasks plugin reads from the end of the
    line;
  - notes created or appended, with quoted excerpts only when the user asks
    for them.

  Postio MUST never edit a note beyond appending a capture.
- **FR-181**: Postio MUST read project notes from the vault (`type: project`
  frontmatter, or a configured folder) to suggest a project. It MUST read
  captured tasks back, to show "Task in <project> · due <day>" on the row
  and to offer to archive the conversation when the task is ticked.
- **FR-185**: A `postio://message/<id>` link MUST open that conversation in
  Postio, on this machine, fetching nothing. A link Postio cannot resolve
  MUST be refused with a message.

### Key Entities

- **Conversation row**: what the list draws for one conversation. The
  newest message's sender, subject and first line, verbatim; unread; up to
  two labels; attachment; message count; time; and optionally an action
  marker. Everything on it is readable without a message body.
- **Action marker**: something a message asks of the user. Its kind
  (invite, question, to-do); a date and time (the event, or a due date);
  for a quote, the sentence as offsets into the body plus a short excerpt;
  what made it (the calendar part, the built-in detector or the user's
  model); the action that answers it; and whether the user has answered or
  dismissed it.
- **Invitation**: what the calendar part says. Title, start and end, time
  zone, location, organiser, whether it is a request, an update or a
  cancellation, and the user's answer.
- **Filter decision**: why a message was archived automatically. The
  message, a reason from the fixed vocabulary with its source, the layer
  that decided it, and when.
- **Guard**: what can never be filtered or held: correspondents (addresses
  the user has written to), the user's own domains, pinned senders, and
  conversations the user took part in.
- **Correction**: something the user taught the classifier. A sender
  restored from Filtered is never filtered again, and a dismissed marker is
  remembered. A correction outranks every layer below the guards.
- **Digest rule**: what to hold and when to deliver it. A name (the
  sender's, unless the user renames it at `g d`); what it matches (senders,
  a list, a query or "like this"); a cadence (daily, weekly, monthly); a day
  and a time; and its next delivery.
- **Digest**: one delivery of a rule. The mail held since the last delivery,
  released as one row when due, and opened on its summary or its list.
- **Digest summary**: the statements the local model wrote for one digest,
  grouped by topic. Each statement carries numbered references, each pinned
  to a passage in one of the digest's messages. It is plain text, written on
  this machine, and absent until it has been written.
- **Model provider**: the runtime and model the user brings, as configured.
  Where it listens on this machine, which model to ask, and which features
  may use it. It is absent until the user configures it.
- **Reminder**: "remind me if no reply". The conversation, when it is due,
  and whether a reply from someone else has cancelled it.
- **Default keymap**: the one default binding for each command id, shared by
  every Postio app, with the user's `[keys]` overrides applied over it.
- **Obsidian capture**: a task line or a note, the vault file it goes to,
  its project, and its `postio://` link back to the message.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: With a store of 100,000 conversations:
  - Focus shows a usable inbox in under 500 ms;
  - it redraws after any keystroke in under 16 ms;
  - it answers a local search in under 100 ms.

  These are gated as counts in CI and timed nightly.
- **SC-002**: Drawing any number of inbox rows reads zero message bodies,
  proven by counting.
- **SC-003**: 100% of the commands reachable in Focus have a key, a
  command-bar row and a visible control, and each shows the key the keymap
  resolves. This is proven by enumeration.
- **SC-004**: With the network absent, a user can open the newest
  conversation, send a one-line reply, archive it, and be back on the next
  row in under 15 seconds, without leaving the keyboard (as the terminal's
  SC-002).
- **SC-005**: Opening a message and closing it returns to the same row, with
  the same selection, in 100% of test runs. A hundred opens build one
  message view.
- **SC-006**: On the filtering fixture corpus:
  - zero messages covered by a guard rule are filtered;
  - every filtered message carries a reason;
  - every filtered message returns to the inbox with one key.
- **SC-007**: 100% of mail matching a sender rule is held, and released as
  exactly one digest row when due, with nothing lost or duplicated across
  restarts. Mail with an invitation is never held.
- **SC-008**: An RSVP leaves only after the user's keypress and after its
  ten-second window, exactly once. Cancelling within the window sends
  nothing.
- **SC-009**: Every screen has been compared with its PNG, and every
  difference is recorded with its reason.
- **SC-010**: Moving code into the shared crates changes no behaviour: the
  suites of every app that still builds pass with nothing changed but
  import paths and default key bindings.
- **SC-011**: For a store of 100,000 messages, on one core of the reference
  workstation, with the interface keeping its interaction budget throughout:
  - the filtering, digest and invitation pass finishes in under 5 minutes,
    reading no body beyond calendar parts;
  - the built-in needs-action pass over the inbox and the last 30 days
    finishes in under 1 minute, reading only bodies already on this machine.
- **SC-012**: Over the maintainer's first month on a real mailbox, fewer
  than 1 in 100 filtered messages are restored. This is measured on this
  machine, from Postio's own store, and reported nowhere else.
- **SC-013**: On the labelled needs-action corpus, at least 9 in 10 of the
  built-in detector's Question and To-do markers are right. How many it
  misses is reported, not gated. On the maintainer's own mailbox, fewer than
  1 in 10 markers are dismissed as wrong, whichever detector made them,
  measured the same way as SC-012.
- **SC-014**: On the summary fixture corpus:
  - every shown statement cites at least one of the digest's messages;
  - every cited passage exists byte-exact in its message;
  - no summary renders markup or a live link.
- **SC-015**: Across every app, no key is bound to two commands in one
  context, every command an app offers has a key, and each command has the
  same default key in every app that offers it. This is proven by
  enumerating the one keymap.
- **SC-016**: With no model configured, every acceptance scenario that needs
  no model passes, Question and To-do markers still appear from the built-in
  detector, and Postio opens zero connections to any model runtime. A test
  that fails on any connection attempt proves it. No Postio package contains
  a language model or an inference engine.

## Assumptions

- **Landing.** Nothing lands on `main` until the maintainer says so
  (Clarifications). The branch is rebased onto `main` as it goes, so it
  stays landable. Nobody runs `issue-land.sh` for it until the maintainer
  asks.
- **The model is the user's own, and optional** (Clarifications).
  - The user installs, chooses and runs it on this machine (for example
    with Ollama, or a llama.cpp server) and names it in `config.toml`.
  - Postio never embeds, bundles, downloads, installs or starts one, and
    never looks for one the user has not configured.
  - Without it, Focus is complete, and every model-backed feature is absent
    or falls back (FR-167). Question and To-do markers fall back to the
    built-in detector, which is plain code, not a model.

  This matches ADR 0009 ("Nothing is bundled") and the constitution's
  Scope (1.3.0).
- **The built-in needs-action detector reads English first.** Mail in a
  language it does not handle gets no Question or To-do marker from it,
  rather than a guess. The user's model, when connected, is not limited this
  way.
- **One inbox across accounts.** This is the brief's proposal. Folders and
  labels are grouped by account wherever there is more than one account;
  `account:` narrows a search to one.
- **Filtering starts on.** It covers mail filed from the first open,
  applying to mail already in the inbox is a deliberate command, and
  `[focus] filtering` turns it off. The brief's caution, that auto-archiving
  must be proven on a real mailbox before it is the default, is met by:
  - the guard rules;
  - a reason on every filtered message;
  - a one-key restore;
  - undo;
  - SC-012, measured on the maintainer's own mailbox.
- **A digest is Focus's view; a filter is a real archive.**
  - Held mail stays filed in the inbox, so the other apps and devices keep
    seeing it there. `A` in the digest is what archives it, which is why
    the digest window offers "Archive all".
  - Filtered mail is archived for real, so every app and device sees it
    archived.
  - Both follow from Focus's rules acting only while Focus runs.
- **Held mail is never out of reach.** It is searchable, listed at `g d`,
  and never held when the user took part in its conversation.
- **One keymap** (Clarifications). `s` snoozes, `*` flags, `d` makes a
  digest rule and delete is the `Delete` key, `mod+z` undoes, `U`
  unsubscribes, and `g d`, `g t`, `g s` and `g f` go where `KEYS.md` sends
  them. Every command any app offers has a key ([contracts/keymap.md](./contracts/keymap.md)).
- **Plain-English search is lowered by local rules.** Correspondent names
  become `from:`/`to:`, date phrases become `after:`/`before:`, and phrases
  such as "with attachments" or "unread" become their operators; the rest
  stays free text.
- **Label colours.** A label's stored colour is used when it has one.
  Otherwise the label gets a stable colour from a palette that excludes the
  accent's hue.
- **Packaging.** Focus is the desktop Flatpak's app. At the package switch
  (T253) it takes the name "Postio", the binary `postio`, the app id
  `dev.postio.Postio`, the launcher, the icon and the `mailto:` and
  `postio:` handlers.
- **Platforms.** This spec covers Linux and GTK. Focus's design on macOS is
  outlined in [macos.md](./macos.md); its logic lives in the toolkit-free
  layers (FR-006), so it is not designed out.
- **The design folders** stay out of the repository until they are
  re-rendered and scrubbed (see *The inputs, and which one wins*).
- **Budgets.** The first-pass numbers (SC-011, FR-141, FR-142) are initial
  targets, converted into counts and refined by nightly measurement.
- **Window sizes.** The screens are drawn at 1440×900. Focus also works at a
  laptop's width and at GNOME's minimum window size. As the window narrows,
  the first line and then the labels give way before the sender, subject and
  time; the open message's frame narrows by FR-039's rule.

## Out of Scope

- A folder sidebar or a three-pane layout. Reading beside the list is the
  open message placed in a pane (FR-038), not a third pane.
- Rewritten subjects, priority scores, summaries or any text Postio wrote in
  a conversation row. The digest summary is the one exception, and it stays
  inside digests (Clarifications).
- Automatic replies or sends. The only automatic acts are archiving filtered
  mail and holding digest mail, both guarded, visible and undoable.
- Cloud models.
- Embedding, bundling, downloading, installing or starting a language model
  or an inference runtime. The model is the user's own, and optional. The
  built-in needs-action detector is plain code, not a model.
- An Obsidian plugin, or editing notes beyond appending captures.
- A new protocol, backend or sync mode.
- Focus in the terminal or on iOS.
