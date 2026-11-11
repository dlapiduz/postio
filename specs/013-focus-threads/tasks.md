---
description: "Task list for spec 013, threads in Postio Focus on the Mac and Linux"
---

# Tasks: Threads in Postio Focus, Mac and Linux

**Input**: `specs/013-focus-threads/`: [spec.md](spec.md), [plan.md](plan.md),
[research.md](research.md), [data-model.md](data-model.md),
[contracts/](contracts), [quickstart.md](quickstart.md)

**This file is the tracker.** These tasks get no issues (CLAUDE.md,
spec-driven work). Tick a box in the commit that finishes it.

**Tests are required** (constitution IV). Each implementation task comes
after the task that writes its test, and that test task names what is seen
red. A test that is green on arrival is a guard pinning behaviour that must
not change, and it says so.

## Conventions

- `[P]` means the task can run in parallel with others: it touches different
  files and depends on no unfinished task. `[USn]` is the spec's user story.
- **Each phase ends runnable on both platforms and ends with a commit.**
  Commits end with `Refs: specs/013-focus-threads` and the task ids. Never
  write a closing keyword. `#2099` is closed by the PR's `Closes` line.
- **On this Mac**: Rust changed → `scripts/macos-build.sh --lib-only`. Swift
  changed → `cd macos && swift build`. Swift tests → `scripts/macos-test.sh`
  (`macos/CLAUDE.md`).
- **GTK does not build on this Mac.** Write the GTK change and its
  `focus_suite` case together, grep every changed shared name's call sites
  (`grep -rn "<name>" crates/postio-gtk`), and let CI's `Frontends` check or
  the Linux box prove it. See
  `docs/notes/2026-09-06-moving-code-out-of-a-crate-you-cannot-compile.md`.
  The Linux box takes **at most two agents** and no back-to-back full-suite
  loops.
- **Screens** (FR-027):
  - Mac: `scripts/macos-shot.sh <nn>-<slug> --seed threads --size 1440x900
    --light` (`--dark` for 02→11), with `POSTIO_DEMO_KEYS` to reach the
    state.
  - Linux: `scripts/run-isolated.sh <commit> --shot` on the Linux box.
  - Compare against `~/src/postio/Design/focus-macos-threads/screens/<nn>-*.png`
    and list every difference in the step's note,
    `docs/notes/2026-11-<dd>-focus-threads-step-<n>.md`, each one fixed or
    explained. Never copy the PNGs in.
- **Storyboards** (ADR 0044): each keyboard path is written in
  `storyboards/threads/` and seen red on `main` before it is built.
  `/ux-review` runs before landing.
- **Keys are the registry's** (spec D1, T2, T3), never the screens'. Labels
  and keycaps come from `postio_ui::hints`.
- **Logs** carry ids and counts only (FR-029).

---

## Phase 1: Setup

**Purpose**: the demo store, fixtures and storyboard folder every later phase
uses.

- [x] T001 [P] Add `Seed::Threads` to `crates/postio-demo/src/lib.rs`, seeded through `ThreadingRepository` (research R14):
  - "Harbor API draft v3": six messages, two unread; message 4 answers message 1; message 5 has a PDF; plus an unsent draft reply.
  - A 23-message thread, a 41-message list thread, single messages, and the "rate limit headers" search fixtures for screens 06/07.
  - Invented names on `example.com`/`example.net` only.
  - Test in `crates/postio-demo/src/lib.rs`'s tests: the seed opens and its thread counts are as listed (red: no such seed).
- [x] T002 [P] Add threading fixtures with `/add-fixture` under `crates/postio-model/tests/corpus/`:
  - a fork;
  - a reply filed in Archive while its parent is in the inbox;
  - a sent reply;
  - two unrelated "Re: Hello" 30 days apart;
  - two "Re: Hello" 3 days apart with no references;
  - a weekly newsletter with one subject (`List-Id`);
  - a list discussion with references.

  Each gets its loader row, categories and README row.
- [x] T003 [P] Create `storyboards/threads/README.md` naming the paths this spec films (one per acceptance scenario that involves a key), in spec 008's format.
- [x] T004 Register `--seed threads` in `scripts/macos-shot.sh` and the GTK demo's seed parser (`crates/postio-gtk/src/demo.rs`), with `POSTIO_DEMO=threads` working on both.

---

## Phase 2: Foundational (blocks every story)

**Purpose**: grouping rules, the own line, the `Thread` read, and the scope
and splice rules every frontend story draws through.

