# Specification Quality Checklist: The Outbox, and reserved mailboxes every account has

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-09-11
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

Three items passed with a qualification worth recording rather than hiding:

- **"No implementation details"** holds for Requirements, Success Criteria,
  Assumptions and the user stories, which is where the checklist's concern
  lies. It does *not* hold for **What is here today**, which names types,
  crates and a `CHECK` constraint on purpose: the feature exists because of
  specific present behaviour, and both prior specs in this repository
  (`001-conversation-reading-pane`, `002-compose-editor`) ground themselves the
  same way. Read that section as evidence for the problem, not as a design.

- **"Success criteria are technology-agnostic"** holds, with SC-008 stated as
  bounded statements and rows. That is the project's own gating unit —
  Principle V gates budgets as counts, not timings, because a shared runner
  cannot defend a millisecond — so a storage-shaped criterion is the
  *technology-agnostic* form here, not a leak.

- **"No [NEEDS CLARIFICATION] markers remain"** holds because all three open
  decisions were put to the maintainer on 2026-09-11 and answered: what the
  Outbox holds, where failed sends live, and what happens when a reserved role
  has no server folder. The answers and their rejected alternatives are in
  Assumptions.

One thing this checklist cannot certify, recorded for planning:

- **The feature has an unmerged dependency.** `feature/mailbox-roles` (epic
  #962) holds the per-account role map that FR-033 and FR-035 rest on, 15
  commits ahead of `main` with no pull request since 2026-09-04. The spec is
  complete; the *plan* is blocked on that branch merging or being explicitly
  abandoned. Settle it before `/speckit-plan`.
