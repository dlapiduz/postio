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
- For the Focus scenarios, a worktree of a lane on `feature/postio-focus`.

## 1. The catalogue loads (sanity tier)

```bash
scripts/storyboards.sh lint
```

**Expect** every storyboard listed with the apps it applies to. On `main`,
Focus-only storyboards read `focus: not present on this branch`. The command
exits 0.

**Negative check.** Add `source` to nothing, or put an address on a real
domain, and the lint fails, naming the file and the field.

## 2. One storyboard, filmed on Classic (US1)

```bash
scripts/storyboards.sh run --app classic --only list/archive-walks-down
scripts/storyboards.sh page --open
```

**Expect:**
- The filmstrip shows frames `00` to `0n`.
- Each outlined frame names `list` as the region holding the keyboard.
- Each step shows its observation and its checks.
- The cursor check on the archive step passes, because `proof = "pinned"`
  for #1687 (research R0).

**Determinism (SC-003).** Run it twice, then compare:

```bash
diff <(jq -S 'del(.commit)' Design/review/<branch>/runs/classic/archive-walks-down/default/run.json) \
     <(jq -S 'del(.commit)' <second run>/run.json)
```

The diff is empty, and the frame hashes are equal.

## 3. A storyboard is red on its base (US1, research R0 source 1)

On a branch that fixes an open interaction defect, write the storyboard
first, then run:

```bash
scripts/storyboards.sh base --app classic
scripts/storyboards.sh run  --app classic --only <the new storyboard>
scripts/storyboards.sh page --open
```

**Expect:**
- The **base** column is red, failing the step the defect lives in.
- The **branch** column is green.
- The page classifies the storyboard as `changed`.

## 4. Chain delivery sees a swallowed key (US1, research R3)

```bash
scripts/storyboards.sh run --app classic --only list/keys-reach-the-list-under-nothing
```

This storyboard opens and closes the cheat sheet, then presses `j`. **Expect**
`keyboard.reachable = true` and the cursor moved.

**Delivery mode.** Rerun with `--delivery direct`, and the run says
`delivery: direct` in its header. A step marked `routing = "real"` reads
`not covered (delivery: chain)` in both runs, never `passed`.

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

On a branch that changes `crates/postio-gtk` with no current review, land:

```bash
scripts/issue-land.sh --detach
scripts/issue-land.sh --status
```

**Expect:**
- The PR body carries a `> [!WARNING]` block naming `/ux-review`.
- The PR carries the label `interactions-unreviewed`.
- The landing still proceeds.

Run `/ux-review`, push again, and the summary arrives as a PR comment.

## 7. One storyboard on both apps (US3), from a Focus lane

```bash
scripts/storyboards.sh run --app all --only list/archive-walks-down
scripts/storyboards.sh page --open
```

**Expect:**
- The parity section shows one row per step, with columns `classic` and
  `focus`.
- The `focus` column uses its override's `expect`.
- A shared storyboard with no override on a step where the two observations
  differ is marked **diverging**.

## 8. The screen sweep (FR-030)

```bash
scripts/storyboards.sh screens
```

**Expect** the same contact sheet `screens.sh` produced: design on the left,
the app on the right, for every `storyboards/screens/*.toml`.
`scripts/screens.sh` no longer exists.

## 9. Every command does something (US6)

```bash
scripts/storyboards.sh coverage --app classic
```

**Expect** `coverage.json` with no `no_effect` entries outside
`storyboards/gaps/classic.toml`, and no `stale_gap` entries.

## 10. The suites

```bash
cargo test -p postio-storyboard --lib                 # lint and the pure logic, in seconds
cargo nextest run -p postio-app --test app_suite storyboards
```

**Expect** both to pass. A failing storyboard is named in the case's message,
with its step and check.
