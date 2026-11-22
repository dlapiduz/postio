---
name: lanes
description: Report who else is working this repository now: claimed issues, worktrees, open branches and PRs, stale claims. Use at session start and before assuming an issue is free.
---

# Lanes

Sessions share a machine, a target directory and an issue tracker. This answers "who else is here, and what can I safely take".

## 1. What is claimed

```bash
gh issue list --label in-progress --json number,title,assignees \
  --jq '.[] | "#\(.number) \(.title)"'
git worktree list
```

An `in-progress` issue with a worktree under `~/src/postio-worktrees/issue-<n>` is live work. Leave it alone.

A quiet claim is not a stale one: a session can spend hours reading, waiting on CI or running a suite with no commit. Treat a claim as abandoned only after **a day or more** with no worktree, no branch and no commits:

```bash
scripts/issue-release.sh --stale        # a day or more; the safe default
scripts/issue-release.sh --stale 3      # a longer threshold
```

It also clears left-over locks whose issue is no longer claimed; those make `issue-claim.sh` refuse the issue forever. Releasing live work is far worse than leaving a label up a day too long; when in doubt, leave it and say so.

## 2. In flight, not merged

```bash
gh pr list --json number,title,headRefName,isDraft \
  --jq '.[] | "#\(.number) \(.headRefName) \(.title)"'
```

A branch with an open PR is finished work awaiting review. Do not restart it.

## 3. Is the machine busy

Every session shares one target directory and cargo serialises on it, so builds queue.

```bash
uptime                      # load average against 8 cores
pgrep -a "rustc|cargo" | head
scripts/test-headless.sh --status
```

Four concurrent release builds put this box into swap. Above a load of ~8, wait rather than start a build. Never run `scripts/run-isolated.sh` while others build; it links `--release` in its own target directory.

## 4. What is free

```bash
scripts/issue-claim.sh --dry-run
```

It applies the real rules (open, `ready`, unassigned, nothing open blocking) and names what it would take, highest priority first. If it says nothing, **there is nothing**: say so and stop. An issue without `ready` is not triaged as agent-work; `epic`, `icebox` and `needs-architecture` are not yours to start.
