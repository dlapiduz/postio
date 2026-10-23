# Research: Test suite value

Each entry gives the decision, why, and what was rejected. Facts were checked on 2026-10-23 against the installed tools or their documentation.

## R1. Where the case list comes from

- **Decision**: Use `cargo nextest list --workspace --message-format json`.
- **Rationale**:
  - It is the runner's own idea of what a case is. SC-001 compares against exactly that.
  - It includes the `harness = false` GTK and widgets suites, because their harnesses answer `--list --format terse` and `--exact` (`widgets_suite/main.rs:1080–1103`).
  - It reports `ignored` and whether the profile's `default-filter` matches, so the inventory can mark live-service and measurement cases without guessing.
- **Rejected**:
  - **Grepping `#[test]`.** This misses about 430 harness-free cases and counts `#[test]` inside macros wrongly.
  - **`cargo test -- --list`.** This is slower; nextest is the runner everything else already uses.

## R2. How each case gets its own coverage profile

- **Decision**: Add a dedicated nextest profile, `per-test-coverage`, with a **wrapper script** (`scripts/per-test-profile.sh`).
  - The wrapper sets `LLVM_PROFILE_FILE=target/test-audit/profiles/<binary-id>/<test-name-hash>-%m.profraw` from `NEXTEST_BINARY_ID` and `NEXTEST_TEST_NAME`, which nextest sets for every test process. It then execs the test.
  - The instrumentation flags come from `cargo llvm-cov show-env --export-prefix`, so the build matches what `scripts/coverage.sh` measures.
  - Wrapper scripts are experimental in nextest (`experimental = ["wrapper-scripts"]`, since 0.9.98). The profile is selected only by `test-coverage-sets.py`, never by the merge path, so an upstream change can break only this tool, and loudly.
  - On Linux the wrapper must run *inside* the existing `headless-runner.sh` target runner, or GTK cases lose their compositor. Wrapper scripts take a `target-runner` option (`within-wrapper` / `around-wrapper`). Which value is right gets verified by T-US2's first red test, which runs a widgets case under the profile and asserts it saw a display.
- **Rationale**:
  - `%p` in `LLVM_PROFILE_FILE` would give a profile per process, but nothing maps a pid back to a test.
  - The environment variables nextest already sets do map to a test.
  - A wrapper can be scoped by filter and profile; a target runner is global.
- **Rejected**:
  - **Invoking each binary directly with `--exact`.** This loses nextest's environment (`NEXTEST_BIN_EXE_*`, the dynamic-library path, the per-test timeout) and re-implements the runner.
  - **Replacing the target runner.** It is global, and the headless runner is load-bearing (`.cargo/config.toml:88–133`).
  - **A coverage service.** Ruled out by Constitution VI and by `scripts/coverage.sh`'s own header.

## R3. What "covered set" means and how subsets are computed

- **Decision**:
  - A covered set is the set of `(file, region start line, region start column)` with a non-zero count, read from `llvm-cov export -format=text` (JSON) for each profile.
  - Files are restricted to the crate under test plus the shared crates it depends on in the workspace. External crates are excluded.
  - A is a candidate against B when `cover(A) ⊆ cover(B)` **and** `layer_cost(B) ≤ layer_cost(A)` (FR-006).
  - Mutual subsets keep the cheaper case; on a tie, the one that cites a spec item; on a further tie, the first by name. This rule is deterministic, which acceptance scenario 2.3 requires.
- **Rationale**:
  - Regions rather than lines: two cases can share a line and take different branches of a `match` arm on it.
  - Subset over a cost order is the textbook reduction step. The cost order stops the report from preferring a GTK case to the rule test it duplicates.
- **Rejected**:
  - **Similarity scores such as Jaccard.** Thresholds invite arguments. Subset is binary, and mutation (R4) does the real judging.

## R4. How a removal is proven safe

