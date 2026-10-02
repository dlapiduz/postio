# Research: Storyboards

**Feature**: [spec.md](./spec.md) | **Plan**: [plan.md](./plan.md) | **Date**: 2026-10-01

Each decision below records what was chosen, why, and what was rejected. File
references are to `main` at `98ba8a8e`, unless marked `focus:`, which means
`origin/feature/postio-focus` at merge-base `b567d14`.

---

## R0. Proving a storyboard honestly (constitution IV)

**Decision.** A storyboard's red evidence comes from one of three places, and
never from re-breaking fixed code:

1. **The base.** A storyboard written for a defect being fixed on a branch is
   run against the branch's base, where the defect still lives (R8). The base
   run is its red; the branch run is its green. This is the ordinary case from
   the moment the runner exists.
2. **An open defect.** A storyboard for a defect that is still open is red on
   `main` until the fix lands. Examples: #1252, and the Focus key-delivery
   rows that are unfixed on `main`.
3. **A defect already fixed before the runner existed.** There is no red to
   observe, so the storyboard is tightened instead. Its checks pin the exact
   outcome the fix established: the cursor's index and identity, the named
   region, the view. "Something happened" is not enough. Each such storyboard
   is labelled `proof = "pinned"`, so a reader knows it was never seen red.

The spec's SC-001 and Independent Tests said "a build with the defect
reintroduced", which is exactly what constitution IV forbids. They are amended
to the three sources above in this branch.

**The reviewer gets its red the same way, without breaking code.**
`storyboards/calibration/` holds storyboards whose prose expectations are
known to be false, or known to be true, against today's correct behaviour. The
reviewer must fail the first kind and pass the second (R10).

**Rejected.**
- *Re-break the fix in a scratch branch.* Constitution IV forbids it.
- *Build the runner at each fix's parent commit.* The runner did not exist
  then, and back-porting it is far more work than the evidence is worth.
- *`cargo-mutants` over the GTK crates.* It is accepted for pure crates
  (`docs/engineering-notes.md` § Mutation testing). Over GTK it would be
  thousands of window launches, and its mutants are not the defects that
  happened.

---

## R1. Where the code lives

**Decision.** There are four places. Each one is where its dependencies
already are.

| Piece | Crate | Why there |
|---|---|---|
| `Observation` and its enums, the neutral record of a step | **`postio-ui`**, a new `observe` module | Toolkit-free, already has `serde`, and every frontend depends on it: GTK, Focus, the TUI, and the macOS app through `postio-ffi`. It is the shared vocabulary of what a person sees, which is what `postio-ui` is for. |
| The format, loader, applicability, checks, diff, parity, review schema and page | **new `postio-storyboard`** (lib + bin) | Pure: no GTK, no database engine, no tokio. It is unit-testable in milliseconds, which "iterate at the cheapest layer" asks for. It is a development tool, so it stays out of every shipped crate's graph. |
| GTK half, shared by both GTK apps: chain delivery, typing, frame sampling, focus outline, settle detection, keyboard reachability | **`postio-gtk::storyboard`** on `main`. On `feature/postio-focus` it moves to `postio-widgets` by a pure `git mv`. | `capture.rs` is already in `postio-gtk` on `main` and in `postio-widgets` on Focus (focus: `crates/postio-widgets/src/capture.rs`). ADR 0043 puts GTK code that both apps share in `postio-widgets`, and Focus may not depend on `postio-gtk`. |
| Each app's `observe()` and runner | **Classic**: `Window::observe()` in `postio-gtk`, and the runner in `postio-app` as `examples/storyboard.rs` plus an `app_suite` case. **Focus**: `FocusWindow::observe()` in `postio-focus`, and `examples/storyboard.rs` plus a `focus_suite` case. | `observe()` must read the private `key_context()` and `is_typing()` (`window.rs:2960, 2990`; focus: `window.rs:729`), so it is a method on the window itself. The runner needs a seeded store, which only the app crates may hold (`check-crate-boundaries.py`; `shot.rs` doc § "Why this lives in postio-app"). |

