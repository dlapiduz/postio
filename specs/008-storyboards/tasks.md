# Tasks: Storyboards — Interactions Reviewed Before They Reach the Maintainer

**Input**: Design documents from `specs/008-storyboards/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md),
[research.md](./research.md), [data-model.md](./data-model.md),
[contracts/](./contracts/), [quickstart.md](./quickstart.md),
[catalogue-seed.md](./catalogue-seed.md)

**Tests**: **Included and non-negotiable** (constitution IV).
- Every `[TEST]` task is written first, and must be **observed failing**
  before the task after it is done.
- A test that was never red is tightened until it visibly constrains the
  behaviour. It is never proven by re-breaking code that works.
- For **catalogue storyboards**, red comes from the base run, an open defect,
  or the storyboard is declared `pinned` (research R0).

**Workflow**: this work is spec-driven, so there are **no issues** (CLAUDE.md,
*Spec-driven work*).
- **Where**: `~/src/postio-worktrees/storyboards`, on `feature/storyboards`,
  with `main` recorded as the base.
- **Commits**: one per task. Each ends `Refs: specs/008-storyboards` and the
  task id, and never contains a closing keyword.
- **Landing**: the branch lands once, with `scripts/issue-land.sh --detach`.
- **Focus lane** tasks (marked **⟨Focus lane⟩**) run in a separate worktree,
  on a branch cut from `feature/postio-focus`, and are cherry-picked into it.
  They **never** run `issue-land.sh` and **never** open a PR: that branch is
  held until the maintainer says otherwise.
- **Discovered work**: under ~10 minutes, fix it here as its own commit.
  Larger, file it with `scripts/issue-file.sh`.

**Where things are tested.** Use the cheapest layer that can fail:

| What | Where |
|---|---|
| Format, lint, checks, comparison, parity, verdicts, page | `cargo test -p postio-storyboard --lib` (sanity tier) |
| `observe` and `clock` types | `cargo test -p postio-ui --lib` |
| GTK half: delivery, typing, reach, settle, outline | `crates/postio-gtk/tests/gtk_suite/`; a new case is a module plus a row in its `CASES` |
| Classic `observe()` and the runner over the real wiring | `crates/postio-app/tests/app_suite/`; a module plus a row in `main.rs`'s `CASES` |
| Scripts | a self-test under `scripts/tests/`, run by `scripts/run-self-tests.sh` |

## Format: `[ID] [P?] [Story] Description`

- **[P]**: can run in parallel. It touches different files and depends on no
  incomplete task.
- **[Story]**: US1 to US7, from spec.md.
- **[TEST]**: must be seen red before the next task.

---

## Phase 1: Setup

**Purpose**: the crate, the catalogue directory, and the boundary rule exist
before anything uses them.

- [X] T001 Confirm the worktree setup:
  - `~/src/postio-worktrees/storyboards` is on `feature/storyboards`;
  - `$(git rev-parse --git-dir)/postio-base` reads `main`;
  - `scripts/install-nextest.sh` and `scripts/install-shims.sh` have run.

  `git fetch origin main` and rebase if `main` has moved.
- [X] T002 Create the crate `crates/postio-storyboard`:
  - **`Cargo.toml`**: `edition.workspace`, and the dependencies `postio-core`,
    `postio-config`, `postio-ui`, `serde`, `toml`, `serde_json` and `blake3`,
    all with `workspace = true`;
  - **`src/lib.rs`**: module declarations only;
  - **`src/bin/postio-storyboard.rs`**: a `main` that prints usage.

  Add `"crates/postio-storyboard"` to `members` in the root `Cargo.toml`.
  `cargo check -p postio-storyboard` passes.
- [X] T003 [TEST] Give `postio-storyboard` a `RULES` row in
  `scripts/checks/check-crate-boundaries.py`, banning `gtk4`, `libadwaita`,
  `turso`, `rusqlite`, `tokio` and `webkit6`. Add a case to the script's
  self-test (`scripts/tests/`) that fails when `gtk4` is added to the crate.
  See it red against a scratch `Cargo.toml` in the test's temp dir, then
  green.
- [X] T004 [P] Create `storyboards/`, laid out per
  `contracts/storyboard-format.md` § Layout:
  - one directory per surface, plus `flows/`, `screens/`, `calibration/` and
    `gaps/`, each holding a `.gitkeep`;
  - a stub `storyboards/README.md` saying it becomes the living reference in
    T040.

---

## Phase 2: Foundational

**Purpose**: everything both GTK runners stand on. **No user story starts
until this phase is green.** The Focus lane (T068 onward) cherry-picks this
phase, so **freeze the GTK half (T022 to T032)** before that lane starts
(plan § Branch shape).

### Measure first: these decide two numbers the rest depends on

- [X] T005 Measure the capture cost and renderer determinism, with a
  throwaway example `crates/postio-app/examples/capture_probe.rs`:
  - seed `seed_small` through `feed_the_window`, as `shot` does;
  - time 200 `capture::texture_within` calls on a settled 1280×800 window;
  - capture the same settled window twice in two separate processes, under the
    default `GSK_RENDERER` and under `GSK_RENDERER=cairo`, and compare blake3
    of the pixels.

  Record the results in `docs/notes/2026-10-0X-storyboard-capture.md` (date
  and title) and list it in `docs/engineering-notes.md`. The note must give:
  - the median and p95 cost of one capture;
  - the sampling stride *n* this implies for a 3 s `settle_max` within SC-002;
  - which renderer is byte-deterministic.

  Update research R4 and R5 with the numbers, then delete the probe in the
  same commit.

### Neutral types in `postio-ui`

- [X] T006 [P] [TEST] Write tests in `crates/postio-ui/src/observe.rs` for
  `Observation` (data-model § Observation):
  - a JSON round trip of a fully populated value;
  - `Observation::shared_eq` ignores `keyboard.widget` and `app.*`, and
    compares every other field;
  - the enums serialise to the snake-case names data-model lists, such as
    `"first_run"` and `"cheatsheet"`.
- [X] T007 Implement `crates/postio-ui/src/observe.rs`:
  - `Observation`, plus `View`, `Region`, `Overlay`, `Tone` and the nested
    structs;
  - `shared_eq`;
  - `pub mod observe` in `crates/postio-ui/src/lib.rs`.

  T006 goes green.
- [X] T008 [P] [TEST] Write tests in `crates/postio-ui/src/clock.rs`:
  - `now()` is within a second of `Local::now()` by default;
  - after `freeze(t)`, `now()` returns `t` exactly, from any thread;
  - `thaw()` restores the default.
- [X] T009 Implement `crates/postio-ui/src/clock.rs`: a process-wide frozen
  instant behind a `OnceLock<Mutex<Option<DateTime<Local>>>>`, with `now()`,
  `freeze()` and `thaw()`. T008 goes green.
- [X] T010 [TEST] Add a case in `crates/postio-gtk/tests/gtk_suite/`,
  `row_timestamp_reads_the_clock.rs`, with its `CASES` row. With
  `clock::freeze(2026-06-02 09:00 UTC)`, a list row for a message received
  2026-06-01 09:00 shows the "yesterday" text that `postio_ui::row::timestamp`
  gives for that pair. Red today: rows read `Local::now()`.
- [X] T011 Replace `Local::now()` with `postio_ui::clock::now()` at the
  timestamp call sites:
  - `crates/postio-gtk/src/row.rs:101, 129, 894`
  - `crates/postio-gtk/src/thread_row.rs:408`
  - `crates/postio-gtk/src/reader/message_header.rs:292`
  - `crates/postio-gtk/src/conversation.rs:1678, 1783`

  Do the same for the wall-clock leaks `shot` shows: "last sync" and
  `queue_send(.., Utc::now())` (research R5). T010 goes green.

### The format and its logic, in `postio-storyboard`

- [X] T012 [P] [TEST] Write format tests in
  `crates/postio-storyboard/src/format.rs`, one per rule in
  `contracts/storyboard-format.md` § Rules:
  - two inputs in a step fail to load;
  - dotted check keys nest;
  - a string `"changed"` is a literal, not a sentinel;
  - `same_as` accepts both an id and an index;
  - `[app.focus.step.<id>]` parses into overrides;
  - `event` accepts only the six names in data-model § Step;
  - the grammar example in the contract parses into the expected value.
- [X] T013 Implement `crates/postio-storyboard/src/format.rs`: serde types
  for `Storyboard`, `Source`, `Step`, `Input`, `Checks` (with the leaf enum),
  `StepOverride` and `Settle`, plus `load(path)`. T012 goes green.
- [X] T014 [P] [TEST] Write lint tests in
  `crates/postio-storyboard/src/lint.rs`. Each of these is a named error:
  - a missing `source`;
  - `proof` missing when `kind = issue`;
  - an unknown command id;
  - a chord that does not parse after `expand_mod`;
  - an address on a non-reserved domain, as `user@realmail.com` would be;
  - `same_as` pointing forward;
  - an override naming a missing step;
  - `calibration` outside `calibration/`;
  - `app.*` or `keyboard.widget` checked in a storyboard that does not name
    exactly one app;
  - a name that differs from the file stem.

  Also: `the_catalogue_loads_and_lints` walks `storyboards/` from
  `CARGO_MANIFEST_DIR` and expects zero errors.
- [X] T015 Implement `crates/postio-storyboard/src/lint.rs`. Reuse
  `check-no-personal-data.py`'s list of reserved domains by reading it as a
  constant, kept in step by a test that greps the script. T014 goes green.
- [X] T016 [P] [TEST] Write applicability tests in
  `crates/postio-storyboard/src/apply.rs`:
  - on `main`, a storyboard using only shared commands applies to `classic`,
    and reads `focus: not present on this branch`;
  - one using a `Requirement::Terminal` command does not apply to `classic`,
    with the command named;
  - `apps = ["classic"]` with an unprovided command is a load error;
  - a seed or preset the app does not declare means not applicable.
- [X] T017 Implement `crates/postio-storyboard/src/apply.rs`: `App`,
  `provides(App, CommandId)` from `postio_core::registry` (research R7), and
  `applies(&Storyboard, App, &RunnerInfo) -> Applicability`. T016 goes
  green.
- [X] T018 [P] [TEST] Write check-evaluator tests in
  `crates/postio-storyboard/src/check.rs`:
  - every leaf (`literal`, `same_as`, `changed`, `unchanged`, `absent`,
    `one_of`) both passing and failing;
  - a check on a field the app declares unobserved (`back_depth: None` by
    declaration) gives `not_applicable`, not pass and not fail;
  - a failure names the path, the expected value and the observed value.
- [X] T019 Implement `crates/postio-storyboard/src/check.rs` over
  `serde_json::Value` paths into a serialised `Observation`. T018 goes
  green.
- [X] T020 [P] [TEST] Write tests for `Run` and `StepRun` in
  `crates/postio-storyboard/src/run.rs`, against data-model § Run's status
  rules:
  - any failed check, `unbound`, `nothing_to_type_into` or `blanked` gives
    `failed`;
  - `jumped` and `unsettled` alone do not;
  - `not_covered` steps never yield `passed` for their storyboard, only
    `not_covered`;
  - frame paths are written relative to `run.json`.
- [X] T021 Implement `crates/postio-storyboard/src/run.rs`: the types,
  `RunWriter`, and `status()`. T020 goes green.

### The shared GTK half, in `postio-gtk::storyboard` (freeze after T029)

- [X] T022 [P] [TEST] Write unit tests in
  `crates/postio-gtk/src/storyboard/deliver.rs` for `chord_to_gdk`:
  - `j` gives `(Key::j, empty)`;
  - `J` gives `(Key::J, SHIFT)`, mirroring `gtk_accelerator`'s unfolding
    (`crates/postio-gtk/src/keymap.rs:50-77`);
  - `ctrl+shift+tab` and `question` round-trip;
  - `mod+z` is refused unless already expanded.

  These are pure `gdk::Key::from_name` calls, with no `adw::init`
  (`check-no-gtk-init-in-unit-tests.py`).
- [X] T023 [TEST] Add a `gtk_suite` case, `storyboard_chain_delivery.rs`, plus
  its `CASES` row:
  - **(a)** A `j` delivered by chain to a window whose focus is in a list
    reaches the window's capture-phase controller.
  - **(b)** With an `adw::Dialog` presented whose own controller consumes
    `j`, the window's controller never sees it.
  - **(c)** With focus left on a widget that was then unparented, the key's
    fate is reported as `dropped`, not as delivered.

  Red: no `deliver` exists.
- [X] T024 Port focus's `support::deliver_with` into
  `crates/postio-gtk/src/storyboard/deliver.rs`. Read it with
  `git show origin/feature/postio-focus:crates/postio-focus/tests/focus_suite/support.rs`,
  lines about 731–950. It should:
  - walk `observe_controllers()` along the focus chain in capture, target,
    then bubble order;
  - honour `Propagation::Stop`;
  - return `Delivered { stopped_at }` or `Dropped`.

  Add `fn press(window, chord)` on top. T022 and T023 go green.
- [X] T025 [TEST] Add a `gtk_suite` case, `storyboard_typing.rs`, plus its
  `CASES` row. `type_text`:
  - inserts at the cursor of a focused `gtk::Text`;
  - inserts at the insert mark of a focused `gtk::TextView`;
  - returns `NothingToTypeInto` when focus is on a list.
- [X] T026 Implement `type_text` in
  `crates/postio-gtk/src/storyboard/deliver.rs`. The composer's web body is
  reached through a `TypeInto` hook the window supplies, which Classic wires to
  `Composer::test_body_eval` (`composer.rs:3382`). T025 goes green.
- [X] T027 [P] [TEST] Add a `gtk_suite` case, `storyboard_reach.rs`, plus its
  `CASES` row. `reachable(&window)`:
  - is true for a focused mapped list;
  - is false after the focused widget is unmapped;
  - is false while a modal dialog is presented over the window;
  - is false when the window has no focus widget.
- [X] T028 Implement `crates/postio-gtk/src/storyboard/reach.rs`. T027 goes
  green.
- [X] T029 [TEST] Add a `gtk_suite` case, `storyboard_settle.rs`, plus its
  `CASES` row. Over a test window:
  - a static label gives `settled` within `K` samples;
  - a label that changes text 150 ms after settling gives `jumped`, keeping
    both frames;
  - an unrealised child gives `blanked`;
  - a label that changes every 50 ms forever gives `unsettled` at `max_ms`.

  Use T005's stride.
- [X] T030 Implement `crates/postio-gtk/src/storyboard/settle.rs`: the
  tick-callback sampler, blake3 of the texture bytes, and the K, watch and max
  parameters per research R4. Add `capture::texture_with(&window, overlay)`
  in `crates/postio-gtk/src/capture.rs`, wrapping the private `drawn()` node in
  a `ContainerNode`. T029 goes green.
- [X] T031 [TEST] Add a `gtk_suite` case, `storyboard_outline.rs`, plus its
  `CASES` row. `outlined(&window, region_name)`:
  - draws a border whose pixels at the focused widget's bounds differ from the
    plain frame;
  - draws a caption carrying the region name;
  - leaves the plain frame's hash unchanged.
- [X] T032 Implement `crates/postio-gtk/src/storyboard/outline.rs`, with
  `pub mod storyboard` in `crates/postio-gtk/src/lib.rs`. T031 goes green.
  **The GTK half is now frozen** for the Focus lane.

### Classic's observation

- [X] T033 [TEST] Add a `gtk_suite` case, `toast_tone_and_undo.rs`, plus its
  `CASES` row:
  - a toast shown by an undoable verb reports `offers_undo() == true` and
    `tone() == Some(Info)`;
  - an error notice reports `Some(Error)` and no undo.
- [X] T034 Add `tone()` and `offers_undo()` to `crates/postio-gtk/src/toast.rs`,
  recorded at show time. Update the show-sites that know their tone. T033
  goes green.
- [X] T035 [TEST] Add an `app_suite` case, `observe.rs`, plus its row in
  `crates/postio-app/tests/app_suite/main.rs`'s `CASES`. Over the seeded
  wiring (as `keystroke.rs` builds it), after `deliver::press`:
  - `j` gives `keyboard.region = list`, `cursor.index = 1` and
    `keyboard.reachable = true`;
  - `/` gives `overlay.kind = finder`, `keyboard.region = search` and
    `keyboard.typing = true`;
  - `Escape` returns to the list;
  - `a` on a row gives `notice.undo = true`;
  - `back_depth` is `None`;
  - `app.classic.pane` is present.
- [X] T036 Implement `Window::observe()` in `crates/postio-gtk/src/window.rs`,
  following `contracts/observation.md` § Classic, including region naming from
  focus ancestors. `observe()` performs no store read. T035 goes green.

**Checkpoint**: the format loads and lints, checks evaluate, the GTK half
delivers, types, samples and outlines, and Classic can say where everything
is. Every user story can now start, and the Focus lane may cherry-pick
T002–T032.

---

## Phase 3: User Story 1 — An interaction, written once and filmed on Classic (Priority: P1) 🎯 MVP

**Goal**: write a storyboard, run it on Classic, and get a filmstrip with
outlined frames, observations and check results.

**Independent Test**: the #1687 storyboard passes on `main` with checks
pinned to the exact row. A storyboard for an open defect fails on the right
step. Two runs are identical (spec US1; quickstart §§ 2–4).

- [X] T037 [US1] Move `shot`'s setup into a new library module,
  `crates/postio-app/src/demo.rs`, behind a new `demo` feature:
  - the module holds `populate`, the `show_*` helpers, seeds per research R11
    and presets;
  - the feature enables `postio-storage/test-support`;
  - `examples/shot.rs` declares `required-features = ["demo"]` and calls the
    module.

  `shot`'s existing tests (`shot.rs` around line 868) stay green.
  `scripts/screens.sh` renders the same screens as before. Compare a handful
  by hash before and after the move.
- [X] T038 [US1] Add the neutral seeds `thirty-threads`, `long-thread` and
  `draft-left-over` to `crates/postio-app/src/demo.rs`, through
  `postio_storage::seed` helpers, so the catalogue seed's rows have the mail
  they need. Keep it deterministic: no `Utc::now()`.
- [X] T039 [US1] [TEST] Add an `app_suite` case, `storyboards.rs`, plus its
  `CASES` row. It runs a two-step fixture storyboard, held in the test as a
  string (`select_next`, then `archive`), through
  `postio_app::demo::storyboard::run`, and asserts:
  - the step outcomes are `delivered` with the chord used;
  - the checks pass;
  - `run.json`'s fields match data-model § Run;
  - `--no-frames` writes no PNG;
  - a `command` the current context does not bind yields
    `unbound in <context>`.

  Red: no runner exists.
- [X] T040 [US1] Implement the runner core in
  `crates/postio-app/src/demo/storyboard.rs`:
  1. Load and check applicability through `postio-storyboard`.
  2. Build the seeded window as `shot` does.
  3. Freeze the clock, turn animations off, and set the size.
  4. For each step, deliver it (command → `binding_for(current context)` →
     `chord_to_gdk` → `deliver::press`), then wait or settle, then observe,
     then evaluate checks.
  5. Write plain and outlined frames and `run.json` through `RunWriter`.

  Implement `event` steps by emitting on the wiring's `EventSink`. T039 goes
  green. Write `storyboards/README.md` from `contracts/storyboard-format.md`.
- [X] T041 [US1] Write the example `crates/postio-app/examples/storyboard.rs`
  (`required-features = ["demo"]`), with the subcommands `list` and `run`
  (`contracts/runner.md`). Add the hermetic re-exec from research R5:
  - `TZ` and `LANG`;
  - a `FONTCONFIG_FILE` written to a temp dir, naming only the embedded faces;
  - temp `XDG_*` directories;
  - `GTK_A11Y=test`;
  - `GSK_RENDERER` per T005's finding.
- [X] T042 [US1] [TEST] Add an `app_suite` case, `storyboard_determinism.rs`,
  plus its `CASES` row. It runs the same fixture storyboard twice in two
  re-exec'd processes, through the example binary, and asserts that the
  `run.json` values are equal apart from `commit`, and the frame hashes are
  equal (SC-003). Red until T041's hermetic setup is complete. If it is
  already green, tighten it by also comparing the outlined frames.
- [X] T043 [US1] [TEST] Write `scripts/tests/test-storyboards-sh.sh` against a
  stub runner binary:
  - `run --only` selects by glob;
  - exit codes are 0, 1 and 2 per `contracts/runner.md`;
  - `lint` calls `postio-storyboard lint`;
  - the runner is built once per invocation.
- [X] T044 [US1] Implement the `run`, `lint` and `key` subcommands of
  `scripts/storyboards.sh`. Output goes to `Design/review/<branch>/runs/`.
  T043 goes green.
- [X] T045 [US1] [TEST] Write the filmstrip test in
  `crates/postio-storyboard/src/page.rs`. From a `runs/` tree of two fixture
  runs, the page lists each storyboard with:
  - every step's outlined frame, as a relative `<img>`;
  - the observation, as a table;
  - each check's result;
  - settle flags;
  - the delivery mode in the header.

  `not_covered` and `not_applicable` runs are shown and counted, never as
  passed.
- [X] T046 [US1] Implement the filmstrip part of
  `crates/postio-storyboard/src/page.rs`, using the template
  `crates/postio-storyboard/templates/page.html`: self-contained, light and
  dark, no external fetches. Add `page` to `scripts/storyboards.sh`. T045
  goes green.
- [X] T047 [P] [US1] Write the first catalogue storyboards, from the
  catalogue seed rows #1687, #1474/#1011, 6eadd8e2, #1473 and #693/#1252:
  - `storyboards/list/archive-walks-down.toml` (`proof = "pinned"`);
  - `storyboards/search/escape-leaves-search.toml` (`pinned`);
  - `storyboards/search/escape-returns-to-the-row.toml` (`pinned`);
  - `storyboards/list/launch-keyboard-on-first-row.toml` (`pinned`; seed
    `small`, no steps before `j`);
  - `storyboards/search/tab-hands-keyboard-to-list.toml` (`open`, if #1252
    still reproduces).

  Run each. The `pinned` ones pass with exact checks. The `open` one is
  observed red, and its failing step is noted on #1252 in a comment.
- [X] T048 [US1] Add `postio-storyboard`'s
  `the_catalogue_loads_and_lints` (T014) to the sanity tier. It already runs
  under `--lib`, so this is a check of `scripts/test-sanity.sh`'s crate list,
  and adds the crate there if the list is explicit.

**Checkpoint**: one command films any storyboard on Classic, deterministically,
with the keyboard's region visible on every frame. This is the MVP.

---

## Phase 4: User Story 2 — A reviewer who did not build it (Priority: P1)

**Goal**: an independent design and UX review, with every verdict cited, and
failures returned to the implementer before the maintainer sees anything.

**Independent Test**: the calibration set. The reviewer fails every
`must_fail` storyboard and passes every `must_pass` one, citing frames (spec
US2; quickstart § 5).

- [X] T049 [P] [US2] [TEST] Write bundle tests in
  `crates/postio-storyboard/src/bundle.rs`. From runs plus an acceptance file:
  - `manifest.json` lists batches of one app and one surface, each at most
    60 frames;
  - every changed or new step is listed for a verdict;
  - design screens are copied only from committed reference directories, and
    a path under `Design/postio-focus-design/` is refused
    (contracts/review.md);
  - with no base present, every run is `new`.
- [X] T050 [US2] Implement `crates/postio-storyboard/src/bundle.rs` and the
  `bundle` subcommand. T049 goes green.
- [X] T051 [P] [US2] [TEST] Write prompt tests in
  `crates/postio-storyboard/src/bundle.rs`. `prompt <bundle>`:
  - renders `templates/reviewer-prompt.md` with the manifest's batches;
  - contains the six sections of contracts/review.md § The prompt template;
  - contains no text from outside the bundle;
  - prints the template's blake3.
- [X] T052 [US2] Write `crates/postio-storyboard/templates/reviewer-prompt.md`
  from contracts/review.md § The prompt template, and implement `prompt`.
  T051 goes green.
- [X] T053 [P] [US2] [TEST] Write verdict-validator tests in
  `crates/postio-storyboard/src/verdicts.rs`. Each of these is a named
  rejection:
  - a missing citation field;
  - a frame path absent from the bundle;
  - a manifest step with no verdict;
  - a `fail` without `severity`;
  - an empty `says`.

  A complete file passes. `contests.toml` entries attach to their refs.
- [X] T054 [US2] Implement `crates/postio-storyboard/src/verdicts.rs` and
  `verdicts check`. T053 goes green.
- [X] T055 [US2] [TEST] Extend the page tests in
  `crates/postio-storyboard/src/page.rs`:
  - verdicts render beside their frames, each linking to its frame;
  - **Needs you** holds exactly the contests and questions;
  - "review incomplete" or "no review ran" shows when appropriate;
  - `summary.md`'s first line is `storyboards-key: <key>`, and it holds no
    image.
- [X] T056 [US2] Implement the review sections of the page and `summary.md`.
  T055 goes green.
- [X] T057 [US2] Write `crates/postio-storyboard/src/key.rs` and its tests
  together, in the same test-first order. The key is blake3 over the given
  git tree ids, in sorted order. Then add `scripts/storyboards.sh key`, which
  computes the trees per research R13 (`git rev-parse HEAD:<crate>` for the
  app's crates, plus `HEAD:storyboards`).
- [X] T058 [P] [US2] Write `.claude/agents/ux-reviewer.md`: model `opus`;
  tools Read, Glob, Grep and Write. Its body says that its whole instruction
  is the prompt it is given, and that it writes only `verdicts.json` in the
  bundle.
- [X] T059 [US2] Write `.claude/skills/ux-review/SKILL.md`, following
  contracts/review.md § Who does what. It covers:
  - running and checking the base;
  - building the bundle;
  - launching `ux-reviewer` **fresh, never as a fork**, with the prompt
    verbatim, one agent per batch and at most 4 in parallel;
  - `verdicts check`, with one re-ask;
  - the resolution loop: fix and re-run, or contest;
  - building the page;
  - recording maintainer rejections as storyboards (FR-024);
  - `--calibrate`.
- [X] T060 [P] [US2] Write the calibration set, `storyboards/calibration/`.
  It has six `must_fail` storyboards, each a past defect stated as intent:
  - #1687: the cursor returns to the top;
  - #1473: typing `j` puts a "j" in search;
  - #1474: Escape keeps the hits;
  - #1177: the first screen shows the draft;
  - #1195: the composer takes a quarter of the window;
  - #1173: two reply bars.

  It has six `must_pass` twins with the correct expectation. The lint passes.
- [X] T061 [US2] Run `/ux-review --calibrate` on Classic. **The `must_fail`
  storyboards are the reviewer's red.** Record the hit rate in
  `docs/notes/2026-10-0X-storyboard-capture.md` (or a sibling note). Tune the
  prompt template, not the calibration set, until 12 of 12 are correct. If
  the template changes, re-run.
- [X] T062 [US2] [TEST] Extend `scripts/tests/test-issue-land*.sh`, or the
  script's existing self-test, so that with a stubbed `gh`:
  - a branch touching `crates/postio-gtk` with no current `summary.md` gets
    the `> [!WARNING]` block naming `/ux-review` and the label
    `interactions-unreviewed`, and the landing still proceeds;
  - with a current `summary.md`, the summary is in the body;
  - on an existing PR whose key changed, the summary is posted as a comment.
- [X] T063 [US2] Implement this in `scripts/issue-land.sh`, beside
  `VERIFY_NOTE` and `VERIFY_LABEL` (`issue-land.sh:1031-1056`), per research
  R13. Create the `interactions-unreviewed` label with `gh label create` if it
  is missing, and say so loudly if that fails. T062 goes green.

**Checkpoint**: `/ux-review` produces a validated, cited review. The page shows
what needs the maintainer, and landing warns when a GTK change was not
reviewed.

---

## Phase 5: User Story 4 — The maintainer sees only what changed (Priority: P2)

US4 comes before US3, because it is entirely on this branch. US3's Focus half
runs in its own lane and can overlap.

**Goal**: base versus branch. Only changed and new storyboards are shown in
full, and a new storyboard's base run is its red.

**Independent Test**: a branch changing only search spacing shows the search
storyboards before and after, and reports the rest as unchanged with a count
(quickstart § 3).

- [X] T064 [P] [US4] [TEST] Write comparison tests in
  `crates/postio-storyboard/src/compare.rs`:
  - classification as `unchanged`, `changed`, `new`, `removed` or
    `base_unavailable`;
  - a frame change detected by the plain-frame hash only (an outline-only
    change is not a change);
  - observation changes named by field path;
  - a seed change reported once, at the top.
- [X] T065 [US4] Implement `crates/postio-storyboard/src/compare.rs` and
  `compare`, plus the page's changed, new and unchanged sections, with base
  and branch side by side. T064 goes green.
- [X] T066 [US4] [TEST] Extend `scripts/tests/test-storyboards-sh.sh` for
  `base`:
  - it creates a detached worktree at the merge-base with the recorded
    `postio-base`;
  - it seeds `target/` by `cp --reflink=auto` from the current tree;
  - it caches under `~/.cache/postio/storyboards/<app>/<sha>/<storyboard-hash>/`,
    and a second call is a cache hit with no build;
  - it links `Design/review/<branch>/base/`.

  Use a temporary git repository and a stub runner.
- [X] T067 [US4] Implement `scripts/storyboards.sh base`, plus `--changed`
  selection through `postio-storyboard select`. Put no worktree path into
  anything rustc sees (#1101). T066 goes green. Prove it end to end: on a
  scratch branch that changes only finder spacing in
  `crates/postio-gtk/src/finder.rs`, `page` shows the search storyboards
  changed and the rest counted. Drop the scratch branch afterwards.

**Checkpoint**: the maintainer's page is a diff of behaviour. New storyboards
show their own red, from the base, beside their green.

---

## Phase 6: User Story 3 — One storyboard, both GTK apps (Priority: P2)

**Goal**: shared storyboards run on Classic and Focus. Per-app overrides
apply, and the parity sheet marks divergence.

**Independent Test**: archive-walks-down on both apps shows both columns. A
real difference with no override is marked diverging, and adding the
override clears it (quickstart § 7).

On this branch:

- [X] T068 [P] [US3] [TEST] Write parity tests in
  `crates/postio-storyboard/src/parity.rs`. Over synthetic runs of two apps:
  - equal `shared_eq` observations do not diverge;
  - a differing `cursor.index` diverges;
  - a step with an override for one app does not diverge on the overridden
    fields;
  - a step skipped for one app is shown as skipped;
  - variants are grouped.
- [X] T069 [US3] Implement `crates/postio-storyboard/src/parity.rs`, the
  `parity` subcommand, the page's parity section, and
  `scripts/storyboards.sh run --app all`. That runs every runner whose crate
  exists on the branch, and reports a missing one as `app not present`. T068
  goes green.

**⟨Focus lane⟩**: worktree `~/src/postio-worktrees/storyboards-focus`, branch
`feature/storyboards-focus` cut from `feature/postio-focus`. **No landing, no
PR.**

- [X] T070 [US3] ⟨Focus lane⟩ Cherry-pick T002–T032 onto
  `feature/storyboards-focus`. Resolve conflicts against Focus's
  `postio-widgets` `capture.rs`: `texture_with` goes there too.
- [X] T071 [US3] ⟨Focus lane⟩ `git mv crates/postio-gtk/src/storyboard
  crates/postio-widgets/src/storyboard` as a pure move commit. Then a second
  commit: `pub mod storyboard` in `postio-widgets/src/lib.rs`, with
  `postio-gtk` re-exporting it (ADR 0043).
- [X] T072 [US3] ⟨Focus lane⟩ [TEST] Update `apply.rs`'s tests for the Focus
  branch:
  - `provides(App::Focus, toggle_has_action)` is true, and `provides(App::Classic, …)` is false;
  - flag (`*`) is not provided by Focus.

  Implement `provides` via `registry::get(id).requires.offered_by(Frontend::…)`
  (research R7).
- [X] T073 [US3] ⟨Focus lane⟩ [TEST] Add a `focus_suite` case, `observe.rs`,
  plus its `CASES` row, mirroring T035 for Focus:
  - `j` gives the list region with the cursor moved;
  - `/` gives `overlay = finder` and `region = search` (the bar);
  - `Return` on a row gives `view = reader` and `region = reader` (the
    dialog);
  - `Escape` returns;
  - `d` gives `notice.undo`;
  - `app.focus.bulk` is present.
- [X] T074 [US3] ⟨Focus lane⟩ Implement `FocusWindow::observe()` in
  `crates/postio-focus/src/window.rs` per contracts/observation.md § Focus.
  Add a bulk-bar getter in `crates/postio-focus/src/bulk.rs`, plus `tone()`
  and `offers_undo()` on its toast path (`postio-widgets` `toast.rs`). T073
  goes green.
- [X] T075 [US3] ⟨Focus lane⟩ Move Focus's `shot` setup into
  `crates/postio-focus/src/demo.rs` behind a `demo` feature, mirroring T037.
  Focus's `shot` tests stay green.
- [X] T076 [US3] ⟨Focus lane⟩ [TEST] Add a `focus_suite` case,
  `storyboards.rs`, plus its `CASES` row, mirroring T039. Then implement
  `crates/postio-focus/src/demo/storyboard.rs` and
  `crates/postio-focus/examples/storyboard.rs`, mirroring T040 and T041, over
  `postio-host` and `postio-client` as Focus's `shot` does.
- [X] T077 [US3] ⟨Focus lane⟩ Add Focus overrides to the shared storyboards
  from T047, wherever Focus legitimately differs. For example, `Return` opens
  a dialog, not a pane, and `/` opens the bar. Run
  `scripts/storyboards.sh run --app all`, and check the parity sheet. Any
  divergence with no override is either a Focus defect, which is fixed or
  filed, or a missing override, which is added. Verify, then cherry-pick the
  lane's commits into `feature/postio-focus` and push. **Do not land.**

**Checkpoint**: one storyboard, two apps, one sheet. Divergence is visible
without anyone looking for it.

---

## Phase 7: User Story 5 — A catalogue from what already went wrong (Priority: P2)

**Goal**: every expressible row of [catalogue-seed.md](./catalogue-seed.md)
becomes a storyboard. The rest are recorded as not expressible, with the
reason.

**Independent Test**: the catalogue holds a storyboard for each Context-table
defect, each with its `proof` (spec US5; SC-001).

For every task in this phase:
- write the storyboard first;
- `proof = "open"` storyboards must be seen red;
- `pinned` ones get exact checks;
- run each on Classic before committing;
- record each row's outcome in `catalogue-seed.md`, as a new column `storyboard`
  holding either the file or `not expressible: <reason>`.

- [X] T078 [P] [US5] List rows → `storyboards/list/`:
  - #1687 multi-press (`a a a`, `d d d`);
  - #1609 rapid keys, one step per key;
  - #753/#750, cursor versus selection and revealing new mail, using an
    `event = "new_mail"` step;
  - #468/#1701/#811/#1300, selection acting on the right messages (unified,
    `two-accounts` seed);
  - #1475/#499, the sort chip.
- [X] T079 [P] [US5] Search rows → `storyboards/search/`:
  - 79b1cd8a, Return on a mode hint;
  - #961/#767/#1526, search scope and opening from a preview;
  - #1011, the list restored after search.
- [X] T080 [P] [US5] Sidebar and navigation rows → `storyboards/sidebar/`:
  - #494/#437, the Tab and Shift-Tab pane cycle (`routing = "real"` on any
    step relying on GTK's native traversal);
  - d2be7412, the sidebar walk past Snoozed;
  - #455/#471, the saved-search keyboard path;
  - #813, the folder reload keeps Flagged (`event = "mailboxes_changed"`);
  - #756, the sidebar toggle from the palette and `ctrl+b`;
  - #825, narrow-window one-pane navigation.
- [ ] T081 [P] [US5] Reader and conversation rows →
  `storyboards/reader/` and `storyboards/conversation/`:
  - #1402/#1431/#438, keys scroll the one-document pane;
  - #1398, view original;
  - #1386/#1365, per-message focus and reply;
  - #1385/#1372, opening on the newest message;
  - #797/#1400, read marks follow focus;
  - #1173/#822, a single reply bar;
  - #601/#1414, the pane filled on launch;
  - #1523–#1525, the outbox reader's verbs;
  - d1ddd2dc/#749/#947, no flash between messages, using settle `blanked` and
    `jumped` as the check.
- [ ] T082 [P] [US5] Compose rows → `storyboards/compose/`:
  - #1177/#1212/#1444, the first screen does not jump to a draft (seed
    `draft-left-over`);
  - #491/#1196/#1240/#426, a left-over draft, and a blank `c`;
  - #1195, the composer takes over the reading pane;
  - #602/#73, typing is not eaten by single keys;
  - #690/#325, where focus lands on reply and forward;
  - #1481, undo send;
  - 15192fb5, the caret above the quote.
- [X] T083 [P] [US5] Onboarding and settings rows → `storyboards/onboarding/`
  and `storyboards/settings/`:
  - #629/#68, Return in every onboarding field (seed `first-run`);
  - #1016, the rebind list does not leak bare keys;
  - #67/#404, a missing credential shows recovery, not an empty inbox (seed
    `locked`).
- [ ] T084 [US5] Write the flows → `storyboards/flows/` (FR-026). These are
  the end-to-end walks `/ux-architect` § 4 names:
  - open → `J`/`K` → `e` → `ctrl+Return` → `Escape`;
  - search → open hit → reply → back;
  - triage: `j`, `a`, `j`, `a`, `u`.

  `source = { kind = "flow", ref = "ux-architect §4" }`.
- [ ] T085 [US5] ⟨Focus lane⟩ Write the Focus rows →
  `storyboards/list/` and `storyboards/reader/`, with `apps = ["focus"]`
  where they are Focus-only:
  - 92a093b8, a key after a removed row;
  - 0cbbd3d8, the rules-list keys (after the rules land);
  - a19c4bbb/de495089, keys under the dialog;
  - 2ba0e97c, toggling `O` keeps the scroll;
  - 8ab954bf, no trailing cursor ring;
  - 88c1f0f7/63641d47, dialog Delete, and More returning focus.

  Run them on the lane, then cherry-pick into `feature/postio-focus`.
- [ ] T086 [US5] Record the pointer rows as not expressible in
  `catalogue-seed.md`, each with its reason: #1679, 2af808b1, #56 and #40.
  Comment on any `open`-proof storyboard's issue with the red run's failing
  step and check. This is the evidence, and it lives where the issue lives.

**Checkpoint**: about 40 storyboards, each naming where it came from and how
it was proved. The past is now a regression suite.

---

## Phase 8: User Story 6 — Every command does something you can see (Priority: P3)

**Goal**: a generated pass over every bound command, in every context, per
GTK app.

**Independent Test**: running the pass on Classic names each command with no
visible effect. Each one is fixed, filed, or listed with a reason (spec US6).

- [ ] T087 [P] [US6] [TEST] Write coverage-logic tests in
  `crates/postio-storyboard/src/coverage.rs`, over synthetic before and after
  observations and frame hashes:
  - `effect` when either one changed;
  - `no_effect` when neither did;
  - `listed_gap`;
  - `stale_gap` when a listed gap now has an effect;
  - gap-list parsing per contracts/storyboard-format.md § Gap list.
- [ ] T088 [US6] Implement `crates/postio-storyboard/src/coverage.rs`, then
  the `every-command` subcommand in `crates/postio-app/examples/storyboard.rs`.
  For each context with a starting state (declared in `list`), and for each
  command bound there, it starts a fresh seeded window, delivers by chain, and
  compares. Add `scripts/storyboards.sh coverage`. T087 goes green.
- [ ] T089 [US6] Run the coverage pass on Classic. For every `no_effect`:
  - a fix under ~10 minutes is made here, in its own commit;
  - anything larger is filed with `scripts/issue-file.sh`, and listed in
    `storyboards/gaps/classic.toml` with `tracked = <issue>`.

  Commit the gap list.
- [ ] T090 [US6] Add an `app_suite` case, `every_command.rs`, with its
  `CASES` row and a `//! POSTIO-MEASUREMENT:` marker. Exclude it from
  `.config/nextest.toml`'s `profile.default` `default-filter`, so it runs
  nightly (CLAUDE.md, the measurement tier). `check-measurement-tier.py`
  passes.
