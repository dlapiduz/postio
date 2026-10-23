# Contract: the ratchets

Every check runs from `scripts/check.sh`, parses files only, finishes in under 1 s, and has a `scripts/tests/test-<check>.py` self-test with a planted violation for each failure it reports.

## `scripts/checks/check-frontend-suite-budget.py` (FR-011)

- **Input**: `scripts/checks/frontend-suite-budget.txt`, with lines `<test file> <count>` and `#` comments. It is seeded with the 53 files the parity review read, at their counts after the removals.
- **Counted**: every case in a listed file:
  - a `#[test]` or `#[tokio::test(...)]` function;
  - in a harness-free GTK or widgets suite, a `pub fn` named in that suite's `CASES` table.

  There are no exemption markers. A baseline seeded at today's counts already holds every case the review kept, and a new composition case raises its line in the commit that adds it.
- **Fails when** a listed file's count rises past its budget, or a listed file is gone. The message names ADR 0049, the controller suite, and the line to raise for a genuine composition case.
- **When a count falls**, it prints the line to lower and passes.
- **Unlisted files** are not judged.

## `scripts/checks/check-assertion-smells.py` (FR-012, withdrawn)

Withdrawn 2026-10-23: see spec FR-012 and research, Measured. What follows is the design as it was.

- **Input**: `scripts/checks/assertion-smells-baseline.txt`, with lines `<file> <smell> <count>`.
- **Smells judged**:
  - `constant-only`;
  - `is-ok-only`;
- **Fails when** a file's count for a smell rises or a new file appears. The message names the case and the remedy:
  - assert the value, not that a value exists;
  - assert behaviour the constant drives, not the constant.
