# Seeding a worktree's target/debug from a sibling's (#1102), shared by `issue-claim.sh` and
# `worktree-seed.sh` (#1717).
# Reads REPO_ROOT and WORKTREES, and COLD if set; sets SEEDED, SEED_CANDIDATE and SEED_FAILURE.
# Needs lib/drop-workspace-artifacts.sh sourced first.
# Sourced, not executed: every caller shares one `set -euo pipefail`.

# Copy a directory, sharing blocks where the filesystem can: `cp -a --reflink=auto` is GNU and BSD
# `cp` rejects it (#1208); APFS clones with `cp -c`; plain `cp -R` last.
# Decided once on a scratch directory: a candidate that cannot be copied must fail once so
# `seed_target` falls through to an older sibling (#1190).
CP_CLONE_FLAGS=""
detect_clone_flags() {
    local probe
    probe="$(mktemp -d)" || { CP_CLONE_FLAGS="-R"; return; }
    mkdir -p "$probe/src"
    if cp -a --reflink=auto "$probe/src" "$probe/gnu" 2>/dev/null; then
        CP_CLONE_FLAGS="-a --reflink=auto"
    elif cp -Rc "$probe/src" "$probe/bsd" 2>/dev/null; then
        CP_CLONE_FLAGS="-Rc"
    else
        CP_CLONE_FLAGS="-R"
    fi
    rm -rf "$probe"
}

clone_tree() { # <src> <dst>
    [ -n "$CP_CLONE_FLAGS" ] || detect_clone_flags
    # Unquoted on purpose: the flags are this script's own, and one of them is
    # two words.
    # shellcheck disable=SC2086
    cp $CP_CLONE_FLAGS "$1" "$2"
}

# Seed a fresh worktree's target/debug from the newest sibling's (#1102): a cold target/ costs 11 to
# 19 minutes and a copy-on-write seed builds the sanity tier in 64 s.
# A copy, so safe against #76 (two trees writing one target); it works only because
# `.cargo/config.toml` names the linker and cc rather than pathing them (#1101).
# A `cp` without `--reflink` (macOS) fails, the partial copy is removed and the tree starts cold.
# The sibling's `target/tmp` is never copied.
# Newest first by the mtime of `target/debug/deps`, which also marks the likeliest to be mid-write
# (#1191). `--cold` and `POSTIO_CLAIM_SEED=0` opt out.
SEEDED=""
# Set as soon as a candidate is found, whatever `cp` does, to tell a failed seed from "nothing to
# try" (#1190).
SEED_CANDIDATE=""
# The first candidate's failure, "<path>: <cp's stderr>", kept: the newest candidate is the
# likeliest to be mid-write (#1191).
SEED_FAILURE=""
seed_target() { # <tree>
    [ "${COLD:-0}" = 0 ] || return 0
    [ "${POSTIO_CLAIM_SEED:-1}" != 0 ] || return 0
    local candidate deps_dirs="" deps src started error
    # Every sibling, not only `issue-*` trees: a spec branch or lane is as good a seed (#1717).
    for candidate in "$WORKTREES"/*/target/debug "$REPO_ROOT/target/debug"; do
        [ -d "$candidate/deps" ] || continue
        [ "$candidate" != "$1/target/debug" ] || continue
        deps_dirs="$deps_dirs $candidate/deps"
    done
    [ -n "$deps_dirs" ] || return 0
    # Try every candidate, newest first, rather than only the newest: a
    # sibling with an active build in it (the newest is the likeliest to)
    # fails a `cp -a` reading it mid-write, and giving up there paid a cold
    # build for a problem the next-newest candidate did not have (#1191).
    # shellcheck disable=SC2086 -- worktree paths carry no spaces, by construction
    for deps in $(ls -td $deps_dirs 2>/dev/null); do
        src="${deps%/deps}"
        [ -n "$src" ] || continue
        SEED_CANDIDATE="$src"
        mkdir -p "$1/target"
        started=$(date +%s)
        if error="$(clone_tree "$src" "$1/target/debug" 2>&1)"; then
            drop_workspace_artifacts "$1/target"
            SEEDED="$src ($(( $(date +%s) - started ))s)"
            return 0
        fi
        rm -rf "$1/target/debug"
        [ -n "$SEED_FAILURE" ] || SEED_FAILURE="$src: $error"
    done
}
