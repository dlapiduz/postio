# Data model: Test suite value

All of these are rows in files under `target/test-audit/`, except **Baseline**, which is committed. The fields are what the tools read and write; the formats are in `contracts/`.

## Case

One test case as the runner lists it.

| Field | Type | Source | Rule |
|---|---|---|---|
| `id` | string | nextest: `<binary-id> <test-name>` | Unique; the join key for everything else |
| `crate` | string | nextest `package` | |
| `suite` | string | nextest binary name; `lib` for unit tests | |
| `file` | path or `ambiguous` | R5 mapping | Never guessed |
| `layer` | `rule` / `controller` / `frontend` / `meta` / `measurement` | crate table (`frontend`: postio-ffi, -gtk, -widgets, -tui; `controller`: postio-focus; everything else `rule`). `meta` if the body reads repo files; `measurement` if the file has `POSTIO-MEASUREMENT:` | FR-002: no per-test list |
| `cost_rank` | 0–3 | rule 0, controller 1, frontend without display 2, frontend with display 3 | Orders candidates (FR-006) |
| `display` | bool | crate is -gtk or -widgets suite with a custom harness, or the body calls the display helper | |
| `ignored` | bool | nextest | Never a removal candidate |
| `duration_s` | float or `unknown` | JUnit `time` | A missing value is `unknown`, never 0 |
| `refs` | list of `spec/item`, `#issue` | R5 | |
| `smells` | set of `constant-only`, `is-ok-only`, `mock-count-only`, `display-skip`, `untraced` | R6 | |

## Parity cluster

A spec item cited by cases in three or more suites spanning at least two layers (FR-003).

| Field | Type | Rule |
|---|---|---|
| `item` | `spec/item`, e.g. `009/FR-021` | `?/…` items are listed separately, not clustered |
| `cases_by_layer` | map layer → list of case ids | |
| `suites` | int | Distinct binaries; at least 3 |
| `cpu_s` | float | Sum of known durations |

## Candidate

Output of the coverage-sets report.

| Field | Type | Rule |
|---|---|---|
| `case` | case id | |
| `kind` | `subset` / `no-own-crate-coverage` | |
| `subsumed_by` | case id, or empty for `no-own-crate-coverage` | `cost_rank(subsumed_by) ≤ cost_rank(case)` |
| `regions` | int | Size of the case's covered set |

**Transitions**: candidate → checked (Verdict exists) → removed, or kept with its verdict recorded.

## Verdict

Output of the removal check.

| Field | Type | Rule |
|---|---|---|
| `cases` | list of case ids | What would be removed |
| `files` | list of paths | What was mutated |
| `new_survivors` | list of cargo-mutants mutant names | Caught with the cases, missed without, and not in the committed baseline |
| `result` | `safe` iff `new_survivors` is empty | FR-007 |
| `commit` | sha | The tree it was measured on |

## Baseline

A committed ratchet file.

| File | Line format | Moves |
|---|---|---|
| `scripts/checks/frontend-suite-budget.txt` | `<test file path> <behaviour case count>` | Down freely; up only in a commit that says why |
| `scripts/checks/assertion-smells-baseline.txt` | `<test file path> <smell> <count>` | Down only |
| `docs/mutants-baseline.txt` | existing | Unchanged by this feature, except where a batch kills a survivor |

## Inventory summary

Written next to the CSV as `summary.json`; the steward diffs two of these.

`cases`, `cases_by_layer`, `cpu_s_by_layer`, `parity_clusters` (count and top 20 by `cpu_s`), `smells_by_kind`, `untraced`, `ambiguous`, `commit`.