**`postio-storyboard`'s dependencies.** `postio-core` (commands, registry,
`Keymap`), `postio-config` (`expand_mod`), `postio-ui` (`Observation`,
chords), `serde`, `toml`, `serde_json` and `blake3`. All of these are already
workspace dependencies. A `RULES` row in `check-crate-boundaries.py` bans
gtk4 and the database engines from it. Tokio is not banned: `postio-core`,
whose registry R7 needs, already depends on it. The script only demands rows for
listed crates, so the row is added deliberately.

**Rejected.**
- *`Observation` inside `postio-storyboard`.* Every frontend's production code
  would then depend on a development tool to say where its keyboard is.
- *One runner crate depending on both apps.* ADR 0043 forbids
  `postio-focus ↔ postio-gtk`. It would also make Classic's runner wait on
  Focus's branch.
- *The runner inside `shot`.* `shot` is 1,492 lines of mode words already. The
  runner shares `shot`'s setup (R11), not its argument parser.

---

## R2. The storyboard file format

**Decision.**
- **TOML**, one file per storyboard, under `storyboards/<surface>/<name>.toml`
  at the repository root.
- **Steps** are `[[step]]` tables.
- **Checks** are an inline table shaped like the observation:
  - a literal value means *equals*;
  - `{ same_as = <step> }` means equal to the value at that step;
  - `{ changed = true }` and `{ unchanged = true }` compare with the step
    before;
  - `{ absent = true }` means the field has no value.
- **Overrides** are `[app.<name>.step.<id>]` tables.

The grammar is in [contracts/storyboard-format.md](./contracts/storyboard-format.md).

**Why TOML.**
- It is already the configuration language (`[keys]` uses the command ids a
  storyboard names), and `toml` is a workspace dependency.
- It takes comments, which is where a storyboard says *why*.
- It diffs line by line, and the maintainer, the reviewer and a Swift test can
  all read it as data.

**Sentinels are tables, not strings.** A string like `"changed"` would collide
with a notice whose text is literally "changed".

**Rejected.**
- *A Rust table like `app_suite`'s `CASES`.* It is not data, the maintainer
  cannot review it as data, and the macOS runner cannot read it.
- *YAML.* It is not a dependency, and indentation errors read as a different
  storyboard rather than a broken one.
- *Gherkin.* Its prose steps need a step-definition layer per frontend, which
  is exactly the per-frontend divergence the shared command vocabulary already
  removes.

---

## R3. Delivering input

**Decision.** There are three named delivery modes. Every run records which
one it used (FR-008). The GTK runners use **`chain`** by default.

| Mode | What it does | What it can see | What it cannot see |
|---|---|---|---|
| `direct` | Calls the window's key handler (`Window::handle_key`, `window.rs:2264`; focus: `FocusWindow::handle_key`, `window.rs:611`). | Command resolution and dispatch. | Which widget the key reaches. |
| **`chain`** | Emits `key-pressed` on every `EventControllerKey` along the focus chain, in GTK's own phase order: capture from the toplevel down, target, then bubble back up. It starts at the widget that really holds focus, and passes through dialogs and popovers. This is Focus's `support::deliver_with` (focus: `tests/focus_suite/support.rs:731-950`, T195/T200), promoted out of a test helper into the shared GTK half. | Swallowed keys: a dialog's controller consuming them, or focus left on a removed or unmapped widget (focus: `92a093b8`, `a19c4bbb`). It also sees keys reaching widget-internal controllers, because `observe_controllers()` lists those too. | Which toplevel the compositor considers active. Input-method composition. GTK's built-in key bindings that are not controllers on the chain. |
| `real` | Input injected through the compositor. Not built in this spec. | Everything. | Nothing, but unproven (R15). |

**Steps that depend on what `chain` cannot see** are marked
`routing = "real"`. They are reported `not covered (delivery: chain)`, never
passed. Examples: native Tab traversal that Postio does not handle itself, and
window activation.