### Grouping (research R2)

- [x] T005 [P] Write failing `assign` cases in `crates/postio-model/src/threading.rs` tests (red: today two same-subject threads merge, an 8-day gap joins, and a `List-Id` message joins by subject):
  - the subject fallback joins the newest thread within 7 days and never merges;
  - an 8-day gap starts a new thread;
  - `is_list` skips the fallback.
- [x] T006 Implement `ThreadCue.date`/`is_list`, `SUBJECT_WINDOW`, and `threads_with_subject(subject, since) -> Option<ThreadId>` in `crates/postio-model/src/threading.rs`, with `assign` joining only.
- [x] T007 Write a failing storage case in `crates/postio-storage/tests/storage_suite/threading.rs` over T002's corpus (red: the two "Re: Hello" 30 days apart and the newsletter group together). Add a count assertion that the bounded subject lookup costs today's statements. Assert rows materialised and trigger firings too, not just statements (constitution V).
- [x] T008 Bound the subject lookup in `crates/postio-storage/src/repository/threading.rs` (`LoadedIndex::load`, `last_at >= ?`), and add the migration with `idx_threads_subject_recent` in `crates/postio-storage/src/schema.rs` plus its fingerprint under `crates/postio-storage/tests/schemas/`.

### Own line (research R5)

- [x] T009 [P] Write failing tests for `own_line(body)` in `crates/postio-body/src/own_text.rs` (red: no function):
  - a top-posted reply gives its first sentence;
  - an all-quote message gives `""`;
  - a long line is cut to `PREVIEW_CHARS` with an ellipsis.
- [x] T010 Implement `own_line` in `crates/postio-body/src/own_text.rs`.
- [x] T011 Write a failing host case: once a body lands, `messages.own_line` is set (red: the column does not exist). Use the body-stage test file beside `crates/postio-host/src/focus/rules.rs`.
- [x] T012 Add `messages.own_line` to the T008 migration and `MessageRepository::set_own_line` in `crates/postio-storage/src/repository/messages.rs`, and write it from the Focus body stage in `crates/postio-host/src/focus/`.

### The thread read (research R6, engine contract)

- [x] T013 Write failing storage tests in `crates/postio-storage/tests/storage_suite/threads.rs` (red: no `members`):
  - `ThreadRepository::members` returns date order;
  - `answers` is set only when `In-Reply-To` names a member that is not the previous one;
  - the draft is found by `drafts.thread_id`;
  - **the statement count is equal at 2 and 500 members** (`postio_storage::test_support::counting`). Assert rows materialised and trigger firings at both sizes as well (constitution V).
- [x] T014 Implement `ThreadRepository::members` and `ThreadMember`/`Line` in `crates/postio-storage/src/repository/threads.rs` (`Line::Own` from `own_line`, `Line::Quoted` for `''`, `Line::Preview` from `preview` cut at `quote::without_attribution`).
- [ ] T015 Add `Req::Thread`/`Resp::Thread`, `ThreadView`, `ThreadDraft` and `Client::thread` in `crates/postio-client/src/protocol.rs` and `crates/postio-client/src/api.rs`, answered in `crates/postio-host/src/lib.rs`, with `Req::family()` naming it. Test in `crates/postio-ffi/tests/ffi_suite/focus_threads.rs` over `Seed::Threads`: Harbor's view has six members, message 4 has `answers = Some(0)`, and the draft is last (red first).
- [ ] T016 Add `Event::ThreadChanged { thread, members_added }` in `crates/postio-core/src/event.rs`, emitted after a message is threaded on arrival (`crates/postio-sync/src/initial.rs`, `resync.rs`, `send.rs`). Test it in the host suite (red: no event).

### Scope and splice (research R7, R9)

- [x] T017 [P] Write `crates/postio-ui/src/scope.rs` tests first: one case per cell of spec D2's table, including `Unavailable` for `alt+v`/`alt+BackSpace` on a row, flag→latest on a row, and read/unread→thread on a row (red: no module).
- [x] T018 Implement `ScopeFocus`, `Scope`, `target` and `footer` in `crates/postio-ui/src/scope.rs`. Export them from `crates/postio-ui/src/lib.rs`.
- [x] T019 [P] Write `crates/postio-ui/src/thread_list.rs` tests first (red: no module):
  - `slot`/`display`/`len` round-trip around an anchor at the top, middle and end;
  - zero children;
  - an anchor beyond the landed pages.
