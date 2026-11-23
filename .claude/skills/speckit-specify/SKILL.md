---
name: "speckit-specify"
description: "Create or update the feature specification from a natural language feature description."
argument-hint: "Describe the feature you want to specify"
compatibility: "Requires spec-kit project structure with .specify/ directory"
metadata:
  author: "github-spec-kit"
  source: "templates/commands/specify.md"
user-invocable: true
disable-model-invocation: false
---


## User Input

```text
$ARGUMENTS
```

You **MUST** consider the user input before proceeding (if not empty).

## Pre-Execution Checks

Extension hooks (`before_specify`): read `.specify/extensions.yml` if it exists; on a missing file, no `hooks.before_specify` entries or unparseable YAML, skip silently.
- Drop hooks with `enabled: false` (absent means enabled). Never evaluate `condition`: no condition or empty means executable; a non-empty one means skip the hook (the HookExecutor owns it).
- Command invocation: replace `.` with `-` (`speckit.git.commit` becomes `/speckit-git-commit`).
- Optional hook (`optional: true`): print `## Extension Hooks`, `**Optional Pre-Hook**: {extension}`, `Command: /{command}`, `Description: {description}`, `Prompt: {prompt}`, `To execute: /{command}`.
- Mandatory hook (`optional: false`): print `## Extension Hooks`, `**Automatic Pre-Hook**: {extension}`, `Executing: /{command}`, `EXECUTE_COMMAND: {command}`, then actually invoke it the way this session runs commands (e.g. `/skill:speckit-...` or `$speckit-...`) and wait for it to finish before continuing; the block alone runs nothing.

## Outline

The text after `/speckit-specify` **is** the feature description (never ask to repeat it unless empty: then ERROR "No feature description provided").

1. **Short name** (2-4 words, action-noun, keep acronyms like OAuth2): "add user authentication" becomes `user-auth`; "fix payment processing timeout bug" becomes `fix-payment-timeout`.

2. **Branch** (optional, via hook): a `before_specify` hook that ran created or switched the branch and printed JSON with `BRANCH_NAME` and `FEATURE_NUM`. Note them; the branch name does not dictate the spec directory name. Pass a user-given `GIT_BRANCH_NAME` through to the hook.

3. **Spec directory.** Resolve `SPECIFY_FEATURE_DIRECTORY`: a user-provided value (environment, argument, configuration) is used as-is; otherwise `specs/<prefix>-<short-name>` where:
   - `.specify/init-options.json` `feature_numbering` (or deprecated `branch_numbering`; if only that is present, warn "`branch_numbering` in init-options.json is deprecated. Rename to `feature_numbering`.") picks the prefix;
   - `"timestamp"` gives `YYYYMMDD-HHMMSS`; `"sequential"` or absent gives the next free 3-digit `NNN` after scanning `specs/` (e.g. `003-user-auth`).

   Then:
   - `mkdir -p SPECIFY_FEATURE_DIRECTORY`; resolve the active `spec-template` through the preset/template stack (`specify preset resolve spec-template`) and copy it to `SPECIFY_FEATURE_DIRECTORY/spec.md` (`SPEC_FILE`).
   - Write `.specify/feature.json` as `{"feature_directory": "<the resolved path, e.g. specs/003-user-auth>"}` (the real path, not the variable name) so `/speckit-plan`, `/speckit-tasks` and the rest find it without git branch conventions.
   - One feature per invocation. Directory and branch names are independent. This command, never the hook, creates the directory and file.

4. Load the resolved `spec-template` for the required sections, and `.specify/memory/constitution.md` if it exists.

5. **Fill the spec**:
   1. Extract actors, actions, data, constraints. No clear user flow: ERROR "Cannot determine user scenarios".
   2. Guess from context and industry standards; mark `[NEEDS CLARIFICATION: specific question]` only when the choice significantly affects scope or UX, has multiple reasonable readings, and has no reasonable default. **At most 3 markers**, prioritised scope > security/privacy > UX > technical details.
   3. Fill User Scenarios & Testing, testable Functional Requirements (record defaults in Assumptions), measurable technology-agnostic Success Criteria (quantitative and qualitative, verifiable without implementation details), and Key Entities if data is involved.
   4. Write to `SPEC_FILE` with the template's section order and headings, placeholders replaced.

