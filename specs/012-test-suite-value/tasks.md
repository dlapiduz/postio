# Tasks: Test suite value

**Input**: `specs/012-test-suite-value/` — spec.md, plan.md, research.md (R1–R8), data-model.md, contracts/, quickstart.md
**Branch**: `feature/test-suite-value`, worktree `~/src/postio-worktrees/test-suite-value`

**Tests are required.** Constitution IV says: red, green, repeat. Every script and check gets its `scripts/tests/test-*.py` self-test written first and seen failing. `scripts/run-self-tests.sh` discovers these self-tests, and `scripts/check.sh` discovers `scripts/checks/check-*.py` by glob. So a new file needs no registration beyond existing.

**Commits**: one per task, with `Refs: specs/012-test-suite-value T0nn`. A cleanup commit also pastes its removal-check line, its coverage result, and the case and CPU-second counts before and after (FR-008).

## Format: `[ID] [P?] [Story] Description`

- **[P]** means the task can run in parallel: it touches different files and has no dependency on an incomplete task.
- **[Story]** names the user story in spec.md.

---

## Phase 1: Setup

- [X] T001 Create `scripts/lib/test_audit.py` with an empty module docstring that names its three consumers: `test-inventory.py`, `test-coverage-sets.py` and `check-assertion-smells.py`. Add `target/test-audit/` to nothing. It is already under the ignored `target/`; confirm with `git check-ignore target/test-audit/x`.
- [X] T002 [P] Create a fixture workspace for the self-tests at `scripts/tests/fixtures/test-audit/`:
  - a two-crate cargo-free tree of `.rs` files: a `tests/<x>_suite/main.rs` with `mod` lines, one unit-test module, and one harness-free `CASES` table;
  - a recorded `nextest-list.json` and `junit.xml` that match it.

  Use the `nextest list --message-format json` shape from a real run on `postio-config`, trimmed. This lets every self-test run without cargo.

---

## Phase 2: Foundational (blocks US1, US2 and US5)

The shared parsing in `scripts/lib/test_audit.py`, with research R5 and R6 as the spec.

- [X] T003 Write `scripts/tests/test-test-audit-lib.py` against the T002 fixture. It asserts:
  - a suite case maps to `tests/<suite>/<mod>.rs`;
  - a `mod.rs` module maps to its file;
  - a unit case maps to `src/<path>.rs`;
  - a harness-free case maps to its `CASES` module;
  - a name defined twice maps to `ambiguous`;
  - a header citing `specs/009-focus-macos` plus a fn doc citing `FR-021` gives the ref `009/FR-021`;
  - a bare `FR-021` with no spec in scope gives `?/FR-021`;
  - `#1234` is kept.

  See it fail, because the module is empty.
- [X] T004 Implement `parse_nextest_list(json) -> list[Case]`, `case_file(case, root)`, `parse_refs(path, fn_name)` and `fn_body(path, fn_name)` (brace-matched) in `scripts/lib/test_audit.py` until T003 passes.
- [X] T005 Extend T003 with the smell cases, red first:
  - `constant-only`;
  - `is-ok-only` (`assert!(x.is_ok())` alone; `.is_some()` alone);
  - `mock-count-only`;
  - `display-skip`;
  - `untraced`;
  - a negative case for each, such as `assert_eq!(x, CONST)` where `x` is computed, which is *not* constant-only.

  Then implement `smells(body, refs) -> set[str]` in `scripts/lib/test_audit.py`.
- [X] T006 Extend T003 with layer and cost cases, red first:
  - postio-focus → `controller`/1;
  - postio-ffi → `frontend`/2;
  - postio-gtk focus_suite → `frontend`/3 (display);
  - a file with `POSTIO-MEASUREMENT:` → `measurement`;
  - a body reading `CARGO_MANIFEST_DIR` + `..` → `meta`;
  - postio-storage → `rule`/0.

  Then implement `layer(case, file, body) -> (layer, cost_rank, display)` (data-model **Case**, FR-002).

**Checkpoint**: `scripts/tests/test-test-audit-lib.py` is green. Nothing so far has run cargo.

---

## Phase 3: User Story 1, the inventory (P1) 🎯 MVP

**Goal**: one command, one row per case, with layer, cost, refs, smells and parity clusters. Contract: `contracts/test-inventory.md`.