- [x] T020 Implement `Splice` and `Slot` in `crates/postio-ui/src/thread_list.rs`.
- [x] T021 Route `postio_ui::focus_target::aim_by` through `scope::target` in `crates/postio-ui/src/focus_target.rs`. Existing focus_target tests are guards and stay green, except the flag-on-row case, which changes on purpose: update it with a comment citing spec D2.

### Registry (research R10)

- [x] T022 Write failing registry tests in `crates/postio-core/src/registry.rs` (red: unknown ids):
  - each new id's default key and contexts;
  - `Right`/`Left` absent from `open_message`/`prev_view` in List, Results and Filtered view;
  - `archive_thread` gone;
  - no clash in any context. Assert explicitly that `alt+e` (`reply_to_latest`) does not clash with `edit_externally`'s `alt+e` alternate in `Context::Composer` (`registry.rs:1613`); if it does, drop that alternate (it keeps `mod+shift+e`). Decide here whether the outline and forward chooser get `Context::Outline`/`Context::ForwardChooser` or reuse the picker context, and record the choice in data-model.md.
- [x] T023 Add the commands in `crates/postio-core/src/command.rs` and `crates/postio-core/src/registry.rs`:
  - `ExpandThread`, `CollapseThread`, `FirstInConversation`, `LatestInConversation`, `FirstUnreadInConversation`, `ThreadOutline`, `ThreadMenu` (`alt+period`), `ReplyToLatest`, `MuteThread`, `SplitThread` (`alt+shift+s`), `MoveThisMessage`, `DeleteThisMessage`, `ForwardWhat1..3`, `ShowNewer` and `ExpandQuoted`;
  - `[`/`]` as defaults of `Prev`/`NextInConversation`;
  - delete `ArchiveThread` and its uses (`crates/postio-ui/src/focus_target.rs` `dispatch`, the TUI, GTK, `crates/postio-ffi/src/registry.rs`, `macos/Sources/PostioKit/Accessibility.swift`). Confirm that a `[keys] archive_thread` left in `config.toml` is warned about and ignored, not fatal; if it is fatal, make it a warning in the same commit.
- [ ] T024 Write a failing host case (red: the snoozed thread stays hidden) beside the filing stage in `crates/postio-host/src/focus/`. A reply arriving in an archived thread puts it back in the inbox, and one arriving in a snoozed thread ends the snooze (`snoozed_until` cleared on its members). Neither copies labels to the new message. Then implement the snooze clearing in the filing stage (D3, FR-004, research R1).

  Regenerate `docs/keybindings.md`.

**Checkpoint**: `scripts/test-fast.sh` is green and the GTK call sites have been grepped. Commit as step 1 (`Refs: specs/013-focus-threads T001–T024`).

---

## Phase 3: User Story 1: see every message in a thread from the list (P1) 🎯 MVP

**Goal**: collapsed thread rows (screen 01), and expanding one in place with
child rows (screens 02, 11).

**Independent test**: `Right` on Harbor, then `j` four times. Each child
shows its own text, and `Return` opens message 4.

### Tests (red first)

- [ ] T025 [P] [US1] Write the words tests in `crates/postio-ui/src/focus_row.rs` (red: no functions):
  - `participants` ("Lena, Ben, Ana, you"; "+2"; a single message gives the full name);
  - `latest_line` ("Ben: Looks good…");
  - `new_badge` ("2 new");
  - `child_tag` ("↳ to Ana");
  - `open_at` (first unread, then open marker, then latest).
- [ ] T026 [P] [US1] Write `crates/postio-focus/tests/controller_suite/threads.rs` cases (red: no expansion):
  - `ExpandThread` emits `Splice` with six children and a draft;
  - expanding B collapses A;
  - `j`/`k` walk the children;
  - `CollapseThread` from a child returns the cursor to the row;
  - `Right` on a single-message thread does nothing;
  - the anchor leaving the list unsplices and moves the cursor down;
  - `ThreadChanged` while expanded keeps the focused child;
  - `Return` on a row opens `open_at` and on a child opens that child.
- [ ] T027 [P] [US1] Write storyboards `storyboards/threads/expand-collapse.*` and `storyboards/threads/open-child.*`, red on `main`.
- [ ] T028 [P] [US1] Write a test in `crates/postio-ffi/tests/ffi_suite/focus_threads.rs`: after `ExpandThread` on `Seed::Threads`, Swift receives `ChildRowFfi`s with own lines, the "↳ to Ana" tag and a draft row.

### Implementation

