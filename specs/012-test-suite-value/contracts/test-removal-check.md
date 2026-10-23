# Contract: `scripts/test-removal-check.sh`

```text
scripts/test-removal-check.sh <source file>... -- <case id>...
```

## Steps

These follow research R4.
1. **Pass A** mutates the files and runs the suite without the cases (a nextest `-E 'not (…)'` filter through `--cargo-test-arg`).
2. **Pass B** re-runs only Pass A's misses that are not already in `docs/mutants-baseline.txt`, this time with the cases included.

The script refuses to start on a machine without `CI` set unless `POSTIO_ALLOW_MUTANTS_HERE=1` is set. The message quotes `scripts/mutants.sh`'s reason.

## Output

`target/test-audit/verdict.json` holds the **Verdict** in `data-model.md`. stdout prints one of:

```text
safe: removing 3 case(s) leaves no new surviving mutant in 2 file(s) (commit abc1234)
```

```text
NOT SAFE: 1 mutant(s) survive without these cases and were caught with them:
  crates/postio-config/src/change.rs:71:13: replace || with && in ConfigChanged::any
Keep the case that catches it, or move its assertion to the cheapest layer that can see it.
```

## Exit status

| Code | Meaning |
|---|---|
| 0 | safe |
| 1 | not safe |
| 2 | bad arguments, a missing tool, or refused to run here |

A cleanup commit pastes the stdout line into its body (FR-008).
