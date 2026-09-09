# Where 475 gigabytes went (2026-09-09, #1428)

`/home` reached 100% -- 499 MB free of 475 GB -- and a landing died in
the middle of its gate:

    error: failed to write query cache to
      .../target/debug/incremental/postio_app-.../query-cache.bin:
      No space left on device (os error 28)
    error: could not compile `postio-app` (lib test)
    issue-land exit 101

## The failure lies about itself

`--status` reports a compile error. The branch compiles perfectly; the
`os error 28` is several lines above, in a log that mostly scrolls past.
The obvious reading -- "my branch does not build" -- sends you to the
wrong question, and it is machine-wide, so every parallel session fails
at once on whatever it happened to be doing.

If a gate fails on a crate you did not touch **and** the error is about
writing a file rather than about types, check `df` before anything else.

## What was actually holding it

Four piles, all cargo output:

| | Size |
|---|---|
| `~/src/postio/target` (the **main checkout**) | 200 GB |
| 47 worktrees, one `target/` each | ~150 GB real |
| `~/scratch/issue53-private-target` | 128 GB |
| `~/.cache/sccache` | 29 GB |

**Cargo never prunes `deps/`.** The main checkout held 439 binaries over
100 MB across 128 distinct targets -- about a dozen stale copies of
each, twenty of `postio` itself. Every dependency change emits a
freshly-hashed binary and leaves its predecessor behind forever. At
~400 MB per GTK+WebKit debug binary that is most of the 194 GB in
`target/debug`.

Two of those piles are anomalies rather than growth:

- **The main checkout should not be a build tree at all.** CLAUDE.md
  says `~/src/postio` is for coordination, not work. Something has been
  running `cargo` there routinely.
- **`~/scratch/issue53-private-target` was 128 GB** on its own, last
  touched 2026-08-24.

## Reading `du` on this filesystem

`/home` is btrfs and `issue-claim.sh` seeds a new worktree's
`target/debug` by **reflink** from the newest sibling (#1102). `du`
counts a shared extent once per file that references it, so it reported
**1.4 TB under `~/src`** on a 475 GB disk. Worktree sizes from `du` are
therefore upper bounds, not costs; `~/src/postio/target` and the scratch
dirs are not reflinked and their numbers were real.

Measured instead by removing five worktrees and watching `df`: about
5 GB each, not the 11 GB `du` implied.

## Reclaiming a worktree safely

The test that works is `git cherry` against the base, which compares by
**patch-id**:

    git cherry origin/feature/conversation-reading-pane "$branch" | grep -c '^+'

Comparing shas does not work. `issue-land.sh` rebases on every attempt,
so a branch that merged an hour ago has entirely different shas from
what is upstream and `git merge-base --is-ancestor` calls it unmerged.
Thirty of the forty-seven worktrees were clean *and* had zero unmerged
commits by patch-id -- pure cost.

Never reclaim a tree that is dirty, or one with commits not upstream:
those are somebody's work in progress, and a worktree is where a session
that ended badly left it.

## Why they accumulate at all

`issue-release.sh` removes a tree when a session explicitly stops. The
ordinary loop never stops -- it lands, claims the next issue, and either
reuses the tree it is in or seeds a fresh one. A session that ends any
other way (context exhausted, killed, or simply finished for the day)
leaves its tree behind permanently. Nothing collects them.

#1428 carries both halves: something should reclaim them, and a landing
that fails on `No space left on device` should say so in those words.
