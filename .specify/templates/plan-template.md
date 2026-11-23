# Implementation Plan: [FEATURE]

**Branch**: `[###-feature-name]` | **Date**: [DATE] | **Spec**: [link]
**Input**: Feature specification from `/specs/[###-feature-name]/spec.md`

## Summary

[Primary requirement and technical approach.]

## Technical Context

<!-- Fill each field, or write NEEDS CLARIFICATION. -->

**Language/Version**:
**Primary Dependencies**:
**Storage**: [or N/A]
**Testing**:
**Target Platform**:
**Project Type**:
**Performance Goals**:
**Constraints**:
**Scale/Scope**:

## Constitution Check

*Gate: passes before Phase 0 research; re-check after Phase 1 design.*

[Gates from `.specify/memory/constitution.md`.]

## Project Structure

### Documentation (this feature)

```text
specs/[###-feature]/
├── plan.md           # /speckit-plan
├── research.md       # Phase 0
├── data-model.md     # Phase 1
├── quickstart.md     # Phase 1
├── contracts/        # Phase 1
└── tasks.md          # /speckit-tasks (not created by /speckit-plan)
```

### Source Code (repository root)

<!-- The concrete crates and paths this feature touches; no option labels. -->

```text
[real paths]
```

**Structure Decision**: [chosen layout, referencing the real directories above]

## Complexity Tracking

<!-- Only when the Constitution Check has violations that must be justified. -->

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
