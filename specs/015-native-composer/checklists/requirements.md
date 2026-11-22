# Specification Quality Checklist: The native composer, over the document

**Purpose**: validate the spec before the maintainer reads it
**Created**: 2026-11-22
**Feature**: [spec.md](../spec.md)

## Content Quality

- [x] No implementation detail the maintainer did not already choose in ADR 0039 (the signature deviation is D1, stated)
- [x] Focused on what a person and a contributor get
- [x] All mandatory sections completed

## Requirement Completeness

- [x] No `[NEEDS CLARIFICATION]` markers; open decisions are Q1 to Q3, each with options and a recommendation
- [x] Requirements are testable (each FR names a case or a count)
- [x] Success criteria are measurable
- [x] Edge cases identified (preedit, quoted blocks, selections across blocks, prefixes)
- [x] Scope bounded: Mac timing (Q1), spell-check (Q3), reply quote (Q2)
- [x] Dependencies stated: spec 014 Part A, a Mac session, `libspelling`

## Feature Readiness

- [x] Every ADR 0039 step has a phase, and steps 1 and 2 are independently landable
- [x] The spike gate is a task that blocks Phase 4 (T017)
- [x] Spec 014's T054 to T069 are mapped (`contracts/spec-014-unblock.md`)
- [x] Performance, privacy, accessibility and keyboard each have a requirement and a test

## Notes

- ADR 0039 says compose on the Mac is deferred; `ComposeEditor.swift` shows it has shipped on `WKWebView`. Q1 and D7 follow from that.
- The ADR's `apply(&Document, EditIntent) -> Document` gains a selection (D1).
