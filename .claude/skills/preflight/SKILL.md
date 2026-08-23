---
name: preflight
description: Run every Postio quality gate and report the true state of the tree — build, tests, clippy, fmt, crate boundaries, personal data — plus the two failure modes that look like bugs but are not (stale scratchpad artifacts, and cargo test fast-failing). Use before starting work, before committing, and whenever the workspace looks broken.
---

# Preflight

Report the real state of the repository. Run every check even if an early one
fails — a partial picture is what sends sessions chasing the wrong problem.

## 1. Gates

```bash
cargo build --workspace
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
python3 scripts/check-crate-boundaries.py
python3 scripts/check-no-personal-data.py
```

**Always `--no-fail-fast`.** Plain `cargo test` aborts remaining targets after
the first failure, so one broken crate hides a thousand passing tests and the
totals look catastrophic. This has already caused a false alarm.

`check-no-personal-data.py` redacts values by default because CI logs are
public. Add `--reveal` locally when fixing what it finds.

## 2. Stale artifacts — check this before believing a failure

If tests fail with `NotFound` on paths containing `/scratchpad/`, or a totals
drop makes no sense, the `target/` directory holds objects built from a *copy*
of this repo somewhere else. `CARGO_MANIFEST_DIR` is baked in at compile time,
so tests that read files from disk look for a directory that no longer exists.

```bash
cargo test --workspace --no-fail-fast 2>&1 | grep -o '/scratchpad/[^ ]*' | head
```

Any hit means stale artifacts, not a regression. Fix with
`cargo clean -p <affected crates>` and re-run. Never build a copy of this repo
outside the workspace.

## 3. Tree and bead state

```bash
git status --porcelain
git log --oneline -10
bd list --status=in_progress
bd ready
```

Two things to flag explicitly:

- **Uncommitted work.** `CLAUDE.md` forbids leaving it. If files are loose,
  identify which beads they belong to and say so.
- **Stale claims.** A bead `in_progress` whose work is already committed means
  a session died before `bd close`. Verify against `git log` and report which
  ones can be closed.

## 4. Report

State plainly: gates passing or failing, test totals, uncommitted files by
crate, stale claims, and what is ready. If something failed, say whether it is
a real regression, another session's in-flight work, or stale artifacts —
those need completely different responses.
