---
name: "speckit-implement"
description: "Execute the implementation plan by processing and executing all tasks defined in tasks.md"
argument-hint: "Optional implementation guidance or task filter"
compatibility: "Requires spec-kit project structure with .specify/ directory"
metadata:
  author: "github-spec-kit"
  source: "templates/commands/implement.md"
user-invocable: true
disable-model-invocation: false
---


## User Input

```text
$ARGUMENTS
```

You **MUST** consider the user input before proceeding (if not empty).

## Pre-Execution Checks

Extension hooks (`before_implement`): read `.specify/extensions.yml` if it exists; on a missing file, no `hooks.before_implement` entries or unparseable YAML, skip silently.
- Drop hooks with `enabled: false` (absent means enabled). Never evaluate `condition`: no condition or empty means executable; a non-empty one means skip the hook (the HookExecutor owns it).
- Command invocation: replace `.` with `-` (`speckit.git.commit` becomes `/speckit-git-commit`).
- Optional hook (`optional: true`): print `## Extension Hooks`, `**Optional Pre-Hook**: {extension}`, `Command: /{command}`, `Description: {description}`, `Prompt: {prompt}`, `To execute: /{command}`.
- Mandatory hook (`optional: false`): print `## Extension Hooks`, `**Automatic Pre-Hook**: {extension}`, `Executing: /{command}`, `EXECUTE_COMMAND: {command}`, then actually invoke it the way this session runs commands (e.g. `/skill:speckit-...` or `$speckit-...`) and wait for it to finish before continuing; the block alone runs nothing.

## Outline

1. Run `.specify/scripts/bash/check-prerequisites.sh --json --require-tasks --include-tasks` from repo root; parse FEATURE_DIR and AVAILABLE_DOCS (absolute paths). Escape single quotes in args (`'I'\''m Groot'`) or double-quote.

2. **Check checklists** (if FEATURE_DIR/checklists/ exists). Checklist markers are a read-only gate: never modify checklist files. `checklists/requirements.md` is the spec-quality checklist maintained by `/speckit-specify` and `/speckit-clarify`.
   - For each file count total items (`- [ ]`, `- [X]`, `- [x]`), checked (`- [X]`, `- [x]`) and unchecked (`- [ ]`), and show:

     ```text
     | Checklist | Total | Checked | Unchecked | Status |
     |-----------|-------|---------|-----------|--------|
     | ux.md     | 12    | 12      | 0         | ✓ PASS |
     | test.md   | 8     | 5       | 3         | ✗ FAIL |
     ```

   - PASS: no unchecked items anywhere; go to step 3.
   - FAIL: show the table, **STOP** and ask "Some checklists have unchecked items. Do you want to proceed with implementation anyway? (yes/no)". "no", "wait" or "stop" halts; "yes", "proceed" or "continue" goes to step 3.

3. Load context. Required: tasks.md (task list, execution plan), plan.md (tech stack, architecture, file structure). If present: data-model.md, contracts/ (API specs, test requirements), research.md, `.specify/memory/constitution.md`, quickstart.md.