**Keyboard reachability** is observed on every step whatever the mode
(`keyboard.reachable`, data model). It answers: is the focused widget mapped,
inside this toplevel, and not covered by a modal? It is a cheap guard on the
class `chain` exists for, and it makes "the keyboard is on nothing" a check,
not a guess.

**Command steps.**
1. Ask the app for its current key context.
2. Look up the app's binding for the command in that context on this
   platform, using `postio_ui::keymap::Keymap::binding_for(KeyContext, cmd)`
   (`postio-ui/src/keymap.rs:650`) over `postio_core::Keymap::defaults()`,
   after `expand_mod` (`postio-config/src/keys.rs:191`).
3. Turn the chord into `(gdk::Key, ModifierType)` through `Key::keysym_name()`
   (`postio-ui/src/keymap.rs:248`), applying the uppercase-to-Shift unfolding
   that `gtk_accelerator` uses (`postio-gtk/src/keymap.rs:50-77`).
4. Deliver it.

If no binding exists in the current context, the step fails as
`unbound in <context>`. **It never falls back to dispatching the command by
id**, because that would pass a broken binding (spec, Edge Cases). The
chord-to-gdk conversion goes in the shared GTK half. Neither app has one today:
tests call `gdk::Key::from_name` by hand (`gtk_composer.rs:41`).

**Typed text.** Text goes to the widget that holds the keyboard:
- a `gtk::Editable` is changed at its cursor position;
- a `gtk::TextView` through `buffer.insert_at_cursor`;
- the composer's web body through the existing `Composer::test_body_eval`
  hook (`composer.rs:3382`).

If the keyboard is on none of these, the step fails as `nothing to type into`,
which is itself the #1473 check. This is what the existing tests already do
(`gtk_finder.rs:400-424`). The runner just does it in one place.

**Rejected.**
- *`direct` as the default.* It is blind to the defects that most need a
  storyboard.