- [ ] T091 [US6] ⟨Focus lane⟩ Mirror T088–T090 for Focus:
  `storyboards/gaps/focus.toml`, and a `focus_suite` `every_command` case.

---

## Phase 9: User Story 7 — Variants, and the screen sweep (Priority: P3)

**Goal**: storyboards run across declared variants. `screens.sh`'s table
becomes zero-step storyboards (FR-030).

**Independent Test**: the open-message storyboard in light and dark, on both
apps, produces four filmstrips grouped by variant (spec US7). `storyboards.sh
screens` reproduces the old contact sheet (quickstart § 8).

- [X] T092 [P] [US7] [TEST] Write variant-expansion tests in
  `crates/postio-storyboard/src/apply.rs`:
  - the cross product of the requested and the supported axes;
  - an unsupported axis is reported as ignored, per app;
  - variant keys are stable strings, such as `scheme=dark,width=narrow`.
- [X] T093 [US7] Implement variant expansion. Have both runners' `list`
  declare their axes, per data-model § Axis, and apply each axis:
  - `scheme` through `adw::StyleManager`;
  - `contrast` through the high-contrast class;
  - `width` through `set_default_size`;
  - `density` through Classic's setting;
  - `text` through `gtk_xft_dpi`.

  Add `storyboards.sh run --variants`. The page names the variant on every
  frame. T092 goes green.
