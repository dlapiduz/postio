---

description: "Task list template for feature implementation"
---

# Tasks: [FEATURE NAME]

**Input**: Design documents from `/specs/[###-feature-name]/`
**Prerequisites**: plan.md and spec.md (required); research.md, data-model.md, contracts/
**Tests**: Include test tasks only when the spec asks for them; write them first and watch them fail.
**Organization**: Grouped by user story so each is implementable and testable alone.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: parallel (different files, no dependencies)
- **[Story]**: US1, US2, ... Include exact file paths.

<!-- The tasks below are samples; /speckit-tasks replaces them from spec.md, plan.md, data-model.md and contracts/. -->

## Phase 1: Setup (Shared Infrastructure)

- [ ] T001 [Setup task with path]

---

## Phase 2: Foundational (Blocking Prerequisites)

**CRITICAL**: No user story starts until this phase is complete.

- [ ] T002 [Foundational task with path]

**Checkpoint**: Foundation ready.

---

## Phase 3: User Story 1 - [Title] (Priority: P1) 🎯 MVP

**Goal**: [what this story delivers]

**Independent Test**: [how to verify it alone]

### Tests for User Story 1 (only if requested)

- [ ] T003 [P] [US1] [Test] in [path]

### Implementation for User Story 1

- [ ] T004 [P] [US1] [Task] in [path]
- [ ] T005 [US1] [Task] in [path] (depends on T004)

**Checkpoint**: User Story 1 works and is testable alone.

---

## Phase 4: User Story 2 - [Title] (Priority: P2)

[Same shape as Phase 3; one phase per story, in priority order.]

---

## Phase N: Polish & Cross-Cutting Concerns

- [ ] TXXX [P] [Docs, cleanup, cross-story tests, quickstart.md validation]

---

## Dependencies & Execution Order

- Setup, then Foundational (blocks all stories), then stories (parallel, or in priority order), then Polish.
- Within a story: tests, models, services, endpoints, integration.
- Stories stay independently testable; [P] tasks in a phase run in parallel.

## Implementation Strategy

- MVP first: Setup, Foundational, User Story 1, then stop and validate.
- Then add one story at a time, validating each before the next.

## Notes

- Commit after each task or logical group.
- Avoid vague tasks, same-file conflicts, and cross-story dependencies.
