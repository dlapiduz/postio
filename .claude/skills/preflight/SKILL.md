---
name: preflight
description: Run every Postio quality gate and report the true state of the tree (build, tests, clippy, fmt, crate boundaries, personal data), plus the two failures that look like bugs but are not (stale scratchpad artifacts, cargo test fast-failing). Use before starting work, before committing, and when the workspace looks broken.
---

# Preflight

Report the real state. Run every check even if an early one fails; a partial picture sends sessions after the wrong problem.

## 1. Gates

The deliberate full sweep, the same as `/steward`'s. Not an inner loop: use `scripts/test-fast.sh` while working and let `scripts/issue-land.sh` gate the landing. A hook refuses a whole-workspace test run unless `POSTIO_WORKSPACE_TESTS=1`.

```bash
cargo build --workspace
POSTIO_WORKSPACE_TESTS=1 cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
scripts/check.sh
```

- **Always `--no-fail-fast`.** Plain `cargo test` aborts the remaining targets after the first failure, so one broken crate hides a thousand passing tests.
- **Never drop `--check` from fmt**: writing rewrites files other sessions are mid-edit in.
- `scripts/check.sh` runs every repository invariant. The personal-data check redacts values because CI logs are public; locally use `python3 scripts/checks/check-no-personal-data.py --reveal`.

## 2. Stale artifacts: check before believing a failure

Tests failing with `NotFound` on paths containing `/scratchpad/`, or totals that make no sense, mean `target/` holds objects built from a *copy* of this repo (`CARGO_MANIFEST_DIR` is baked in at compile time).

```bash
POSTIO_WORKSPACE_TESTS=1 cargo test --workspace --no-fail-fast 2>&1 | grep -o '/scratchpad/[^ ]*' | head
```

A hit is stale artifacts, not a regression. `cargo clean -p <affected crates>` and re-run. Never build a copy of this repo outside the workspace.

## 3. Tree and issue state

```bash
git status --porcelain
git log --oneline -10
git worktree list
gh issue list --label in-progress --json number,title --jq '.[] | "#\(.number) \(.title)"'
gh pr list --json number,headRefName --jq '.[] | "#\(.number) \(.headRefName)"'
```

Flag:
- **Uncommitted work** (`CLAUDE.md` forbids leaving it): name the issues the loose files belong to.
- **Stale claims**: `in-progress` with no worktree and no open PR means a dead session. `scripts/issue-release.sh --stale` sweeps those whose worktree is gone; report any others.

## 4. Report

State: gates passing or failing, test totals, uncommitted files by crate, stale claims, what is ready. For a failure, say whether it is a real regression, another session's in-flight work, or stale artifacts; each needs a different response.
