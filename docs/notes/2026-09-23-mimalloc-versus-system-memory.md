# An allocator swap saves tens of MiB here, not a gigabyte

On 2026-09-23, two release builds of Postio at `76219a28` differed only in
the binary's global allocator: the existing `mimalloc::MiMalloc`, or Rust's
system allocator. A temporary probe launched each build on the same headless
compositor with a separate copy of one encrypted, current-schema synthetic
store. It opened the normal GTK window, then ran an empty-result query and a
broad query that found 416 messages and displayed 200. The synthetic account
had no stored password. The probe and build switch were removed after the
measurement; neither allocator nor production behavior changed.

`/proc/<pid>/smaps_rollup` supplied proportional set size (PSS) for the app
and all of its descendants, including WebKit. PSS counts shared pages by
their share, so summing the process tree does not multiply shared libraries.
Runs were sequential, with the order reversed for the second pair. Values
below are MiB after both searches had settled. Search times are the
executor's logged `elapsed_ms`, not end-to-end GTK paint time:

| Run | Allocator | Postio PSS | Process-tree PSS | Empty / broad search |
| --- | --- | ---: | ---: | ---: |
| 1 | System | 140.5 | 246.0 | 3 / 8 ms |
| 1 | mimalloc | 171.9 | 282.1 | 2 / 9 ms |
| 2 | mimalloc | 178.3 | 289.0 | 4 / 9 ms |
| 2 | System | 143.7 | 253.7 | 315 / 20 ms |

At five seconds after the UI was wired in run 2, before searching, the app
was at 161.7 MiB with mimalloc and 137.9 MiB with the system allocator; the
whole process tree was at 267.1 and 243.1 MiB. After searching, the system
allocator saved about 33 MiB in Postio itself and 36 MiB across the tree,
averaged over the two pairs. WebKit's share was broadly similar in both.

These timings do **not** establish a speed tie. Other cargo builds shared
the machine, and the second system run's first search took 315 ms despite a
3 ms result in its first run. The workload is also small: the interrupted
synthetic seeder left a roughly 4.5 MiB database, not the large real mailbox
whose memory use prompted the experiment. The available 1.3 GiB real store
was stamped for an older Postio schema, which this build correctly refused
to open; no real store was changed or resynced for this test.

The allocator is a measurable but secondary memory cost in this workload.
This evidence does not justify trading away an allocator chosen for store and
index allocation churn when search latency on a large compatible store is
still unknown. Before changing the production allocator, repeat the pair on
a current-schema mailbox near the observed size, on a quiet machine, and
measure both PSS and repeated interaction latency.
