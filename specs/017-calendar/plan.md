# Implementation Plan: Calendar

**Branch**: `feature/calendar` | **Date**: 2026-11-22 | **Spec**: [spec.md](spec.md)

**Input**: `specs/017-calendar/spec.md`; spec 007 US8, FR-100 to FR-103 and
research R9; `origin/main` at `17fff8da`.

The maintainer answered Q1 to Q4 on 2026-11-22 (spec, Clarifications). The
Phase 1 artifacts are [data-model.md](data-model.md),
[contracts/](contracts) and [quickstart.md](quickstart.md), and the queue is
[tasks.md](tasks.md). Phase 0's research is in this file (R1 to R9). The
build is not yet scheduled.

## Summary

A calendar in the store, beside mail, with three parts:

- **Agenda, Day and Week views** as a mode of the main window (`g e`), with
  Month later;
- **local-first management**: create, edit, move, delete and RSVP, each a
  store write first;
- **CalDAV sync** for providers described as preset rows.

Invitations join it: the Invite marker names clashes, and an answer goes
once, by server scheduling or by mail. Reminders are the events' own alarms,
shown only while an app holds the store (Q2 = A). Calendar can **pop out** into
its own window and back (D12).

**The first slice (A)** is the store, the recurrence expansion and Agenda,
over "On this computer" and the invitations already in mail. It needs no
network and no new design (Q4 = C), and it makes accepting an invitation put
the event somewhere.

## Technical Context

**Language/Version**: Rust (1.100, edition 2024); Swift for the Mac.

**Primary Dependencies**:

- **iCalendar**: `calcard` 0.3.14, today's choice (spec 007 R9), or
  Pimalaya's new **`ical-rs`** 0.7. Spike S1 decides (R2);
- **CalDAV**: Pimalaya's **`io-webdav`** 0.13, over `io-http` and
  `pimalaya-stream`, which the graph already has;
- **discovery**: **`io-pim-discovery`**'s `rfc6764` feature. The crate is
  already a dependency of `postio-account`.

**Storage**: Turso store. New tables `calendars`, `events`,
`event_occurrences`, `event_changes` and `reminders`. The raw iCalendar
object is in the blob store, or inline when small.

**Testing**:

- `postio-calendar` `--lib` over the corpus and new recurrence and zone
  fixtures;
- an in-process CalDAV server written against the wire, as
  `postio-account`'s IMAP server is;
- counting assertions on view reads;
- fake clocks for reminders (`start_paused`);
- `focus_suite` and the storyboards.

**Target Platform**: Linux (GTK, terminal) and macOS.

**Performance Goals**:

- Agenda and Week pages are bounded statements, independent of store size;
- a store write and repaint fit within 16 ms;
- startup is unchanged, because calendar sync starts after mail is usable.

**Constraints**:

- no awaited network on any path that repaints;
- no fetch of anything an event links to;
- no provider in code;
- `postio-calendar` stays a pure leaf.

**Scale/Scope**: up to 10 years of history across several calendars (tens of
thousands of events); occurrences are expanded only in a window.

## Constitution Check

| Principle | How this plan meets it |
|---|---|
| I. Local-first | Each verb is a store write, an `event_changes` row, an emitted event, then a repaint. The CalDAV drain runs in `postio-sync`'s queue later. Sync state is per calendar |
| II. Keyboard is a system | Every verb is in the registry, in the Calendar context. `accept_invite`, `decline_invite` and `delete` are reused, and `go_to_calendar` is `g e`. `docs/keybindings.md` regenerates |
| III. One query language | Calendar search uses `postio-search`'s parser. The executor for events reads event fields. No new grammar (D9) |
| IV. Test-first | Every slice opens with red cases: corpus, wire server and storyboard |
| V. Performance | Occurrences are windowed and paged, with counting assertions on every view read |
| VI. Privacy | No free/busy, link or attachment fetch (FR-050). A `caldav` value joins `egress_log.subsystem`. Logs carry ids only. Reminders never send mail |
| VII. Boundaries; providers are data; Pimalaya first | `postio-calendar` stays pure. CalDAV sits behind a `CalendarBackend` trait in a protocol crate, as IMAP sits behind `MailBackend`. Presets gain `[caldav]`. Pimalaya was surveyed first (R1) |

