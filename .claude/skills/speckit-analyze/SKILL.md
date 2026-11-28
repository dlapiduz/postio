---
name: "speckit-analyze"
description: "Perform a non-destructive cross-artifact consistency and quality analysis across spec.md, plan.md, and tasks.md after task generation."
argument-hint: "Optional focus areas for analysis"
compatibility: "Requires spec-kit project structure with .specify/ directory"
metadata:
  author: "github-spec-kit"
  source: "templates/commands/analyze.md"
user-invocable: true
disable-model-invocation: false
---


## User Input

```text
$ARGUMENTS
```

You **MUST** consider the user input before proceeding (if not empty).

## Pre-Execution Checks

Extension hooks (`before_analyze`): read `.specify/extensions.yml` if it exists; on a missing file, no `hooks.before_analyze` entries or unparseable YAML, skip silently.
- Drop hooks with `enabled: false` (absent means enabled). Never evaluate `condition`: no condition or empty means executable; a non-empty one means skip the hook (the HookExecutor owns it).
- Command invocation: replace `.` with `-` (`speckit.git.commit` becomes `/speckit-git-commit`).
- Optional hook (`optional: true`): print `## Extension Hooks`, `**Optional Pre-Hook**: {extension}`, `Command: /{command}`, `Description: {description}`, `Prompt: {prompt}`, `To execute: /{command}`.
- Mandatory hook (`optional: false`): print `## Extension Hooks`, `**Automatic Pre-Hook**: {extension}`, `Executing: /{command}`, `EXECUTE_COMMAND: {command}`, then actually invoke it the way this session runs commands (e.g. `/skill:speckit-...` or `$speckit-...`) and wait for it to finish before continuing; the block alone runs nothing.

## Goal

Find inconsistencies, duplications, ambiguities and underspecified items across `spec.md`, `plan.md` and `tasks.md` before implementation. Run only after `/speckit-tasks` has produced a complete `tasks.md`.

## Operating Constraints

- **STRICTLY READ-ONLY**: modify no files. Output a structured report and offer a remediation plan; the user must explicitly approve before any follow-up edit.
- **Constitution authority**: `.specify/memory/constitution.md` is non-negotiable here. A conflict is automatically CRITICAL and is fixed in the spec, plan or tasks, never by diluting or ignoring the principle. Changing a principle is a separate constitution update.

## Execution Steps

### 1. Initialize

Run `.specify/scripts/bash/check-prerequisites.sh --json --require-spec --require-tasks --include-tasks` once from repo root; parse FEATURE_DIR and AVAILABLE_DOCS. SPEC, PLAN and TASKS are FEATURE_DIR/spec.md, plan.md and tasks.md (absolute). If one is missing, abort and name the prerequisite command to run. Escape single quotes in args (`'I'\''m Groot'`) or double-quote.

### 2. Load artifacts (progressive disclosure, minimal context)

- spec.md: Overview/Context, Functional Requirements, Success Criteria (measurable outcomes), User Stories, Edge Cases
- plan.md: architecture and stack, data model references, phases, technical constraints
- tasks.md: IDs, descriptions, phase grouping, `[P]` markers, file paths
- `.specify/memory/constitution.md`: for principle validation

### 3. Build internal models (not in the output)

- **Requirements inventory**: a stable key per Functional Requirement (FR-###) and Success Criterion (SC-###), the explicit id as primary key, optionally a slug (`user-can-upload-file`). Include only Success Criteria that need buildable work (load-testing infrastructure, security audit tooling); exclude post-launch outcome metrics and business KPIs ("Reduce support tickets by 50%").
- **User story/action inventory**: discrete actions with acceptance criteria
- **Task coverage map**: each task to requirements or stories, by explicit id or keyword
- **Constitution rule set**: principle names and MUST/SHOULD statements

### 4. Detection passes

High-signal findings only; at most 50, with the remainder summarised as overflow.
- **A. Duplication**: near-duplicate requirements; mark the weaker phrasing for consolidation
- **B. Ambiguity**: vague adjectives (fast, scalable, secure, intuitive, robust) without measurable criteria; unresolved placeholders (TODO, TKTK, ???, `<placeholder>`)
- **C. Underspecification**: requirements with a verb but no object or measurable outcome; stories without matching acceptance criteria; tasks naming files or components absent from spec/plan
- **D. Constitution alignment**: requirements or plan elements conflicting with a MUST; missing mandated sections or quality gates
- **E. Coverage gaps**: requirements with no task; tasks with no requirement or story; buildable Success Criteria (performance, security, availability) absent from tasks
- **F. Inconsistency**: terminology drift; entities in plan but not spec (or reverse); task ordering contradictions (integration before foundational setup without a dependency note); conflicting requirements (Next.js vs Vue)

### 5. Severity

- **CRITICAL**: violates a constitution MUST, missing core artifact, or zero-coverage requirement blocking baseline functionality
- **HIGH**: duplicate or conflicting requirement, ambiguous security/performance attribute, untestable acceptance criterion
- **MEDIUM**: terminology drift, missing non-functional task coverage, underspecified edge case
- **LOW**: wording or minor redundancy not affecting execution order

### 6. Report (Markdown, no file writes)

## Specification Analysis Report

| ID | Category | Severity | Location(s) | Summary | Recommendation |
|----|----------|----------|-------------|---------|----------------|
| A1 | Duplication | HIGH | spec.md:L120-134 | Two similar requirements ... | Merge phrasing; keep clearer version |

One row per finding, with stable IDs prefixed by the category initial. Then:

**Coverage Summary Table:**

| Requirement Key | Has Task? | Task IDs | Notes |
|-----------------|-----------|----------|-------|

**Constitution Alignment Issues:** (if any)

**Unmapped Tasks:** (if any)

**Metrics:** Total Requirements; Total Tasks; Coverage % (requirements with at least one task); Ambiguity Count; Duplication Count; Critical Issues Count.

### 7. Next actions

A concise block: CRITICAL issues mean resolve before `/speckit-implement`; only LOW/MEDIUM means the user may proceed, with suggestions. Give explicit commands, e.g. "Run /speckit-specify with refinement", "Run /speckit-plan to adjust architecture", "Manually edit tasks.md to add coverage for 'performance-metrics'".

### 8. Offer remediation

Ask: "Would you like me to suggest concrete remediation edits for the top N issues?" Do NOT apply them automatically.

### 9. Extension hooks

After reporting, run the `hooks.after_analyze` hooks from `.specify/extensions.yml` by the rules of the other speckit skills' Post-Execution Hooks: skip silently if the file is missing or unparseable; drop `enabled: false`; never evaluate `condition` (non-empty means skip); replace `.` with `-` in command names.
- Optional hook: emit `## Extension Hooks`, `**Optional Hook**: {extension}`, `Command: /{command}`, `Description: {description}`, `Prompt: {prompt}`, `To execute: /{command}`.
- Mandatory hook: emit `## Extension Hooks`, `**Automatic Hook**: {extension}`, `Executing: /{command}`, `EXECUTE_COMMAND: {command}`, then actually invoke it and wait for it to finish.

## Operating Principles

- Minimal high-signal tokens; load artifacts incrementally; limit the findings table to 50 rows and summarise overflow.
- Deterministic: rerunning without changes gives the same IDs and counts.
- NEVER modify files; NEVER hallucinate missing sections (report them as absent).
- Constitution violations are always CRITICAL.
- Cite specific instances, not generic patterns.
- Report zero issues gracefully: a success report with coverage statistics.

## Context

$ARGUMENTS
