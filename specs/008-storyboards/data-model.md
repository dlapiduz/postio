# Data Model: Storyboards

**Feature**: [spec.md](./spec.md) | **Research**: [research.md](./research.md)

Every entity here is plain data. Storyboards are TOML, and runs, reviews and
bundles are JSON. All of it lives in toolkit-free crates: `Observation` is in
`postio-ui`, and the rest is in `postio-storyboard`. The file grammar is
[contracts/storyboard-format.md](./contracts/storyboard-format.md).

---

## Storyboard

One interaction.

| Field | Type | Rule |
|---|---|---|
| `name` | string | Required. Unique across the catalogue. Kebab case. Defaults to the file stem and must equal it. |
| `source` | `Source` | Required (FR-001, US5). |
| `proof` | `base` \| `open` \| `pinned` | Required for `source.kind = issue`. Says where the red evidence comes from (research R0). |
| `seed` | `SeedName` | Defaults to `small`. |
| `preset` | `PresetName`? | A window condition the app names (R11). An app without it makes the storyboard not applicable to that app. |
| `apps` | `auto` \| `[App]` | Defaults to `auto` (FR-007). |
| `vary` | map `Axis → [Value]` | Optional (FR-017). |
| `design` | `DesignScreen`? | A canvas screen id, such as `01-inbox-reading` (FR-006). |
| `routing` | `chain` \| `real` | Defaults to `chain`. `real` marks the whole storyboard as not covered under `chain` (FR-008). |
| `calibration` | `must_pass` \| `must_fail`? | Only under `storyboards/calibration/` (R10). Excluded from catalogue runs. |
| `steps` | `[Step]` | Zero steps is a screen (FR-030). |
| `overrides` | map `App → map StepRef → StepOverride` | FR-005. Every `StepRef` must name a real step. |

**Validation, enforced by the lint.**
- Every command id exists.
- Every chord parses.
- Every address uses a reserved domain (FR-027).
- No field names a path outside the repository.
- Override keys name apps that the storyboard applies to.
- `same_as` refers to an earlier step.
- `calibration` appears only under `calibration/`.

### Source

| Field | Type |
|---|---|
| `kind` | `issue` \| `commit` \| `spec` \| `design` \| `flow` \| `maintainer` |
| `ref` | string: `#1687`, `92a093b8`, `specs/001…/FR-012`, `03-compose`, `ux-architect §4`, or `PR #… comment` |

### Step

| Field | Type | Rule |
|---|---|---|
| `id` | string? | For overrides and `same_as`. Defaults to the 1-based index. |
| input | exactly one of `command`, `key`, `type`, `wait`, `event` | FR-002 |
| `command` | `CommandId` (its string id) | Pressed via the app's binding in its *current* context (R3). |
| `key` | chord, such as `j`, `shift+tab` or `mod+z` | Raw chord; `mod` expands per platform. |
| `type` | string | Goes into the widget holding the keyboard (R3). |
| `wait` | `{ ms = n }` \| `{ until = Checks }` | |
| `event` | `EnvEvent` | An environment change (R11): `new_mail`, `mailboxes_changed`, `connection_lost`, `connection_restored`, `backfill_progress`, `body_arrived`. |
| `check` | `Checks` | FR-003 |
| `expect` | string | Prose for the reviewer (FR-004). |
| `design` | `DesignScreen`? | Overrides the storyboard's. |
| `settle` | `{ until = Checks, max_ms = n, watch_ms = n }`? | R4 |
| `routing` | `chain` \| `real` | Per step. |

### Checks

These are nested tables shaped like `Observation`. A leaf is one of:

| Leaf | Meaning |
|---|---|
| literal (string, int, bool) | equals |
| `{ same_as = StepRef }` | equals the same field at that earlier step |
| `{ changed = true }` / `{ unchanged = true }` | compared with the previous step |
| `{ absent = true }` | the field is `None` |
| `{ one_of = [..] }` | equals any of the values |

A check on a field the app does not observe (`None` by declaration, such as
`back_depth` on the desktop app) evaluates to **not applicable**. It is never a
pass and never a failure.

### StepOverride

| Field | Rule |
|---|---|
| `check` | replaces the step's checks for that app |
| `expect` | replaces its prose expectation |
| `skip` | `{ reason = "…" }`: the step does not exist in this app. It is shown as skipped, with the reason. |

---

## App, Axis, SeedName, PresetName

