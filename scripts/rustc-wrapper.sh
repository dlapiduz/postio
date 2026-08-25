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
if command -v sccache >/dev/null 2>&1; then
    exec sccache "$@"
fi
exec "$@"
