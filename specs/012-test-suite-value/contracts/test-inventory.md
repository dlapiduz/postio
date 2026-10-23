# Contract: `scripts/test-inventory.py`

```text
scripts/test-inventory.py [-p <crate>]... [--junit <path>] [--out <dir>]
```

| Option | Default | Meaning |
|---|---|---|
| `-p <crate>` | whole workspace | Restrict the listing to these packages (repeatable) |
| `--junit <path>` | newest `target/nextest/*/junit.xml`, else none | Durations; with none, every `duration_s` is `unknown` |
| `--out <dir>` | `target/test-audit/` | Where `cases.csv` and `summary.json` go |

## Output

- `cases.csv`:
  - one row per case `cargo nextest list` reports, in its order;
  - the columns are the **Case** fields in `data-model.md`, in that order;
  - `refs` and `smells` are `;`-joined.
- `summary.json`: the **Inventory summary** in `data-model.md`.
- stdout:
  - a short human summary: cases per layer, CPU-seconds per layer, the 10 largest parity clusters, smell counts, `ambiguous` and `untraced` counts;
  - each line ends with the next thing to run, for example `scripts/test-coverage-sets.py -p <crate>` (FR-015).

## Exit status

| Code | Meaning |
|---|---|
| 0 | Written |
| 1 | `cases.csv` row count ≠ the runner's `test-count`. This is a bug in the parsing; the message gives both numbers |
| 2 | Bad arguments or no runner |

## Guarantees

- No network.
- Reads no file outside the repository and `target/`.
- Builds only what `nextest list` builds.