- **Decision**: `scripts/test-removal-check.sh <files…> -- <case ids…>` works in two passes.
  1. **Pass A**, on CI only:
     - `cargo mutants --test-tool=nextest -f <files…> --no-shuffle --jobs 1`;
     - with `--cargo-test-arg=-E --cargo-test-arg='not (test(=a) | test(=b) …)'`, which runs the suite *without* the cases;
     - `--cargo-test-arg` is the documented way to pass arguments through to nextest.
  2. **Pass B**: for each mutant Pass A missed that is not in `docs/mutants-baseline.txt`, re-run it *with* the cases. A mutant caught in B and missed in A is a **new survivor**, and that is the verdict.

  Pass B is limited to Pass A's misses, so the second pass costs only what A missed, not a whole second run.
- **Rationale**: Coverage cannot distinguish two cases that execute the same code and assert different things; mutation can. Reusing the baseline means the check judges only what the removal changed, which matches how `scripts/mutants.sh` already judges.
- **Rejected**:
  - **Removing the test in a scratch commit and running the plain mutants job twice.** This doubles the cost and needs a branch per candidate.
  - **Coverage-only reduction.** Known to lose fault detection; it is the reason this gate exists.
- **Constraint**: Removal checks run only on the dispatch workflow (`test-audit.yml`). It has the same runner shape as `mutants.yml`, one crate's files per job, `--jobs 1`.

## R5. Mapping a case to its file and its refs

- **Decision**:
  - **Integration suites.** A test id is `<crate>::<suite>` plus `<module path>::<fn>`. The module path maps to `tests/<suite>/<mod>.rs` or `<mod>/mod.rs`, which `check-suite-modules.py` guarantees.
  - **Unit tests.** `src/<path>.rs` comes from the module path, with `tests` stripped.
  - **Harness-free cases.** The `CASES` table name equals the module path, by `widgets_suite`'s own contract.
  - **Fallback.** Search the crate for `fn <last segment>`. Several hits are marked `ambiguous`, never guessed.
  - **Refs** are parsed from the file's `//!` header and the case's own `///` doc comment:
    - spec paths `specs/NNN-…`;
    - item ids `FR-`, `SC-`, `US`, `T0nn`;
    - issues `#NNN`.

    An item id is keyed by the spec the file header cites (`012/FR-003`). An item id with no spec in scope is kept as `?/FR-003` and flagged, because ids repeat across specs.
- **Rationale**: Every integration file already has a header (619 of 619). The data exists; only the join is missing.

## R6. The assertion smells, and how a script sees them

- **Decision**:
  - Read the case body by brace matching from `fn <name>`. A smell is flagged when:
    - **constant-only**: every `assert*!` compares two literals or `CONST`/`CONST` identifiers;
    - **`is_ok()`-only**: the only assertions are `.is_ok()` or `.is_some()` in `assert!`;
    - **mock-count-only**: every assertion reads a `calls`, `call_count` or `times` field or method;
    - **display skip**: the body returns early after `eprintln!("skipping`;
    - **untraced**: the case has no ref and neither does its file.
  - The smells are flags in the inventory. Only constant-only and `is_ok()`-only become a ratchet (FR-012), because they have no false-positive story worth defending.
- **Rationale**: A parser this simple fits `check.sh`'s sub-second budget, and the existing checks are built the same way (`check-no-clock-waits.py`).
- **Rejected**: A `syn`-based Rust tool. It is more precise, but it is a new binary to build in `check.sh`, for flags a person reads anyway.

## R7. The ratchet shape

- **Decision**:
  - Both ratchets follow `check-spacing-literals-ratchet.py`: a committed baseline file of `path count` lines.
  - The check fails when a count rises or a new path appears. It prints the line to change to lower the baseline when a count falls, and it never fails for falling.
  - The frontend budget counts behaviour cases per frontend suite file, only for files listed in the baseline, which means surfaces already cleaned. An unlisted file is not judged until its batch lands.
- **Rationale**: This is a known shape with a self-test template, and it never blocks unrelated work.
- **Rejected**: A hard cap per suite. It would block feature work on surfaces nobody has cleaned yet.

