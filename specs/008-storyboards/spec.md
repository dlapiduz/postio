# Feature Specification: Storyboards — Interactions Reviewed Before They Reach the Maintainer

**Feature Branch**: `feature/storyboards`

**Created**: 2026-10-01

**Status**: Draft

**Input**: User description: "We've created the app that is 80/90% of the way of where i want it to be but i struggle with the interactions, everything is very hard to get right with the agents. Can you help me figure out a system where a claude agent can review the interactions and the screens before shipping them to me for review? Some way that we list all the interactions that we should check for (maybe check the last history of interactions) and create screenshots for them and review them with a design/ux agent?" Refined: "This should cover the 2 gtk apps (focus and normal), the tui and the mac app. We should have storyboards that apply to multiple versions and storyboards that apply to specific ones or variants." Then: "Focus on the gtk and focus versions first, we can move on from there."

## Context

This spec is about **how an interaction gets looked at before the maintainer
looks at it**. Postio can already draw a picture of any *screen*. `shot`
renders a window through the real wiring, and `scripts/screens.sh` lays out
about forty screens beside the design canvas screens they answer to. What it
cannot draw is a *sequence*. The defects that keep reaching the maintainer are
sequence defects:

| Defect | Issue | What a still picture shows |
|---|---|---|
| The window opens with the keyboard in the search field | #1473 | a correct-looking window |
| Escape does not leave search once the finder has closed | #1474, #1011 | a correct search screen |
| Archive moves the cursor elsewhere in the list | #1687 | a correct list |
| Page keys and J/K do not scroll the conversation | #1431, #1402 | a correct conversation |
| The first screen shows a draft, then jumps away | #1177 | either frame, both correct |
| Tab claims the keyboard but the list does not get it | #1252 | nothing |
| Scrolling navigates the reader to "The URL can't be shown" | #1433 | nothing, until the step after |

Each of these passed every test in the repository. The cause is structural,
not a lack of care:

| Cause | Where |
|---|---|
| Interactions are never written down before they are built. The agent implementing a change guesses what the interaction should be, then writes a test asserting its guess. | issue bodies; `app_suite` cases |
| The tests that drive keys call the window's key handler directly. They skip toolkit event delivery and focus routing, which is where #1252 and #1473 lived. GTK 4 offers no supported way to synthesise input (#424, #437). | `crates/postio-gtk/src/window.rs` `handle_key`; about 70 test files |
| Nothing records *where things are* after a step: which region has the keyboard, where the cursor sits, which overlay is up, what notice is shown. `SharedState` holds the view, the selection and the back stack. It does not hold the region with the keyboard, the cursor position, overlays or notices. | `crates/postio-core/src/state.rs` |
| The reviewer of a change is the agent that made it. | the `/issue` loop |

What this spec adds:

- **Storyboards.** A storyboard is an interaction written down once, as steps
  with expectations, in the vocabulary every Postio frontend shares.
- **A runner** for the app. It plays a storyboard and records a picture and
  a neutral observation at every step.
- **A reviewer** that did not write the change.
- **One page for the maintainer**, showing what changed.

It was written for the two GTK apps, Classic (`postio-app`) and Focus
(`postio-gtk`, spec 007). Focus is now Postio, the one desktop app (ADR
0043), and the maintainer kept this feature by rebuilding its runner over it
(2026-10-02, specs/007-postio-focus T265): storyboards play on Postio and
nothing else, and the runner, the demo store and the catalogue are its. The
stories below that name Classic say where the feature started. The terminal
and macOS apps are later phases. Nothing in the format
may assume a toolkit, so they can join without rewriting a single storyboard.

### What this spec inherits

- **The command vocabulary**, `CommandId`, and the shared default bindings in
  the command registry (`postio-core`).
- **The key resolver** (`postio_ui::keymap`) and the Tab and Escape focus
  policy (`postio_ui::focus`). Every frontend already resolves keys through
  these.
- **The frontend tags on registry rows.** These say which app a command
  exists in. Spec 007 adds Focus to them.
