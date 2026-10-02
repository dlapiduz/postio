# Contract: The Review

This covers the `/ux-review` skill, the `ux-reviewer` agent, the bundle
between them, and what comes back. The reasoning is in research R9 and R10.

## Who does what

```text
implementing session                      ux-reviewer (fresh agent, Opus)
────────────────────                      ──────────────────────────────
/ux-review
  storyboards.sh run --changed --variants
  storyboards.sh base
  postio-storyboard bundle  ──► bundle/
  postio-storyboard prompt  ──► prompt.txt ──────►  reads the SKILL.md files,
                                                     the design screens,
                                                     every outlined frame,
                                                     run.json and comparison.json
                                         ◄──────  writes bundle/verdicts.json
  postio-storyboard verdicts check
  for each fail: fix + re-run, or contest
  storyboards.sh page
```

**The reviewer is launched fresh, never forked.** Its whole input is
`prompt.txt`, passed verbatim. The implementing session adds nothing to it
(FR-018).

## The bundle

```text
Design/review/<branch>/bundle/
├── manifest.json        # what to review: storyboards, apps, variants, and changed steps
├── acceptance.md        # the issue's acceptance, or the spec's scenarios: `gh issue view` / spec.md
├── design/              # the canvas screens named by the storyboards, as PNG
├── runs/ -> ../runs     # branch runs
├── base/ -> ../base     # base runs
├── comparison.json
├── parity.json
├── prompt.txt           # written by `postio-storyboard prompt`
├── verdicts.json        # written by the reviewer
└── contests.toml        # written by the implementing session
```

With more than one batch, each reviewer writes `verdicts.N.json`, and
`postio-storyboard verdicts merge <bundle>` folds them into `verdicts.json`
in batch order before `verdicts check`.

`manifest.json` lists, per batch (one app and one surface, at most 60
frames), the storyboard files, each changed or new run, and **every step that
needs a verdict**. These are the steps of changed and new runs. Unchanged runs
are listed by count only.

**Design screens.** The bundle copies only the screens a storyboard names.

- **Classic** names the canvas renders already in `Design/screens/`, from
  `01-inbox-reading.png` to `24-app-icon.png`.
- **Focus** names the scrubbed references committed on
  `feature/postio-focus`. These are the ones its `shot` compares against at
  1440×900.
- **Never `Design/postio-focus-design/`.** That folder is untracked and its
  PNGs carry a real first name. A bundle may be summarised on a public PR, so
  `bundle` refuses any design path outside the committed reference
  directories.

## The prompt template

The template is fixed, versioned, and hashed into every review. Its content
is:

1. **Role.** You are Postio's design and UX reviewer. You did not build this
   and have not been told what it is meant to do, beyond the acceptance and
   the storyboards. Find what a person would notice.
2. **Read first.**
   - `.claude/skills/ux-architect/SKILL.md`
   - `.claude/skills/gtk-design/SKILL.md`
   - `docs/PRODUCT.md` (skim)
   - each `design/*.png` named in the manifest
3. **For every step listed in `manifest.json`:**
   1. Read its outlined frame, its `run.json` step and its prose `expect`.
   2. If the step changed, read the base frame beside it.
   3. Return `pass`, `fail` or `question`, citing the frame.
   4. Fail a step whose frame contradicts its `expect`, even if every check
      passed.
4. **Beyond the expectations.** Report findings for anything a person would
   notice:
   - a dead end;
   - a missing state (of the six);
   - a clipped or misaligned element;
   - a difference from the named design screen;
   - an inconsistency with another storyboard or app in the batch;
   - a jump or blank the runner flagged.
5. **How to word a finding.** Say what a person would notice, in their terms,
   and name the rule it rests on. Do not propose code.
6. **Where to write.** Write `verdicts.json` in the schema below, and nothing
   else.

## `verdicts.json`

```json
{
  "bundle":   { "tree_key": "…", "base": "…" },
  "reviewer": { "agent": "ux-reviewer", "model": "…", "template": "<blake3>" },
  "verdicts": [
    {
      "storyboard": "archive-walks-down",
      "step": "archive",
      "app": "classic",
      "variant": "default",
      "frame": "runs/classic/archive-walks-down/default/02.outlined.png",
      "verdict": "fail",
      "severity": "wrong",
      "says": "After archiving, the list scrolls back to the top and the cursor lands on the first row, so the next `a` archives a message the person never looked at.",
      "rule": "ux-architect §2 — a verb leaves the person where they were"
    }
  ],
  "findings": [
    {
      "storyboard": "archive-walks-down",
      "step": "archive",
      "app": "classic",
      "variant": "scheme=dark",
      "frame": "runs/classic/archive-walks-down/scheme=dark/02.outlined.png",
      "severity": "polish",
      "says": "The undo toast's button has less contrast than the canvas's.",
      "rule": "canvas 07-dark"
    }
  ]
}
```

**`verdicts check` rejects the review** (FR-019) when:
- a verdict or finding is missing any citation field;
- a frame path does not exist in the bundle;
- a step listed in the manifest has no verdict;
- `severity` is missing on a `fail` or on a finding;
- `says` is empty.

A rejected review is re-asked once, with the validator's message appended to
`prompt.txt`. If it is still rejected, the page says **review incomplete**
(spec, Edge Cases).

## Resolutions (FR-020)

The implementing session, never the reviewer, resolves each `fail` and each
finding of severity `blocker` or `wrong`:

| Resolution | How |
|---|---|
| **fixed** | A commit, then `storyboards.sh run --only <storyboard>` passes, then a re-review of only those storyboards passes. The verdict's `resolution` becomes `fixed { rerun }`. |
| **contested** | An entry in `contests.toml` (`ref` and `reason`) saying why the reviewer is wrong. The maintainer decides. |

A `polish` finding may be left `open`. It is listed, and does not hold the
branch.

A `question` is always shown to the maintainer under **Needs you**.

## Calibration (research R10)

```text
/ux-review --calibrate
```

This reviews `storyboards/calibration/` and writes `calibration.json`: for
each storyboard, its expected verdict, the actual verdict, and the hit rate.

- **Passing** means every `must_fail` failed and every `must_pass` passed.
- **When it runs**: on any change to `.claude/agents/ux-reviewer.md` or to the
  prompt template, and during a `/steward` pass as a measurement. CI has no
  model to run a reviewer.
- **The page** shows the last calibration's date and result in its header.

## What reaches the maintainer (FR-021, FR-022)

- **The page**: `Design/review/<branch>/index.html`, sectioned as in
  data-model § Review page.
- **The PR**: `summary.md`, text only. The first line is
  `storyboards-key: <tree key>`. Then:
  - one line for review complete or incomplete, and for calibration;
  - **Needs you**: contests and questions, each with its storyboard, step and
    app;
  - changed and new storyboards, with their verdict counts;
  - counts of unchanged, not-covered and not-applicable runs.
- **Maintainer rejections (FR-024).** When the maintainer rejects an
  interaction on the PR or the page, the implementing session writes a
  storyboard, or an `expect` in an existing one, with
  `source = { kind = "maintainer", ref = "<PR #… comment>" }`. It does this
  before re-running `/ux-review`.
