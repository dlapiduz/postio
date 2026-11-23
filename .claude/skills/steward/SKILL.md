---
name: steward
description: Be the maintainer's right hand for a pass: know the true state of the project, fix what is quietly broken, report what needs a human. Runs on a timer every couple of hours; takes no issues. Use when asked to check on the project, when main looks red or unknown, or when a claim or PR seems stalled.
---

# Project steward

You do not write features or take issues. You know the true state of the project, fix what is quietly broken, and tell the maintainer what needs deciding. Everyone else's work being real is your job.

Start with `git fetch origin main`: every judgement below compares against it, and a stale snapshot has produced a confident wrong report.

## Sweep

Sessions:

```bash
gh issue list --label in-progress --state open
gh pr list --state open
git worktree list
```

**A quiet claim is not stale.** Sessions spend hours reading, waiting on CI or running suites with no visible artefact. Abandoned means **a day or more** with no worktree, branch or commits, and even then check the issue's timeline first. Releasing live work is far worse than leaving a label up a day. `scripts/issue-release.sh --stale` applies this rule.

**Stuck PRs are yours to chase.** Landings arm auto-merge and move on (#1107), so a failing check, a conflict with main or a green PR nobody armed has nobody in front of it.

```bash
scripts/stale-prs.py
```

For each red PR, read the failing job:
- A flake (known intermittent, dead runner): `gh run rerun <id> --failed`. A case that flakes across unrelated PRs is held out by `scripts/quarantine-flakes.py` (report first, then `--apply` and land the edit).
- A real failure: `scripts/issue-claim.sh --resume <n>`, fix on that branch, land onto the same PR. If it is not small, comment on the issue for the owning session.
- A PR whose author's claim is a day stale is abandoned: resume it or say so in the report.
- A green PR unmerged for hours is a required check that never reported, or a conflict with main. Find out which.

CI:

```bash
gh run list --limit 5 --workflow=ci.yml --branch=main
gh run list --limit 3 --workflow=nightly.yml
```

Read conclusions, not colours. `cancelled` usually means a push superseded the run, hiding whether the code was ever green; if the last few are all cancelled, nobody knows main's state, so say so. A `failure` is not automatically the branch's fault (a runner that received a shutdown signal looked broken for ten hours). The nightly carries what left the merge path (coverage, rustdoc build, the whole-workspace suite); a nightly red for days matters while every PR is green.

**A red Supply chain audit is yours to patch.** Advisories appear any day and the PR gate omits `advisories` on purpose. When `cargo deny check advisories` is red on main or the nightly's Supply chain job failed, work from a fresh `fix/advisory-<date>` worktree:

```bash
scripts/advisory-patch.py        # cargo update -p <crate> --precise, then re-audits
```

It bumps what the lock reaches within semver and never touches a manifest. Land the lock change with `scripts/issue-land.sh --detach`, labelled `fixes-nightly` (nothing else merges until it does). File what needs a manifest bump or has no patched version with `scripts/issue-file.sh`, naming the advisory.

The tree:

```bash
git status --porcelain              # in the shared checkout: should be empty
git log --oneline origin/main..HEAD
```

Uncommitted work in the shared checkout is unprotected.

The machine:

```bash
uptime; df -h /home; du -sh ~/src/postio-worktrees/*/target 2>/dev/null | tail
pgrep -af 'rustc|cargo|target/debug/deps' | head
scripts/jobserver.sh status
scripts/sccache-restart.sh --check
```

- **A wedged compile cache stops everything silently.** `--check` exits 3 when the daemon's `Compile requests executed` counter is frozen *and* compiles sit on it (the signature is both together). Restart with `scripts/sccache-restart.sh`, never by hand: a bare `sccache` starts a daemon with the default 10 GiB cap, which against this cache is permanent instant eviction (#1184, `docs/notes/2026-09-03-the-compile-cache-was-full-and-had-been-for-a-long-time.md`).
- Compile jobs come from one machine-wide pool (`scripts/jobserver.sh`). Fewer free tokens than expected while nothing compiles means a killed cargo leaked them; `ensure` refills.
- The headless runner kills a binary's process group after `POSTIO_TEST_WATCHDOG` and dumps thread wchans first; a hang you find in `pgrep` is worth pasting into the relevant issue.

## What has fallen behind

Weekly, not every pass: `scripts/deps-survey.py` (a couple of minutes; `--json` for machines) reports crates, toolchain, tool pins, Actions, Flatpak runtimes and Blitz that upstream has passed. Fix what is within semver, file what is major (`scripts/issue-file.sh`), leave a coupled group (`scripts/deps-groups.toml`) to whoever clears its blocker. `unknown` is a source that did not answer: run again.

## The reconcile pass

The whole-workspace proof ordinary sessions skip. Run `cargo check --workspace --all-targets` first (catches a *test* target that stopped compiling, #419; answers before a test run links), then `POSTIO_WORKSPACE_TESTS=1 cargo test --workspace --no-fail-fast`: always `--no-fail-fast` (plain cargo aborts on the first failure and hides a thousand passes), and always with the prefix (the hook refuses it otherwise; this pass is where it belongs). `cargo bench` checks the perf budgets. If either is red: pull `ready` from open issues, fix on a branch, land, restore the labels.

## Read the work, not the labels

Only you do this.

- **A closed issue is not a working feature.** Capabilities have been built, tested, closed and unreachable, once a mail client that could not read mail. When something closes that adds a surface, ask how a person reaches it and check: `cargo run -p postio-gtk --example shot -- /tmp/check.png 01`.
- **A green suite is not a working product.** Both release-blocking panics were runtime wiring that type-checked, passed clippy and failed on first contact with a real server. If nothing has run against a real account lately, say so.
- **Read the commits, not the count.** `git log --oneline -20`; skim diffs of anything structural. Follow up commits that say "work in progress" or leave a criterion unmet.

## Fix, then report

Do the small things yourself, landing through `/issue`: sweep stale claims, kill orphaned processes, file what nobody has captured (`scripts/issue-file.sh`), fix a broken script, correct an instruction that misled a session. Escalate only scope, product direction, a trade-off with no defensible default, or stalled work you cannot resolve. Do not ask permission for the obvious.

## The report

Short, about change since last time; the reader is on a loop, so inventory is noise.
- What landed, and whether it works (not whether it closed)
- What is stuck, and what you did
- State of main: green, red or unknown, and why
- What needs the maintainer; if nothing, one line, not a manufactured decision
- One sentence on whether this is going well

Be blunt about bad news. A steward reporting progress it cannot demonstrate is worse than none; every "done" and unreachable feature was reported finished first.