- [ ] T029 [US1] Implement the T025 functions in `crates/postio-ui/src/focus_row.rs`, and the thread row's `ThreadRowView` (participants, count, subject, labels, latest line, new, attachment, time, chevron).
- [ ] T030 [US1] Add `Expansion`, `Request::ReadThread`, `Input::ThreadRead`/`ThreadChanged`, and `Intent::Splice`/`Unsplice` to `crates/postio-focus/src/lib.rs`. Wire `perform` in `crates/postio-focus/src/perform.rs`. Make the cursor read through `Splice` in `crates/postio-focus/src/cursor.rs` (`Rows` positions become display positions). Add a count guard: a list page with the new row fields costs today's statements, rows and trigger firings (SC-003; mute's +1 is T061's).
- [ ] T031 [US1] Add `ChildRowView` to `crates/postio-ui/src/thread_list.rs` (own line or "Quoted reply only", tag, attachment, time, draft styling, focused hint keys from `postio_ui::hints`).
- [ ] T032 [US1] FFI: `crates/postio-ffi/src/focus_threads.rs` (new; `ChildRowFfi`, `ThreadRowFfi`), splice events in `crates/postio-ffi/src/focus_list.rs`, and `ChevronClicked` input.
- [ ] T033 [US1] Mac: thread row columns (34 gutter, 230 participants, flexible subject, badges, 58 time) and the chevron in `macos/Sources/PostioKit/FocusList.swift`. Add `macos/Sources/PostioKit/ChildRow.swift` (38 tall, tinted, 1.5 pt line, dot, 20 pt avatar). Do row insertion and removal in `macos/Sources/PostioAppKit/FocusListTable.swift`, with no animation under Reduce Motion. Swift test: focus stays on the row after expand and returns to it after collapse.
- [ ] T034 [US1] GTK: the thread row and child row widgets, and the list model splice, in `crates/postio-gtk/src/list/` and `crates/postio-gtk/src/window.rs`, styled per `/gtk-design` with spec 007's metric roles. Add `crates/postio-gtk/tests/focus_suite/threads.rs` cases for expand, walk, collapse and open child, with real keys. No transition when animations are off (`gtk-enable-animations=false`).
- [ ] T035 [US1] Draw thread rows and expansion in the Filtered view and folder lists too (FR-009): Mac `macos/Sources/PostioKit/FilteredView.swift` and the folder list's table, GTK `crates/postio-gtk/src/filtered.rs`. Add one controller case per surface in `crates/postio-focus/tests/controller_suite/threads.rs` (red: `ExpandThread` is ignored there) and a `focus_suite` case for Filtered.
- [ ] T036 [US1] Screens 01, 02 and 11 (dark) on both platforms. Write the differences in `docs/notes/2026-11-<dd>-focus-threads-step-2-3.md`.

**Checkpoint**: #2099's reproduction passes on both platforms.

---

## Phase 4: User Story 2: every action says what it acts on (P1)

