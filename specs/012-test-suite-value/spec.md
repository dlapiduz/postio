# Feature Specification: Test suite value

**Feature Branch**: `feature/test-suite-value`

**Created**: 2026-10-23

**Status**: Draft. The maintainer decided on 2026-10-23 that the layer rule is an ADR (0049) and that there is no CPU target

**Input**: The maintainer said: "I have a lot of tests in this repo, and it is hard to tell whether they are duplicates or whether they are valuable." They asked for a framework and a process that simplifies or reduces the suite while keeping its quality.

## Context

The suite has grown faster than anyone can read it:
- about 7,500 `#[test]` and `#[tokio::test]` functions;
- about 430 more `harness = false` cases in the GTK and widgets suites;
- about 214k lines under `crates/*/tests`;
- 8,040 cases and 127 CPU-minutes in the 2026-10-22 full suite.

Much of the measuring equipment already exists, but nothing points it at the question "is this test worth keeping?":
- per-crate coverage floors (`scripts/coverage.sh`);
- weekly mutation testing against a baseline (`scripts/mutants.sh`, `docs/mutants-baseline.txt`);
- per-case durations (`scripts/ci-test-budget.py`);
- flake history (`scripts/quarantine-flakes.py`);
- a doc header on every test file that names its spec item or issue.

A sample of the suite (2026-10-23) found these patterns. They are why this spec exists. They are not a complete audit.

| Pattern | Example | Spread |
|---|---|---|
| One behaviour re-asserted in every frontend | "a number chooses in the snooze picker" is asserted in the `postio-focus` controller suite, `ffi_suite/focus_pickers.rs`, `focus_suite/pickers.rs` and `widgets_suite/pickers.rs` | Most of the 21 controller-suite files have an FFI twin; many also have a GTK twin |
| Display-free logic inside a display binary | `focus_suite/one_keymap.rs` repeats `core_suite/one_keymap.rs`'s "no key means two things" | Unknown; the inventory (US1) counts it |
| A hand copy of a table the code owns | `one_keymap.rs` lists default key strings by hand, which is the third copy `check-one-default-binding-table.py` warns about | A handful |
| A Rust test that repeats a repository check | `model_suite/purity.rs` repeats `check-crate-boundaries.py` | One found |
| Copied helpers | `fn shown(` in 23 files, `fn asks(` in 14, `fn drawn(` in 8 | Tens of files |
| Weak assertions | constant-only (~17), `is_ok()`-only (~26), mock-call counts instead of outcomes (~30) | Small share |
| A silent pass when there is no display | 125 `skipping: no display` early returns in the widgets suite; CI is guarded by `display_required::ci_has_a_display_to_run_the_gtk_suites_on`, a local run is not | Widgets suite |

### What this spec keeps, deliberately

- **Frontend tests are not the problem as such.** Constitution IV says the defects that reach users live *between* layers. The memory of 31 Mac bindings dying on the Mac side of a shared layer is why every platform keeps its own composition test. The aim is to give each layer **its own question**, not to delete a layer's tests.
- **A test that is cheap, unique and traceable stays**, however small it is.
- **Nothing is removed on suspicion.** Removal needs evidence that no regression would now go uncaught (FR-007).

## The rubric

A test earns its place by being **the cheapest test that would catch some real regression no other test catches**. Every test is read along four axes:

| Axis | The question | The evidence |
|---|---|---|
| Unique protection | Is there a defect this test catches and no other test does? | Per-test coverage proposes; mutation confirms |
| Traceability | Which spec item, issue or bug does it guard? | Its file header and doc comment |
| Right layer | Is it at the cheapest layer that can see the behaviour? | The layer it runs in, and whether it needs a display |
| Cost | What does it cost each run? | Duration, need for a display, flake history |

### Layer ownership

This rule outlives the cleanup, so it gets an ADR (FR-010).

