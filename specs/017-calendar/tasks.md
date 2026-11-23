---
description: "Task list for spec 017, calendar"
---

# Tasks: Calendar

**Input**: `specs/017-calendar/`: [spec.md](spec.md), [plan.md](plan.md)
(Phase 0 research is R1 to R9), [data-model.md](data-model.md),
[contracts/](contracts), [quickstart.md](quickstart.md)

**This file is the tracker.** These tasks get no issues (CLAUDE.md,
spec-driven work). Tick a box in the commit that finishes it. The branch
lands once, with `scripts/issue-land.sh --detach --full-suite --title
'feat: a calendar beside mail'`. It is not yet scheduled.

**Tests are required** (constitution IV). Each implementation task follows
the task that writes its test, and that test task names what is seen red. A
test that is green on arrival is a guard, and says so.

## Conventions

- `[P]`: the task touches different files and depends on no unfinished
  task. `[USn]`: the spec's user story.
- `⛔ S1`, `⛔ S2`, `⛔ S3`: the task waits on that spike's "go". `⛔ handoff`:
  held until the designer handoff for the grid views arrives (Q4 = C). A held
  task is not started, and is not deleted.
- **Commits** end with `Refs: specs/017-calendar` and the task ids. Never
  write a closing keyword.
- **Code comments follow CLAUDE.md's rules.** A comment is a why the code
  cannot show, in at most three lines. It carries **no FR, task, slice or
  decision ids** and no narrative; those belong in the commit body. A bare
  `(ADR 0034)` or `(#2145)` after a why is allowed. Prefer a test name or an
  assert message to a comment. Test files get no `//!`. `POSTIO-CONSENT:`
  lines stay.
- **The fast tier carries most of this.** Run `scripts/test-fast.sh` between
  edits, and `cargo nextest run -p <crate> --test <suite> <case>` to confirm.
- **No test waits on the clock**: reminders use
  `#[tokio::test(start_paused = true)]`.
- **Swift** tasks run on the Mac (`macos/CLAUDE.md`).
- **Storyboards** are seen red on `main` before they are built, and
  `/ux-review` runs before landing.
- **Fixtures** use `example.com` and invented names. No test reaches a real
  server; live checks are `#[ignore]`.
- **Traps** go in `docs/gotchas/<area>.md` (`sync.md` for CalDAV, `gtk.md` for
  windows).

---

## Phase 1: Setup

- [x] T001 [P] Add calendar fixtures under `crates/postio-calendar/tests/fixtures/` (`.ics`, synthetic, `example.com`), each listed in a `README.md` there:
  - a weekly rule with an exception and EXDATEs across a DST change;
  - an all-day multi-day event;
  - a floating time;
  - a Windows zone name;
  - a zone with no VTIMEZONE;
  - unknown `X-` properties and a VTODO;
  - an `EMAIL` alarm;
  - hostile URLs in `DESCRIPTION`, `ATTACH` and `CONFERENCE`.
- [x] T002 [P] Write `storyboards/calendar/agenda-walk.toml` and `storyboards/calendar/pop-out.toml` from `contracts/calendar-mode.md`, and see both red on `main` with `scripts/storyboards.sh`

## Phase 2: Spikes (gating)

Each spike ends with a one-paragraph verdict, "go" or "no-go", appended to
the plan's research section it settles. Spike code is not merged.

- [x] T003 **S1, the iCalendar library (gates Phase 4 onward).**
  - In a scratch crate, run Pimalaya's `ical-rs` 0.7 and calcard 0.3.14 over spec 007 R9's corpus invitations and T001's fixtures.
  - Compare instants (expansion over two years), Windows zone names, byte-faithful edits of an unmodelled property, and three-way merge.
  - **Go for `ical-rs`** if its instants match. With Windows names missing, it is still a go, with a CLDR `windowsZones` table. Otherwise calcard stays.
  - Record the choice in plan R2, with its effect on #2145.
- [x] T004 [P] **S2, CalDAV through `io-webdav` (gates Phase 8).** Against a minimal wire server in a scratch crate, run:
  - discovery (current-user-principal, then home set);
  - `calendar-query` and `calendar-multiget`;
  - `sync-collection` with a token, and an expired token;
  - `PUT` with `If-Match` answered 412.

  **Go** when each works over `pimalaya-stream` with the graph's existing TLS. Record it in plan R3.
- [ ] T005 [P] **S3, Google CalDAV scheduling (gates Phase 11).** With a test Google account and the person's own OAuth client (never CI), create an event with an attendee over CalDAV and see whether Google sends the invitation. Record `schedules` for the Google row in plan R5.

## Phase 3: Foundational (blocks every story)