- **`App`**: `focus` \| `terminal` \| `macos`. `focus` is Postio, the one
  desktop app, and the only one with a runner (specs/007-postio-focus
  T265); the terminal and macOS name theirs when they have one. The format
  still parses `classic` for the classic app's own runner, which no script
  reaches; both go with the classic app in T256.
  `provides(App, CommandId)` comes from the registry (R7).
- **`Axis` and its values.** Each app declares its subset in `runner list`:

  | Axis | Values | Declared by |
  |---|---|---|
  | `scheme` | `light`, `dark` | Postio |
  | `width` | `wide` (1600×900), `normal` (1280×800), `narrow` (900×700) | Postio |
  | `text` | `100`, `200` | Postio |

  Postio follows the system's contrast and has one row density
  (`classic-parity.md` rows 18 and 19), so it declares no `contrast` or
  `density` axis, and a storyboard that varies one is not applicable.

- **`SeedName`**: listed in research R11. Each app declares which seeds it can
  build.
- **`PresetName`**: app-declared, such as `settings/account-form` or
  `add-account/browser`.

---

## Observation

This lives in `postio_ui::observe`. It is the same shape for every app
(FR-010), and is serialised as JSON in runs. Each app's mapping is
[contracts/observation.md](./contracts/observation.md).

| Field | Type | Meaning |
|---|---|---|
| `window` | `open` \| `closed` | The window closed, as quit does. |
| `view` | `View` | `list`, `conversation`, `reader`, `search`, `composer`, `settings`, `first_run`, `locked`, `digest`, `filtered` |
| `scope` | string? | The mailbox, the unified view, or a saved search, by display name. |
| `keyboard.region` | `Region` | `sidebar`, `list`, `reader`, `conversation`, `composer`, `search`, `palette`, `picker`, `cheatsheet`, `settings`, `dialog`, `menu`, `banner`, `none`, `other` |
| `keyboard.field` | string? | Within the region: composer `to`, `cc`, `bcc`, `subject` or `body`; or the search field. |
| `keyboard.typing` | bool | The resolver's own `typing` flag: the keyboard is on text entry. |
| `keyboard.reachable` | bool | The focused widget is mapped, inside this toplevel, and not under a modal (R3). |
| `keyboard.widget` | string | The widget path. **Informative only.** The lint rejects checks on it. |
| `cursor.index` | u32? | The list cursor's position in the model. |
| `cursor.id` | string? | The message or thread id under the cursor. It is stable for a seed. |
| `cursor.subject` | string? | Fixture subject, for the reader of a run. |
| `rows.first_visible` | u32? | The list's scroll, as the first visible row. |
| `rows.count` | u32? | Rows in the model. |
| `selection.count` | u32 | Explicitly selected rows. 0 when the cursor alone aims. |
| `overlay.kind` | `Overlay` | `none`, `palette`, `finder`, `picker`, `cheatsheet`, `dialog`, `menu`, `keymap`, `popover` |
| `overlay.mode` | string? | The finder mode, the picker kind, or the dialog name. |
| `notice.text` | string? | The toast or notice showing. |
| `notice.tone` | `info` \| `success` \| `warning` \| `error`? | |
| `notice.undo` | bool | It offers undo. |
| `banner.title` | string? | A persistent banner: offline, or sync failed. |
| `reading.id` | string? | The message or thread shown in the reading surface or dialog. |
| `reading.focused` | u32? | The focused message's index within a conversation. |
| `reading.scroll` | `{ offset: u32, max: u32 }`? | Vertical position of the reading surface. |
| `composer.open` | bool | |
| `composer.detached` | bool | |
| `back_depth` | u32? | `None` where the app has no back stack, as on the desktop app today. |
| `app` | map string → JSON | Namespaced app-specific fields: `focus.bulk`, `focus.digest_page`, `focus.bar.typed`, ... (contracts/observation.md). Shared storyboards may not check them; the lint enforces this. |

`Observation` derives `PartialEq`, so divergence (FR-016) is a field-wise
comparison of the non-informative fields. `keyboard.widget` and `app.*` are
left out of it.

---

## Run

One storyboard, on one app, in one variant, at one tree. It is written as
`run.json`.

| Field | Type |
|---|---|
| `storyboard` | name and blake3 of the file |
| `app` | `App` |
| `variant` | map `Axis → Value`, plus the requested axes that were `ignored` |
| `tree_key` | the review key (R13) |
| `commit` | sha, informative |
| `delivery` | `chain` \| `direct` \| `real` |
| `renderer` | e.g. `ngl` or `cairo` (R5) |
| `stride` | frames sampled every this many ticks (R4) |
| `seed`, `preset` | as run |
| `status` | `passed` \| `failed` \| `not_applicable{reason}` \| `not_covered{reason}` \| `unavailable{reason}` \| `error{message}` |
| `steps` | `[StepRun]` |

