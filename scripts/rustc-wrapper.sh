#!/usr/bin/env bash
# Cargo's RUSTC_WRAPPER, wired up in .cargo/config.toml: use sccache when the
# machine has it, get out of the way when it does not.
#
# Worktrees stopped sharing a target directory (#178), so without a wrapper
# every worktree recompiles ~400 third-party crates from scratch unless the
# session remembered `export RUSTC_WRAPPER=sccache` -- and sessions forget.
# Wiring it through cargo config makes the machine-wide cache the default in
# every worktree, and a box without sccache still builds (fail open, like the
# headless runner). `mise use -g sccache` installs it.
#
# An explicit RUSTC_WRAPPER in the environment still wins over this file --
# cargo gives the environment precedence over config.
#
# # Why TMPDIR is set here and not left to cargo (#359)
#
# sccache is ONE daemon shared by every session on the machine, and it takes
# its temp directory from the `TMPDIR` of whichever client invocation happened
# to spawn it. It never re-reads it afterwards: a later client's TMPDIR is
# ignored outright, and every compile the daemon runs -- rustc, and the linker
# under it -- uses the daemon's copy, not the caller's. Both halves of that
# were measured on this box, not inferred.
#
# `.cargo/config.toml` sets `TMPDIR = target/tmp` relative to the workspace
# root, which for a worktree is that worktree. So the first cargo invocation
# after a daemon restart donates its own `target/tmp` to every session, and:
#
#   * `scripts/issue-release.sh` deletes that worktree when its issue lands,
#     and from then on EVERY build on the machine fails with "Failed to create
#     temp dir ... No such file or directory" naming a path that has nothing
#     wrong with it and belongs to nobody's current work (#359);
#   * the tmpfs protection that TMPDIR setting exists to give is accidental --
#     it holds only while the donated directory is a worktree's on-disk
#     `target/tmp`. Spawn the daemon from a plain shell and it takes the real
#     /tmp, a 6 GB tmpfs here, and linking the GTK stack fills it.
#
# Pinning it beside sccache's own cache fixes both: that directory outlives
# every worktree, sits on disk rather than the tmpfs, and is the same whichever
# invocation wins the race to start the daemon.
#
# Only on the sccache path. Without sccache there is no daemon, rustc runs
# straight from cargo, and cargo's per-worktree TMPDIR is what keeps its
# scratch off the tmpfs -- so that case is left exactly as it was.
if command -v sccache >/dev/null 2>&1; then
    scratch="${SCCACHE_DIR:-${HOME:-}/.cache/sccache}/tmp"
    # Fail open, like everything else in this file: a temp directory that
    # cannot be created costs the pinning, never the build.
    if mkdir -p "$scratch" 2>/dev/null; then
        export TMPDIR="$scratch"
    fi
    exec sccache "$@"
fi
exec "$@"
