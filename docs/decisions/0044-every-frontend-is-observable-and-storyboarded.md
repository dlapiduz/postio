# ADR 0044 — Every frontend is observable and storyboarded

- **Status:** Accepted (2026-10-02), with `specs/008-storyboards`
- **Decision:** Every frontend reports where everything is as one shared `Observation`, has a runner that plays the shared storyboard format, and ships a change to its interaction with the storyboard that describes it.

## Rules
- Observable: `observe()` returns `postio_ui::observe::Observation`, filled per `specs/008-storyboards/contracts/observation.md`. Fill it from what is on screen (the widget holding the keyboard, the cursor's row, the notice up), never from what a layer was told. Reading only, no store access or side effect. A field the frontend cannot fill is declared unobserved, never guessed.
- Storyboarded: the runner reads `storyboards/` (format in `storyboards/README.md`), presses commands by the frontend's own bindings, films each step and writes `run.json` through `postio-storyboard`, so no two runners read a storyboard differently.
- A change to keyboard behaviour ships with its storyboard, written from the acceptance before the code, and is reviewed with `/ux-review`. A defect fixed in one frontend gets a storyboard. One that applies to every frontend is shared, with an override where an app legitimately differs.
- The desktop app has one runner (`postio-gtk` over `postio-widgets`). The terminal and macOS apps owe one before their next interaction work (FR-031). The terminal's is cheapest because its update is pure.
- `postio-storyboard` is a development crate shipped frontends never depend on. `Observation` lives in `postio-ui`.

## Why
- Defects that reached the maintainer were sequence defects every layer's tests passed (#1744 to #1752). Only a picture of the sequence plus a record of where things are shows them.

## Revisit when
- A frontend legitimately cannot be observed.