- [ ] T094 [US7] Turn each row of `scripts/screens.sh`'s `SCREENS` table into
  `storyboards/screens/<name>.toml`. Each has zero steps, its seed or preset,
  its variant, and `design = "<canvas screen>"`, or no `design` where the
  table said `-`. Add any preset `shot` hand-feeds to `postio_app::demo`'s
  preset list.
- [ ] T095 [US7] [TEST] Extend `scripts/tests/test-storyboards-sh.sh`:
  `screens` writes an `index.html` that pairs each design PNG with the
  rendered frame, and exits non-zero naming any screen that failed to render
  (the old `screens.sh` contract).
- [ ] T096 [US7] Implement `scripts/storyboards.sh screens` over the
  zero-step storyboards, and delete `scripts/screens.sh`. Update every
  mention of it: `grep -rn screens.sh` across `.claude/`, `docs/`, `scripts/`
  and `CLAUDE.md`. T095 goes green.

---

## Phase 10: Polish and cross-cutting

- [ ] T097 [P] Write `docs/decisions/0044-every-frontend-is-observable-and-storyboarded.md`.
  It states the rule only: the `Observation` contract, a runner per frontend,
  and interaction changes shipping with storyboards (plan § ADR). Add it to
  `docs/decisions/README.md`.
- [ ] T098 [P] Update `CLAUDE.md`:
  - **in the loop**: a change to a GTK app's interaction ships with its
    storyboard, and runs `/ux-review` before `issue-land.sh`;
  - add `/ux-review` to § Skills;
  - under "Say it where it persists", add a maintainer rejection → a
    storyboard.
