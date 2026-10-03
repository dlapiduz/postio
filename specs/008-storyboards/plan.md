# Implementation Plan: Storyboards — Interactions Reviewed Before They Reach the Maintainer

**Branch**: `feature/storyboards` | **Date**: 2026-10-01 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/008-storyboards/spec.md`

## Summary

The spec asks for four things:

- interactions **written down** as storyboards, in the vocabulary every
  frontend shares;
- a **runner** per GTK app, which films each step and records where things
  are;
- a **reviewer** that did not write the change;
- **one page** for the maintainer, showing only what changed.

**The approach.**

- **One new pure crate, `postio-storyboard`.** It holds the format, the
  loader and lint, applicability, check evaluation, base-versus-branch
  comparison, parity, the review bundle and schema, and the page. Its tests
  run in milliseconds (R1).
- **One new shared type, `postio_ui::observe::Observation`.** Every frontend
  fills it from accessors it already has. Two small gaps get closed: the
  toast's tone and its undo flag (R6).
- **A shared GTK half.** It lived in `postio-gtk::storyboard` on `main`, and
  is `postio_widgets::storyboard` now (specs/007-postio-focus T265). It
  provides:
  - **chain delivery**: keys go through every key controller along the real
    focus chain. This is Focus's T195 test helper, promoted (R3).
  - **settle sampling**, which reports jumps and blanks (R4);
  - a **focus outline** drawn on the frame;
  - a **keyboard-reachability** check.
- **Determinism.** A hermetic re-exec, a frozen clock behind a new
  `postio_ui::clock` seam, embedded fonts only, and animations off (R5).
- **Two runners, then one.**
  - **Classic** (`postio-app`) shares `shot`'s setup through a new
    `postio_app::demo` module. It landed on `main`, and goes with the
    classic app in T256.
  - **Focus** (`postio-focus`) was built on a lane and is the runner design
    review plays on since specs/007-postio-focus T265: `postio_focus::demo`
    is the one demo store its `shot` and its runner share, with the seeds'
    store halves in `postio_storage::seed`, and the catalogue is written
    against Focus's surfaces.
- **`scripts/storyboards.sh` drives everything.** It replaces
  `scripts/screens.sh`, whose table becomes zero-step storyboards (R12).
- **Base runs.** The branch's storyboards are run against the merge-base's
  code, in a reflink-seeded detached worktree, with results cached by sha.
  This is also how a new storyboard gets its red (R0, R8).
- **`/ux-review` and `ux-reviewer`.**
  - A skill builds a bundle and launches a fresh Opus reviewer with a
    generated prompt.
  - The review is validated: every verdict must cite a frame.
  - Failures go back to the implementer, to fix or to contest.
  - A calibration set measures the reviewer itself (R9, R10).
- **`issue-land.sh`.** When a GTK app changed and no review matches the
  current tree, it warns and labels the PR. It never refuses (R13).

**Resolved against the constitution.** The spec's "reintroduce the defect"
Independent Tests and SC-001 conflicted with constitution IV, which forbids
re-breaking working code to test a test. This branch amends them. Red evidence
now comes from one of three places:

- **the base run**, for a defect being fixed on a branch;
- **an open defect**, which is red until its fix lands;
- a **`pinned`** storyboard, for a defect fixed before the runner existed. Its
  checks are tightened to the exact outcome the fix established, and it says
  plainly that it was never seen red (R0).

**A fifth step kind.** The spec also gains `event`, because a third of the
catalogue seed's timing rows need new mail or a folder change to arrive in
the middle of a storyboard (R11).

All technical unknowns are resolved in [research.md](./research.md). Two are
measured rather than decided, each by the first Foundational task:

- **The cost of one capture.** This decides whether settle sampling runs on
  every tick or on every *n*th tick (R4).
- **Whether the headless renderer is byte-identical across runs.** If it is
  not, the runner pins the cairo renderer (R5).

## Technical Context

**Language/Version**: Rust, pinned by `rust-toolchain.toml` (edition 2024).
Bash for `scripts/storyboards.sh`. Markdown for the skill and the agent
definition.

**Primary Dependencies**:
- **Nothing new to the workspace.** The new crate uses `toml`, `serde`,
  `serde_json` and `blake3`, which are all workspace dependencies already,
  plus `postio-core`, `postio-config` and `postio-ui`.
- **The GTK half** uses `gtk4`, `gsk4` and `libadwaita`, as `capture.rs`
  already does.

**Storage**:
- **Storyboards** are files in `storyboards/`.
- **Runs and reviews** are files under `Design/review/<branch>/`, which is
  gitignored.
- **Base runs** are cached under `~/.cache/postio/storyboards/`.
- **The app under test** runs on `postio_storage::seed` in memory, discarded
  after each run.

**Testing**:
- **Sanity tier** (`cargo test --lib`): `postio-storyboard`,
  `postio-ui::observe` and `postio-ui::clock`.
- **`gtk_suite`**: the shared GTK half — chain delivery, the outline, settle
  and reachability.
- **`app_suite`**: one `storyboards` case for Classic. **`focus_suite`**: the
  same, on the Focus branch.
- **Script self-tests**: under `scripts/run-self-tests.sh`.

**Target Platform**: Linux, on the private headless mutter compositor the
tests already use. Never the maintainer's display.

**Project Type**: developer tooling inside a desktop-app workspace. It ships
nothing to users. The only changes to shipped code are `observe()` on each
window, the clock seam, and the toast's tone and undo accessors.

**Performance Goals** (SC-002):

| What | Budget |
|---|---|
| One storyboard on one app, warm | under 15 s |
| The whole GTK catalogue, both apps, default variants | under 5 min |
| `observe()` (it reads only widget state) | under 1 ms |
| The catalogue lint | under 1 s |

**Constraints**:
- No network and no user store (FR-013).
- Deterministic frames (FR-012).
- Nothing a runner writes goes into git.
- Frames never go on a PR; only text summaries do.
- No worktree path in anything rustc sees. The base worktree is seeded by
  reflink, never pointed at (#1101).
- Never build `--release` while other sessions are building.

**Scale/Scope**:
- 47 catalogue seed rows ([catalogue-seed.md](./catalogue-seed.md)), about 40
  of them expressible with keyboard and events;
- 38 screens migrated from `screens.sh`;
- the flows;
- about 12 calibration storyboards;
- about 200 registry commands for the generated pass.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design.*

| Principle | Status | How |
|---|---|---|
| **I. Local-first; the UI never awaits the network** | Pass | No shipped behaviour changes. `observe()` is read-only and touches no store (contracts/observation.md, rule 1). Runs open no socket (FR-013). |
| **II. The keyboard is a system** | Pass, and strengthened | Storyboards press commands through the shared registry and resolver, so a broken binding fails a step (R3). Chain delivery tests the keymap where people actually meet it. The generated pass (US6) gives both GTK apps the "every command is answered" guarantee. |
| **III. One query language** | N/A | Search storyboards type queries; they do not parse them. |
| **IV. Test-first** | Pass, after the amendment | The runner and the format are built red-green like any code. Catalogue storyboards get their red from the base, from an open defect, or are labelled `pinned`. They never get it from re-breaking a fix (R0). This branch amends the spec's conflicting wording. Tests assert on what a person would see (the region holding the keyboard, the frame, the cursor), not on what a layer was handed. |
| **V. Performance is a requirement** | Pass | SC-002 budgets the runner. The `app_suite` case is subject to the four-minute landing budget and to `POSTIO-MEASUREMENT` like any other case. The clock seam is one function call per row format. |
| **VI. Privacy** | Pass | Fixtures only, on reserved domains, and linted (FR-027). No real store. The PR gets text summaries, never frames. The bundle refuses the untracked `Design/postio-focus-design/`, which carries a real name (contracts/review.md). Logs carry no content, because runs are files, not logs. |
| **VII. Boundaries are enforced** | Pass | See the list below. |

**Boundaries, in detail:**
- `postio-storyboard` gets a `RULES` row that bans gtk4, the database engines
  and tokio.
- `postio-ui` stays toolkit-free: `observe` and `clock` are plain data.
- The GTK half is in `postio-gtk` on `main`, and in `postio-widgets` on Focus,
  per ADR 0043.
- Seeds stay in the app crates, behind a `demo` feature.
- Focus never depends on `postio-gtk`.

**Re-check after Phase 1: pass.** The design added one crate, two
toolkit-free modules and one feature flag (`demo`). None of them crosses a
boundary. The one deviation is the spec amendment for IV, recorded above.

## Project Structure

### Documentation (this feature)

```text
specs/008-storyboards/
├── spec.md
├── plan.md               # this file
├── research.md           # R0–R15
├── data-model.md
├── catalogue-seed.md     # 47 mined interaction defects: the catalogue's first queue
├── quickstart.md
├── contracts/
│   ├── storyboard-format.md
│   ├── observation.md
│   ├── runner.md
│   └── review.md
├── checklists/requirements.md
└── tasks.md              # /speckit-tasks
```

### Source Code (repository root)

```text
crates/
├── postio-storyboard/            # NEW, pure: format, lint, checks, compare, parity, review, page
│   ├── src/{lib,format,lint,apply,check,run,compare,parity,bundle,verdicts,page,key}.rs
│   ├── src/bin/postio-storyboard.rs
│   └── templates/{reviewer-prompt.md,page.html}
├── postio-ui/src/
│   ├── observe.rs                # NEW: Observation and its enums
│   └── clock.rs                  # NEW: the clock seam
├── postio-gtk/src/
│   ├── storyboard/               # NEW: the shared GTK half (moves to postio-widgets on Focus)
│   │   ├── deliver.rs            # chain delivery, chord→gdk, typing
│   │   ├── settle.rs             # frame sampling; jumped/blanked/unsettled
│   │   ├── outline.rs            # the outlined frame
│   │   └── reach.rs              # keyboard.reachable
│   ├── capture.rs                # + texture_with(overlay)
│   ├── toast.rs                  # + tone(), offers_undo()
│   ├── window.rs                 # + observe()
│   └── row.rs, thread_row.rs, conversation.rs, reader/message_header.rs   # Local::now → clock::now
├── postio-app/
│   ├── src/demo.rs               # NEW (feature "demo"): shot's setup, seeds, presets
│   ├── examples/shot.rs          # slimmed; uses demo
│   ├── examples/storyboard.rs    # NEW: the Classic runner
│   └── tests/app_suite/storyboards.rs   # NEW case
└── (on feature/postio-focus)
    ├── postio-widgets/src/storyboard/   # git mv from postio-gtk
    └── postio-focus/{src/window.rs + observe(), src/demo.rs, examples/storyboard.rs,
                      tests/focus_suite/storyboards.rs}

