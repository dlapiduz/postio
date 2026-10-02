# What a storyboard capture costs, and which renderer repeats itself

*2026-10-01, specs/008-storyboards T005.*

A storyboard runner samples frames after every step to tell a settled
screen from one that jumps or blanks (spec 008 research R4), and compares
frames byte for byte between runs and between a branch and its base (R5).
Both rest on two numbers nobody had measured: what one capture costs, and
whether the same window captures to the same bytes twice.

## Measured

A throwaway example seeded `seed_small_with_bodies` through `feed_the_window`
(the same path `shot` takes), settled a 1280×800 Classic window with
animations off, and timed `capture::texture_within` plus a download of the
pixels. It ran on the private headless compositor
(`scripts/test-headless.sh`), twice per renderer, each run its own process.

| Renderer | Run | Median | p95 | Same bytes within the run | Hash of the settled frame |
|---|---|---|---|---|---|
| default | 1 | 25.9 ms | 37.7 ms | yes (100 of 100) | `7961091f…` |
| default | 2 | 53.4 ms | 86.4 ms | yes (100 of 100) | `f4a15325…` |
| `GSK_RENDERER=cairo` | 1 | 23.0 ms | 35.2 ms | yes (100 of 100) | `e27d5db7…` |
| `GSK_RENDERER=cairo` | 2 | 24.2 ms | 37.7 ms | yes (100 of 100) | `e27d5db7…` |

## What follows

- **Runs pin `GSK_RENDERER=cairo`.** The default renderer repeats itself
  inside one process but not across two, which is exactly the comparison a
  base-versus-branch diff makes. Cairo gave the same bytes in both. The
  runner records the renderer in every `run.json`, so the choice is never
  silent. What it costs: cairo is not what people see on a GL desktop, so a
  defect that lives only in the GL path will not show in a storyboard. The
  capture is for interaction and layout, not for the GL renderer.
- **Settle sampling takes every second tick.** One capture costs more than a
  16.7 ms frame, so sampling every tick only queues work behind itself. At a
  stride of two, the default K of 6 identical samples settles in about
  200 ms, and the 300 ms watch window takes about nine more samples. A
  six-step storyboard spends roughly four seconds capturing, within SC-002's
  15 s for one storyboard. Whether the whole catalogue fits 5 minutes is
  measured again at T101, with the real catalogue.
- **The second default run was twice as slow** while three other builds
  shared the machine. Timings here are a floor under contention, not a
  promise.
