# Feature Specification: Calendar — a view, management and sync beside mail

**Feature Branch**: `feature/calendar`

**Created**: 2026-11-22

**Status**: Draft. The maintainer answered Q1 to Q4 on 2026-11-22
(Clarifications). Not yet scheduled; `tasks.md` is written so it can be

**Input**: The maintainer, 2026-11-22: "I want to create roadmap specs/docs
for: … A calendar view and management".

## Context

Postio reads calendar invitations today and answers them. It has no calendar.

| What exists on `main` | Where |
|---|---|
| An **Invite marker** on a message carrying a `text/calendar` part. The marker shows the event's time in the user's zone and offers Accept `y` and Decline `Y`. An update replaces the time, and a cancellation says so | spec 007 US8, FR-100 to FR-103 |
| **RSVP by mail**: an iTIP `METHOD:REPLY` to the organiser, sent through the outbox about ten seconds after the key, and undoable until then | spec 007 research R9; `Command::Rsvp` |
| **`postio-calendar`**, a pure leaf (no store, toolkit, async runtime or network) over calcard, with three calls: `parse`, `reply` and `supersedes` | constitution VII; `docs/ARCHITECTURE.md` §9 |
| **Desktop notifications** for new mail, per folder role, while an app holds the store | `PRODUCT.md` §17; `postio-host/src/notify.rs` |
| **Focus's timed rules** (snooze, remind-if-no-reply, digest delivery) act only while a Focus app holds the store | spec 007 FR-134 |

What a person cannot do:

- see what is on their calendar;
- see that an invitation clashes with something;
- add, move or cancel an event;
- answer an invitation that came by sync rather than by mail;
- be reminded of an event.

This spec gives Postio a calendar. It is built the way Postio builds mail:

- the store is the truth;
- every change is a store write, then an enqueued remote operation, then
  an event, then a repaint;
- the server is reconciled later, somewhere else;
- providers are rows in the preset table.

### What this spec inherits

- **Constitution I**: local-first; the UI never awaits the network; sync
  state is visible; destructive actions are undoable or confirmed.
- **Constitution II**: every verb is a registry command, in the one keymap,
  rebindable, discoverable in the command bar and the `?` key map.
- **Constitution III**: one query language. Calendar search lowers to it
  (D9).
- **Constitution V**: a calendar is never loaded whole. Views are windowed
  over the paged store, with budgets gated as counts.
- **Constitution VI**: no request the person did not ask for. Free/busy
  lookups of other people, remote attachments and conference links are never
  fetched.
- **Constitution VII**: providers are data; Pimalaya first; `postio-calendar`
  stays a pure leaf.
- **Spec 007's interaction rules** (`screens.md`): one list and one pattern
  over it, the one dialog, the one close, the pickers at the row, `Esc` back
  to the same place. Spec 010 D17 and spec 011 K2 made results and Contacts
  *modes of the main window* with History. Calendar follows them (D1).
- **ADR 0041**: one app holds the store at a time. It bounds what reminders
  can do (D8).
- **ADR 0045 and ADR 0049**: behaviour in `postio-focus` and the rule crates;
  frontends draw intents and assert composition.
- **Issue #2145** (chrono is unmaintained) bears on the iCalendar library
  choice (plan, R2).

## Decisions this spec takes

These are design calls an agent can make. Each is reviewable. The calls only
the maintainer can make are under "Clarifications".

**D1. Calendar is a mode of the main window, not a second app or a pane.**
`g e` ("Go to calendar", mnemonic *events*) opens it. `g e` is free in the
registry; `g c` is Contacts. It sits beside `g i` and `g c` among the
destinations. History returns to where it was opened from, with the cursor
and scroll kept (spec 010 D17). The maintainer chose this on 2026-11-22
(Q1 = A), "with some way of having two windows at once": that is D12's pop
out.

**D2. The views are Agenda, Day and Week, with Month later.**

- **Agenda** (`1`) is a list: one row per occurrence, grouped by day,
  windowed and paged like the inbox. `j` and `k` move the cursor. It is the
  default, because it is the list-shaped view the keyboard model already
  serves. It works at every width.
