---
name: "speckit-tasks"
description: "Generate an actionable, dependency-ordered tasks.md for the feature based on available design artifacts."
argument-hint: "Optional task generation constraints"
compatibility: "Requires spec-kit project structure with .specify/ directory"
metadata:
  author: "github-spec-kit"
  source: "templates/commands/tasks.md"
user-invocable: true
disable-model-invocation: false
---


## User Input

```text
$ARGUMENTS
```

You **MUST** consider the user input before proceeding (if not empty).

## Pre-Execution Checks

Extension hooks (`before_tasks`): read `.specify/extensions.yml` if it exists; on a missing file, no `hooks.before_tasks` entries or unparseable YAML, skip silently.
- Drop hooks with `enabled: false` (absent means enabled). Never evaluate `condition`: no condition or empty means executable; a non-empty one means skip the hook (the HookExecutor owns it).
- Command invocation: replace `.` with `-` (`speckit.git.commit` becomes `/speckit-git-commit`).
- Optional hook (`optional: true`): print `## Extension Hooks`, `**Optional Pre-Hook**: {extension}`, `Command: /{command}`, `Description: {description}`, `Prompt: {prompt}`, `To execute: /{command}`.
- Mandatory hook (`optional: false`): print `## Extension Hooks`, `**Automatic Pre-Hook**: {extension}`, `Executing: /{command}`, `EXECUTE_COMMAND: {command}`, then actually invoke it the way this session runs commands (e.g. `/skill:speckit-...` or `$speckit-...`) and wait for it to finish before continuing; the block alone runs nothing.

## Outline

1. **Setup**: run `.specify/scripts/bash/setup-tasks.sh --json` from repo root; parse FEATURE_DIR, TASKS_TEMPLATE_CONTENT, TASKS_TEMPLATE and AVAILABLE_DOCS (document names or relative paths under FEATURE_DIR, e.g. `research.md`, `contracts/`). FEATURE_DIR and TASKS_TEMPLATE are absolute. Escape single quotes in args (`'I'\''m Groot'`) or double-quote.

2. **Load design documents** from FEATURE_DIR, using what exists:
   - Required: plan.md (tech stack, libraries, structure), spec.md (user stories with priorities)
   - Optional: data-model.md (entities), contracts/, research.md (decisions), quickstart.md (test scenarios)
   - `.specify/memory/constitution.md` if it exists

3. **Generate**: map entities and contracts to user stories, take research decisions for setup tasks, organise tasks by user story (Task Generation Rules below), and add a dependency graph of story completion order, parallel examples per story, and a check that each story has every task it needs and is independently testable.

4. **Write tasks.md** from TASKS_TEMPLATE_CONTENT (older setup scripts omit it: read TASKS_TEMPLATE), with the feature name from plan.md and:
   - Phase 1 Setup (project initialization); Phase 2 Foundational (blocking prerequisites for all stories); Phase 3+ one phase per user story in spec.md priority order (story goal, independent test criteria, tests if requested, implementation tasks); final Phase Polish & cross-cutting concerns
   - every task in the strict checklist format with a clear file path
   - a Dependencies section (story completion order), parallel examples per story, and an Implementation strategy section (MVP first, incremental delivery)

## Mandatory Post-Execution Hooks

**Complete this section before reporting completion.** Same rules as the Pre-Execution Checks, with `hooks.after_tasks`; skip to the Completion Report when there are none.
- Mandatory hook: emit `## Extension Hooks`, `**Automatic Hook**: {extension}`, `Executing: /{command}`, `EXECUTE_COMMAND: {command}`, then actually invoke it and wait for it to finish.
- Optional hook: emit `## Extension Hooks`, `**Optional Hook**: {extension}`, `Command: /{command}`, `Description: {description}`, `Prompt: {prompt}`, `To execute: /{command}`.

## Completion Report

Output the path to the generated tasks.md and:
- total task count, and count per user story
- parallel opportunities
- independent test criteria per story
- suggested MVP scope (typically User Story 1)
- format validation: ALL tasks have checkbox, ID, labels and file paths

Context for task generation: $ARGUMENTS

tasks.md must be immediately executable: each task specific enough for an LLM to complete without more context.

## Task Generation Rules

**Tasks MUST be organised by user story** for independent implementation and testing. **Tests are optional**: generate test tasks only if the spec requests them or the user asks for TDD.

### Checklist Format (REQUIRED)

```text
- [ ] [TaskID] [P?] [Story?] Description with file path
```

1. Start with `- [ ]`.
2. Task ID: sequential T001, T002... in execution order.
3. `[P]` only if parallelizable (different files, no dependency on incomplete tasks).
4. `[US1]`, `[US2]`... (from spec.md) REQUIRED in user story phases only; none in Setup, Foundational or Polish.
5. Clear action with the exact file path.

- CORRECT: `- [ ] T001 Create project structure per implementation plan`
- CORRECT: `- [ ] T005 [P] Implement authentication middleware in src/middleware/auth.py`
- CORRECT: `- [ ] T012 [P] [US1] Create User model in src/models/user.py`
- CORRECT: `- [ ] T014 [US1] Implement UserService in src/services/user_service.py`
- WRONG: `- [ ] Create User model` (no ID or story label)
- WRONG: `T001 [US1] Create model` (no checkbox)
- WRONG: `- [ ] [US1] Create User model` (no Task ID)
- WRONG: `- [ ] T001 [US1] Create model` (no file path)

### Task Organization

1. **User stories (spec.md)** are the primary organisation: each story (P1, P2, P3...) gets a phase holding its models, services, interfaces/UI and, if requested, tests. Mark story dependencies; most stories should be independent.
2. **Contracts**: map each to the story it serves; if tests are requested, a contract test task [P] before implementation in that story's phase.
3. **Data model**: map each entity to the stories that need it (one serving several goes in the earliest story or Setup); relationships become service-layer tasks in the right story phase.
4. **Infrastructure**: shared goes in Setup (Phase 1), blocking in Foundational (Phase 2), story-specific inside the story's phase.

### Phase Structure

- Phase 1: Setup (project initialization)
- Phase 2: Foundational (MUST complete before any user story)
- Phase 3+: user stories in priority order; within each: tests (if requested), models, services, endpoints, integration; each phase an independently testable increment
- Final: Polish & Cross-Cutting Concerns

## Done When

- [ ] tasks.md generated with all phases, task IDs and file paths
- [ ] Extension hooks dispatched or skipped per the Mandatory Post-Execution Hooks
- [ ] Completion reported with task count, story breakdown and MVP scope