- **The seeded demo store** (`postio_storage::seed`) that `shot` already feeds
  through the real wiring. Its mail is fictional and uses reserved domains.
- **The screen sweep and its pairing with the design canvas**
  (`scripts/screens.sh`). This spec generalises it rather than standing a
  second system beside it (FR-030).
- **The UX authority's invariants** (`/ux-architect`): the six states,
  nothing is a dead end, and reviewing the flow rather than the screen. The
  visual authority (`/gtk-design`, the design canvas) is inherited the same
  way.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - An interaction, written once and filmed on Classic (Priority: P1)

A developer writes down an interaction: the setup, the steps (a command, a
raw key, typed text or a wait), and what should be true after each step. The
runner plays it against the Classic GTK app and produces a filmstrip. Each step
has its own frame, with the region that holds the keyboard outlined and named,
plus an observation of where things are and a pass or fail for every
machine-checkable expectation.

**Why this priority**: Writing the interaction down is what fixes "the agent
guesses". Filming it is what makes a sequence defect visible at all. Every
other story consumes what this one produces.

**Independent Test**: Write the storyboard for a defect that is still open,
such as #1252 or one the catalogue mining finds unfixed, and run it against
Classic. It fails on the right step, and the frame shows where the cursor or
keyboard went. Write the storyboard for #1687 (archive walks down) and run it
on current `main`. It passes, and its checks pin the exact row the cursor must
land on. Neither test is proved by re-breaking fixed code (constitution IV;
research R0).

**Acceptance Scenarios**:

1. **Given** a storyboard whose steps are commands, **When** it runs on
   Classic, **Then** each command is delivered by pressing Classic's own
   binding for it on this platform, so a broken binding fails the step.
2. **Given** a step, **When** it has run, **Then** the filmstrip shows the
   frame, the name of the region that holds the keyboard drawn on the frame,
   and the observation for that step.
3. **Given** a step with a machine check such as "the keyboard is in the list"
   or "the cursor is at the position it held before", **When** the observation
   disagrees, **Then** the step is marked failed and names the expected and
   the observed value.
4. **Given** a step after which the screen keeps changing, **When** the runner
   samples frames for a settle window, **Then** a change after the window has
   settled, or a blank frame anywhere in it, is reported as a jump with the
   frames that show it.
5. **Given** a storyboard run twice on the same build, **When** the results
   are compared, **Then** the observations are identical and the frames are
   identical.

---

### User Story 2 - A reviewer who did not build it, before the maintainer sees it (Priority: P1)

When a branch changes a GTK app, the storyboards for that app run, and an
independent design/UX reviewer judges the filmstrips. The reviewer is an agent
that did not write the change. It sees the storyboard's expectations, the
frames, the observations and the design screen each storyboard answers to, and
it never sees the implementer's account of the change. For each step it
returns pass, fail or question, citing the frame it judged. Failures go back to
the implementing session, which fixes them or contests them in writing. The
maintainer only receives what is left: contested findings, open questions, and
what changed.

**Why this priority**: This is the request: something that looks at the
interaction with a designer's eye before the maintainer has to.

**Independent Test**: Run the review over the calibration set (research
R10). These are storyboards whose prose expectations are known to be false of
today's correct behaviour, such as "after archive the cursor returns to the
top". The reviewer fails every one of them, citing the frame, and passes their
known-true twins. The implementing session receives those failures before
anything is offered to the maintainer.

**Acceptance Scenarios**:

1. **Given** a filmstrip, **When** the reviewer judges it, **Then** every
   verdict cites a step and a frame. A verdict without a citation is rejected
   as incomplete.
2. **Given** a step with a prose expectation, such as "the row below takes its
   place; nothing jumps", **When** the frames contradict it, **Then** the
   reviewer fails the step even though every machine check passed.
3. **Given** a frame showing something no expectation covers, such as a
   clipped label, a misaligned row, a dead end or a state the design canvas
   draws differently, **When** the reviewer sees it, **Then** it reports a
   finding against that frame.
