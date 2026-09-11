# Specification Quality Checklist: The Conversation Reading Pane

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-09-08
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

**2026-09-08, second pass — the design brief, then resolved.** All three
conflicts the brief raised were decided by the maintainer the same day and are
recorded in the spec's **Decisions** section: sender styling is rendered and the
brief's layout contract is refused; the brief's "nothing is hidden" is adopted
but the pane opens on the most recent message; the brief's proposed keyboard
shortcuts are disregarded in favour of the registry.

**One consequence needs carrying out of this spec.** FR-013 and FR-015
supersede ADR 0015 in two places — *"read ones collapsed"* and *"first unread
in focus"* — and `postio-ui/src/conversation.rs`'s `collapsed_runs()` is the
shipped implementation of the superseded rule. The ADR amendment and that
removal belong to the plan, not to a later discovery.

**2026-09-08, third pass — the cost of moving.** FR-057 to FR-065 and SC-009a
to SC-009d were added on the maintainer's prompt that switching messages had
been a problem before. They are written as regression locks: every mechanism
they forbid was diagnosed on #749 and fixed, and this spec replaces the pane
those fixes live in. Per Principle V they are stated as counts — renders per
gesture, surfaces per conversation, bytes per document — not as timings, so
they can gate on a shared runner.

Everything in the brief that does not conflict has been applied: the pinned
two-row header, action scoping with archive at conversation scope, the required
scoping note and worded tooltips, per-message actions on hover and focus that
reserve no space, the reading measure and in-block overflow scrolling, lazy
body preparation, and the rail with its derivation, degradation and
accessibility rules.

- **All three open questions were answered by the maintainer on 2026-09-08** and
  are recorded in the spec's **Decisions** section: full layout fidelity with
  the privacy posture unchanged; the conversation action bar fixed to the most
  recent message; the spec kept mechanism-neutral against #1316.
- **Two residual risks are accepted rather than resolved**, and are recorded
  here so that planning does not rediscover them as surprises:
  1. *Fidelity vs. containment.* FR-019 admits sender CSS, so FR-020 (a sender's
     styling cannot leave its message) and FR-021 (a message cannot escape its
     bounds) become load-bearing security requirements rather than properties
     the sanitizer supplied for free. SC-006a exists to test them.
  2. *A fixed action-bar target.* FR-010 fixes the bar to the most recent
     message. A user reading an older message who activates the bar acts on a
     different message than the one on screen. FR-010a is the mitigation.
- **Naming an existing surface is not an implementation detail.** The spec
  cites ADR 0032, #1316, #1285, #1259 and #946 in Context and Dependencies so
  that a reader knows which defects this replaces. No requirement names a
  toolkit, a widget, or a rendering mechanism.
- Items marked incomplete require spec updates before `/speckit-clarify` or
  `/speckit-plan`.