4. **Verify ignore files** from the real project setup:
   - `git rev-parse --git-dir 2>/dev/null` succeeds: `.gitignore`; Dockerfile* or Docker in plan.md: `.dockerignore`; `.eslintrc*`: `.eslintignore` (with `eslint.config.*`, ensure its `ignores` cover the patterns); `.prettierrc*`: `.prettierignore`; `.npmrc` or package.json (if publishing): `.npmignore`; `*.tf`: `.terraformignore`; Helm charts: `.helmignore`.
   - Existing file: append only missing critical patterns. Missing file: create with the full set for the detected technology.
   - Patterns by technology (from plan.md):
     - Node/JS/TS: `node_modules/`, `dist/`, `build/`, `*.log`, `.env*`
     - Python: `__pycache__/`, `*.pyc`, `.venv/`, `venv/`, `dist/`, `*.egg-info/`
     - Java: `target/`, `*.class`, `*.jar`, `.gradle/`, `build/`
     - C#/.NET: `bin/`, `obj/`, `*.user`, `*.suo`, `packages/`
     - Go: `*.exe`, `*.test`, `vendor/`, `*.out`
     - Ruby: `.bundle/`, `log/`, `tmp/`, `*.gem`, `vendor/bundle/`
     - PHP: `vendor/`, `*.log`, `*.cache`, `*.env`
     - Rust: `target/`, `debug/`, `release/`, `*.rs.bk`, `*.rlib`, `*.prof*`, `.idea/`, `*.log`, `.env*`
     - Kotlin: `build/`, `out/`, `.gradle/`, `.idea/`, `*.class`, `*.jar`, `*.iml`, `*.log`, `.env*`
     - C++: `build/`, `bin/`, `obj/`, `out/`, `*.o`, `*.so`, `*.a`, `*.exe`, `*.dll`, `.idea/`, `*.log`, `.env*`
     - C: as C++ plus `autom4te.cache/`, `config.status`, `config.log`
     - Swift: `.build/`, `DerivedData/`, `*.swiftpm/`, `Packages/`
     - R: `.Rproj.user/`, `.Rhistory`, `.RData`, `.Ruserdata`, `*.Rproj`, `packrat/`, `renv/`
     - Universal: `.DS_Store`, `Thumbs.db`, `*.tmp`, `*.swp`, `.vscode/`, `.idea/`
   - Patterns by tool:
     - Docker: `node_modules/`, `.git/`, `Dockerfile*`, `.dockerignore`, `*.log*`, `.env*`, `coverage/`
     - ESLint: `node_modules/`, `dist/`, `build/`, `coverage/`, `*.min.js`
     - Prettier: `node_modules/`, `dist/`, `build/`, `coverage/`, `package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`
     - Terraform: `.terraform/`, `*.tfstate*`, `*.tfvars`, `.terraform.lock.hcl`
     - Kubernetes: `*.secret.yaml`, `secrets/`, `.kube/`, `kubeconfig*`, `*.key`, `*.crt`

5. Parse tasks.md for phases (Setup, Tests, Core, Integration, Polish), dependencies (sequential vs parallel), task details (ID, description, file paths, `[P]`) and execution order.

6. **Execute** phase by phase, validating each phase before the next. Run sequential tasks in order and `[P]` tasks together; tasks touching the same file run sequentially; test tasks come before their implementation (TDD). Within the plan: setup (structure, dependencies, configuration), tests for contracts, entities and integration, core (models, services, CLI, endpoints), integration (database, middleware, logging, external services), polish (unit tests, performance, documentation).

7. **Progress and errors**:
   - Report after each task; give clear error messages with context; suggest next steps if blocked.
   - Halt if a non-parallel task fails; for `[P]` tasks continue with the successful ones and report the failures.
   - **Mark each completed task `[X]` in tasks.md.**

8. **Validate completion**: all required tasks done, features match the spec, tests pass and coverage meets requirements, implementation follows the plan.

This command assumes a complete tasks.md; if tasks are incomplete or missing, suggest `/speckit-tasks` first.

## Mandatory Post-Execution Hooks

**Complete this section before reporting completion.** Same rules as the Pre-Execution Checks, with `hooks.after_implement`; skip to the Completion Report when there are none.
- Mandatory hook: emit `## Extension Hooks`, `**Automatic Hook**: {extension}`, `Executing: /{command}`, `EXECUTE_COMMAND: {command}`, then actually invoke it and wait for it to finish.
- Optional hook: emit `## Extension Hooks`, `**Optional Hook**: {extension}`, `Command: /{command}`, `Description: {description}`, `Prompt: {prompt}`, `To execute: /{command}`.

## Completion Report

Report final status with summary of completed work.

## Done When

- [ ] All tasks in tasks.md completed and marked `[X]`
- [ ] Implementation validated against specification, plan, and test coverage
- [ ] Extension hooks dispatched or skipped according to the rules in Mandatory Post-Execution Hooks above
- [ ] Completion reported to user with summary of completed work
