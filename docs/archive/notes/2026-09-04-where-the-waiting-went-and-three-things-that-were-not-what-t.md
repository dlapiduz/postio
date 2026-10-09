# Where the waiting went, and three things that were not what they seemed (2026-09-04, #1101/#1102/#1104)

Every session's transcript was mined for how long each shell command took
(23,894 commands over 79 sessions, ~110 hours of shell wall time), and the
per-issue cycle was reconstructed from claim to merged PR. The numbers,
because they are what the next tuning should be measured against:

| where the time went | hours | share |
|---|---|---|
| `cargo test` | 33.3 | 30% |
| `cargo build` / `check` | 21.5 | 20% |
| polling a backgrounded `issue-land.sh` | 16.7 | 15% |
| `issue-land.sh` in the foreground | 10.4 | 9% |

Per issue (95 matched): claim → PR open p50 47 min; PR open → merged p50
10 min, p75 16, p90 41. Fresh claims: 393 of 396. `test-fast.sh`: 6 runs,
ever, against 852 whole-crate and 427 `--workspace` test runs used as the
inner loop. The polling is tool calls rather than wall time, but 925 of
them; the notification the land script's completion sends is enough.

### sccache was a per-worktree cache, and the linker path was why (#1101)

Hit rate this boot: 2 hits, 178 misses. `.cargo/config.toml` said
`linker = "scripts/linker.sh"`, which cargo resolves against the config's
directory, so every rustc invocation carried
`-C linker=/home/.../postio-worktrees/issue-N/scripts/linker.sh` — a
different string in each tree. sccache hashes it. Isolated with a one-crate
experiment: same flags in another directory hits, a different linker path
misses. `CC = scripts/cc-wrapper.sh` had the same shape for build scripts,
which the `cc` crate reruns on any change to CC's value — and both sit in
cargo's own fingerprints, which is why a `target/` copied from a sibling
rebuilt everything too.

**The rule: nothing rustc or a build script sees may contain a worktree
path.** Linker and CC are bare names now (`postio-linker`, `postio-cc`),
installed into `$CARGO_HOME/bin` by `scripts/install-shims.sh`, which the
claim, land and test scripts and every CI workflow run first. A bare name
in `linker` is passed through verbatim (verified: `-C linker=postio-linker`)
and a name is the same string everywhere. `rustc-wrapper` is exempt: it is
the wrapper, not an argument.

It recovers a third, not all: a second cold worktree against a populated
cache still missed 644 of 948 (#1106 has the suspects). Seeding is what
made that stop mattering.

### A copied target/ is 12 seconds; a shared one was #76 (#1102)

Three throwaway worktrees, `cargo test --workspace --lib --no-run`, on the
shared box with two other sessions compiling:

| | | wall | compiled |
|---|---|---|---|
| A | cold, path-free linker (populates the cache) | 1149 s | 389 crates |
| B | cold, against A's cache | 928 s | 389 crates |
| C | `cp -a --reflink=always A/target/debug` (1 s, 3.3 GB) | **12 s** | **3 crates** |

So `issue-claim.sh` seeds a fresh tree from the newest sibling's
`target/debug` (or the shared checkout's), `--reflink=auto`, and a plain
claim run inside a landed worktree reuses it instead.

**C's 12 s was too good, and the first reused landing said so.** Our own
crates bake the tree's absolute path in (`env!("CARGO_MANIFEST_DIR")`,
fourteen files), and cargo does not rebuild on a directory move — verified
with a two-line crate — so a moved or copied target runs binaries that
point at the old tree; `postio-session`'s crate-list test failed exactly
that way. Both paths now drop Postio's own artifacts
(`scripts/lib/drop-workspace-artifacts.sh`, the same rule CI's cache uses)
and keep the dependencies. Measured on the shared box: **64 s** to rebuild
the 20 workspace crates for the sanity tier. That is the honest number for
a reused or seeded claim, against 1149 s cold. Constraints that are
load-bearing:

- **Copy, never share.** #76 was two trees writing one target. Each tree
  here owns its copy; cargo's fingerprints are self-consistent inside it.
- **`target/tmp` is never copied** — it is a sibling's live test scratch.
- The seed's own linker/CC must already be names, or the fingerprints will
  not match and the copy is dead weight. Yesterday's copy of an 11 GB
  sibling rebuilt all 71 of postio-core's deps for exactly that reason.
- A fresh checkout's sources are newer than the seed's artifacts, so
  Postio's own crates rebuild; the third-party ~470 do not. The 3-crate
  number above is the best case (same commit, older checkout).

### `jobs = 2` idled six cores most of the day; a jobserver does not (#1104)

While anything was building, ONE session was building 60% of the time and
two 26%. Only 3 cargo commands in the whole history raised `-j`. Cargo
1.98 honors `MAKEFLAGS=--jobserver-auth=fifo:<path>` and ignores `jobs`
while it has one (measured: a six-crate build 13.3 s → 4.7 s under `-j2`
with seven tokens); with the fifo missing it warns and falls back.
`scripts/jobserver.sh` owns the pool. Things about it that are not obvious:

- **The fifo path is fixed** (`/tmp/postio-jobserver/fifo`) because
  `.claude/settings.json` `env` cannot expand variables and it is what
  hands MAKEFLAGS to every session. `/tmp` is tmpfs here, so a reboot
  empties it; `ensure` recreates it and the PreToolUse hook runs `ensure`
  before any command mentioning cargo.
- **A holder process keeps the fifo open.** A fifo drops its buffer when
  the last fd closes, which between two cargo runs is always.
- **Tokens leak** when a cargo is killed mid-build (79 tool timeouts in
  the transcripts), so `ensure` resets the pool to N — but only when
  `pgrep` finds no cargo/rustc/rustdoc/clippy-driver/cargo-nextest, since
  a token that is out is somebody's live job. `POSTIO_JOBSERVER_IDLE=1|0`
  overrides that for the self-test, whose neighbours really are compiling.
- N is `nproc - 2` (six here). Each cargo also holds one implicit token,
  so four sessions can reach ten jobs; memory was the thing four sessions
  actually exhausted, and `--threads=4` on the linker still applies.
- `-j` is now wrong in both states: ignored with the pool up, a cap on
  the fallback without it.

### Left open

#1107 (`needs-maintainer`): PR open → merged is ten minutes a session spends
in `wait-for-checks.sh`; auto-merge behind a required-checks ruleset would
remove it, but changes who watches a red PR. Also there: the Tests job is
3 min restoring a 7 GB cache, 8 min of nextest, 3 min saving it again.