| Layer | Owns | Does not own |
|---|---|---|
| Rule crates (`postio-core`, `postio-ui`, `postio-body`, `postio-model`, …) | Every case of a rule: each state, key and edge | — |
| Controller (`postio-focus`) | What a surface does with a command: every branch | Re-proving a rule a rule crate owns |
| Frontend (`postio-ffi`, `postio-gtk`, `postio-widgets`, `postio-tui`) | **Composition:** a real key reaches the shared logic and a person sees the result. One case per surface and path, plus the registry and binding parity sweeps, once per platform | Branches the controller already covers |
| Meta (reads source, docs or manifests) | Facts only compiled types can check, such as generated docs being current | Anything a `scripts/checks/*.py` guard already enforces |

## User Scenarios & Testing *(mandatory)*

The "users" here are the maintainer and the agent sessions that work the suite.

### User Story 1: See the whole suite in one table (Priority: P1)

The maintainer or a session runs one command and gets a table with a row for every test case in the workspace, including the `harness = false` cases. Each row has:
- crate, suite, file and layer;
- last-recorded duration;
- whether it needs a display;
- the spec items and issues it cites;
- any assertion smell it carries.

A summary groups the cases by spec item and by surface, and flags every one asserted in two or more behaviour layers as a **parity cluster**.

**Why this priority**: Every later decision reads this table. It is also useful on its own: it answers "how many tests, where, and what do they cost" today.

**Independent Test**: Run the command on a clean tree and compare its row count with the test runner's own list of cases.

**Acceptance Scenarios**:

1. **Given** a tree that builds, **When** the inventory runs, **Then** it has one row per case the test runner lists, harness-free cases included, and no other rows.
2. **Given** a test file whose header cites `FR-012` and `#1234`, **When** the inventory runs, **Then** every case in that file carries both refs.
3. **Given** a recorded run report, **When** the inventory runs, **Then** each case carries its duration from that report; a case missing from the report shows "unknown", not zero.
4. **Given** a spec item cited by cases in the controller, FFI and GTK suites, **When** the summary is read, **Then** that item is listed as a parity cluster with its cases per layer.
5. **Given** a case whose only assertion is `is_ok()`, **When** the inventory runs, **Then** the row is flagged with that smell.

---

### User Story 2: Propose the redundant cases (Priority: P1)

For a chosen set of crates, the session runs one command that records what each case executes. It then lists:
- (a) every case whose executed code is a subset of another case's, at the same layer or a cheaper one;
- (b) every case that executes none of the code of the crate it tests.

Each line is a **candidate**, never a verdict.

**Why this priority**: Without this list, finding duplicates means reading 214k lines.

**Independent Test**: Plant a copy of an existing case in a small crate. Run the command and confirm the copy and its original are reported as mutual subsets.

**Acceptance Scenarios**:

1. **Given** two cases that execute the same code, **When** the report runs, **Then** each is listed as a subset of the other.
2. **Given** a frontend case and a controller case where the frontend case executes everything the controller case does and more, **When** the report runs, **Then** the controller case is *not* proposed for removal in favour of the more expensive one.
3. **Given** a crate with no instrumentable code changes, **When** the report runs twice, **Then** both reports are identical.

---

### User Story 3: Prove a removal is safe before making it (Priority: P1)

Given a set of cases proposed for removal, the session runs one command. It mutates the code those cases execute, runs the suite with and without them, and answers yes or no: **does any mutant survive without them that was killed with them?** A "no" is the only licence to remove.

**Why this priority**: Coverage cannot tell two tests apart when they run the same code but assert different things. Mutation can. This is the gate that keeps the cleanup from lowering quality.

**Independent Test**: Run it on a case known to be unique (the only one asserting some branch) and confirm it answers "yes, these mutants survive", naming them. Run it on a planted duplicate and confirm "no".

**Acceptance Scenarios**:

1. **Given** a case that is the only one to assert a branch, **When** its removal is checked, **Then** the answer names the mutant in that branch that now survives.
2. **Given** a planted exact duplicate, **When** its removal is checked, **Then** the answer is "no new survivor".
3. **Given** mutants already listed as surviving in the committed baseline, **When** a removal is checked, **Then** they are not counted against it.

---

### User Story 4: Clean up in batches, each one proven (Priority: P2)

The suite is cleaned through the normal loop, one batch per change. Each change shows its proof in its description:
- no new surviving mutant;
- coverage floors held;
- cases and CPU-seconds before and after.