4. **Given** a failed finding, **When** the implementing session resolves it,
   **Then** the resolution is either a fix, with a re-run that passes, or a
   written contest that the maintainer sees beside the finding.
5. **Given** a review, **When** it completes, **Then** its summary is recorded
   where it persists (the pull request), and the full page is kept beside the
   frames.

---

### User Story 3 - One storyboard, both GTK apps, and where they differ (Priority: P2)

A storyboard that describes a shared interaction runs on both Classic and
Focus. Moving down a list, archiving, undoing, opening and closing a message,
and leaving search are examples. Where an app legitimately behaves differently,
the storyboard says so for that app and that step. For example, Focus opens a
message as a dialog where Classic fills a reading pane. A storyboard that uses
a command only one app has runs only on that app, without anyone listing it.
The parity sheet shows each step as a row and each app as a column.

**Why this priority**: The maintainer asked for storyboards that apply to
several versions and storyboards that apply to one. Divergence between the apps
is itself a class of defect, and side by side is the only view that shows it.

**Independent Test**: Run the archive-walks-down storyboard on both apps. The
parity sheet shows both columns. Run a shared storyboard over an interaction
the two apps really do perform differently, with no override yet. The sheet
flags that step as diverging. Adding the override clears the flag.

**Acceptance Scenarios**:

1. **Given** a storyboard that names no apps, **When** it is loaded, **Then**
   it applies to every GTK app whose command registry provides every command
   it uses. The load reports which apps it applies to and why it excludes any
   others.
2. **Given** a storyboard that uses a Focus-only command, **When** the
   catalogue runs, **Then** it runs on Focus only, and nothing reports it
   missing on Classic.
3. **Given** a per-app override for one step's expectation, **When** that app
   runs the step, **Then** it is judged against the override. The other app is
   judged against the shared expectation.
4. **Given** a shared storyboard with no override on a step, **When** the two
   apps' observations for that step differ, **Then** the parity sheet marks the
   step as diverging, even if each app passed its own checks.
5. **Given** a storyboard that names an app explicitly, **When** that app
   cannot run one of its commands, **Then** loading fails and names the
   command. It does not quietly skip.

---

### User Story 4 - The maintainer sees only what changed (Priority: P2)

The page offered to the maintainer for a branch shows the storyboards whose
frames or observations differ from the branch's base. Each is shown before and
after, with the reviewer's verdicts and anything contested. Unchanged
storyboards are counted, not shown.

**Why this priority**: The catalogue will hold many storyboards across two
apps and several variants. The maintainer's attention is the scarce thing, and
a page of unchanged frames spends it on nothing.

**Independent Test**: On a branch that changes only the search overlay's
spacing, the page shows the search storyboards before and after, and reports
the rest as unchanged with a count.

**Acceptance Scenarios**:

1. **Given** a branch and its base, **When** the page is built, **Then** each
   storyboard is classified as changed, unchanged, new or removed against the
   base, and only changed and new ones are shown in full.
2. **Given** a changed storyboard, **When** it is shown, **Then** the base and
   branch frames for each changed step sit side by side, with the observation
   fields that changed named.
3. **Given** a maintainer comment rejecting an interaction, **When** the
   implementing session acts on it, **Then** the comment becomes a new
   storyboard or a new expectation in an existing one, in the same branch,
   before the branch is re-reviewed (FR-024).

---

### User Story 5 - A catalogue that starts from what already went wrong (Priority: P2)

The catalogue is seeded from the project's history. Every closed interaction
defect in the GTK apps becomes a storyboard that would have caught it, and
names the issue it came from. To this the catalogue adds the end-to-end flows
the UX authority names (read, walk a thread, reply, send, return), and over
time the interactions the maintainer has rejected.

**Why this priority**: The defects that already escaped are the best predictor
of the ones that will. A regression catalogue turns each one into a permanent
check, and shows the maintainer their own reports being honoured.

