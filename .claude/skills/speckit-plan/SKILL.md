---
name: "speckit-plan"
description: "Execute the implementation planning workflow using the plan template to generate design artifacts."
argument-hint: "Optional guidance for the planning phase"
compatibility: "Requires spec-kit project structure with .specify/ directory"
metadata:
  author: "github-spec-kit"
  source: "templates/commands/plan.md"
user-invocable: true
disable-model-invocation: false
---


## User Input

```text
$ARGUMENTS
```

You **MUST** consider the user input before proceeding (if not empty).

## Pre-Execution Checks

Extension hooks (`before_plan`): read `.specify/extensions.yml` if it exists; on a missing file, no `hooks.before_plan` entries or unparseable YAML, skip silently.
- Drop hooks with `enabled: false` (absent means enabled). Never evaluate `condition`: no condition or empty means executable; a non-empty one means skip the hook (the HookExecutor owns it).
- Command invocation: replace `.` with `-` (`speckit.git.commit` becomes `/speckit-git-commit`).
- Optional hook (`optional: true`): print `## Extension Hooks`, `**Optional Pre-Hook**: {extension}`, `Command: /{command}`, `Description: {description}`, `Prompt: {prompt}`, `To execute: /{command}`.
- Mandatory hook (`optional: false`): print `## Extension Hooks`, `**Automatic Pre-Hook**: {extension}`, `Executing: /{command}`, `EXECUTE_COMMAND: {command}`, then actually invoke it the way this session runs commands (e.g. `/skill:speckit-...` or `$speckit-...`) and wait for it to finish before continuing; the block alone runs nothing.

## Outline

1. **Setup**: run `.specify/scripts/bash/setup-plan.sh --json` from repo root; parse FEATURE_SPEC, IMPL_PLAN, SPECS_DIR, BRANCH. Escape single quotes in args (`'I'\''m Groot'`) or double-quote.
2. **Load context**: FEATURE_SPEC, `.specify/memory/constitution.md`, and the IMPL_PLAN template (already copied).
3. **Execute** the IMPL_PLAN template structure:
   - Fill Technical Context (unknowns become "NEEDS CLARIFICATION") and the Constitution Check; evaluate gates (ERROR if violations are unjustified)
   - Phase 0: `research.md`, resolving every NEEDS CLARIFICATION
   - Phase 1: `data-model.md`, `contracts/`, `quickstart.md`
   - Re-evaluate the Constitution Check after design

## Mandatory Post-Execution Hooks

**Complete this section before reporting completion.** Same rules as the Pre-Execution Checks, with `hooks.after_plan`; skip to the Completion Report when there are none.
- Mandatory hook: emit `## Extension Hooks`, `**Automatic Hook**: {extension}`, `Executing: /{command}`, `EXECUTE_COMMAND: {command}`, then actually invoke it and wait for it to finish.
- Optional hook: emit `## Extension Hooks`, `**Optional Hook**: {extension}`, `Command: /{command}`, `Description: {description}`, `Prompt: {prompt}`, `To execute: /{command}`.

## Completion Report

The command ends after Phase 1. Report the branch, IMPL_PLAN path and generated artifacts.

## Phases

### Phase 0: Outline & Research

1. Extract unknowns from Technical Context: each NEEDS CLARIFICATION becomes a research task, each dependency a best-practices task, each integration a patterns task.
2. Dispatch research agents:

   ```text
   For each unknown in Technical Context:
     Task: "Research {unknown} for {feature context}"
   For each technology choice:
     Task: "Find best practices for {tech} in {domain}"
   ```

3. Consolidate in `research.md` as Decision / Rationale / Alternatives considered.

**Output**: research.md with all NEEDS CLARIFICATION resolved.

### Phase 1: Design & Contracts

**Prerequisite:** `research.md` complete.

1. Entities from the spec go to `data-model.md`: name, fields, relationships, validation rules from requirements, state transitions.
2. Interface contracts go to `/contracts/` if the project has external interfaces (public library APIs, CLI command schemas, web endpoints, parser grammars, UI contracts), in the format fitting the project type. Skip for purely internal projects.
3. `quickstart.md` is a validation guide: runnable end-to-end scenarios with prerequisites, setup and run commands and expected outcomes, linking to contracts and the data model rather than duplicating them. No implementation code, model/service/controller bodies, migrations or full test suites (those belong in `tasks.md` and implementation).

**Output**: data-model.md, /contracts/*, quickstart.md

## Key rules

- Absolute paths for filesystem operations; project-relative paths in documentation
- ERROR on gate failures or unresolved clarifications

## Done When

- [ ] Plan workflow executed and design artifacts generated
- [ ] Extension hooks dispatched or skipped per the Mandatory Post-Execution Hooks
- [ ] Completion reported with branch, plan path and generated artifacts