The batches, in order of value per unit of effort:
1. **Mechanical.**
   - Copied helpers become one shared helper.
   - Hand-copied key lists assert against the registry.
   - Display-free assertions move out of display binaries.
   - `purity.rs` gives way to `check-crate-boundaries.py`.
2. **Smells.** Constant-only, `is_ok()`-only and mock-count-only cases are tightened to assert an outcome, or removed where US3 says nothing is lost.
3. **Parity clusters, one surface per change.**
   - The controller keeps its branches.
   - Each frontend keeps its composition cases.
   - The rest go, behind the US3 gate.
4. **Table merges.** Cases that differ only in a constant become one table of cases.

**Why this priority**: This is where the suite actually shrinks. It depends on US1–US3 for its evidence.

**Independent Test**: Each batch is a change of its own: its proof is in its description, and the full suite is green on it.

**Acceptance Scenarios**:

1. **Given** a parity cluster for one surface, **When** its batch lands, **Then** the frontend suites keep at least one case per path a person can take through that surface on that platform, and the US3 check reports no new survivor.
2. **Given** the mechanical batch, **When** it lands, **Then** no helper named in the inventory's copy list is defined in more than one test file.
3. **Given** any batch, **When** it lands, **Then** every coverage floor in `scripts/coverage-floors.json` still holds.

---

### User Story 5: Stop it growing back (Priority: P2)

Once a surface is cleaned, a repository check refuses a change that grows that frontend suite's count of behaviour cases past its committed baseline, unless the baseline is raised in the same change with a reason. 

**Why this priority**: Without a ratchet, the next feature re-creates the duplication. It only matters once there is something to protect.

**Independent Test**: Plant each violation in turn and see each check go red with a message that names the remedy.

**Acceptance Scenarios**:

1. **Given** a cleaned surface, **When** a change adds a behaviour case to its frontend suite without raising the baseline, **Then** the check fails and names the controller suite as the place for it.
2. **Given** a new test whose only assertion is `is_ok()`, **When** the check runs, **Then** it fails and names the case.

---

### User Story 6: Keep the picture current (Priority: P3)

`/steward`'s periodic pass runs the inventory and the redundancy report and posts the change since its last pass:
- case count;
- CPU-minutes;
- parity clusters;
- surviving mutants;
- new candidates.

**Why this priority**: It keeps the work visible after the initial push. It is not needed to start.

**Independent Test**: Run the steward step twice on two commits that differ by one deleted test, and see the delta report exactly that.

**Acceptance Scenarios**:

1. **Given** two passes, **When** one test was removed between them, **Then** the delta shows −1 case and that case's CPU-seconds.

### Edge Cases