- [x] T006 Write failing cases in `crates/postio-core/tests/core_suite/`:
  - `Context::Calendar` and `Context::CalendarWindow`;
  - `contracts/calendar-mode.md`'s and `contracts/event-verbs.md`'s commands, not held, each with a key, a command-bar row and an accessible action;
  - no collision within a context;
  - `pop_out` and `pop_in` on one chord in different contexts.
  - `quit` on `mod+q` alone and `close_window` on `mod+w` in every context (Q5).
- [x] T007 Add the contexts and commands to `crates/postio-core/src/context.rs`, `crates/postio-core/src/command.rs` and `crates/postio-core/src/registry.rs`. Regenerate `docs/keybindings.md` (`POSTIO_UPDATE_DOCS=1 cargo test -p postio-ui`) and `crates/postio-core/tests/golden/linux-bindings.txt`. Mark `pop_out` and `pop_in` not offered in the terminal (`registry::offered_on`). Drop `quit`'s `mod+w` alternate and add `close_window` on `mod+w`, with the Apple exception for `quit` on `mod+w` removed from `registry.rs` (Q5)
- [x] T008 Write failing schema cases in `crates/postio-storage/tests/storage_suite/`, then add `calendars`, `events`, `event_occurrences`, `event_changes`, `reminders`, `event_search` and the `caldav` value of `egress_log.subsystem` to `crates/postio-storage/src/schema.rs` (`data-model.md`). Add a new `crates/postio-storage/src/repository/calendar.rs`, and create "On this computer" at store creation.

## Phase 4: User Story 1, see what is coming up (P1) 🎯 first slice ⛔ S1

**Goal**: `g e` opens Agenda over the local store.
**Independent test**: quickstart "Slice A".

- [x] T009 [P] [US1] Write failing cases in `crates/postio-calendar/src/expand.rs` over T001's fixtures: instants across DST, exceptions, EXDATEs, all-day, floating, idempotence over a window
- [x] T010 [US1] Implement `Event`, `parse_object` and `expand` in `crates/postio-calendar/src/` on S1's library, behind the crate's adapter. No library type crosses its edge.
- [x] T011 [US1] Write failing counted cases in `crates/postio-host/tests/host_suite/calendar_agenda.rs`:
  - a page is a fixed number of statements at 50,000 occurrences;
  - keyset paging;
  - expansion past the window.

  Then implement the agenda read and the occurrence materialisation in a new `crates/postio-host/src/calendar/`, and `Req::AgendaPage` and `Resp::AgendaPage` in `crates/postio-client/src/protocol.rs` and `crates/postio-client/src/api.rs`.
- [x] T012 [P] [US1] Write failing cases in `crates/postio-ui/tests/ui_suite/calendar_agenda.rs` for `AgendaRow`, `EventView` and every state string in the spec's "Every state" table, then implement them in a new `crates/postio-ui/src/calendar/`
- [x] T013 [US1] Write failing behaviour cases in a new `crates/postio-focus/src/calendar.rs`:
  - `g e` opens on today, with the cursor on the next event that has not ended;
  - `j`, `k`, `[`, `]`, `t`, `Return`;
  - `Esc` back to the same row.

  Then implement them, and wire them in `crates/postio-focus/src/surfaces.rs`.
- [x] T014 [US1] GTK: Agenda and the open event in a new `crates/postio-widgets/src/calendar/`, in the existing list language (`/gtk-design`, agent-designed per Q4 = C), with one `crates/postio-gtk/tests/focus_suite/calendar.rs` case per path. T002's `agenda-walk` goes green
- [x] T015 [P] [US1] Terminal: Agenda and the open event in a new `crates/postio-tui/src/view/calendar.rs`, with cases in `crates/postio-tui/tests/tui_suite/`
- [ ] T016 [P] [US1] Mac: Agenda through `crates/postio-ffi/src/` and `macos/Sources/PostioKit/`, with `macos/Tests/PostioKitTests/` cases. `crates/postio-ffi/tests/ffi_suite/command_coverage.rs` stays green
- [ ] T017 [US1] Write failing cases in `crates/postio-index/` and `crates/postio-host/tests/host_suite/` for event search (plan R8): free text, `after:`, `before:`, `with:`, `in:`, and an operator with no event meaning giving the hint. Then implement the executor arm and `event_search`.

## Phase 5: User Story 2, invitations with clashes (P1)

**Goal**: the Invite marker names clashes, and accepting writes the event.
**Independent test**: US2 scenarios 1, 3, 4 and 5.

- [ ] T018 [US2] Write failing cases in `crates/postio-host/tests/host_suite/calendar_invitations.rs`:
  - a filed invitation overlapping a local occurrence gets `Clashes`;
  - `y` writes `my_partstat` and the event, and queues one REPLY, as today;
  - `mod+z` in the window sends nothing, and removes the event;
  - an update moves the event, with `Moved`.

  Then implement them in `crates/postio-host/src/calendar/` and the marker code in `crates/postio-host/src/focus/`.