**Independent test**: quickstart §1. The row count equals `cargo nextest list -p postio-config`'s count.

- [X] T007 [US1] Write `scripts/tests/test-test-inventory.py`. It runs the script with `--list-json` and `--junit` pointed at the T002 fixtures (a test-only flag that skips cargo) and asserts:
  - the CSV columns and order match data-model **Case**;
  - a case missing from the JUnit file has `duration_s=unknown`;
  - an item cited by controller, ffi and gtk cases is in `summary.json`'s `parity_clusters`;
  - when a row is dropped, the exit code is 1 and the message names the missing id;
  - every stdout section ends with a next command (FR-015).

  See it fail.
- [X] T008 [US1] Implement `scripts/test-inventory.py`:
  - argument parsing per the contract;
  - run `cargo nextest list --message-format json [-p …]` (or read `--list-json`);
  - find the newest `target/nextest/*/junit.xml` unless `--junit` is given;
  - write `cases.csv` and `summary.json` to `--out` (default `target/test-audit/`);
  - print the human summary.

  Until T007 passes.
- [X] T009 [US1] Run `scripts/test-inventory.py -p postio-config` in the worktree and check it against quickstart §1's count. Fix the mapping until `ambiguous` is 0 for that crate.
- [ ] T010 [US1] Run the inventory on the whole workspace, using JUnit from the newest `full-suite.yml` artifact (`gh run download`) into `target/test-audit/junit.xml`. Record in `specs/012-test-suite-value/research.md` under a new "Measured" heading:
  - cases per layer;
  - CPU-seconds per layer;
  - the 20 largest parity clusters;
  - smell counts;
  - `ambiguous` and `untraced` counts.

  Then order the US4 parity batches (T030 onward) by that list.

**Checkpoint**: SC-001 holds. The inventory is useful on its own.

---

## Phase 4: User Story 2, coverage sets (P1)

**Goal**: list the cases whose executed regions are a subset of a case that is cheaper or equally cheap. Contract: `contracts/test-coverage-sets.md`; research R2 and R3.

**Independent test**: quickstart §2, where a planted duplicate in postio-config is reported as `subset`.

- [X] T011 [P] [US2] Write `scripts/tests/test-test-coverage-sets.py` against hand-written per-case region sets (JSON fixtures, no LLVM). It asserts:
  - mutual subsets keep the lower `cost_rank`, then the one with a spec ref, then the first by name;
  - a rule case is never reported as subsumed by a frontend case (FR-006);
  - an `ignored` or `measurement` case is never listed;
  - a case with no regions in its own crate is `no-own-crate-coverage`;
  - two runs on the same input give byte-identical output.

  See it fail.
- [X] T012 [US2] Implement the candidate computation in `scripts/test-coverage-sets.py`, reading region sets and `cases.csv`, until T011 passes.
- [X] T013 [US2] Write `scripts/per-test-profile.sh`. It execs `"$@"` with `LLVM_PROFILE_FILE="$NEXTEST_WORKSPACE_ROOT/target/test-audit/profiles/$NEXTEST_BINARY_ID/$(printf %s "$NEXTEST_TEST_NAME" | shasum | cut -c1-16)/%m.profraw"`, and fails loudly if either `NEXTEST_*` variable is unset.
- [X] T014 [US2] Add `experimental = ["wrapper-scripts"]`, `[scripts.wrapper.per-test-profile]` and `[profile.per-test-coverage]` with a `[[profile.per-test-coverage.scripts]]` `run-wrapper` to `.config/nextest.toml`. The comment must say why this lives in a profile the merge path never selects (research R2). Check two things, red first:
  - `cargo nextest run --profile per-test-coverage -p postio-config` writes one profile directory per case;
  - on Linux CI, one widgets case run under this profile still sees a display.

  Choose the wrapper's `target-runner` value (`within-wrapper` or `around-wrapper`) by the second check, and record the result in research R2.
- [X] T015 [US2] Implement the run half of `scripts/test-coverage-sets.py`:
  - `cargo llvm-cov show-env --export-prefix`, then `cargo nextest run --profile per-test-coverage`;
  - per case, `llvm-profdata merge` and `llvm-cov export --format=text --skip-functions`;
  - region extraction limited to workspace crates;
  - `--reuse`;
  - exit 1 naming any case with no profile;
  - exit 2 with the install line when a tool is missing, worded as `scripts/coverage.sh` words it.
