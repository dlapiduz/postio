# Two compile caches, because neither can do the other's job (2026-08-28, #736)

The workspace has *two* machine-wide compile caches, and the split is forced,
not stylistic:

- **sccache caches Rust**, wired in as `build.rustc-wrapper`
  (`scripts/rustc-wrapper.sh`). A rustc wrapper never sees anything but
  rustc: the C compiler that the `openssl-src`, `libsqlite3-sys` and
  `zstd-sys` build scripts invoke through make runs outside it entirely.
  After ADR 0014 put the vendored OpenSSL + SQLCipher into the graph, 77% of
  a fresh worktree's `postio-storage` build (255 of 330 unit-seconds) was C
  that no cache ever saw — which is what was pushing `issue-land.sh` past a
  foreground tool call's ten-minute cap, and every killed run leaks its live
  tests' `/dev/shm/postio-test-*` scratch (171 at once observed).
  *Correction (#1101):* sccache was not carrying the Rust half across
  worktrees either, because `-C linker=<per-worktree path>` was in every
  invocation's hash — 1.1% hits, measured. The linker and `CC` are bare
  names now; the ccache measurement below stands.
- **ccache caches that C**, wired in as `[env] CC = postio-cc`
  (`scripts/cc-wrapper.sh`, installed on PATH by `scripts/install-shims.sh`).
  Routing the C through sccache instead does not work, and this was measured
  before being believed: `openssl-src` extracts and compiles its sources
  *inside each target directory*, so every include path and `#line`
  directive carries that directory's absolute path, and sccache has no path
  normalization for C. Two fresh-target builds through `sccache cc` shared
  2 of 1148 compiles (0.17%) and the second build was *slower* than no cache
  at all. ccache's `base_dir` + `hash_dir = false` exist for exactly this
  and hit on 1193 of 1196 (the wrapper defaults `CCACHE_BASEDIR=$HOME`,
  `CCACHE_NOHASHDIR=1`, and mtime/ctime sloppiness because the sources are
  re-extracted fresh per target dir).

Constraints this leaves behind: do not "simplify" the two wrappers into one
cache, and do not strip the `CCACHE_*` defaults out of `cc-wrapper.sh` — any
of those regressions is invisible until someone times a fresh worktree.
`ccache -s` is how you check the C half is alive; `sccache --show-stats` only
ever shows the Rust half. And the standing instruction that came out of the
same incident: **run `issue-land.sh` in the background, always** (CLAUDE.md);
the ten-minute cap belongs to the harness, not the script, and a run killed
mid-gates commits nothing. Since #742 a killed run's retry is also cheap:
the script records `git write-tree` plus the crate list after the gates
pass, and a retry on a byte-identical tree skips clippy and the per-crate
tests (loudly), re-running only the invariants. The `[timing]` lines in its
output are where "landing is slow" conversations should start.
