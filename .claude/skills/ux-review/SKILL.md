---
name: ux-review
description: Film the interactions a branch changed and have an independent design/UX reviewer judge them before the maintainer sees them — runs the storyboards, compares against the base, launches a fresh ux-reviewer that cites a frame for every verdict, sends failures back to you to fix or contest, and builds the page and PR summary. Use before landing any change to a GTK app's interaction, when the maintainer rejects an interaction, and with --calibrate after changing the reviewer.
---

# /ux-review

The maintainer's request, verbatim: *"a system where a claude agent can review
the interactions and the screens before shipping them to me for review."* This
is that system's loop (`specs/008-storyboards`, contracts/review.md). You are
the implementing session; the reviewer is someone else.

## 0. Before anything

- **Commit first.** The review key is computed from `HEAD`'s trees
  (`scripts/storyboards.sh key`), so uncommitted work is reviewed under the
  wrong key and the landing warning will fire anyway.
- **Every interaction you changed has a storyboard.** If the change has none,
  write it now, from the issue's acceptance -- not from what you built. A
  storyboard written after the code describes the code. `storyboards/README.md`
  is the format; `storyboards/list/archive-walks-down.toml` is a good model.
- **A storyboard for a defect you are fixing is seen red on the base** (step 2);
  that is its proof (research R0). Mark it `proof = "base"`.

## 1. Film the branch

```bash
scripts/storyboards.sh lint
scripts/storyboards.sh run
```

A run that fails here is yours to fix before any reviewer sees it -- the
machine checks are cheaper than a review. `not covered` and `not applicable`
are not failures; they are counted and shown.

## 2. Film the base

```bash
scripts/storyboards.sh base
```

(Until `base` exists -- tasks T066/T067 -- skip this; every run is reviewed as
new.) The page shows base and branch side by side, and a storyboard written
for the defect you fixed is red there.

## 3. Bundle and launch the reviewer

```bash
gh issue view <n> --json title,body --jq '"# " + .title + "\n\n" + .body' \
    > Design/review/<branch>/acceptance.md          # or the spec's scenarios
scripts/storyboards.sh bundle --acceptance Design/review/<branch>/acceptance.md
scripts/storyboards.sh tool prompt Design/review/<branch>/bundle --list   # batch count
scripts/storyboards.sh tool prompt Design/review/<branch>/bundle --batch 1 > /tmp/…/prompt-1.md
```

Launch **one `ux-reviewer` agent per batch, fresh -- never a fork** -- at most
four at once, each given **exactly** its prompt file's contents and nothing
else. Do not add context, explain the change, or say what you meant: the
reviewer judges what a person would see, and your account of the change is
the one thing it must not have (FR-018). If the `ux-reviewer` agent type is
not available in this session, use a general-purpose agent on Opus whose
prompt is `.claude/agents/ux-reviewer.md`'s body followed by the prompt file,
verbatim.

With more than one batch each reviewer writes `verdicts.N.json`; then:

```bash
scripts/storyboards.sh tool verdicts merge Design/review/<branch>/bundle
```

## 4. Validate

```bash
scripts/storyboards.sh tool verdicts check Design/review/<branch>/bundle
```

A rejection (a verdict without a citation, a frame that does not exist, a
step with no verdict) is re-asked **once**: launch the reviewer again with the
same prompt plus the validator's output appended. Still rejected: the page
will say *review incomplete*, and you say so too.

## 5. Resolve every fail

For each `fail`, and each finding of severity `blocker` or `wrong`:

- **Fix it**: commit, re-run only that storyboard
  (`scripts/storyboards.sh run --only <path>`), re-review only it. To
  re-review only what moved after a round of fixes, copy the reviewed runs
  aside first, re-film, and bundle against the copy
  (`scripts/storyboards.sh tool bundle --base <copy> ...`): the reviewer is
  asked only about new runs and changed steps. Carry the earlier verdicts for
  everything else into the new bundle as `verdicts.0.json` -- a frame that
  did not change keeps the verdict it had -- and `verdicts merge` joins them. Do not
  argue with a frame by changing the storyboard's expectation unless the
  expectation was wrong about the *product* -- and then say so in the commit.
- **Or contest it**: an entry in `Design/review/<branch>/bundle/contests.toml`
  (`[[contest]]`, `ref = "<storyboard>/<step>/<app>/<variant>"`, `reason`).
  The maintainer settles contests; you do not get to drop one.

`polish` findings may stay open; they are listed. `question`s always go to
the maintainer.

## 6. The page, and what reaches the maintainer

```bash
scripts/storyboards.sh page --open
```

`summary.md` beside it is text only, first line `storyboards-key: …`; it goes
on the PR (`issue-land.sh` puts it in the body, or comments it on a PR that
already exists). Frames never go on a PR. If the maintainer is away from the
workstation, offer to publish the page privately -- never by default.

## When the maintainer rejects an interaction

Their rejection becomes a storyboard, or a new `expect` in an existing one,
with `source = { kind = "maintainer", ref = "PR #… comment" }`, on the same
branch, **before** you re-run this skill (FR-024). A rejection never has to
be made twice.

## --calibrate

After any change to `.claude/agents/ux-reviewer.md` or
`crates/postio-storyboard/templates/reviewer-prompt.md`, and on a `/steward`
pass:

```bash
scripts/storyboards.sh run --calibration
scripts/storyboards.sh bundle --calibration --acceptance <a note saying "calibration">
scripts/storyboards.sh tool prompt Design/review/<branch>/calibration/bundle --batch 1
```

Launch the reviewer as in step 3. **Every `must_fail` storyboard must be
failed and every `must_pass` passed** (`storyboards/calibration/`). Record the
hit rate in `docs/notes/2026-10-01-what-a-storyboard-capture-costs.md`'s
calibration section with the template's blake3. Below 12 of 12, change the
*prompt template*, never the calibration set, and calibrate again.

## What the reviewer will not flag, because the runner does it on purpose

Boxes for CJK and emoji (embedded fonts only), the magenta keyboard outline
and caption, dates as of 2 June 2026 (frozen clock), no animation. The prompt
tells the reviewer; you should not "fix" any of them.