- [X] T016 [US2] Prove quickstart §2 on a scratch branch: the planted postio-config duplicate is reported `subset` of its original. Delete the scratch branch, and record the run time for postio-config in research.md "Measured".

**Checkpoint**: candidates exist for any crate. None is acted on yet.

---

## Phase 5: User Story 3, the removal check (P1)

**Goal**: a `safe` or `NOT SAFE` verdict based on mutation. Contract: `contracts/test-removal-check.md`; research R4.

**Independent test**: quickstart §3, run on CI.

- [ ] T017 [P] [US3] Write `scripts/tests/test-test-removal-check.py` with a stub `cargo` on `PATH` that replays recorded `mutants.out` directories (caught.txt and missed.txt for Pass A and Pass B). It asserts:
  - a miss in A that is caught in B is a new survivor → exit 1, and the mutant is named;
  - a miss in A that is also in `docs/mutants-baseline.txt` is not counted;
  - no new survivors → exit 0 and the `safe:` line;
  - with no `CI` and no `POSTIO_ALLOW_MUTANTS_HERE`, it refuses with exit 2 and quotes `scripts/mutants.sh`'s reason.

  See it fail.
- [ ] T018 [US3] Implement `scripts/test-removal-check.sh`:
  - Pass A is `cargo mutants --test-tool=nextest -f … --no-shuffle --jobs 1`, with `--cargo-test-arg=-E --cargo-test-arg='not (test(=…) | …)'`;
  - Pass B re-runs only A's misses that are not in the baseline, using a `--re` per mutant;
  - it writes `target/test-audit/verdict.json` per data-model **Verdict**.

  Reuse `scripts/mutants.sh`'s baseline `comm` diff rather than re-implementing it. Implement until T017 passes.
- [ ] T019 [US3] Add `.github/workflows/test-audit.yml`, dispatch-only, with inputs `mode` (`inventory` / `coverage` / `removal`), `crates`, `files` and `cases`:
  - same runner shape, pinned action SHAs and `ci-anything-new.sh` pattern as `mutants.yml`;
  - results uploaded as artifacts.

  Add it to `scripts/checks/ci-expected-workflows.py`'s list.
- [ ] T020 [US3] Prove quickstart §3 on CI:
  - a case that alone asserts `ConfigChanged::any` gives `NOT SAFE`, naming a `crates/postio-config/src/change.rs` mutant;
  - the T016 planted duplicate gives `safe`.

  Record both run times in research.md "Measured".

**Checkpoint**: The first PR can land here: T001–T020 plus ADR 0049, which already landed with the plan. It changes no test, so use `scripts/issue-land.sh --detach`, without `--full-suite`.

---

## Phase 6: User Story 5, part one: display skips (P2)

The spec puts US5 after US4. This part goes first because batch 1 touches the same 125 sites.

