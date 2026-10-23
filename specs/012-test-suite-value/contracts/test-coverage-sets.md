# Contract: `scripts/test-coverage-sets.py`

```text
scripts/test-coverage-sets.py -p <crate> [-p <crate>]... [--profiles <dir>] [--out <dir>] [--reuse]
```

## Steps

1. Get the instrumentation environment from `cargo llvm-cov show-env --export-prefix`.
2. Run `cargo nextest run --profile per-test-coverage -p …`. The profile's wrapper is `scripts/per-test-profile.sh`, which writes one `.profraw` set per case under `target/test-audit/profiles/<binary-id>/<sha1(test-name)>/`.
3. For each case:
   - `llvm-profdata merge` its profiles;
   - `llvm-cov export --format=text --skip-functions` against the case's binary;
   - keep the regions with a non-zero count in the crate's own files and in the workspace crates it depends on.
4. Compute the candidates (research R3).

`--reuse` skips steps 1–2 when profiles already exist; it is for iterating on the report.

## Output

`<out>/candidates.csv` has the columns `case,kind,subsumed_by,regions`, sorted by `kind`, then by the case's `cpu_s` descending. stdout gives counts by kind and the next step:

```text
scripts/test-removal-check.sh <files> -- <cases>
```

## Exit status

| Code | Meaning |
|---|---|
| 0 | Report written, whether or not there are candidates |
| 1 | A case produced no profile. The message names it: usually a wrapper or target-runner problem, see research R2 |
| 2 | Bad arguments, or a missing `cargo-llvm-cov` or `llvm-tools-preview`. The message gives the install line, as `scripts/coverage.sh` does |

## Guarantees

- Deterministic for a given tree (acceptance 2.3).
- Never proposes `subsumed_by` with a higher `cost_rank` (FR-006).
- Never lists an `ignored` or `measurement` case.