### StepRun

| Field | Type |
|---|---|
| `step` | `StepRef` |
| `input` | what was delivered: for a command, the chord it resolved to and the context |
| `expect` | the step's prose expectation for this app (an override's if any), so a run reads on its own |
| `outcome` | `delivered` \| `unbound{context}` \| `dropped` (the key reached nothing on the focus chain) \| `nothing_to_type_into` \| `skipped{reason}` \| `not_covered{delivery}` |
| `observation` | `Observation` |
| `checks` | `[CheckResult { path, expected, observed, result: pass\|fail\|not_applicable }]` |
| `settle` | `settled{ms}` \| `jumped{frames}` \| `blanked{frames}` \| `unsettled{ms}` \| `not_sampled` |
| `frame` | the plain frame's path and its blake3 |
| `outlined` | the outlined frame's path |
| `extra_frames` | the paths named by `jumped` or `blanked` |

**Status rules.**
- `failed` if any check fails, any step is `unbound` or `nothing_to_type_into`,
  or any step `blanked`.
- `jumped` and `unsettled` are reported but do not fail a run, because a late
  load can be legitimate (R4). The reviewer judges them.

---

## Comparison

A base run and a branch run of one storyboard, app and variant.

| Field | Type |
|---|---|
| `class` | `unchanged` \| `changed` \| `new` \| `removed` \| `base_unavailable` |
| `steps` | per step: `frame_changed` (by plain-frame hash), `observation_changed` (the field paths), and `status_changed` |

### Parity

A shared storyboard's branch runs across apps, per variant.

| Field | Type |
|---|---|
| `rows` | per step: the observation from each app |
| `diverging` | the step refs whose observations differ with no override explaining them (FR-016) |

---

## Review

Written as `verdicts.json`, schema in [contracts/review.md](./contracts/review.md).

| Field | Type |
|---|---|
| `bundle` | the bundle's id: its tree key and the base sha |
| `reviewer` | the agent definition, its model, and the prompt template's hash |
| `verdicts` | `[Verdict]`, one per step of every changed or new run |
| `findings` | `[Finding]`: what no expectation covered |
| `calibration` | the accuracy, when it was a calibration run |

### Verdict and Finding

| Field | Type |
|---|---|
| `storyboard`, `step`, `app`, `variant`, `frame` | the citation. All are required (FR-019). |
| `verdict` | `pass` \| `fail` \| `question` (Verdict only) |
| `severity` | `blocker` \| `wrong` \| `polish` (Finding, and failed Verdicts) |
| `says` | one paragraph: what a person would notice, in their terms |
| `rule` | the invariant or design rule it rests on, such as `ux-architect §2 nothing is a dead end` or `canvas 01` |
| `resolution` | `open` \| `fixed{rerun}` \| `contested{reason}`. Set by the implementer's session (FR-020). |

### Contest

These are in `contests.toml`, one per contested verdict or finding:
`{ ref, reason }`. The page shows the reason beside the verdict. A contest is
never silently dropped; only the maintainer settles it.

---

## Review page and summary

These are derived. Neither is stored as truth.

- **`index.html`** has these parts, in this order:
  1. The header: branch, base, tree key, delivery, renderer, and whether a
     review ran and is complete.
  2. **Needs you**: contests and questions.
  3. Changed storyboards: base | branch per changed step, observation diffs,
     and verdicts.
  4. New storyboards.
  5. Parity divergences.
  6. Coverage gaps.
  7. Counts of unchanged runs.
- **`summary.md`**: the same, as text, with no images. It carries the tree key
  on its first line, so `issue-land.sh` can tell whether it is current (R13).

## Coverage press

One command pressed by the generated pass (US6), in `coverage.json`: its
command and context, and what it came to -- `effect`, `no_effect`,
`listed_gap{reason}`, `stale_gap{reason}`, `unbound`, `typing` (a bare key
while the keyboard is in a text field: typed, not run, which is right), or
`dropped`. Commands that would reach outside the window are never pressed
and are listed as skipped with the reason (FR-013).

## Gap list

This is `storyboards/gaps/<app>.toml`.

| Field | Type |
|---|---|
| `command` | `CommandId` |
| `context` | the key context |
| `reason` | why it has no visible effect yet |
| `tracked` | an issue number, when one exists |

**States.** `listed`, which means it is expected to have no effect. It
becomes **`stale`** when the generated pass sees an effect (FR-029). The pass
then fails, until the row is removed.
