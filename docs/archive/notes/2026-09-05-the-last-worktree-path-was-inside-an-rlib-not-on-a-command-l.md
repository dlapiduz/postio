# The last worktree path was inside an rlib, not on a command line (2026-09-05, #1106)

#1101 got the per-worktree paths out of rustc's *arguments* — the linker and
`CC` are bare names now. A second cold worktree still missed most of its
compiles, and the residual was in a place no argument diff could show: inside
a compiled artifact.

A build script that generates Rust source the crate `include!`s leaves the
generated file's absolute path in the artifact.

```
$ strings target/debug/deps/libserde_core-*.rmeta | grep out/private.rs
/home/.../issue-1141/target/debug/build/serde_core-<hash>/out/private.rs
```

sccache hashes `--extern` inputs by **content**, so `libserde_core.rmeta`
differing byte for byte between two trees makes every crate downstream of
serde miss as well — which here is nearly all of them. Measured on two trees
at one commit, `serde_core` and `serde` were the only artifacts embedding a
path, and everything else that differed was downstream of them.

**Having a build script is not the predictor.** `num_traits` and
`proc-macro2` both run one and both hit; what matters is whether the build
script generates *source that gets included*. That is a much smaller set, and
it is the set `scripts/rustc-wrapper.sh` now hands a
`--remap-path-prefix=$OUT_DIR=/postio-out`.

## Two things that do not work, both measured

Worth writing down because both are the obvious next move.

**sccache does not normalise `--remap-path-prefix` out of its key.** Passing
one unconditionally through `RUSTFLAGS` — the tidy spelling — took a second
tree from 10 cache hits to **zero**: the flag necessarily contains the
per-worktree path, so it becomes exactly the kind of argument #1101 removed.
That is why the wrapper adds it only to the compiles that already miss across
trees. It also means those two crates go on missing; what the remap buys is
that their *outputs* now match, which is what stops the cascade.

**A rustc shim below sccache never sees the compiles that matter.** The
appealing trick is to inject the flag under sccache, so sccache hashes the
clean argument list and rustc still gets the remap. It does not work: with
`build.rustc` pointed at a shim, the shim was handed the `-vV` probes, the
build-script compilations and the proc-macro builds — and not one cacheable
library compile. sccache runs those against the real rustc.

## What it is worth

`cargo build -p postio-storage --lib`, second tree, private cache, same
commit:

| | hits | misses | wall clock |
|---|---|---|---|
| before | 63 | 10 | 212 s |
| after | 66 | 7 | 87 s |

`libserde_core`, `libserde` and `libchrono` are byte-identical across the two
trees afterwards, and no worktree path remains in any of them.

The measurement harness is two `git worktree add --detach` trees at one
commit with a private `SCCACHE_DIR` and `SCCACHE_SERVER_PORT` — never the
shared daemon, which other sessions are building against — then `cmp` the
`.rmeta` files and read `sccache --show-stats` either side of the second
build.