**Goal**: the scope-aware footer and the hint on the focused child (screen
02's footer). Verbs resolve per spec D2.

**Independent test**: on a child, `r` quotes that message, `e` archives the
thread, and `z` brings it back.

- [ ] T037 [P] [US2] Write controller cases in `crates/postio-focus/tests/controller_suite/threads.rs` (red: the reply aims at the row's latest; the footer has no scope):
  - on a child, `Reply` aims `Messages([child])`;
  - `Archive` aims `Threads([thread])`;
  - `MoveThisMessage` on a row is refused;
  - the footer is "Message 4 of 6 · Ben Adeyemi" on a child and "Harbor API draft v3 · 6 messages" on a row;
  - labels read "reply to this" / "archive thread".
- [ ] T038 [P] [US2] Write the storyboard `storyboards/threads/scope-footer.*`, red on `main`.
- [ ] T039 [US2] Emit `Intent::Footer(FooterView)` on every cursor move in `crates/postio-focus/src/verbs.rs` and `crates/postio-focus/src/cursor.rs`, from `scope::footer`. Resolve list verbs through `scope::target`.
- [ ] T040 [US2] Mac footer in `macos/Sources/PostioKit/FocusSurfaceParts.swift` (keycap + label pairs, right-hand note). GTK footer in `crates/postio-gtk/src/chrome.rs`. Add a `focus_suite` case: `r` on a child opens a reply quoting that child.
- [ ] T041 [US2] Write the undo guard test: a label added from a child applies to every member and one `z` removes it from all of them, in `crates/postio-focus/tests/controller_suite/threads.rs`. It should be green on arrival, since the write-through is today's (research R1).

---

## Phase 5: User Story 3: walk a thread in the message window (P1)

**Goal**: the thread bar, `[` `]` `{` `}` `g u`, the kicker, Reply to, and
the older-message banner (screen 03).

**Independent test**: open Harbor, press `[` twice, `}`, then `g u`.

- [ ] T042 [P] [US3] Write `crates/postio-ui/src/thread_bar.rs` tests first (red: no module):
  - `chips` with everything fitting;
  - 40 members in room for 8 keeps the first, a More, and the latest, with the current and every unread always kept;
  - forced chips beyond the room produce several More chips;
  - `kicker` ("Message 4 of 6 · ] newer · [ older", keys from the keymap);
  - `older_banner`;
  - `reply_to_row` ("Ana Ruiz, Mon 21 Sep (message 1)", none when it answers the previous member).
- [ ] T043 [P] [US3] Write controller cases (red: no new `ReaderVerb`s; `[` unbound):
  - `[`/`]` step and mark read;
  - `]` on the latest is a no-op;
  - `{`/`}`;
  - `g u` goes to the first unread, and stays put with none;
  - `j` in the window opens the next thread;
  - `BarRoom` changes the chips.
- [ ] T044 [P] [US3] Write the storyboard `storyboards/threads/window-walk.*`, red on `main`.
- [ ] T045 [US3] Implement `thread_bar.rs` in `crates/postio-ui/src/thread_bar.rs`. Remove `position_line`'s and `thread_chip_at`'s thread parts from `crates/postio-ui/src/focus_dialog.rs` in favour of it (the GTK and FFI callers move in T047/T048).
- [ ] T046 [US3] Add the `Walk` state and `ReaderVerb::{FirstInThread, LatestInThread, FirstUnread}` in `crates/postio-focus/src/surfaces.rs` (mapping the new ids at the `NextInConversation` match). Emit `ThreadBar`, `Kicker`, `OlderBanner` and `ReplyTo` intents.
- [ ] T047 [US3] FFI: `crates/postio-ffi/src/focus_message.rs` reads `Client::thread`, not `conversation`, and emits `ThreadBarFfi`. Mac: `macos/Sources/PostioKit/ThreadBar.swift` (new: 60 tall, 66 label column, 72×46 chips with radius 8, accent ring and dots, Thread ▾), hosted under the action row in `macos/Sources/PostioAppKit/MessageWindowChrome.swift`, with the kicker, banner and Reply to row in `macos/Sources/PostioKit/MessageWindowView.swift`. Swift test: the keyboard path `[ [ } g u` changes the ringed chip.
- [ ] T048 [US3] GTK: the bar, kicker, banner and Reply to row in `crates/postio-gtk/src/open.rs` and `crates/postio-gtk/src/open_header.rs`, reading `Client::thread`, plus a `focus_suite` case for the walk keys. Move the remaining callers onto `Client::thread`: the terminal app (`crates/postio-tui/src/run.rs:1361`, `crates/postio-tui/src/app.rs:4820`, keeping its own view per spec D4), `crates/postio-ffi/src/session.rs:906`, and `crates/postio-host/src/tests.rs:714`. Then retire `Req::Conversation` and `Client::conversation` from `crates/postio-client/src/{protocol,api}.rs` and the host. Before removing, grep for callers: there must be none.
- [ ] T049 [US3] Screen 03 on both platforms; write the differences in the step-4 note.

---

## Phase 6: User Story 4: a reply arrives while reading (P2)

**Goal**: the new-reply chip and banner, and the list row updating behind
the window, without ever moving the reader (screen 08).

**Independent test**: with message 4 open, deliver a reply. Nothing moves,
and `}` shows it.

- [ ] T050 [P] [US4] Write controller cases (red: no reaction to `ThreadChanged` in a walk):
  - `ThreadChanged` with an open `Walk` keeps `shown` and emits `NewReply` naming the latest added member;
  - two arrivals name the latest and add two chips;
  - `}` clears the banner;
  - the list row's `ThreadRow` intent updates count, participants, new and time.
- [ ] T051 [P] [US4] Write the storyboard `storyboards/threads/live-reply.*`, red on `main` (an arrival key from the demo).
- [ ] T052 [US4] Add a live-reply key to `crates/postio-demo/src/lib.rs` (`POSTIO_DEMO_KEYS` token) that delivers a reply into Harbor through the real arrival path.
- [ ] T053 [US4] Implement `pending_new` and `NewReply` in `crates/postio-focus/src/surfaces.rs` and the row refresh in `crates/postio-focus/src/feed.rs`. Add `new_reply_banner` to `crates/postio-ui/src/thread_bar.rs`.
- [ ] T054 [US4] Mac banner (accent-tinted, "Read it }") in `macos/Sources/PostioKit/MessageWindowView.swift`, with a Swift test that the scroll offset is unchanged after an arrival. GTK banner in `crates/postio-gtk/src/open.rs`, with a `focus_suite` case asserting that the shown message and its adjustment value are unchanged.
- [ ] T055 [US4] Screen 08 on both platforms; write the differences in the step-4 note. Commit step 4.

---

## Phase 7: User Story 5: thread menu, outline, mute and split (P2)

**Goal**: screens 04 and 05; mute on arrival; split remembered.

**Independent test**: mute, deliver a reply, and the inbox does not gain it
("muted · 1"). Split message 4, and a reply to it joins the split.

### Mute and split in the engine (research R3, R4)

- [ ] T056 [P] [US5] Write failing tests in `crates/postio-model/src/threading.rs` (red):
  - a cue whose links find a split thread and its origin joins the nearest link's thread and does not merge;
  - `descendants` covers transitive replies.
- [ ] T057 [US5] Implement `ThreadIndex::is_split`, the split rule in `assign`, and `descendants` in `crates/postio-model/src/threading.rs`.
- [ ] T058 [P] [US5] Write failing storage tests in `crates/postio-storage/tests/storage_suite/threads.rs` (red: no tables):
  - `mute` survives a merge and a renumbering;
  - `muted(ids)` is one statement per page;
  - `split` moves message 4 and its replies, relinks claims and recomputes both aggregates in one transaction;
  - a later reply to message 4 joins the split thread. Assert rows and trigger firings for `muted(ids)` and `split` (constitution V). Also write the undo case: one `z` after a split forgets the `thread_splits` row and rejoins the messages, and a later reply to message 4 then joins the original thread (FR-007, constitution I).
- [ ] T059 [US5] Add `thread_mutes` and `thread_splits` to the migration in `crates/postio-storage/src/schema.rs` (with a fingerprint), `mute`/`unmute`/`muted`/`split` in `crates/postio-storage/src/repository/threads.rs`, and split flags in `LoadedIndex` in `crates/postio-storage/src/repository/threading.rs`.
- [ ] T060 [P] [US5] Write a failing host case (red: the reply lands in the inbox): a reply arriving in a muted thread is archived by the filing stage, `replies_since` becomes 1, it is searchable, and it is not a Filtered decision. Also write the exemption case: a reply addressed to the person alone (only identity in To, no Cc) lands in the inbox and the thread stays muted (FR-012). Use the filing-stage tests in `crates/postio-host/src/focus/`.
- [ ] T061 [US5] Add the mute check first in the filing stage in `crates/postio-host/src/focus/`, and `Req::{Mute, Unmute, Split, MutedOf}` in `crates/postio-client/src/{protocol,api}.rs`, answered in `crates/postio-host/src/lib.rs` (undoable mute). Add the `muted` field to list pages in `crates/postio-storage/src/repository/threads.rs` (`FocusListQuery`), with a count assertion of today's statements + 1. The count assertion covers rows and trigger firings too. Make `split` undoable through the existing undo stack.

### Menu and outline

- [ ] T062 [P] [US5] Write `crates/postio-ui/src/thread_outline.rs` tests first (red): `filter_rows` by text and by person, people chips with counts, `u` to the first unread, and the menu's two groups with keycaps from the keymap.
- [ ] T063 [P] [US5] Write controller cases (red):
  - `ThreadMenu` opens the menu;
  - `ThreadOutline` (`alt+t`) opens the outline;
  - `Return` in the outline shows that message and closes it;
  - `MuteThread` (`m`) toggles mute;
  - `SplitThread` from the menu;
  - `MoveThisMessage`/`DeleteThisMessage` aim `Messages([shown])`.
- [ ] T064 [P] [US5] Write the storyboards `storyboards/threads/menu.*`, `outline.*` and `mute.*`, red on `main`.
- [ ] T065 [US5] Implement `crates/postio-ui/src/thread_outline.rs`, the `Outline` and `ThreadMenu` surfaces in `crates/postio-focus/src/surfaces.rs`, and the "muted" badge and count on `ThreadRowView`.
- [ ] T066 [US5] Mac: `macos/Sources/PostioKit/ThreadMenu.swift` (340 wide, two groups, explanations) and `macos/Sources/PostioKit/ThreadOutline.swift` (440 wide, filter field, person chips, 34-tall rows, footer) as `NSPopover`s from Thread ▾. Swift test for the outline's keyboard path.
- [ ] T067 [US5] GTK: `crates/postio-gtk/src/thread_popovers.rs` (new) with the menu and outline `GtkPopover`s, and `focus_suite` cases for outline filter → `Return`, and for mute.
- [ ] T068 [US5] Screens 04 and 05 on both platforms; write the differences in the step-5 note. Commit step 5.

---

## Phase 8: User Story 6: search answers in threads (P2)

**Goal**: results grouped by thread (screen 06), opened at the match with
`mod+g` across messages (screen 07).

**Independent test**: search `rate limit headers`, then `j`, `Return`,
`mod+g` twice, `Escape`.

- [ ] T069 [P] [US6] Write `crates/postio-ui/src/search_view.rs` tests first (red):
  - `children(view, matches)` gives three `Match` children and one `Gap` ("Message 4 doesn't match · Ben, Thu");
  - a file match is tagged "message 5 · file";
  - a single-match thread gives the summary "Message 5 of 8 · Tomás: …";
  - "3 of 6 messages match".
- [ ] T070 [P] [US6] Write controller cases in `crates/postio-focus/tests/controller_suite/threads.rs` (red):
  - the top multi-match result is expanded on arrival;
  - `Return` on a match child opens the window at that message with `ThreadMatches`;
  - `FindNext` past the shown message's last match moves to the next matched message;
  - on the last match it stays and says so;
  - `Escape` returns to Results with the cursor on that child (History).
- [ ] T071 [P] [US6] Write the storyboard `storyboards/threads/search-open-at-match.*`, red on `main`.
- [ ] T072 [US6] Implement `ResultChild`, `children` and the match summary in `crates/postio-ui/src/search_view.rs`. In `crates/postio-focus/src/results.rs`, read the top result's `conversation_matches` with the page and expand it.
- [ ] T073 [US6] Add `ThreadMatches` to `Walk` and `ReaderVerb::{NextMatch, PrevMatch}` in `crates/postio-focus/src/surfaces.rs`, mapped from `FindNext`/`FindPrevious` when matches are present, plus `match_banner` in `crates/postio-ui/src/thread_bar.rs` and matched chips (yellow square).
- [ ] T074 [US6] Mac: children and gaps in `macos/Sources/PostioAppKit/ResultsTable.swift` and `macos/Sources/PostioKit/ResultRowView.swift`; the match banner in `MessageWindowView.swift`. GTK: the same in `crates/postio-gtk/src/results.rs` and `crates/postio-gtk/src/open.rs`, with a `focus_suite` case for the independent test.
- [ ] T075 [US6] Screens 06 and 07 on both platforms; write the differences in the step-6 note. Commit step 6.

---

## Phase 9: User Story 7: reply to an older message, forward a thread (P3)

**Goal**: screens 09 and 10.

**Independent test**: from message 4, `r` shows the bar, and `alt+e` keeps
the text and quotes message 6. Then `f`, `2` gives "Fwd: Harbor API draft v3
(6 messages)".

- [ ] T076 [P] [US7] Write failing tests in `crates/postio-body/src/forward.rs` (new) (red: no module):
  - `forward_thread` puts six members oldest first under a "From · date" header each, folds quotes, and includes each attachment once by content hash;
  - `forward_as_attachments` gives one `message/rfc822` per member.
- [ ] T077 [US7] Implement `crates/postio-body/src/forward.rs` and export it from `crates/postio-body/src/lib.rs`.
- [ ] T078 [P] [US7] Write session/host tests (red):
  - `ReplyToLatest` swaps `in_reply_to`, `References` and the quote, and keeps the typed text;
  - `SetForwardForm` rebuilds the forwarded content and keeps the typed text;
  - forms 2 and 3 with a member not local answer `NotLocal` and fetch nothing (assert no transport call through the `MailBackend` mock).
- [ ] T079 [US7] Implement `ReplyToLatest`, `SetForwardForm` and `Req::ForwardThread` in `crates/postio-session/src/` and `crates/postio-client/src/{protocol,api}.rs`, with the subject "Fwd: <subject> (N messages)".
- [ ] T080 [P] [US7] Write controller cases in `crates/postio-focus/tests/controller_suite/threads.rs` (red):
  - `f` in a thread of 2+ opens the chooser on This;
  - `2`/`3` switch the form;
  - `f` in a one-message thread shows no chooser;
  - a reply to a non-latest member shows `OlderReply`;
  - `ShowNewer` and `ExpandQuoted` toggle their views.
- [ ] T081 [P] [US7] Write the storyboards `storyboards/threads/reply-older.*` and `forward-thread.*`, red on `main`.
- [ ] T082 [US7] Implement the chooser (`ComposerKind::Forward { form }`) and the older-reply state in `crates/postio-focus/src/compose.rs`, with the words in `crates/postio-ui/src/compose.rs`.
- [ ] T083 [US7] Mac: `macos/Sources/PostioKit/ForwardChooser.swift` (three cards with number keys) and the older-reply bar in `macos/Sources/PostioKit/ComposeColumn.swift` / `ComposeModel.swift`, with a Swift test of the chooser keys. GTK: the same under `crates/postio-gtk/src/compose/`, with a `focus_suite` case.
- [ ] T084 [US7] Screens 09 and 10 on both platforms; write the differences in the step-7 note. Commit step 7.

---

## Phase 10: Polish and cross-cutting

- [ ] T085 [P] Write the edge-case controller cases from spec "Edge Cases" not already covered, in `crates/postio-focus/tests/controller_suite/threads.rs`:
  - a merge while expanded;
  - a window on a message deleted elsewhere;
  - a body not yet local;
  - a draft whose thread was archived. Also: a thread with messages in Archive, Sent and the inbox counts every message on its inbox row.
- [ ] T086 [P] Add a benchmark in `crates/postio-bench/benches/` for `thread_bar::chips` and `Thread` at 500 members (compiled on CI, timed by hand). Record p50/p95 in the step-4 note. Stop rule per plan.md.
- [ ] T087 [P] Add the scope rule's boundary note to the `postio-ui` entry in `docs/ARCHITECTURE.md`: scope is resolved only by `postio_ui::scope::target`.
- [ ] T088 [P] Check the logging added in this spec carries ids and counts only, by grepping new `tracing::` calls in the changed crates (FR-029).
- [ ] T089 Run `/ux-review` over `storyboards/threads/`, fix or contest each failure, and attach the summary to the PR.
- [ ] T090 Walk `quickstart.md` by hand on both platforms, then land with `scripts/issue-land.sh --detach --full-suite --title 'feat(focus): threads in Focus, Mac and Linux'`, with `Closes #2099` in the PR body.

---

## Dependencies and execution order

```text
Phase 1 (T001–T004) ─▶ Phase 2 (T005–T024) ─┬─▶ US1 (T025–T036) ─▶ US2 (T037–T041)
                                             ├─▶ US3 (T042–T049) ─▶ US4 (T050–T055)
                                             │                    └▶ US5 (T056–T068)
                                             ├─▶ US6 (T069–T075)   (needs US3's Walk for T073)
                                             └─▶ US7 (T076–T084)   (needs US3's Walk for the older-reply bar)
All ─▶ Polish (T085–T090)
```

- US2 needs US1's child rows (scope on a child).
- US4 and US5's menu need US3's window. US5's engine half (T056–T061) needs only Phase 2.
- US6's list half (T069, T072) needs only Phase 2; its window half (T073) needs US3.
- US7's body and session half (T076–T079) needs only Phase 2.

## Parallel opportunities

- **Phase 1**: T001, T002 and T003 together.
- **Phase 2**: T005, T009, T017 and T019 together (different crates). After that, T013 alongside T022.
- **Once Phase 2 is done**, three lanes run at once, each in its own crates:
  - US1 (T025–T036);
  - the US5 engine half (T056–T061);
  - the US7 body half (T076–T079).
- **Within a story**: the `[P]` test tasks are written together, e.g. for US1, T025 + T026 + T027 + T028.
- **The Linux box**: at most two agents, so GTK verification is one lane at a time.

## Implementation strategy

1. **MVP = Phase 1 + Phase 2 + US1** (T001–T036). Every message of a thread
   is visible and openable on both platforms, which closes #2099's
   reproduction. If the maintainer wants #2099 closed early, land this as
   its own PR (`--full-suite`) and keep the branch going.
2. **Then US2 + US3**: scope made explicit, and the window walk. This is the
   P1 set.
3. **Then US4, US5 and US6** (P2), in that order, so the walk is reused.
4. **Then US7** (P3).
5. Each phase ends runnable on both platforms with its screens compared and
   noted, and is committed on `feature/focus-threads`.
