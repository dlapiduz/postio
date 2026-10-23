# Contract: the ratchets

Every check runs from `scripts/check.sh`, parses files only, finishes in under 1 s, and has a `scripts/tests/test-<check>.py` self-test with a planted violation for each failure it reports.

## `scripts/checks/check-frontend-suite-budget.py` (FR-011)

- **Input**: `scripts/checks/frontend-suite-budget.txt`, with lines `<test file> <count>`.
- **Counted**: the behaviour cases in each listed file:
  - `#[test]` functions, or the `pub fn` rows in a harness-free `CASES` table;
  - minus the cases marked `// parity sweep` or whose doc comment cites `ADR 0049 composition`.
- **Fails when** a listed file's count rises. The message:
  ```text
  crates/postio-ffi/tests/ffi_suite/focus_pickers.rs: 4 behaviour cases, budget 3.
  ADR 0049: a branch belongs in the controller suite
  (crates/postio-focus/tests/controller_suite/). If this case is composition --
  a path a person takes that no existing case drives -- raise the budget in
  scripts/checks/frontend-suite-budget.txt in this commit and say why in the body.
  ```
- **When a count falls**, it prints the baseline line to lower and passes.
- **Unlisted files** are not judged.

## `scripts/checks/check-assertion-smells.py` (FR-012, R8)

- **Input**: `scripts/checks/assertion-smells-baseline.txt`, with lines `<file> <smell> <count>`.
- **Smells judged**:
  - `constant-only`;
  - `is-ok-only`;
- **Fails when** a file's count for a smell rises or a new file appears. The message names the case and the remedy:
  - assert the value, not that a value exists;
  - assert behaviour the constant drives, not the constant.