- *`gtk_test_widget_send_key` and other synthesised GDK events.* GTK 4 removed
  them (#424, #437).
- *`ydotool`.* It drives the real seat through `uinput`, so it would type into
  the maintainer's session, not the headless compositor.

---

## R4. Frames, settling, jumps and blanks

**Decision.** After the input is delivered, the runner:

1. drains the main context (`iteration(false)` until idle);
2. then samples frames on the window's frame clock (`add_tick_callback`), each
   one through `capture::texture_within`.

The step is **settled** when *K* consecutive samples are identical, by blake3
of the pixels (*K* = 6, about 100 ms). After it settles, the runner keeps
watching for a further `watch` window (default 300 ms):

| Result | Meaning |
|---|---|
| **jumped** | A frame changed during the watch window. The frames before and after are kept. |
| **blanked** | A sampled frame was empty (`presenting()` false; `capture.rs:272`) or a single colour. The frame is kept. |
| **unsettled** | The step never settled within `settle_max` (default 3 s). The last frame is kept. |

Only distinct frames are written to disk. The step's frame is the settled one.

A step may say `settle = { until = { <check> } }` to wait for content such as
a body arriving before the settle clock starts. This is how a legitimate load
is told apart from a jump.

**The focus outline.** The window's render node is wrapped in a
`gsk::ContainerNode`, together with a `BorderNode` over the bounds of the
focused widget. A caption is drawn as a text node with the region's name. This
needs one new public function in `capture.rs`,
`texture_with(&window, overlay: impl FnOnce(&gtk::Snapshot))`, because
`drawn()` is private (`capture.rs:239`).

- The outlined frame is written beside the plain frame, never instead of it.
- The plain frame is the one the base-versus-branch comparison hashes, because
  an outline that moves with focus would otherwise count as a visual change.
- The outlined frame is the one the reviewer and the maintainer look at.

**Measured (T005).** One capture of a 1280×800 window costs about 25 ms
median and 37 ms p95, which is more than a frame. Sampling therefore takes
every **second** tick (stride 2), and the stride is recorded in every run.
The numbers are in `docs/notes/2026-10-01-what-a-storyboard-capture-costs.md`.

**Rejected.**
- *A fixed sleep per step, as `shot` does* (`shot.rs:1282, 1319, 1370`). A
  sleep is either too short, and flaky, or too long, and slow, and it can never
  report a jump.
- *Screen recording through the compositor's screencast.* Real time, lossy, and
  a second capture path to keep honest.

---

## R5. Determinism

**Decision.** The runner re-executes itself into a hermetic environment, the
same way `focus_suite` does (focus: `focus_suite/main.rs`,
`POSTIO_FOCUS_SUITE_HERMETIC`). It fixes:

| What | How |
|---|---|
| **The clock** | A new seam, `postio_ui::clock`, holds a process-wide `now()`. It defaults to `Local::now()`, and the runner can `freeze` it. The seven GTK call sites that pass `Local::now()` to `postio_ui::row::timestamp` switch to it: `row.rs:101, 129, 894`, `thread_row.rs:408`, `reader/message_header.rs:292` and `conversation.rs:1678, 1783`. Focus's equivalents switch on its branch. `timestamp` already takes `now` as a parameter (`postio-ui/src/row.rs:111`), so the seam is only at the call sites. The frozen time is the seed's anchor, 2026-06-01 09:00 UTC (`seed.rs:107`), plus one day, so relative dates read as they do in the design. |
| **Time zone and locale** | `TZ=UTC`, `LANG=C.UTF-8`. |
| **Animations** | `gtk::Settings::set_gtk_enable_animations(false)`. Focus's `shot` already does this (focus: `shot.rs:569`). Nothing on `main` sets it. |
| **Fonts** | `fonts::install()` adds the embedded faces (`fonts.rs:54`). The runner also sets `FONTCONFIG_FILE` to a config holding only those faces. Otherwise a system font update would change every frame. |
| **Geometry** | The variant's size goes through `set_default_size`. The runner refuses a clamped size and reports it, the same way `shot` does (`size_mismatch`, `shot.rs:934`). |
| **Renderer** | **`GSK_RENDERER=cairo`, pinned and recorded in each run.** Measured at T005: the default renderer repeats itself within one process but gives different bytes in two, and cairo gives the same bytes in both. The cost is that a defect living only in the GL path does not show in a storyboard. |
| **Seed** | Deterministic by construction (`seed_small(&db, n)`, with a seeded `recency`). Wall-clock leaks such as `queue_send(.., Utc::now())` and "last sync 12s" read the clock seam instead. |

**Rejected.**
- *Comparing frames with a perceptual tolerance.* It hides exactly the 1 px
  rule and the 4 px scroll (#1679) the catalogue is for. A tolerance is only
  the fallback if the renderer proves non-deterministic.
- *Shifting the seed's dates to today.* Every frame would then change at
  midnight.

---

## R6. The observation

**Decision.**
- The fields are in [data-model.md](./data-model.md) § Observation, and the
  per-app mapping is in [contracts/observation.md](./contracts/observation.md).
- `Observation` is a plain `serde` struct in `postio_ui::observe`.
- Each app builds it in one method from accessors it already has. The
  surveys found nearly all of them public:
  - **Classic**: `shell().focused_pane()`, `reader_occupant()`,
    `list().cursor()`, `cursor_id()`, `selection()`, `finder().is_open()`,
    `mode()`, `cheatsheet().is_visible()`, `toast().showing()`,
    `composer().focused_field()`, `conversation().focused_index()`, and
    `reader().view()`'s vadjustment.
  - **Focus**: `cursor_row()`, `selection()`, `reading()`, `bar()`,
    `places()`, `open_picker()`, `row_menu()`, `digest()`, `filtered()`,
    `toast_showing()`, and `banner_showing()`.

**Two small gaps are closed in passing.**
- **Notice tone and undo.** Neither app's toast exposes them; both would have
  to be guessed from a button label. Each toast wrapper gains
  `tone() -> Option<Tone>` and `offers_undo() -> bool`, recorded when the
  toast is shown.
- **Back depth.** Classic has no back stack (`CommandId::Back` is a cascade,
  `window.rs:2732`), and Focus has none either. The field is `Option` and
  reads as `not observed by <app>`. A check on it in a shared storyboard
  becomes `not applicable` for that app, not a failure.

**Region names are neutral**: `sidebar`, `list`, `reader`, `conversation`,
`composer`, `search`, `palette`, `picker`, `cheatsheet`, `settings`, `dialog`,
`menu`, `banner`, `none` and `other`. "Which region holds the keyboard" is
computed from the toplevel's focus widget by walking up to the first ancestor
the app names as a region. That is the approach in focus:
`support::focus_path` (`support.rs:1211`). The full widget path is recorded
too, as an informative field no check may name, because widget paths are not
stable across refactors.

**Rejected.**
- *Extending `SharedState` with these fields.* It holds what commands aim at,
  and is serialised as `StateSnapshot` for that purpose. Pane focus and
  toasts are presentation, and putting them there would make every toast a
  state change.
- *Reading the accessibility tree.* `pyatspi` is not installed. It would also
  be a second, slower path to facts the windows already hold.

---

## R7. Which apps a storyboard applies to

**Decision.** `postio-storyboard` defines `App { Classic, Focus, Terminal,
Macos }`, and computes `provides(app, CommandId)` from the command registry:

- **On `main`**: Classic means commands that are not `Requirement::Terminal`.
  Focus is `not present on this branch` (spec, Edge Cases).
- **On `feature/postio-focus`**: it is
  `registry::get(id).requires.offered_by(Frontend::…)` (focus:
  `registry.rs:218, 243`). The Focus lane makes that one-line change.

Applicability is therefore static. The loader and the lint answer it with no
runner built.

A command the registry offers but the app never wired is still "provided".
That gap is what the generated pass is for (US6).

---

## R8. Base versus branch

**Decision.** `scripts/storyboards.sh base` runs **the branch's storyboards
against the base's code**:

1. Make a detached worktree at `git merge-base HEAD origin/<base>`, where
   `<base>` is the recorded `postio-base`, so Focus lanes compare against
   `feature/postio-focus`.
2. Seed its `target/` by reflink from the current tree, exactly as
   `issue-claim.sh` seeds a fresh tree (#1102).
3. Build the runner there.
4. Cache the runs under `~/.cache/postio/storyboards/<app>/<base-sha>/<storyboard-hash>/`.

Because runs are deterministic (R5), a cached base run is reused until the
base or the storyboard changes.

Running the branch's storyboards, not the base's, is what turns a new
storyboard for a defect into its own red evidence (R0, source 1). It also makes
"changed" mean *behaviour* changed, not *file* changed.

If a storyboard uses something the base runner cannot do, such as a step kind
added on the branch, its base run is `unavailable`. The page then shows it as
new.

**Rejected.**
- *A CI artifact from `main`.* Focus lanes have no such artifact. It also adds
  a download and a staleness question to every review.
- *`scripts/run-isolated.sh`.* It links `--release`, which CLAUDE.md says
  never to do while other sessions build.

---

## R9. The review

**Decision.** There are two pieces.

- **A skill, `/ux-review`, run by the implementing session.** It:
  1. runs the affected storyboards and the base (R8);
  2. builds a **bundle**: the changed runs, their frames and observations, the
     design screens named, and the acceptance text, taken from
     `gh issue view` or the spec's acceptance scenarios;
  3. launches the reviewer;
  4. validates the verdicts;
  5. hands failures back to its own session to fix or contest;
  6. builds the page.
- **An agent definition, `.claude/agents/ux-reviewer.md`.** It runs on Opus,
  because design lanes do. Its tools are Read, Glob, Grep and Write. It is
  launched **fresh, never as a fork**, so it inherits none of the
  implementer's conversation.

**Keeping the implementer's account out.** `postio-storyboard prompt <bundle>`
writes the reviewer's whole prompt from a fixed template. The skill passes that
prompt verbatim, so the implementer has nowhere to add its own spin (FR-018).

The prompt tells the reviewer to read `/ux-architect`'s and `/gtk-design`'s
`SKILL.md` and the named design screens, then every outlined frame.

**The output** is `verdicts.json` in the schema in
[contracts/review.md](./contracts/review.md).
`postio-storyboard verdicts check` rejects:
- any verdict without storyboard, step, app, variant and frame;
- any frame path that does not exist;
- any step of a changed run that has no verdict (FR-019).

A rejected review is re-asked once with the validator's message, then reported
as `review incomplete`.

**Batching.** One reviewer per app and surface, with at most 60 frames each and
at most 4 in parallel. That keeps each reviewer's context on one surface, and
it stays inside this session's workflow-size guideline.

**Contests.** The implementer writes `contests.toml` in the bundle, one entry
per contested verdict with a reason. A fix is proved by re-running only the
failed storyboards, then re-reviewing only them (FR-020).

**Rejected.**
- *A reviewer that also reads the diff.* It drifts into reviewing code, which
  `/code-review` already does. It also reads intent off the diff, which is the
  implementer's account by another route.
- *One reviewer for everything.* Its context fills with frames from unrelated
  surfaces, and its verdicts get vaguer as it goes.

---

## R10. Calibrating the reviewer

**Decision.** `storyboards/calibration/*.toml` declare `calibration =
"must_fail"` or `"must_pass"`. A `must_fail` storyboard carries a prose
expectation that is false of today's correct behaviour. An example is "the
cursor returns to the top of the list after archive", which is the #1687 defect
stated as if it were intended.

`/ux-review --calibrate` reviews the set and reports accuracy. It runs:
- whenever the reviewer's prompt template or agent definition changes;
- during a `/steward` pass, as a measurement. CI has no model to run a
  reviewer, so this cannot be a nightly CI job.

A reviewer that passes a `must_fail` storyboard has failed calibration, and the
page says so at the top.

**Why.** This is the reviewer's own red/green evidence (R0). It is also the
only way to know whether a prompt change made it better or merely different.

---

## R11. Seeds and presets, shared with `shot`

**Decision.** `shot.rs`'s roughly 600 lines of setup (`populate`, the `show_*`
helpers, `shot.rs:125-715`) move into a library module, `postio_app::demo`.
It is behind a `demo` feature, which turns on `postio-storage/test-support`,
needed because `seed` and `test_support::memory()` are `cfg(feature =
"test-support")` (`postio-storage/src/lib.rs:56, 65`). Both `shot` and the
runner use it, with `required-features = ["demo"]`. Focus's `shot` setup moves
the same way on its branch.

**Neutral seed names** are store conditions a step cannot produce:

| Seed |
|---|
| `small` (the default) |
| `empty` |
| `locked` |
| `first-run` |
| `two-accounts` |
| `outbox` |
| `draft-left-over` |
| `long-thread` |
| `thirty-threads` |
| `backfilling` |

**Presets** are window conditions with no command to reach them, such as
`settings/account-form`, which `shot` hand-feeds. They are named by the app,
and a storyboard that names a preset an app lacks is not applicable to it.

Everything a command can reach is a step, not a preset. `shot`'s `selected`,
`open`, `reply` and `compose` become steps.

---

## R12. The screen sweep becomes the catalogue (FR-030)

**Decision.**
- Each row of `screens.sh`'s table becomes `storyboards/screens/<name>.toml`.
  Each has zero steps, a seed or preset, variants, and `design = "<canvas
  screen>"`.
- `scripts/storyboards.sh screens` builds the same contact sheet from them.
- `screens.sh` is deleted. The `/gtk-design` skill's § 6 and its other
  mentions are updated in the same commit.
- `shot` stays, as the one-picture tool `/gtk-design` iterates with. It now
  shares the setup (R11) but keeps no screen table.

---

## R13. Where results go, and the landing warning (FR-021–023)

**Decision.**

**The local page.**
- Runs, frames, the bundle, verdicts, contests and `index.html` live under
  `Design/review/<branch>/`, which is already gitignored (`.gitignore:30`).
- `scripts/storyboards.sh page --open` opens it. The session may also publish
  it as a private claude.ai page when the maintainer is away from the
  workstation. That is an offer, never the default.

**The PR summary.**
- `summary.md` is text only: counts, changed storyboards, verdicts, contests
  and questions. It is safe on a public PR, because the frames are not in it
  and the fixtures are fictional.
- `issue-land.sh` puts it in the PR body on create, and as a PR comment when
  it pushes to an existing PR whose summary key changed.

**The review key.**
- The key is blake3 over:
  - `git rev-parse HEAD:<crate>` for each crate of the affected app (Classic:
    `postio-gtk`, `postio-app`, `postio-ui`; Focus: `postio-focus`,
    `postio-widgets`, `postio-ui`);
  - `HEAD:storyboards`.
- The key survives a rebase that does not touch those trees. A commit sha does
  not survive a rebase, and `issue-land.sh` rebases on every attempt.

**The warning.** It follows the existing `VERIFY_NOTE` / `VERIFY_LABEL`
pattern (`issue-land.sh:1031-1056`).
- **When**: the changed crates include an app's crates and no `summary.md`
  carries the current key.
- **What**:
  - a `> [!WARNING]` block in the PR body naming `/ux-review`;
  - the label `interactions-unreviewed`.
- It never refuses the landing (FR-023).

---

## R14. Where the catalogue checks run

**Decision.**

| Where | What runs |
|---|---|
| `app_suite` | One case, `storyboards`, runs every Classic storyboard with checks on and frames off: settle and blank detection still run, nothing is written. |
| `focus_suite` | The equivalent case, on the Focus branch. |
| `postio-storyboard` unit tests | Load and lint the whole catalogue in the **sanity tier**, so a malformed storyboard fails in seconds. Linting covers sources, reserved domains, known command ids, applicability and overrides that name real steps. |

**Why the suites.** `app_suite` is an integration suite, which runs nightly
and as the release gate (CLAUDE.md § "What runs the integration suites"). That
is where a broken join is first read. One case keeps `app_suite`'s `--list`
contract untouched; `list_contract.rs` is what notices a change to it.

**The time budget.** If the case outgrows the four-minute landing budget it
goes on `SLOW` / `POSTIO-MEASUREMENT` like any other.

**`check-no-personal-data.py`** scans tracked files (`tracked_files`, `:92`).
`storyboards/` is not in `SKIP_PATHS`, so FR-027 is met by adding a test that
the scan covers it.

---

## R15. Later, and why not now

These are out of scope. They are recorded so the format admits them.

- **Real input** (`delivery = "real"`). Through mutter's
  `org.gnome.Mutter.RemoteDesktop` interface or libei, against the headless
  compositor the tests already run on (`scripts/headless-runner.sh:162`). It
  is unproven whether a `--headless --virtual-monitor` mutter exposes
  RemoteDesktop on the runner's private bus. The spike for it is the first
  task of a later spec.
- **Pointer steps.** These cover click, scroll and drag. They wait on real
  input, because GTK 4 cannot synthesise them either (#424, #437). The
  catalogue seed's four pointer rows are recorded as not expressible.
- **The terminal runner.** It is the cheapest to add: `update()` is pure,
  `shot` already turns key sequences into frames, and its getters already
  cover the observation (`postio-tui/src/app.rs`).
- **The macOS runner.** A Swift test reads the storyboard plan through
  `postio-ffi` and renders with `NSHostingView` and `cacheDisplay` on CI's
  `macos-latest`. Its frames arrive as CI artifacts.
- **The monkey walker.** Random command sequences checked against the
  invariants alone: the keyboard is reachable, no frame is blank, and Back
  always leaves.
