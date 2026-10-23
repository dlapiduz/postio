# Quickstart: proving each piece works

Run these from the feature worktree. Each step names what "working" looks like. None of them needs the network.

## 1. The inventory counts what the runner counts (US1, SC-001)

```bash
scripts/test-inventory.py -p postio-config
cargo nextest list -p postio-config --message-format json | python3 -c \
  'import json,sys; d=json.load(sys.stdin); print(sum(len(b["testcases"]) for b in d["rust-suites"].values()))'
wc -l < target/test-audit/cases.csv   # = the number above + 1 header
```

Then the whole workspace:

```bash
scripts/test-inventory.py
```

The summary's parity-cluster list should include `009/…` items with `controller`, `frontend` cases. That is the snooze picker cluster the spec names.

## 2. A planted duplicate is found (US2)

On a scratch branch:
1. Copy one `postio-config` case under a new name.
2. Run:
   ```bash
   scripts/test-coverage-sets.py -p postio-config
   grep <new name> target/test-audit/candidates.csv   # kind=subset, subsumed_by=<original>
   ```
3. Drop the scratch branch.

## 3. The removal check tells unique from duplicate (US3)

This runs on CI: dispatch `test-audit.yml` with `mode=removal`.

| Input | Expected |
|---|---|
| A case that alone asserts `ConfigChanged::any` | `NOT SAFE`, naming a `change.rs` mutant |
| The planted duplicate from step 2 | `safe` |

## 4. Each ratchet goes red on a planted violation (US5, SC-007)

```bash
scripts/run-self-tests.sh --dir scripts/tests   # includes the two new check self-tests
scripts/check.sh                                 # green on the real tree
```

## 5. A cleanup batch carries its proof (US4, FR-008)

The commit body has:
- the `safe:` line from step 3's tool, for that batch's cases;
- `scripts/coverage.sh <crate>` passing for every crate touched;
- cases and CPU-seconds before and after, taken from two `summary.json` files.