- [ ] T019 [US2] Add `tentative_invite` (`alt+y`) beside `accept_invite` in `crates/postio-calendar/src/reply.rs` and the RSVP path, with cases
- [ ] T020 [US2] Draw the new chips in each frontend's marker (`crates/postio-widgets/src/`, `crates/postio-tui/src/view/list.rs`, `macos/Sources/PostioKit/`), with one frontend case each

## Phase 6: User Story 7, pop out (P2)

**Goal**: Calendar in its own window and back, with the keyboard in the
focused window.
**Independent test**: quickstart "Slice A2"; US7's scenarios.

- [ ] T021 [US7] Write failing cases in `crates/postio-focus/src/` for the surface host:
  - `pop_out` moves `CalendarMode` unchanged;
  - `g e` and the mail destinations route to the window that shows them;
  - each window has its own History;
  - `Esc` at a window's root is a no-op;
  - `pop_in` and closing return the mode.
  - `close_window` pops Calendar in from its window, closes a detached composer, and in the main window does what its X does.

  Then implement a new `crates/postio-focus/src/surface_host.rs`.
- [ ] T022 [US7] Write a failing case in `crates/postio-host/tests/host_suite/` for the `calendar.window` settings row (`{ popped, width, height }`, no position), then implement it
- [ ] T023 [US7] GTK: the Calendar window on the window machinery ADR 0034's detached composers use, in `crates/postio-gtk/src/`. `present()` the target window with the key event's activation token. Add `crates/postio-gtk/tests/focus_suite/calendar_pop_out.rs`, one case per row of `contracts/calendar-mode.md` that pops out. T002's `pop-out` goes green. Note any compositor focus trap in `docs/gotchas/gtk.md`
- [ ] T024 [P] [US7] Mac: an `NSWindow` with frame autosave in `macos/Sources/PostioKit/`, with `macos/Tests/PostioKitTests/` cases
- [ ] T025 [US7] Run `/ux-review` on both storyboards, and fix or contest the verdicts
- [ ] T026 [US7] File the follow-up that folds `detach_composer` into `pop_out`, with `scripts/issue-file.sh` (plan R9)

## Phase 7: User Story 3, management (P1)

**Goal**: create, edit, move, duplicate and delete, local-first and undoable.
**Independent test**: quickstart "Slice B"; US3 scenarios 1 to 4.

- [ ] T027 [P] [US3] Write failing cases in `crates/postio-calendar/src/` for `patch` (byte-faithful edits over the `X-` and VTODO fixtures), the three recurrence scopes, and `request` (REQUEST, CANCEL), then implement them
- [ ] T028 [US3] Write failing cases in `crates/postio-host/tests/host_suite/calendar_verbs.rs`, one per row of `contracts/event-verbs.md`'s rules:
  - store write and `event_changes` before the repaint, with no connection before it;
  - undo;
  - the scope picker;
  - `save_event` sends nothing;
  - `save_and_send_event` after the window.

  Then implement the verbs in `crates/postio-host/src/calendar/`.
- [ ] T029 [US3] Extend the date grammar with durations ("1h", "until 4") in `crates/postio-search/src/date.rs`, with unit cases, for the editor (plan R6)
- [ ] T030 [US3] The event editor dialog in `crates/postio-widgets/src/calendar/`, the terminal and the Mac, with one frontend case per path (`c`, type, `mod+s`; `v` "Next week"; `#` then `z`)

## Phase 8: User Story 4, CalDAV sync: generic, iCloud, Fastmail (P2) ⛔ S2

**Goal**: an account's calendars sync both ways.
**Independent test**: quickstart "Slice C"; US4 scenarios 1 to 3; US3
scenario 5.

- [ ] T031 [US4] Create the crate `crates/postio-dav/` (the `CalendarBackend` trait, the fake, and `io-webdav`), with a `check-crate-boundaries.py` entry that keeps the store engine and toolkit out, and `postio-sync` free of `io-webdav` types. See the check fail on a deliberate `postio-storage` dependency, then remove it
- [ ] T032 [US4] Write the in-process wire server and its suite, `crates/postio-dav/tests/dav_suite/`, from `contracts/caldav-backend.md`'s table, red first. Then implement the backend.
- [ ] T033 [US4] Add the `[caldav]` schema to `crates/postio-account/src/discovery/providers_toml.rs` with validation, the rows for iCloud and Fastmail to `crates/postio-account/data/providers.toml`, and `io-pim-discovery`'s `rfc6764` feature in `crates/postio-account/Cargo.toml`, with `account_suite` cases
- [ ] T034 [US4] Write failing cases in `crates/postio-sync/tests/sync_suite/calendar.rs` against the fake:
  - the first sync windowed, then the rest;
  - incremental by token;
  - 412, merge or conflict;
  - 403, failed;
  - `egress_log` rows with `caldav`.

  Then implement the queue drain and the poll in `crates/postio-sync/src/`.
