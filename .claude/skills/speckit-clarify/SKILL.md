---
name: "speckit-clarify"
description: "Identify underspecified areas in the current feature spec by asking up to 5 highly targeted clarification questions and encoding answers back into the spec."
argument-hint: "Optional areas to clarify in the spec"
compatibility: "Requires spec-kit project structure with .specify/ directory"
metadata:
  author: "github-spec-kit"
  source: "templates/commands/clarify.md"
user-invocable: true
disable-model-invocation: false
---


## User Input

```text
$ARGUMENTS
```

You **MUST** consider the user input before proceeding (if not empty).

## Pre-Execution Checks

Extension hooks (`before_clarify`): read `.specify/extensions.yml` if it exists; on a missing file, no `hooks.before_clarify` entries or unparseable YAML, skip silently.
- Drop hooks with `enabled: false` (absent means enabled). Never evaluate `condition`: no condition or empty means executable; a non-empty one means skip the hook (the HookExecutor owns it).
- Command invocation: replace `.` with `-` (`speckit.git.commit` becomes `/speckit-git-commit`).
- Optional hook (`optional: true`): print `## Extension Hooks`, `**Optional Pre-Hook**: {extension}`, `Command: /{command}`, `Description: {description}`, `Prompt: {prompt}`, `To execute: /{command}`.
- Mandatory hook (`optional: false`): print `## Extension Hooks`, `**Automatic Pre-Hook**: {extension}`, `Executing: /{command}`, `EXECUTE_COMMAND: {command}`, then actually invoke it the way this session runs commands (e.g. `/skill:speckit-...` or `$speckit-...`) and wait for it to finish before continuing; the block alone runs nothing.

## Outline

Goal: detect and reduce ambiguity in the active feature spec and record the clarifications in the spec file. Run this BEFORE `/speckit-plan`; if the user skips it (e.g. an exploratory spike), warn that downstream rework risk rises.

1. Run `.specify/scripts/bash/check-prerequisites.sh --json --paths-only` from repo root **once** (`-Json -PathsOnly` on PowerShell). Parse `FEATURE_DIR` and `FEATURE_SPEC` (optionally `IMPL_PLAN`, `TASKS`). On a JSON parse failure, abort and tell the user to re-run `/speckit-specify` or check the feature branch. Escape single quotes in args (`'I'\''m Groot'`) or double-quote.

2. Load `.specify/memory/constitution.md` if it exists.

3. Load the spec and scan it against this taxonomy, marking each category Clear / Partial / Missing as an internal coverage map (print it only if no questions will be asked):
   - Functional Scope & Behavior: core goals and success criteria; out-of-scope declarations; roles and personas
   - Domain & Data Model: entities, attributes, relationships; identity and uniqueness; lifecycle and state transitions; volume and scale
   - Interaction & UX Flow: critical journeys; error, empty and loading states; accessibility and localization
   - Non-Functional Quality: performance; scalability; reliability and availability; observability; security and privacy; compliance
   - Integration & External Dependencies: services and APIs with failure modes; import/export formats; protocol and versioning
   - Edge Cases & Failure Handling: negative scenarios; rate limiting; conflict resolution
   - Constraints & Tradeoffs: technical constraints; explicit tradeoffs and rejected alternatives
   - Terminology & Consistency: canonical glossary; avoided synonyms
   - Completion Signals: testable acceptance criteria; measurable Definition of Done
   - Misc / Placeholders: TODO markers; unquantified adjectives ("robust", "intuitive")

   Each Partial or Missing category yields a candidate question unless the answer would not materially change implementation or validation, or is better deferred to planning (note it internally).

4. Build an internal prioritised queue of at most 5 questions (top 5 by Impact * Uncertainty). Never show it. Rules:
   - Answerable by a short multiple-choice (2-5 distinct, mutually exclusive options) or a short phrase ("Answer in <=5 words").
   - Only questions whose answers materially affect architecture, data modeling, task decomposition, test design, UX behavior, operational readiness or compliance; favour reducing rework and misaligned acceptance tests; cover the highest-impact unresolved category before low-impact ones.
   - Exclude answered questions, trivial style preferences and plan-level execution details (unless blocking correctness).