- **Day** (`2`) and **Week** (`3`) are time grids. `j` and `k` step through
  events in time order, and the cursor is an event, never an empty slot.
  Below 1024 px, Week falls back to Day.
- **Month** (`4`) is a later slice. It is a planning glance with little
  keyboard work in it, and it is the view that most needs a design handoff.

Agenda is built now, designed by an agent in the existing list language. Day,
Week and Month wait for a designer handoff (Q4 = C).

**D3. Moving through time is `[`, `]` and `t`.** `[` and `]` go to the
previous and next period: a day in Day, a week in Week, a week of agenda in
Agenda. `t` goes to today. `g g` and `G` go to the first and last loaded
occurrence in Agenda. These keys are in the Calendar context only, as
Contacts' `[` and `]` are.

**D4. Making and changing events.**

| Verb | Key (Calendar context) | Recovery |
|---|---|---|
| New event | `c` | Undo (it was a store write) |
| Open event | `Return` | None |
| Edit event | `mod+e` (as Edit contact) | Save is undoable |
| Delete event | `#` or `Delete` | Undo |
| Move to another time | `v`, a picker at the row with "Tomorrow", "+1 h", "Next week" and a typed time (spec 007's date picker) | Undo |
| Accept, decline | `y`, `Y` (the existing ids, offered in Calendar too) | Undo briefly, as today |
| Tentative | `alt+y` | Undo briefly |
| Duplicate | `mod+d` | Undo |

`c` is the key for making a new thing on the surface the person is on. It is
Compose in mail and New event in Calendar. Gmail and Google Calendar split it
the same way. The command bar shows "New event" there and "Compose"
elsewhere, so the key teaches itself.

**D5. The event editor is one dialog over the calendar, with its own Save
verbs, and no confirmation dialogs.**

- The fields are:
  - title;
  - start and end, typed in the date grammar Postio already parses
    (`postio-search`'s `parse_when`: "tue 3pm", "next friday 10-11");
  - all-day;
  - time zone, shown only when it is not the person's;
  - repeat;
  - location, plain text;
  - calendar;
  - attendees, completed from Contacts;
  - reminders;
  - notes, plain text.
- `mod+s` saves.
- When the event has attendees, `mod+Return` saves **and sends** the
  invitation or update. Sending is the externally visible half, so it is a
  separate verb and never a side effect of Save. It is undoable for about
  ten seconds, as RSVP and Send are.
- When a recurring event is changed, a picker at the row asks: "This event",
  "This and following", "All events". The same picker appears for Delete
  and for Move. It is a question the edit cannot be made without, so it is
  not a confirmation.

**D6. An invitation and the calendar are one thing.**

- An Invite marker in the inbox shows **"Clashes with Standup"** when a
  local occurrence overlaps. This is a local read with no network.
- Accepting writes the event into the person's calendar. If the invitation
  came by mail to an account with a synced calendar, it goes into that
  account's default calendar. Otherwise it goes into "On this computer".
- **One reply, never two**:
  - when the account's calendar server does scheduling itself (CalDAV
    scheduling, RFC 6638, which the preset row declares), the answer is a
    participation change on the server's copy, and the server tells the
    organiser;
  - otherwise the answer is the iTIP reply by mail, as today.

  The preset row decides which, not code.
- An invitation that arrives only by sync, with no mail, shows in Agenda
  with the same Invite chip and the same `y`, `Y` and `alt+y`.

**D7. "On this computer" always exists.** A store with no calendar account is
a normal, fully working state, as Contacts' is (spec 011 H1). Events there
never leave the machine. One calendar per account syncs, and calendars can
be shown or hidden per calendar.

**D8. Reminders are the event's own alarms, shown as desktop notifications.**

- `DISPLAY` and `AUDIO` alarms notify.
- An `EMAIL` alarm is **never** acted on, because Postio never sends mail on
  a timer.
- A calendar has a default reminder, set in Settings.
- A notification offers "Open" and "Snooze 5 min", from the registry.
- Reminders fire **only while an app holds the store** (Q2 = A). Nothing
  new runs in the background.
- On start, a missed reminder from the last 12 hours shows once: "While
  Postio was closed: Standup at 09:30."

**D9. Calendar search is the one query language.** `/` in Calendar searches
events, as `/` searches mail:

- free text matches the title, location, notes and attendee names;
- `after:` and `before:` bound the time;
- `with:` and `from:` name an attendee or the organiser;
- `in:` names a calendar.

An operator that means nothing for an event narrows nothing, and the hint
says so. No second grammar is added.

**D10. Nothing about other people is fetched.** Free/busy lookups, a
conference link's page, an attachment by URL and a location's map are not
fetched. A link in an event's notes is text until the person opens it, as in
mail.

**D11. Providers are rows.** A preset row gains an optional `[caldav]`
table. It holds:

- the context root, or discovery by RFC 6764;
- the authentication it reuses from the row (app password or OAuth);
- any extra OAuth scope, asked for only when the person turns the account's
  calendar on;
- whether the server schedules (D6);
- quirks, as data with a comment, as `condstore_reliable` is.

A provider with no row is reached by RFC 6764 discovery and the person's
credentials, as a custom IMAP server is.

**D12. Calendar can pop out into its own window, and back.** The maintainer
asked for "some way of having two windows at once. maybe pop out" (Q1). It
was settled with `/ux-architect`. The rules allow a second window when it is
"a real non-modal window, opt-in only", and this is that.

- **The verb is one, and general.** `pop_out` ("Open in its own window") is
  offered on any surface that declares it can stand alone. Calendar is the
  first. Contacts declares it when Linux adopts Contacts (spec 011 S13), and
  the registry needs no new command then. The composer's `detach_composer` is
  the same idea; folding it into `pop_out` is filed as follow-up work, not
  done here.
- **The key** is `mod+shift+o` in the Calendar context: the same chord
  `detach_composer` uses in the composer, so the one gesture means "this
  surface in its own window" wherever it is offered. In the popped-out
  window, the same chord is `pop_in` ("Back into the main window"), so one
  key toggles. It cannot collide with `toggle_reader_view`'s `ctrl+shift+o`,
  which is offered only in list, thread and reader contexts.
- **The keyboard is the focused window's.** Each window has its own context,
  cursor and History. The command bar, `?` and Settings open in the window
  that has focus. Nothing is typed into a window the person is not looking
  at.
- **Destinations go to the window that shows them.** With Calendar popped
  out:
  - `g e` from mail raises and focuses the Calendar window;
  - `g i`, `g c` and the other mail destinations, pressed in the Calendar
    window, raise and focus the main window at that destination;
  - the main window offers no Calendar mode while it is popped out, so a
    surface is never in two places.
- **`Esc` and History stay per window.** In the Calendar window, `Esc` climbs
  that window's History: the open event, then the editor, then a search, then
  the view back to Agenda on today. At the root it does nothing. It never
  closes the window, because a key that sometimes closes a window is a trap.
- **Closing the Calendar window pops Calendar back in.** Its X, or `pop_in`,
  does this. Nothing is lost: the view, the date and the cursor come back as
  Calendar's mode. `mod+q` still quits Postio, both windows. `mod+w` keeps its
  registry meaning (see the open question).
- **What is remembered** (ADR 0024: intent is stored, the viewport's
  constraint is applied): whether Calendar is popped out, and the window's
  size. Its **position is not**. Wayland gives position to the compositor,
  so remembering it would work on X11 and the Mac only, and differ by
  platform. The Mac remembers its frame through the system, as Mac windows
  do. A popped-out Calendar reopens popped out on the next start, after the
  main window is usable, so the startup budget is the main window's.
- **Where it is offered**: the GTK app and the Mac. Not the terminal, which
  has one screen.
- **Same process, same store.** No second app opens the store (ADR 0041). A
  reminder's "Open" goes to whichever window shows Calendar.

## Clarifications

### Session 2026-11-22 (the maintainer)

- **Q1. Where does Calendar live?**
  → **A, a mode of the main window on `g e`, "with some way of having two
  windows at once. maybe pop out".** Pop out is D12.
- **Q2. Do reminders fire when no Postio app is running?**
  → **A.** Only while an app holds the store, with the "While Postio was
  closed" catch-up on start (D8).
- **Q3. Which providers are in the first sync slice?**
  → **A.** Generic CalDAV, iCloud and Fastmail first; Google second.
- **Q4. Where does the visual design come from?**
  → **C.** Agenda is built now, designed by an agent in the existing list
  language. Day, Week and Month wait for a designer handoff, and their tasks
  are held for it (D2).

### Open question

- **Q5. What does `mod+w` do in a second window?** Today it is `quit`,
  everywhere. GNOME's convention is that `ctrl+w` closes the focused window.
  - **A: keep it `quit`.** One verb, one meaning. A second window closes with
    its X or `pop_in`. This is the spec's default.
  - **B: `mod+w` closes the focused window**, which in the main window still
    quits. That gives the key a second meaning in a second window, and
    changes a key people already use.
  - **C: move `quit` to `mod+q` alone**, and make `mod+w` "close this
    window" everywhere. The composer's detached windows get it too.

## The surface

### Agenda

- One row per occurrence, under a sticky day heading ("Today · Tue 24
  Nov"), with:
  - time, in Adwaita Mono;
  - title;
  - the calendar's colour as a 3 px mark (colour is never the only carrier:
    the calendar's name is in the row's accessible name and in the open
    event);
  - location;
  - the Invite chip when unanswered;
  - "Tentative" or "Declined" when so;
  - a sync mark while a change is queued.
- The cursor and the selection follow constitution II, as in the inbox.
- It opens on today, with the cursor on the next event that has not ended.

### Day and Week (held for the designer handoff)

- A time grid with all-day events across the top, and "now" as a line.
- An event's block shows its title and time.
- The cursor is drawn as the focus ring on an event block.
- `j` and `k` step through events in time order; `[` and `]` go to the
  previous and next period.

### The open event

The one dialog shows:

- the title, time and zone, and repeat as words ("Every weekday until 18
  Dec");
- the location;
- the organiser and attendees, with their answers;
- the notes as text;
- the calendar;
- the reminders;
- the verbs with their keycaps: Edit, Delete, Move, Accept, Tentative,
  Decline, Duplicate, and "Email attendees". The last opens the composer to
  every attendee, which is the one cross-over to mail.

### Every state

| State | What the calendar shows |
|---|---|
| Empty (no events in range) | "Nothing on <range>." with `c` to add one, and `]` to look further |
| No calendar account | Agenda of "On this computer", with a line: "Add a calendar account in Settings to sync" (`mod+,`) |
| Partial (first sync running) | What is local, and a line with the sync state: "Syncing Work calendar · 340 of 1,200" |
| Offline | Everything works. Changes are queued and show the sync mark, and the strip says "Offline · 2 changes waiting" |
| Failing | The strip names the error ("Work calendar: the server refused the password") with the retry key and the credential key |
| Conflict (a change both here and on the server) | The event shows "Changed on <server> too", and opens on both versions with "Keep this one" for each. Nothing is lost |
| Narrow (1024 px and below) | Week falls back to Day. Agenda is unchanged |

## User Scenarios & Testing *(mandatory)*

### User Story 1 - See what is coming up (Priority: P1)

A person presses `g e` and sees today and the days after, as a list they can
walk with `j` and `k`. They open an event with `Return`, and go back to the
inbox with `Esc` at the row they left.

**Why this priority**: It is the "calendar view", and it needs nothing else
in this spec but a local store of events. Even with no sync, accepted
invitations and local events fill it.

**Independent Test**: Seed a store with occurrences (single, recurring
across a DST change, all-day, multi-day, and in another zone). Press `g e`
and assert the rows, groups and times a person would see. Assert the counted
statements of a page.

**Acceptance Scenarios**:

1. **Given** events today and tomorrow, **When** the person presses `g e`,
   **Then** Agenda opens on today, with the cursor on the next event that has
   not ended.
2. **Given** a weekly event with an exception and an EXDATE across a DST
   change, **When** Agenda shows those weeks, **Then** each occurrence is at
   its correct local time, and the excluded one is absent.
3. **Given** Agenda open from the inbox, **When** the person presses `Esc`,
   **Then** the inbox is back with its cursor and scroll unchanged.
4. **Given** 50,000 occurrences in the store, **When** Agenda opens, **Then**
   it reads one page, within the interaction budget, gated as counts.

---

### User Story 2 - Answer an invitation knowing what it clashes with (Priority: P1)

An invitation arrives by mail. Its Invite marker says it clashes with a
standup. The person presses `y`, and the event appears in their calendar.
The organiser gets one answer.

**Why this priority**: It joins the one calendar feature Postio has (US8 of
spec 007) to the calendar, and it is the most common calendar act for a
person with too much email.

**Independent Test**: Over the corpus invitations, with local events seeded,
assert the clash wording on the marker. Press `y`, and assert the event in
Agenda. Against the mock backends, assert exactly one answer: a mail reply
for a non-scheduling account, or a participation change for a scheduling
one.

**Acceptance Scenarios**:

1. **Given** an invitation overlapping a local event, **When** it is filed,
   **Then** its marker reads "Clashes with Standup".
2. **Given** an account whose preset says its server schedules, **When** the
   person presses `y`, **Then** the server's copy changes and no mail reply
   is queued.
3. **Given** a non-scheduling account, **When** the person presses `y`,
   **Then** the mail reply is queued as today, and the event is written to
   "On this computer" or the account's calendar.
4. **Given** `y` pressed, **When** the person presses `mod+z` within the
   window, **Then** nothing is sent, and the event is gone from the
   calendar.
5. **Given** an update to an accepted event, **When** it arrives, **Then**
   the calendar's copy moves, and the marker says "Moved to Wed 10:00".

---

### User Story 3 - Add, move and cancel events (Priority: P1)

A person presses `c`, types "Dentist" and "thu 3pm", and saves. They move it
with `v` to "next week" and later delete it with `#`. Every step is instant,
offline too, and undoable.

**Why this priority**: It is "management", and it is local-first by
construction, so it tests the whole store-write, enqueue, repaint path.

**Independent Test**: Against the mock CalDAV backend, run each verb offline
and online. Assert that the repaint precedes any network, that the queue
holds one operation per verb, that undo removes or reverts it, and that a
server conflict parks both versions.

**Acceptance Scenarios**:

1. **Given** Calendar open, **When** the person presses `c`, types a title and
   "thu 3pm", and presses `mod+s`, **Then** the event appears at once, with a
   sync mark until the server has it.
2. **Given** a recurring event, **When** the person deletes one occurrence,
   **Then** the picker asks "This event / This and following / All events",
   and the choice is applied as an exception or a truncation.
3. **Given** an event with attendees, **When** the person presses `mod+s`,
   **Then** no invitation is sent. **When** they press `mod+Return`, **Then**
   the invitation is sent after the undo window.
4. **Given** the account offline, **When** the person moves an event,
   **Then** it moves at once, and the strip shows the waiting change.
5. **Given** an event changed here and on the server, **When** sync meets
   both, **Then** non-overlapping field changes merge. An overlap shows the
   conflict state with both versions, and nothing is lost.

---

### User Story 4 - Sync calendars from a provider (Priority: P2)

A person whose mail account is iCloud, Fastmail or Google, or a CalDAV
server, turns on that account's calendars in Settings. Postio finds them,
syncs them, and keeps them current.

**Why this priority**: It makes the calendar the person's real calendar. It
waits on US1 to US3 being solid on the local store, and on Q3.

**Independent Test**: Against an in-process CalDAV server written against
the wire (as `postio-account`'s IMAP test server is), assert discovery, the
initial sync, incremental sync by sync token, ETag conflicts, and deletion
on both sides. Live servers run only in `#[ignore]` cases.

**Acceptance Scenarios**:

1. **Given** an iCloud mail account, **When** the person turns on its
   calendars, **Then** Postio discovers them from the preset row with the
   same app password, and lists them with show and hide.
2. **Given** a synced calendar, **When** an event changes on the server,
   **Then** the next sync moves only that event, by sync token.
3. **Given** a provider with no preset row, **When** the person enters a
   CalDAV address or lets discovery find it (RFC 6764), **Then** it syncs
   like a preset.
4. **Given** Google (Q3), **When** the person turns on calendars, **Then**
   Postio asks for the calendar scope at that moment, and not at mail
   sign-in.

---

### User Story 5 - Be reminded (Priority: P2)

Ten minutes before a meeting, a notification says "Standup · 09:30 · in 10
min", with Open and Snooze.

**Why this priority**: A calendar without reminders is half a calendar, but
reminders fire only while Postio runs (Q2 = A).

**Independent Test**: With a fake clock, assert each alarm's notification
text and time. Assert that snooze moves it, that an `EMAIL` alarm does
nothing, and that a missed alarm shows once on start.

**Acceptance Scenarios**:

1. **Given** an event with a 10-minute display alarm, **When** the clock
   reaches it, **Then** one notification shows, worded as above.
2. **Given** a notification, **When** the person chooses Snooze, **Then** it
   returns in 5 minutes.
3. **Given** an alarm that fell while no app ran, **When** Postio starts
   within 12 hours, **Then** "While Postio was closed: …" shows once.
4. **Given** an `EMAIL` alarm, **When** it falls due, **Then** no mail is
   sent and no notification shows.

---

### User Story 6 - Plan the week (Priority: P3)

A person presses `3` and sees the week as a grid, steps through its events
with `j` and `k`, and goes to next week with `]`.

**Why this priority**: It is valuable, and it waits on a designer handoff
(Q4 = C).

**Independent Test**: Seed a week. Press `3`, `j`, `k`, `]` and `t`, and
assert the event under the cursor and the week shown after each. At 1024 px,
assert the Day fallback.

**Acceptance Scenarios**:

1. **Given** a week with overlapping events, **When** Week shows it, **Then**
   overlapping events share the column, and `j` steps through them in start
   order.
2. **Given** the window at 1024 px, **When** the person presses `3`, **Then**
   Day shows, and the footer says Week needs a wider window.

---

### User Story 7 - Calendar beside mail, in two windows (Priority: P2)

A person planning their week wants mail and the calendar side by side. In
Calendar they press `mod+shift+o`. Calendar moves into its own window. They
put it beside the main window, which is back on the inbox. `g e` from mail
brings the Calendar window forward, and `g i` there brings mail forward.
Closing the Calendar window puts Calendar back where it was.

**Why this priority**: The maintainer asked for it with Q1. It needs Agenda
(US1), and nothing else in this spec.

**Independent Test**: In `focus_suite`, pop out, assert two windows and the
Calendar window focused with its cursor where it was. Press `g i` there and
assert the main window is focused on the inbox. Press `g e` and assert the
Calendar window is raised. Close it, and assert Calendar is a mode again,
with the date and cursor kept. Restart, and assert it reopens popped out with
its size.

**Acceptance Scenarios**:

1. **Given** Calendar on Thursday with the cursor on an event, **When** the
   person presses `mod+shift+o`, **Then** Calendar opens in its own window on
   Thursday at that event, and the main window returns to where Calendar was
   opened from.
2. **Given** Calendar popped out, **When** the person presses `g e` in the main
   window, **Then** the Calendar window is raised and focused, and the main
   window shows no Calendar mode.
3. **Given** the Calendar window at its root, **When** the person presses
   `Esc`, **Then** nothing closes.
4. **Given** the Calendar window, **When** the person presses `mod+shift+o` or
   closes it, **Then** Calendar is the main window's mode again, with the date
   and cursor it had.
5. **Given** Calendar popped out at quit, **When** Postio starts, **Then** the
   main window is usable first, and the Calendar window then opens at its
   remembered size.

---

### Edge Cases

- **Floating times** (no zone): shown at the same clock time in every zone,
  and marked so in the open event.
- **An event in another zone**: Agenda shows local time, and the open event
  also shows the event's own zone.
- **The person travels** (the system zone changes): occurrences re-resolve,
  and floating times stay put.
- **A recurring rule with no end**: occurrences are expanded only within a
  window (plan R4), and paging past it expands more.
- **A huge calendar** (10 years of history): the initial sync pulls a window
  first (from a year back to two years ahead), then the rest in the
  background, as backfill does.
- **A read-only calendar** (a subscription, a shared calendar without write
  access): verbs that would write are absent for its events, not refused
  after the fact.
- **The same event in two calendars** (invited on two accounts): shown once,
  with both calendars named.
- **A cancelled occurrence of an accepted series**: removed, and the Invite
  chip on its mail says "Cancelled".
- **Unknown properties and components** (VTODO, VJOURNAL, X- properties):
  kept byte-for-byte through an edit. VTODO is not shown.
- **The server rejects a change** (403, 412): the change stays local with
  the failing state and "Retry" or "Discard my change". Nothing is silently
  dropped.

## Requirements *(mandatory)*

### Functional Requirements

**The surface and keys**

- **FR-001**: `g e` MUST open Calendar as a mode of the main window, with
  History back to where it was opened, keeping the cursor and scroll.
- **FR-002**: Agenda (`1`), Day (`2`) and Week (`3`) MUST be offered, Month
  (`4`) in a later slice. `[`, `]` and `t` MUST move by period and to today.
- **FR-003**: Every calendar verb MUST be a registry command with a key, a
  command-bar row and an accessible action, in the one keymap and
  rebindable. Existing ids (`accept_invite`, `decline_invite`, `delete`)
  MUST be reused where the verb is the same.
- **FR-004**: Agenda MUST be windowed over the paged store. A view MUST NOT
  read more than its visible range plus one page. Each MUST carry counting
  assertions.
- **FR-005**: Every state in "Every state" MUST be drawn as specified,
  including offline, failing and conflict.
- **FR-006**: Every interface that offers Calendar (GTK, terminal, macOS)
  MUST draw it from one view model, with behaviour in `postio-focus`.
- **FR-007**: `pop_out` (`mod+shift+o`) MUST move Calendar into its own
  window in the same process, and `pop_in`, the same chord there, or closing
  that window MUST bring it back as a mode. The view, the date and the cursor
  MUST be kept both ways.
- **FR-008**: With Calendar popped out, `g e` MUST raise and focus its window,
  and mail destinations pressed there MUST raise and focus the main window.
  Each window MUST keep its own History, and `Esc` MUST NOT close a window.
- **FR-009**: Whether Calendar is popped out, and the window's size, MUST be
  remembered. Its position MUST NOT be stored.

**Events**

- **FR-010**: Creating, editing, moving, duplicating and deleting an event
  MUST be a store write, then an enqueued remote operation, then an event,
  then a repaint. They MUST NOT await the network.
- **FR-011**: Each of those verbs MUST be undoable from the toast and
  `mod+z`.
- **FR-012**: A change to a recurring event MUST ask "This event / This and
  following / All events" with a picker at the row, and MUST write an
  exception, a truncated rule or a changed master to match.
- **FR-013**: Saving an event with attendees MUST NOT send anything.
  "Save and send" (`mod+Return`) MUST send invitations or updates only after
  an undo window, through the server's scheduling or by iTIP mail as the
  preset row says.
- **FR-014**: An edit MUST preserve, byte for byte, every property and
  component Postio does not model.
- **FR-015**: Times MUST be resolved with their zones, including Windows and
  Exchange zone names and zones with no VTIMEZONE (spec 007 R9's corpus).
  Floating times MUST stay floating.

**Invitations**

- **FR-020**: An Invite marker MUST show a clash with any overlapping local
  occurrence, computed locally when the message is filed and again when the
  calendar changes.
- **FR-021**: Accepting, declining or answering tentatively MUST write the
  event's participation to the person's calendar, and MUST send exactly one
  answer to the organiser: by the server's scheduling when the preset says
  the server schedules, otherwise by mail as spec 007 FR-102 does.
- **FR-022**: An invitation that arrives by sync MUST offer the same verbs
  in Calendar.

**Sync**

- **FR-030**: CalDAV MUST be the sync protocol: discovery (RFC 6764 and
  current-user-principal), the initial sync of a time window and then the
  rest, incremental sync by sync token, and writes with ETag preconditions.
- **FR-031**: Server settings MUST be data in the preset table's `[caldav]`
  table. Code MUST NOT name a provider.
- **FR-032**: Credentials MUST be the account's own, from the OS keyring. An
  extra OAuth scope MUST be requested only when the person turns that
  account's calendars on.
- **FR-033**: A conflict MUST be merged field by field when the two sides
  changed different fields, and MUST otherwise be kept as two versions for
  the person to choose. Sync MUST NOT lose either.
- **FR-034**: Sync state MUST be visible per calendar, and MUST name the
  error and the key that retries it.
- **FR-035**: "On this computer" MUST always exist, and MUST never sync.

**Reminders**

- **FR-040**: `DISPLAY` and `AUDIO` alarms MUST produce one desktop
  notification at their time while an app holds the store. An `EMAIL` alarm
  MUST NOT send mail or notify.
- **FR-041**: A notification MUST offer Open and Snooze, which are registry
  commands.
- **FR-042**: A reminder that fell due while no app held the store MUST be
  shown once on the next start, if it is less than 12 hours old.
- **FR-043**: Postio MUST NOT add a background process, an autostart entry or
  a helper for reminders (Q2 = A).

**Privacy**

- **FR-050**: Postio MUST NOT fetch any free/busy, conference page,
  attachment URL or map for an event, nor any URL in its text.
- **FR-051**: Logs MUST carry no event titles, locations, notes, attendees
  or calendar names: ids, counts and outcomes only.
- **FR-052**: Every calendar connection MUST be recorded in the egress log
  under its account, with a new `caldav` subsystem value.

### Key Entities

- **Calendar**: an account, or "On this computer"; a name and a colour;
  writable or read-only; shown or hidden; a default reminder; its sync
  state (sync token, last synced, error).
- **Event**: one iCalendar object, identified by its UID. It holds the
  server's resource address and ETag, the raw object kept for byte-faithful
  edits, and the modelled fields: title, start and end with zone, all-day,
  recurrence rule and exceptions, location, notes, organiser, attendees with
  their participation, and alarms.
- **Occurrence**: one instance of an event in time, expanded within a
  window, which is what views page over.
- **Pending change**: a queued create, update or delete, with the base
  version it was made against, for the three-way merge.
- **Reminder**: an alarm's due instant for one occurrence, and whether it
  has fired, been snoozed or been missed.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: From anywhere in mail, a person reaches today's agenda in one
  chord, and back to the same row in one key.
- **SC-002**: Creating, moving and deleting an event repaints within the
  interaction budget, online or offline, with no network wait. That is
  asserted as zero awaited connections before the repaint.
- **SC-003**: Every occurrence in the corpus's recurrence and zone fixtures
  lands at the instant a reference implementation gives: 100%.
- **SC-004**: Across sync tests, zero local or remote changes are lost in
  conflicts: every conflict either merges or keeps both versions.
- **SC-005**: Accepting an invitation sends exactly one answer to the
  organiser, in 100% of scheduling and non-scheduling cases.
- **SC-006**: Opening Calendar on a store with 50,000 occurrences reads a
  bounded number of statements, independent of the store's size.
- **SC-007**: No calendar action makes a connection to any host other than
  the person's calendar server. Asserted over the hostile event fixtures
  (URLs in notes, attachments, conference links).

## Assumptions

- CalDAV covers iCloud, Fastmail, Google (with the calendar scope) and
  generic servers. Microsoft 365 has no CalDAV; it waits on Microsoft Graph,
  which is unscheduled (`PRODUCT.md` §3), and is out of scope.
- JMAP Calendars is not assumed. If the Fastmail row's JMAP backend gains
  calendars, it is a second calendar backend behind the same trait, filed
  then.
- Tasks (VTODO) are out of scope. They are kept intact and not shown.
- Event search is the one query language over event fields (D9). Mail search
  does not return events.
- Sharing calendars, managing other people's access and booking pages are
  out of scope.
- The terminal draws Agenda and the open event in character cells. Day and
  Week in the terminal are a later question.
