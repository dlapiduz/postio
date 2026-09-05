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
# Cargo's own TMPDIR is `target/tmp` (relative), and nothing creates it, so the
# first temp file in a fresh clone or worktree fails with `NotFound` (#613).
# Ensured here, before the branch below replaces TMPDIR with the daemon's, so
# what gets created is cargo's directory rather than sccache's -- and on every
# platform, including the ones with no `runner` to do it at launch.
#
# Fail open, like everything else in this file.
if [ -n "${TMPDIR:-}" ]; then
    mkdir -p "$TMPDIR" 2>/dev/null || true
fi

# # Sizing, and why the default is wrong here (2026-09-03)
#
# sccache's default cache is 10 GiB, nobody had ever changed it, and on this
# box it was **full**: 10 GiB of 10 GiB, 11G on disk. A full cache is a cache
# in permanent eviction -- every new compile throws out somebody else's entry,
# and with nine worktrees each holding ~2.1 GB of dependency artifacts, the
# sessions were evicting each other continuously. That reads from inside a
# session as "the compile cache died and it fell back to compiling locally".
#
# 30G rather than a bigger number because the disk is shared with the
# worktrees themselves: /home had 88 GB free with nine of them present, and
# a target directory is the larger appetite of the two.
#
# `SCCACHE_IDLE_TIMEOUT=0` keeps the server up. The default stops it after ten
# idle minutes, which does not lose the on-disk cache but does mean several
# sessions a day pay a cold start and re-read config.
#
# `SCCACHE_ERROR_LOG` because when this does go wrong there is currently no
# evidence at all -- the failure above was diagnosed from `--show-stats`
# after the fact, and only because someone happened to mention it.
#
# **These are read when the server STARTS, not per invocation.** A running
# server keeps whatever it was born with, exactly like the TMPDIR hazard
# below (#359). After changing them: `sccache --stop-server`, and the next
# compile starts a server that honours them. `sccache --show-stats` prints
# "Max cache size" and is how you check which one you have.
#
# Set with `:-` like everything else here, so an explicit value in the
# environment still wins.
if command -v sccache >/dev/null 2>&1; then
    export SCCACHE_CACHE_SIZE="${SCCACHE_CACHE_SIZE:-30G}"
    export SCCACHE_IDLE_TIMEOUT="${SCCACHE_IDLE_TIMEOUT:-0}"
    export SCCACHE_ERROR_LOG="${SCCACHE_ERROR_LOG:-${SCCACHE_DIR:-${HOME:-}/.cache/sccache}/sccache.log}"
    scratch="${SCCACHE_DIR:-${HOME:-}/.cache/sccache}/tmp"
    # Fail open, like everything else in this file: a temp directory that
    # cannot be created costs the pinning, never the build.
    if mkdir -p "$scratch" 2>/dev/null; then
        export TMPDIR="$scratch"
    fi
    exec sccache "$@"
fi
exec "$@"