storyboards/                      # NEW: the catalogue (contracts/storyboard-format.md § Layout)
scripts/storyboards.sh            # NEW; replaces scripts/screens.sh, which is deleted
scripts/issue-land.sh             # + the review-key warning and summary (R13)
scripts/checks/check-crate-boundaries.py   # + a postio-storyboard row
.claude/skills/ux-review/SKILL.md # NEW
.claude/agents/ux-reviewer.md     # NEW
.claude/skills/{gtk-design,ux-architect,issue}/SKILL.md   # point at storyboards
docs/decisions/0044-every-frontend-is-observable-and-storyboarded.md   # NEW (§ ADR)
CLAUDE.md                         # the loop: UI work ships with storyboards and /ux-review
```

**Structure Decision.** The code follows the dependencies (R1):
- **Pure logic** goes in a new crate.
- **Neutral types** go in `postio-ui`, which every frontend already has.
- **GTK code** goes where ADR 0043 says shared GTK lives.
- **Each app's runner** goes in the crate allowed to hold a seeded store.

The two apps share nothing new except through `postio-ui` and the GTK half.

## ADR

**ADR 0044, "Every frontend is observable and storyboarded."** This records
the rule that outlives this feature and that other work must obey (CLAUDE.md,
"A spec and an ADR are not both needed"). The rule has three parts:

1. Every frontend implements `Observation` as contracts/observation.md
   requires.
2. Every frontend has a runner that reads the shared storyboard format.
3. A change to a frontend's interaction ships with the storyboard that
   describes it.

The spec keeps the feature's reasoning; the ADR keeps only the rule. It is
0044 because `feature/postio-focus` already holds 0043 and `main` will receive
it.

## Branch shape and lanes

**`feature/storyboards`** (this branch) lands on `main` once, as one PR
reviewed against the spec. Rebase it as `main` moves. It carries:

- the pure crate;
- `observe` and `clock`;
- the GTK half;
- the Classic runner;
- the catalogue;
- the skill, the agent and the page;
- the `issue-land.sh` warning;
- the `screens.sh` migration;
- the ADR.

**The Focus lane** is a branch cut from `feature/postio-focus`, cherry-picked
into it per the memory rule for that branch. It starts once this branch's
Foundational phase is green. It does four things:

1. Cherry-picks the commits for `postio-storyboard`, `postio-ui` and the GTK
   half.
2. `git mv`s the GTK half into `postio-widgets`, as a pure move commit.
3. Adds `FocusWindow::observe()`, Focus's `demo`, its runner and its
   `focus_suite` case.
4. Switches `provides` to `Frontend::Focus` (R7).

**Nothing in the Focus lane runs `issue-land.sh` or opens a PR**: the branch
is held until the maintainer says so. When `feature/postio-focus` next rebases
onto a `main` that has 008, the cherry-picked commits drop by patch id.

**Risk:** the `git mv` conflicts with later edits to the GTK half on this
branch. **Mitigation:** finish and freeze the GTK half before the Focus lane
starts, and land any fix to it on this branch first.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A capture per tick is too slow for SC-002 | Medium | Measured first, in the Foundational phase. Sample every *n*th tick and record *n*. |
| The headless renderer is not byte-deterministic | Medium | Measured first. Pin `GSK_RENDERER=cairo` for runs and record that in the run, so it is never silent. |
| Chain delivery differs from real GTK routing in a way that lets a defect pass | Medium | `keyboard.reachable` on every step, and honest `routing = "real"` marking (FR-008). Real input is the next spec. |
| The reviewer is vague or too lenient | Medium | Citations enforced (FR-019), the calibration set (R10), and batches of one surface each. |
| The review becomes a ritual: run, ignore, land | Medium | The landing warning and the label make a skipped review visible. SC-004 measures whether escaped defects actually fall. |
| The Focus lane's `git mv` conflicts | Low | Freeze the GTK half first (see Branch shape). |
| `app_suite`'s `storyboards` case blows the landing budget | Low | It is an integration suite, so it runs nightly. If it is slow, it goes on `SLOW` / `POSTIO-MEASUREMENT`. |

## Complexity Tracking

No violation of constitution II or VII needs justifying. One choice adds
surface, and is recorded here:

| Addition | Why needed | Simpler alternative rejected because |
|---|---|---|
| A new crate, `postio-storyboard` | The format, checks, comparison and page must be shared by two runners now and four later, and tested in milliseconds. | Putting it in `postio-ui` would put a development tool (a TOML catalogue, an HTML page, a review schema) into the crate every shipped frontend compiles. Putting it in `postio-app` would make Focus depend on Classic, which ADR 0043 forbids. |
