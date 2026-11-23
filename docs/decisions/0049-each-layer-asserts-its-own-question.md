# ADR 0049: Each test layer asserts its own question

- **Status:** Amended 2026-11-22 (accepted 2026-10-23)
- **Decision:** A behaviour's branches are asserted once, at the cheapest layer that can see them (a rule crate or the `postio-focus` controller). Each frontend asserts composition: one case per surface a person reaches, plus one key-binding table check.

## Rules
- Rule crates (`postio-core`, `postio-ui`, `postio-body`, `postio-model`, storage, sync) assert every case of a rule: each state, key and edge.
- The controller (`postio-focus`) asserts every branch of what a surface does with a command, but not rules a rule crate owns.
- Each frontend has one case per surface a person reaches: the list, reader, composer, each picker, search, settings, contacts and threads. Each also has one key-binding table check that every binding the frontend offers reaches the controller.
  - GTK: `postio-gtk`'s `focus_suite` and `postio-widgets`' `widgets_suite`.
  - Mac: `postio-ffi`'s `ffi_suite` plus the Swift tests.
  - Terminal: `postio-tui`.
- Tests of frontend-only behaviour stay while the controller cannot see it: geometry, focus handling, input methods, rendering and teardown.
- A frontend does not re-assert branches the controller covers. A new behaviour gets its branches in the lowest layer that can see it. A frontend case is added only for a surface no existing case reaches.
- A test that reads the repository belongs in `scripts/checks/` unless it needs compiled types. Meta tests are only for facts compiled types alone can check (generated docs current).
- Moving or removing a test under this rule needs a "safe" verdict from `scripts/test-removal-check.sh`: no mutant survives without it.
- A cleaned frontend suite file gets a budget in `scripts/checks/frontend-suite-budget.txt`. Raising it needs the reason in the commit body.
- Spec 012 holds the rubric, tooling and cleanup. This rule does not license deleting tests on sight.

## Why
- About 8,000 cases and 127 CPU-minutes asserted the same behaviour at every layer (for example "a number chooses in the snooze picker" four times). A branch asserted four times is four times slower with four places to update.
- Defects live between layers (Constitution IV). A Linux-only gate once let 31 Mac bindings die at the macOS end of a shared layer, so each frontend keeps its surface and binding checks.

## Revisit when
- A defect class passes every layer under this split.