6. **Quality validation.**

   a. Create `SPECIFY_FEATURE_DIRECTORY/checklists/requirements.md` titled "Specification Quality Checklist: [FEATURE NAME]" (**Purpose**: Validate specification completeness and quality before proceeding to planning; **Created**; **Feature**: link to spec.md) with these checkbox items under three headings, and a Notes line "Items marked incomplete require spec updates before `/speckit-clarify` or `/speckit-plan`":
   - Content Quality: no implementation details (languages, frameworks, APIs); focused on user value and business needs; written for non-technical stakeholders; all mandatory sections completed
   - Requirement Completeness: no [NEEDS CLARIFICATION] markers remain; requirements testable and unambiguous; success criteria measurable; success criteria technology-agnostic; all acceptance scenarios defined; edge cases identified; scope clearly bounded; dependencies and assumptions identified
   - Feature Readiness: all functional requirements have clear acceptance criteria; user scenarios cover primary flows; feature meets the measurable outcomes in Success Criteria; no implementation details leak into the specification

   b. Check the spec against each item, quoting the sections at fault.

   c. Results:
   - All pass: mark the checklist complete and go to the post-execution hooks.
   - Failures other than markers: fix the spec and re-validate, max 3 iterations; then record the remainder in the checklist notes and warn the user.
   - Markers remain: keep only the 3 most critical (guess the rest), then present all questions together, numbered Q1-Q3, each as:

     ```markdown
     ## Question [N]: [Topic]

     **Context**: [Quote relevant spec section]

     **What we need to know**: [Specific question from NEEDS CLARIFICATION marker]

     **Suggested Answers**:

     | Option | Answer | Implications |
     |--------|--------|--------------|
     | A      | [First suggested answer] | [What this means for the feature] |
     | B      | [Second suggested answer] | [What this means for the feature] |
     | C      | [Third suggested answer] | [What this means for the feature] |
     | Custom | Provide your own answer | [Explain how to provide custom input] |

     **Your choice**: _[Wait for user response]_
     ```

     Keep tables well-formed (spaces around cell content, header separator of at least 3 dashes). Wait for all answers (e.g. "Q1: A, Q2: Custom - [details], Q3: B"), replace each marker with the answer, and re-validate.

   d. Update the checklist file after each iteration.

## Mandatory Post-Execution Hooks

**Complete this section before reporting completion.** Same rules as the Pre-Execution Checks, with `hooks.after_specify`; skip to the Completion Report when there are none.
- Mandatory hook: emit `## Extension Hooks`, `**Automatic Hook**: {extension}`, `Executing: /{command}`, `EXECUTE_COMMAND: {command}`, then actually invoke it and wait for it to finish.
- Optional hook: emit `## Extension Hooks`, `**Optional Hook**: {extension}`, `Command: /{command}`, `Description: {description}`, `Prompt: {prompt}`, `To execute: /{command}`.

## Completion Report

Report completion to the user with:
- `SPECIFY_FEATURE_DIRECTORY` — the feature directory path
- `SPEC_FILE` — the spec file path
- Checklist results summary
- Readiness for the next phase (`/speckit-clarify` or `/speckit-plan`)

**NOTE:** Branch creation is handled by the `before_specify` hook (git extension). Spec directory and file creation are always handled by this core command.

## Quick Guidelines

- Focus on **WHAT** and **WHY**, for business stakeholders; no HOW (tech stack, APIs, code structure).
- Do not embed checklists in the spec.
- Mandatory sections are filled for every feature; optional ones only when relevant; a section that does not apply is removed, not marked "N/A".
- Think like a tester: a vague requirement fails "testable and unambiguous".
- Ask only where no reasonable default exists: feature scope and boundaries, user types and permissions (when readings conflict), legally or financially significant security/compliance. Do not ask about data retention (industry standard), performance targets (standard expectations), error handling (friendly messages with fallbacks), authentication (session or OAuth2 for web), or integration patterns (those fitting the project).

### Success Criteria

Measurable (time, percentage, count, rate), technology-agnostic, user-focused, verifiable without implementation knowledge.
- Good: "Users can complete checkout in under 3 minutes"; "95% of searches return results in under 1 second"; "Task completion rate improves by 40%".
- Bad: "API response time is under 200ms" (say "Users see results instantly"); "Database can handle 1000 TPS"; "React components render efficiently"; "Redis cache hit rate above 80%".

## Done When

- [ ] Specification written to `SPEC_FILE` and validated against the quality checklist
- [ ] Extension hooks dispatched or skipped per the Mandatory Post-Execution Hooks
- [ ] Completion reported to the user with feature directory, spec file path and checklist results
