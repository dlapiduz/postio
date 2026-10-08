# Issue tracker: GitHub

Issues live in this repo's GitHub Issues (`dlapiduz/postio`). Use `gh`, and
this repo's scripts where one exists for the operation.

## Conventions

- **Create an issue**: `scripts/issue-file.sh --title "..." --body-file <f> [--label ready,p2]`.
  It searches first and exits 2 when it finds possible duplicates: read them,
  comment on the existing issue when it is the same bug, and pass `--anyway`
  only when yours is genuinely different.
- **Read an issue**: `gh issue view <n> --comments --json title,body,labels,comments`.
- **List issues**: `gh issue list --state open --json number,title,body,labels --label <l>`.
- **Comment**: `gh issue comment <n> --body "..."`.
- **Labels**: `gh issue edit <n> --add-label "..."` / `--remove-label "..."`.
- **Claim to work it**: `scripts/issue-claim.sh <n>` (worktree, branch,
  assignee and `in-progress` in one step); `scripts/issue-claim.sh` alone takes
  the most important ready issue.
- **Close**: a PR body's `Closes: #<n>` closes on merge; `gh issue close <n> --comment "..."`
  for anything closed without one.

**PRs as a request surface: no.** _(Set to `yes` if external PRs should run
through triage; `/triage` reads this flag.)_

## When a skill says "publish to the issue tracker"

File through `scripts/issue-file.sh`.

## When a skill says "fetch the relevant ticket"

`gh issue view <n> --comments`.

## Wayfinding operations

Used by `/wayfinder`.

- **Map**: an issue labelled `epic`, holding Notes / Decisions so far / Fog.
- **Child ticket**: a GitHub sub-issue of the map, labelled for what it is
  (`ready` and a priority once it can be started unattended).
- **Blocking**: GitHub's native issue dependencies:
  `gh api --method POST repos/dlapiduz/postio/issues/<child>/dependencies/blocked_by -F issue_id=<blocker-db-id>`,
  where the db id is `gh api repos/dlapiduz/postio/issues/<n> --jq .id`.
  `scripts/issue-claim.sh` skips an issue blocked by anything still open.
- **Frontier**: the map's open, unassigned, unblocked children; first in map order wins.
- **Claim**: `scripts/issue-claim.sh <n>`.
- **Resolve**: comment the answer, close, and add a one-line pointer to the
  map's Decisions so far.
