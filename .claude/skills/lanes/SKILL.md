---
name: lanes
description: Report who else is working this repository right now — which issues are claimed, which worktrees exist, which branches and PRs are open, and whether any claim is stale. Use at the start of a session, and before assuming an issue is free.
---

# Lanes

Several sessions work this repository at once. Each takes a GitHub issue and
gets a private worktree, so they no longer collide in one tree — but they do
still share a machine, a target directory, and an issue tracker. This answers
"who else is here, and what can I safely take".

## 1. What is claimed

```bash
gh issue list --label in-progress --json number,title,assignees \
  --jq '.[] | "#\(.number) \(.title)"'
git worktree list
```

An issue labelled `in-progress` with a worktree under
`~/src/postio-worktrees/issue-<n>` is someone's live work. Leave it alone.

**A claim is not stale because it is quiet.** A session can spend hours on one
issue — reading it, waiting on CI, running a suite — and leave no worktree
commit or branch for most of that time. An issue claimed this morning is
somebody's afternoon, not an abandoned lock.

Treat a claim as abandoned only after **a day or more** with no worktree, no
branch and no commits:

```bash
scripts/issue-release.sh --stale        # a day or more; the safe default
scripts/issue-release.sh --stale 3      # a longer threshold
```

It also clears left-over locks whose issue is no longer claimed — those are
harmless-looking and make `issue-claim.sh` refuse that issue forever.

Releasing live work is far worse than leaving a label up a day too long. When
in doubt, leave it and say so.

## 2. What is in flight but not merged

```bash
gh pr list --json number,title,headRefName,isDraft \
  --jq '.[] | "#\(.number) \(.headRefName) \(.title)"'
```

A branch with an open PR is finished work waiting on review, not abandoned
work. Do not restart it.

## 3. Is the machine busy

This is the part the worktrees did *not* fix. Every session shares one target
directory and cargo serialises on it, so builds queue rather than parallelise.

```bash
uptime                      # load average against 8 cores
pgrep -a "rustc|cargo" | head
scripts/test-headless.sh --status
```

Four concurrent release builds is what put this box into swap. If the load
average is already above ~8, wait rather than starting a build — and never run
`scripts/run-isolated.sh` while others are building, since it links `--release`
in a target directory of its own.

## 4. What is free

```bash
scripts/issue-claim.sh --dry-run
```

It applies the real rules — open, `ready`, unassigned, nothing still-open
blocking it — and names what it would take, highest priority first, without
taking it.

If that says there is nothing, **there is nothing**. Say so and stop. An issue
without the `ready` label has not been triaged as agent-work, and an `epic`,
`icebox` or `needs-architecture` issue is deliberately not yours to start.