**Gate result**: pass. Q2 = A adds no background presence, so ADR 0041 is
untouched. Pop out is a second window in the same process: one app still
holds the store.

## Phase 0: Research

### R1. Pimalaya first: what it offers for calendars

Surveyed 2026-11-22.

| Pimalaya piece | What it is | Fit |
|---|---|---|
| **`io-webdav`** 0.13 (2026-11-21) | WebDAV client: I/O-free coroutines, a light client over a caller's stream, a full client with TLS. It covers CalDAV (RFC 4791) with home-set discovery, multiget and ETag-only listing; sync-collection and sync tokens (RFC 6578); current-user-principal (RFC 5397); scheduling's UID conflict (RFC 6638); Basic and Bearer auth. It also covers CardDAV (RFC 6352) | **Adopted** as the CalDAV transport. It is the same family and the same `io-http` and `pimalaya-stream` layers Postio's JMAP and AI clients already use. Spec 011's CardDAV slice (S12e) can share it |
| `io-calendar` 0.0.1 | Common types over `io-vdir` and `io-webdav`, using calcard and chrono | **Not adopted**: too young, and Postio needs its own model and store, not common types. Surveyed again before slice C |
| **`ical-rs`** 0.7.3 (2026-11-20, a rewrite) | An iCalendar parser, validator, **byte-faithful editor**, recurrence expansion across zones with the **IANA database included**, a **three-way merge**, and iTIP rules. It is built on `jiff`, not chrono, and is no_std | **Strong candidate** for R2. Byte-faithful edits (FR-014) and three-way merge (FR-033) are two hard parts of this spec, already built |
| `io-pim-discovery` 0.15 | CalDAV and CardDAV context-root discovery: DNS SRV, TXT and the well-known URI (RFC 6764) | **Adopted**: one feature on a crate already in `postio-account` |
| `io-gcal` 0.6 | The Google Calendar REST API, with sync tokens and push channels | **Not adopted** (Q3 = A: Google over CalDAV, second) |
| `io-msgraph` 0.10 | Microsoft Graph | Out of scope (Graph is unscheduled) |
| `calendula`, `neverest`, `pimdir` | A CLI, a sync engine and a store format | **Prior art, not dependencies.** Postio has its own store and engine. Neverest's "a real conflict parked for a decision, never lost" matches FR-033 |

### R2. The iCalendar library: calcard or ical-rs

Spec 007 R9 chose calcard over the *old* `ical-rs` for three reasons:

- calcard resolves Windows and Exchange zone names and zones sent with no
  VTIMEZONE;
- it types iTIP;
- it read all nine corpus invitations correctly.

R9 asked for `ical-rs` to be surveyed again. The rewrite changes the picture:

| Need | calcard 0.3 | ical-rs 0.7 |
|---|---|---|
| Parse every corpus invitation correctly | proven (R9 spike S1) | **to prove** (spike S1 here) |
| Windows and Exchange zone names | yes | **unknown**: the README names the IANA database only |
| Expand recurrence across zones | yes | yes |
| Byte-faithful edit of unmodelled properties (FR-014) | no (re-serialises) | **yes** |
| Three-way merge (FR-033) | no | **yes** |
| Time library | chrono (#2145: unmaintained) | **jiff** |
| Pimalaya family | no | yes |

**Decision, provisional**: spike S1 runs `ical-rs` over R9's corpus and the
new recurrence fixtures.

- If it matches calcard's instants, `postio-calendar` moves to `ical-rs`
  behind its existing adapter (no type crosses its edge), which also clears
  #2145 for this crate.
- If it fails only on Windows zone names, a small mapping table (CLDR's
  `windowsZones`) goes in `postio-calendar`, and `ical-rs` is still chosen.
- Otherwise calcard stays, and edits keep the raw object and patch only what
  Postio models.

**S1 verdict (2026-11-23): go for `ical-rs` 0.7.3.** A scratch crate expanded
R9's eight corpus invitations and T001's eight fixtures over 2026 to 2027 with
both libraries. All 16 gave the same instants, ends and floating flags:

- the New York weekly rule across the end of daylight time, with two EXDATEs
  and a moved occurrence;
- Chicago's EXDATEs;
- a floating daily rule;
- an all-day span;
- `Pacific Standard Time` with no VTIMEZONE (its `tzdb` feature carries CLDR's
  `windowsZones`, so no table of Postio's own is needed);
- `Europe/Berlin` and `Asia/Kolkata` with no VTIMEZONE.

Renaming SUMMARY in `unknown-properties.ics` changed that one line and no other
byte. `IcalMerge` reports each side's actions and their collisions, which is
FR-033's merge. So:

- `postio-calendar` moves to `ical-rs`, pinned exactly, with features `parser`
  and `tzdb`, and calcard leaves the workspace;
- `parse` and `reply` move with it, and the crate's public types are
  unchanged.

**Effect on #2145:**

- `ical-rs` is built on jiff 1.x, which leaves no chrono inside the calendar
  library, and adds jiff 1.x and `jiff-tzdb` to the graph;
- `postio-calendar`'s own edge stays on chrono (`EventTime`) until #2145
  chooses, because its callers are chrono today;
- if #2145 chooses jiff, the crate's edge moves with no library in the way;
- if it chooses `time`, the adapter converts at the edge.

Either way, one fewer crate holds chrono through a dependency.

### R3. CalDAV sync shape

- **Discovery**:
  1. use the preset row's `[caldav] url`, else RFC 6764;
  2. find the `current-user-principal`;
  3. find the `calendar-home-set`;
  4. list the calendars (display name, colour, `supported-calendar-component-set`,
     privileges for read-only, `schedule-*` for D6).
- **Initial sync**: `calendar-query` with a time range (a year back to two
  years ahead), then `calendar-multiget` in batches. The rest of history
  backfills in the background, as mail does (ADR 0016's shape).
- **Incremental sync**: `sync-collection` with the stored token. With no
  token support, compare ETags (`io-webdav`'s ETag-only listing).
- **Writes**: `PUT` with `If-Match: <etag>`, or `If-None-Match: *` for a
  create; `DELETE` with `If-Match`. A 412 fetches the server's copy and runs
  the three-way merge against the base kept in `event_changes`.
- **Polling**: on the mail sync's cadence while an app holds the store.
  Push (WebDAV-Push is a draft; Google channels are a webhook to a public
  URL) is out of scope.
- **Where it runs**: `postio-sync` drains the calendar queue through a
  `CalendarBackend` trait, never `io-webdav` types, as for `MailBackend`
  (ARCHITECTURE §8).

**S2 verdict (2026-11-23): go for `io-webdav` 0.13.0.** A scratch crate ran
`WebdavClientStd` over a `pimalaya-stream` connection, the stream type
`postio-jmap` already opens with the graph's TLS, against a minimal wire
server. Each step worked:

- `current_user_principal`, then `calendar_home_set`;
- `list_calendars`, with display name, colour, components, the sync token and
  the advertised reports. A VTODO-only collection is told apart by its
  component set;
- a time-ranged `calendar-query`, through `client.run(WebdavReport::new(..))`.
  The client has no method for it, and needs none;
- `multiget_items`, returning the object byte for byte with its ETag;
- `sync_items` with no token, with a token (changed and vanished), and with
  an expired token. The last surfaces as `InvalidSyncToken`, which leads to
  the full resync;
- `update_item` with `If-Match` answered 412, which surfaces as
  `HttpStatus { status: 412 }`;
- `create_item` with `If-None-Match: *`;
- `delete_item` with `If-Match`.

Three notes for slice C:

- **Read-only calendars.** The listing carries no
  `current-user-privilege-set`. Read-only is a raw PROPFIND through
  `client.run`, as the query is.
- **Async.** The client is blocking. It runs under `spawn_blocking` with a
  cancellable stream, as `postio-jmap` does.
- **A second io-http.** `io-webdav` 0.13 brings `io-http` 0.9 beside the
  workspace's 0.8. Both stay until the other users move.

The spike code is not merged.

### R4. Occurrences and paging

- **Expansion** happens in `postio-calendar`: a pure function of the event
  and a window. Results are materialised in `event_occurrences`
  (`event_id`, `start_utc`, `end_utc`, `all_day`, `recurrence_id`), indexed
  on `start_utc`.
- **The window** runs from a year back to two years ahead. Paging past it
  expands more, and the expansion is idempotent.
- Agenda pages by `(start_utc, event_id)` keyset, as the list pages by its
  sort key. Week reads one range. Both get counting assertions.
- **Floating times** are stored as local wall time with a flag, and resolved
  at read against the current zone. A zone change re-resolves them, and
  costs nothing for zoned events.

### R5. Providers as data

- `[provider.<id>.caldav]` holds:
  - `url`, a context root, or absent for RFC 6764;
  - `schedules` (bool, D6);
  - `oauth_scopes`, extra scopes asked for when calendars are turned on;
  - quirk flags, each with a comment.
- **iCloud**: `https://caldav.icloud.com/`, the account's app-specific
  password, server scheduling.
- **Fastmail**: `https://caldav.fastmail.com/`, the app password, server
  scheduling.
- **Google** (second, Q3 = A): `https://apidata.googleusercontent.com/caldav/v2/`.
  Google's guide (fetched 2026-11-22) says:
  - **OAuth is required**; Basic auth is refused;
  - a Cloud project with the CalDAV API enabled is required, so this means
    the person's own client or broker, as the row's `sources` say;
  - the scheduling inbox is always empty, and received invitations land
    directly in the events collection;
  - free/busy is not supported, and neither are the `AUDIO` alarm action,
    `MKCALENDAR`, VTODO or VJOURNAL.

  Whether CalDAV writes send invitations to attendees is undocumented. Spike
  S3 tests it against a test account. Until it is known, the Google row says
  `schedules = false`, so Postio sends iTIP mail itself.
- **Microsoft 365**: no CalDAV. Out of scope.

### R6. The editor's date input

`postio-search` already parses relative dates for search (`parse_when`) and
`postio-search::natural`. The editor reuses that grammar for start, end and
"Move to", so "thu 3pm" means the same thing in a search and in an event.
Durations ("1h", "until 4") join the same parser as new forms, with tests.

### R7. Reminders, only while an app holds the store (Q2 = A)

- The host keeps one timer, on the runtime, for the next due reminder. It is
  re-armed when `reminders` changes (an edit, a sync, a snooze).
- On start, the host reads reminders due in the last 12 hours that have not
  fired, and shows them once as one notification: "While Postio was closed:
  Standup at 09:30". They are then marked `missed`.
- Notifications go through the existing notifier (`postio-host/src/notify.rs`
  and each frontend's delivery). "Open" and "Snooze 5 min" are registry
  commands.
- There is no background mode, no autostart entry and no helper binary.
  ADR 0041 is unchanged.

### R8. Search over events (D9)

`postio-search` parses. A new executor arm in `postio-index` evaluates the
AST over event fields:

- free text against an FTS index of title, location, notes and attendee
  names (a new `event_search` table);
- `after:` and `before:` against occurrences;
- `with:` and `from:` against attendees and the organiser;
- `in:` against calendar names.

Operators with no event meaning return "not applicable" for the hint. The
100 ms local search budget applies.

### R9. Pop out (D12)

- **Precedent**: ADR 0034 already runs composers in windows of their own in
  the same process. Pop out reuses that window machinery, with a
  `postio-focus` "surface host" that a mode can be moved into and back out
  of, keeping its state (view, date, cursor, History).
- **Contexts per window**: the keymap resolves against the focused window's
  context stack. A window's controller owns its own History. `back` (`Esc`)
  at a popped-out window's root is a no-op.
- **Destination routing**: `go_to_*` commands look up which window shows the
  destination, and present that window. GTK's `gtk::Window::present` passes
  the key event's activation token (xdg-activation on Wayland), which GNOME
  honours for the focused app. A compositor that refuses the token shows its
  own "is ready" notice. That is a risk, not a failure.
- **Remembered**: a `settings` row, `calendar.window`, holding `{ popped,
  width, height }`. It is in the store, not `config.toml`, because it is
  state rather than preference (ADR 0024). No position is stored. Wayland does not let a
  client place its window, and on the Mac `NSWindow`'s frame autosave
  remembers position as Mac windows do.
- **General**: a surface declares `can_pop_out` in `postio-focus`. Calendar
  sets it, and Contacts sets it when Linux adopts Contacts. Folding
  `detach_composer` into `pop_out` is follow-up work, filed with
  `scripts/issue-file.sh` when slice A2 lands, not done here.

## Phasing

| Slice | What | Stories, FRs | Depends on |
|---|---|---|---|
| **S1 (spike, gates A)** | `ical-rs` against calcard over R9's corpus, new recurrence and zone fixtures, and byte-faithful edits (R2) | — | — |
| **S2 (spike, gates C)** | `io-webdav` against an in-process CalDAV server: discovery, sync token, ETag 412 | — | — |
| **S3 (spike, gates G)** | Whether Google CalDAV writes send invitations | — | — |
| **A. Local calendar and Agenda** (first) | Schema; `postio-calendar` expansion; "On this computer"; Agenda and the open event; `g e`; accepting an invitation writes the event; clash on the marker; search (R8) | US1, US2 (non-scheduling path); FR-001 to FR-006, FR-020, FR-022 (local), FR-035, FR-051 | S1 |
| **A2. Pop out** | `pop_out` and `pop_in`; destination routing; per-window History; remembered state (R9) | US7; FR-007 to FR-009 | A |
| **B. Management** | Create, edit, move, duplicate, delete with undo; recurring-scope picker; "Save and send" by iTIP mail; byte-faithful edits | US3 (local); FR-010 to FR-015 | A |
| **C. CalDAV sync** | `postio-dav` backend; presets' `[caldav]` for generic CalDAV, iCloud and Fastmail; discovery; sync; conflicts; the scheduling path for RSVP and invitations; `caldav` egress rows; Settings for calendars | US4, US2 (scheduling), US3 (synced); FR-021, FR-030 to FR-034, FR-052 | B, S2 |
| **D. Reminders** | Alarms; notifications with Open and Snooze; the missed-reminder catch-up (R7) | US5; FR-040 to FR-043 | A |
| **E. Week and Day** ⛔ handoff | The grids, and the narrow fallback | US6; FR-002 | the designer handoff |
| **F. Month** ⛔ handoff | The month grid | FR-002 | E, the handoff |
| **G. Google** | Its row, the scope, quirks | US4 | C, S3 |

**The first slice is A**, and **pop out is A2**, the next one. It needs only
Agenda, and the maintainer asked for it with Q1, so it comes before
management rather than waiting for the grids.

## Crate placement

| Crate | What it gains | Boundary it keeps |
|---|---|---|
| `postio-calendar` | Event model; recurrence expansion in a window; iTIP REQUEST, CANCEL and COUNTER builders beside `reply`; three-way merge; byte-faithful patching | **pure leaf**: no store, toolkit, async runtime or network (unchanged) |
| **`postio-dav`** (new, slice C) | `CalendarBackend` over `io-webdav`, and later CardDAV for spec 011 S12e. Its in-process test server is written against the wire | a protocol crate: no toolkit and no store engine. `postio-sync` sees only the trait |
| `postio-account` | Preset `[caldav]` schema and validation; `io-pim-discovery`'s `rfc6764` | as today |
| `postio-storage` | `calendars`, `events`, `event_occurrences`, `event_changes`, `reminders`, `event_search`; `egress_log.subsystem` gains `caldav` | — |
| `postio-sync` | The calendar queue drain and the poll loop | talks to `CalendarBackend`, never `io-webdav` |
| `postio-index` | The event executor arm (R8) | — |
| `postio-search` | Duration forms in the date grammar (R6) | pure leaf |
| `postio-core` | The Calendar context and the commands | no GTK |
| `postio-focus` | Calendar mode behaviour: views, cursor, `[`, `]`, `t`, History (ADR 0045). The surface host that pop out moves a mode into, and destination routing | — |
| `postio-ui` | View models: agenda rows, the grid layout as pure functions, the open event, every state's strings | no GTK, no SQL |
| `postio-host` / `postio-session` | Verbs; the reminder timer; RSVP routing (D6); the clash computation at filing | — |
| `postio-client` | Req/Resp for pages and verbs | no toolkit, engine or protocol |
| `postio-widgets`, `postio-gtk` | Agenda, the dialog, the editor; grids in slice E | as today |
| `postio-tui`, `postio-ffi`, `macos/` | Agenda and the open event. Pop out on the Mac, as an `NSWindow` with frame autosave. The terminal has no pop out | as today |

## Testing (ADR 0049)

- **`postio-calendar`** holds every recurrence, zone, merge and iTIP branch,
  over fixtures.
- **The wire server** holds sync, conflicts and discovery. Live servers run
  only as `#[ignore]`.
- **Counting**: Agenda and Week pages, the filing-time clash check, and
  search.
- **Fake clock**: reminders, snooze and the missed-reminder catch-up. No
  real-time waits.
- **Frontends**: one `focus_suite` case per path (`g e`, walk, open, `Esc`,
  `c` and save, `y` on an invitation). Storyboards in
  `storyboards/calendar/`, red on `main`, judged by `/ux-review`.

## Risks

| Risk | Effect | Mitigation |
|---|---|---|
| Recurrence and zone bugs | Wrong times are the worst calendar bug | S1 against a reference over many fixtures; SC-003 at 100% |
| Two replies to an organiser, or none | Social cost, lost meetings | D6's rule is one decision from the preset row; SC-005 asserted for both paths |
| Data loss in a sync conflict | Trust | `event_changes` keeps the base; the three-way merge or both versions are kept; SC-004 |
| Google's CalDAV quirks and OAuth | The provider with the most users lags | Google is second (Q3 = A); S3 settles scheduling |
| Reminders that do not fire while Postio is closed | A missed meeting | The maintainer chose this (Q2 = A). D8's catch-up shows what was missed, and Settings says reminders need Postio running |
| No design for the grids | UI churn | Q4 = C: Agenda is list-shaped and needs no new design; the grid tasks are held |
| A compositor refuses to raise the other window | `g e` seems to do nothing | The activation token from the key event; the compositor's own notice; a focus_suite case on the headless compositor |
| Two windows desynchronise | Stale views | Both read the same store and subscribe to the same event hub (ADR 0013); no state is copied between windows |
| `ical-rs` is new | Library churn | It stays behind `postio-calendar`'s adapter, and is pinned exactly as calcard is |

## Project Structure

### Documentation (this feature)

```text
specs/017-calendar/
├── spec.md
├── plan.md
├── data-model.md
├── contracts/
│   ├── calendar-mode.md     # keys, views, states, pop out
│   ├── event-verbs.md       # create, edit, move, delete, RSVP, undo
│   └── caldav-backend.md    # the CalendarBackend trait, presets, sync
├── quickstart.md
├── checklists/requirements.md
└── tasks.md
```

### Source Code (repository root)

```text
crates/postio-calendar/            # grows: model, expansion, iTIP, merge
crates/postio-dav/                 # new: CalendarBackend over io-webdav (slice C)
crates/postio-account/data/providers.toml   # [caldav] tables
crates/postio-storage/src/schema.rs
crates/postio-sync/src/
crates/postio-index/src/executor.rs
crates/postio-core/src/registry.rs
crates/postio-focus/src/
crates/postio-ui/src/calendar/
crates/postio-host/src/calendar/
crates/postio-widgets/src/calendar/
crates/postio-gtk/src/
crates/postio-tui/src/view/
crates/postio-ffi/src/
storyboards/calendar/
```

**Structure Decision**: the engine-plus-frontends layout of
`docs/ARCHITECTURE.md`. One new protocol crate sits behind a trait, as the
mail backends do.

## Complexity Tracking

| Addition | Why needed | Simpler alternative rejected because |
|---|---|---|
| New crate `postio-dav` | CalDAV (and later CardDAV) is protocol code, which constitution VII holds at arm's length behind a trait | Putting it in `postio-account` would put an HTTP and WebDAV stack in the same crate as mail's protocol, and blur §8's seam |
| `event_occurrences` materialised | Paging must be bounded and independent of store size (constitution V) | Expanding at read time makes every Agenda page scan every recurring master |