**Independent Test**: The catalogue holds a storyboard for each defect in the
Context table. Each one names its issue and says where its red evidence comes
from:
- **`base`**: it was red on the base of the branch that fixed it.
- **`open`**: it is red until the fix lands.
- **`pinned`**: it was written after the fix. Its checks pin the exact outcome
  the fix established, and it says it was never seen red.

**Acceptance Scenarios**:

1. **Given** a closed issue describing an interaction defect in a GTK app,
   **When** it is mined, **Then** it yields a storyboard naming the issue, or
   is recorded as not expressible, with the reason.
2. **Given** any storyboard, **When** it is loaded, **Then** it names its
   source: an issue, a spec requirement, a design canvas screen, a UX flow, or
   the maintainer. A storyboard with no source fails to load.

---

### User Story 6 - Every command does something you can see (Priority: P3)

For each GTK app, a generated pass invokes every command the app offers, in
every context where it is bound, from a representative starting state. It
checks that something observable changed. A command that changes nothing is
named. The terminal app already does this for its own commands
(`every_command_is_answered_here_or_by_the_dispatcher`). This story gives both
GTK apps the same guarantee, and it costs no reviewer time.

**Why this priority**: "I pressed it and nothing happened" is the bug that
reads as the application ignoring you. The macOS app has six open issues of
exactly this shape (#1571–#1576, #1705, #1706). It is cheap to generate, but
it depends on US1's observation.

**Independent Test**: Run the generated pass on Classic. Every command it
names as having no visible effect is either a real gap, which is filed or
fixed, or is added to the gap list with a reason. The pass's own logic is
proved red-green against synthetic observations in its unit tests.

**Acceptance Scenarios**:

1. **Given** a command bound in a context, **When** it is invoked from that
   context's starting state, **Then** the observation or the frame differs
   from the state before it, or the command is listed as a known gap with a
   reason.
2. **Given** a command that is listed as a gap, **When** it starts having a
   visible effect, **Then** the pass reports the gap list as stale.

---

### User Story 7 - Variants (Priority: P3)

A storyboard can ask to be run across variants. These include colour scheme
(light, dark, high contrast), window width (wide, narrow), row density, text
scale and body treatment. Each app declares which variant axes it supports, and
a storyboard runs on the cross product of what it asks for and what the app
supports.

**Why this priority**: A design that works only at one width or in one scheme
is unfinished, and #1179's defects showed up in only one of them. It multiplies
cost, so it comes after the catalogue proves itself.

**Independent Test**: Run the open-message storyboard with scheme set to light
and dark, on both apps. It produces four filmstrips, and the parity sheet
groups them by variant.

**Acceptance Scenarios**:

1. **Given** a storyboard asking for an axis an app does not support, **When**
   it runs on that app, **Then** the axis is ignored for that app, and the run
   says so.
2. **Given** a variant, **When** its frames are shown, **Then** the variant is
   named on every frame.

---

### Edge Cases

- **A step's command has no binding on this platform.** The step fails as
  "unbound". The runner does not fall back to invoking the command by name,
  because that would pass a broken binding.
- **The interaction is about where input is routed** (Tab, focus on launch,
  typing into a field). The runner delivers keys along the window's real focus
  chain, through every key controller on it, but not through the compositor.
  It catches a key swallowed by a dialog, or one lost on a removed widget. It
  cannot see window activation, input methods, or toolkit built-ins that are
  not on the chain. A step that depends on those is reported as **not covered
  by this delivery mode**, never as passed (FR-008; research R3).
- **A storyboard for Focus is run from `main`.** Focus does not exist on
  `main` until spec 007 lands. The run reports it as "app not present on this
  branch". It is neither a failure nor a pass.
- **The seed changes.** Every storyboard's observations shift. The base and
  branch comparison treats a seed change as a change to every storyboard, and
  says so once at the top of the page rather than once per frame.
- **A step never settles**, for example a spinner or an animation. The settle
  window ends at its limit, the step is reported as unsettled, and the last
  frame is kept.
- **The reviewer is unavailable or exceeds its budget.** The page is offered
  with the machine results and states that no review ran. It never presents an
  unreviewed run as reviewed.
- **A frame contains message content.** Frames show only the seeded fictional
  mail, so they are safe on a public pull request. A storyboard cannot point
  the runner at a real store (FR-027).

## Requirements *(mandatory)*

### Functional Requirements

**The storyboard**

- **FR-001**: A storyboard MUST be a plain text file in the repository,
  reviewable in a diff. It MUST contain:
  - a name
  - a source (US5)
  - a seed
  - optionally, the apps it applies to and the variants it asks for
  - an ordered list of steps
- **FR-002**: A step MUST be exactly one of the following. The format MUST
  NOT contain toolkit-specific steps.
  - a **command** from the shared vocabulary
  - a **raw key chord** in the shared chord syntax, including the
    platform-neutral modifier
  - **typed text**
  - a **wait**, either for a duration or for an observation to hold
  - an **environment event** from a fixed, neutral list: new mail arrives,
    the folder list changes, the connection drops or returns, a backfill
    progresses, or a body arrives
- **FR-003**: A step MAY carry **checks**: assertions over the observation
  (FR-010). Checks can be:
  - equal to a value
  - equal to the value the same field held at an earlier step
  - changed from the previous step
  - unchanged from the previous step
- **FR-004**: A step MAY carry a **prose expectation** for the reviewer.
- **FR-005**: A storyboard MAY carry **per-app overrides** that replace a
  step's checks or prose expectation for one app.
- **FR-006**: A storyboard MAY name the design canvas screen it answers to,
  per step or for the whole storyboard. The reviewer sees that screen beside
  the frame.
- **FR-007**: When a storyboard names no apps, the apps it applies to MUST be
  derived from the command registry: every app that provides every command it
  uses. When it names apps, an app that cannot run one of its commands MUST
  fail the load and name the command.

**Running**

- **FR-008**: A command step MUST be delivered by pressing the app's own
  binding for that command on the current platform. Every run MUST state its
  input delivery mode. Each storyboard and each step MAY declare that it
  depends on real input routing. Such steps MUST be reported as not covered
  under any delivery mode short of real input. Every step MUST also observe
  whether a real key would reach the window at all: the focused widget is
  mapped, inside the window, and not under a modal.
- **FR-009**: After each step, the runner MUST sample frames for a bounded
  settle window. It MUST report a step as having:
  - **jumped**, if the frame changed after the window had settled
  - **blanked**, if any sampled frame was empty or showed only the ground
  - **unsettled**, if it never settled within the limit
- **FR-010**: After each step, the runner MUST record a **neutral
  observation**, the same shape for every app:
  - the current view
  - which region holds the keyboard
  - whether the keyboard is in a text field
  - the cursor's identity and position
  - the selection's size
  - the topmost overlay, if any
  - the current notice, with its text, its tone and whether it offers undo
  - the reading surface's scroll position
  - the back stack's depth

  An app MAY add fields of its own under a namespace. Shared checks MUST NOT
  refer to them.
- **FR-011**: Each step's frame MUST show the region that holds the keyboard,
  outlined and named on the frame.
- **FR-012**: A run MUST be deterministic. On the same build, seed, variant
  and storyboard, the observations MUST be identical and the frames identical.
  To achieve this the runner MUST hold fixed:
  - the clock
  - the fonts
  - the window geometry
  - the animation settings
- **FR-013**: A run MUST NOT touch the network. It MUST use only the seeded
  store, which is created and thrown away in process.
- **FR-014**: The runner MUST exit non-zero, and say which storyboard and
  step, if any storyboard failed to load, failed to run, or produced no frame.
  A sweep that half worked and says nothing is how a blank pane gets reviewed
  as though it were a design decision.

**Both apps**

- **FR-015**: There MUST be a runner for Classic and a runner for Focus. They
  MUST read the same storyboard files and produce the same observation shape.
- **FR-016**: The parity sheet MUST lay out a shared storyboard with one row
  per step and one column per app (and per variant, when variants run). It
  MUST mark a step as **diverging** when the apps' observations differ and no
  per-app override explains the difference.
- **FR-017**: Each app MUST declare the variant axes it supports. A storyboard
  MUST run on the cross product of the variants it asks for and the variants
  the app supports, and MUST say which requested axes an app ignored.

**Review**

- **FR-018**: There MUST be a design/UX review, run as an agent that is not
  the implementing session. It MUST receive:
  - the storyboards
  - the filmstrips and observations
  - the design canvas screens the storyboards name
  - the branch's issue or spec acceptance

  It MUST NOT receive the implementing session's account of the change.
- **FR-019**: The reviewer MUST work under the UX authority's invariants and
  the visual authority's rules (`/ux-architect`, `/gtk-design`). It MUST return
  a verdict of pass, fail or question for every step of every changed
  storyboard, and may add free findings. Every verdict and finding MUST cite a
  storyboard, a step, an app, a variant and a frame. A verdict without a
  citation is incomplete and MUST be rejected.
- **FR-020**: Failed verdicts MUST go back to the implementing session. That
  session MUST resolve each one by a fix (with a re-run) or by a written
  contest, before the branch is offered to the maintainer.
- **FR-021**: The page offered to the maintainer MUST show:
  - changed and new storyboards, base and branch side by side (US4)
  - the reviewer's verdicts
  - every contest
  - every open question
  - a count of unchanged storyboards

  It MUST say plainly when no review ran.
- **FR-022**: A review's summary MUST be recorded on the branch's pull
  request. The full page and frames MUST be kept beside the run's output.
- **FR-023**: When a branch's diff touches a GTK app, landing it MUST warn if
  no storyboard run and review exists for that app at the branch's current
  commit. It MUST NOT refuse to land. The warning names the command that
  produces the run.

**The catalogue**

- **FR-024**: When the maintainer rejects an interaction, it MUST be captured
  as a new storyboard, or as a new expectation in an existing one, on the same
  branch, with the maintainer as its source, before the branch is re-reviewed.
- **FR-025**: The catalogue MUST be seeded from closed interaction defects in
  the GTK apps. Each defect becomes a storyboard naming its issue, or a
  recorded reason why it is not expressible. At a minimum the catalogue MUST
  cover every defect in the Context table.
- **FR-026**: The catalogue MUST include the end-to-end flows the UX authority
  names for reading, walking a thread, replying, sending and returning.
- **FR-027**: Storyboards MUST NOT name a real store, account or address. Any
  address in a storyboard MUST use a reserved domain. The personal-data check
  MUST scan storyboards.

**Generated coverage**

- **FR-028**: For each GTK app there MUST be a generated pass that invokes
  every command bound in every context from that context's starting state. It
  MUST report each command that produced neither an observation change nor a
  frame change, unless that command is on a gap list with a reason.
- **FR-029**: The generated pass MUST report a gap-list entry as stale when its
  command now has a visible effect.

**One system, not two**

- **FR-030**: The existing screen sweep (`scripts/screens.sh`) MUST become a
  view over the catalogue. A screen is a storyboard of a setup and no steps,
  paired with its design canvas screen. The sweep's table MUST NOT survive as
  a second list of screens.

**Later phases** (not built here, but the format must admit them)

- **FR-031**: The storyboard format and the observation shape MUST NOT assume
  GTK. The terminal app and the macOS app MUST be able to join as runners
  without any storyboard being rewritten. Joining means:
  - each registers as a further app in applicability (FR-007)
  - each fills the same observation (FR-010)
  - each declares its variant axes (FR-017)

### Key Entities

- **Storyboard**: one interaction. It holds a name, a source, a seed,
  applicability, requested variants, an optional design canvas screen, ordered
  steps and per-app overrides.
- **Step**: one input (a command, a chord, text or a wait), with optional
  checks, a prose expectation, an optional design canvas screen, and an
  optional marker that it depends on real input routing.
- **Observation**: the neutral record of where things are after a step
  (FR-010). It has the same shape for every app, plus namespaced app-specific
  fields.
- **Run**: one storyboard on one app, in one variant, at one commit. It holds
  frames, observations, check results, settle results and the input delivery
  mode.
- **Filmstrip**: a run's frames and observations laid out for reading.
- **Parity sheet**: runs of one storyboard across apps and variants, step by
  step, with divergences marked.
- **Review**: verdicts and findings over a set of runs. Each cites a frame,
  and each carries a resolution: fixed, contested or open.
- **Review page**: what the maintainer receives for a branch. It holds base
  and branch runs for changed storyboards, the review, and the counts.
- **Gap list**: per app, the commands known to have no visible effect yet,
  each with a reason.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Every defect in the Context table has a storyboard. Its red
  evidence comes from the base, from an open defect, or is declared `pinned`;
  never from re-breaking a fix (constitution IV; research R0). A storyboard
  with `base` or `open` proof fails on the right step where the defect is
  present. Every storyboard passes on current `main`, except those for open
  defects. Two kinds are exceptions, and neither is ever
  reported as passed:
  - Defects that need routing beyond the focus chain are reported as not
    covered (FR-008).
  - Pointer defects such as #1433 (a scroll) are recorded as not expressible
    until pointer input exists.
- **SC-002**: A single storyboard on one app gives its filmstrip in under 15
  seconds on the maintainer's workstation, from a warm build. The whole GTK
  catalogue on both apps, in default variants, runs in under 5 minutes.
- **SC-003**: Running the same storyboard twice on the same build produces
  identical observations and identical frames, in 100% of runs across the
  catalogue.
- **SC-004**: Over the first month of use, the share of GTK interaction
  defects the maintainer reports after a branch was offered to them falls by
  at least half. The baseline is the interaction defects reported in
  September 2026.
- **SC-005**: Every verdict on a review page cites a frame, and a reader can
  reach that frame in one click.
- **SC-006**: The maintainer's review page for a typical branch shows 10 or
  fewer storyboards in full.
- **SC-007**: Every rejection the maintainer makes on a review page or pull
  request becomes a storyboard or expectation before the branch is
  re-reviewed. A rejection never has to be made twice.

## Assumptions

- **Focus lives on its own branch.** Focus is on `feature/postio-focus` and
  does not land on `main` until the maintainer says so. The format, the
  observation, the Classic runner, the review and the page land on `main`
  through this branch. The Focus runner is built on `feature/postio-focus`,
  which picks up the rest when it rebases. On that branch, ADR 0043 places GTK
  code shared by both apps in `postio-widgets`, so the GTK half of the runner
  that both apps share moves there with it. Until then it lives beside
  Classic's `shot`.
- **Input is delivered along the focus chain in this phase** (research R3).
  This is closer to real input than direct dispatch, but it is not real.
  Real input routing is a later phase. One way would be injecting input through the headless
  compositor's remote-desktop interface. Its feasibility is unproven, so this
  spec requires honesty about the limit (FR-008) rather than the capability.
- **Review output is not secret.** Fixture mail is fictional and public, so
  frames and review summaries are safe to put on a public pull request.
- **The reviewer is an agent session** loading the existing UX and visual
  authorities. A model-judged review is advisory. The machine checks are the
  part that gates, and they only warn at landing (FR-023).
- **Landing warns rather than refuses** at first, so the catalogue can grow
  without blocking work. Making it refuse is a later decision for the
  maintainer, once the false-positive rate is known.
- **No backwards compatibility.** `screens.sh`'s table is replaced, not kept
  beside the catalogue (FR-030).
- **Out of scope here:**
  - the terminal and macOS runners (FR-031 keeps the door open)
  - real input injection
  - a random-walk "monkey" pass over the invariants (the keyboard is never on
    nothing, a pane is never blank, Escape always goes back)
  - pointer interactions (click, drag, scroll), which need the same input
    capability as real routing

  Each comes after this spec, in that order of likely value.
