# Specification Quality Checklist: Postio Focus

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-09-26
**Feature**: [spec.md](../spec.md)

## Content Quality

- [x] No implementation details (languages, frameworks, APIs)
- [x] Focused on user value and business needs
- [x] Written for non-technical stakeholders
- [x] All mandatory sections completed

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain
- [x] Requirements are testable and unambiguous
- [x] Success criteria are measurable
- [x] Success criteria are technology-agnostic (no implementation details)
- [x] All acceptance scenarios are defined
- [x] Edge cases are identified
- [x] Scope is clearly bounded
- [x] Dependencies and assumptions identified

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria
- [x] User scenarios cover primary flows
- [x] Feature meets measurable outcomes defined in Success Criteria
- [x] No implementation details leak into specification

## Notes

- **Implementation names are there on purpose, as in spec 005.** Crate and
  tool names (`postio-host`, `postio-client`, `postio-ui`, the boundary
  check, the counting harness) appear only where the constitution or the
  maintainer's handoff makes them a constraint: which boundary Focus may not
  cross, what it must reuse rather than rewrite, and how budgets are gated.
  How to build anything is left to the plan.
- **The reader is technical.** This spec is written for the maintainer and
  for the agents who will plan and build it, the same audience as every spec
  in `specs/`. It keeps to what a person sees and does, and says why.
- **Decisions confirmed in clarify (2026-09-26).** The handoff's six open
  decisions went to the maintainer, with two more the spec raised: which
  renderer planning waits for, and whether Focus's rules act while another
  app runs. All eight are recorded in the spec's Clarifications.
  - Three changed what Focus does: one keymap for every app, digest
    summaries as drawn, and Focus's rules acting only while it runs.
  - One changed how the branch ends: nothing lands until the maintainer
    says so.
  - Two more came unasked:
    - The local model is the user's own and optional, and Postio never
      embeds or ships one (FR-165 to FR-169).
    - A simple built-in detector marks questions and to-dos when no model is
      connected (FR-104 to FR-108).