- [ ] T035 [US4] Add the scheduling path to RSVP and invitations: with `calendars.schedules`, a PUT and no mail. Add the case to `crates/postio-host/tests/host_suite/calendar_invitations.rs`
- [ ] T036 [US4] Settings: turn on an account's calendars, show or hide each, the default reminder, "Enter a calendar address". Do this in the settings model in `crates/postio-ui/src/` and each frontend, with cases. Show the conflict state with "Keep this one" for each version.

## Phase 9: User Story 5, reminders (P2)

**Goal**: alarms notify while an app holds the store, and missed ones show
once.
**Independent test**: quickstart "Slice D".

- [ ] T037 [US5] Write failing fake-clock cases in `crates/postio-host/tests/host_suite/calendar_reminders.rs`:
  - display and audio alarms notify;
  - email alarms do nothing;
  - snooze;
  - the missed catch-up under 12 hours, shown once;
  - nothing over 12 hours.

  Then implement the reminder timer and the `reminders` maintenance in `crates/postio-host/src/calendar/`.
- [ ] T038 [US5] Deliver through the existing notifier (`crates/postio-host/src/notify.rs` and each frontend's delivery), with Open and Snooze as `open_reminder` and `snooze_reminder`, and one frontend case each. Settings states that reminders need Postio running

## Phase 10: User Story 6, Day, Week and Month (P3) ⛔ handoff

Every task here is held for the designer handoff (Q4 = C).

- [ ] T039 [US6] ⛔ handoff Add the grid layout as pure functions in `crates/postio-ui/src/calendar/grid.rs` (columns, overlaps, all-day row, the narrow fallback), with unit cases
- [ ] T040 [US6] ⛔ handoff Add Day and Week behaviour (`2`, `3`, `j` and `k` through events in time order) to `crates/postio-focus/src/calendar.rs`, and offer `calendar_day` and `calendar_week` in the registry
- [ ] T041 [US6] ⛔ handoff Draw Day and Week in `crates/postio-widgets/src/calendar/` and on the Mac, to the handoff, with `focus_suite` cases and a storyboard
- [ ] T042 [US6] ⛔ handoff Month (`4`): layout, behaviour and drawing, as T039 to T041 did for the other grids

## Phase 11: Google (second sync slice, P2) ⛔ S3

- [ ] T043 [US4] ⛔ S3 Add the Google `[caldav]` row (with `oauth_scopes`, and `schedules` from S3) to `crates/postio-account/data/providers.toml`. Request the scope incrementally when calendars are turned on, in `crates/postio-account/src/oauth/`. Add quirks as data (no `AUDIO` alarms, no VTODO), with cases on the wire server

## Phase 12: Polish and landing

- [ ] T044 [P] Add the hostile event fixtures' zero-connection test to `crates/postio-host/tests/host_suite/calendar_privacy.rs` (spec SC-007)
- [ ] T045 [P] Update `docs/ARCHITECTURE.md` (the diagram and §9 for `postio-dav`) and `docs/PRODUCT.md` (a calendar section, and §17 for reminders)
- [ ] T046 Run `/ux-review` on every calendar storyboard, then land with `scripts/issue-land.sh --detach --full-suite`

## Dependencies and order

```text
T001-T002 ─▶ T003 S1 ─▶ Phase 3 ─▶ Phase 4 (US1) ─┬─▶ Phase 5 (US2)
                                                  ├─▶ Phase 6 (US7 pop out)
                                                  ├─▶ Phase 7 (US3) ─▶ Phase 8 (US4) ⛔ S2 ─▶ Phase 11 ⛔ S3
                                                  └─▶ Phase 9 (US5)
                                    Phase 10 (US6) ⛔ handoff, after Phase 4
```

- Phases 5, 6, 7 and 9 each depend only on Phase 4, and can run in any order.
- Phase 8 needs Phase 7 and S2. Phase 11 needs Phase 8 and S3.
- Phase 10 is held.

## Parallel examples

- Phase 2: T004 and T005 beside T003.
- Phase 4: T009 and T012, then T015 and T016 once T011 is in.
- After Phase 4: Phase 5, Phase 6 and Phase 9 in parallel lanes.

## Implementation strategy

1. **MVP is Phase 4 plus Phase 5**: a local calendar where accepted
   invitations land, and clashes show in the inbox. No network.
2. Then pop out (Phase 6), which the maintainer asked for.
3. Then management (Phase 7), then sync (Phase 8), then reminders (Phase 9).
4. The grids follow the handoff, and Google follows S3.
