# ADR 0049: Each test layer asserts its own question

- **Status:** Accepted (2026-10-23). The rule binds new tests now. The ratchet that enforces it on frontend suites lands with spec 012's first parity batch, one surface at a time.
- **Related:**
  - [specs/012-test-suite-value](../../specs/012-test-suite-value/spec.md), which has the rubric, the tooling and the cleanup;
  - Constitution IV (assert on what a person would see);
  - [ADR 0044](0044-every-frontend-is-observable-and-storyboarded.md) and [ADR 0045](0045-focus-behaviour-lives-in-postio-focus.md).
- **Decision:**
  - **A behaviour's branches are asserted once, at the cheapest layer that can see them:** a rule crate, or the `postio-focus` controller.
  - **A frontend suite asserts composition.** This means a real key reaches the shared logic and a person sees the result:
    - one case per path a person can take through a surface on that platform;
    - plus the registry and binding parity sweeps, once per platform.
  - **A test that reads the repository** belongs in `scripts/checks/`, unless it needs compiled types.
  - **No test is removed for being in the wrong layer** without mutation evidence that nothing it caught goes uncaught.

---

## Context

The suite reached about 8,000 cases and 127 CPU-minutes. A 2026-10-23 sample found the same behaviour asserted at every layer. For example, "a number chooses in the snooze picker" is asserted in:
- the controller suite;
- `ffi_suite`;
- `focus_suite`;
- `widgets_suite`.

Each new feature added a case per layer, because nothing said which layer owned which question.

Two lessons pull in opposite directions, and this ADR keeps both:
- **The defects that reach users live between layers** (Constitution IV). So the frontends cannot be left untested on the strength of the controller's tests. A Linux-only gate let 31 Mac bindings die at the macOS end of a shared layer.
- **A branch asserted four times is not four times as safe.** It is four times as slow, four places to update, and three binaries that need a display to say what a rule crate says in a millisecond.

## Decision

| Layer | Asserts | Does not assert |
|---|---|---|
| Rule crates (`postio-core`, `postio-ui`, `postio-body`, `postio-model`, storage, sync, …) | Every case of a rule: each state, key and edge | — |
| Controller (`postio-focus`) | Every branch of what a surface does with a command | Rules a rule crate already owns |
| Frontend (`postio-ffi`, `postio-gtk`, `postio-widgets`, `postio-tui`) | Composition: a real key or call reaches the shared logic and the result is what a person sees. One case per path per platform. The registry and binding parity sweeps, once per platform | Branches the controller already covers |
| Meta (reads source, docs or manifests) | Facts only compiled types can check, such as generated docs being current | Anything a `scripts/checks/*.py` guard enforces |

1. **A new behaviour gets its branches in the lowest layer that can see it.** A frontend case is added only for a path no existing frontend case drives.
2. **"Path" means a route a person takes.** Opening a surface, choosing in it and leaving it are paths. The fifth value of a choice is a branch.
3. **Moving or removing a test because of this rule** needs a "safe" verdict from `scripts/test-removal-check.sh`: no mutant survives without the test that was caught with it.
4. **A frontend suite file that has been cleaned** gets a budget in `scripts/checks/frontend-suite-budget.txt`. To grow it, raise the budget in the same commit, with the reason in the body.

## Consequences

- A feature lands with most of its cases in the controller or rule crates, which are fast. Each frontend gets a few composition cases, which are slow and few.
- The Mac and GTK keep their own composition and parity tests. The lesson of the 31 bindings stands.
- Reviewers have a question to ask of any frontend case: which path does this drive that no other case does?
- The cleanup of existing duplication is spec 012's work, and it is gated by mutation evidence. This ADR does not license deleting anything on sight.