## R8. Display skips (withdrawn)

- **Decision**: No change. `crates/postio-widgets/tests/widgets_suite/display_required.rs` already answers this. It is one case that fails when CI has no display, and when cargo never routed the binary through `scripts/headless-runner.sh` (the #551 bug). Its header records that it is one case on purpose, rather than an edit to the 126 inline checks.
- **Rejected**: A `display_or_fail()` helper replacing the inline checks. It would duplicate that guard 126 times, and it would undo a decision already made.

## Open items that do not block planning

- Whether to include `postio-storage` in the first coverage-sets run: it is the largest suite (96 test files), so its per-case export time decides. It is measured in US2.

## Measured

### 2026-10-23: inventory, on a Mac (T010, partial)

- **Coverage of this run:**
  - Covers the 7,297 cases a Mac can build. `postio-gtk`, `postio-widgets` and `postio-bench` are left out (`scripts/unbuildable-crates.sh`).
  - No durations: CI keeps no JUnit artifact. `test-audit.yml` produces both on Linux (T019), and the rest of T010 waits for it.
- **Cases by layer:**

  | Layer | Cases |
  |---|---|
  | rule | 6,000 |
  | frontend | 881 |
  | controller | 370 |
  | meta | 32 |
  | measurement | 14 |

- **Smells:**

  | Smell | Cases |
  |---|---|
  | is-ok-only | 28 |
  | constant-only | 8 |
  | mock-count-only | 6 |
  | display-skip | 2 (the widgets suite's 125 are on Linux) |
  | untraced | 3,904, mostly unit tests in `src/` |

  9 files were `ambiguous`.
- **Spec items rarely join layers.** Only 16 items are cited in two behaviour layers, because each layer cites its own task ids. Clusters were therefore extended to surfaces (FR-003). The run then found 28 clusters. The largest, by cases above their cheapest layer, are:
  - `010/US2`: controller 73, rule 14;
  - `surface:compose`: controller 21, frontend 58;
  - `surface:search`: frontend 14, rule 1;
  - `surface:keys`: controller 7, frontend 13;
  - `surface:surfaces`: controller 26, frontend 10.

### 2026-10-23: coverage sets, postio-config (T015, T016)

- **What it costs:**
  - an instrumented cold build of postio-config and its dependencies into `target/llvm-cov-target`: 4 min 34 s;
  - the 257 cases: 6 s;
  - per-case export and the report: 18 s (`--reuse`).
- **Test code must be excluded.** The first report found 0 candidates: every case runs its own body, which no other case runs, so every set was unique. Regions under `tests/` and inside `#[cfg(test)]` modules are now excluded (`test_audit.is_test_code`).
- **Result:**
  - 111 subset candidates of 257, and 3 cases with no coverage of their own crate.
  - Most of the subsets run the same function with different inputs. `keys::mod_token_tests` is typical: eight cases, each a subset of one. Coverage cannot tell inputs apart. These are the table-merge batch's material (T033) or the removal check's to reject, not deletions.
- **Planted duplicate.** A copy of `keys::mod_token_tests::delete_is_the_key_a_mac_calls_delete` was reported as `subset` of its original (quickstart §2).

### 2026-10-23: the removal check on CI (T020)

- **Removing all six of `change.rs`'s own unit tests: NOT SAFE.** The check named 14 mutants in `ConfigChanged::any` and `ConfigChanged::between` that only those cases catch. Run 39975022697, about 13 minutes.
- **Removing a planted copy of `changing_focus_touches_only_focus`: safe.** Run 39975026200, about 10 minutes, on the scratch branch `chore/audit-planted-duplicate`.
- **Baseline line numbers drift.** `docs/mutants-baseline.txt` holds `change.rs:71:13` for a mutant now at line 74. A drifted entry no longer matches, so Pass A treats it as new and Pass B re-runs it. It counts against the removal only if the cases catch it. Drift therefore costs time in Pass B, never a false verdict.
