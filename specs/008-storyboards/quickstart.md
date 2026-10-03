# Quickstart: Storyboards

This guide checks the feature end to end. Each scenario names the spec story
it proves. The commands are in [contracts/runner.md](./contracts/runner.md),
and the file format is in
[contracts/storyboard-format.md](./contracts/storyboard-format.md).

## Prerequisites

- A worktree with a warm build. Run `scripts/issue-claim.sh`, or seed the
  tree as CLAUDE.md describes.
- `mutter` installed. The runner uses the same private headless compositor as
  the tests (`scripts/test-headless.sh --status`).

Every command plays Focus, the one desktop app (ADR 0043), unless `--app`
names another: since specs/007-postio-focus T265 the runner is
`postio-focus`'s, and the catalogue is written against Focus's surfaces.
`--app classic` plays the classic app until T256 removes it.

## 1. The catalogue loads (sanity tier)

```bash
scripts/storyboards.sh lint
```

**Expect** every storyboard listed with the apps it applies to: nearly all
name `apps = ["focus"]`. The command exits 0.

**Negative check.** Add `source` to nothing, or put an address on a real
domain, and the lint fails, naming the file and the field.

## 2. One storyboard, filmed on Focus (US1)

```bash
scripts/storyboards.sh run --only list/cursor-and-selection-look-different
scripts/storyboards.sh page --open
```

**Expect:**
- The filmstrip shows frames `00` to `0n`.
- Each outlined frame names `list` as the region holding the keyboard.
- Each step shows its observation and its checks, all passing: `proof =
  "pinned"` for #753 (research R0).

`list/archive-walks-down` is the red counterpart: `proof = "open"` on
#1746, Focus's own form of #1687, so its cursor checks fail by design until
that lands.

**Determinism (SC-003).** Run it twice, then compare:

```bash
diff <(jq -S 'del(.commit, .steps[].settle.ms)' Design/review/<branch>/runs/focus/cursor-and-selection-look-different/default/run.json) \
     <(jq -S 'del(.commit, .steps[].settle.ms)' <second run>/run.json)
```

The diff is empty, and the frame hashes are equal. `settle.ms` is left out
because it is how long the step took to come to rest, which is wall time and
varies by a few milliseconds between runs; it is recorded for the budget, and
nothing compares it.

## 3. A storyboard is red on its base (US1, research R0 source 1)

On a branch that fixes an open interaction defect, write the storyboard
first, then run:

```bash
scripts/storyboards.sh base
scripts/storyboards.sh run  --only <the new storyboard>
scripts/storyboards.sh page --open
```

**Expect:**
- The **base** column is red, failing the step the defect lives in.
- The **branch** column is green.
- The page classifies the storyboard as `changed`.

## 4. Chain delivery sees a swallowed key (US1, research R3)

```bash
cargo nextest run -p postio-widgets --test widgets_suite storyboard_chain_delivery
```

**Expect:** a key reaches the window from inside a list; a dialog over the
window keeps the key from it; a key aimed at a widget that has left the
window is reported **dropped**, never delivered; and `Return` in a text
field activates it, mirroring GTK's own binding.

**Delivery mode.** Any storyboard re-run with `--delivery direct` says
`delivery: direct` in its run and page header. The two modes can disagree,
and that disagreement is evidence: #1748's `K` in the reading pane is
dropped along the focus chain and accepted-but-inert handed straight to the
window. A step marked `routing = "real"` reads `not covered` in both, never
`passed`.

## 5. A review before the maintainer (US2)

```text
/ux-review
```

**Expect:**
- `bundle/verdicts.json` exists.
- `postio-storyboard verdicts check` passes.
- Every verdict on the page links to its frame.
- **Needs you** lists only contests and questions.
- `summary.md`'s first line is `storyboards-key: …`, matching
  `scripts/storyboards.sh key`.

**Calibration.**

```text
/ux-review --calibrate
```

**Expect** every `must_fail` storyboard failed and every `must_pass` passed,
with the result shown in the page header.

## 6. The landing warning (FR-023)

On a branch that changes `crates/postio-focus` or `crates/postio-widgets`
with no current review, land:

```bash
scripts/issue-land.sh --detach
scripts/issue-land.sh --status
```

**Expect:**
- The PR body carries a `> [!WARNING]` block naming `/ux-review`.
- The PR carries the label `interactions-unreviewed`.
- The landing still proceeds.

Run `/ux-review`, push again, and the summary arrives as a PR comment.

## 7. One storyboard on every app present (US3)

```bash
scripts/storyboards.sh run --app all --only calibration/cal-cheatsheet-lists-keys --calibration
scripts/storyboards.sh page --open
```

**Expect:**
- The parity section shows one row per step, with a column per app present.
- A shared storyboard with no override on a step where the two observations
  differ is marked **diverging**.

Until T256 removes it, `all` includes the classic app; a storyboard naming
`apps = ["focus"]` reads `not applicable` there.

## 8. The screen sweep (FR-030)

```bash
scripts/storyboards.sh screens
```

**Expect** a contact sheet of every `storyboards/screens/*.toml`, filmed on
Focus in the variants each asks for, with a design on the left where a
screen names one. The classic canvas's designs are not Focus's, and Focus's
references are never committed (specs/007-postio-focus/screens.md), so the
screens cite screens.md's numbers rather than a file.
`scripts/screens.sh` no longer exists.

## 9. Every command does something (US6)

```bash
scripts/storyboards.sh coverage
```

**Expect** `coverage.json` with no `no_effect` entries outside
`storyboards/gaps/focus.toml`, and no `stale_gap` entries.

## 10. The suites

```bash
cargo test -p postio-storyboard --lib                 # lint and the pure logic, in seconds
cargo nextest run -p postio-widgets --test widgets_suite storyboard   # the GTK half
cargo nextest run -p postio-focus --test focus_suite storyboard observe
cargo nextest run -p postio-focus --test focus_suite --profile nightly storyboard_catalogue every_command
```

**Expect** all to pass. A failing storyboard is named in the catalogue
case's message, with its step and check.
