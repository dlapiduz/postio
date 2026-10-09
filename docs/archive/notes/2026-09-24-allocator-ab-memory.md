# Allocator A/B: small-store savings do not persist after a large search

On 2026-09-23–24, two release builds of each tested Postio revision differed
only in their global allocator: the existing `mimalloc::MiMalloc` or Rust's
system allocator. A temporary probe launched each build on a headless GTK
compositor, suppressed sync, opened the normal window, then ran an
empty-result query and a broad query. The probe and allocator switch were
removed afterward. Production behavior did not change.

`/proc/<pid>/smaps_rollup` supplied proportional set size (PSS) for Postio
and its process tree, including WebKit. PSS divides shared pages among their
users. Runs were sequential; the second pair reversed allocator order.
Search times below are the executor's logged `elapsed_ms`, **not** GTK paint
or end-to-end interaction time. The shared workstation was not isolated from
other builds.

## Small synthetic store on the current revision

At `76219a28`, both builds opened separate copies of the same encrypted,
current-schema synthetic store. Its database was only about 4.5 MiB because
the seeder had been interrupted. The broad query found 416 messages and
displayed 200. The account had no stored password. Values below were taken
after both searches had settled:

| Run | Allocator | Postio PSS | Tree PSS | Empty / broad search |
| --- | --- | ---: | ---: | ---: |
| 1 | System | 140.5 MiB | 246.0 MiB | 3 / 8 ms |
| 1 | mimalloc | 171.9 MiB | 282.1 MiB | 2 / 9 ms |
| 2 | mimalloc | 178.3 MiB | 289.0 MiB | 4 / 9 ms |
| 2 | System | 143.7 MiB | 253.7 MiB | 315 / 20 ms |

At five seconds after UI readiness in run 2, before searching, Postio used
161.7 MiB with mimalloc and 137.9 MiB with the system allocator; tree PSS
was 267.1 and 243.1 MiB. After searching, system saved about 33 MiB in
Postio and 36 MiB across the tree, averaged over the two pairs. The timing
spread, especially the second system run's 315 ms empty query, precludes a
speed conclusion.

## Larger mailbox on a matching historical revision

The current build refused the available older-schema store, so this pair
used historical revision `ddcbe223`, which matches it. Each allocator build
opened its own reflinked copy of the same approximately 918 MiB encrypted
scratch store. No original store was modified, and sync was disabled. The
broad query returned 200 messages with its total capped at 10,000. Idle PSS
was sampled five seconds after UI readiness; the last PSS sample followed
both searches.

| Run | Allocator | Idle Postio PSS | Idle tree PSS | After Postio PSS | After tree PSS | Empty / broad search |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | mimalloc | 288.8 MiB | 370.2 MiB | 367.2 MiB | 448.8 MiB | 698 / 7012 ms |
| 1 | System | 198.3 MiB | 280.2 MiB | 359.2 MiB | 440.5 MiB | 1963 / 4642 ms |
| 2 | System | 259.3 MiB | 341.4 MiB | 355.2 MiB | 436.9 MiB | 897 / 6499 ms |
| 2 | mimalloc | 172.4 MiB | 225.0 MiB | 340.9 MiB | 422.9 MiB | 596 / 4902 ms |

Idle PSS varied by more than 100 MiB with launch order and cache state. Once
the broad search completed, tree PSS converged to 423–449 MiB: system was
8 MiB lower in the first pair, while mimalloc was 14 MiB lower in the second.
The difference changed direction, so this larger workload does not support a
stable memory saving from an allocator swap. Nor does its noisy timing show
that one allocator searches faster. The broad query did take 4.6–7.0 seconds
on this **historical** build, far beyond the local-search budget; that is an
observed search-path problem to investigate separately, not evidence about
the current revision's latency.

## Decision and limit

Keep mimalloc for now. The small store shows a real tens-of-MiB saving with
system allocation, but it does not explain a roughly 1 GiB process; the
larger store shows no repeatable post-search saving. These runs did not
capture startup/indexing peaks or a sync cycle, so they cannot attribute the
reported 1 GiB peak. Any renewed allocator decision needs a current-schema
large mailbox, repeated interaction timing, and peak-memory measurements on
a quiet machine.
