# Implementation Plan: Test suite value

**Branch**: `feature/test-suite-value` | **Date**: 2026-10-23 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/012-test-suite-value/spec.md`

## Summary

This feature adds three local tools, an ADR, three ratchets and a series of cleanup batches.

**The three tools.** Each one answers one question about the suite.

| Tool | Question | Built on |
|---|---|---|
| `scripts/test-inventory.py` | What is there, where is it, what does it cost, and what does it cite? | `cargo nextest list --message-format json`, nextest JUnit, test-file headers |
| `scripts/test-coverage-sets.py` | Which cases run nothing that another, equally cheap case does not? | A nextest profile whose wrapper gives each case its own LLVM profile (`NEXTEST_BINARY_ID`, `NEXTEST_TEST_NAME`), then `llvm-profdata` and `llvm-cov export` per case |
| `scripts/test-removal-check.sh` | If these cases go, does any mutant that was caught now survive? | `cargo mutants --test-tool=nextest` with the cases filtered out, then re-run with them in, only for the mutants that were missed |

**The ADR.** It records the layer-ownership rule.

**The ratchets.** All three are checks under `scripts/checks/`:
- a frontend suite's behaviour-case count never grows;
- no new constant-only or `is_ok()`-only assertion;
- a widgets case with no display fails on CI.

**The cleanup.** The batches then run behind the removal check, as the spec orders them.

## Technical Context

**Language/Version**: Python 3, matching every other `scripts/checks/*.py`, with self-tests in `scripts/tests/`. Bash is used only for the thin mutants wrapper, alongside `scripts/mutants.sh`. Rust (toolchain pinned by `rust-toolchain.toml`) is touched only for test helpers and the cleanups themselves.

**Primary Dependencies**: Each of these is already used by the repository:
- cargo-nextest 0.9.143 locally (`list --message-format json`, JUnit in `profile.ci`, wrapper scripts);
- cargo-llvm-cov 0.9.1 (`show-env` for the instrumentation flags), with `llvm-profdata` and `llvm-cov` from `llvm-tools-preview`;
- cargo-mutants (CI only, through `mutants.yml`).

There are no new third-party services.

**Storage**: Files only. Reports go in `target/test-audit/`, which is never committed. Baselines go in `scripts/checks/*-baseline.txt` and are committed.

**Testing**:
- Each script has a `scripts/tests/test-<script>.py` self-test, run by `scripts/run-self-tests.sh`, with the same red-first rule as the existing checks.
- Cleanup batches are judged by the removal check, `scripts/coverage.sh` and the full suite.

**Target Platform**:
- The inventory and the checks run on Linux and macOS.
- Per-case coverage and the removal check run on Linux CI. Mutation runs never go on the shared workstation, as `scripts/mutants.sh` records.

**Project Type**: Repository tooling and test-suite maintenance. No product code changes.

**Performance Goals**:
- The inventory runs in under 60 s after a build; it is `nextest list` plus file reads.
- Each check adds under 1 s to `scripts/check.sh`. The checks parse files and never build.
- Per-case coverage for one rule crate fits in one CI job.

**Constraints**:
- Never on the shared workstation for mutation, per `scripts/mutants.sh`'s own header.
- Never `-j`; the jobserver.
- No worktree path in anything rustc sees. Profiles go under `target/test-audit/`, named by test id, not by path.
- Nothing leaves the machine except through the project's own CI artifacts, exactly as `mutants.yml` already works (FR-014).

**Scale/Scope**:
- About 8,000 cases in 37 crates, 619 integration test files and 24 suites.
- Per-case coverage first targets the 9 crates in `coverage-floors.json`, then `postio-focus` and `postio-ffi`.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | How this plan stands | Verdict |
|---|---|---|
| I. Local-first | No product change | n/a |
| II. Keyboard is a system | The cleanup must not weaken registry or binding parity. FR-009 keeps the parity sweeps once per platform. Hand-copied key lists become registry-derived, which *strengthens* "the registry is the one truth" | Pass |
| III. Search | No change | n/a |
| IV. Test-first | Every new script and check is red on a planted case before it is green (the `scripts/tests/` convention). Removal needs mutation evidence, so the suite's ability to fail never drops. The layer rule keeps "assert on what a person would see" for frontends, because composition cases still drive real keys and read what is shown | Pass |
| V. Performance | Count-based budget tests (`counting`) are rule-layer cases and are never proposed for removal in favour of a slower layer (FR-006) | Pass |
| VI. Privacy | Local tools; CI artifacts only; reports contain test ids and counts, never mail | Pass |
| VII. Boundaries enforced | Each new rule ships as a `scripts/checks/*.py` check with a self-test, not as advice. The ADR states the rule; the checks enforce it | Pass |

There are no violations, so there is no Complexity Tracking table.

**Re-check after Phase 1:** still passing. The design adds one experimental nextest feature (wrapper scripts), and only in a profile no merge-path command selects; see research R2.

## Project Structure

### Documentation (this feature)

```text
specs/012-test-suite-value/
├── spec.md
├── plan.md              # this file
├── research.md          # R1–R8: the decisions and what was rejected
├── data-model.md        # case, cluster, candidate, verdict, baseline
├── contracts/
│   ├── test-inventory.md       # CLI and output columns
│   ├── test-coverage-sets.md   # CLI and report format
│   ├── test-removal-check.md   # CLI and verdict format
│   └── ratchets.md             # the three checks: inputs, messages, baselines
├── quickstart.md        # how to prove each piece works
├── checklists/requirements.md
└── tasks.md             # /speckit-tasks
```

### Source Code (repository root)

```text
scripts/
├── test-inventory.py                    # US1 (new)
├── test-coverage-sets.py                # US2 (new)
├── test-removal-check.sh                # US3 (new; reuses mutants.sh's baseline diff)
├── per-test-profile.sh                  # nextest wrapper: one LLVM profile per case (new)
├── lib/test_audit.py                    # shared: nextest list parsing, test id → file, ref parsing (new)
├── checks/
│   ├── check-frontend-suite-budget.py   # US5 (new)
│   ├── frontend-suite-budget.txt        # its baseline (new)
│   ├── check-assertion-smells.py        # US5 (new)
│   └── assertion-smells-baseline.txt    # its baseline (new)
└── tests/
    ├── test-test-inventory.py
    ├── test-test-coverage-sets.py
    ├── test-check-frontend-suite-budget.py
    └── test-check-assertion-smells.py
.config/nextest.toml                     # + profile.per-test-coverage and its wrapper script
.github/workflows/test-audit.yml         # dispatch-only: inventory, coverage sets, removal check (new)
docs/decisions/0049-each-layer-asserts-its-own-question.md   # FR-010 (new)
CLAUDE.md                                # Tests: one bullet pointing at the ADR
.claude/skills/steward/SKILL.md          # US6: run the inventory, post the delta
crates/postio-widgets/tests/widgets_suite/support.rs   # display_or_fail() (US5)
crates/*/tests/**                        # the cleanup batches (US4)
```

**Structure Decision**:
- The tools sit beside the existing coverage and mutants scripts. Their shared parsing goes in `scripts/lib/`, which already exists for shared script code.
- The checks follow the `spacing-literals-baseline.txt` ratchet pattern.
- There is no new crate: the work is tooling plus test edits.

## Phases of delivery

These map to the spec's user stories. `tasks.md` splits them further.

1. **US1, the inventory.** It first runs on `postio-config` and is checked against `nextest list`. Then it runs on the whole workspace, and its parity-cluster summary orders the batches.
2. **US2, coverage sets.** This adds the nextest profile and wrapper. It is proven on `postio-config` with a planted duplicate, then run on the coverage-floor crates on CI.
3. **US3, the removal check.** It is proven on a unique case (red, naming the mutant) and on a planted duplicate (green).
4. **ADR 0049 and the CLAUDE.md pointer.** These are written with the plan, so every batch cites a rule.
5. **US4, the batches, in the spec's order.** Each batch is one commit with its proof in the body, and the PR body carries the totals:
   1. mechanical (helpers, the registry-derived key lists, display-free cases out of the GTK binary, `purity.rs`);
   2. smells;
   3. parity clusters, one surface per commit, largest first by the inventory's CPU-seconds;
   4. table merges.
6. **US5, the ratchets.** The display-skip fix lands with batch 1. The two ratchets land with the first parity batch, with a baseline seeded from the tree as it then stands.
7. **US6, the steward step.**

The feature lands with `scripts/issue-land.sh --detach --full-suite`. US1–US3 and the ADR can land as a first PR before the batches, since they change no tests.