- [ ] T099 [P] Update the skills:
  - `.claude/skills/gtk-design/SKILL.md` § 6: storyboards beside `shot`, and
    `storyboards.sh screens` in place of `screens.sh`;
  - `.claude/skills/ux-architect/SKILL.md` § 4: "review the flow" means
    writing it as a storyboard in `storyboards/flows/`;
  - `.claude/skills/issue/SKILL.md`: UI issues get a storyboard first,
    written from the acceptance, before the implementation.
- [ ] T100 [P] Add a test in `scripts/tests/` asserting
  `check-no-personal-data.py`'s `tracked_files` includes `storyboards/`
  (FR-027).
- [ ] T101 Measure SC-002 with:
  - one storyboard, warm;
  - the whole Classic catalogue at default variants;
  - on the Focus lane, both apps.

  Record the numbers in the T005 note. If over budget, apply the sampling
  stride or `SLOW` / `POSTIO-MEASUREMENT` per CLAUDE.md, never by loosening
  settle detection.
- [ ] T102 Run `quickstart.md` §§ 1–10 end to end. Fix anything that does
  not hold, or amend the quickstart where it was wrong about the design. Run
  `/speckit-analyze` for consistency across spec, plan and tasks.
- [ ] T103 Run `/ux-review` on this branch itself. It touches `postio-gtk`, so
  the landing warning would otherwise fire. Resolve its verdicts. Then
  `git fetch origin main`, rebase, `scripts/test-sanity.sh`,
  `cargo nextest run -p postio-app --test app_suite storyboards`, and
  `scripts/issue-land.sh --detach`. Its PR is reviewed against the spec.

