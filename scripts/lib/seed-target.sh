# Seeding a worktree's target/debug from a sibling's (#1102), shared by
# `issue-claim.sh` (a claimed tree) and `worktree-seed.sh` (any other tree:
# a spec branch, a parallel lane -- #1717).
#
# Reads REPO_ROOT and WORKTREES, and COLD if the caller sets it; sets
# SEEDED, SEED_CANDIDATE and SEED_FAILURE for the caller to report. Needs
# lib/drop-workspace-artifacts.sh sourced first.
#
# Sourced, not executed: every caller shares one `set -euo pipefail`.

# Copy a directory, sharing blocks where the filesystem can.
#
# `cp -a --reflink=auto` is GNU, and BSD `cp` rejects it outright -- so on
# macOS every seed failed its usage check and fell back to a cold build, which
# is the 11-to-19 minutes this whole feature exists to avoid (#1208). The
# fallback hid it: the claim still worked, just slowly, and said so in a line
# that reads as an unlucky filesystem rather than as a flag that never had a
# chance.
#
# APFS clones with `cp -c`, which is the same copy-on-write bargain. Plain
# `cp -R` last, because a slow seed still beats a cold build.
# Decided once, on a scratch directory, rather than by retrying per copy: a
# candidate that genuinely cannot be copied has to fail *once* so `seed_target`
# falls through to an older sibling (#1190). Trying three flag sets against the
# real source would turn one doomed copy into three and skip the fallthrough.
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

# Seed a fresh worktree's target/debug from the newest sibling's (#1102).
#
# A cold target/ is 11 to 19 minutes before the first gate can say anything,
# and 393 of 396 claims paid it. Measured on this box: `cp -a --reflink` of
# an 11 GB target/debug took one second on btrfs, and after dropping the
# sibling's own crates (lib/drop-workspace-artifacts.sh says why) the seeded
# tree built the whole sanity tier in 64 s rebuilding Postio's 20 crates,
# against 1149 s and 389 crates cold. Copy-on-write, so it costs no disk
# until files diverge.
#
# It is a *copy*, which is what makes it safe against #76: that was two
# trees writing one target and handing each other stale libraries. Each
# tree here owns its own; cargo's fingerprints are self-consistent inside
# it, and anything the seed built for a different source simply rebuilds.
# Only works because `.cargo/config.toml` names the linker and cc rather
# than pathing them (#1101): with a per-worktree path in every fingerprint,
# the copy rebuilt everything.
#
# `--reflink=auto`, so a filesystem without reflinks gets a plain copy
# (slower, still faster than a cold build); a `cp` without the flag at all
# (macOS) fails, the partial copy is removed, and the tree starts cold,
# which is what it did before. The sibling's `target/tmp` is never copied:
# it is live scratch for whatever that session is running. Newest first by
# the mtime of `target/debug/deps`, which moves every time cargo finishes a
# crate there -- and, for the same reason, the likeliest to still have a
# build actively writing into it (#1191). `--cold` and `POSTIO_CLAIM_SEED=0`
# opt out.
SEEDED=""
# Set as soon as a candidate is found, whatever `cp` goes on to do -- the one
# thing this needs to tell apart from "there was nothing to try" (#1190). A
# `cp` that fails (permissions, disk full, a sibling deleted mid-copy, a
# filesystem without reflinks and without a plain-copy fallback -- macOS)
# used to leave `SEEDED` empty exactly like the early-return case, so a real
# failure and "nothing to seed from" printed the same line and looked the
# same as the ordinary cold-start path this script already expects to take
# sometimes.
SEED_CANDIDATE=""
# The first candidate's own failure, "<path>: <cp's stderr>" -- kept rather
# than discarded (#1191). The newest candidate is also the one most likely
# to be mid-write, so its error is the one worth reading; a later
# candidate's failure in the same run is usually a symptom of the same
# underlying problem (a full disk, a permissions issue) rather than a
# second, independent thing to report.
SEED_FAILURE=""
seed_target() { # <tree>
    [ "${COLD:-0}" = 0 ] || return 0
    [ "${POSTIO_CLAIM_SEED:-1}" != 0 ] || return 0
    local candidate deps_dirs="" deps src started error
    # Every sibling, not only `issue-*` trees: a spec branch or a lane is as
    # good a seed, and the warmest tree on the box was often one of those
    # (#1717). The tree being seeded is skipped below.
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
            # The sibling's own crates have *its* path baked in; drop them so
            # cargo rebuilds ours and keeps the dependencies (lib/ says why).
            drop_workspace_artifacts "$1/target"
            SEEDED="$src ($(( $(date +%s) - started ))s)"
            return 0
        fi
        rm -rf "$1/target/debug"
        [ -n "$SEED_FAILURE" ] || SEED_FAILURE="$src: $error"
    done
}
