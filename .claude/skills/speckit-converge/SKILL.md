---
name: "speckit-converge"
description: "Assess the current codebase against the feature's spec, plan, and tasks, then append any remaining unbuilt work as new tasks to tasks.md so implement can complete it."
compatibility: "Requires spec-kit project structure with .specify/ directory"
metadata:
  author: "github-spec-kit"
  source: "templates/commands/converge.md"
user-invocable: true
disable-model-invocation: false
---


## User Input

```text
$ARGUMENTS
```

You **MUST** consider the user input before proceeding (if not empty).

## Pre-Execution Checks

Extension hooks (`before_converge`): read `.specify/extensions.yml` if it exists; on a missing file, no `hooks.before_converge` entries or unparseable YAML, skip silently.
- Drop hooks with `enabled: false` (absent means enabled). Never evaluate `condition`: no condition or empty means executable; a non-empty one means skip the hook (the HookExecutor owns it).
- Command invocation: replace `.` with `-` (`speckit.git.commit` becomes `/speckit-git-commit`).
- Optional hook (`optional: true`): print `## Extension Hooks`, `**Optional Pre-Hook**: {extension}`, `Command: /{command}`, `Description: {description}`, `Prompt: {prompt}`, `To execute: /{command}`.
- Mandatory hook (`optional: false`): print `## Extension Hooks`, `**Automatic Pre-Hook**: {extension}`, `Executing: /{command}`, `EXECUTE_COMMAND: {command}`, then actually invoke it the way this session runs commands (e.g. `/skill:speckit-...` or `$speckit-...`) and wait for it to finish before continuing; the block alone runs nothing.

## Goal

Close the gap between what `spec.md`, `plan.md` and `tasks.md` call for (the **sole source of intent**, under the constitution) and what the codebase implements: find unmet, incomplete or partial requirements, acceptance criteria, plan decisions and tasks, and **append each piece of remaining work as a new traceable task** at the bottom of `tasks.md` for `/speckit-implement`. Run only after `/speckit-tasks` produced a complete `tasks.md` and `/speckit-implement` has run on it.

It assesses the present code against the artifacts. It is not a diff tool: no git, no branch comparison, no history.

## Operating Constraints