---

## Dependencies and execution order

```text
Setup (T001–T004)
  └─► Foundational (T005–T036)   ── T005 first: its numbers feed T029/T030/T041
        ├─► US1 (T037–T048)  🎯 MVP
        │     ├─► US2 (T049–T063)
        │     │     └─► US4 (T064–T067)
        │     ├─► US5 Classic rows (T078–T084, T086)   needs US1's runner and T038's seeds
        │     ├─► US6 (T087–T090)
        │     └─► US7 (T092–T096)
        └─► Focus lane, after T032 freezes the GTK half:
              T070–T077 (US3) → T085 (US5) → T091 (US6)
Polish (T097–T103) last; T103 lands the branch.
```

**Within each story**, every `[TEST]` comes before the task that turns it
green. Pure logic in `postio-storyboard` comes before the runner or page code
that uses it.

**Story independence.**
- **US1** stands alone.
- **US2** needs US1's runs to review, but its pure parts (T049–T057) can be
  written against fixture runs in parallel with US1.
- **US4** needs only US1.
- **US3**'s parity logic (T068, T069) needs only Foundational. Its Focus half
  needs the frozen GTK half.

## Parallel opportunities

**Foundational.** Three groups can run in parallel:
- **Pure** (T006–T009, T012–T021): five pairs touching five files. Run them
  as parallel lanes, each pair red then green in one lane.
