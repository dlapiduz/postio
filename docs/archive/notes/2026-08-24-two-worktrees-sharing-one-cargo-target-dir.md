# Two worktrees sharing one `CARGO_TARGET_DIR` can hand you another worktree's library

*Archived 2026-09-14: resolved by #178 (every worktree builds into its own `target/`) and superseded by #1102 (a fresh worktree's `target/debug` is a reflink copy of the newest sibling's); the race it describes cannot happen in a tree claimed since then.*

Observed 2026-08-24 while working issue #33. This
session had added `crates/postio-core/src/invocation.rs` and a new `Event`
variant; `cargo test -p postio-core` was green, `cargo build --workspace` was
green, and `cargo test -p postio-app` then failed to compile with *"could not
find `invocation` in `postio_core`"* — against source that plainly contained
it. `cargo build -p postio-core` immediately beforehand did not help.

The link line named the culprit: `postio-app` was compiled with
`--extern postio_core=.../libpostio_core-d8f157….rlib`, while the core built
seconds earlier was `libpostio_core-00a841….rlib`. The stale one's dep-info
(`target/debug/deps/postio_core-d8f157….d`) lists its sources **relative to
the workspace root** — `crates/postio-core/src/lib.rs` — and did not mention
`invocation.rs` at all. Only this worktree had that file, so that unit was
built from a *different* worktree of the same workspace and cargo considered
it fresh for this one. Both worktrees present cargo with the same relative
paths and the same package name and version, so they land in the same build
slot and overwrite each other.

Consequences, in rough order of how badly they bite:

- **A green suite can be a lie in either direction.** Your crate can be tested
  against somebody else's version of its dependency. The compile error above
  is the lucky case, because it is loud; the silent case is a test that passes
  against a library your change never reached.
- It is a *race*, so it is intermittent. Re-running often "fixes" it, which is
  the worst possible property — it trains you to re-run instead of to look.
- `scripts/issue-land.sh` used to share the target directory by default
  (`export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$MAIN_CHECKOUT/target}"`), so
  the landing gates — the one run a merge is staked on — were exposed to it
  too. #253 removed that default: the script now builds in the calling
  worktree's own `target/` and only honours `CARGO_TARGET_DIR` when a caller
  has genuinely set one. `scripts/tests/test-issue-land-target-dir.py` holds it
  there, by building a real crate through `--gates-only` and looking at where
  the artifacts landed.

What to do, until #76 settles it properly:

- Within a **single** `cargo` invocation you are safe — cargo holds the build
  lock for the whole run. So verify across crates in one command
  (`cargo test -p postio-core -p postio-app`) rather than one per crate.
- When a result surprises you, **check the link line before you re-run**:
  `cargo test -p <crate> -v 2>&1 | grep -o "extern <dep>=[^ ]*"`, then
  `head -1 target/debug/deps/<dep>-<hash>.d` and look at whether the source
  list matches the tree you are actually in.
- For a result you are going to stake a merge on, build into a target
  directory of your own: `CARGO_TARGET_DIR=$PWD/target-verify cargo test …`.
  It costs one full build of the third-party crates and nothing after that.

CLAUDE.md and the `/issue` skill used to recommend
`export CARGO_TARGET_DIR=~/src/postio/target` to keep the GTK and WebKit
builds warm. That advice was right about the cost it was avoiding — it just
was not free, and the above is the bill. #178 settled it the other way:
every worktree gets its own `target/`, and the ~400 third-party crates stay
warm through `export RUSTC_WRAPPER=sccache` instead, which keys on exact
compiler inputs and so cannot produce this confusion at all.

The lesson generalises past cargo: when a fix removes a shared resource,
grep for the places that *default* to it, not just the places that name it.
#178 changed both instruction documents and left `issue-land.sh`'s default
in place for #253 to find — and a default is worse than an instruction,
because nobody has to read it for it to fire.
