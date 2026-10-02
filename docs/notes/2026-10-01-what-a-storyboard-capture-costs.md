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

## Calibration (T061)

The first calibration of the reviewer, 2026-10-02: template blake3
`1c2f8a26…`, a fresh Opus reviewer given only the generated prompt, over the
twelve storyboards in `storyboards/calibration/` (34 steps).

| | Result |
|---|---|
| `must_fail` storyboards failed | 6 of 6 |
| `must_pass` storyboards passed | 5 of 6 |
| Verdicts that cite a frame | 34 of 34 (`verdicts check` clean) |

**The one miss was the app, not the reviewer.** In `cal-archive-offers-undo`
the frame really showed no archive and no notice: pressing `a` as the first
key after the window opened moved the cursor and archived nothing, while the
identical step in `cal-archive-says-nothing`, in another process, archived and
said so. The reviewer described the frame correctly and noticed the two runs
disagreed. Filed as #1745; the two archive calibration storyboards now move
the cursor once first, so the reviewer's ground truth does not race.

**What it found beyond the expectations**, which is the part a machine check
cannot do (findings, not verdicts):

- `a` (Archive) on a conversation row said "Archived 8 messages" while the
  cheat sheet lists a separate `A` (Archive thread): the two verbs read as
  one.
- A search's header said "10 results" while its scope said "All mail 11".
- The search bar said "still syncing" while the status line one step
  earlier said "idle · imap".
- Opened from search, the reader's actions are labelled text buttons; opened
  from the inbox, they are icons.
- The cheat sheet runs off the bottom of the window with nothing to say there
  is more, and the keyboard stays on the list behind it.
- The composer footer's last hint is cut off ("Escape keeps the…"), and
  Schedule… is live with no recipient.

## The budget, measured (T101)

SC-002 asks for one storyboard in under 15 s warm and the whole GTK catalogue
in under 5 minutes. Measured 2026-10-02 on the workstation, frames on, each
invocation on its own headless compositor:

| What | Time |
|---|---|
| One storyboard (`list/archive-walks-down`), warm, build check included | 7.5 s |
| The whole Classic catalogue, 78 storyboards (42 interactions, 36 screens) | 3 min 32 s |
| The same with frames off, as the app suite's catalogue case | 3 min 9 s |
| The screen sweep alone (36 screens, in their variants) | 53 s |
| The generated pass, every command in every context (119 presses) | 3 min 16 s |

Frames off saves less than expected because "no frames" still settles every
step: it means nothing is written, not less waiting, after a live search's
debounce made the two modes disagree. Focus plays most of the catalogue too,
so the both-apps figure is measured on the Focus lane; if it passes five
minutes, the stride (2) is the dial, not the settle rules.