- **GTK half** (T022–T032): sequential within itself, because T024 is needed
  by T026 and T030 is needed by T032. It is parallel with the pure group.
- **Classic `observe`** (T033–T036): after T024, parallel with T025–T032.

**US1.** T037/T038 (demo) and T043/T044 (script) and T045/T046 (page) are
three independent lanes. T039–T042 join them.

**US2.** T049/T050, T051/T052, T053/T054 and T058 are four independent
lanes. T059–T063 follow.

**US5.** T078–T083 are six surfaces, each its own lane writing storyboards
into its own directory. This is the most parallel phase.

**Focus lane.** It runs entirely alongside US2, US4, US5 and US6 on this
branch.

**Example.** Launch the Foundational pure group as five lanes. These are
implementation lanes, so they run on Sonnet, per the subagent memory; each
has its own worktree, and is cherry-picked back here.

```text
lane A: T006 → T007    crates/postio-ui/src/observe.rs
lane B: T008 → T009    crates/postio-ui/src/clock.rs        (then T010 → T011 here)
lane C: T012 → T013    crates/postio-storyboard/src/format.rs
lane D: T016 → T017    crates/postio-storyboard/src/apply.rs (after C's types)
lane E: T018 → T021    crates/postio-storyboard/src/{check,run}.rs
```

## Implementation strategy

1. **The MVP is Setup + Foundational + US1.** One command films any storyboard
   on Classic, deterministically, with the keyboard's region outlined. That
   alone makes sequence defects visible. Stop and look at the #1687 and #1473
   filmstrips before going on.
2. **Then US2.** Without the reviewer, the filmstrips are only something the
   maintainer could look at. With it, they are reviewed before the maintainer
   sees them, which is the request.
3. **Then US4.** The page shrinks to what changed, and new storyboards get
   their red from the base.
4. **Then the Focus lane and US5, in parallel.** The maintainer asked for
   both GTK apps first. The catalogue is the work that pays back longest.
5. **US6 and US7 are worth doing, not urgent.** US7 also retires
   `screens.sh`, so it must finish before landing (FR-030).
6. **The branch lands once** (T103). The Focus lane's commits stay on
   `feature/postio-focus` until the maintainer lands that branch.