- [ ] T021 [US5] Add `display_or_fail() -> bool` to `crates/postio-widgets/tests/widgets_suite/` (in the suite's support module, or a new `support.rs` declared in `main.rs`, per `check-suite-modules.py`). It returns `false` with today's message only when neither `CI` nor `POSTIO_HEADLESS=1` is set; otherwise it panics with the contract's message.
  - Red first: a case in `widgets_suite` that runs the helper with `CI=1` and the display env cleared in a child process (`postio-test-support`'s child-process helper) must see the panic.
- [ ] T022 [US5] Replace the 125 inline `adw::init().is_err() || gdk::Display::default().is_none()` checks in `crates/postio-widgets/tests/widgets_suite/*.rs` with `if !display_or_fail() { return; }`. This is mechanical; one commit. Then run `cargo nextest run -p postio-widgets --test widgets_suite` under `scripts/test-headless.sh`.

---

## Phase 7: User Story 4, cleanup batches (P2)

**Goal**: shrink by what is proven redundant. Each task is a commit whose body carries the FR-008 proof. Batches follow the spec's order. Inside batch 3, take the T010 cluster list in descending CPU-seconds.

**Independent test**: each commit's removal-check line says `safe`, `scripts/coverage.sh <crate>` passes for every crate touched, and the full suite passes on the PR.

### Batch 1: mechanical (no removal check needed for moves; needed for deletions)

- [ ] T023 [P] [US4] Move the copies of `fn shown(` (23 files), `fn asks(` (14) and `fn drawn(` (8) into one helper module per suite. Use the suite's existing support module, or `postio-test-support` if the copies span crates. Where copies differ, keep each difference as a parameter and do not paper over it. List the files using the T010 inventory.
- [ ] T024 [P] [US4] Rewrite `crates/postio-gtk/tests/focus_suite/one_keymap.rs:636–731` so the expected key strings come from the registry's default binding table, not hand-copied literals. Confirm `check-one-default-binding-table.py` still passes. Red first: break one registry default in a scratch edit and see the rewritten case fail; then revert.
- [ ] T025 [US4] Move the display-free assertions in `crates/postio-gtk/tests/focus_suite/one_keymap.rs` (the case near line 83, "no default key means two things…") out of the GTK binary. Where `crates/postio-core/tests/core_suite/one_keymap.rs:535,561` already asserts the same thing, delete the GTK copy behind a T018 `safe` verdict. Otherwise move it into `core_suite`.
- [ ] T026 [US4] Delete `crates/postio-model/tests/model_suite/purity.rs` and its `mod` line once `check-crate-boundaries.py` is confirmed to refuse the same violation. Red first: plant a forbidden dependency in `postio-model/Cargo.toml` in a scratch edit and see the check fail; then revert.

### Batch 2: smells

- [ ] T027 [US4] For every `is-ok-only` case in the T010 inventory, tighten it to assert the value it produces. Where that duplicates an existing case, delete it behind a `safe` verdict. One commit per crate.
- [ ] T028 [US4] For every `constant-only` case (e.g. `crates/postio-ui/src/search_view.rs:2478`, `crates/postio-ui/src/focus_dialog.rs:540,562`), either assert the behaviour the constant drives or delete it behind a `safe` verdict. One commit per crate.
- [ ] T029 [US4] For every `mock-count-only` case (e.g. `crates/postio-core/tests/core_suite/dispatch.rs:110`, `crates/postio-host/tests/host_suite/oauth.rs:185`), add the outcome assertion a person would see (Constitution IV). Remove the count only if it then adds nothing and its removal is `safe`.

### Batch 3: parity clusters, one surface per commit (ADR 0049)

For each surface: the controller keeps its branches. Each frontend keeps one composition case per path, plus the parity sweeps. The rest go behind a `safe` verdict on that frontend's glue files.

- [ ] T030 [US4] Pickers:
  - `crates/postio-focus/tests/controller_suite/pickers.rs` keeps its branches;
  - reduce `crates/postio-ffi/tests/ffi_suite/focus_pickers.rs`, `crates/postio-gtk/tests/focus_suite/pickers.rs` and `crates/postio-widgets/tests/widgets_suite/pickers.rs` to composition.
- [ ] T031 [US4] Registry parity:
  - keep one sweep per platform among `crates/postio-tui/tests/tui_suite/registry_parity.rs`, `crates/postio-gtk/tests/focus_suite/registry_parity.rs` and `crates/postio-ffi/tests/ffi_suite/command_coverage.rs`;
  - move what they share into one helper that each calls with its platform's dispatcher;
  - each platform still asserts its own end, as the memory of the 31 Mac bindings requires.
- [ ] T032 [US4] Then the remaining controller/ffi twins in the order T010 recorded: `contact_popover`, `contacts_settings`, `compose`, `keymap`, `states`, `surfaces`, `bar`, `keys`, and whatever T010 ranks above them. One commit per surface, each with its proof. Mark the cases that stay as composition: `// ADR 0049 composition: <path>`.

### Batch 4: table merges

- [ ] T033 [P] [US4] Run a local clone detector over `crates/*/tests` and `#[cfg(test)]` modules (`similarity-rs`, or `jscpd --format rust`; local only, FR-014). Write its candidate list to `target/test-audit/clones.txt`. Merge cases that differ only in a constant into one case table, keeping each row's name in its failure message. One commit per file, with the removal check where cases disappear rather than merge.

---

## Phase 8: User Story 5, part two: the ratchets (P2)

These land with the first parity batch (T030), so there is a cleaned surface to protect. Contract: `contracts/ratchets.md`.

- [ ] T034 [P] [US5] Write `scripts/tests/test-check-frontend-suite-budget.py`, modelled on `scripts/tests/test-check-spacing-literals-ratchet.py`. It asserts:
  - a listed file over budget fails with the ADR 0049 message;
  - under budget passes and prints the line to lower;
  - an unlisted file is ignored;
  - `// parity sweep` and `ADR 0049 composition` cases are not counted;
  - harness-free `CASES` rows are counted.

  See it fail.
- [ ] T035 [US5] Implement `scripts/checks/check-frontend-suite-budget.py` until T034 passes. Seed `scripts/checks/frontend-suite-budget.txt` with each file T030–T032 cleaned, at its count after cleaning.
- [ ] T036 [P] [US5] Write `scripts/tests/test-check-assertion-smells.py`. It asserts that each of `constant-only`, `is-ok-only` and `inline-display-skip` fails when a file's count rises or a new file appears, names the case, and names the remedy, and that a falling count passes. See it fail.
- [ ] T037 [US5] Implement `scripts/checks/check-assertion-smells.py` on `scripts/lib/test_audit.py`'s `smells()` until T036 passes. Seed `scripts/checks/assertion-smells-baseline.txt` from the tree after batch 2. Confirm `scripts/check.sh` stays under its budget: the new checks add under 1 s.

---

## Phase 9: User Story 6, keep the picture current (P3)

- [ ] T038 [US6] Add `--diff <old summary.json>` to `scripts/test-inventory.py`. It prints the change in cases, CPU-seconds, parity clusters, smells and candidates. Red first, in `scripts/tests/test-test-inventory.py`: two summaries that differ by one removed case give `−1 case, −<its cpu_s> s`.
- [ ] T039 [US6] Add a step to `.claude/skills/steward/SKILL.md`:
  - run the inventory against the latest `full-suite.yml` JUnit;
  - diff it against the `summary.json` attached to the previous pass's comment on the steward's tracking issue;
  - post the delta.

  Keep the step under ten lines, in the skill's voice.

---

## Phase 10: Polish

- [ ] T040 [P] Add a `docs/gotchas.md` entry on why nextest wrapper scripts live only in `profile.per-test-coverage`, and what to check if an upstream nextest release changes them (research R2).
- [ ] T041 Mark the spec's status as built in `specs/012-test-suite-value/spec.md`, with the measured totals from the last batch's `summary.json`. Update ADR 0049's status line to say which surfaces have budgets.
- [ ] T042 Land with `scripts/issue-land.sh --detach --full-suite` (CLAUDE.md, spec-driven work).

---

## Dependencies

```text
Setup (T001–T002) → Foundational (T003–T006)
  ├─ US1 inventory (T007–T010) ────────────────┐
  ├─ US2 coverage sets (T011–T016; T012 needs T008's cases.csv)
  └─ US3 removal check (T017–T020; independent of US1/US2 except T020 uses T016's duplicate)
US5a display skips (T021–T022) — independent; before T023
US4 batches (T023–T033) need T010 (ordering), T018 (verdicts); T033 needs nothing but T018
US5b ratchets (T034–T037) need T030 landed (something to protect) and batch 2 (baseline)
US6 (T038–T039) needs US1
Polish last
```

- **US1, US2 and US3 are independent stories.** Each has its own self-test and quickstart step.
- **US4 needs the verdicts from US3 and the ordering from US1.** US2 only *proposes*, so US4 can start without it, using parity clusters alone.

## Parallel opportunities

- T002 can run beside T001.
- After Phase 2, the self-tests T007, T011 and T017 can be written in parallel.
- T013 and T014 (wrapper and profile) can run beside T012 (pure computation).
- In batch 1, T023 and T024 touch disjoint files.
- T034 and T036 (the two ratchet self-tests) can run in parallel.
- Batch-3 surfaces are independent of each other. Keep them as separate commits so each verdict stays readable.

## Implementation strategy

1. **MVP: Phases 1–3.** The inventory answers "what is there and what does it cost" for the whole suite, with parity clusters. It is useful before anything is removed.
2. **First PR: T001–T020.** This is the three tools plus the dispatch workflow. It changes no tests, and the ADR is already on the branch.
3. **Then the batches**, each provable on its own. The ratchets come in with the first parity batch.
4. **No numeric target** (maintainer, 2026-10-23). The work is done when the T010 parity clusters are each in one layer (SC-005) and the ratchets protect them.
