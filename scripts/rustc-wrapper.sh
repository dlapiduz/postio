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
# **`SCCACHE_LOG` is the other half of that, and without it the first half
# does nothing** (#1184). `SCCACHE_ERROR_LOG` says *where* log records go;
# `SCCACHE_LOG` decides whether there are any. The daemon wedged twice with
# the error log set the whole time and the file empty both times, which read
# as "the instrument is broken" and was really "the instrument was never
# switched on". Measured here with an isolated daemon on its own port and
# cache directory:
#
#   SCCACHE_ERROR_LOG only                the file is created, 0 bytes
#   SCCACHE_ERROR_LOG + SCCACHE_LOG=info  348 bytes of server lifecycle
#
# What made it look like it worked: a *client* that cannot start a second
# server writes "Address in use" there regardless of level, so the live log
# had 45 bytes in it and nothing from the server that mattered.
#
# `info` rather than `debug`: four lines per server start and **nothing per
# compile** -- six compiles produced the same 348 bytes as one -- so it costs
# nothing and records what a wedge investigation wants, which is when the
# daemon started and how it was configured. `debug` is for a session that is
# actively chasing one, and the `:-` below lets it say so.
#
# **These are read when the server STARTS, not per invocation.** A running
# server keeps whatever it was born with, exactly like the TMPDIR hazard
# below (#359). After changing them: `sccache --stop-server`, and the next
# compile starts a server that honours them. `sccache --show-stats` prints
# "Max cache size" and is how you check which one you have.
#
# Set with `:-` like everything else here, so an explicit value in the
# environment still wins.
# # Why a --remap-path-prefix goes in here (#1106)
#
# With the linker and CC path-free (#1101), a second cold worktree still
# missed two thirds of its compiles, and the residual had one cause: a build
# script that *generates Rust source the crate `include!`s* leaves the
# generated file's absolute path inside the compiled artifact.
#
#   $ strings target/debug/deps/libserde_core-*.rmeta | grep out/private.rs
#   /home/.../issue-1141/target/debug/build/serde_core-<hash>/out/private.rs
#
# That path names the worktree, so `libserde_core.rmeta` differs byte for byte
# between two trees at the same commit -- and sccache hashes `--extern` inputs
# by *content*, so every crate downstream of serde misses too. In this
# workspace that is nearly all of them. Measured on a two-tree harness,
# serde_core and serde were the only artifacts embedding a path and everything
# else that differed was downstream of them.
#
# Remapping the prefix makes those two artifacts identical, which stops the
# cascade. What it cannot do is make the two crates themselves hit: the flag
# necessarily *contains* the per-worktree path, and sccache does not normalise
# `--remap-path-prefix` out of its key -- measured, by passing one
# unconditionally through RUSTFLAGS, which took a second tree from 10 cache
# hits to zero. Nor can it be injected below sccache: a rustc shim named by
# `build.rustc` sees the `-vV` probes and the build scripts, and never the
# cacheable compiles, which sccache runs against the real rustc.
#
# So it is conditional, and the condition is the one that decides whether a
# path can reach the artifact at all: did this crate's build script generate
# Rust source. A build script that only prints `cargo:rustc-cfg`
# (proc-macro2, num_traits) leaves no path behind, already hits across
# worktrees, and must not be handed a flag that would cost that.
#
# The replacement is a constant, so it is the same in every tree; it shows up
# in panic messages and backtraces for those crates instead of a path that
# names somebody's worktree, which is no worse and arguably clearer.
#
# `scripts/tests/test-rustc-wrapper-remap.py` is what holds all of this.
if [ -n "${OUT_DIR:-}" ]; then
    # `compgen -G` rather than a glob test, so an OUT_DIR that does not exist
    # or holds no Rust source simply answers no. Fail open, like the rest of
    # this file: a shell without `compgen` costs the remapping, never the
    # build.
    if compgen -G "${OUT_DIR}/*.rs" >/dev/null 2>&1; then
        set -- "$@" --remap-path-prefix="${OUT_DIR}=/postio-out"
    fi
fi

if command -v sccache >/dev/null 2>&1; then
    export SCCACHE_CACHE_SIZE="${SCCACHE_CACHE_SIZE:-30G}"
    export SCCACHE_IDLE_TIMEOUT="${SCCACHE_IDLE_TIMEOUT:-0}"
    export SCCACHE_ERROR_LOG="${SCCACHE_ERROR_LOG:-${SCCACHE_DIR:-${HOME:-}/.cache/sccache}/sccache.log}"
    export SCCACHE_LOG="${SCCACHE_LOG:-info}"
    scratch="${SCCACHE_DIR:-${HOME:-}/.cache/sccache}/tmp"
    # Fail open, like everything else in this file: a temp directory that
    # cannot be created costs the pinning, never the build.
    if mkdir -p "$scratch" 2>/dev/null; then
        export TMPDIR="$scratch"
    fi
    exec sccache "$@"
fi
exec "$@"