- A case that needs a live service (`#[ignore]`) or belongs to the nightly measurement tier is inventoried and marked as such. It is never proposed for removal on coverage evidence, because it never runs on the measured path.
- A case that runs inside a display binary, where per-case coverage may be unavailable, is inventoried. It is then judged by the parity-cluster rule and US3 on its glue files, not by subset coverage.
- Doctests are listed separately, because the test runner does not run them. They are not proposed for removal by this feature.
- A case that cites no spec item and no issue is flagged "untraced". That is a reason to read it, not to remove it.
- When mutation testing would take longer than the remaining time (several hours for a large file), US3 checks a cluster's own files rather than the crate.
- When two cases are mutual subsets, the report proposes keeping the cheaper one. On a tie, it keeps the one that cites a spec item.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The inventory MUST produce one row per test case the runner lists, including `harness = false` cases, with crate, suite, file, layer, duration, display need, cited refs and smell flags.
- **FR-002**: The inventory MUST classify each case's layer as rule, controller, frontend, meta or measurement, using the crate and file, with no hand-kept list per test.
- **FR-003**: The inventory MUST group cases by cited spec item, and by surface (the test file's name with a frontend prefix such as `focus_` removed). Every item or surface asserted in two or more behaviour layers (rule, controller, frontend) is a parity cluster. Surfaces are needed because each layer tends to cite its own task ids: on 2026-10-23 only 16 items were cited in two layers, while the duplication the sample found follows file names (`pickers.rs`, `focus_pickers.rs`).
- **FR-004**: The inventory MUST flag these smells: constant-only assertion, `is_ok()`-only assertion, mock-call-count-only assertion, display skip, and untraced.
- **FR-005**: The redundancy report MUST record the code each case executes, per case, and list subset and zero-coverage candidates for the crates named.
- **FR-006**: The redundancy report MUST NOT propose removing a cheaper-layer case in favour of a more expensive one.
- **FR-007**: No test may be removed by this feature unless the removal check reports no mutant that survives without it and was killed with it, measured against the committed mutants baseline.
- **FR-008**: Every cleanup change MUST state in its description: new survivors (zero), coverage floors (held), and cases and CPU-seconds before and after.
- **FR-009**: After a parity cleanup, each frontend MUST keep at least one case per path a person can take through the surface on that platform, and the registry and binding parity sweeps MUST remain once per platform.
- **FR-010**: The layer-ownership rule MUST be recorded as an ADR in `docs/decisions/`, and CLAUDE.md's Tests section MUST point to it.
- **FR-011**: A repository check MUST refuse growth of a cleaned frontend suite's behaviour-case count past its committed baseline.
- **FR-012**: *Withdrawn 2026-10-23.* Triage of every flag on the Mac inventory found all 8 constant-only cases to be deliberate pins: a default the product decided, the contact-sync write gate shipping off, two header bit-sets that must not overlap, UI copy. It also found most of the 28 is-ok-only cases to be asserting exactly the property they name. A ratchet would refuse mostly good tests. The smells stay as inventory flags for a person to read, and the 4 genuinely weak cases were tightened (T027, T028).
- **FR-013**: *Withdrawn 2026-10-23.* A silent display skip is already guarded, by design, by one case: `display_required::ci_has_a_display_to_run_the_gtk_suites_on` (#114, #551). It fails when CI has no display and when cargo never routed the binary through the headless runner. Its header says it is deliberately one case "rather than an edit to every one". `display-skip` stays an inventory flag only.
- **FR-014**: Every tool this feature adds MUST run entirely on the machine. Nothing is uploaded and no third-party service is used (Constitution VI).
- **FR-015**: Every tool MUST name what to do next when it finds something, as the existing `scripts/checks` do.

### Key Entities

- **Test case**: one case the runner lists. It has a crate, suite, file, layer, duration, display need, refs and smells.
- **Parity cluster**: a spec item or surface asserted by cases in two or more behaviour layers, together with those cases.
- **Candidate**: a case the redundancy report proposes, with the case that subsumes it.
- **Removal verdict**: the result of the removal check for a set of cases. It is either "no new survivor" or the list of mutants that would survive.
- **Baseline**: a committed count or list that a ratchet compares against, which only moves down unless a change raises it with a reason.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: The whole-suite inventory, including its parity-cluster summary, comes from one command, with a row count equal to the runner's case count.
- **SC-002**: Every case removed under this feature has a recorded "no new survivor" verdict. Zero removals happen without one.
- **SC-003**: The mutants baseline never grows as a result of a removal, and every batch reports cases and CPU-seconds before and after. There is no numeric reduction target (maintainer, 2026-10-23): the suite gets smaller by exactly what is proven redundant, and no more.
- **SC-004**: No helper on the inventory's copy list is defined in more than one test file.
- **SC-005**: After cleanup, every parity cluster has its branch cases in exactly one layer.
- **SC-006**: Every coverage floor holds at every step.
- **SC-007**: Each ratchet goes red on a planted violation and names its remedy.

## Assumptions

- Per-case coverage relies on the runner giving each case its own process, which the workspace's runner already does.
- Mutation runs go on CI or a machine of their own, never the shared workstation (`scripts/mutants.sh` records why).
- GTK suites are judged by parity clusters and US3 on their glue files. Per-case coverage of display binaries is a stretch goal, not a requirement.
- Doctests and the nightly measurement tier are out of scope for removal.
- This feature changes tests and tooling only. No product behaviour changes.
- External clone detectors on test code (token- or AST-based) may produce a candidate list for the table-merge batch. They are optional and local-only (FR-014).