**APPEND-ONLY, NEVER REWRITE.** The only write is appending a `## Phase N: Convergence` section to `tasks.md`. Never:
- modify `spec.md` or `plan.md`;
- rewrite, renumber, reorder or delete any existing task, including a prior Convergence phase;
- modify, create or delete application code (completing the tasks is `/speckit-implement`'s job).

When the code already satisfies everything, leave `tasks.md` **byte-for-byte unchanged** (no empty Convergence header) and report a clean result.

**Constitution authority**: `.specify/memory/constitution.md` is non-negotiable. Code violating a MUST is the highest-severity finding and gets a remediation task. If the constitution is an unfilled template, skip its checks.

## Execution Steps

### 1. Initialize

Run `.specify/scripts/bash/check-prerequisites.sh --json --require-spec --require-tasks --include-tasks` once from repo root; parse FEATURE_DIR and AVAILABLE_DOCS. SPEC, PLAN and TASKS are FEATURE_DIR/spec.md, plan.md, tasks.md (absolute); CONSTITUTION is `.specify/memory/constitution.md` if present. If any of the three is missing, STOP with an actionable message naming the command (`/speckit-specify`, `/speckit-plan`, `/speckit-tasks`); no partial output. Escape single quotes in args (`'I'\''m Groot'`) or double-quote.

### 2. Load artifacts (progressive disclosure)

- spec.md: Functional Requirements (FR-###); Success Criteria (SC-###) needing buildable work only (no post-launch outcome metrics or business KPIs); User Stories with Acceptance Scenarios; Edge Cases
- plan.md: architecture, stack and technical decisions; data model references; phases and named touch-points (files and components to create or edit); technical constraints
- tasks.md: task IDs (to compute the next ID and phase number), descriptions, phase grouping, file paths
- constitution (if not an unfilled template): principle names and MUST/SHOULD statements

### 3. Build the intent inventory (internal)

- **Requirements inventory**: a stable key per FR-### / SC-### / acceptance scenario (e.g. `US1/AC2`), plus the plan decisions and constitution principles that impose buildable obligations.
- **Code-scope map**: the files and components from the paths in `plan.md` and `tasks.md` plus a keyword search for each requirement's concepts. Bound the assessment to these; infer no wider scope.

### 4. Assess the codebase

Inspect the in-scope code for each inventory item and make a `Finding` only for a gap, classified by **gap type**:
- `missing`: the work is absent
- `partial`: it exists but does not fully satisfy the requirement, criterion or decision
- `contradicts`: the code conflicts with stated intent or a constitution MUST
- `unrequested`: work no artifact calls for (surfaced only: converge deletes nothing, it appends a task to review, justify or remove it)

A finding records a stable id, its `source-ref`, `gap-type`, severity, and a description with the evidence (file or area). Little or no code yet: the whole specified scope is `missing`. Nothing remains: zero findings, take the converged branch in Step 7.

### 5. Severity

- **CRITICAL**: violates a constitution MUST, or a `missing`/`contradicts` gap blocking baseline functionality of a P1 story
- **HIGH**: `missing` or `partial` on a core requirement or acceptance criterion
- **MEDIUM**: `partial` on a secondary requirement, or an `unrequested` addition with unclear justification
- **LOW**: minor partial gaps, polish, low-risk `unrequested` additions

### 6. In-session findings summary (before appending)

## Convergence Findings

| ID | Gap Type | Severity | Source | Evidence | Remaining Work |
|----|----------|----------|--------|----------|----------------|
| F1 | missing  | HIGH     | FR-008 | Example: no append-only guard detected in path/to/module.py when writing tasks.md | Add append-only enforcement |

**Summary metrics:** requirements and acceptance criteria checked; plan decisions checked; constitution principles checked (or "skipped — template"); findings by gap type (missing / partial / contradicts / unrequested); findings by severity.

### 7. Append tasks, or report converged

**Actionable findings** (`tasks_appended`): at the **end** of `tasks.md`:
1. Let `M` be the maximum existing task ID and `N` the highest existing phase plus 1.
2. Write one header `## Phase N: Convergence`.
3. Emit one item per finding, CRITICAL/HIGH first (constitution violations first, described as `CRITICAL`), with IDs `T{M+1:03d}, T{M+2:03d}, …`:

   ```markdown
   - [ ] T042 <imperative description> per <source-ref> (<gap-type>)
   ```

   `<source-ref>` traces the origin (`FR-003`, `SC-002`, `US1/AC2`, `plan: storage decision`, `Constitution II`); `<gap-type>` is `missing`, `partial`, `contradicts` or `unrequested`.
4. Never reuse or renumber IDs. With a prior Convergence phase, add a new separately-numbered one below it and leave the old one alone.

**No actionable findings** (`converged`): do not modify `tasks.md` at all. Report **"✅ Converged — the implementation satisfies the spec, plan, and tasks."** with the summary counts of what was checked.

### 8. Next actions

- `tasks_appended`: state how many tasks went under which phase and recommend `/speckit-implement`; a later converge run will find fewer or none.
- `converged`: recommend review and opening a PR; no further implement pass is needed for the specified scope.

### 9. Extension hooks

After reporting the outcome (`converged` or `tasks_appended`) in-session, run the `hooks.after_converge` hooks from `.specify/extensions.yml` by the same rules as the other speckit skills' Post-Execution Hooks: skip silently if the file is missing or unparseable; drop `enabled: false`; never evaluate `condition` (non-empty means skip); replace `.` with `-` in command names.
- Optional hook: emit `## Extension Hooks`, `**Optional Hook**: {extension}`, `Command: /{command}`, `Description: {description}`, `Prompt: {prompt}`, `To execute: /{command}`.
- Mandatory hook: emit `## Extension Hooks`, `**Automatic Hook**: {extension}`, `Executing: /{command}`, `EXECUTE_COMMAND: {command}`, then actually invoke it and wait for it to finish.