5. **Question loop**: EXACTLY ONE question at a time.
   - Format: `**Question:** <full interrogative ending in ?>` or `**Question:** <interrogative>? (FR-023)`; the only permitted suffix after the `?` is a parenthesised requirement id. Never use a topic label, section heading or id as the question (`Acceptance device/runtime matrix (FR-023)` is invalid).
   - Follow with one plain-language "Why it matters" sentence. Use everyday wording and define any jargon; a reader who does not know Spec Kit must be able to answer from the Question line alone.
   - Multiple choice: pick the best option (best practice, common patterns, risk reduction, fit with the spec's goals), then give `**Recommended:** Option [X] - <1-2 sentence reasoning>`, then the table:

     | Option | Description |
     |--------|-------------|
     | A | <Option A description> |
     | B | <Option B description> |
     | C | <Option C description> (add D/E as needed up to 5) |
     | Short | Provide a different short answer (<=5 words) (Include only if free-form alternative is appropriate) |

     then: `You can reply with the option letter (e.g., "A"), accept the recommendation by saying "yes" or "recommended", or provide your own short answer.`
   - Short answer: `**Suggested:** <answer> - <brief reasoning>`, then `Format: Short answer (<=5 words). You can accept the suggestion by saying "yes" or "suggested", or provide your own answer.`
   - After the answer: "yes", "recommended" or "suggested" adopts your stated recommendation; otherwise check it maps to an option or fits <=5 words. If ambiguous, disambiguate without advancing the count. Then hold it in memory and ask the next question.
   - Stop when critical ambiguities are resolved, the user signals completion ("done", "good", "no more"), or 5 questions are asked. Never reveal future questions. If none are valid at the start, report no critical ambiguities.

6. **Integrate after EACH accepted answer**:
   - First answer of the session: ensure a `## Clarifications` section exists (create it after the template's top contextual/overview section) with a `### Session YYYY-MM-DD` subheading for today.
   - Append `- Q: <question> → A: <final answer>`.
   - Apply it to the right section: functional ambiguity to Functional Requirements; actor or interaction to User Stories or Actors; data shape to Data Model (fields, types, relationships, constraints); non-functional to Success Criteria > Measurable Outcomes (vague adjective becomes metric); edge case to Edge Cases / Error Handling; terminology conflict normalised across the spec, with `(formerly referred to as "X")` once if needed.
   - Replace any earlier statement the answer invalidates; leave no contradiction.
   - Save the spec after each integration (atomic overwrite). Do not reorder unrelated sections or change heading hierarchy. Keep each insertion minimal and testable.

7. **Validate** after each write and in a final pass:
   - exactly one bullet per accepted answer, no duplicates; at most 5 questions
   - no lingering placeholder the answer should have resolved; no contradictory earlier statement
   - valid Markdown; the only new headings are `## Clarifications` and `### Session YYYY-MM-DD`
   - one canonical term across updated sections

8. Write the updated spec to `FEATURE_SPEC`.

9. **Re-validate the Spec Quality Checklist.** Skip silently if `FEATURE_DIR/checklists/requirements.md` does not exist. Otherwise:
   1. Read it; take every task-list line (`- [ ]`, `- [x]`, `- [X]`, leading whitespace tolerated, outside code fences) into a before-snapshot of state and text. Ignore everything else.
   2. Re-evaluate each item against the updated spec. Toggle only items whose state changed (passes now and was `[ ]`: `[x]`; fails now and was checked: `[ ]`); leave unchanged markers and their case alone.
   3. Save, changing only the marker of changed lines (headings, metadata, notes, order and whitespace untouched).
   4. From the snapshot compute **Newly passing**, **Regressions** and **Still unchecked**, and before/after pass counts (e.g. "12/16 → 15/16 items passing").

Behavior rules:
- No meaningful ambiguity: reply "No critical ambiguities detected worth formal clarification." and suggest proceeding; with full coverage, give a compact summary (all categories Clear).
- Spec missing: tell the user to run `/speckit-specify` first; never create one here.
- Never exceed 5 questions (retries of one question do not count). Avoid speculative tech-stack questions unless their absence blocks functional clarity. Honour early termination ("stop", "done", "proceed").
- If the quota is reached with high-impact categories unresolved, flag them as Deferred with rationale.

Context for prioritization: $ARGUMENTS

## Mandatory Post-Execution Hooks

**Complete this section before reporting completion.** Same rules as the Pre-Execution Checks, with `hooks.after_clarify`; skip to the Completion Report when there are none.
- Mandatory hook: emit `## Extension Hooks`, `**Automatic Hook**: {extension}`, `Executing: /{command}`, `EXECUTE_COMMAND: {command}`, then actually invoke it and wait for it to finish.
- Optional hook: emit `## Extension Hooks`, `**Optional Hook**: {extension}`, `Command: /{command}`, `Description: {description}`, `Prompt: {prompt}`, `To execute: /{command}`.

## Completion Report

After the loop ends or terminates early, report:
- questions asked and answered; path to the updated spec; sections touched
- checklist status (if re-validated): before/after pass counts, newly checked items, regressions, and any still-unchecked items as needing attention
- a coverage table, each taxonomy category Resolved (was Partial/Missing, now addressed), Deferred (over quota or better for planning), Clear, or Outstanding (Partial/Missing but low impact)
- if any Outstanding or Deferred remain, whether to proceed to `/speckit-plan` or run `/speckit-clarify` again after planning
- the suggested next command

## Done When

- [ ] Ambiguities identified and clarifications integrated into the spec
- [ ] Spec quality checklist re-validated (if `FEATURE_DIR/checklists/requirements.md` exists)
- [ ] Extension hooks dispatched or skipped per the Mandatory Post-Execution Hooks
- [ ] Completion reported with questions answered, sections touched, checklist status and coverage summary
