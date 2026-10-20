# Specification Quality Checklist: Contacts for Postio Focus

**Purpose**: Validate specification completeness and quality before tasks
**Created**: 2026-10-19; **Revised**: 2026-10-19 (the designer's handoff)
**Feature**: [spec.md](../spec.md)

## Content Quality

- [x] Surface, keys and states are described by what a person sees; storage and wire detail is cited from spec 005 or lives in `plan.md`
- [x] Focused on user value (know a sender, review and tidy, merge, keep in sync)
- [x] All mandatory sections completed
- [x] This is a Postio spec: it names registry ids and crates where the constitution requires it (command table, FR-018, FR-019)
- [x] The design source is cited by filename and nothing from `Design/` is copied into the repository
- [x] Examples use invented names and reserved domains

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers; maintainer-only calls are in "Questions for the maintainer" with options and a recommendation each
- [x] The maintainer's decisions of 2026-10-19 are recorded with the date (M1 to M8)
- [x] Where the handoff supersedes the first draft, the table says what and why
- [x] Requirements are testable and unambiguous
- [x] Success criteria are measurable (SC-001 to SC-008)
- [x] All acceptance scenarios are defined
- [x] Edge cases are identified (empty, catch-up, offline, failing, large store, privacy)
- [x] Scope is clearly bounded (Out of Scope: Finder, import undo, Groups and Deleted tabs, photos before sync)
- [x] Dependencies and assumptions identified (the 005 port, what is asked of it)

## Feature Readiness

- [x] Every key in the handoff is checked against `registry.rs`, `docs/keybindings.md`, the earlier handoff's `KEYS.md`, and `U`, `i`, `x`, `g a`, with a resolution per collision
- [x] The duplicates-count decision (M6) is checked against the handoff's "Worth a look" line: no conflict
- [x] The migration is specified to change no behaviour (SC-005) and not to block startup
- [x] Sync safety is specified: writes off until reviewed, scratch container only, permission only on enable, dry-run never logged
- [x] What changes for Linux is listed per slice, with the items that need the maintainer's OK before merge

## Open before tasks

- [ ] Q1 to Q13 answered, above all Q1 (where settings live), Q2 (`i` against `i i`), Q3 (one Contacts permission)
- [ ] The 005 port's schema confirmed against `plan.md` section 2

## Notes

- The handoff's `PROMPT.md` asks for a plan and the maintainer's OK before any
  build; `plan.md` is that plan.
- `KEYS.md` of the earlier handoff predates #1834; the registry wins.
