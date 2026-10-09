# Specification Quality Checklist: The Compose Editor

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-09-10
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

**All 16 items pass** (validated 2026-09-10, fourth iteration).

**This is a complete specification of the compose editor, not a list of its
gaps.** Asked for explicitly, and it changed the document: behaviour that
already works — opening and where the composer lives, identity, subject,
sending, scheduling, discarding, mark-as-sent, the detached window — was
previously excluded in Assumptions as "already exists" and is now specified
alongside everything else. "Already built" is not a reason a requirement is
absent; undefined built behaviour is how it gets changed by accident.

79 requirements, numbered sequentially in document order, no dangling
cross-references.

**Clarification session 2026-09-10 (5 questions).** Three of the five answers
went against the recommendation, and two of those change existing behaviour
rather than filling a gap:

- **The quote carries the sender's sanitised HTML** (FR-042). This reverses
  the reply-construction property ADR 0003 and ADR 0004 rest on — the quote is
  currently rebuilt from a closed type precisely so that a script or tracking
  pixel has no representation to carry forward. The safety rule does not move
  (FR-045 still permits only what the reader would render), but **an ADR is
  required before this is built**, and the spec says so in Assumptions rather
  than pretending a spec can amend an ADR.
- **Changing identity never touches the body** (FR-030). Reverses today's
  replace-the-signature behaviour. Nothing can stack, because nothing is
  inserted after the draft opens.
- One-in-the-pane/many-detached (FR-010, FR-011, FR-013), one size total
  naming the largest items (FR-053, FR-054), and `undisclosed-recipients:;`
  for Bcc-only (FR-022) complete the set.

Each answer replaced the edge case that raised it, so no question is recorded
twice as both answered and open.

- **FR-064 resolved.** The one marker was a conflict rather than a gap: ADR
  0003 records a maintainer decision rejecting a Markdown-authored composer,
  and "can write markdown" read two ways. Answered 2026-09-10 — markdown is an
  **input method** inside the WYSIWYG editor, so ADR 0003 and ADR 0004 stand
  unamended and no superseding ADR is needed.
- That answer made the requirement testable rather than merely decided. FR-065
  bounds the supported sequences to formatting the editor already offers as a
  command, which is what keeps this from becoming an open-ended markdown
  dialect: no new document structure, no second plaintext path, no mode.
  SC-009 and SC-010 measure that bound.
- **Two findings came out of grounding the follow-up requests in the code**,
  and both changed the shape of the spec:
  - A reply to rich HTML quotes a *reduced* version by design — the quote is
    rebuilt from the parsed document, so anything outside the subset has no
    representation. That is a security property. FR-042 to FR-045 therefore
    ask for the result to be legible and predictable rather than faithful,
    which is achievable; asking for fidelity would have contradicted the
    hardening rule.
  - The editing surface carries **no stylesheet at all** while the reader is
    themed. That is why "visually aligned" became User Story 4 with its own
    acceptance scenarios rather than a styling note — in dark mode it is
    currently a white page.
- To/Cc/Bcc, threading and signatures were all found to exist in the model, so
  their requirements specify what the editor must *honour and show* rather
  than new mechanisms. FR-021 (Bcc never disclosed) and FR-030 (a signature
  never stacks) are the two worth a test each regardless.
- The `Context` section deliberately records that four of the five requested
  capabilities already exist. A spec that read as though none did would send a
  planner to rebuild a working composer.
- Named rather than hidden: the spec references ADRs and the command registry
  by name. These are decision records and product vocabulary rather than
  implementation detail, and omitting them would have made the markdown
  conflict invisible — which was the single most useful thing this spec had to
  say.
