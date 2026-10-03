# ADR 0044 — Every frontend is observable and storyboarded

- **Status:** Accepted (2026-10-02), with `specs/008-storyboards`
- **Spec:** [`specs/008-storyboards`](../../specs/008-storyboards/spec.md)
  (FR-010, FR-015, FR-031; the spec carries the reasoning, this records the
  rule)
- **Related:** ADR 0043 on `feature/postio-focus` (where the GTK half both
  desktop apps share lives),
  [ADR 0019](0019-macos-frontend.md) (the frontend the rule will reach next)
- **Decision:** **Every Postio frontend reports where everything is as one
  shared `Observation`, has a runner that plays the shared storyboard
  format, and ships a change to its interaction with the storyboard that
  describes it.**

---

## The rule

1. **Observable.** A frontend implements `observe()` returning
   `postio_ui::observe::Observation`, filled as
   `specs/008-storyboards/contracts/observation.md` says: from what is on
   screen (the widget that really holds the keyboard, the cursor's row, the
   notice that is up), never from what a layer was told; reading only, with
   no store access and no side effect; and a field the frontend cannot fill
   is declared unobserved, never guessed.
2. **Storyboarded.** A frontend has a runner that reads the storyboard files
   in `storyboards/` -- the format in `storyboards/README.md` -- presses
   commands by its own bindings, films each step, and writes `run.json` in
   the shared shape, through `postio-storyboard`, so no two runners can read a
   storyboard differently.
3. **Shipped together.** A change to how a frontend behaves under the
   keyboard ships with the storyboard that describes the new behaviour,
   written from the acceptance before the code, and is reviewed with
   `/ux-review` before it reaches the maintainer. A defect fixed in one
   frontend gets a storyboard; one that applies to every frontend is shared,
   with an override where an app legitimately differs.

## Why a rule and not a habit

The defects that reached the maintainer were sequence defects that every
layer's tests passed (the spec's Context table), and the first day of
storyboards found nine more (#1744 to #1752). They live between layers, and only a
picture of the sequence plus a record of where things are can show them.
A frontend that cannot be observed or played is one where that class of
defect is invisible again, so it is a boundary, not a nicety.

## Consequences

- The desktop app has one runner (`postio-focus`, over `postio-widgets`'
  GTK half), and storyboards play on it and nothing else since the classic
  app's removal was approved (ADR 0043; specs/007-postio-focus T265). The terminal and macOS apps owe one each before their next
  interaction work (FR-031). The terminal's is the cheapest: its update is already pure.
- `postio-storyboard` is a development crate the shipped frontends never
  depend on; `Observation` lives in `postio-ui`, which they all do.
