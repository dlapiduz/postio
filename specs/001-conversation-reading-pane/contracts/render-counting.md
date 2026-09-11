# Contract: counting the cost of moving

**What it governs**: how FR-057 to FR-065 are asserted. Principle V — budgets
are gated as counts, not timings, because the causes of a budget are the same
number on any machine and a millisecond is not defensible on a shared runner.

Modelled on `postio_storage::test_support::counting`, which reads statements,
rows and trigger firings off SQLite's trace hook and exposes `counted(|| …)`.
That one cannot see this feature's defect: a duplicate document load issues no
extra queries.

## What is counted

| Count | Asserts |
|---|---|
| Renders issued | FR-058 — one gesture, at most one render; zero when re-selecting what is displayed |
| Rendering surfaces created | FR-057 — zero additional per message or per conversation |
| Bytes handed to the renderer, per document | FR-059 — no per-message bulk; the ~1.2 MB of inlined fonts #749 found must not return |
| Store queries per conversation open | FR-030, SC-002 |
| Surfaces held after leaving | FR-063 |

## Where it lives

On the seam a frontend calls, in `postio-ui` — not inside `postio-gtk` — so
that both frontends are held to the same numbers and the assertions run without
a display, in milliseconds.

## Required assertions

1. Walking the fixture corpus creates zero additional surfaces (SC-009a).
2. Holding navigation through 200 conversations renders no more times than the
   number settled on (SC-009b).
3. Reading 50 conversations leaves the same resources held as after one
   (SC-009c).
4. A body arriving for the displayed conversation costs one render and does not
   move the scroll position (SC-009d, FR-062).
5. Arriving at an absent body and having it arrive costs one render, not two
   (FR-064) — the ordinary case under backfill-first sync.

**A wall-clock assertion is not an acceptable substitute for any of these.**
